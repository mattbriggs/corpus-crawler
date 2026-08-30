//! Observability port (REQ-110 - REQ-118).

use crawl_domain::events::CrawlEvent;

/// Receives every operational event the pipeline produces.
///
/// Implementations must be cheap and non-blocking: the sink is called from the
/// hot path once per discovered file.
pub trait EventSink: Send + Sync {
    /// Records one event.
    fn emit(&self, event: &CrawlEvent);
}
