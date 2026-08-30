# ADR 0004: Resolving the contract-shaping open questions

**Status:** Accepted · **Covers:** OQ-005, OQ-011, OQ-012, OQ-014, OQ-015, OQ-016, OQ-018, OQ-020, OQ-023, OQ-024, OQ-025

## Context

The SRS leaves 25 questions open. Eight of them shape externally observable
contracts and had to be settled before their phase could be frozen. Each is
isolated behind a policy or contract boundary so it can be revised without
restructuring the crawl engine.

## Decisions

| OQ | Decision | Where it lives |
|---|---|---|
| 011 Error response schema | `{code, message, detail?}`; correlation on the enclosing response; error data never reaches the CSV | `crawl-protocol::response::PluginError` |
| 012 Extra record fields | **Reject** by default; a manifest may opt into `extra_fields: ignore` | `domain::schema::ExtraFieldPolicy` |
| 014 Semantic versioning | Not required; an opaque non-empty version token | `domain::ids::PluginVersion` |
| 015 Configuration precedence | `CLI > manifest > host default` | `application::services::configuration` |
| 016 Retry eligibility | Retry worker crashes, timeouts, and protocol violations; never plugin errors, file-access errors, or schema errors | `domain::policy::RetryPolicy` |
| 018 Automatic worker count | Available parallelism, clamped to 1..=32 | `domain::policy::WorkerCount` |
| 020 Error-threshold semantics | Every error a *running* crawl can record counts: traversal, file access, runtime startup, plugin processing, timeout, worker crash, protocol, schema, and report. Preflight categories, cancellation, and host-fatal do not | `domain::policy::FailurePolicy` |
| 023 Plugin self-test | An optional `selftest` control request, declared by `runtime.selftest` | `crawl-protocol::RequestOp` |
| 024 Removal ownership | `remove` unregisters only; installation records a path and never copies files | `infrastructure::registry::FileRegistry` |
| 025 Exit codes | `0` success, `1` partial, `2` configuration, `3` cancelled, `4` plugin, `5` host fatal | `crawl-cli::exit_codes` |

## Rationale for the two contentious ones

**Retry eligibility (OQ-016).** The dividing line is whether the failure is
attributable to *worker state* or to *the file*. A crash or timeout says
nothing about the file and a fresh worker may succeed. A plugin-declared error
or a schema violation is a deterministic answer about that file; retrying only
multiplies the same failure and the same log noise.

**Error-threshold semantics (OQ-020).** Rejected rows count. A plugin emitting
thousands of schema-invalid rows is a contract failure the operator should hear
about early, and excluding them would let the loudest possible failure mode
slip past an error budget entirely.

Report-write errors and mid-crawl worker-startup failures count for a blunter
reason: a crawl that lost rows, or that could not replace a dead worker, must
not be able to exit `0`. An early draft excluded both, and the coordinator
tests caught it reporting success for a crawl that had written nothing.

## Still open

OQ-001 (multi-plugin crawls), OQ-002 (host metadata columns), OQ-003
(deduplication), OQ-004 (deterministic ordering), OQ-006 (sandboxing), OQ-007
(extension override), OQ-008 (further output formats), OQ-009 (batching),
OQ-010 (resumable crawls), OQ-013 (independent schema version), OQ-019 (path
metadata), and OQ-022 (`datetime`) remain unresolved. None is required for v1,
and each stays isolated behind an existing boundary. In particular OQ-005
(per-plugin virtual environments) is deliberately left to the packaging model:
the manifest names an interpreter command, so a plugin *may* point at its own
venv today without the host taking ownership of one.
