# ADR 0002: JSON Lines as the host-worker protocol

**Status:** Accepted · **Covers:** REQ-080, REQ-088, ARC-004, NFR-033

## Context

The host and plugins are developed independently and must stay compatible
across versions. The transport had to be debuggable by hand and implementable
by a future non-Python runtime.

## Decision

One UTF-8 JSON object per newline-terminated line. Compatibility is governed by
a single `api_version` token, shared by the manifest and the protocol.

Operations are `handshake`, `process`, `selftest`, and `shutdown`. `op` is
omitted for `process`, which keeps the minimal documented request
(`{"id": 42, "filepath": "/data/a.txt"}`) valid on the wire.

## Consequences

- Decoding is **strict**: unknown statuses, undefined top-level fields, and
  non-object rows are protocol failures, not best-effort interpretations
  (REQ-087). Strictness makes a version mismatch visible immediately.
- One version token, not two. OQ-013 (an independent schema-language version)
  stays unresolved rather than inventing an unrequired contract.
- A `ready` handshake response proves the runtime started *and* the plugin
  loaded before any file is dispatched, which is what makes REQ-014 and
  REQ-015 checkable at install time.
- Shared fixtures in `tests/fixtures/protocol/cases.json` are consumed by both
  languages, so a strictness divergence fails a test rather than a crawl.
