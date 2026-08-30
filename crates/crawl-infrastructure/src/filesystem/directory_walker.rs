//! Streaming recursive traversal built on the `ignore` crate's walker.
//!
//! # Why `ignore`
//!
//! It provides symlink control, per-entry error reporting, and a genuinely
//! streaming iterator. Its VCS-aware filters are switched **off**: a file
//! processing runtime must see every file in the tree, not the subset git
//! would track.

use std::path::PathBuf;

use async_trait::async_trait;
use crawl_application::ports::walker::{DirectoryWalker, Discovery};
use crawl_domain::errors::{ErrorCategory, ErrorEvent};
use crawl_domain::plugin::InputSpec;
use ignore::WalkBuilder;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Recursive walker over a local filesystem.
#[derive(Debug, Default, Clone, Copy)]
pub struct FilesystemWalker;

#[async_trait]
impl DirectoryWalker for FilesystemWalker {
    /// Walks the tree on a blocking thread and streams discoveries.
    ///
    /// Traversal is synchronous by nature, so it runs on the blocking pool and
    /// pushes into the async channel. `blocking_send` is what converts a full
    /// channel into backpressure on the directory scan itself (REQ-051).
    async fn walk(
        &self,
        root: PathBuf,
        input: InputSpec,
        sink: mpsc::Sender<Discovery>,
        cancel: CancellationToken,
    ) {
        let _ = tokio::task::spawn_blocking(move || {
            let walker = WalkBuilder::new(&root)
                .standard_filters(false)
                .hidden(false)
                .follow_links(input.follow_symlinks)
                .build();

            for entry in walker {
                if cancel.is_cancelled() || sink.is_closed() {
                    break;
                }
                let discovery = match entry {
                    Ok(entry) => {
                        // Only regular files are dispatched. Directories are
                        // traversed; sockets, FIFOs, and devices are not files
                        // a plugin could meaningfully open. Symlinks report as
                        // regular files only when `follow_symlinks` is set, so
                        // the traversal policy governs them too (REQ-044).
                        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                            continue;
                        }
                        Discovery::File(entry.into_path())
                    }
                    Err(error) => Discovery::Error(Box::new(ErrorEvent::new(
                        ErrorCategory::Traversal,
                        error.to_string(),
                    ))),
                };
                if sink.blocking_send(discovery).is_err() {
                    break;
                }
            }
        })
        .await;
    }
}
