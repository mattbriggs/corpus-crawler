//! Pure domain model for the `crawl` file-processing runtime.
//!
//! # Purpose
//!
//! This crate owns the vocabulary of the system: validated value objects, the
//! plugin manifest contract, the output-schema language, records, file tasks,
//! crawl state, statistics, events, the error taxonomy, and execution policies.
//!
//! # Caller contract
//!
//! Everything in this crate is deterministic and synchronous. The crate has no
//! knowledge of the CLI, Tokio, subprocesses, CSV serialization, filesystem
//! traversal, or registry persistence. Those concerns belong to
//! `crawl-application` (orchestration) and `crawl-infrastructure` (adapters).
//!
//! # Extension points
//!
//! Behaviour that the SRS deliberately leaves open is modelled as an explicit
//! policy value object in [`policy`] rather than being hard-coded into the
//! engine, so a decision can be changed without restructuring the pipeline.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod crawl;
pub mod errors;
pub mod events;
pub mod ids;
pub mod manifest;
pub mod plugin;
pub mod policy;
pub mod record;
pub mod schema;
pub mod statistics;
pub mod task;

pub use crawl::{CrawlOutcome, CrawlState};
pub use errors::{ErrorCategory, ErrorEvent};
pub use events::CrawlEvent;
pub use ids::{ApiVersion, CrawlId, PluginId, PluginName, PluginVersion, RequestId, WorkerId};
pub use plugin::{ExecutionDefaults, InputSpec, Plugin, RuntimeSpec};
pub use record::{RawRecord, RawValue, ScalarValue, ValidatedRecord};
pub use schema::{FieldType, OutputSchema, SchemaField, SchemaViolation};
pub use statistics::CrawlStatistics;
pub use task::{FileTask, FileTaskState};

/// The single host-plugin API version supported by this release.
///
/// Compatibility between an installed plugin and this host is governed by the
/// manifest's `api_version` field (REQ-088, ARC-004, NFR-032).
pub const SUPPORTED_API_VERSION: &str = "1";
