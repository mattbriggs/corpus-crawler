//! Crawl counters and the summary contract (REQ-046, REQ-116 - REQ-118).

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::errors::ErrorCategory;

/// Aggregate counters for one crawl.
///
/// # Caller contract
///
/// A single owner mutates these counters. In the running host that owner is
/// the result-aggregation task, which keeps the counts consistent without
/// atomics scattered across the pipeline.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CrawlStatistics {
    /// Filesystem entries examined during traversal, including directories.
    pub entries_examined: u64,
    /// Regular files observed during traversal.
    pub files_discovered: u64,
    /// Files that matched the effective extension configuration.
    pub files_matched: u64,
    /// Files that reached a terminal successful outcome.
    pub files_completed: u64,
    /// Files that reached a terminal failed outcome.
    pub files_failed: u64,
    /// Schema-valid records written to the report.
    pub records_emitted: u64,
    /// Records rejected by schema validation.
    pub records_rejected: u64,
    /// Per-file deadlines exceeded, counted per attempt.
    pub timeouts: u64,
    /// Replacement workers started after a failure.
    pub worker_restarts: u64,
    /// Retried processing attempts, counted per retry.
    pub retries: u64,
    /// Error counts by category.
    pub errors_by_category: BTreeMap<ErrorCategory, u64>,
    /// Wall-clock crawl duration in seconds, set at finalization.
    pub duration_seconds: f64,
}

impl CrawlStatistics {
    /// Creates zeroed statistics.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one error occurrence in its category.
    pub fn record_error(&mut self, category: ErrorCategory) {
        *self.errors_by_category.entry(category).or_insert(0) += 1;
    }

    /// Returns the total number of recorded errors across all categories.
    #[must_use]
    pub fn total_errors(&self) -> u64 {
        self.errors_by_category.values().sum()
    }

    /// Returns the number of errors recorded in one category.
    #[must_use]
    pub fn errors_in(&self, category: ErrorCategory) -> u64 {
        self.errors_by_category.get(&category).copied().unwrap_or(0)
    }

    /// Returns the number of errors that consume the configured error budget
    /// (OQ-020).
    #[must_use]
    pub fn qualifying_errors(&self) -> u64 {
        self.errors_by_category
            .iter()
            .filter(|(category, _)| {
                crate::policy::FailurePolicy::counts_toward_threshold(**category)
            })
            .map(|(_, count)| *count)
            .sum()
    }

    /// Returns `true` when nothing failed in a way that should downgrade the
    /// crawl outcome to partial success.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.qualifying_errors() == 0
    }

    /// Sets the measured crawl duration.
    pub fn set_duration(&mut self, duration: Duration) {
        self.duration_seconds = duration.as_secs_f64();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_counts_accumulate_per_category() {
        let mut stats = CrawlStatistics::new();
        stats.record_error(ErrorCategory::Timeout);
        stats.record_error(ErrorCategory::Timeout);
        stats.record_error(ErrorCategory::Schema);
        assert_eq!(stats.errors_in(ErrorCategory::Timeout), 2);
        assert_eq!(stats.errors_in(ErrorCategory::Schema), 1);
        assert_eq!(stats.errors_in(ErrorCategory::Protocol), 0);
        assert_eq!(stats.total_errors(), 3);
    }

    #[test]
    fn preflight_errors_do_not_make_a_crawl_dirty() {
        let mut stats = CrawlStatistics::new();
        stats.record_error(ErrorCategory::Configuration);
        assert!(stats.is_clean());
        stats.record_error(ErrorCategory::PluginProcessing);
        assert!(!stats.is_clean());
        assert_eq!(stats.qualifying_errors(), 1);
    }

    #[test]
    fn duration_is_recorded_in_seconds() {
        let mut stats = CrawlStatistics::new();
        stats.set_duration(Duration::from_millis(2500));
        assert!((stats.duration_seconds - 2.5).abs() < f64::EPSILON);
    }
}
