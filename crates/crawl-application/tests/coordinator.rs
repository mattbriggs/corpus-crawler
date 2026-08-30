//! Coordinator orchestration tests.
//!
//! These drive the real coordinator against in-memory ports, so the pipeline
//! invariants, retry accounting, and failure policies are asserted directly
//! rather than inferred from a subprocess crawl.

mod support;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crawl_application::ports::report::ReportWriter;
use crawl_application::services::configuration::{EffectiveConfig, RunOverrides};
use crawl_application::services::crawl_coordinator::{CrawlCoordinator, HostFatalError};
use crawl_domain::crawl::CrawlOutcome;
use crawl_domain::errors::ErrorCategory;
use crawl_domain::policy::{PartialOutputPolicy, RetryLimit, Timeout, WorkerCount};
use crawl_domain::statistics::CrawlStatistics;
use tokio_util::sync::CancellationToken;

use support::{
    count_events, Behaviour, FailingFactory, FailingWriter, RecordingSink, RecordingWriter,
    ScriptedFactory, ScriptedWalker, WorkerObservations,
};

/// Everything a finished test crawl leaves behind for assertions.
struct Outcome {
    result: Result<crawl_application::services::crawl_coordinator::CrawlReport, HostFatalError>,
    rows: Vec<Vec<String>>,
    finalized: Option<PartialOutputPolicy>,
    events: Vec<String>,
    observations: Arc<WorkerObservations>,
}

impl Outcome {
    fn statistics(&self) -> &CrawlStatistics {
        &self.result.as_ref().expect("crawl completed").statistics
    }

    fn outcome(&self) -> CrawlOutcome {
        self.result.as_ref().expect("crawl completed").outcome
    }
}

/// Runs a crawl over `count` `.txt` files with the given script and overrides.
async fn run_crawl(
    count: usize,
    script: HashMap<String, Behaviour>,
    overrides: RunOverrides,
    first_attempt_only: bool,
    cancel: CancellationToken,
) -> Outcome {
    let plugin = support::test_plugin();
    let paths: Vec<PathBuf> = (0..count)
        .map(|index| PathBuf::from(format!("/corpus/f{index}.txt")))
        .collect();

    let mut factory = ScriptedFactory::new(script);
    factory.first_attempt_only = first_attempt_only;
    let observations = factory.observations();

    let writer = RecordingWriter::default();
    let (rows, finalized) = writer.handles();
    let sink = RecordingSink::default();
    let events = sink.handle();

    let config = EffectiveConfig::resolve(
        &plugin,
        PathBuf::from("/corpus"),
        PathBuf::from("/report.csv"),
        &overrides,
        4,
    );

    let coordinator = CrawlCoordinator::new(
        plugin,
        config,
        Arc::new(ScriptedWalker { paths }),
        Arc::new(factory),
        Box::new(writer) as Box<dyn ReportWriter>,
        Arc::new(sink),
        cancel,
    );
    let result = coordinator.run().await;

    let rows = rows.lock().expect("rows").clone();
    let finalized = *finalized.lock().expect("finalized");
    let events = events.lock().expect("events").clone();
    Outcome {
        result,
        rows,
        finalized,
        events,
        observations,
    }
}

fn overrides() -> RunOverrides {
    RunOverrides {
        workers: Some(WorkerCount::fixed(2).expect("two workers")),
        timeout: Some(Timeout::from_seconds(5.0).expect("timeout")),
        ..RunOverrides::default()
    }
}

// --- happy path ----------------------------------------------------------

#[tokio::test]
async fn every_matched_file_is_processed_and_written() {
    let result = run_crawl(
        10,
        HashMap::new(),
        overrides(),
        false,
        CancellationToken::new(),
    )
    .await;

    assert_eq!(result.outcome(), CrawlOutcome::Success);
    assert_eq!(result.statistics().files_matched, 10);
    assert_eq!(result.statistics().files_completed, 10);
    assert_eq!(result.statistics().records_emitted, 10);
    assert_eq!(result.rows.len(), 10);
    assert_eq!(result.finalized, Some(PartialOutputPolicy::Promote));
}

#[tokio::test]
async fn rows_are_written_in_schema_column_order() {
    let mut script = HashMap::new();
    script.insert("f0.txt".to_owned(), Behaviour::Rows(1));
    let result = run_crawl(1, script, overrides(), false, CancellationToken::new()).await;

    // filename, line, note - with the optional trailing field empty.
    assert_eq!(result.rows[0], vec!["/corpus/f0.txt", "1", ""]);
}

#[tokio::test]
async fn a_file_may_emit_zero_or_many_rows() {
    let mut script = HashMap::new();
    script.insert("f0.txt".to_owned(), Behaviour::Rows(0));
    script.insert("f1.txt".to_owned(), Behaviour::Rows(5));
    let result = run_crawl(2, script, overrides(), false, CancellationToken::new()).await;

    assert_eq!(result.statistics().files_completed, 2);
    assert_eq!(result.statistics().records_emitted, 5);
    assert_eq!(result.outcome(), CrawlOutcome::Success);
}

// --- pipeline invariants -------------------------------------------------

#[tokio::test]
async fn a_worker_never_has_two_requests_in_flight() {
    let result = run_crawl(
        50,
        HashMap::new(),
        overrides(),
        false,
        CancellationToken::new(),
    )
    .await;
    assert!(
        !result
            .observations
            .concurrent_dispatch_detected
            .load(Ordering::SeqCst),
        "two dispatches overlapped on one worker"
    );
}

#[tokio::test]
async fn live_workers_never_exceed_the_configured_count() {
    let result = run_crawl(
        50,
        HashMap::new(),
        overrides(),
        false,
        CancellationToken::new(),
    )
    .await;
    assert!(
        result.observations.peak_live.load(Ordering::SeqCst) <= 2,
        "more workers were live than configured"
    );
}

// --- retry accounting ----------------------------------------------------

#[tokio::test]
async fn timeouts_are_retried_up_to_the_configured_limit() {
    let mut script = HashMap::new();
    script.insert("f0.txt".to_owned(), Behaviour::Timeout);
    let mut settings = overrides();
    settings.max_retries = Some(RetryLimit::new(2));

    let result = run_crawl(1, script, settings, false, CancellationToken::new()).await;

    // 1 + max_retries attempts, and no more.
    assert_eq!(
        result
            .observations
            .attempts_for(&PathBuf::from("/corpus/f0.txt")),
        3
    );
    assert_eq!(result.statistics().files_failed, 1);
    assert_eq!(result.statistics().retries, 2);
    assert_eq!(result.statistics().timeouts, 3);
}

#[tokio::test]
async fn a_retry_that_succeeds_completes_the_file() {
    let mut script = HashMap::new();
    script.insert("f0.txt".to_owned(), Behaviour::Crash);
    let mut settings = overrides();
    settings.max_retries = Some(RetryLimit::new(1));

    // `first_attempt_only` makes the second attempt succeed.
    let result = run_crawl(1, script, settings, true, CancellationToken::new()).await;

    assert_eq!(result.statistics().files_completed, 1);
    assert_eq!(result.statistics().files_failed, 0);
    assert_eq!(result.statistics().retries, 1);
    assert_eq!(result.statistics().worker_restarts, 1);

    // A transient failure that a retry recovered from does not downgrade the
    // outcome: no file ended in a processing error, so the crawl succeeded
    // (REQ-132). The retry and restart counters are what tell the operator the
    // run was not smooth, which is why AC-024 requires them in the summary.
    assert_eq!(result.outcome(), CrawlOutcome::Success);
    assert_eq!(result.statistics().qualifying_errors(), 0);
}

#[tokio::test]
async fn plugin_declared_errors_are_never_retried() {
    let mut script = HashMap::new();
    script.insert("f0.txt".to_owned(), Behaviour::PluginError);
    let mut settings = overrides();
    settings.max_retries = Some(RetryLimit::new(5));

    let result = run_crawl(1, script, settings, false, CancellationToken::new()).await;

    assert_eq!(
        result
            .observations
            .attempts_for(&PathBuf::from("/corpus/f0.txt")),
        1,
        "a deterministic plugin answer must not be retried"
    );
    assert_eq!(result.statistics().retries, 0);
    assert_eq!(
        result
            .statistics()
            .errors_in(ErrorCategory::PluginProcessing),
        1
    );
}

#[tokio::test]
async fn zero_retries_means_one_attempt() {
    let mut script = HashMap::new();
    script.insert("f0.txt".to_owned(), Behaviour::Timeout);
    let mut settings = overrides();
    settings.max_retries = Some(RetryLimit::new(0));

    let result = run_crawl(1, script, settings, false, CancellationToken::new()).await;
    assert_eq!(
        result
            .observations
            .attempts_for(&PathBuf::from("/corpus/f0.txt")),
        1
    );
}

// --- failure isolation ---------------------------------------------------

#[tokio::test]
async fn one_failing_file_does_not_stop_the_others() {
    let mut script = HashMap::new();
    script.insert("f3.txt".to_owned(), Behaviour::PluginError);
    let result = run_crawl(20, script, overrides(), false, CancellationToken::new()).await;

    assert_eq!(result.statistics().files_completed, 19);
    assert_eq!(result.statistics().files_failed, 1);
    assert_eq!(result.outcome(), CrawlOutcome::PartialSuccess);
}

#[tokio::test]
async fn a_crashed_worker_is_replaced() {
    let mut script = HashMap::new();
    script.insert("f0.txt".to_owned(), Behaviour::Crash);

    // One worker fixes the order: f0 crashes first, the slot is replaced, and
    // the replacement processes the remaining four files. That makes every
    // count below exact rather than a lower bound.
    let mut settings = overrides();
    settings.workers = Some(WorkerCount::fixed(1).expect("one worker"));
    let result = run_crawl(5, script, settings, true, CancellationToken::new()).await;

    assert_eq!(result.statistics().worker_restarts, 1);
    assert_eq!(
        count_events(
            &Arc::new(std::sync::Mutex::new(result.events.clone())),
            "worker_restarted"
        ),
        1
    );
    // The crash cost exactly one file; the rest completed on the replacement.
    assert_eq!(result.statistics().files_failed, 1);
    assert_eq!(result.statistics().files_completed, 4);
    assert_eq!(result.rows.len(), 4);
}

#[tokio::test]
async fn invalid_rows_are_rejected_without_reaching_the_writer() {
    let mut script = HashMap::new();
    script.insert("f0.txt".to_owned(), Behaviour::OneInvalidRow);
    let result = run_crawl(1, script, overrides(), false, CancellationToken::new()).await;

    assert_eq!(result.statistics().records_emitted, 1);
    assert_eq!(result.statistics().records_rejected, 1);
    assert_eq!(result.rows.len(), 1, "an invalid row reached the report");
    assert_eq!(result.statistics().errors_in(ErrorCategory::Schema), 1);
    // The file still completed: rejecting a row is not a file failure.
    assert_eq!(result.statistics().files_completed, 1);
    assert_eq!(result.statistics().files_failed, 0);
}

// --- stopping policies ---------------------------------------------------

#[tokio::test]
async fn fail_fast_stops_scheduling_new_work() {
    let mut script = HashMap::new();
    for index in 0..200 {
        script.insert(format!("f{index}.txt"), Behaviour::PluginError);
    }
    let mut settings = overrides();
    settings.workers = Some(WorkerCount::fixed(1).expect("one worker"));
    settings.fail_fast = true;

    let result = run_crawl(200, script, settings, false, CancellationToken::new()).await;

    // With one worker, the first error trips the policy immediately. A handful
    // of tasks may already be in flight, but nothing close to the corpus:
    // "fewer than 200" would also pass if the policy stopped at file 199.
    let dispatched = result.observations.total_dispatches();
    assert!(
        (1..=5).contains(&dispatched),
        "fail-fast dispatched {dispatched} of 200 files; it should stop almost immediately"
    );
    assert_eq!(result.outcome(), CrawlOutcome::PartialSuccess);
}

#[tokio::test]
async fn the_error_threshold_stops_scheduling_new_work() {
    let mut script = HashMap::new();
    for index in 0..200 {
        script.insert(format!("f{index}.txt"), Behaviour::PluginError);
    }
    let mut settings = overrides();
    settings.workers = Some(WorkerCount::fixed(1).expect("one worker"));
    settings.max_errors = Some(5);

    let result = run_crawl(200, script, settings, false, CancellationToken::new()).await;

    // The budget is five, so scheduling must stop shortly after the fifth
    // error rather than merely before the corpus runs out.
    let dispatched = result.observations.total_dispatches();
    assert!(
        (5..=10).contains(&dispatched),
        "threshold of 5 allowed {dispatched} dispatches"
    );
    let errors = result.statistics().qualifying_errors();
    assert!(
        (5..=10).contains(&errors),
        "threshold of 5 recorded {errors} qualifying errors"
    );
    assert_eq!(result.outcome(), CrawlOutcome::PartialSuccess);
}

#[tokio::test]
async fn cancellation_reports_a_cancelled_outcome_and_promotes_output() {
    let cancel = CancellationToken::new();
    cancel.cancel();

    let result = run_crawl(50, HashMap::new(), overrides(), false, cancel).await;

    assert_eq!(result.outcome(), CrawlOutcome::Cancelled);
    // Cancellation is an orderly ending, so the partial report is promoted.
    assert_eq!(result.finalized, Some(PartialOutputPolicy::Promote));
}

// --- fatal failures ------------------------------------------------------

#[tokio::test]
async fn no_startable_worker_is_a_fatal_plugin_failure() {
    let plugin = support::test_plugin();
    let writer = RecordingWriter::default();
    let (_, finalized) = writer.handles();
    let config = EffectiveConfig::resolve(
        &plugin,
        PathBuf::from("/corpus"),
        PathBuf::from("/report.csv"),
        &RunOverrides::default(),
        4,
    );

    let coordinator = CrawlCoordinator::new(
        plugin,
        config,
        Arc::new(ScriptedWalker {
            paths: vec![PathBuf::from("/corpus/a.txt")],
        }),
        Arc::new(FailingFactory),
        Box::new(writer) as Box<dyn ReportWriter>,
        Arc::new(RecordingSink::default()),
        CancellationToken::new(),
    );

    let error = coordinator
        .run()
        .await
        .expect_err("expected a fatal failure");
    assert!(matches!(error, HostFatalError::NoWorkers(_)));
    assert_eq!(error.outcome(), CrawlOutcome::PluginFailure);
    // Nothing was vouched for, so the partial file is not promoted.
    assert_eq!(
        *finalized.lock().expect("finalized"),
        Some(PartialOutputPolicy::RetainTemporary)
    );
}

// --- events --------------------------------------------------------------

#[tokio::test]
async fn lifecycle_events_bracket_the_crawl() {
    let result = run_crawl(
        3,
        HashMap::new(),
        overrides(),
        false,
        CancellationToken::new(),
    )
    .await;
    assert_eq!(
        result.events.first().map(String::as_str),
        Some("crawl_started")
    );
    assert_eq!(
        result.events.last().map(String::as_str),
        Some("crawl_completed")
    );
}

#[tokio::test]
async fn non_matching_files_are_skipped_not_dispatched() {
    let plugin = support::test_plugin();
    let sink = RecordingSink::default();
    let events = sink.handle();
    let factory = ScriptedFactory::new(HashMap::new());
    let observations = factory.observations();

    let config = EffectiveConfig::resolve(
        &plugin,
        PathBuf::from("/corpus"),
        PathBuf::from("/report.csv"),
        &RunOverrides::default(),
        2,
    );
    let coordinator = CrawlCoordinator::new(
        plugin,
        config,
        Arc::new(ScriptedWalker {
            paths: vec![
                PathBuf::from("/corpus/keep.txt"),
                PathBuf::from("/corpus/skip.rs"),
                PathBuf::from("/corpus/no-extension"),
            ],
        }),
        Arc::new(factory),
        Box::new(RecordingWriter::default()) as Box<dyn ReportWriter>,
        Arc::new(sink),
        CancellationToken::new(),
    );
    let report = coordinator.run().await.expect("crawl completed");

    assert_eq!(report.statistics.files_discovered, 3);
    assert_eq!(report.statistics.files_matched, 1);
    assert_eq!(observations.total_dispatches(), 1);
    assert_eq!(count_events(&events, "file_skipped"), 2);
}

// --- degraded and error paths --------------------------------------------

#[tokio::test]
async fn a_report_write_failure_is_counted_without_ending_the_crawl() {
    let plugin = support::test_plugin();
    let writer = FailingWriter::default();
    let finalized = writer.handle();
    let sink = RecordingSink::default();
    let events = sink.handle();

    let config = EffectiveConfig::resolve(
        &plugin,
        PathBuf::from("/corpus"),
        PathBuf::from("/report.csv"),
        &RunOverrides::default(),
        2,
    );
    let coordinator = CrawlCoordinator::new(
        plugin,
        config,
        Arc::new(ScriptedWalker {
            paths: (0..3)
                .map(|index| PathBuf::from(format!("/corpus/f{index}.txt")))
                .collect(),
        }),
        Arc::new(ScriptedFactory::new(HashMap::new())),
        Box::new(writer) as Box<dyn ReportWriter>,
        Arc::new(sink),
        CancellationToken::new(),
    );
    let report = coordinator.run().await.expect("the crawl still completes");

    // Every row failed to write, so every one is a report error, but the crawl
    // finalized rather than aborting mid-way.
    assert_eq!(report.statistics.errors_in(ErrorCategory::Report), 3);
    assert_eq!(report.statistics.records_emitted, 0);
    // A crawl that lost every row must not claim success.
    assert_eq!(report.outcome, CrawlOutcome::PartialSuccess);
    assert_eq!(
        *finalized.lock().expect("finalized"),
        Some(PartialOutputPolicy::Promote),
        "an orderly ending must still promote whatever was written"
    );
    // One row per file, each failing to write, each reported once.
    assert_eq!(count_events(&events, "protocol_violation_detected"), 3);
}

#[tokio::test]
async fn a_pool_that_only_partly_starts_still_runs_the_crawl() {
    let plugin = support::test_plugin();
    // Two workers are configured but only one can ever start.
    let factory = ScriptedFactory::new(HashMap::new()).with_spawn_limit(1);
    let observations = factory.observations();
    let writer = RecordingWriter::default();
    let (rows, _) = writer.handles();

    let overrides = RunOverrides {
        workers: Some(WorkerCount::fixed(2).expect("two workers")),
        ..RunOverrides::default()
    };
    let config = EffectiveConfig::resolve(
        &plugin,
        PathBuf::from("/corpus"),
        PathBuf::from("/report.csv"),
        &overrides,
        4,
    );
    let coordinator = CrawlCoordinator::new(
        plugin,
        config,
        Arc::new(ScriptedWalker {
            paths: (0..6)
                .map(|index| PathBuf::from(format!("/corpus/f{index}.txt")))
                .collect(),
        }),
        Arc::new(factory),
        Box::new(writer) as Box<dyn ReportWriter>,
        Arc::new(RecordingSink::default()),
        CancellationToken::new(),
    );
    let report = coordinator.run().await.expect("degraded pool still runs");

    assert_eq!(report.statistics.files_completed, 6);
    assert_eq!(rows.lock().expect("rows").len(), 6);
    assert_eq!(observations.peak_live.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_failed_replacement_fails_its_task_without_stalling_the_crawl() {
    let mut script = HashMap::new();
    script.insert("f0.txt".to_owned(), Behaviour::Crash);

    let plugin = support::test_plugin();
    // One worker may start; the post-crash replacement cannot.
    let factory = ScriptedFactory::new(script).with_spawn_limit(1);
    let writer = RecordingWriter::default();

    let overrides = RunOverrides {
        workers: Some(WorkerCount::fixed(1).expect("one worker")),
        max_retries: Some(RetryLimit::new(1)),
        ..RunOverrides::default()
    };
    let config = EffectiveConfig::resolve(
        &plugin,
        PathBuf::from("/corpus"),
        PathBuf::from("/report.csv"),
        &overrides,
        4,
    );
    let coordinator = CrawlCoordinator::new(
        plugin,
        config,
        Arc::new(ScriptedWalker {
            paths: (0..4)
                .map(|index| PathBuf::from(format!("/corpus/f{index}.txt")))
                .collect(),
        }),
        Arc::new(factory),
        Box::new(writer) as Box<dyn ReportWriter>,
        Arc::new(RecordingSink::default()),
        CancellationToken::new(),
    );

    // The crawl must terminate rather than hang waiting on a dead pool.
    let report = tokio::time::timeout(std::time::Duration::from_secs(10), coordinator.run())
        .await
        .expect("the crawl must not hang")
        .expect("it completes, degraded");

    // The single slot crashes on f0 and cannot be replaced, so exactly one
    // file reaches a terminal failure and none completes. The remaining files
    // stay queued and are abandoned when the pool empties, which is the
    // degraded behaviour this test pins down.
    assert_eq!(report.statistics.files_failed, 1);
    assert_eq!(report.statistics.files_completed, 0);
    assert_eq!(
        report.statistics.errors_in(ErrorCategory::RuntimeStartup),
        1,
        "the failure must be attributed to the replacement that could not start"
    );
    assert_eq!(report.outcome, CrawlOutcome::PartialSuccess);
}
