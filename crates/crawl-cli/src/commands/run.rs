//! `crawl run` (REQ-030 - REQ-035).

use std::sync::Arc;

use crawl_application::ports::registry::PluginRegistry;
use crawl_application::services::configuration::{
    ConfigurationError, EffectiveConfig, RunOverrides,
};
use crawl_application::services::crawl_coordinator::CrawlCoordinator;
use crawl_application::{commands as use_cases, ports::report::ReportWriter};
use crawl_domain::policy::{QueueCapacity, RetryLimit, Timeout, WorkerCount};
use crawl_infrastructure::{
    CsvReportWriter, FilesystemWalker, ProcessWorkerFactory, TracingEventSink,
};
use tokio_util::sync::CancellationToken;

use crate::cli::RunArgs;
use crate::exit_codes;

/// Executes one crawl and returns its process exit status.
pub async fn execute(registry: &dyn PluginRegistry, args: RunArgs) -> Result<i32, (String, i32)> {
    // --- resolve the plugin (REQ-031) ------------------------------------
    let id = crate::commands::plugin::parse_id(&args.plugin)?;
    let plugin = use_cases::resolve_plugin(registry, &id)
        .map_err(|error| (error.to_string(), exit_codes::CONFIGURATION_FAILURE))?;

    // --- preflight validation (REQ-032, REQ-033) --------------------------
    let overrides = build_overrides(&args)?;
    preflight(&args)?;

    // Workers run with the plugin directory as their working directory, so a
    // relative input root would produce paths they cannot open (REQ-162).
    // Resolving it once here keeps every downstream path absolute.
    let input = args.input.canonicalize().map_err(|error| {
        (
            ConfigurationError::Input {
                path: args.input.clone(),
                reason: error.to_string(),
            }
            .to_string(),
            exit_codes::CONFIGURATION_FAILURE,
        )
    })?;

    let parallelism = std::thread::available_parallelism()
        .map(|value| value.get() as u32)
        .unwrap_or(1);
    let config =
        EffectiveConfig::resolve(&plugin, input, args.output.clone(), &overrides, parallelism);

    // Creating the writer proves the destination is usable and writes the
    // header before any file is dispatched (REQ-032, AC-026).
    let writer = CsvReportWriter::create(&config.output, &plugin.schema)
        .map_err(|error| (error.to_string(), exit_codes::CONFIGURATION_FAILURE))?;

    // --- cancellation (REQ-125) -------------------------------------------
    let cancel = CancellationToken::new();
    let signal_token = cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            tracing::warn!(event = "crawl_cancelled", "cancellation requested");
            signal_token.cancel();
        }
    });

    let coordinator = CrawlCoordinator::new(
        plugin,
        config,
        Arc::new(FilesystemWalker),
        Arc::new(ProcessWorkerFactory),
        Box::new(writer) as Box<dyn ReportWriter>,
        Arc::new(TracingEventSink),
        cancel,
    );

    match coordinator.run().await {
        Ok(report) => {
            print_summary(&report);
            Ok(exit_codes::for_outcome(report.outcome))
        }
        Err(error) => {
            let code = exit_codes::for_outcome(error.outcome());
            Err((error.to_string(), code))
        }
    }
}

/// Converts raw CLI values into validated overrides (NFR-050).
fn build_overrides(args: &RunArgs) -> Result<RunOverrides, (String, i32)> {
    let invalid = |option: &'static str, reason: String| {
        (
            ConfigurationError::Option { option, reason }.to_string(),
            exit_codes::CONFIGURATION_FAILURE,
        )
    };

    let workers = match args.workers.as_deref() {
        None => None,
        Some("auto") => Some(WorkerCount::AUTO),
        Some(value) => {
            let count: u32 = value.parse().map_err(|_| {
                invalid(
                    "--workers",
                    format!("{value:?} is not an integer or 'auto'"),
                )
            })?;
            Some(
                WorkerCount::fixed(count)
                    .map_err(|error| invalid("--workers", error.to_string()))?,
            )
        }
    };

    let timeout = args
        .timeout
        .map(Timeout::from_seconds)
        .transpose()
        .map_err(|error| invalid("--timeout", error.to_string()))?;

    let queue_capacity = args
        .queue_capacity
        .map(QueueCapacity::new)
        .transpose()
        .map_err(|error| invalid("--queue-capacity", error.to_string()))?;

    if args.max_errors == Some(0) {
        return Err(invalid(
            "--max-errors",
            "must be greater than zero".to_owned(),
        ));
    }

    Ok(RunOverrides {
        workers,
        timeout,
        disable_timeout: args.no_timeout,
        max_retries: args.max_retries.map(RetryLimit::new),
        queue_capacity,
        fail_fast: args.fail_fast,
        max_errors: args.max_errors,
    })
}

/// Validates input and output locations before any file is scheduled.
fn preflight(args: &RunArgs) -> Result<(), (String, i32)> {
    let fail = |error: ConfigurationError| (error.to_string(), exit_codes::CONFIGURATION_FAILURE);

    let metadata = std::fs::metadata(&args.input).map_err(|error| {
        fail(ConfigurationError::Input {
            path: args.input.clone(),
            reason: error.to_string(),
        })
    })?;
    if !metadata.is_dir() {
        return Err(fail(ConfigurationError::Input {
            path: args.input.clone(),
            reason: "not a directory".to_owned(),
        }));
    }

    // REQ-034 / AC-023: never silently replace an existing report.
    if args.output.exists() && !args.overwrite {
        return Err(fail(ConfigurationError::OutputExists {
            path: args.output.clone(),
        }));
    }
    Ok(())
}

/// Prints the operator-facing summary (REQ-117, REQ-118, AC-024).
fn print_summary(report: &crawl_application::services::crawl_coordinator::CrawlReport) {
    let stats = &report.statistics;
    println!();
    println!("crawl {} finished: {}", report.crawl, report.outcome);
    println!("  files discovered : {}", stats.files_discovered);
    println!("  files matched    : {}", stats.files_matched);
    println!("  files completed  : {}", stats.files_completed);
    println!("  files failed     : {}", stats.files_failed);
    println!("  records emitted  : {}", stats.records_emitted);
    println!("  records rejected : {}", stats.records_rejected);
    println!("  timeouts         : {}", stats.timeouts);
    println!("  worker restarts  : {}", stats.worker_restarts);
    println!("  retries          : {}", stats.retries);
    println!("  duration         : {:.3}s", stats.duration_seconds);
    if stats.total_errors() > 0 {
        println!("  errors by category:");
        for (category, count) in &stats.errors_by_category {
            println!("    {:<18} {}", category.as_str(), count);
        }
    }
}
