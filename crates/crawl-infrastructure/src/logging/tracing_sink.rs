//! Structured event logging over `tracing` (REQ-110 - REQ-118).
//!
//! Levels are chosen so a million-file crawl stays readable: per-file events
//! are `TRACE`, failures are `WARN`, crawl lifecycle is `INFO`.

use crawl_application::ports::event_sink::EventSink;
use crawl_domain::events::CrawlEvent;

/// Emits crawl events as structured `tracing` events.
#[derive(Debug, Default, Clone, Copy)]
pub struct TracingEventSink;

impl EventSink for TracingEventSink {
    /// Renders one event, choosing its level by frequency rather than by
    /// severity.
    ///
    /// Per-file events are `TRACE` because a million-file crawl emits a
    /// million of them; failures are `WARN` because they are rare and
    /// actionable; crawl lifecycle is `INFO`. Fields are named consistently
    /// across arms - `category`, `file`, `worker` - so a log consumer can
    /// filter on them without knowing which event it is looking at.
    fn emit(&self, event: &CrawlEvent) {
        let name = event.name();
        match event {
            CrawlEvent::CrawlStarted {
                crawl,
                plugin,
                plugin_version,
                input,
                output,
                workers,
                queue_capacity,
                timeout_seconds,
                max_retries,
            } => tracing::info!(
                event = name,
                crawl = %crawl,
                plugin = %plugin,
                plugin_version = %plugin_version,
                input = %input.display(),
                output = %output.display(),
                workers,
                queue_capacity,
                timeout_seconds,
                max_retries,
                "crawl started"
            ),
            CrawlEvent::CrawlCompleted {
                crawl,
                outcome,
                statistics,
            } => tracing::info!(
                event = name,
                crawl = %crawl,
                outcome = outcome.as_str(),
                files_discovered = statistics.files_discovered,
                files_matched = statistics.files_matched,
                files_completed = statistics.files_completed,
                files_failed = statistics.files_failed,
                records_emitted = statistics.records_emitted,
                records_rejected = statistics.records_rejected,
                total_errors = statistics.total_errors(),
                timeouts = statistics.timeouts,
                worker_restarts = statistics.worker_restarts,
                duration_seconds = statistics.duration_seconds,
                "crawl completed"
            ),
            CrawlEvent::CrawlCancelled { crawl, reason } => {
                tracing::warn!(event = name, crawl = %crawl, reason, "crawl stopping");
            }
            CrawlEvent::WorkerStarted { worker, pid } => {
                tracing::debug!(event = name, worker = %worker, pid, "worker started");
            }
            CrawlEvent::WorkerCrashed { worker, detail } => {
                tracing::warn!(event = name, worker = %worker, detail, "worker crashed");
            }
            CrawlEvent::WorkerTimedOut {
                worker,
                path,
                timeout_seconds,
            } => tracing::warn!(
                event = name,
                worker = %worker,
                file = %path.display(),
                timeout_seconds,
                "worker exceeded per-file timeout"
            ),
            CrawlEvent::WorkerRestarted { worker, reason } => {
                tracing::warn!(event = name, worker = %worker, reason, "worker restarted");
            }
            CrawlEvent::WorkerStopped { worker } => {
                tracing::debug!(event = name, worker = %worker, "worker stopped");
            }
            CrawlEvent::WorkerDiagnostic { worker, line } => {
                tracing::debug!(event = name, worker = %worker, line, "worker diagnostic");
            }
            CrawlEvent::FileProcessingRetried {
                path,
                error,
                attempt,
            } => tracing::warn!(
                event = name,
                file = %path.display(),
                category = error.category.as_str(),
                attempt,
                message = error.message,
                "retrying file"
            ),
            CrawlEvent::FileProcessingFailed { error, attempts } => tracing::warn!(
                event = name,
                category = error.category.as_str(),
                file = error.file.as_ref().map(|p| p.display().to_string()),
                attempts,
                message = error.message,
                code = error.code,
                "file processing failed"
            ),
            CrawlEvent::RecordRejected { violation } => tracing::warn!(
                event = name,
                category = "schema",
                file = violation
                    .error
                    .file
                    .as_ref()
                    .map(|p| p.display().to_string()),
                row = violation.row_index,
                field = violation.field,
                message = violation.error.message,
                "record rejected"
            ),
            CrawlEvent::ProtocolViolationDetected { error } => tracing::warn!(
                event = name,
                category = error.category.as_str(),
                file = error.file.as_ref().map(|p| p.display().to_string()),
                message = error.message,
                "protocol violation"
            ),
            CrawlEvent::TraversalErrorDetected { error } => tracing::warn!(
                event = name,
                category = error.category.as_str(),
                message = error.message,
                "traversal error"
            ),
            CrawlEvent::FileDiscovered { path }
            | CrawlEvent::FileMatched { path }
            | CrawlEvent::FileSkipped { path }
            | CrawlEvent::RecordAccepted { path } => {
                tracing::trace!(event = name, file = %path.display());
            }
            CrawlEvent::FileProcessingStarted {
                path,
                request,
                worker,
                attempt,
            } => tracing::trace!(
                event = name,
                file = %path.display(),
                request = request.get(),
                worker = %worker,
                attempt
            ),
            CrawlEvent::FileProcessingCompleted {
                path,
                rows_accepted,
                rows_rejected,
                duration_seconds,
                ..
            } => tracing::trace!(
                event = name,
                file = %path.display(),
                rows_accepted,
                rows_rejected,
                duration_seconds
            ),
        }
    }
}
