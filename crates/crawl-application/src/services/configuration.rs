//! Effective-configuration resolution (REQ-158, OQ-015).
//!
//! # Precedence decision
//!
//! `CLI override > plugin manifest > host default`. The operator running the
//! command is the most specific authority, the plugin author the next, and the
//! host only fills gaps. Resolution is a pure function so precedence is
//! exhaustively testable without starting a crawl.

use std::num::NonZeroU32;
use std::path::PathBuf;

use crawl_domain::plugin::Plugin;
use crawl_domain::policy::{
    FailurePolicy, QueueCapacity, RetryLimit, RetryPolicy, Timeout, WorkerCount,
};

/// Preflight and configuration failures (REQ-032, REQ-033, REQ-131).
#[derive(Debug, thiserror::Error)]
pub enum ConfigurationError {
    /// The input directory is missing or is not a directory.
    #[error("input directory {path} is not accessible: {reason}")]
    Input {
        /// The offending path.
        path: PathBuf,
        /// Why it was rejected.
        reason: String,
    },
    /// The output path is unusable.
    #[error("output path {path} is not usable: {reason}")]
    Output {
        /// The offending path.
        path: PathBuf,
        /// Why it was rejected.
        reason: String,
    },
    /// The output file exists and overwrite was not authorized (REQ-034).
    #[error("output file {path} already exists; pass --overwrite to replace it")]
    OutputExists {
        /// The existing report path.
        path: PathBuf,
    },
    /// A supplied option value was invalid.
    #[error("invalid value for {option}: {reason}")]
    Option {
        /// The offending option name.
        option: &'static str,
        /// Why it was rejected.
        reason: String,
    },
}

/// Operator-supplied overrides from the CLI. `None` means "not specified".
#[derive(Debug, Clone, Default)]
pub struct RunOverrides {
    /// `--workers`
    pub workers: Option<WorkerCount>,
    /// `--timeout`
    pub timeout: Option<Timeout>,
    /// `--no-timeout` disables the deadline even if the manifest sets one.
    pub disable_timeout: bool,
    /// `--max-retries`
    pub max_retries: Option<RetryLimit>,
    /// `--queue-capacity`
    pub queue_capacity: Option<QueueCapacity>,
    /// `--fail-fast`
    pub fail_fast: bool,
    /// `--max-errors`
    pub max_errors: Option<u64>,
}

/// Fully resolved execution configuration for one crawl.
#[derive(Debug, Clone)]
pub struct EffectiveConfig {
    /// Input root.
    pub input: PathBuf,
    /// Requested report path.
    pub output: PathBuf,
    /// Number of concurrently active workers.
    pub workers: NonZeroU32,
    /// Bounded work-queue depth.
    pub queue_capacity: QueueCapacity,
    /// Per-file deadline, if any.
    pub timeout: Option<Timeout>,
    /// Retry policy, including eligibility rules.
    pub retry: RetryPolicy,
    /// Fail-fast and error-threshold policy.
    pub failure: FailurePolicy,
}

impl EffectiveConfig {
    /// Resolves manifest defaults and CLI overrides into one configuration.
    ///
    /// `available_parallelism` is injected rather than read from the OS so the
    /// `workers: auto` policy stays deterministic under test.
    #[must_use]
    pub fn resolve(
        plugin: &Plugin,
        input: PathBuf,
        output: PathBuf,
        overrides: &RunOverrides,
        available_parallelism: u32,
    ) -> Self {
        let workers = overrides
            .workers
            .or(plugin.execution.workers)
            .unwrap_or(WorkerCount::AUTO)
            .resolve(available_parallelism);

        let queue_capacity = overrides
            .queue_capacity
            .or(plugin.execution.queue_capacity)
            .unwrap_or_default();

        let timeout = if overrides.disable_timeout {
            None
        } else {
            overrides.timeout.or(plugin.execution.timeout)
        };

        let max_retries = overrides
            .max_retries
            .or(plugin.execution.max_retries)
            .unwrap_or_default();

        Self {
            input,
            output,
            workers,
            queue_capacity,
            timeout,
            retry: RetryPolicy::new(max_retries),
            failure: FailurePolicy::new(overrides.fail_fast, overrides.max_errors),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crawl_domain::manifest::PluginManifest;
    use std::path::Path;

    /// A manifest declaring every execution default, so precedence is visible.
    fn plugin_with_defaults() -> Plugin {
        let manifest = r#"
api_version: "1"
plugin:
  name: p
  version: "1"
runtime:
  type: python
  command: ["python3", "w.py"]
input:
  extensions: [".txt"]
output:
  format: records
  schema:
    a:
      type: string
      required: true
execution:
  workers: 8
  timeout_seconds: 30
  max_retries: 3
  queue_capacity: 64
"#;
        PluginManifest::parse(manifest, Path::new("plugin.yaml"))
            .expect("parses")
            .validate(Path::new("/p"))
            .expect("validates")
    }

    /// A manifest declaring no execution block at all.
    fn plugin_without_defaults() -> Plugin {
        let manifest = r#"
api_version: "1"
plugin:
  name: p
  version: "1"
runtime:
  type: python
  command: ["python3", "w.py"]
input:
  extensions: [".txt"]
output:
  format: records
  schema:
    a:
      type: string
      required: true
"#;
        PluginManifest::parse(manifest, Path::new("plugin.yaml"))
            .expect("parses")
            .validate(Path::new("/p"))
            .expect("validates")
    }

    fn resolve(plugin: &Plugin, overrides: &RunOverrides) -> EffectiveConfig {
        EffectiveConfig::resolve(
            plugin,
            PathBuf::from("/in"),
            PathBuf::from("/out.csv"),
            overrides,
            4,
        )
    }

    #[test]
    fn manifest_defaults_apply_when_the_cli_is_silent() {
        let config = resolve(&plugin_with_defaults(), &RunOverrides::default());
        assert_eq!(config.workers.get(), 8);
        assert_eq!(config.timeout.map(Timeout::seconds), Some(30.0));
        assert_eq!(config.retry.limit().max_retries(), 3);
        assert_eq!(config.queue_capacity.get(), 64);
    }

    #[test]
    fn cli_overrides_beat_manifest_defaults() {
        let overrides = RunOverrides {
            workers: Some(WorkerCount::fixed(2).unwrap()),
            timeout: Some(Timeout::from_seconds(1.5).unwrap()),
            max_retries: Some(RetryLimit::new(0)),
            queue_capacity: Some(QueueCapacity::new(16).unwrap()),
            ..RunOverrides::default()
        };
        let config = resolve(&plugin_with_defaults(), &overrides);
        assert_eq!(config.workers.get(), 2);
        assert_eq!(config.timeout.map(Timeout::seconds), Some(1.5));
        assert_eq!(config.retry.limit().max_retries(), 0);
        assert_eq!(config.queue_capacity.get(), 16);
    }

    #[test]
    fn host_defaults_apply_when_neither_specifies() {
        let config = resolve(&plugin_without_defaults(), &RunOverrides::default());
        // `workers: auto` resolves against the injected parallelism.
        assert_eq!(config.workers.get(), 4);
        assert_eq!(
            config.timeout, None,
            "no timeout unless someone asks for one"
        );
        assert_eq!(config.retry.limit().max_retries(), 0);
        assert_eq!(config.queue_capacity.get(), QueueCapacity::DEFAULT as usize);
    }

    #[test]
    fn no_timeout_beats_a_manifest_timeout() {
        let overrides = RunOverrides {
            disable_timeout: true,
            ..RunOverrides::default()
        };
        assert_eq!(resolve(&plugin_with_defaults(), &overrides).timeout, None);
    }

    #[test]
    fn auto_workers_from_the_cli_override_a_fixed_manifest_count() {
        let overrides = RunOverrides {
            workers: Some(WorkerCount::AUTO),
            ..RunOverrides::default()
        };
        assert_eq!(
            resolve(&plugin_with_defaults(), &overrides).workers.get(),
            4
        );
    }

    #[test]
    fn failure_policy_comes_only_from_the_cli() {
        let overrides = RunOverrides {
            fail_fast: true,
            ..RunOverrides::default()
        };
        let config = resolve(&plugin_with_defaults(), &overrides);
        assert!(config.failure.fail_fast);
        assert_eq!(config.failure.max_errors, None);

        let overrides = RunOverrides {
            max_errors: Some(10),
            ..RunOverrides::default()
        };
        let config = resolve(&plugin_with_defaults(), &overrides);
        assert!(!config.failure.fail_fast);
        assert_eq!(config.failure.max_errors, Some(10));
    }

    #[test]
    fn input_and_output_paths_are_carried_through_verbatim() {
        let config = resolve(&plugin_with_defaults(), &RunOverrides::default());
        assert_eq!(config.input, PathBuf::from("/in"));
        assert_eq!(config.output, PathBuf::from("/out.csv"));
    }
}
