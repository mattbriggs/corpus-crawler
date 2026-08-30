//! The crawl coordinator: the concurrent pipeline and its invariants.
//!
//! # Pipeline
//!
//! ```text
//! walker -> discovery channel -> dispatcher -> bounded work queue
//!        -> worker slots -> result channel -> aggregator -> CSV
//! ```
//!
//! # Invariants held here
//!
//! * `active_workers <= configured_workers` - one Tokio task per slot, created
//!   once and never cloned.
//! * `queued_tasks <= queue_capacity` - the work queue is a bounded channel and
//!   the dispatcher is its only producer (REQ-050, REQ-051, AC-006).
//! * `active_request_per_worker <= 1` - a slot owns its handle exclusively and
//!   awaits each dispatch (REQ-062).
//! * `attempts <= 1 + max_retries` - retries happen in place inside the slot
//!   that owns the task, so a retry can never re-enter the queue and deadlock
//!   against a full channel (REQ-073).
//!
//! # Async boundary
//!
//! Everything decided here - retry eligibility, threshold arithmetic, state
//! transitions, schema validation - is a pure domain call. Async is confined to
//! channels, process dispatch, and traversal.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crawl_domain::crawl::{CrawlOutcome, CrawlState};
use crawl_domain::errors::{ErrorCategory, ErrorEvent, SchemaViolationEvent};
use crawl_domain::events::CrawlEvent;
use crawl_domain::ids::{CrawlId, PluginId, RequestId, WorkerId};
use crawl_domain::plugin::Plugin;
use crawl_domain::policy::{FailurePolicy, PartialOutputPolicy};
use crawl_domain::record::RawRecord;
use crawl_domain::statistics::CrawlStatistics;
use crawl_domain::task::FileTask;
use crawl_protocol::{Request, Response};
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use crate::ports::{
    DirectoryWalker, Discovery, EventSink, ReportWriter, WorkerFactory, WorkerHandle,
};
use crate::services::configuration::EffectiveConfig;

/// Capacity of the walker-to-dispatcher channel.
///
/// Deliberately small and independent of the work queue: it exists only to keep
/// traversal from stalling on each individual send. The *work queue* the
/// operator configures is the bounded channel between dispatcher and workers.
const DISCOVERY_CHANNEL_CAPACITY: usize = 64;

/// An unrecoverable crawl-engine failure (REQ-134).
#[derive(Debug, thiserror::Error)]
pub enum HostFatalError {
    /// No worker could be started, so no file could ever be processed.
    #[error("no plugin worker could be started: {0}")]
    NoWorkers(String),
    /// The report could not be written or finalized.
    #[error("report failure: {0}")]
    Report(String),
}

impl HostFatalError {
    /// Returns the outcome this failure produces.
    #[must_use]
    pub const fn outcome(&self) -> CrawlOutcome {
        match self {
            Self::NoWorkers(_) => CrawlOutcome::PluginFailure,
            Self::Report(_) => CrawlOutcome::HostFailure,
        }
    }
}

/// The result of one completed crawl.
#[derive(Debug, Clone)]
pub struct CrawlReport {
    /// Identity of the crawl.
    pub crawl: CrawlId,
    /// The completion outcome.
    pub outcome: CrawlOutcome,
    /// Final counters.
    pub statistics: CrawlStatistics,
}

/// Messages flowing from producers to the single aggregator task.
enum Pipeline {
    Discovered(PathBuf),
    Matched(PathBuf),
    Skipped(PathBuf),
    TraversalError(Box<ErrorEvent>),
    Started {
        path: PathBuf,
        request: RequestId,
        worker: WorkerId,
        attempt: u32,
    },
    Rows {
        path: PathBuf,
        request: RequestId,
        rows: Vec<RawRecord>,
        duration: Duration,
    },
    Failed {
        error: Box<ErrorEvent>,
        attempts: u32,
    },
    Retried {
        path: PathBuf,
        error: Box<ErrorEvent>,
        attempt: u32,
    },
    WorkerRestarted {
        worker: WorkerId,
        reason: String,
    },
    Diagnostic {
        worker: WorkerId,
        line: String,
    },
}

/// Coordinates preflight, discovery, scheduling, supervision, and finalization.
pub struct CrawlCoordinator {
    plugin: Plugin,
    config: EffectiveConfig,
    walker: Arc<dyn DirectoryWalker>,
    factory: Arc<dyn WorkerFactory>,
    writer: Box<dyn ReportWriter>,
    events: Arc<dyn EventSink>,
    cancel: CancellationToken,
}

impl CrawlCoordinator {
    /// Assembles a coordinator from its ports.
    #[must_use]
    pub fn new(
        plugin: Plugin,
        config: EffectiveConfig,
        walker: Arc<dyn DirectoryWalker>,
        factory: Arc<dyn WorkerFactory>,
        writer: Box<dyn ReportWriter>,
        events: Arc<dyn EventSink>,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            plugin,
            config,
            walker,
            factory,
            writer,
            events,
            cancel,
        }
    }

    /// Runs the crawl to completion.
    ///
    /// # Errors
    /// Returns [`HostFatalError`] only for failures that make the crawl
    /// meaningless: no worker could start, or the report could not be written.
    /// File-level failures are counted, not returned (REQ-120).
    pub async fn run(mut self) -> Result<CrawlReport, HostFatalError> {
        let crawl = CrawlId::generate();
        let started = Instant::now();
        let plugin_id = PluginId::new(self.plugin.name.clone());

        self.events.emit(&CrawlEvent::CrawlStarted {
            crawl,
            plugin: plugin_id.clone(),
            plugin_version: self.plugin.version.clone(),
            input: self.config.input.clone(),
            output: self.config.output.clone(),
            workers: self.config.workers.get(),
            queue_capacity: self.config.queue_capacity.get() as u32,
            timeout_seconds: self.config.timeout.map(|t| t.seconds()),
            max_retries: self.config.retry.limit().max_retries(),
        });

        let (discovery_tx, mut discovery_rx) = mpsc::channel(DISCOVERY_CHANNEL_CAPACITY);
        let (task_tx, task_rx) = mpsc::channel::<FileTask>(self.config.queue_capacity.get());
        let (result_tx, mut result_rx) =
            mpsc::channel::<Pipeline>(self.config.workers.get() as usize * 4 + 16);

        // --- traversal -------------------------------------------------------
        let walker = Arc::clone(&self.walker);
        let input_root = self.config.input.clone();
        let input_spec = self.plugin.input.clone();
        let walk_cancel = self.cancel.clone();
        let walk_task = tokio::spawn(async move {
            walker
                .walk(input_root, input_spec, discovery_tx, walk_cancel)
                .await;
        });

        // --- dispatcher: selection and backpressure --------------------------
        let selector = self.plugin.input.clone();
        let dispatch_results = result_tx.clone();
        let dispatch_cancel = self.cancel.clone();
        let dispatch_task = tokio::spawn(async move {
            while let Some(discovery) = discovery_rx.recv().await {
                if dispatch_cancel.is_cancelled() {
                    break;
                }
                match discovery {
                    Discovery::Error(error) => {
                        let _ = dispatch_results.send(Pipeline::TraversalError(error)).await;
                    }
                    Discovery::File(path) => {
                        let _ = dispatch_results
                            .send(Pipeline::Discovered(path.clone()))
                            .await;
                        if !selector.matches(&path) {
                            let _ = dispatch_results.send(Pipeline::Skipped(path)).await;
                            continue;
                        }
                        let _ = dispatch_results.send(Pipeline::Matched(path.clone())).await;
                        // The only place a task enters the bounded queue.
                        // A full queue suspends this loop, which suspends
                        // traversal: that is the backpressure guarantee.
                        tokio::select! {
                            biased;
                            () = dispatch_cancel.cancelled() => break,
                            result = task_tx.send(FileTask::new(path)) => {
                                if result.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            drop(task_tx);
        });

        // --- worker slots ----------------------------------------------------
        let shared_rx = Arc::new(Mutex::new(task_rx));
        let mut slots = Vec::with_capacity(self.config.workers.get() as usize);
        let mut startup_failure = None;
        for slot in 0..self.config.workers.get() {
            let id = WorkerId::new(slot);
            match self.factory.spawn(&self.plugin, id).await {
                Ok(handle) => {
                    self.events.emit(&CrawlEvent::WorkerStarted {
                        worker: handle.id(),
                        pid: handle.pid(),
                    });
                    slots.push((id, Some(handle)));
                }
                Err(error) => {
                    startup_failure = Some(error.to_string());
                    break;
                }
            }
        }

        if slots.is_empty() {
            self.cancel.cancel();
            walk_task.abort();
            dispatch_task.abort();
            let _ = self.writer.finalize(PartialOutputPolicy::RetainTemporary);
            return Err(HostFatalError::NoWorkers(
                startup_failure.unwrap_or_else(|| "worker count resolved to zero".to_owned()),
            ));
        }
        if let Some(reason) = startup_failure {
            // Some slots started: degrade rather than abort (REQ-121).
            self.events.emit(&CrawlEvent::WorkerCrashed {
                worker: WorkerId::new(slots.len() as u32),
                detail: format!("replacement slot could not start: {reason}"),
            });
        }

        let mut worker_tasks = Vec::with_capacity(slots.len());
        for (id, handle) in slots {
            let rx = Arc::clone(&shared_rx);
            let results = result_tx.clone();
            let factory = Arc::clone(&self.factory);
            let plugin = self.plugin.clone();
            let config = self.config.clone();
            let cancel = self.cancel.clone();
            worker_tasks.push(tokio::spawn(async move {
                run_worker_slot(id, handle, rx, results, factory, plugin, config, cancel).await;
            }));
        }
        drop(result_tx);

        // --- aggregation, validation, and writing ----------------------------
        let mut statistics = CrawlStatistics::new();
        let mut state = CrawlState::Running;
        let mut next_request_display = RequestId::new(0);
        let _ = &mut next_request_display;

        while let Some(message) = result_rx.recv().await {
            self.handle_message(message, &mut statistics);

            if state == CrawlState::Running && !self.cancel.is_cancelled() {
                if self
                    .config
                    .failure
                    .should_stop(statistics.qualifying_errors())
                {
                    state = state.transition(CrawlState::Stopping).unwrap_or(state);
                    self.cancel.cancel();
                    self.events.emit(&CrawlEvent::CrawlCancelled {
                        crawl,
                        reason: if self.config.failure.fail_fast {
                            "fail-fast policy triggered".to_owned()
                        } else {
                            format!(
                                "maximum error threshold of {} reached",
                                self.config.failure.max_errors.unwrap_or_default()
                            )
                        },
                    });
                }
            } else if state == CrawlState::Running && self.cancel.is_cancelled() {
                state = state.transition(CrawlState::Cancelling).unwrap_or(state);
            }
        }

        if state == CrawlState::Running && self.cancel.is_cancelled() {
            state = state.transition(CrawlState::Cancelling).unwrap_or(state);
        }

        for task in worker_tasks {
            let _ = task.await;
        }
        let _ = dispatch_task.await;
        let _ = walk_task.await;

        // --- finalization ----------------------------------------------------
        statistics.set_duration(started.elapsed());
        let outcome = CrawlOutcome::derive(state, &statistics);
        let policy = match outcome {
            CrawlOutcome::HostFailure => PartialOutputPolicy::RetainTemporary,
            _ => PartialOutputPolicy::Promote,
        };
        self.writer
            .finalize(policy)
            .map_err(|error| HostFatalError::Report(error.to_string()))?;

        self.events.emit(&CrawlEvent::CrawlCompleted {
            crawl,
            outcome,
            statistics: Box::new(statistics.clone()),
        });

        Ok(CrawlReport {
            crawl,
            outcome,
            statistics,
        })
    }

    /// Applies one pipeline message to the counters, the report, and the log.
    ///
    /// This is the only place records are validated and written, which is what
    /// makes "one writer" a structural property rather than a convention
    /// (REQ-102, AC-022).
    fn handle_message(&mut self, message: Pipeline, statistics: &mut CrawlStatistics) {
        match message {
            Pipeline::Discovered(path) => {
                statistics.entries_examined += 1;
                statistics.files_discovered += 1;
                self.events.emit(&CrawlEvent::FileDiscovered { path });
            }
            Pipeline::Matched(path) => {
                statistics.files_matched += 1;
                self.events.emit(&CrawlEvent::FileMatched { path });
            }
            Pipeline::Skipped(path) => self.events.emit(&CrawlEvent::FileSkipped { path }),
            Pipeline::TraversalError(error) => {
                statistics.entries_examined += 1;
                statistics.record_error(ErrorCategory::Traversal);
                self.events
                    .emit(&CrawlEvent::TraversalErrorDetected { error });
            }
            Pipeline::Started {
                path,
                request,
                worker,
                attempt,
            } => self.events.emit(&CrawlEvent::FileProcessingStarted {
                path,
                request,
                worker,
                attempt,
            }),
            Pipeline::Rows {
                path,
                request,
                rows,
                duration,
            } => {
                let mut accepted = 0u64;
                let mut rejected = 0u64;
                for (index, row) in rows.iter().enumerate() {
                    match self.plugin.schema.validate(row, self.plugin.extra_fields) {
                        Ok(validated) => {
                            if let Err(error) = self.writer.write(&validated) {
                                statistics.record_error(ErrorCategory::Report);
                                self.events.emit(&CrawlEvent::ProtocolViolationDetected {
                                    error: Box::new(
                                        ErrorEvent::new(ErrorCategory::Report, error.to_string())
                                            .with_file(path.clone()),
                                    ),
                                });
                            } else {
                                accepted += 1;
                                statistics.records_emitted += 1;
                            }
                        }
                        Err(violation) => {
                            rejected += 1;
                            statistics.records_rejected += 1;
                            statistics.record_error(ErrorCategory::Schema);
                            let field = match &violation {
                                crawl_domain::schema::SchemaViolation::MissingRequiredField {
                                    field,
                                }
                                | crawl_domain::schema::SchemaViolation::NullRequiredField {
                                    field,
                                }
                                | crawl_domain::schema::SchemaViolation::TypeMismatch {
                                    field,
                                    ..
                                }
                                | crawl_domain::schema::SchemaViolation::NestedValue {
                                    field,
                                    ..
                                }
                                | crawl_domain::schema::SchemaViolation::UndeclaredField {
                                    field,
                                } => Some(field.clone()),
                            };
                            let event =
                                SchemaViolationEvent::new(violation.to_string(), index, field)
                                    .with_file(path.clone())
                                    .with_request(request);
                            self.events.emit(&CrawlEvent::RecordRejected {
                                violation: Box::new(event),
                            });
                        }
                    }
                }
                statistics.files_completed += 1;
                self.events.emit(&CrawlEvent::FileProcessingCompleted {
                    path,
                    request,
                    rows_accepted: accepted,
                    rows_rejected: rejected,
                    duration_seconds: duration.as_secs_f64(),
                });
            }
            Pipeline::Failed { error, attempts } => {
                statistics.files_failed += 1;
                statistics.record_error(error.category);
                if error.category == ErrorCategory::Timeout {
                    statistics.timeouts += 1;
                }
                self.events
                    .emit(&CrawlEvent::FileProcessingFailed { error, attempts });
            }
            Pipeline::Retried {
                path,
                error,
                attempt,
            } => {
                statistics.retries += 1;
                if error.category == ErrorCategory::Timeout {
                    statistics.timeouts += 1;
                }
                self.events.emit(&CrawlEvent::FileProcessingRetried {
                    path,
                    error,
                    attempt,
                });
            }
            Pipeline::WorkerRestarted { worker, reason } => {
                statistics.worker_restarts += 1;
                self.events
                    .emit(&CrawlEvent::WorkerRestarted { worker, reason });
            }
            Pipeline::Diagnostic { worker, line } => {
                self.events
                    .emit(&CrawlEvent::WorkerDiagnostic { worker, line });
            }
        }
    }
}

/// Drives one worker slot for the life of the crawl.
///
/// The slot owns its handle exclusively, so "one active request per worker" is
/// enforced by ownership rather than by a lock. Retries happen here, in place,
/// against a replacement worker.
#[allow(clippy::too_many_arguments)]
async fn run_worker_slot(
    mut id: WorkerId,
    mut handle: Option<Box<dyn WorkerHandle>>,
    tasks: Arc<Mutex<mpsc::Receiver<FileTask>>>,
    results: mpsc::Sender<Pipeline>,
    factory: Arc<dyn WorkerFactory>,
    plugin: Plugin,
    config: EffectiveConfig,
    cancel: CancellationToken,
) {
    let mut next_request = u64::from(id.slot) * 1_000_000_000 + 1;

    loop {
        if cancel.is_cancelled() {
            break;
        }
        let task = {
            let mut receiver = tasks.lock().await;
            tokio::select! {
                biased;
                () = cancel.cancelled() => None,
                task = receiver.recv() => task,
            }
        };
        let Some(mut task) = task else { break };

        loop {
            let attempt = task.record_attempt();
            let request_id = RequestId::new(next_request);
            next_request += 1;

            // A previous failure may have left this slot without a worker.
            if handle.is_none() {
                id = id.next_generation();
                match factory.spawn(&plugin, id).await {
                    Ok(replacement) => {
                        let _ = results
                            .send(Pipeline::WorkerRestarted {
                                worker: id,
                                reason: "replacing an invalidated worker".to_owned(),
                            })
                            .await;
                        handle = Some(replacement);
                    }
                    Err(error) => {
                        let _ = results
                            .send(Pipeline::Failed {
                                error: Box::new(
                                    ErrorEvent::new(
                                        ErrorCategory::RuntimeStartup,
                                        format!("replacement worker could not start: {error}"),
                                    )
                                    .with_file(task.path())
                                    .with_worker(id),
                                ),
                                attempts: attempt,
                            })
                            .await;
                        return;
                    }
                }
            }
            let worker = handle.as_mut().expect("worker present");

            let _ = results
                .send(Pipeline::Started {
                    path: task.path().to_path_buf(),
                    request: request_id,
                    worker: worker.id(),
                    attempt,
                })
                .await;

            let request = Request::process(request_id, task.path().to_string_lossy().into_owned());
            let started = Instant::now();
            let dispatched = worker
                .dispatch(request, config.timeout.map(|t| t.duration()))
                .await;
            let elapsed = started.elapsed();

            for line in worker.take_diagnostics() {
                let _ = results
                    .send(Pipeline::Diagnostic {
                        worker: worker.id(),
                        line,
                    })
                    .await;
            }

            let failure = match dispatched {
                Ok(Response::Ok { rows, .. }) => {
                    let _ = results
                        .send(Pipeline::Rows {
                            path: task.path().to_path_buf(),
                            request: request_id,
                            rows,
                            duration: elapsed,
                        })
                        .await;
                    break;
                }
                Ok(Response::Error { error, .. }) => Some((
                    ErrorCategory::PluginProcessing,
                    format!("{}: {}", error.code, error.message),
                    Some(error.code),
                )),
                Ok(other) => Some((
                    ErrorCategory::Protocol,
                    format!("worker sent an unexpected {} message", other.status()),
                    None,
                )),
                Err(worker_failure) => {
                    let category = worker_failure.category();
                    let message = worker_failure.to_string();
                    if worker_failure.invalidates_worker() {
                        if let Some(mut invalid) = handle.take() {
                            invalid.kill().await;
                        }
                    }
                    Some((category, message, None))
                }
            };

            let Some((category, message, code)) = failure else {
                break;
            };

            let mut error = ErrorEvent::new(category, message)
                .with_file(task.path())
                .with_request(request_id)
                .with_worker(id);
            if let Some(code) = code {
                error = error.with_code(code);
            }

            if config.retry.should_retry(category, attempt) && !cancel.is_cancelled() {
                let _ = results
                    .send(Pipeline::Retried {
                        path: task.path().to_path_buf(),
                        error: Box::new(error),
                        attempt,
                    })
                    .await;
                continue;
            }

            let _ = results
                .send(Pipeline::Failed {
                    error: Box::new(error),
                    attempts: attempt,
                })
                .await;
            break;
        }
    }

    if let Some(mut worker) = handle.take() {
        worker.shutdown().await;
    }
}

/// Convenience alias documenting that fail-fast and thresholds share one policy.
pub type StopPolicy = FailurePolicy;
