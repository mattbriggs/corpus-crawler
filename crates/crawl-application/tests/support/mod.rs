//! Test doubles for the application ports.
//!
//! These let the coordinator's concurrency invariants be asserted directly,
//! with no subprocesses, filesystem, or CSV involved. The acceptance suite
//! covers the real adapters; this covers the orchestration logic.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use crawl_application::ports::report::{ReportError, ReportWriter};
use crawl_application::ports::walker::{DirectoryWalker, Discovery};
use crawl_application::ports::worker::{
    WorkerFactory, WorkerFailure, WorkerHandle, WorkerStartupError,
};
use crawl_application::ports::EventSink;
use crawl_domain::events::CrawlEvent;
use crawl_domain::ids::{RequestId, WorkerId};
use crawl_domain::manifest::PluginManifest;
use crawl_domain::plugin::{InputSpec, Plugin};
use crawl_domain::policy::PartialOutputPolicy;
use crawl_domain::record::{RawRecord, RawValue, ValidatedRecord};
use crawl_protocol::{PluginError, Request, Response};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Builds a validated plugin for tests, with a three-field schema.
pub fn test_plugin() -> Plugin {
    let manifest = r#"
api_version: "1"
plugin:
  name: test-plugin
  version: "1.0.0"
runtime:
  type: python
  command: ["python3", "worker.py"]
input:
  extensions: [".txt"]
output:
  format: records
  schema:
    filename:
      type: string
      required: true
    line:
      type: integer
      required: true
    note:
      type: string
      required: false
"#;
    PluginManifest::parse(manifest, Path::new("plugin.yaml"))
        .expect("fixture manifest parses")
        .validate(Path::new("/plugins/test"))
        .expect("fixture manifest validates")
}

/// A well-formed row for [`test_plugin`]'s schema.
pub fn valid_row(path: &Path, line: i64) -> RawRecord {
    [
        (
            "filename".to_owned(),
            RawValue::String(path.to_string_lossy().into_owned()),
        ),
        ("line".to_owned(), RawValue::Integer(line)),
    ]
    .into_iter()
    .collect()
}

/// A row violating the declared type of `line`.
pub fn invalid_row(path: &Path) -> RawRecord {
    [
        (
            "filename".to_owned(),
            RawValue::String(path.to_string_lossy().into_owned()),
        ),
        (
            "line".to_owned(),
            RawValue::String("not-an-integer".to_owned()),
        ),
    ]
    .into_iter()
    .collect()
}

// --- walker --------------------------------------------------------------

/// Emits a fixed list of paths, then finishes.
pub struct ScriptedWalker {
    pub paths: Vec<PathBuf>,
}

#[async_trait]
impl DirectoryWalker for ScriptedWalker {
    async fn walk(
        &self,
        _root: PathBuf,
        _input: InputSpec,
        sink: mpsc::Sender<Discovery>,
        cancel: CancellationToken,
    ) {
        for path in &self.paths {
            if cancel.is_cancelled() || sink.send(Discovery::File(path.clone())).await.is_err() {
                break;
            }
        }
    }
}

// --- report writer -------------------------------------------------------

/// Rows captured by a recording writer, in write order.
pub type CapturedRows = Arc<Mutex<Vec<Vec<String>>>>;

/// The finalization policy a writer was closed with, once it has been.
pub type CapturedFinalization = Arc<Mutex<Option<PartialOutputPolicy>>>;

/// Records every written row and the finalization policy applied.
#[derive(Default)]
pub struct RecordingWriter {
    pub rows: CapturedRows,
    pub finalized: CapturedFinalization,
}

impl RecordingWriter {
    pub fn handles(&self) -> (CapturedRows, CapturedFinalization) {
        (Arc::clone(&self.rows), Arc::clone(&self.finalized))
    }
}

impl ReportWriter for RecordingWriter {
    fn write(&mut self, record: &ValidatedRecord) -> Result<(), ReportError> {
        self.rows.lock().expect("rows").push(record.to_csv_fields());
        Ok(())
    }

    fn finalize(&mut self, policy: PartialOutputPolicy) -> Result<(), ReportError> {
        *self.finalized.lock().expect("finalized") = Some(policy);
        Ok(())
    }
}

/// A writer whose `write` always fails, for report-error paths.
#[derive(Default)]
pub struct FailingWriter {
    pub finalized: CapturedFinalization,
}

impl FailingWriter {
    pub fn handle(&self) -> CapturedFinalization {
        Arc::clone(&self.finalized)
    }
}

impl ReportWriter for FailingWriter {
    fn write(&mut self, _record: &ValidatedRecord) -> Result<(), ReportError> {
        Err(ReportError("disk full".to_owned()))
    }

    fn finalize(&mut self, policy: PartialOutputPolicy) -> Result<(), ReportError> {
        *self.finalized.lock().expect("finalized") = Some(policy);
        Ok(())
    }
}

// --- event sink ----------------------------------------------------------

/// Collects the name of every emitted event.
#[derive(Default)]
pub struct RecordingSink {
    pub events: Arc<Mutex<Vec<String>>>,
}

impl RecordingSink {
    pub fn handle(&self) -> Arc<Mutex<Vec<String>>> {
        Arc::clone(&self.events)
    }
}

impl EventSink for RecordingSink {
    fn emit(&self, event: &CrawlEvent) {
        self.events
            .lock()
            .expect("events")
            .push(event.name().to_owned());
    }
}

/// Counts occurrences of one event name.
pub fn count_events(events: &Arc<Mutex<Vec<String>>>, name: &str) -> usize {
    events
        .lock()
        .expect("events")
        .iter()
        .filter(|event| *event == name)
        .count()
}

// --- workers -------------------------------------------------------------

/// How a scripted worker should answer one dispatch.
#[derive(Debug, Clone)]
pub enum Behaviour {
    /// Return the given number of valid rows.
    Rows(usize),
    /// Return one valid row and one schema-invalid row.
    OneInvalidRow,
    /// Return a plugin-declared error.
    PluginError,
    /// Fail as a timeout.
    Timeout,
    /// Fail as a crash.
    Crash,
}

/// Shared state observed by assertions after a crawl.
#[derive(Default)]
pub struct WorkerObservations {
    /// Dispatches per file path.
    pub dispatches: Mutex<HashMap<PathBuf, u32>>,
    /// Number of workers ever spawned.
    pub spawned: AtomicUsize,
    /// Highest number of simultaneously live workers.
    pub peak_live: AtomicU32,
    /// Currently live workers.
    live: AtomicU32,
    /// Set if two dispatches ever overlapped on one worker.
    pub concurrent_dispatch_detected: AtomicBool,
    /// Spawns fail once this many workers have been created.
    pub spawn_limit: Mutex<Option<usize>>,
}

impl WorkerObservations {
    pub fn attempts_for(&self, path: &Path) -> u32 {
        self.dispatches
            .lock()
            .expect("dispatches")
            .get(path)
            .copied()
            .unwrap_or(0)
    }

    pub fn total_dispatches(&self) -> u32 {
        self.dispatches.lock().expect("dispatches").values().sum()
    }
}

/// Spawns workers that follow a per-path behaviour script.
pub struct ScriptedFactory {
    /// Behaviour by file name; paths not listed return one valid row.
    pub script: HashMap<String, Behaviour>,
    /// Behaviour applied only on a path's first attempt, so retries can succeed.
    pub first_attempt_only: bool,
    pub observations: Arc<WorkerObservations>,
}

impl ScriptedFactory {
    pub fn new(script: HashMap<String, Behaviour>) -> Self {
        Self {
            script,
            first_attempt_only: false,
            observations: Arc::new(WorkerObservations::default()),
        }
    }

    pub fn observations(&self) -> Arc<WorkerObservations> {
        Arc::clone(&self.observations)
    }

    /// Makes spawns fail once `limit` workers have been created, so the
    /// degraded-pool and failed-replacement paths can be exercised.
    pub fn with_spawn_limit(self, limit: usize) -> Self {
        *self.observations.spawn_limit.lock().expect("limit") = Some(limit);
        self
    }
}

#[async_trait]
impl WorkerFactory for ScriptedFactory {
    async fn spawn(
        &self,
        _plugin: &Plugin,
        id: WorkerId,
    ) -> Result<Box<dyn WorkerHandle>, WorkerStartupError> {
        let observations = Arc::clone(&self.observations);
        let already = observations.spawned.fetch_add(1, Ordering::SeqCst);
        if let Some(limit) = *observations.spawn_limit.lock().expect("limit") {
            if already >= limit {
                return Err(WorkerStartupError::Spawn {
                    command: "scripted".to_owned(),
                    reason: "spawn limit reached".to_owned(),
                });
            }
        }
        let live = observations.live.fetch_add(1, Ordering::SeqCst) + 1;
        observations.peak_live.fetch_max(live, Ordering::SeqCst);

        Ok(Box::new(ScriptedWorker {
            id,
            script: self.script.clone(),
            first_attempt_only: self.first_attempt_only,
            observations,
            dispatching: AtomicBool::new(false),
        }))
    }
}

struct ScriptedWorker {
    id: WorkerId,
    script: HashMap<String, Behaviour>,
    first_attempt_only: bool,
    observations: Arc<WorkerObservations>,
    dispatching: AtomicBool,
}

#[async_trait]
impl WorkerHandle for ScriptedWorker {
    fn id(&self) -> WorkerId {
        self.id
    }

    fn pid(&self) -> Option<u32> {
        Some(4242)
    }

    async fn dispatch(
        &mut self,
        request: Request,
        _timeout: Option<Duration>,
    ) -> Result<Response, WorkerFailure> {
        // REQ-062: a worker must never have two requests in flight.
        if self.dispatching.swap(true, Ordering::SeqCst) {
            self.observations
                .concurrent_dispatch_detected
                .store(true, Ordering::SeqCst);
        }

        let path = PathBuf::from(request.filepath.clone().unwrap_or_default());
        let attempt = {
            let mut dispatches = self.observations.dispatches.lock().expect("dispatches");
            let counter = dispatches.entry(path.clone()).or_insert(0);
            *counter += 1;
            *counter
        };

        // Yield so overlapping dispatches would actually be observable.
        tokio::task::yield_now().await;
        self.dispatching.store(false, Ordering::SeqCst);

        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        let behaviour = self.script.get(&name).cloned();
        let behaviour = match behaviour {
            Some(_) if self.first_attempt_only && attempt > 1 => None,
            other => other,
        };

        match behaviour {
            None | Some(Behaviour::Rows(_)) => {
                let count = match behaviour {
                    Some(Behaviour::Rows(count)) => count,
                    _ => 1,
                };
                Ok(Response::Ok {
                    id: request.id,
                    rows: (0..count)
                        .map(|index| valid_row(&path, index as i64 + 1))
                        .collect(),
                })
            }
            Some(Behaviour::OneInvalidRow) => Ok(Response::Ok {
                id: request.id,
                rows: vec![valid_row(&path, 1), invalid_row(&path)],
            }),
            Some(Behaviour::PluginError) => Ok(Response::Error {
                id: request.id,
                error: PluginError::new("refused", "this fixture refuses this file"),
            }),
            Some(Behaviour::Timeout) => Err(WorkerFailure::Timeout { seconds: 1.0 }),
            Some(Behaviour::Crash) => Err(WorkerFailure::Crash("worker exited".to_owned())),
        }
    }

    async fn shutdown(&mut self) {
        self.observations.live.fetch_sub(1, Ordering::SeqCst);
    }

    async fn kill(&mut self) {
        self.observations.live.fetch_sub(1, Ordering::SeqCst);
    }

    fn take_diagnostics(&mut self) -> Vec<String> {
        Vec::new()
    }
}

/// A factory that always fails to spawn, for startup-failure tests.
pub struct FailingFactory;

#[async_trait]
impl WorkerFactory for FailingFactory {
    async fn spawn(
        &self,
        _plugin: &Plugin,
        _id: WorkerId,
    ) -> Result<Box<dyn WorkerHandle>, WorkerStartupError> {
        Err(WorkerStartupError::Spawn {
            command: "missing-interpreter".to_owned(),
            reason: "no such file or directory".to_owned(),
        })
    }
}

/// A factory whose worker answers the handshake but nothing else, used to
/// exercise validation.
pub struct HandshakeOnlyFactory {
    pub selftest_fails: bool,
}

#[async_trait]
impl WorkerFactory for HandshakeOnlyFactory {
    async fn spawn(
        &self,
        _plugin: &Plugin,
        id: WorkerId,
    ) -> Result<Box<dyn WorkerHandle>, WorkerStartupError> {
        Ok(Box::new(HandshakeOnlyWorker {
            id,
            selftest_fails: self.selftest_fails,
        }))
    }
}

struct HandshakeOnlyWorker {
    id: WorkerId,
    selftest_fails: bool,
}

#[async_trait]
impl WorkerHandle for HandshakeOnlyWorker {
    fn id(&self) -> WorkerId {
        self.id
    }

    fn pid(&self) -> Option<u32> {
        None
    }

    async fn dispatch(
        &mut self,
        request: Request,
        _timeout: Option<Duration>,
    ) -> Result<Response, WorkerFailure> {
        if self.selftest_fails {
            return Ok(Response::Error {
                id: request.id,
                error: PluginError::new("selftest_failed", "assertion failed"),
            });
        }
        Ok(Response::Ok {
            id: request.id,
            rows: vec![],
        })
    }

    async fn shutdown(&mut self) {}

    async fn kill(&mut self) {}

    fn take_diagnostics(&mut self) -> Vec<String> {
        Vec::new()
    }
}

/// Convenience: the request id the coordinator would use first.
pub const FIRST_REQUEST: RequestId = RequestId::new(1);
