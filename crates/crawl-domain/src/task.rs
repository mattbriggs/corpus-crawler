//! File task identity, attempt accounting, and the file-level state model
//! (SRS 7.5).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::errors::ErrorCategory;
use crate::ids::RequestId;

/// One unit of plugin work: process exactly one discovered file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileTask {
    /// Absolute path of the file to process.
    path: PathBuf,
    /// Number of attempts already completed.
    attempts: u32,
}

impl FileTask {
    /// Creates a fresh task with zero completed attempts.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            attempts: 0,
        }
    }

    /// Returns the file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the number of attempts already completed.
    #[must_use]
    pub const fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Records that another attempt has been made and returns the new count.
    pub fn record_attempt(&mut self) -> u32 {
        self.attempts += 1;
        self.attempts
    }

    /// Returns `true` when this task has been retried at least once.
    #[must_use]
    pub const fn is_retry(&self) -> bool {
        self.attempts > 1
    }
}

/// Lifecycle state of a file task (SRS 7.5).
///
/// The state is not stored on [`FileTask`]: it is the *outcome* of a dispatch,
/// reported to the aggregator. Keeping it separate stops the queue entry from
/// carrying stale state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum FileTaskState {
    /// The file was seen by traversal.
    Discovered,
    /// The file did not match the effective extension configuration.
    Skipped,
    /// The file is waiting in the bounded queue.
    Queued,
    /// A worker is processing the file.
    Processing,
    /// The response is being validated against the output schema.
    Validating,
    /// All rows for the file were accepted (possibly zero rows).
    Completed,
    /// The file processed, but one or more rows were rejected.
    CompletedWithErrors,
    /// The file reached a terminal failure.
    Failed {
        /// Why the task failed.
        category: ErrorCategory,
    },
}

impl FileTaskState {
    /// Returns `true` when no further transition is possible.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Skipped | Self::Completed | Self::CompletedWithErrors | Self::Failed { .. }
        )
    }

    /// Returns `true` when the file was processed without a terminal failure.
    #[must_use]
    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Completed | Self::CompletedWithErrors)
    }
}

/// Result of one dispatch attempt, as reported by a worker task.
#[derive(Debug, Clone, PartialEq)]
pub struct AttemptOutcome {
    /// The request that produced this outcome.
    pub request: RequestId,
    /// The attempt number, starting at 1.
    pub attempt: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attempts_start_at_zero_and_increment() {
        let mut task = FileTask::new("/data/a.txt");
        assert_eq!(task.attempts(), 0);
        assert!(!task.is_retry());
        assert_eq!(task.record_attempt(), 1);
        assert!(!task.is_retry());
        assert_eq!(task.record_attempt(), 2);
        assert!(task.is_retry());
    }

    #[test]
    fn terminal_states_are_classified() {
        assert!(FileTaskState::Completed.is_terminal());
        assert!(FileTaskState::CompletedWithErrors.is_terminal());
        assert!(FileTaskState::Skipped.is_terminal());
        assert!(FileTaskState::Failed {
            category: ErrorCategory::Timeout
        }
        .is_terminal());
        assert!(!FileTaskState::Queued.is_terminal());
        assert!(!FileTaskState::Processing.is_terminal());
    }

    #[test]
    fn success_excludes_failure_and_skip() {
        assert!(FileTaskState::Completed.is_success());
        assert!(FileTaskState::CompletedWithErrors.is_success());
        assert!(!FileTaskState::Skipped.is_success());
        assert!(!FileTaskState::Failed {
            category: ErrorCategory::Schema
        }
        .is_success());
    }
}
