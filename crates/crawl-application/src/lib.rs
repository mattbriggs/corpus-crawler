//! Application layer: use cases, ports, and orchestration.
//!
//! This crate depends on [`crawl_domain`] and [`crawl_protocol`] but never on
//! infrastructure. It states *what* the host does through ports; adapters in
//! `crawl-infrastructure` decide *how*.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod commands;
pub mod ports;
pub mod services;

pub use services::configuration::{ConfigurationError, EffectiveConfig, RunOverrides};
pub use services::crawl_coordinator::{CrawlCoordinator, CrawlReport, HostFatalError};
