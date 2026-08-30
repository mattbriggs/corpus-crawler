# Operations

## Exit codes

| Code | Meaning |
|-----:|---|
| 0 | Success: no qualifying errors |
| 1 | Partial success: completed with file- or plugin-level errors |
| 2 | Configuration, usage, or preflight failure |
| 3 | Cancelled by the operator |
| 4 | Plugin or runtime failure |
| 5 | Fatal host failure |

`2` matches `clap`'s usage-error code, so argument errors and preflight errors
agree without special-casing.

## Structured log fields

`--log-format json` and `--log FILE` emit one JSON object per event. Every
event carries `event`; failures additionally carry `category`, and where known
`file`, `worker`, `request`, and `code`.

Per-file events (`file_discovered`, `file_matched`, `file_processing_started`,
`record_accepted`) are `TRACE`. Failures are `WARN`. Crawl lifecycle is `INFO`.
This keeps a million-file crawl's log readable by default; use `-vv` when you
need per-file detail.

## Troubleshooting

The failures below are the ones operators hit most often. Each is diagnosable
from the summary and the log alone, without reading the plugin's source. If
you're the plugin's author rather than its operator,
[Developing a plugin](plugin-authoring/developing.md#common-failures-and-what-they-mean)
covers the same ground from the other side.

**A worker fails to start.** Run `crawl plugin validate <name>`, which starts
the plugin in isolation and reports where it fails. By far the most common
cause is a `runtime.command` that names an interpreter unable to import the
plugin's dependencies, and the host surfaces the worker's own traceback in the
error, so the missing module is usually named for you. Pointing the manifest at
the correct virtual environment fixes it, after which the plugin needs
reinstalling with `--force`.

**Every row is rejected.** This means the plugin runs correctly but its output
disagrees with its declared schema, so compare the keys it returns against the
`output.schema` block. The log names the offending field on every rejection,
which usually identifies the mismatch immediately. If the plugin legitimately
returns more than it reports, setting `extra_fields: ignore` in its manifest
keeps the extra keys from rejecting the whole record.

**No files are matched.** Discovery found files but none matched the plugin's
extensions. Matching is case-insensitive and considers only the final
extension, so `.tar.gz` never matches and files without an extension never
match at all. Check the plugin's declared extensions with `crawl plugin inspect
<name>`.

**A crawl hangs.** A plugin that never responds is only bounded if a timeout is
configured, and no timeout is set unless someone asks for one. Pass `--timeout
SECONDS`, or declare `execution.timeout_seconds` in the manifest so the plugin
carries a sensible default of its own. Once a timeout is in force, a hung
worker is killed and replaced rather than stalling the run.

**The report is missing.** After a fatal host failure the partial report is
left at `<output>.partial` instead of being promoted to the requested path,
which is deliberate: a report nothing has vouched for should not appear where
automation expects a finished one. The error message names the file, and its
contents are valid CSV up to the point of failure.

## Performance tuning

Start with `--workers auto`. More workers is not always faster: the bottleneck
may be disk I/O, network storage, or memory rather than CPU. For network-mounted
input, worker counts above the storage's effective parallelism reduce
throughput.

Queue capacity trades memory for smoothness. The default of 1024 keeps
discovery slightly ahead of the pool. Lower it if paths are very long and the
corpus is enormous; raising it rarely helps.
