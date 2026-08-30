//! Worker process ports (REQ-060 - REQ-071).

use std::time::Duration;

use async_trait::async_trait;
use crawl_domain::ids::WorkerId;
use crawl_domain::plugin::Plugin;
use crawl_protocol::{ProtocolError, Request, Response};

/// A worker could not be started or failed its handshake.
#[derive(Debug, thiserror::Error)]
pub enum WorkerStartupError {
    /// The runtime command could not be executed (REQ-014).
    #[error("cannot start plugin runtime {command:?}: {reason}")]
    Spawn {
        /// The command that could not be started.
        command: String,
        /// The operating-system reason.
        reason: String,
    },
    /// The worker started but did not complete the handshake (REQ-015).
    #[error("plugin worker failed its handshake: {0}")]
    Handshake(String),
    /// The worker declared an incompatible protocol version.
    #[error("{0}")]
    Protocol(#[from] ProtocolError),
}

/// A failure of one dispatch attempt.
#[derive(Debug, thiserror::Error)]
pub enum WorkerFailure {
    /// The per-file deadline expired (REQ-070).
    #[error("plugin exceeded the {seconds}s per-file timeout")]
    Timeout {
        /// The configured deadline.
        seconds: f64,
    },
    /// The worker process exited unexpectedly (REQ-067).
    #[error("plugin worker exited unexpectedly: {0}")]
    Crash(String),
    /// The worker violated the protocol contract (REQ-086, REQ-087).
    #[error("{0}")]
    Protocol(#[from] ProtocolError),
}

impl WorkerFailure {
    /// Returns the error category this failure belongs to.
    #[must_use]
    pub const fn category(&self) -> crawl_domain::errors::ErrorCategory {
        use crawl_domain::errors::ErrorCategory;
        match self {
            Self::Timeout { .. } => ErrorCategory::Timeout,
            Self::Crash(_) => ErrorCategory::WorkerCrash,
            Self::Protocol(_) => ErrorCategory::Protocol,
        }
    }

    /// Returns `true` when the worker cannot be reused after this failure.
    ///
    /// Every current failure class invalidates the worker: a timeout leaves a
    /// hung interpreter (REQ-071), a crash has no process left, and a protocol
    /// violation means the stream is desynchronised.
    #[must_use]
    pub const fn invalidates_worker(&self) -> bool {
        true
    }
}

/// One live worker process.
#[async_trait]
pub trait WorkerHandle: Send {
    /// Returns the worker's slot and generation identity.
    fn id(&self) -> WorkerId;

    /// Returns the operating-system process identifier, when known.
    fn pid(&self) -> Option<u32>;

    /// Sends a request and awaits its correlated response.
    ///
    /// # Caller contract
    /// At most one dispatch may be in flight per worker (REQ-062). The adapter
    /// verifies that the response identifier matches the request (REQ-163).
    async fn dispatch(
        &mut self,
        request: Request,
        timeout: Option<Duration>,
    ) -> Result<Response, WorkerFailure>;

    /// Asks the worker to exit cleanly, then reaps it.
    async fn shutdown(&mut self);

    /// Terminates the worker immediately, used after a timeout.
    async fn kill(&mut self);

    /// Returns and clears captured `stderr` lines (REQ-066, REQ-115).
    fn take_diagnostics(&mut self) -> Vec<String>;
}

/// Creates worker processes for a plugin.
#[async_trait]
pub trait WorkerFactory: Send + Sync {
    /// Starts a worker and completes its handshake.
    ///
    /// # Errors
    /// Returns [`WorkerStartupError`] when the process cannot be started or
    /// does not complete the handshake.
    async fn spawn(
        &self,
        plugin: &Plugin,
        id: WorkerId,
    ) -> Result<Box<dyn WorkerHandle>, WorkerStartupError>;
}
