//! Report output port (REQ-100 - REQ-107).

use crawl_domain::policy::PartialOutputPolicy;
use crawl_domain::record::ValidatedRecord;

/// Failure to write or finalize the report.
#[derive(Debug, thiserror::Error)]
#[error("cannot write report: {0}")]
pub struct ReportError(pub String);

/// The single host-owned component that serializes the report (REQ-102).
///
/// Only validated records cross this boundary (ARC-008).
pub trait ReportWriter: Send {
    /// Writes one validated record in schema order.
    ///
    /// # Errors
    /// Returns [`ReportError`] when the destination cannot be written.
    fn write(&mut self, record: &ValidatedRecord) -> Result<(), ReportError>;

    /// Flushes and closes the report, applying the partial-output policy.
    ///
    /// # Errors
    /// Returns [`ReportError`] when the report cannot be finalized.
    fn finalize(&mut self, policy: PartialOutputPolicy) -> Result<(), ReportError>;
}
