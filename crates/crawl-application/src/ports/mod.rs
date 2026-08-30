//! Ports: the interfaces infrastructure adapters implement.

pub mod event_sink;
pub mod registry;
pub mod report;
pub mod walker;
pub mod worker;

pub use event_sink::EventSink;
pub use registry::{PluginRegistry, RegistryError};
pub use report::{ReportError, ReportWriter};
pub use walker::{DirectoryWalker, Discovery};
pub use worker::{WorkerFactory, WorkerFailure, WorkerHandle, WorkerStartupError};
