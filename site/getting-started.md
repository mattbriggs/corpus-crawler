# Getting started

This page gets `crawl` built, installs a plugin, and runs your first crawl. It
assumes a working Rust toolchain and Python 3.10 or later; if you want to write
a plugin rather than just run one, the
[tutorial](plugin-authoring/tutorial.md) picks up where this leaves off.

## Build

The host is a Rust binary and the plugin SDK is a Python package, so there are
two things to set up. You only need the second if you plan to write plugins in
Python.

```bash
cargo build --release
python3 -m venv .venv
.venv/bin/python -m pip install -e "python[dev]"
```

The binary lands at `target/release/crawl`. Put it on your path, or use the full
path in the commands that follow.

## Install a plugin

Installing does more than record a name. The host parses and validates the
manifest, starts the plugin's worker process, and completes a handshake with
it, so a plugin that can't run is rejected now rather than partway through a
long crawl.

```bash
crawl plugin install ./plugins/reference/entity-lines
crawl plugin validate entity-lines
crawl plugin list
```

If that install fails with a `ModuleNotFoundError`, the manifest is naming an
interpreter that can't see the SDK, which is the subject of the next section.

## Point a plugin at the right interpreter

The `runtime.command` field in a plugin's manifest names the program the host
executes. The host doesn't create or manage Python environments—it runs exactly
what you tell it to—so a plugin with dependencies needs a manifest that names an
interpreter that has them installed.

```yaml
runtime:
  type: python
  command: ["/path/to/.venv/bin/python", "worker.py"]
```

Because the worker starts with the plugin's own directory as its working
directory, a relative entry point like `worker.py` resolves the way you'd
expect. Only the interpreter needs the absolute path.

## Run a crawl

With a plugin registered, point it at a directory and a destination:

```bash
crawl run entity-lines --input ./corpus --output ./report.csv
```

The crawl walks the tree, sends matching files to the plugin's workers,
validates every record against the declared schema, and streams the accepted
rows to the CSV as they arrive. Nothing is held in memory waiting for the end,
so the corpus can be far larger than RAM.

Useful options while you're finding your footing: `--workers N` sets the
concurrency, `--timeout SECONDS` bounds any single file, `--overwrite` replaces
an existing report, and `-v` or `-vv` raises the log detail.

## Read the results

Check the exit code first, because it distinguishes outcomes that look similar
in the summary:

```bash
echo $?     # 0 success, 1 partial success, 2 configuration, 3 cancelled ...
```

A `1` means the crawl finished but something failed along the way—rejected
rows, a timed-out file, a crashed worker. The printed summary breaks the errors
down by category, so a long run can't hide an important failure class behind a
single aggregate count. The [operations](operations.md) page lists every exit
code and log field.

## Where to go next

If you want to write a plugin, start with
[Build your first plugin](plugin-authoring/tutorial.md). If you want to
understand how the host works before trusting it with a large job, the
[architecture overview](architecture/components.md) and the
[failure model](architecture/failure-model.md) explain the pipeline and what
happens when parts of it break.
