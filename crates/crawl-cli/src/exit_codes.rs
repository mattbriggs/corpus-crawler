//! Process exit statuses (REQ-130 - REQ-134, NFR-053, OQ-025).
//!
//! # Decision
//!
//! | Code | Meaning |
//! |-----:|---------|
//! | 0 | success |
//! | 1 | partial success: the crawl finished with file-level errors |
//! | 2 | configuration, usage, or preflight failure |
//! | 3 | cancelled |
//! | 4 | plugin or runtime failure |
//! | 5 | fatal host failure |
//!
//! `2` is deliberately the configuration code because `clap` already exits
//! with `2` on a usage error, so argument errors and preflight errors agree
//! without special-casing.

use crawl_domain::crawl::CrawlOutcome;

/// Successful crawl.
pub const SUCCESS: i32 = 0;
/// Crawl completed with file- or plugin-level errors.
pub const PARTIAL_SUCCESS: i32 = 1;
/// Configuration, usage, or preflight failure.
pub const CONFIGURATION_FAILURE: i32 = 2;
/// Crawl cancelled by the operator.
pub const CANCELLED: i32 = 3;
/// Plugin runtime could not be started or kept running.
pub const PLUGIN_FAILURE: i32 = 4;
/// Unrecoverable host failure.
pub const HOST_FAILURE: i32 = 5;

/// Maps a crawl outcome to its process exit status.
#[must_use]
pub const fn for_outcome(outcome: CrawlOutcome) -> i32 {
    match outcome {
        CrawlOutcome::Success => SUCCESS,
        CrawlOutcome::PartialSuccess => PARTIAL_SUCCESS,
        CrawlOutcome::ConfigurationFailure => CONFIGURATION_FAILURE,
        CrawlOutcome::Cancelled => CANCELLED,
        CrawlOutcome::PluginFailure => PLUGIN_FAILURE,
        CrawlOutcome::HostFailure => HOST_FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_outcome_maps_to_a_distinct_code() {
        let codes = [
            for_outcome(CrawlOutcome::Success),
            for_outcome(CrawlOutcome::PartialSuccess),
            for_outcome(CrawlOutcome::ConfigurationFailure),
            for_outcome(CrawlOutcome::Cancelled),
            for_outcome(CrawlOutcome::PluginFailure),
            for_outcome(CrawlOutcome::HostFailure),
        ];
        let mut sorted = codes.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), codes.len());
    }
}
