//! Persistent Python worker processes over stdin/stdout JSONL.
//!
//! # Lifecycle
//!
//! `spawn` starts the process, drains `stderr` onto a side channel, and
//! performs the handshake that proves the runtime started and the plugin
//! loaded (REQ-014, REQ-015). After that the worker is reused for many files
//! (REQ-005, REQ-061), one request at a time (REQ-062).
//!
//! # Failure handling
//!
//! A closed stdout means the process died (REQ-067). An expired deadline kills
//! the process rather than waiting on it (REQ-071). A response whose id does
//! not match the outstanding request is a protocol violation (REQ-163).

use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use crawl_application::ports::worker::{
    WorkerFactory, WorkerFailure, WorkerHandle, WorkerStartupError,
};
use crawl_domain::ids::{RequestId, WorkerId};
use crawl_domain::plugin::Plugin;
use crawl_protocol::{
    decode_response, encode_request, ProtocolError, Request, RequestOp, Response,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use super::stderr::{spawn_reader, DiagnosticBuffer};

/// Time allowed for the startup handshake before the worker is declared unusable.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// Time allowed for a worker to exit after a `shutdown` request.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// Starts worker processes from a plugin's runtime specification.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessWorkerFactory;

#[async_trait]
impl WorkerFactory for ProcessWorkerFactory {
    async fn spawn(
        &self,
        plugin: &Plugin,
        id: WorkerId,
    ) -> Result<Box<dyn WorkerHandle>, WorkerStartupError> {
        let spec = &plugin.runtime;
        let mut command = Command::new(spec.program());
        command
            .args(spec.args())
            .current_dir(plugin.working_directory())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (name, value) in &spec.environment {
            command.env(name, value);
        }
        // Unbuffered stdio keeps request/response latency predictable; a
        // block-buffered worker would look like a hang to the host.
        command.env("PYTHONUNBUFFERED", "1");
        command.env("CRAWL_API_VERSION", crawl_protocol::PROTOCOL_VERSION);

        let mut child = command.spawn().map_err(|error| WorkerStartupError::Spawn {
            command: spec.command.join(" "),
            reason: error.to_string(),
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| WorkerStartupError::Spawn {
                command: spec.command.join(" "),
                reason: "worker stdin was not piped".to_owned(),
            })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| WorkerStartupError::Spawn {
                command: spec.command.join(" "),
                reason: "worker stdout was not piped".to_owned(),
            })?;
        let diagnostics = DiagnosticBuffer::new();
        if let Some(stderr) = child.stderr.take() {
            spawn_reader(stderr, diagnostics.clone());
        }

        let mut worker = ProcessWorker {
            id,
            child,
            stdin,
            stdout: BufReader::new(stdout),
            diagnostics,
        };

        worker.handshake().await?;
        Ok(Box::new(worker))
    }
}

/// One live worker process.
struct ProcessWorker {
    id: WorkerId,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    diagnostics: DiagnosticBuffer,
}

impl ProcessWorker {
    /// Verifies the worker speaks this protocol version before any file is sent.
    async fn handshake(&mut self) -> Result<(), WorkerStartupError> {
        let request = Request::control(RequestId::new(0), RequestOp::Handshake);
        let response = self
            .exchange(request, Some(HANDSHAKE_TIMEOUT))
            .await
            .map_err(|failure| {
                WorkerStartupError::Handshake(self.describe(&failure.to_string()))
            })?;

        match response {
            Response::Ready { api_version, .. } => {
                if api_version == crawl_protocol::PROTOCOL_VERSION {
                    Ok(())
                } else {
                    Err(WorkerStartupError::Protocol(
                        ProtocolError::UnsupportedVersion {
                            declared: api_version,
                            supported: crawl_protocol::PROTOCOL_VERSION,
                        },
                    ))
                }
            }
            other => Err(WorkerStartupError::Protocol(
                ProtocolError::UnexpectedMessage {
                    expected: "ready",
                    actual: match other {
                        Response::Ok { .. } => "ok",
                        Response::Error { .. } => "error",
                        Response::Ready { .. } => "ready",
                    },
                },
            )),
        }
    }

    /// Appends captured diagnostics to a failure message so a crash report
    /// carries the plugin's own traceback.
    fn describe(&self, reason: &str) -> String {
        let tail = self.diagnostics.peek();
        if tail.is_empty() {
            reason.to_owned()
        } else {
            let recent: Vec<&str> = tail
                .iter()
                .rev()
                .take(5)
                .rev()
                .map(String::as_str)
                .collect();
            format!("{reason}; worker stderr: {}", recent.join(" | "))
        }
    }

    /// Sends one request and reads its correlated response.
    async fn exchange(
        &mut self,
        request: Request,
        timeout: Option<Duration>,
    ) -> Result<Response, WorkerFailure> {
        let expected = request.id;
        let line = encode_request(&request).map_err(|error| {
            WorkerFailure::Protocol(ProtocolError::MalformedJson {
                reason: error.to_string(),
                excerpt: String::new(),
            })
        })?;

        let write = async {
            self.stdin.write_all(line.as_bytes()).await?;
            self.stdin.flush().await
        };
        let read = async {
            let mut buffer = String::new();
            let read_bytes = self.stdout.read_line(&mut buffer).await?;
            Ok::<_, std::io::Error>((read_bytes, buffer))
        };

        let exchange = async {
            write.await.map_err(|error| {
                // A broken pipe on write means the worker is already gone.
                WorkerFailure::Crash(format!("cannot send request: {error}"))
            })?;
            let (read_bytes, buffer) = read
                .await
                .map_err(|error| WorkerFailure::Crash(format!("cannot read response: {error}")))?;
            if read_bytes == 0 {
                return Err(WorkerFailure::Crash(
                    "worker closed its protocol stdout".to_owned(),
                ));
            }
            decode_response(buffer.trim_end_matches(['\n', '\r'])).map_err(WorkerFailure::Protocol)
        };

        let response = match timeout {
            Some(limit) => match tokio::time::timeout(limit, exchange).await {
                Ok(result) => result?,
                Err(_) => {
                    return Err(WorkerFailure::Timeout {
                        seconds: limit.as_secs_f64(),
                    })
                }
            },
            None => exchange.await?,
        };

        if response.id() != expected {
            return Err(WorkerFailure::Protocol(
                ProtocolError::CorrelationMismatch {
                    expected,
                    actual: response.id(),
                },
            ));
        }
        Ok(response)
    }
}

#[async_trait]
impl WorkerHandle for ProcessWorker {
    fn id(&self) -> WorkerId {
        self.id
    }

    fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    async fn dispatch(
        &mut self,
        request: Request,
        timeout: Option<Duration>,
    ) -> Result<Response, WorkerFailure> {
        match self.exchange(request, timeout).await {
            Ok(response) => Ok(response),
            Err(WorkerFailure::Crash(reason)) => Err(WorkerFailure::Crash(self.describe(&reason))),
            Err(other) => Err(other),
        }
    }

    async fn shutdown(&mut self) {
        let request = Request::control(RequestId::new(u64::MAX), RequestOp::Shutdown);
        if let Ok(line) = encode_request(&request) {
            let _ = self.stdin.write_all(line.as_bytes()).await;
            let _ = self.stdin.flush().await;
        }
        // Closing stdin gives a worker that ignores `shutdown` an EOF to exit on.
        let _ = self.stdin.shutdown().await;
        if tokio::time::timeout(SHUTDOWN_GRACE, self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.kill().await;
        }
    }

    async fn kill(&mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }

    fn take_diagnostics(&mut self) -> Vec<String> {
        self.diagnostics.take()
    }
}
