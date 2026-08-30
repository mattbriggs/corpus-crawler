//! The crawl state machine and completion outcomes (SRS 7.6, REQ-130 - REQ-134).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::errors::ErrorCategory;
use crate::statistics::CrawlStatistics;

/// Lifecycle state of one `crawl run` execution.
///
/// Transitions are performed exclusively through [`CrawlState::transition`],
/// so an illegal transition is a rejected value rather than a silent state
/// corruption caused by scattered boolean flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrawlState {
    /// The job exists but nothing has been validated yet.
    Created,
    /// Preflight validation is running (REQ-032).
    Validating,
    /// Discovery, scheduling, and processing are active.
    Running,
    /// Operator cancellation has been accepted (REQ-125).
    Cancelling,
    /// Fail-fast or the error threshold has stopped scheduling (REQ-124).
    Stopping,
    /// Work is finished; workers and the report are being closed down.
    Finalizing,
    /// The crawl finished with no qualifying errors.
    Completed,
    /// The crawl finished with at least one qualifying error (REQ-133).
    CompletedWithErrors,
    /// The crawl ended because of cancellation.
    Cancelled,
    /// Preflight, or the crawl engine itself, failed unrecoverably.
    Failed,
}

/// Rejected state transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("illegal crawl state transition from {from:?} to {to:?}")]
pub struct IllegalTransition {
    /// The current state.
    pub from: CrawlState,
    /// The state that was requested.
    pub to: CrawlState,
}

impl CrawlState {
    /// Returns `true` when the state admits no further transition.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::CompletedWithErrors | Self::Cancelled | Self::Failed
        )
    }

    /// Returns `true` when new file tasks may still be scheduled (REQ-126).
    #[must_use]
    pub const fn accepts_new_work(self) -> bool {
        matches!(self, Self::Running)
    }

    /// Returns `true` when the requested transition is legal.
    #[must_use]
    pub const fn can_transition_to(self, to: Self) -> bool {
        matches!(
            (self, to),
            (Self::Created, Self::Validating)
                | (Self::Validating, Self::Running | Self::Failed)
                | (
                    Self::Running,
                    Self::Cancelling | Self::Stopping | Self::Finalizing | Self::Failed
                )
                | (Self::Cancelling, Self::Finalizing | Self::Failed)
                | (Self::Stopping, Self::Finalizing | Self::Failed)
                | (
                    Self::Finalizing,
                    Self::Completed | Self::CompletedWithErrors | Self::Cancelled | Self::Failed
                )
        )
    }

    /// Performs a transition.
    ///
    /// # Errors
    ///
    /// Returns [`IllegalTransition`] when the transition is not permitted by
    /// the state model.
    pub fn transition(self, to: Self) -> Result<Self, IllegalTransition> {
        if self.can_transition_to(to) {
            Ok(to)
        } else {
            Err(IllegalTransition { from: self, to })
        }
    }
}

impl fmt::Display for CrawlState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Created => "created",
            Self::Validating => "validating",
            Self::Running => "running",
            Self::Cancelling => "cancelling",
            Self::Stopping => "stopping",
            Self::Finalizing => "finalizing",
            Self::Completed => "completed",
            Self::CompletedWithErrors => "completed_with_errors",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        };
        f.write_str(text)
    }
}

/// Materially different completion outcomes (REQ-130).
///
/// The numeric exit-code mapping lives in the CLI crate; the domain only says
/// which outcomes must be distinguishable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrawlOutcome {
    /// The crawl completed with no qualifying errors (REQ-132).
    Success,
    /// The crawl completed with file- or plugin-level errors (REQ-133).
    PartialSuccess,
    /// Preflight or configuration validation failed (REQ-131).
    ConfigurationFailure,
    /// The crawl was cancelled by the operator.
    Cancelled,
    /// The plugin runtime could not be started or kept running.
    PluginFailure,
    /// The crawl engine failed unrecoverably (REQ-134).
    HostFailure,
}

impl CrawlOutcome {
    /// Derives the outcome of a finished crawl from its terminating state and
    /// counters.
    ///
    /// # Caller contract
    ///
    /// `state` must be the state the crawl reached *before* finalization, that
    /// is one of `Running`, `Cancelling`, or `Stopping`. Any other state means
    /// the crawl never reached finalization and the outcome is a host failure.
    #[must_use]
    pub fn derive(state: CrawlState, statistics: &CrawlStatistics) -> Self {
        match state {
            CrawlState::Cancelling | CrawlState::Cancelled => Self::Cancelled,
            CrawlState::Running | CrawlState::Stopping | CrawlState::Finalizing => {
                if statistics.is_clean() {
                    Self::Success
                } else {
                    Self::PartialSuccess
                }
            }
            CrawlState::Created | CrawlState::Validating => Self::ConfigurationFailure,
            CrawlState::Completed => Self::Success,
            CrawlState::CompletedWithErrors => Self::PartialSuccess,
            CrawlState::Failed => Self::HostFailure,
        }
    }

    /// Maps an error category to the outcome a preflight or fatal failure of
    /// that category produces.
    #[must_use]
    pub const fn from_fatal_category(category: ErrorCategory) -> Self {
        match category {
            ErrorCategory::Configuration
            | ErrorCategory::Manifest
            | ErrorCategory::Registry
            | ErrorCategory::Schema => Self::ConfigurationFailure,
            ErrorCategory::RuntimeStartup | ErrorCategory::PluginLoad => Self::PluginFailure,
            ErrorCategory::Cancellation => Self::Cancelled,
            _ => Self::HostFailure,
        }
    }

    /// Returns the terminal crawl state corresponding to this outcome.
    #[must_use]
    pub const fn terminal_state(self) -> CrawlState {
        match self {
            Self::Success => CrawlState::Completed,
            Self::PartialSuccess => CrawlState::CompletedWithErrors,
            Self::Cancelled => CrawlState::Cancelled,
            Self::ConfigurationFailure | Self::PluginFailure | Self::HostFailure => {
                CrawlState::Failed
            }
        }
    }

    /// Returns the stable snake_case label used in logs and summaries.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::PartialSuccess => "partial_success",
            Self::ConfigurationFailure => "configuration_failure",
            Self::Cancelled => "cancelled",
            Self::PluginFailure => "plugin_failure",
            Self::HostFailure => "host_failure",
        }
    }
}

impl fmt::Display for CrawlOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_transitions_are_legal() {
        let state = CrawlState::Created
            .transition(CrawlState::Validating)
            .and_then(|s| s.transition(CrawlState::Running))
            .and_then(|s| s.transition(CrawlState::Finalizing))
            .and_then(|s| s.transition(CrawlState::Completed))
            .unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn cancellation_and_stopping_route_through_finalizing() {
        assert!(CrawlState::Running.can_transition_to(CrawlState::Cancelling));
        assert!(CrawlState::Cancelling.can_transition_to(CrawlState::Finalizing));
        assert!(CrawlState::Stopping.can_transition_to(CrawlState::Finalizing));
        assert!(!CrawlState::Cancelling.can_transition_to(CrawlState::Completed));
    }

    #[test]
    fn illegal_transitions_are_rejected() {
        let error = CrawlState::Created
            .transition(CrawlState::Running)
            .unwrap_err();
        assert_eq!(error.from, CrawlState::Created);
        assert_eq!(error.to, CrawlState::Running);
        assert!(CrawlState::Completed
            .transition(CrawlState::Running)
            .is_err());
    }

    #[test]
    fn only_running_accepts_new_work() {
        assert!(CrawlState::Running.accepts_new_work());
        for state in [
            CrawlState::Cancelling,
            CrawlState::Stopping,
            CrawlState::Finalizing,
            CrawlState::Validating,
        ] {
            assert!(!state.accepts_new_work(), "{state} must not accept work");
        }
    }

    #[test]
    fn outcome_distinguishes_success_from_partial_success() {
        let clean = CrawlStatistics::new();
        assert_eq!(
            CrawlOutcome::derive(CrawlState::Running, &clean),
            CrawlOutcome::Success
        );

        let mut dirty = CrawlStatistics::new();
        dirty.record_error(ErrorCategory::PluginProcessing);
        assert_eq!(
            CrawlOutcome::derive(CrawlState::Running, &dirty),
            CrawlOutcome::PartialSuccess
        );
    }

    #[test]
    fn cancellation_outcome_wins_over_error_counts() {
        let mut dirty = CrawlStatistics::new();
        dirty.record_error(ErrorCategory::Timeout);
        assert_eq!(
            CrawlOutcome::derive(CrawlState::Cancelling, &dirty),
            CrawlOutcome::Cancelled
        );
    }

    #[test]
    fn fatal_categories_map_to_distinct_outcomes() {
        assert_eq!(
            CrawlOutcome::from_fatal_category(ErrorCategory::Manifest),
            CrawlOutcome::ConfigurationFailure
        );
        assert_eq!(
            CrawlOutcome::from_fatal_category(ErrorCategory::RuntimeStartup),
            CrawlOutcome::PluginFailure
        );
        assert_eq!(
            CrawlOutcome::from_fatal_category(ErrorCategory::Report),
            CrawlOutcome::HostFailure
        );
    }

    #[test]
    fn outcomes_map_back_to_terminal_states() {
        assert_eq!(
            CrawlOutcome::Success.terminal_state(),
            CrawlState::Completed
        );
        assert_eq!(
            CrawlOutcome::PartialSuccess.terminal_state(),
            CrawlState::CompletedWithErrors
        );
        assert_eq!(
            CrawlOutcome::Cancelled.terminal_state(),
            CrawlState::Cancelled
        );
        assert_eq!(
            CrawlOutcome::HostFailure.terminal_state(),
            CrawlState::Failed
        );
    }
}
