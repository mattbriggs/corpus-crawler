//! Operational events (SRS 6.7).
//!
//! Events are the observability and testing boundary of the pipeline. They are
//! not an event-sourcing API: nothing is reconstructed from them. The host
//! emits them through an application port so that logging is an adapter
//! concern, and integration tests can assert on events without parsing logs.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::crawl::CrawlOutcome;
use crate::errors::{ErrorEvent, SchemaViolationEvent};
use crate::ids::{CrawlId, PluginId, PluginVersion, RequestId, WorkerId};
use crate::statistics::CrawlStatistics;

/// One observable occurrence in the crawl pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum CrawlEvent {
    /// A crawl began, carrying the execution metadata required by REQ-035.
    CrawlStarted {
        /// Identity of this crawl.
        crawl: CrawlId,
        /// Selected plugin.
        plugin: PluginId,
        /// Selected plugin version (REQ-113).
        plugin_version: PluginVersion,
        /// Input root.
        input: PathBuf,
        /// Requested report path.
        output: PathBuf,
        /// Effective worker count.
        workers: u32,
        /// Effective queue capacity.
        queue_capacity: u32,
        /// Effective per-file timeout in seconds, if any.
        timeout_seconds: Option<f64>,
        /// Effective retry allowance.
        max_retries: u32,
    },
    /// A crawl reached a terminal outcome.
    CrawlCompleted {
        /// Identity of this crawl.
        crawl: CrawlId,
        /// The final outcome.
        outcome: CrawlOutcome,
        /// Final counters.
        statistics: Box<CrawlStatistics>,
    },
    /// Cancellation was accepted.
    CrawlCancelled {
        /// Identity of this crawl.
        crawl: CrawlId,
        /// Why cancellation was triggered.
        reason: String,
    },
    /// Traversal observed a regular file.
    FileDiscovered {
        /// Path of the discovered file.
        path: PathBuf,
    },
    /// A discovered file matched the effective extension configuration.
    FileMatched {
        /// Path of the matched file.
        path: PathBuf,
    },
    /// A discovered file did not match and will not be dispatched (REQ-043).
    FileSkipped {
        /// Path of the skipped file.
        path: PathBuf,
    },
    /// A worker began processing a file.
    FileProcessingStarted {
        /// Path of the file.
        path: PathBuf,
        /// Correlating request identifier.
        request: RequestId,
        /// Worker handling the request.
        worker: WorkerId,
        /// Attempt number, starting at 1.
        attempt: u32,
    },
    /// A file reached a terminal successful outcome.
    FileProcessingCompleted {
        /// Path of the file.
        path: PathBuf,
        /// Correlating request identifier.
        request: RequestId,
        /// Rows accepted for this file.
        rows_accepted: u64,
        /// Rows rejected for this file.
        rows_rejected: u64,
        /// Processing duration in seconds.
        duration_seconds: f64,
    },
    /// A file reached a terminal failed outcome.
    FileProcessingFailed {
        /// The attributable error.
        error: Box<ErrorEvent>,
        /// Attempts made before giving up.
        attempts: u32,
    },
    /// A failed attempt will be retried.
    FileProcessingRetried {
        /// Path of the file.
        path: PathBuf,
        /// The failure that triggered the retry.
        error: Box<ErrorEvent>,
        /// The attempt number that just failed.
        attempt: u32,
    },
    /// A record satisfied the declared schema and was written.
    RecordAccepted {
        /// Source file of the record.
        path: PathBuf,
    },
    /// A record violated the declared schema and was not written (REQ-097).
    RecordRejected {
        /// The violation, with attribution.
        violation: Box<SchemaViolationEvent>,
    },
    /// A worker process started and completed its handshake.
    WorkerStarted {
        /// Identity of the worker slot and generation.
        worker: WorkerId,
        /// Operating-system process identifier, when known.
        pid: Option<u32>,
    },
    /// A worker process exited unexpectedly.
    WorkerCrashed {
        /// Identity of the worker.
        worker: WorkerId,
        /// Description of the termination, including exit status when known.
        detail: String,
    },
    /// A worker exceeded the per-file deadline and was invalidated.
    WorkerTimedOut {
        /// Identity of the worker.
        worker: WorkerId,
        /// File being processed when the deadline expired.
        path: PathBuf,
        /// The configured deadline in seconds.
        timeout_seconds: f64,
    },
    /// A replacement worker was started for a failed slot.
    WorkerRestarted {
        /// Identity of the replacement worker.
        worker: WorkerId,
        /// Why the previous generation was replaced.
        reason: String,
    },
    /// A worker was shut down as part of orderly termination.
    WorkerStopped {
        /// Identity of the worker.
        worker: WorkerId,
    },
    /// A worker wrote a line to `stderr` (REQ-115).
    WorkerDiagnostic {
        /// Identity of the worker.
        worker: WorkerId,
        /// The captured line, never parsed as protocol.
        line: String,
    },
    /// A worker message violated the protocol contract.
    ProtocolViolationDetected {
        /// The attributable error.
        error: Box<ErrorEvent>,
    },
    /// A directory entry could not be traversed (REQ-045).
    TraversalErrorDetected {
        /// The attributable error.
        error: Box<ErrorEvent>,
    },
}

impl CrawlEvent {
    /// Returns the stable event name used in structured logs.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::CrawlStarted { .. } => "crawl_started",
            Self::CrawlCompleted { .. } => "crawl_completed",
            Self::CrawlCancelled { .. } => "crawl_cancelled",
            Self::FileDiscovered { .. } => "file_discovered",
            Self::FileMatched { .. } => "file_matched",
            Self::FileSkipped { .. } => "file_skipped",
            Self::FileProcessingStarted { .. } => "file_processing_started",
            Self::FileProcessingCompleted { .. } => "file_processing_completed",
            Self::FileProcessingFailed { .. } => "file_processing_failed",
            Self::FileProcessingRetried { .. } => "file_processing_retried",
            Self::RecordAccepted { .. } => "record_accepted",
            Self::RecordRejected { .. } => "record_rejected",
            Self::WorkerStarted { .. } => "worker_started",
            Self::WorkerCrashed { .. } => "worker_crashed",
            Self::WorkerTimedOut { .. } => "worker_timed_out",
            Self::WorkerRestarted { .. } => "worker_restarted",
            Self::WorkerStopped { .. } => "worker_stopped",
            Self::WorkerDiagnostic { .. } => "worker_diagnostic",
            Self::ProtocolViolationDetected { .. } => "protocol_violation_detected",
            Self::TraversalErrorDetected { .. } => "traversal_error_detected",
        }
    }

    /// Returns `true` for the high-frequency per-file events that are only
    /// useful at trace verbosity.
    ///
    /// Emitting one log line per discovered file would dominate the log of a
    /// million-file crawl, so the logging adapter uses this to choose a level.
    #[must_use]
    pub const fn is_high_frequency(&self) -> bool {
        matches!(
            self,
            Self::FileDiscovered { .. }
                | Self::FileMatched { .. }
                | Self::FileSkipped { .. }
                | Self::FileProcessingStarted { .. }
                | Self::FileProcessingCompleted { .. }
                | Self::RecordAccepted { .. }
        )
    }

    /// Returns the error carried by this event, if it reports a failure.
    #[must_use]
    pub fn error(&self) -> Option<&ErrorEvent> {
        match self {
            Self::FileProcessingFailed { error, .. }
            | Self::FileProcessingRetried { error, .. }
            | Self::ProtocolViolationDetected { error }
            | Self::TraversalErrorDetected { error } => Some(error),
            Self::RecordRejected { violation } => Some(&violation.error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crawl::CrawlOutcome;
    use crate::errors::ErrorCategory;
    use crate::ids::{PluginName, PluginVersion};

    /// One instance of every event variant.
    ///
    /// Adding a variant without adding it here makes
    /// [`every_variant_is_represented`] fail, which is what keeps the
    /// exhaustive tests below honest as the enum grows.
    fn every_variant() -> Vec<CrawlEvent> {
        let crawl = CrawlId::generate();
        let worker = WorkerId::new(0);
        let path = PathBuf::from("/data/a.txt");
        let error = || Box::new(ErrorEvent::new(ErrorCategory::Timeout, "deadline"));

        vec![
            CrawlEvent::CrawlStarted {
                crawl,
                plugin: PluginId::new(PluginName::new("p").expect("name")),
                plugin_version: PluginVersion::new("1").expect("version"),
                input: PathBuf::from("/in"),
                output: PathBuf::from("/out.csv"),
                workers: 2,
                queue_capacity: 8,
                timeout_seconds: None,
                max_retries: 0,
            },
            CrawlEvent::CrawlCompleted {
                crawl,
                outcome: CrawlOutcome::Success,
                statistics: Box::new(CrawlStatistics::new()),
            },
            CrawlEvent::CrawlCancelled {
                crawl,
                reason: "operator".into(),
            },
            CrawlEvent::FileDiscovered { path: path.clone() },
            CrawlEvent::FileMatched { path: path.clone() },
            CrawlEvent::FileSkipped { path: path.clone() },
            CrawlEvent::FileProcessingStarted {
                path: path.clone(),
                request: RequestId::new(1),
                worker,
                attempt: 1,
            },
            CrawlEvent::FileProcessingCompleted {
                path: path.clone(),
                request: RequestId::new(1),
                rows_accepted: 1,
                rows_rejected: 0,
                duration_seconds: 0.0,
            },
            CrawlEvent::FileProcessingFailed {
                error: error(),
                attempts: 1,
            },
            CrawlEvent::FileProcessingRetried {
                path: path.clone(),
                error: error(),
                attempt: 1,
            },
            CrawlEvent::RecordAccepted { path: path.clone() },
            CrawlEvent::RecordRejected {
                violation: Box::new(SchemaViolationEvent::new("bad", 0, None)),
            },
            CrawlEvent::WorkerStarted { worker, pid: None },
            CrawlEvent::WorkerCrashed {
                worker,
                detail: "exit 1".into(),
            },
            CrawlEvent::WorkerTimedOut {
                worker,
                path: path.clone(),
                timeout_seconds: 1.0,
            },
            CrawlEvent::WorkerRestarted {
                worker,
                reason: "crash".into(),
            },
            CrawlEvent::WorkerStopped { worker },
            CrawlEvent::WorkerDiagnostic {
                worker,
                line: "note".into(),
            },
            CrawlEvent::ProtocolViolationDetected { error: error() },
            CrawlEvent::TraversalErrorDetected { error: error() },
        ]
    }

    #[test]
    fn every_variant_is_represented() {
        // A count, rather than a match, because a match on `_` would defeat the
        // purpose. Bump this when you add a variant, and the exhaustive tests
        // below start covering it.
        assert_eq!(
            every_variant().len(),
            20,
            "every_variant() must construct one of each CrawlEvent variant"
        );
    }

    #[test]
    fn event_names_are_unique_across_every_variant() {
        // Log consumers key off these names, so a collision would silently
        // merge two distinct occurrences into one apparent event type.
        let mut names: Vec<&str> = every_variant().iter().map(CrawlEvent::name).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total, "two event variants share a name");
    }

    #[test]
    fn event_names_are_stable_snake_case_identifiers() {
        for event in every_variant() {
            let name = event.name();
            assert!(!name.is_empty());
            assert!(
                name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{name:?} is not a snake_case identifier"
            );
        }
    }

    #[test]
    fn only_per_file_events_are_classified_high_frequency() {
        // Misclassifying a rare event as high frequency would bury it at TRACE;
        // misclassifying a per-file event as rare would flood an INFO log.
        for event in every_variant() {
            let expected = matches!(
                event,
                CrawlEvent::FileDiscovered { .. }
                    | CrawlEvent::FileMatched { .. }
                    | CrawlEvent::FileSkipped { .. }
                    | CrawlEvent::FileProcessingStarted { .. }
                    | CrawlEvent::FileProcessingCompleted { .. }
                    | CrawlEvent::RecordAccepted { .. }
            );
            assert_eq!(
                event.is_high_frequency(),
                expected,
                "{} is misclassified",
                event.name()
            );
        }
    }

    #[test]
    fn exactly_the_failure_events_expose_an_error() {
        let carries_error = [
            "file_processing_failed",
            "file_processing_retried",
            "record_rejected",
            "protocol_violation_detected",
            "traversal_error_detected",
        ];
        for event in every_variant() {
            assert_eq!(
                event.error().is_some(),
                carries_error.contains(&event.name()),
                "{} exposes the wrong error attribution",
                event.name()
            );
        }
    }

    #[test]
    fn exposed_errors_carry_the_originating_category() {
        let event = CrawlEvent::FileProcessingFailed {
            error: Box::new(ErrorEvent::new(ErrorCategory::Timeout, "deadline")),
            attempts: 2,
        };
        assert_eq!(
            event.error().expect("error").category,
            ErrorCategory::Timeout
        );

        let rejected = CrawlEvent::RecordRejected {
            violation: Box::new(SchemaViolationEvent::new(
                "bad type",
                3,
                Some("line".into()),
            )),
        };
        // A rejected record always reports the schema category, whatever the
        // underlying violation was.
        assert_eq!(
            rejected.error().expect("error").category,
            ErrorCategory::Schema
        );
    }
}
