//! Execution policies and their value objects.
//!
//! # Purpose
//!
//! The SRS leaves several operational behaviours open (OQ-015 through OQ-020).
//! Each is modelled here as an explicit, testable policy so the decision lives
//! in one place and can be revised without touching the crawl engine.

use std::fmt;
use std::num::NonZeroU32;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::errors::ErrorCategory;

/// Errors produced when an execution value object rejects its input.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PolicyError {
    /// A count or capacity was zero where a positive value is required.
    #[error("{setting} must be greater than zero")]
    NotPositive {
        /// The rejected setting name.
        setting: &'static str,
    },
    /// A duration was zero, negative, or not finite.
    #[error("{setting} must be a finite value greater than zero, got {value}")]
    InvalidDuration {
        /// The rejected setting name.
        setting: &'static str,
        /// The rejected value.
        value: f64,
    },
}

/// Number of concurrently active plugin workers (REQ-052, REQ-053, VAL-008).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WorkerCount {
    /// Resolve the count from the host's automatic policy.
    Auto(AutoMarker),
    /// Use exactly this many workers.
    Fixed(NonZeroU32),
}

/// Serialization marker for `workers: auto`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutoMarker {
    /// The literal string `auto`.
    Auto,
}

impl WorkerCount {
    /// The automatic worker-count mode.
    pub const AUTO: Self = Self::Auto(AutoMarker::Auto);

    /// Constructs a fixed worker count.
    ///
    /// # Errors
    ///
    /// Returns [`PolicyError::NotPositive`] when `count` is zero (VAL-008).
    pub fn fixed(count: u32) -> Result<Self, PolicyError> {
        NonZeroU32::new(count)
            .map(Self::Fixed)
            .ok_or(PolicyError::NotPositive { setting: "workers" })
    }

    /// Resolves the effective worker count.
    ///
    /// # Automatic policy (OQ-018)
    ///
    /// `auto` resolves to the number of available CPUs, clamped to
    /// [`Self::AUTO_MIN`]..=[`Self::AUTO_MAX`]. The SRS requires this to be
    /// treated as a benchmarked implementation policy rather than a
    /// requirement, so it is a pure function of `available_parallelism` and is
    /// unit-testable without spawning anything.
    #[must_use]
    pub fn resolve(self, available_parallelism: u32) -> NonZeroU32 {
        match self {
            Self::Fixed(count) => count,
            Self::Auto(_) => {
                let clamped = available_parallelism.clamp(Self::AUTO_MIN, Self::AUTO_MAX);
                NonZeroU32::new(clamped).unwrap_or(NonZeroU32::MIN)
            }
        }
    }

    /// Lower bound applied to the automatic worker count.
    pub const AUTO_MIN: u32 = 1;
    /// Upper bound applied to the automatic worker count.
    pub const AUTO_MAX: u32 = 32;
}

impl fmt::Display for WorkerCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto(_) => f.write_str("auto"),
            Self::Fixed(count) => write!(f, "{count}"),
        }
    }
}

/// Depth of the bounded work queue (REQ-050, REQ-054, NFR-014).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct QueueCapacity(NonZeroU32);

impl QueueCapacity {
    /// Host default when neither the CLI nor the manifest specifies a depth.
    ///
    /// Chosen to keep discovery slightly ahead of the pool without letting
    /// queued paths dominate memory (NFR-021).
    pub const DEFAULT: u32 = 1024;

    /// Constructs a queue capacity.
    ///
    /// # Errors
    ///
    /// Returns [`PolicyError::NotPositive`] when `capacity` is zero.
    pub fn new(capacity: u32) -> Result<Self, PolicyError> {
        NonZeroU32::new(capacity)
            .map(Self)
            .ok_or(PolicyError::NotPositive {
                setting: "queue capacity",
            })
    }

    /// Returns the capacity as a `usize` for channel construction.
    #[must_use]
    pub fn get(self) -> usize {
        self.0.get() as usize
    }
}

impl Default for QueueCapacity {
    fn default() -> Self {
        Self(NonZeroU32::new(Self::DEFAULT).expect("non-zero default"))
    }
}

impl fmt::Display for QueueCapacity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// Per-file processing deadline (REQ-070, VAL-009).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timeout(f64);

impl Timeout {
    /// Constructs a timeout from a number of seconds.
    ///
    /// # Errors
    ///
    /// Returns [`PolicyError::InvalidDuration`] when the value is not a finite
    /// number greater than zero.
    pub fn from_seconds(seconds: f64) -> Result<Self, PolicyError> {
        if !seconds.is_finite() || seconds <= 0.0 {
            return Err(PolicyError::InvalidDuration {
                setting: "timeout_seconds",
                value: seconds,
            });
        }
        Ok(Self(seconds))
    }

    /// Returns the deadline as a [`Duration`].
    #[must_use]
    pub fn duration(self) -> Duration {
        Duration::from_secs_f64(self.0)
    }

    /// Returns the configured number of seconds.
    #[must_use]
    pub fn seconds(self) -> f64 {
        self.0
    }
}

impl fmt::Display for Timeout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}s", self.0)
    }
}

/// Bounded retry allowance for one file task (REQ-072, REQ-073, VAL-010).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RetryLimit(u32);

impl RetryLimit {
    /// Constructs a retry limit. Zero means "no retries".
    #[must_use]
    pub const fn new(max_retries: u32) -> Self {
        Self(max_retries)
    }

    /// Returns the maximum number of *additional* attempts after the first.
    #[must_use]
    pub const fn max_retries(self) -> u32 {
        self.0
    }

    /// Returns the total permitted attempts, `1 + max_retries`.
    ///
    /// This is the invariant the scheduler enforces, which is why it is
    /// expressed once here rather than recomputed at each call site.
    #[must_use]
    pub const fn max_attempts(self) -> u32 {
        self.0 + 1
    }
}

impl fmt::Display for RetryLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// Decides which failure classes may be retried (OQ-016).
///
/// # Decision
///
/// Retries are limited to failures that are plausibly transient *and* whose
/// side effects the host controls:
///
/// * worker crashes and timeouts are retried on a fresh worker, because the
///   failure is attributable to worker state rather than to the file;
/// * protocol violations are retried, because a desynchronised stream is a
///   worker-level fault and the replacement worker starts a clean stream;
/// * plugin-declared processing errors, file-access errors, and schema errors
///   are **not** retried, because the plugin has given a deterministic answer
///   about that file and repeating it would only multiply the same failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RetryPolicy {
    limit: RetryLimit,
}

impl RetryPolicy {
    /// Creates a retry policy with the given bound.
    #[must_use]
    pub const fn new(limit: RetryLimit) -> Self {
        Self { limit }
    }

    /// Returns the configured limit.
    #[must_use]
    pub const fn limit(self) -> RetryLimit {
        self.limit
    }

    /// Returns `true` when this error class is eligible for retry at all.
    #[must_use]
    pub const fn is_retryable(self, category: ErrorCategory) -> bool {
        matches!(
            category,
            ErrorCategory::WorkerCrash | ErrorCategory::Timeout | ErrorCategory::Protocol
        )
    }

    /// Returns `true` when another attempt is permitted.
    ///
    /// `attempts_made` counts attempts already completed, so the guarantee
    /// `attempts <= 1 + max_retries` follows directly.
    #[must_use]
    pub const fn should_retry(self, category: ErrorCategory, attempts_made: u32) -> bool {
        self.is_retryable(category) && attempts_made < self.limit.max_attempts()
    }
}

/// Failure-handling policy for the crawl as a whole (REQ-120 - REQ-124).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FailurePolicy {
    /// Stop scheduling as soon as one qualifying error is recorded (REQ-122).
    pub fail_fast: bool,
    /// Optional error budget; `None` means unlimited (REQ-123).
    pub max_errors: Option<u64>,
}

impl FailurePolicy {
    /// Creates a failure policy.
    #[must_use]
    pub const fn new(fail_fast: bool, max_errors: Option<u64>) -> Self {
        Self {
            fail_fast,
            max_errors,
        }
    }

    /// Returns `true` when this category counts toward the error budget
    /// (OQ-020).
    ///
    /// # Decision
    ///
    /// Every error the host attributes to a running crawl counts: traversal
    /// errors, terminal file failures (plugin error, timeout, worker crash),
    /// failures to start a replacement worker, protocol violations,
    /// schema-invalid rows, and rows that could not be written.
    ///
    /// Two groups are excluded. Preflight categories - configuration,
    /// manifest, registry, plugin load - cannot occur once files are being
    /// scheduled, because they fail the crawl before it starts. Cancellation
    /// and host-fatal outcomes are excluded because they already determine the
    /// outcome on their own; counting them would double-report.
    ///
    /// Counting rejected rows is deliberate: a plugin emitting thousands of
    /// invalid rows is a contract failure the operator should hear about
    /// early. Counting report errors is equally deliberate: a crawl that
    /// silently failed to write its rows must never be reported as a success.
    #[must_use]
    pub const fn counts_toward_threshold(category: ErrorCategory) -> bool {
        matches!(
            category,
            ErrorCategory::Traversal
                | ErrorCategory::FileAccess
                | ErrorCategory::RuntimeStartup
                | ErrorCategory::PluginProcessing
                | ErrorCategory::Timeout
                | ErrorCategory::WorkerCrash
                | ErrorCategory::Protocol
                | ErrorCategory::Schema
                | ErrorCategory::Report
        )
    }

    /// Returns `true` when the crawl must stop scheduling new work.
    #[must_use]
    pub fn should_stop(self, qualifying_errors: u64) -> bool {
        if qualifying_errors == 0 {
            return false;
        }
        if self.fail_fast {
            return true;
        }
        self.max_errors
            .is_some_and(|threshold| qualifying_errors >= threshold)
    }
}

/// Disposition of the partially written report when a crawl does not finish
/// normally (OQ-017, REQ-183).
///
/// # Decision
///
/// The writer always streams into a sibling temporary file and promotes it to
/// the requested path only when finalization succeeds. Cancellation and
/// error-threshold shutdown are *orderly* terminations, so their partial output
/// is promoted and the operator is told the report is partial. A host-fatal
/// failure leaves the temporary file in place instead of publishing a report
/// that no one has vouched for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartialOutputPolicy {
    /// Promote the temporary file to the requested output path.
    Promote,
    /// Leave the temporary file unpromoted and report its location.
    RetainTemporary,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_count_rejects_zero_and_resolves_auto() {
        assert_eq!(
            WorkerCount::fixed(0).unwrap_err(),
            PolicyError::NotPositive { setting: "workers" },
            "a zero worker count must name the setting it rejected (VAL-008)"
        );
        assert_eq!(WorkerCount::fixed(4).unwrap().resolve(64).get(), 4);
        assert_eq!(WorkerCount::AUTO.resolve(8).get(), 8);
        assert_eq!(WorkerCount::AUTO.resolve(0).get(), WorkerCount::AUTO_MIN);
        assert_eq!(WorkerCount::AUTO.resolve(1024).get(), WorkerCount::AUTO_MAX);
    }

    #[test]
    fn queue_capacity_rejects_zero() {
        assert_eq!(
            QueueCapacity::new(0).unwrap_err(),
            PolicyError::NotPositive {
                setting: "queue capacity"
            }
        );
        assert_eq!(QueueCapacity::new(16).unwrap().get(), 16);
        assert_eq!(
            QueueCapacity::default().get(),
            QueueCapacity::DEFAULT as usize
        );
    }

    #[test]
    fn timeout_rejects_non_positive_and_non_finite() {
        // Each rejected value must be reported as the timeout setting, and the
        // rejected value must be echoed back so the operator can see what the
        // host actually parsed (NFR-050).
        for rejected in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            match Timeout::from_seconds(rejected) {
                Err(PolicyError::InvalidDuration { setting, value }) => {
                    assert_eq!(setting, "timeout_seconds");
                    assert!(
                        value == rejected || (value.is_nan() && rejected.is_nan()),
                        "the rejected value was not echoed back"
                    );
                }
                other => panic!("{rejected} should be rejected, got {other:?}"),
            }
        }

        let timeout = Timeout::from_seconds(1.5).expect("positive and finite");
        assert_eq!(timeout.duration(), Duration::from_millis(1500));
        assert_eq!(timeout.seconds(), 1.5);
    }

    #[test]
    fn attempts_are_bounded_by_one_plus_max_retries() {
        let policy = RetryPolicy::new(RetryLimit::new(2));
        assert_eq!(policy.limit().max_attempts(), 3);
        assert!(policy.should_retry(ErrorCategory::Timeout, 1));
        assert!(policy.should_retry(ErrorCategory::Timeout, 2));
        assert!(!policy.should_retry(ErrorCategory::Timeout, 3));
    }

    #[test]
    fn zero_retries_never_retries() {
        let policy = RetryPolicy::new(RetryLimit::new(0));
        assert!(!policy.should_retry(ErrorCategory::WorkerCrash, 1));
    }

    #[test]
    fn deterministic_failures_are_not_retried() {
        let policy = RetryPolicy::new(RetryLimit::new(5));
        assert!(policy.is_retryable(ErrorCategory::WorkerCrash));
        assert!(policy.is_retryable(ErrorCategory::Timeout));
        assert!(policy.is_retryable(ErrorCategory::Protocol));
        assert!(!policy.is_retryable(ErrorCategory::PluginProcessing));
        assert!(!policy.is_retryable(ErrorCategory::Schema));
        assert!(!policy.is_retryable(ErrorCategory::FileAccess));
    }

    #[test]
    fn fail_fast_stops_on_first_error() {
        let policy = FailurePolicy::new(true, None);
        assert!(!policy.should_stop(0));
        assert!(policy.should_stop(1));
    }

    #[test]
    fn threshold_stops_on_nth_error() {
        let policy = FailurePolicy::new(false, Some(10));
        assert!(!policy.should_stop(9));
        assert!(policy.should_stop(10));
        assert!(policy.should_stop(11));
    }

    #[test]
    fn unlimited_policy_never_stops() {
        let policy = FailurePolicy::default();
        assert!(!policy.should_stop(u64::MAX));
    }

    #[test]
    fn preflight_categories_do_not_consume_the_error_budget() {
        assert!(!FailurePolicy::counts_toward_threshold(
            ErrorCategory::Configuration
        ));
        assert!(!FailurePolicy::counts_toward_threshold(
            ErrorCategory::Manifest
        ));
        assert!(FailurePolicy::counts_toward_threshold(
            ErrorCategory::Schema
        ));
        assert!(FailurePolicy::counts_toward_threshold(
            ErrorCategory::Traversal
        ));
    }
}
