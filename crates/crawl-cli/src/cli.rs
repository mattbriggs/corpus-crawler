//! Command-line surface (SRS 4.1, REQ-140 - REQ-143).

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// A modular high-throughput file-processing crawler.
///
/// Installed plugins run as trusted local code. The subprocess boundary
/// provides fault isolation, not a security sandbox.
#[derive(Debug, Parser)]
#[command(name = "crawl", version, about, long_about = None)]
pub struct Cli {
    /// Path to the plugin registry file.
    #[arg(long, global = true, env = "CRAWL_REGISTRY")]
    pub registry: Option<PathBuf>,

    /// Increase log verbosity; repeat for more detail.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Suppress all logging except errors.
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    /// Log presentation format.
    #[arg(long, global = true, value_enum, default_value_t = LogFormat::Text)]
    pub log_format: LogFormat,

    /// The command to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Log presentation (REQ-110, REQ-111).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LogFormat {
    /// Human-readable lines.
    Text,
    /// One JSON object per event.
    Json,
}

/// Top-level commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run a crawl with a registered plugin.
    Run(RunArgs),
    /// Manage installed plugins.
    #[command(subcommand)]
    Plugin(PluginCommand),
}

/// Arguments for `crawl run` (REQ-030, REQ-142, REQ-143).
#[derive(Debug, Args)]
pub struct RunArgs {
    /// Registered plugin name.
    pub plugin: String,

    /// Directory to crawl recursively.
    #[arg(short, long)]
    pub input: PathBuf,

    /// Destination CSV report.
    #[arg(short, long)]
    pub output: PathBuf,

    /// Write structured logs to this file in addition to stderr.
    #[arg(short, long)]
    pub log: Option<PathBuf>,

    /// Number of plugin workers, or `auto`.
    #[arg(short, long)]
    pub workers: Option<String>,

    /// Per-file processing timeout in seconds.
    #[arg(short, long)]
    pub timeout: Option<f64>,

    /// Disable the per-file timeout even if the manifest declares one.
    #[arg(long, conflicts_with = "timeout")]
    pub no_timeout: bool,

    /// Maximum retries per file after the first attempt.
    #[arg(long)]
    pub max_retries: Option<u32>,

    /// Bounded work-queue depth.
    #[arg(long)]
    pub queue_capacity: Option<u32>,

    /// Stop scheduling after the first qualifying error.
    #[arg(long)]
    pub fail_fast: bool,

    /// Stop scheduling once this many qualifying errors are recorded.
    #[arg(long, conflicts_with = "fail_fast")]
    pub max_errors: Option<u64>,

    /// Overwrite the report if it already exists.
    #[arg(long)]
    pub overwrite: bool,
}

/// Plugin lifecycle commands.
#[derive(Debug, Subcommand)]
pub enum PluginCommand {
    /// Validate and register a plugin from a directory or manifest path.
    Install {
        /// Plugin directory or `plugin.yaml` path.
        source: PathBuf,
        /// Replace an existing registration with the same name.
        #[arg(long)]
        force: bool,
        /// Register without starting the runtime.
        #[arg(long)]
        skip_runtime_check: bool,
    },
    /// Validate an installed plugin, or a plugin directory before install.
    Validate {
        /// Registered plugin name, or a path when `--path` is given.
        target: String,
        /// Treat the target as a filesystem path rather than a registered name.
        #[arg(long)]
        path: bool,
    },
    /// List registered plugins.
    List {
        /// Emit JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Show a registered plugin's effective metadata.
    Inspect {
        /// Registered plugin name.
        plugin: String,
        /// Emit JSON instead of YAML.
        #[arg(long)]
        json: bool,
    },
    /// Unregister a plugin. Plugin files are never deleted.
    Remove {
        /// Registered plugin name.
        plugin: String,
    },
}
