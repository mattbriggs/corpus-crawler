//! Worker `stderr` capture (REQ-066, REQ-115, REQ-191).
//!
//! Diagnostics are drained on a dedicated task into a bounded buffer. They are
//! never parsed as protocol, and a chatty plugin cannot exhaust host memory.

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::ChildStderr;

/// Maximum diagnostic lines retained between drains.
const BUFFER_LIMIT: usize = 256;

/// Shared, bounded buffer of captured `stderr` lines.
#[derive(Debug, Default, Clone)]
pub struct DiagnosticBuffer {
    lines: Arc<Mutex<Vec<String>>>,
}

impl DiagnosticBuffer {
    /// Creates an empty buffer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a line, dropping the oldest when the buffer is full.
    pub fn push(&self, line: String) {
        let mut lines = self.lines.lock().expect("diagnostic buffer poisoned");
        if lines.len() >= BUFFER_LIMIT {
            lines.remove(0);
        }
        lines.push(line);
    }

    /// Returns and clears the buffered lines.
    #[must_use]
    pub fn take(&self) -> Vec<String> {
        let mut lines = self.lines.lock().expect("diagnostic buffer poisoned");
        std::mem::take(&mut *lines)
    }

    /// Returns the buffered lines without clearing them.
    #[must_use]
    pub fn peek(&self) -> Vec<String> {
        self.lines
            .lock()
            .expect("diagnostic buffer poisoned")
            .clone()
    }
}

/// Spawns the task that drains a worker's `stderr` for the life of the process.
pub fn spawn_reader(stderr: ChildStderr, buffer: DiagnosticBuffer) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            buffer.push(line);
        }
    })
}
