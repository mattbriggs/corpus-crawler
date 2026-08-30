# ADR 0001: A process boundary between host and plugins

**Status:** Accepted · **Covers:** REQ-004, REQ-005, ARC-003, NFR-010

## Context

Plugins are Python; the host is Rust. Python's GIL makes in-process thread
parallelism a poor fit for CPU-bound extraction, and an embedded interpreter
would put plugin crashes and dependency conflicts inside the host's address
space.

## Decision

Plugins run in separate OS processes speaking a line protocol over stdin and
stdout. Workers are **persistent**: one interpreter serves many files.

## Consequences

- A plugin segfault, hang, or dependency failure is contained; the host detects
  it, replaces the worker, and continues (REQ-121).
- Separate interpreters mean separate GILs, so worker count is the real
  concurrency control.
- Interpreter startup is amortized across a crawl instead of paid per file,
  which matters most for large corpora of small files (NFR-004).
- The cost is serialization at the boundary and the inability to pass rich
  objects, which the flat-record contract already rules out.
- This is explicitly **not** a security boundary (REQ-007, NFR-041).
