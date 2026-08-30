//! Infrastructure adapters for the crawl host.
//!
//! Every module here implements a port declared by `crawl-application`. This is
//! the only crate that knows about Tokio processes, the filesystem, the `csv`
//! crate, and `tracing`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod filesystem;
pub mod logging;
pub mod registry;
pub mod reports;
pub mod workers;

pub use filesystem::directory_walker::FilesystemWalker;
pub use logging::tracing_sink::TracingEventSink;
pub use registry::file_registry::FileRegistry;
pub use reports::csv_writer::CsvReportWriter;
pub use workers::process::ProcessWorkerFactory;
