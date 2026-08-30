//! Streaming filesystem discovery port (REQ-040, REQ-041).

use std::path::PathBuf;

use async_trait::async_trait;
use crawl_domain::errors::ErrorEvent;
use crawl_domain::plugin::InputSpec;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// One item produced by traversal.
#[derive(Debug, Clone)]
pub enum Discovery {
    /// A regular file was observed.
    File(PathBuf),
    /// A directory entry could not be traversed (REQ-045).
    Error(Box<ErrorEvent>),
}

/// Recursive, streaming directory traversal.
///
/// The walker sends discoveries into a bounded channel, so a slow consumer
/// applies backpressure to traversal instead of letting paths accumulate
/// (REQ-041, NFR-002).
#[async_trait]
pub trait DirectoryWalker: Send + Sync {
    /// Walks `root`, sending each discovery to `sink` until the tree is
    /// exhausted, the sink closes, or `cancel` fires.
    async fn walk(
        &self,
        root: PathBuf,
        input: InputSpec,
        sink: mpsc::Sender<Discovery>,
        cancel: CancellationToken,
    );
}
