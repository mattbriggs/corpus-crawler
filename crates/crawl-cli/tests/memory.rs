//! Bounded-memory verification (NFR-001, NFR-021, AC-005, AC-021).
//!
//! The central scalability claim is that scheduler memory depends on queue
//! capacity and worker count, **not** on how many files the crawl encounters.
//! This test falsifies that claim if it is false: it runs the same crawl over
//! corpora that differ by an order of magnitude and compares peak RSS.
//!
//! It is `#[ignore]`d because it writes tens of thousands of files. Run it
//! deliberately:
//!
//! ```text
//! cargo test -p crawl-cli --test memory -- --ignored --nocapture
//! ```

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

/// Runs one crawl under `/usr/bin/time` and returns its peak RSS in bytes.
fn peak_rss_for_crawl(home: &Path, corpus: &Path, files: usize) -> u64 {
    for index in 0..files {
        // Spread files over subdirectories so traversal is genuinely recursive.
        let directory = corpus.join(format!("d{:03}", index % 100));
        fs::create_dir_all(&directory).expect("corpus directory");
        fs::write(directory.join(format!("f{index:06}.txt")), "one line\n").expect("corpus file");
    }

    let mut command = Command::new("/usr/bin/time");
    command
        .arg(if cfg!(target_os = "macos") {
            "-l"
        } else {
            "-v"
        })
        .arg(env!("CARGO_BIN_EXE_crawl"))
        .args(["run", "fixture-echo", "--input"])
        .arg(corpus)
        .arg("--output")
        .arg(home.join(format!("report-{files}.csv")))
        // Fixed resources: only the corpus size varies between runs.
        .args([
            "--workers",
            "2",
            "--queue-capacity",
            "64",
            "--quiet",
            "--overwrite",
        ])
        .env("CRAWL_HOME", home)
        .current_dir(repo_root());

    let output = command.output().expect("run under /usr/bin/time");
    assert!(
        output.status.success(),
        "crawl failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let report = String::from_utf8_lossy(&output.stderr);
    let line = report
        .lines()
        .find(|line| line.to_lowercase().contains("maximum resident set size"))
        .unwrap_or_else(|| panic!("no RSS line in:\n{report}"));
    let value: u64 = line
        .split_whitespace()
        .find_map(|token| token.parse().ok())
        .expect("an RSS number");

    // macOS reports bytes; GNU time reports kilobytes.
    if cfg!(target_os = "macos") {
        value
    } else {
        value * 1024
    }
}

#[test]
#[ignore = "writes tens of thousands of files; run deliberately"]
fn scheduler_memory_does_not_grow_with_the_file_count() {
    let home = tempfile::tempdir().expect("tempdir");

    let install = Command::new(env!("CARGO_BIN_EXE_crawl"))
        .args(["plugin", "install"])
        .arg(repo_root().join("plugins/fixtures/echo"))
        .arg("--force")
        .env("CRAWL_HOME", home.path())
        .current_dir(repo_root())
        .output()
        .expect("install");
    assert!(install.status.success());

    let small_corpus = home.path().join("small");
    let large_corpus = home.path().join("large");
    fs::create_dir_all(&small_corpus).expect("dir");
    fs::create_dir_all(&large_corpus).expect("dir");

    let small = peak_rss_for_crawl(home.path(), &small_corpus, 2_000);
    let large = peak_rss_for_crawl(home.path(), &large_corpus, 20_000);

    let ratio = large as f64 / small as f64;
    println!("peak RSS: 2,000 files = {small} B, 20,000 files = {large} B (ratio {ratio:.2}x)");

    // A ten-fold increase in files must not produce anything close to a
    // ten-fold increase in memory. Anything under 2x is comfortably
    // sub-linear; proportional growth would land near 10x.
    assert!(
        ratio < 2.0,
        "memory grew {ratio:.2}x for a 10x larger corpus: discovery or report \
         aggregation is not streaming"
    );
}
