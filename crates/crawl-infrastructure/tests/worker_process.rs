//! Worker process adapter tests against real subprocesses.
//!
//! The adapter's whole job is to behave correctly when a child process
//! misbehaves, so these drive real Python processes rather than fakes.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crawl_application::ports::worker::{WorkerFactory, WorkerFailure, WorkerStartupError};
use crawl_domain::ids::{RequestId, WorkerId};
use crawl_domain::manifest::PluginManifest;
use crawl_domain::plugin::Plugin;
use crawl_infrastructure::ProcessWorkerFactory;
use crawl_protocol::{Request, Response};

/// Unwraps the error of a spawn attempt.
///
/// `expect_err` is unavailable because `Box<dyn WorkerHandle>` is not `Debug`,
/// which is the right trade: a handle owns a live process and should not be
/// casually formatted.
fn spawn_error(
    result: Result<Box<dyn crawl_application::ports::worker::WorkerHandle>, WorkerStartupError>,
) -> WorkerStartupError {
    match result {
        Ok(_) => panic!("expected the spawn to fail"),
        Err(error) => error,
    }
}

/// Writes a plugin directory whose worker runs `body`, and returns the plugin.
fn plugin_running(body: &str) -> (tempfile::TempDir, Plugin) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("worker.py"), body).expect("worker");
    let manifest = r#"
api_version: "1"
plugin:
  name: probe
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
"#;
    let plugin = PluginManifest::parse(manifest, Path::new("plugin.yaml"))
        .expect("parses")
        .validate(dir.path())
        .expect("validates");
    (dir, plugin)
}

/// A worker that handshakes correctly and then behaves as `on_process` says.
fn worker_source(handshake: &str, on_process: &str) -> String {
    format!(
        r#"import json, sys

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

for raw in sys.stdin:
    raw = raw.strip()
    if not raw:
        continue
    request = json.loads(raw)
    op = request.get("op", "process")
    if op == "shutdown":
        break
    if op == "handshake":
        {handshake}
        continue
    {on_process}
"#
    )
}

fn process_request(id: u64) -> Request {
    Request::process(RequestId::new(id), "/data/a.txt")
}

#[tokio::test]
async fn a_healthy_worker_handshakes_and_serves_many_requests() {
    let (_dir, plugin) = plugin_running(&worker_source(
        r#"send({"id": request["id"], "status": "ready", "api_version": "1"})"#,
        r#"send({"id": request["id"], "status": "ok", "rows": [{"a": request["filepath"]}]})"#,
    ));

    let mut worker = ProcessWorkerFactory
        .spawn(&plugin, WorkerId::new(0))
        .await
        .expect("worker starts");
    assert!(worker.pid().is_some());
    assert_eq!(worker.id(), WorkerId::new(0));

    // Reuse is the point of a persistent worker: many requests, one process.
    for id in 1..=5 {
        let response = worker
            .dispatch(process_request(id), Some(Duration::from_secs(10)))
            .await
            .expect("dispatch");
        assert_eq!(response.id(), RequestId::new(id));
        assert_eq!(response.status(), "ok");
    }
    worker.shutdown().await;
}

#[tokio::test]
async fn a_missing_interpreter_is_a_startup_failure() {
    let (_dir, mut plugin) = plugin_running("");
    plugin.runtime.command = vec!["definitely-not-an-interpreter".into(), "worker.py".into()];

    let error = spawn_error(ProcessWorkerFactory.spawn(&plugin, WorkerId::new(0)).await);
    assert!(matches!(error, WorkerStartupError::Spawn { .. }));
    assert!(error.to_string().contains("cannot start plugin runtime"));
}

#[tokio::test]
async fn a_worker_that_dies_before_handshaking_fails_startup_with_its_stderr() {
    let (_dir, plugin) = plugin_running(
        "import sys\nsys.stderr.write('cannot import my_dependency\\n')\nsys.exit(1)\n",
    );

    let error = spawn_error(ProcessWorkerFactory.spawn(&plugin, WorkerId::new(0)).await);
    assert!(matches!(error, WorkerStartupError::Handshake(_)));
    // The plugin's own diagnostic is the actionable part (NFR-052).
    assert!(
        error.to_string().contains("cannot import my_dependency"),
        "startup failure lost the worker's stderr: {error}"
    );
}

#[tokio::test]
async fn a_version_mismatch_at_handshake_is_rejected() {
    let (_dir, plugin) = plugin_running(&worker_source(
        r#"send({"id": request["id"], "status": "ready", "api_version": "99"})"#,
        r#"send({"id": request["id"], "status": "ok", "rows": []})"#,
    ));

    let error = spawn_error(ProcessWorkerFactory.spawn(&plugin, WorkerId::new(0)).await);
    assert!(error.to_string().contains("99"));
}

#[tokio::test]
async fn a_non_ready_handshake_reply_is_rejected() {
    let (_dir, plugin) = plugin_running(&worker_source(
        r#"send({"id": request["id"], "status": "ok", "rows": []})"#,
        r#"send({"id": request["id"], "status": "ok", "rows": []})"#,
    ));

    let error = spawn_error(ProcessWorkerFactory.spawn(&plugin, WorkerId::new(0)).await);
    assert!(error.to_string().contains("ready"));
}

#[tokio::test]
async fn a_worker_that_exits_mid_request_is_detected_as_a_crash() {
    let (_dir, plugin) = plugin_running(&worker_source(
        r#"send({"id": request["id"], "status": "ready", "api_version": "1"})"#,
        "sys.stderr.write('going down\\n')\n    sys.exit(3)",
    ));

    let mut worker = ProcessWorkerFactory
        .spawn(&plugin, WorkerId::new(0))
        .await
        .expect("worker starts");
    let failure = worker
        .dispatch(process_request(1), Some(Duration::from_secs(10)))
        .await
        .expect_err("must report a crash");

    assert!(matches!(failure, WorkerFailure::Crash(_)));
    assert_eq!(
        failure.category(),
        crawl_domain::errors::ErrorCategory::WorkerCrash
    );
    assert!(failure.invalidates_worker());
    assert!(
        failure.to_string().contains("going down"),
        "crash report lost the worker's stderr: {failure}"
    );
}

#[tokio::test]
async fn an_unresponsive_worker_hits_the_deadline_and_is_killed() {
    let (_dir, plugin) = plugin_running(&worker_source(
        r#"send({"id": request["id"], "status": "ready", "api_version": "1"})"#,
        "import time\n    time.sleep(300)",
    ));

    let mut worker = ProcessWorkerFactory
        .spawn(&plugin, WorkerId::new(0))
        .await
        .expect("worker starts");
    let failure = worker
        .dispatch(process_request(1), Some(Duration::from_millis(300)))
        .await
        .expect_err("must time out");

    assert!(matches!(failure, WorkerFailure::Timeout { .. }));
    assert_eq!(
        failure.category(),
        crawl_domain::errors::ErrorCategory::Timeout
    );
    // The deadline is reported as configured, so the operator can tell which
    // limit was hit when several are in play.
    let WorkerFailure::Timeout { seconds } = failure else {
        unreachable!("checked above");
    };
    assert!(
        (seconds - 0.3).abs() < 1e-6,
        "reported deadline was {seconds}s"
    );

    // The hung process must not survive the crawl (REQ-071).
    let pid = worker.pid().expect("a running worker has a pid");
    worker.kill().await;
    assert!(!is_alive(pid), "the hung worker survived being killed");
}

#[tokio::test]
async fn a_mismatched_response_id_is_a_protocol_failure() {
    let (_dir, plugin) = plugin_running(&worker_source(
        r#"send({"id": request["id"], "status": "ready", "api_version": "1"})"#,
        r#"send({"id": request["id"] + 1000, "status": "ok", "rows": []})"#,
    ));

    let mut worker = ProcessWorkerFactory
        .spawn(&plugin, WorkerId::new(0))
        .await
        .expect("worker starts");
    let failure = worker
        .dispatch(process_request(1), Some(Duration::from_secs(10)))
        .await
        .expect_err("correlation must be checked");

    assert!(matches!(failure, WorkerFailure::Protocol(_)));
    assert!(failure.to_string().contains("outstanding"));
}

#[tokio::test]
async fn malformed_protocol_output_is_a_protocol_failure() {
    let (_dir, plugin) = plugin_running(&worker_source(
        r#"send({"id": request["id"], "status": "ready", "api_version": "1"})"#,
        r#"sys.stdout.write("not json\n")
    sys.stdout.flush()"#,
    ));

    let mut worker = ProcessWorkerFactory
        .spawn(&plugin, WorkerId::new(0))
        .await
        .expect("worker starts");
    let failure = worker
        .dispatch(process_request(1), Some(Duration::from_secs(10)))
        .await
        .expect_err("malformed output must fail");
    assert!(matches!(failure, WorkerFailure::Protocol(_)));
}

#[tokio::test]
async fn stderr_is_captured_without_disturbing_a_valid_response() {
    let (_dir, plugin) = plugin_running(&worker_source(
        r#"send({"id": request["id"], "status": "ready", "api_version": "1"})"#,
        r#"sys.stderr.write("diagnostic one\n")
    sys.stderr.write("diagnostic two\n")
    sys.stderr.flush()
    send({"id": request["id"], "status": "ok", "rows": []})"#,
    ));

    let mut worker = ProcessWorkerFactory
        .spawn(&plugin, WorkerId::new(0))
        .await
        .expect("worker starts");
    let response = worker
        .dispatch(process_request(1), Some(Duration::from_secs(10)))
        .await
        .expect("stderr must not invalidate the response");
    assert!(matches!(response, Response::Ok { .. }));

    // Diagnostics reach the host separately, and draining clears them.
    tokio::time::sleep(Duration::from_millis(150)).await;
    let diagnostics = worker.take_diagnostics();
    assert!(
        diagnostics
            .iter()
            .any(|line| line.contains("diagnostic one")),
        "stderr was not captured: {diagnostics:?}"
    );
    assert!(worker.take_diagnostics().is_empty(), "draining must clear");
    worker.shutdown().await;
}

#[tokio::test]
async fn declared_environment_variables_reach_the_worker() {
    let (dir, mut plugin) = plugin_running(&worker_source(
        r#"send({"id": request["id"], "status": "ready", "api_version": "1"})"#,
        r#"import os
    send({"id": request["id"], "status": "ok",
          "rows": [{"a": os.environ.get("MY_SETTING", "unset")}]})"#,
    ));
    let _ = &dir;
    plugin.runtime.environment = vec![("MY_SETTING".into(), "configured".into())];

    let mut worker = ProcessWorkerFactory
        .spawn(&plugin, WorkerId::new(0))
        .await
        .expect("worker starts");
    let Response::Ok { rows, .. } = worker
        .dispatch(process_request(1), Some(Duration::from_secs(10)))
        .await
        .expect("dispatch")
    else {
        panic!("expected an ok response");
    };
    assert_eq!(
        rows[0].get("a"),
        Some(&crawl_domain::record::RawValue::String("configured".into()))
    );
    worker.shutdown().await;
}

#[tokio::test]
async fn a_worker_that_ignores_shutdown_is_still_reaped() {
    // This worker never exits on its own; closing stdin and the grace period
    // must still bring it down rather than leaking a process.
    let (_dir, plugin) = plugin_running(
        r#"import json, sys, time

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

line = sys.stdin.readline()
request = json.loads(line)
send({"id": request["id"], "status": "ready", "api_version": "1"})
time.sleep(300)
"#,
    );

    let mut worker = ProcessWorkerFactory
        .spawn(&plugin, WorkerId::new(0))
        .await
        .expect("worker starts");
    let pid = worker.pid().expect("a running worker has a pid");
    assert!(
        is_alive(pid),
        "the worker should be running before shutdown"
    );

    tokio::time::timeout(Duration::from_secs(20), worker.shutdown())
        .await
        .expect("shutdown must not hang");

    // Not hanging is only half the requirement. A worker that ignored the
    // request must actually be gone, or a long-running host would leak one
    // process per crawl.
    assert!(
        !is_alive(pid),
        "worker {pid} survived shutdown; the grace period did not kill it"
    );
}

/// Returns whether a process exists, using a null signal.
fn is_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// The plugin root is the worker's working directory, so relative entry points
/// resolve without the author spelling out an absolute path.
#[tokio::test]
async fn the_working_directory_is_the_plugin_root() {
    let (dir, plugin) = plugin_running(&worker_source(
        r#"send({"id": request["id"], "status": "ready", "api_version": "1"})"#,
        r#"import os
    send({"id": request["id"], "status": "ok", "rows": [{"a": os.getcwd()}]})"#,
    ));

    let mut worker = ProcessWorkerFactory
        .spawn(&plugin, WorkerId::new(0))
        .await
        .expect("worker starts");
    let Response::Ok { rows, .. } = worker
        .dispatch(process_request(1), Some(Duration::from_secs(10)))
        .await
        .expect("dispatch")
    else {
        panic!("expected an ok response");
    };

    let reported = match rows[0].get("a") {
        Some(crawl_domain::record::RawValue::String(text)) => PathBuf::from(text),
        other => panic!("unexpected value: {other:?}"),
    };
    assert_eq!(
        reported.canonicalize().expect("canonical"),
        dir.path().canonicalize().expect("canonical")
    );
    worker.shutdown().await;
}
