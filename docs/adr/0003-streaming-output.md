# ADR 0003: Streaming discovery and temp-then-promote output

**Status:** Accepted · **Covers:** REQ-041, REQ-101, REQ-183, NFR-001, OQ-017, OQ-021

## Context

A crawl may encounter millions of files. Memory must not scale with the file
count or the row count. Separately, the SRS left the fate of a partially
written report undefined.

## Decision

Discovery streams into a bounded queue; validated rows stream into a single
CSV writer. Neither the path set nor the row set is ever materialized.

The writer creates `<output>.partial`, writes the header immediately, appends
rows as they are validated, and renames onto the requested path at
finalization.

## Consequences

- Steady-state memory is a function of queue capacity and worker count, not of
  corpus size (NFR-021).
- Writing the header at open time means a zero-record crawl still produces a
  valid header-only CSV, which resolves **OQ-021**, and it proves the
  destination is writable during preflight rather than after processing.
- **OQ-017** is resolved by outcome: orderly endings (success, partial success,
  cancellation, error-threshold shutdown) promote the partial report; a
  host-fatal failure leaves the temporary file unpromoted and names it, so no
  unvouched-for report ever appears at the path automation reads.
- Rows appear in completion order, not a deterministic order (REQ-106, OQ-004).
