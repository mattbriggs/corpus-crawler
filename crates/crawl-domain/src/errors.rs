//! The error taxonomy (SRS 8.7).
//!
//! Every externally observable failure maps to exactly one [`ErrorCategory`].
//! The category drives four things at once: the structured log event, whether
//! the failure is retryable, whether it consumes the error budget, and the
//! process exit outcome. Keeping that mapping in one enum is what stops those
//! four decisions from drifting apart.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ids::{CrawlId, PluginId, RequestId, WorkerId};

/// The error domains the system distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    /// Invalid CLI option, unresolvable plugin, unusable output path.
    Configuration,
    /// Unsupported API version or malformed manifest/schema.
    Manifest,
    /// Plugin registry could not be read or written.
    Registry,
    /// The worker runtime could not be started or failed its handshake.
    RuntimeStartup,
    /// The plugin module or callable could not be loaded.
    PluginLoad,
    /// A directory entry could not be traversed.
    Traversal,
    /// A discovered file could not be opened by the plugin.
    FileAccess,
    /// The plugin reported a processing failure for a file.
    PluginProcessing,
    /// A per-file deadline was exceeded.
    Timeout,
    /// A worker process exited unexpectedly.
    WorkerCrash,
    /// A worker message could not be interpreted as the supported protocol.
    Protocol,
    /// A record violated the declared output schema.
    Schema,
    /// The CSV destination could not be written or finalized.
    Report,
    /// The crawl was cancelled by the operator.
    Cancellation,
    /// An unrecoverable internal host failure.
    HostFatal,
}

impl ErrorCategory {
    /// Returns the stable snake_case identifier used in logs and summaries.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::Manifest => "manifest",
            Self::Registry => "registry",
            Self::RuntimeStartup => "runtime_startup",
            Self::PluginLoad => "plugin_load",
            Self::Traversal => "traversal",
            Self::FileAccess => "file_access",
            Self::PluginProcessing => "plugin_processing",
            Self::Timeout => "timeout",
            Self::WorkerCrash => "worker_crash",
            Self::Protocol => "protocol",
            Self::Schema => "schema",
            Self::Report => "report",
            Self::Cancellation => "cancellation",
            Self::HostFatal => "host_fatal",
        }
    }

    /// Every category, in declaration order.
    ///
    /// Used to render a complete per-category breakdown in the summary so a
    /// large crawl cannot hide an important failure class behind one aggregate
    /// count (REQ-118).
    pub const ALL: [Self; 15] = [
        Self::Configuration,
        Self::Manifest,
        Self::Registry,
        Self::RuntimeStartup,
        Self::PluginLoad,
        Self::Traversal,
        Self::FileAccess,
        Self::PluginProcessing,
        Self::Timeout,
        Self::WorkerCrash,
        Self::Protocol,
        Self::Schema,
        Self::Report,
        Self::Cancellation,
        Self::HostFatal,
    ];

    /// Returns `true` when the category can only arise before any file is
    /// scheduled, and therefore fails preflight rather than the crawl.
    #[must_use]
    pub const fn is_preflight(self) -> bool {
        matches!(
            self,
            Self::Configuration | Self::Manifest | Self::Registry | Self::PluginLoad
        )
    }
}

impl fmt::Display for ErrorCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A structured, attributable error occurrence (VAL-060).
///
/// Fields that are unknown or irrelevant to the category are left `None`
/// rather than filled with placeholders, so a log consumer can tell "not
/// applicable" from "unknown".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorEvent {
    /// The error domain.
    pub category: ErrorCategory,
    /// Human-readable description of what failed.
    pub message: String,
    /// The crawl in which the error occurred, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crawl: Option<CrawlId>,
    /// The plugin involved, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin: Option<PluginId>,
    /// The affected file, when the error is attributable to one (NFR-051).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    /// The worker involved, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker: Option<WorkerId>,
    /// The request involved, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<RequestId>,
    /// Plugin-supplied error code, for plugin-declared failures.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl ErrorEvent {
    /// Creates an error event carrying only a category and message.
    #[must_use]
    pub fn new(category: ErrorCategory, message: impl Into<String>) -> Self {
        Self {
            category,
            message: message.into(),
            crawl: None,
            plugin: None,
            file: None,
            worker: None,
            request: None,
            code: None,
        }
    }

    /// Attributes the event to a crawl.
    #[must_use]
    pub fn with_crawl(mut self, crawl: CrawlId) -> Self {
        self.crawl = Some(crawl);
        self
    }

    /// Attributes the event to a plugin.
    #[must_use]
    pub fn with_plugin(mut self, plugin: PluginId) -> Self {
        self.plugin = Some(plugin);
        self
    }

    /// Attributes the event to a file.
    #[must_use]
    pub fn with_file(mut self, file: impl Into<PathBuf>) -> Self {
        self.file = Some(file.into());
        self
    }

    /// Attributes the event to a worker.
    #[must_use]
    pub fn with_worker(mut self, worker: WorkerId) -> Self {
        self.worker = Some(worker);
        self
    }

    /// Attributes the event to a request.
    #[must_use]
    pub fn with_request(mut self, request: RequestId) -> Self {
        self.request = Some(request);
        self
    }

    /// Attaches a plugin-supplied error code.
    #[must_use]
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }
}

impl fmt::Display for ErrorEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.category, self.message)?;
        if let Some(file) = &self.file {
            write!(f, " (file: {})", file.display())?;
        }
        Ok(())
    }
}

/// A rejected output record, with enough context to fix the plugin (REQ-097).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchemaViolationEvent {
    /// The underlying attributable error, always of category
    /// [`ErrorCategory::Schema`].
    pub error: ErrorEvent,
    /// Zero-based index of the rejected row within its response.
    pub row_index: usize,
    /// The offending field name, when the violation names one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

impl SchemaViolationEvent {
    /// Creates a schema-violation event for one rejected row.
    #[must_use]
    pub fn new(message: impl Into<String>, row_index: usize, field: Option<String>) -> Self {
        Self {
            error: ErrorEvent::new(ErrorCategory::Schema, message),
            row_index,
            field,
        }
    }

    /// Attributes the violation to a file.
    #[must_use]
    pub fn with_file(mut self, file: impl Into<PathBuf>) -> Self {
        self.error = self.error.with_file(file);
        self
    }

    /// Attributes the violation to a request.
    #[must_use]
    pub fn with_request(mut self, request: RequestId) -> Self {
        self.error = self.error.with_request(request);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_category_has_a_unique_stable_name() {
        let mut names: Vec<&str> = ErrorCategory::ALL.iter().map(|c| c.as_str()).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total);
    }

    #[test]
    fn preflight_categories_are_marked() {
        assert!(ErrorCategory::Configuration.is_preflight());
        assert!(ErrorCategory::Manifest.is_preflight());
        assert!(!ErrorCategory::Timeout.is_preflight());
    }

    #[test]
    fn attribution_builders_populate_known_fields_only() {
        let event = ErrorEvent::new(ErrorCategory::Timeout, "deadline exceeded")
            .with_file("/data/a.txt")
            .with_request(RequestId::new(9));
        assert_eq!(
            event.file.as_deref(),
            Some(std::path::Path::new("/data/a.txt"))
        );
        assert_eq!(event.request, Some(RequestId::new(9)));
        assert!(event.worker.is_none());
        assert!(event.to_string().contains("/data/a.txt"));
    }

    #[test]
    fn schema_violation_events_carry_row_and_field() {
        let violation = SchemaViolationEvent::new("bad type", 2, Some("line".into()))
            .with_file("/data/a.txt")
            .with_request(RequestId::new(4));
        assert_eq!(violation.error.category, ErrorCategory::Schema);
        assert_eq!(violation.row_index, 2);
        assert_eq!(violation.field.as_deref(), Some("line"));
        assert!(violation.error.file.is_some());
    }
}
