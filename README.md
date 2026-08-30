# crawl

A modular, high-throughput file-processing runtime: a Rust host that owns
traversal, concurrency, supervision, validation, and reporting, plus Python
plugins that own the file-specific extraction logic.

```text
File -> Plugin -> Records -> Validated rows -> CSV
```

The host decides *which* files to process, *when*, and *how safely*. A plugin
decides *what to extract from one file*. That boundary is the whole point: you
write the interesting part, and stop rewriting directory walkers, worker pools,
timeout handling, and CSV writers.

> **Installed plugins run as trusted local code.** The subprocess boundary
> provides fault isolation—a crashing or hanging plugin cannot take down the
> host—but it is **not** a security sandbox. Only install plugins you trust.

## Install

```bash
cargo build --release          # builds target/release/crawl
python3 -m venv .venv          # only needed to author plugins with the SDK
.venv/bin/python -m pip install -e "python[dev]"
```

## Plugin lifecycle

```bash
crawl plugin install ./plugins/reference/entity-lines   # validate + register
crawl plugin list                                       # name, version, extensions
crawl plugin inspect entity-lines                       # effective metadata
crawl plugin validate entity-lines                      # start runtime, run self-test
crawl plugin remove entity-lines                        # unregister only
```

Installation starts the plugin's runtime and completes a protocol handshake, so
a plugin that cannot start is rejected before it can be selected for a crawl.
`remove` unregisters and never deletes your code or Python environment.

## Your first crawl

```bash
crawl run entity-lines \
  --input ./corpus \
  --output ./report.csv \
  --log ./crawl.log \
  --workers 4 \
  --timeout 30
```

Useful options: `--workers N|auto`, `--timeout SECONDS`, `--no-timeout`,
`--max-retries N`, `--queue-capacity N`, `--fail-fast`, `--max-errors N`,
`--overwrite`, `--log-format json|text`, `-v`/`-q`.

## Outputs

| Output | Content |
|---|---|
| CSV report | One row per schema-valid record, columns in declared schema order. A crawl that emits no records still produces a header-only file. |
| Log | Structured events on stderr, and JSON to `--log` when given. |
| Summary | Files discovered/matched/completed/failed, records emitted/rejected, timeouts, worker restarts, retries, duration, and errors by category. |
| Exit code | `0` success, `1` partial success, `2` configuration/usage, `3` cancelled, `4` plugin failure, `5` fatal host failure. |

The report is streamed to `<output>.partial` and promoted on successful
finalization, so a crash never leaves a half-written file at the path your
automation reads.

## Writing a plugin

A plugin is a directory holding a `plugin.yaml` manifest and the code it points
to. The manifest tells the host what to call your plugin, how to start it, which
files to send it, and what columns it produces:

```yaml
api_version: "1"
plugin:
  name: entity-lines
  version: "1.0.0"
runtime:
  type: python
  command: ["python3", "worker.py"]
input:
  extensions: [".txt", ".md"]
output:
  format: records
  schema:
    filename: { type: string, required: true }
    line:     { type: integer, required: true }
    entity:   { type: string, required: true }
```

The code is a single function. It takes one file path and returns a list of
records matching the schema you declared:

```python
from crawl_plugin_sdk import PluginProcessingError, run

def process(path: str) -> list[dict[str, object]]:
    with open(path, encoding="utf-8") as handle:
        return [{"filename": path, "line": n, "entity": line.strip()}
                for n, line in enumerate(handle, start=1)]

if __name__ == "__main__":
    raise SystemExit(run(process, plugin="entity-lines"))
```

Return zero, one, or many records per file; returning none is a success. Raise
`PluginProcessingError` to fail one file without ending the crawl. You never
write to `stdout`—the SDK reserves it for the protocol and redirects stray
`print` output to `stderr` for you.

One thing catches almost everyone the first time: `runtime.command` names the
interpreter the host will actually execute, so it has to be one that can import
your dependencies. If your plugin uses the SDK from a virtual environment, name
that environment's interpreter explicitly.

[Build your first plugin](docs/plugin-authoring/tutorial.md) walks through the
whole process from an empty directory to a finished report.

## Development

```bash
cargo test                                   # 226 unit, contract, and acceptance tests
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
cargo llvm-cov --workspace --summary-only    # coverage, gated at 85% in CI
cargo bench -p crawl-domain --bench schema   # validation micro-benchmarks

.venv/bin/python -m pytest python/tests --cov=crawl_plugin_sdk
.venv/bin/ruff check python/ && .venv/bin/mypy python/src/crawl_plugin_sdk
mkdocs build --strict                        # documentation site
```

The bounded-memory verification is excluded from the default run because it
writes tens of thousands of files:

```bash
cargo test -p crawl-cli --test memory -- --ignored --nocapture
```

Acceptance tests are named after the SRS criteria they verify
(`ac_013_worker_crash_isolated_and_replaced`), so requirements stay executable
rather than detached from the implementation.

## Architecture

Five Rust crates in a strict dependency order, plus an independently packaged
Python SDK:

| Crate | Responsibility |
|---|---|
| `crawl-domain` | Value objects, manifest, schema, policies, state machines. Pure and synchronous. |
| `crawl-protocol` | The versioned JSONL contract with plugins. Isolated because it is the compatibility boundary. |
| `crawl-application` | Ports and use cases, including the crawl coordinator. Knows no adapter. |
| `crawl-infrastructure` | Filesystem, subprocess, registry, CSV, and logging adapters. |
| `crawl-cli` | The composition root and command surface. |

Full documentation lives in [`docs/`](docs/), including architecture,
the failure model, the protocol reference, and the ADRs recording every
resolved open question.

## Status

See [`docs/implementation-status.md`](docs/implementation-status.md) for what is
built, what is verified, and what remains.

## License

MIT
