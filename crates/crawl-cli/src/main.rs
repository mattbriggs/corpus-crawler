//! The `crawl` command-line application: the composition root.
//!
//! This binary is the only place concrete adapters are chosen and wired to the
//! application's ports.

#![forbid(unsafe_code)]

mod cli;
mod commands;
mod exit_codes;

use clap::Parser;
use crawl_infrastructure::FileRegistry;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::prelude::*;

#[tokio::main]
async fn main() {
    let args = cli::Cli::parse();
    let log_path = match &args.command {
        cli::Command::Run(run_args) => run_args.log.clone(),
        cli::Command::Plugin(_) => None,
    };
    init_logging(&args, log_path.as_deref());

    let registry = match &args.registry {
        Some(path) => FileRegistry::at(path),
        None => FileRegistry::default_location(),
    };

    let result = match args.command {
        cli::Command::Run(run_args) => commands::run::execute(&registry, run_args).await,
        cli::Command::Plugin(command) => match command {
            cli::PluginCommand::Install {
                source,
                force,
                skip_runtime_check,
            } => commands::plugin::install(&registry, &source, force, skip_runtime_check).await,
            cli::PluginCommand::Validate { target, path } => {
                commands::plugin::validate(&registry, &target, path).await
            }
            cli::PluginCommand::List { json } => commands::plugin::list(&registry, json),
            cli::PluginCommand::Inspect { plugin, json } => {
                commands::plugin::inspect(&registry, &plugin, json)
            }
            cli::PluginCommand::Remove { plugin } => commands::plugin::remove(&registry, &plugin),
        },
    };

    let code = match result {
        Ok(code) => code,
        Err((message, code)) => {
            eprintln!("error: {message}");
            code
        }
    };
    std::process::exit(code);
}

/// Installs the tracing subscriber for this invocation (REQ-190).
///
/// `stderr` always receives the operator-facing log. When `--log` is given the
/// same events are additionally written to that file as JSON, which is the
/// archival form automation consumes (REQ-110).
fn init_logging(args: &cli::Cli, log_path: Option<&std::path::Path>) {
    let level = if args.quiet {
        LevelFilter::ERROR
    } else {
        match args.verbose {
            0 => LevelFilter::INFO,
            1 => LevelFilter::DEBUG,
            _ => LevelFilter::TRACE,
        }
    };

    let stderr_layer: Box<dyn tracing_subscriber::Layer<_> + Send + Sync> = match args.log_format {
        cli::LogFormat::Json => Box::new(
            tracing_subscriber::fmt::layer()
                .json()
                .with_writer(std::io::stderr),
        ),
        cli::LogFormat::Text => Box::new(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_writer(std::io::stderr),
        ),
    };

    let registry = tracing_subscriber::registry()
        .with(level)
        .with(stderr_layer);

    match log_path.and_then(|path| std::fs::File::create(path).ok()) {
        Some(file) => registry
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_writer(std::sync::Mutex::new(file)),
            )
            .init(),
        None => registry.init(),
    }
}
