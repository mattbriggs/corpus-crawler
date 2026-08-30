//! Automated evidence for the SRS acceptance criteria (SRS section 9).
//!
//! Each test is named `ac_NNN_...` so the requirement, the criterion, and the
//! test share one identifier. These drive the real `crawl` binary against the
//! real fixture plugins: nothing here is mocked, because the criteria are
//! about process behaviour.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Absolute path of the repository root.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

/// Path of a fixture plugin directory.
fn fixture(name: &str) -> PathBuf {
    repo_root().join("plugins/fixtures").join(name)
}

/// One isolated test environment: its own registry, corpus, and report path.
struct Harness {
    home: tempfile::TempDir,
    corpus: PathBuf,
}

impl Harness {
    fn new() -> Self {
        let home = tempfile::tempdir().expect("temp dir");
        let corpus = home.path().join("corpus");
        fs::create_dir_all(&corpus).expect("corpus");
        Self { home, corpus }
    }

    /// Writes a file into the corpus, creating parent directories.
    fn write(&self, relative: &str, contents: &str) -> PathBuf {
        let path = self.corpus.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent");
        }
        fs::write(&path, contents).expect("write");
        path
    }

    fn report(&self) -> PathBuf {
        self.home.path().join("report.csv")
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_crawl"));
        command.env("CRAWL_HOME", self.home.path());
        command.current_dir(repo_root());
        command
    }

    /// Installs a fixture plugin, skipping the runtime check for speed where
    /// the test is not about installation itself.
    fn install(&self, name: &str) -> Output {
        self.command()
            .args(["plugin", "install"])
            .arg(fixture(name))
            .arg("--force")
            .output()
            .expect("install")
    }

    /// Runs a crawl while recording every worker process that starts, and
    /// returns the output together with the number of distinct worker
    /// processes the host created.
    ///
    /// The `echo` fixture writes a file named after its own pid when
    /// `CRAWL_PID_DIR` is set, so the count is observed rather than inferred.
    fn run_counting_workers(&self, plugin: &str, extra: &[&str]) -> (Output, usize) {
        let pid_dir = self.home.path().join("pids");
        fs::create_dir_all(&pid_dir).expect("pid dir");

        let mut command = self.command();
        command
            .env("CRAWL_PID_DIR", &pid_dir)
            .args(["run", plugin, "--input"])
            .arg(&self.corpus)
            .arg("--output")
            .arg(self.report())
            .args(["--quiet", "--overwrite"])
            .args(extra);
        let output = command.output().expect("run");

        let workers = fs::read_dir(&pid_dir).expect("pid dir").count();
        (output, workers)
    }

    /// Runs a crawl with extra arguments and returns the process output.
    fn run(&self, plugin: &str, extra: &[&str]) -> Output {
        let mut command = self.command();
        command
            .args(["run", plugin, "--input"])
            .arg(&self.corpus)
            .arg("--output")
            .arg(self.report())
            .args(["--quiet", "--overwrite"])
            .args(extra);
        command.output().expect("run")
    }

    /// Returns the report's data rows, excluding the header.
    fn report_rows(&self) -> Vec<String> {
        let text = fs::read_to_string(self.report()).expect("report");
        text.lines().skip(1).map(str::to_owned).collect()
    }

    fn report_header(&self) -> String {
        let text = fs::read_to_string(self.report()).expect("report");
        text.lines().next().expect("header").to_owned()
    }
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Extracts a labelled counter from the printed summary.
fn summary_count(output: &Output, label: &str) -> u64 {
    let text = stdout_of(output);
    text.lines()
        .find(|line| line.trim_start().starts_with(label))
        .and_then(|line| line.split(':').nth(1))
        .map(str::trim)
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("summary has no {label:?} counter in:\n{text}"))
}

// --- AC-001 / AC-003 / AC-002 -------------------------------------------

#[test]
fn ac_001_valid_plugin_installs_and_is_listed() {
    let harness = Harness::new();
    let install = harness.install("echo");
    assert!(install.status.success(), "{}", stderr_of(&install));

    let list = harness
        .command()
        .args(["plugin", "list"])
        .output()
        .expect("list");
    assert!(stdout_of(&list).contains("fixture-echo"));
}

#[test]
fn ac_002_unsupported_api_version_is_rejected_before_use() {
    let harness = Harness::new();
    let plugin_dir = harness.home.path().join("future-plugin");
    fs::create_dir_all(&plugin_dir).expect("dir");
    fs::write(
        plugin_dir.join("plugin.yaml"),
        r#"
api_version: "99"
plugin:
  name: future
  version: "1.0.0"
runtime:
  type: python
  command: ["python3", "worker.py"]
input:
  extensions: [".txt"]
output:
  format: records
  schema:
    a:
      type: string
      required: true
"#,
    )
    .expect("manifest");

    let output = harness
        .command()
        .args(["plugin", "install"])
        .arg(&plugin_dir)
        .output()
        .expect("install");
    assert!(!output.status.success());
    assert!(
        stderr_of(&output).contains("unsupported api_version"),
        "{}",
        stderr_of(&output)
    );
}

#[test]
fn ac_003_inspect_shows_effective_registered_metadata() {
    let harness = Harness::new();
    harness.install("echo");
    let output = harness
        .command()
        .args(["plugin", "inspect", "fixture-echo"])
        .output()
        .expect("inspect");
    let rendered = stdout_of(&output);
    assert!(rendered.contains("1.0.0"), "version missing");
    assert!(rendered.contains(".txt"), "extensions missing");
    assert!(rendered.contains("filename"), "schema missing");
    assert!(rendered.contains("command"), "runtime missing");
}

// --- discovery and selection --------------------------------------------

#[test]
fn ac_004_only_matching_extensions_are_submitted() {
    let harness = Harness::new();
    harness.install("echo");
    harness.write("top.txt", "one\n");
    harness.write("a/b/deep.txt", "two\n");
    harness.write("a/skip.rs", "ignored\n");
    harness.write("a/b/skip.md", "ignored\n");

    let output = harness.run("fixture-echo", &[]);
    assert_eq!(summary_count(&output, "files discovered"), 4);
    assert_eq!(summary_count(&output, "files matched"), 2);
    assert_eq!(harness.report_rows().len(), 2);
}

// --- worker pool ---------------------------------------------------------

#[test]
fn ac_007_worker_process_is_reused() {
    let harness = Harness::new();
    harness.install("echo");
    for index in 0..25 {
        harness.write(&format!("f{index}.txt"), "line\n");
    }

    let (output, worker_processes) =
        harness.run_counting_workers("fixture-echo", &["--workers", "1"]);

    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(summary_count(&output, "files completed"), 25);
    // Exactly one interpreter served all twenty-five files. Starting one per
    // file would produce twenty-five distinct pids here, which is the failure
    // this criterion exists to rule out (REQ-005, NFR-004).
    assert_eq!(
        worker_processes, 1,
        "expected one reused worker process, found {worker_processes}"
    );
    assert_eq!(summary_count(&output, "worker restarts"), 0);
}

#[test]
fn ac_008_worker_count_is_bounded() {
    let harness = Harness::new();
    harness.install("echo");
    for index in 0..40 {
        harness.write(&format!("f{index}.txt"), "line\n");
    }

    let (output, worker_processes) =
        harness.run_counting_workers("fixture-echo", &["--workers", "3"]);

    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(summary_count(&output, "files completed"), 40);
    // No crash occurred, so no replacements were needed: the host must have
    // created exactly the configured number of processes and no more, however
    // many files were waiting (REQ-052, REQ-055).
    assert_eq!(
        worker_processes, 3,
        "configured 3 workers for 40 files but {worker_processes} processes started"
    );
    assert_eq!(summary_count(&output, "worker restarts"), 0);
}

#[test]
fn ac_006_queue_applies_backpressure() {
    const CAPACITY: usize = 4;
    const WORKERS: usize = 2;

    let harness = Harness::new();
    harness.install("echo");
    for index in 0..400 {
        harness.write(&format!("f{index:04}.txt"), "line\n");
    }
    let log = harness.home.path().join("trace.json");

    let mut command = harness.command();
    command
        .args(["run", "fixture-echo", "--input"])
        .arg(&harness.corpus)
        .arg("--output")
        .arg(harness.report())
        .arg("--log")
        .arg(&log)
        .args([
            "-vv",
            "--log-format",
            "json",
            "--overwrite",
            "--workers",
            &WORKERS.to_string(),
            "--queue-capacity",
            &CAPACITY.to_string(),
        ]);
    let output = command.output().expect("run");
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(summary_count(&output, "files completed"), 400);

    // Replay the log and track how many files have been matched but not yet
    // dispatched. That difference is the work queue's depth, so its peak is
    // direct evidence of the bound rather than an inference from the crawl
    // having finished at all.
    let text = fs::read_to_string(&log).expect("trace log");
    let mut queued: i64 = 0;
    let mut peak: i64 = 0;
    for line in text.lines() {
        if line.contains("\"file_matched\"") {
            queued += 1;
            peak = peak.max(queued);
        } else if line.contains("\"file_processing_started\"") {
            queued -= 1;
        }
    }

    // The dispatcher may hold one task beyond the channel's capacity while each
    // worker holds the one it is running, so the ceiling is capacity plus
    // workers plus one. Without backpressure this figure would approach 400.
    let ceiling = (CAPACITY + WORKERS + 1) as i64;
    assert!(
        peak <= ceiling,
        "queue depth reached {peak} with a capacity of {CAPACITY}; \
         discovery is not being throttled (REQ-050, REQ-051)"
    );
    assert!(
        peak > 0,
        "no queue depth was observed; the log may have changed shape"
    );
}

// --- response handling ---------------------------------------------------

#[test]
fn ac_009_zero_rows_is_a_successful_file() {
    let harness = Harness::new();
    harness.install("echo");
    harness.write("empty.txt", "");
    let output = harness.run("fixture-echo", &[]);
    assert_eq!(summary_count(&output, "files completed"), 1);
    assert_eq!(summary_count(&output, "records emitted"), 0);
    assert!(harness.report_rows().is_empty());
}

#[test]
fn ac_010_multiple_rows_are_all_written() {
    let harness = Harness::new();
    harness.install("echo");
    harness.write("three.txt", "one\ntwo\nthree\n");
    let output = harness.run("fixture-echo", &[]);
    assert_eq!(summary_count(&output, "records emitted"), 3);
    assert_eq!(harness.report_rows().len(), 3);
}

#[test]
fn ac_011_csv_columns_follow_schema_order() {
    let harness = Harness::new();
    harness.install("echo");
    harness.write("a.txt", "value\n");
    harness.run("fixture-echo", &[]);
    assert_eq!(harness.report_header(), "filename,line,text");
}

#[test]
fn ac_012_invalid_rows_are_rejected_and_logged() {
    let harness = Harness::new();
    harness.install("schema_violation");
    harness.write("a.txt", "content\n");

    let output = harness.run("fixture-schema_violation", &[]);
    // The fixture returns one valid row and two invalid rows per file.
    assert_eq!(summary_count(&output, "records emitted"), 1);
    assert_eq!(summary_count(&output, "records rejected"), 2);
    assert_eq!(harness.report_rows().len(), 1);
    assert!(stdout_of(&output).contains("schema"));
}

#[test]
fn ac_028_nested_values_are_rejected() {
    let harness = Harness::new();
    let plugin_dir = harness.home.path().join("nested");
    fs::create_dir_all(&plugin_dir).expect("dir");
    fs::write(
        plugin_dir.join("plugin.yaml"),
        r#"
api_version: "1"
plugin:
  name: nested-rows
  version: "1.0.0"
runtime:
  type: python
  command: ["python3", "worker.py"]
input:
  extensions: [".txt"]
output:
  format: records
  schema:
    filename:
      type: string
      required: true
"#,
    )
    .expect("manifest");
    fs::write(
        plugin_dir.join("worker.py"),
        r#"import json, sys
for raw in sys.stdin:
    raw = raw.strip()
    if not raw:
        continue
    request = json.loads(raw)
    op = request.get("op", "process")
    if op == "shutdown":
        break
    if op == "handshake":
        sys.stdout.write(json.dumps({"id": request["id"], "status": "ready", "api_version": "1"}) + "\n")
    else:
        sys.stdout.write(json.dumps({"id": request["id"], "status": "ok",
                                     "rows": [{"filename": {"nested": "object"}}]}) + "\n")
    sys.stdout.flush()
"#,
    )
    .expect("worker");

    harness
        .command()
        .args(["plugin", "install"])
        .arg(&plugin_dir)
        .arg("--force")
        .output()
        .expect("install");
    harness.write("a.txt", "x\n");

    let output = harness.run("nested-rows", &[]);
    assert_eq!(summary_count(&output, "records rejected"), 1);
    assert_eq!(summary_count(&output, "records emitted"), 0);
}

// --- failure isolation ---------------------------------------------------

#[test]
fn ac_013_worker_crash_isolated_and_replaced() {
    let harness = Harness::new();
    harness.install("crash");
    harness.write("boom.txt", "trigger\n");
    for index in 0..5 {
        harness.write(&format!("ok{index}.txt"), "fine\n");
    }

    let output = harness.run("fixture-crash", &["--workers", "1", "--max-retries", "1"]);
    // The host survived, the crashing file failed, everything else completed,
    // and a replacement worker was started.
    assert_eq!(summary_count(&output, "files failed"), 1);
    assert_eq!(summary_count(&output, "files completed"), 5);

    // One worker, one crashing file, one retry: the slot is replaced after the
    // first crash and again after the retry crashes, then serves the rest. An
    // open-ended "at least one" would also accept a host that restarted on
    // every file.
    let restarts = summary_count(&output, "worker restarts");
    assert!(
        (1..=3).contains(&restarts),
        "expected one or two replacements, saw {restarts}"
    );
}

#[test]
fn ac_014_timeout_invalidates_worker_and_crawl_continues() {
    let harness = Harness::new();
    harness.install("timeout");
    harness.write("hang.txt", "trigger\n");
    for index in 0..3 {
        harness.write(&format!("ok{index}.txt"), "fine\n");
    }

    let output = harness.run("fixture-timeout", &["--workers", "1", "--timeout", "1"]);
    assert_eq!(summary_count(&output, "timeouts"), 1);
    assert_eq!(summary_count(&output, "files failed"), 1);
    assert_eq!(summary_count(&output, "files completed"), 3);
}

#[test]
fn ac_015_malformed_stdout_is_a_protocol_error() {
    let harness = Harness::new();
    harness.install("malformed");
    harness.write("bad.txt", "trigger\n");
    harness.write("good.txt", "fine\n");

    let output = harness.run("fixture-malformed", &["--workers", "1"]);
    assert!(
        stdout_of(&output).contains("protocol"),
        "{}",
        stdout_of(&output)
    );
    assert_eq!(summary_count(&output, "files failed"), 1);
    assert_eq!(harness.report_rows().len(), 1);
}

#[test]
fn ac_016_stderr_does_not_invalidate_a_valid_response() {
    let harness = Harness::new();
    harness.install("stderr");
    harness.write("a.txt", "content\n");

    let output = harness.run("fixture-stderr", &[]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(summary_count(&output, "files completed"), 1);
    assert_eq!(summary_count(&output, "records emitted"), 1);
}

#[test]
fn ac_017_file_failure_does_not_stop_the_crawl() {
    let harness = Harness::new();
    harness.install("plugin_error");
    harness.write("fail.txt", "trigger\n");
    for index in 0..20 {
        harness.write(&format!("ok{index}.txt"), "fine\n");
    }

    let output = harness.run("fixture-plugin_error", &["--workers", "2"]);
    assert_eq!(summary_count(&output, "files failed"), 1);
    assert_eq!(summary_count(&output, "files completed"), 20);
}

#[test]
fn ac_018_fail_fast_stops_scheduling() {
    let harness = Harness::new();
    harness.install("plugin_error");
    harness.write("fail.txt", "trigger\n");
    for index in 0..200 {
        harness.write(&format!("ok{index}.txt"), "fine\n");
    }

    let output = harness.run("fixture-plugin_error", &["--workers", "1", "--fail-fast"]);
    // Scheduling stops promptly, so far fewer than all files are processed.
    assert!(
        summary_count(&output, "files completed") < 200,
        "fail-fast did not stop scheduling"
    );
}

#[test]
fn ac_019_maximum_error_threshold_stops_scheduling() {
    let harness = Harness::new();
    harness.install("schema_violation");
    for index in 0..100 {
        harness.write(&format!("f{index}.txt"), "content\n");
    }

    // Each file produces two schema errors, so ten errors is five files' worth.
    let output = harness.run(
        "fixture-schema_violation",
        &["--workers", "1", "--max-errors", "10"],
    );

    // Stopping "before file 100" would be satisfied by stopping at file 99.
    // The budget says scheduling must stop at about the fifth file.
    let completed = summary_count(&output, "files completed");
    assert!(
        (5..=8).contains(&completed),
        "a budget of 10 errors allowed {completed} files to complete"
    );
    let rejected = summary_count(&output, "records rejected");
    assert!(
        (10..=16).contains(&rejected),
        "a budget of 10 errors recorded {rejected} rejections"
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "the outcome is partial success"
    );
}

// --- outputs and outcomes ------------------------------------------------

#[test]
fn ac_021_report_rows_are_not_fully_buffered() {
    let harness = Harness::new();
    harness.install("echo");
    for index in 0..300 {
        harness.write(&format!("f{index}.txt"), "a\nb\nc\n");
    }
    let output = harness.run("fixture-echo", &["--workers", "2", "--queue-capacity", "8"]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(harness.report_rows().len(), 900);
}

#[test]
fn ac_022_concurrent_workers_produce_a_structurally_valid_csv() {
    let harness = Harness::new();
    harness.install("echo");
    for index in 0..120 {
        harness.write(&format!("f{index}.txt"), "one,with comma\n\"quoted\"\n");
    }
    harness.run("fixture-echo", &["--workers", "4"]);

    // Parse the report back: every row must have exactly the header's arity.
    let text = fs::read_to_string(harness.report()).expect("report");
    let mut reader = csv::Reader::from_reader(text.as_bytes());
    let width = reader.headers().expect("headers").len();
    let mut rows = 0;
    for record in reader.records() {
        assert_eq!(record.expect("valid row").len(), width);
        rows += 1;
    }
    assert_eq!(rows, 240);
}

#[test]
fn ac_023_existing_output_is_protected() {
    let harness = Harness::new();
    harness.install("echo");
    harness.write("a.txt", "x\n");
    fs::write(harness.report(), "pre-existing\n").expect("seed");

    let mut command = harness.command();
    command
        .args(["run", "fixture-echo", "--input"])
        .arg(&harness.corpus)
        .arg("--output")
        .arg(harness.report())
        .arg("--quiet");
    let output = command.output().expect("run");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        fs::read_to_string(harness.report()).expect("report"),
        "pre-existing\n",
        "the existing report was modified"
    );
}

#[test]
fn ac_024_summary_reports_required_counters() {
    let harness = Harness::new();
    harness.install("echo");
    harness.write("a.txt", "one\ntwo\n");
    let output = harness.run("fixture-echo", &[]);
    let summary = stdout_of(&output);
    for label in [
        "files discovered",
        "files matched",
        "files completed",
        "files failed",
        "records emitted",
        "timeouts",
        "worker restarts",
        "duration",
    ] {
        assert!(summary.contains(label), "summary is missing {label:?}");
    }
}

#[test]
fn ac_025_files_removed_after_discovery_fail_attributably() {
    let harness = Harness::new();
    // This fixture deletes its sibling files as it processes, which makes the
    // discovery-to-open race deterministic: every file is queued before any is
    // processed, so the files deleted by the first one are already in flight.
    harness.install("vanish");
    for index in 0..4 {
        harness.write(&format!("f{index}.txt"), "content\n");
    }

    let output = harness.run("fixture-vanish", &["--workers", "1"]);

    // Exactly one file survived to be processed; the other three vanished
    // between discovery and opening.
    assert_eq!(summary_count(&output, "files matched"), 4);
    assert_eq!(summary_count(&output, "files completed"), 1);
    assert_eq!(summary_count(&output, "files failed"), 3);

    // The outcome is partial success, not a fatal error: a file disappearing
    // mid-crawl is an ordinary attributable failure, not a broken snapshot
    // guarantee the host never offered (REQ-171, REQ-172).
    assert_eq!(output.status.code(), Some(1));

    // Each failure names the file it belongs to, which is the "attributably"
    // half of this criterion (NFR-051).
    let summary = stdout_of(&output);
    assert!(
        summary.contains("plugin_processing"),
        "failures were not categorised: {summary}"
    );
}

#[test]
fn ac_026_preflight_failure_dispatches_no_work() {
    let harness = Harness::new();
    harness.write("a.txt", "x\n");

    let output = harness.run("not-installed", &[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(!harness.report().exists(), "a report was created anyway");

    // An unreadable input directory is also caught before any dispatch.
    let mut command = harness.command();
    command
        .args(["run", "fixture-echo", "--input"])
        .arg(harness.home.path().join("does-not-exist"))
        .arg("--output")
        .arg(harness.report())
        .arg("--quiet");
    let output = command.output().expect("run");
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn ac_027_partial_success_is_distinguishable() {
    let harness = Harness::new();
    harness.install("plugin_error");
    harness.write("fail.txt", "trigger\n");
    harness.write("ok.txt", "fine\n");

    let output = harness.run("fixture-plugin_error", &["--workers", "1"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "partial success must be distinguishable from success and fatal failure"
    );
    assert!(stdout_of(&output).contains("partial_success"));
}

#[test]
fn ac_021_zero_result_crawl_still_produces_a_header_only_report() {
    let harness = Harness::new();
    harness.install("echo");
    harness.write("ignored.rs", "no matching extension\n");

    let output = harness.run("fixture-echo", &[]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(harness.report_header(), "filename,line,text");
    assert!(harness.report_rows().is_empty());
}

#[test]
fn plugin_remove_unregisters_without_deleting_files() {
    let harness = Harness::new();
    harness.install("echo");
    let output = harness
        .command()
        .args(["plugin", "remove", "fixture-echo"])
        .output()
        .expect("remove");
    assert!(output.status.success());
    assert!(
        fixture("echo").join("plugin.yaml").exists(),
        "plugin files were deleted"
    );

    let list = harness
        .command()
        .args(["plugin", "list"])
        .output()
        .expect("list");
    assert!(stdout_of(&list).contains("no plugins installed"));
}

// --- streaming and cancellation ------------------------------------------

#[test]
fn ac_005_processing_begins_before_discovery_finishes() {
    let harness = Harness::new();
    harness.install("slow");
    for index in 0..400 {
        harness.write(&format!("f{index:04}.txt"), "line\n");
    }
    let log = harness.home.path().join("trace.json");

    let mut command = harness.command();
    command
        .args(["run", "fixture-slow", "--input"])
        .arg(&harness.corpus)
        .arg("--output")
        .arg(harness.report())
        .arg("--log")
        .arg(&log)
        .args([
            "-vv",
            "--log-format",
            "json",
            "--workers",
            "2",
            "--overwrite",
        ]);
    let output = command.output().expect("run");
    assert!(output.status.success(), "{}", stderr_of(&output));

    // Traversal and processing must interleave: the first file is dispatched
    // long before the last file is discovered. If discovery were materialised
    // up front, every discovery would precede every dispatch.
    let text = fs::read_to_string(&log).expect("trace log");
    let lines: Vec<&str> = text.lines().collect();
    let first_dispatch = lines
        .iter()
        .position(|line| line.contains("file_processing_started"))
        .expect("no dispatch was logged");
    let last_discovery = lines
        .iter()
        .rposition(|line| line.contains("file_discovered"))
        .expect("no discovery was logged");
    assert!(
        first_dispatch < last_discovery,
        "discovery completed before the first dispatch: traversal is not streaming"
    );
}

#[cfg(unix)]
#[test]
fn ac_020_graceful_cancellation_stops_scheduling_and_finalizes_output() {
    use std::io::Read;
    use std::time::{Duration, Instant};

    let harness = Harness::new();
    harness.install("slow");
    for index in 0..2000 {
        harness.write(&format!("f{index:04}.txt"), "line\n");
    }

    let mut command = harness.command();
    command
        .args(["run", "fixture-slow", "--input"])
        .arg(&harness.corpus)
        .arg("--output")
        .arg(harness.report())
        .args(["--quiet", "--overwrite", "--workers", "2"])
        .stdout(std::process::Stdio::piped());
    let mut child = command.spawn().expect("spawn");

    // Let the crawl get properly under way, then ask it to stop.
    std::thread::sleep(Duration::from_millis(1500));
    let status = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("send SIGINT");
    assert!(status.success(), "could not signal the crawl");

    let deadline = Instant::now() + Duration::from_secs(30);
    let exit = loop {
        if let Some(status) = child.try_wait().expect("wait") {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "crawl did not stop after cancellation"
        );
        std::thread::sleep(Duration::from_millis(50));
    };

    let mut summary = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut summary);
    }

    assert_eq!(
        exit.code(),
        Some(3),
        "cancellation must have its own exit code"
    );
    assert!(
        summary.contains("cancelled"),
        "summary did not report cancellation: {summary}"
    );

    // Output resources are finalized: the partial report is promoted, valid,
    // and holds fewer rows than the corpus would have produced.
    assert!(
        harness.report().exists(),
        "the partial report was not finalized"
    );
    assert_eq!(harness.report_header(), "filename,line,text");
    let rows = harness.report_rows().len();
    assert!(
        rows < 2000,
        "cancellation did not stop scheduling ({rows} rows)"
    );
}

#[test]
fn a_relative_input_root_yields_paths_workers_can_open() {
    // Workers run with the plugin directory as their working directory, so a
    // relative path from the host's own directory would not resolve for them.
    // This regression guards REQ-162: the path a worker receives must be one
    // it can open.
    let harness = Harness::new();
    harness.install("echo");
    harness.write("a.txt", "one line\n");

    let relative = harness
        .corpus
        .strip_prefix(harness.home.path())
        .expect("corpus is under the harness home");

    let mut command = Command::new(env!("CARGO_BIN_EXE_crawl"));
    command
        .env("CRAWL_HOME", harness.home.path())
        // Run from the harness home so the input argument really is relative.
        .current_dir(harness.home.path())
        .args(["run", "fixture-echo", "--input"])
        .arg(relative)
        .arg("--output")
        .arg(harness.report())
        .args(["--quiet", "--overwrite"]);
    let output = command.output().expect("run");

    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(summary_count(&output, "files completed"), 1);
    assert_eq!(summary_count(&output, "files failed"), 0);
    assert_eq!(harness.report_rows().len(), 1);

    // The recorded path is absolute, which is what makes it openable.
    assert!(
        harness.report_rows()[0].starts_with('/'),
        "worker received a relative path: {:?}",
        harness.report_rows()[0]
    );
}
