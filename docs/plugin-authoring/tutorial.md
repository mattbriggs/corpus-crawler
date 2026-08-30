# Build your first plugin

This walkthrough takes you from an empty directory to a finished CSV report. The
plugin you'll build counts the words in every text file it's given, which is
simple enough to stay out of the way while you learn the moving parts. Along the
way you'll meet the two things that trip up most people the first time: choosing
the right Python interpreter, and getting the output schema to match what your
code actually returns.

You'll need a built `crawl` binary and a Python virtual environment with the
plugin SDK installed. If you haven't done that yet, the
[getting started](../getting-started.md) page covers it in a few commands. Set
aside about fifteen minutes.

## Step 1: Create the plugin directory

A plugin is a directory containing a manifest and whatever code that manifest
points to. Nothing about the location is special, so put it wherever you keep
projects.

```bash
mkdir word-count && cd word-count
```

The host will treat this directory as your plugin's root, which matters in a
practical way: it becomes the working directory of your worker process. That's
what lets the manifest refer to `worker.py` rather than an absolute path, and
it's why relative paths inside your plugin behave the way you'd expect.

## Step 2: Write the processor

Start with the code, because the manifest describes it and is easier to write
second. Create `worker.py`:

```python
"""Count the words in a text file."""

from crawl_plugin_sdk import PluginProcessingError, run


def process(path: str) -> list[dict[str, object]]:
    """Return one record giving the word count for this file."""
    try:
        with open(path, encoding="utf-8") as handle:
            text = handle.read()
    except UnicodeDecodeError as error:
        raise PluginProcessingError(
            "not_utf8_text", f"{path} is not UTF-8 text"
        ) from error

    return [{"filename": path, "words": len(text.split())}]


if __name__ == "__main__":
    raise SystemExit(run(process, plugin="word-count"))
```

Three things in that file are worth pausing on, because they're the pattern
you'll follow in every plugin you write. The `process` function receives one
absolute path and returns a list of dictionaries, one per output row; returning
an empty list is perfectly valid and simply means the file produced nothing.
The `PluginProcessingError` marks a file your plugin can't handle, which fails
that file and lets the crawl continue rather than taking everything down with
it. The call to `run` at the bottom is the entire worker loop—it reads
requests, calls your function, encodes responses, and translates unexpected
exceptions into failure reports.

Notice what isn't there. There's no argument parsing, no loop, no JSON, and no
error handling for anything other than the one failure mode you care about. If
your code raises an exception you didn't anticipate, the SDK catches it,
reports that one file as failed, and includes the traceback in the log.

## Step 3: Declare the manifest

Now describe the plugin to the host. Create `plugin.yaml`:

```yaml
api_version: "1"

plugin:
  name: word-count
  version: "1.0.0"
  description: Count the words in each text file

runtime:
  type: python
  command: ["python3", "worker.py"]

input:
  extensions: [".txt", ".md"]

output:
  format: records
  schema:
    filename:
      type: string
      required: true
    words:
      type: integer
      required: true
```

The `output.schema` block is the part to get right, because it becomes the CSV
columns in exactly the order you declare them. Each key must match a key your
`process` function returns, and each declared type must match the value you
actually produce. Returning a key you didn't declare is an error by default,
which is stricter than most people expect and is the subject of Step 6.

The `runtime.command` line deserves equal attention for a different reason: it
names the program the host will execute, and `python3` here means whichever
`python3` is first on the path. That's almost certainly not the virtual
environment where you installed the SDK, which brings us to the step where
things usually go wrong.

## Step 4: Point the manifest at the right interpreter

Your worker imports `crawl_plugin_sdk`, so it has to run under a Python that
can find it. The host doesn't create or manage environments for you—it runs the
command you declare—so the manifest has to name an interpreter that has your
dependencies installed.

Change the command to the absolute path of your virtual environment's
interpreter:

```yaml
runtime:
  type: python
  command: ["/absolute/path/to/.venv/bin/python", "worker.py"]
```

If you skip this step, installation fails with a message that tells you exactly
what happened, which is worth seeing once so you recognize it later:

```text
error: plugin worker failed its handshake: plugin worker exited unexpectedly:
worker closed its protocol stdout; worker stderr: Traceback (most recent call
last): | File "worker.py", line 3, in <module> | from crawl_plugin_sdk import
run | ModuleNotFoundError: No module named 'crawl_plugin_sdk'
```

The host surfaces your worker's own traceback rather than hiding it behind a
generic failure, so the fix is usually visible in the error itself. Once the
interpreter is right, you're ready to register the plugin.

## Step 5: Install and validate

Installing a plugin does more than record its name. The host parses and
validates the manifest, starts your worker, and completes a handshake with it,
so a plugin that can't run is rejected now rather than in the middle of a
million-file crawl.

```bash
crawl plugin install ./word-count
```

A successful install prints a one-line summary:

```text
installed word-count@1.0.0 (2 extensions, 2 schema fields)
```

You can re-run the same checks at any time with `crawl plugin validate
word-count`, which is the fastest way to confirm the plugin still starts after
you've changed its code or moved its environment. Now that the host knows about
your plugin, you can run it against real files.

## Step 6: Run a crawl

Create a few files to process, then run the crawl:

```bash
mkdir -p corpus
printf 'one two three\n' > corpus/a.txt
printf 'four five\n' > corpus/b.md
printf 'ignored\n' > corpus/c.rs

crawl run word-count --input ./corpus --output ./report.csv
```

The summary tells you what happened, and the report holds the rows:

```text
crawl 8f3c… finished: success
  files discovered : 3
  files matched    : 2
  files completed  : 2
  records emitted  : 2
```

```csv
filename,words
/absolute/path/corpus/a.txt,3
/absolute/path/corpus/b.md,2
```

Two details in that output are worth connecting back to the manifest. Three
files were discovered but only two matched, because `c.rs` isn't one of the
extensions you declared. And the columns are `filename,words` in that order
because that's the order you wrote them in `output.schema`, not the order your
dictionary happened to use.

## Step 7: See what a schema mismatch looks like

Since the schema is the one contract the host enforces, it's worth breaking it
deliberately once so the failure is familiar. Change `worker.py` to return
`wordcount` instead of `words`, reinstall with `crawl plugin install
./word-count --force`, and run the crawl again.

The crawl completes, but the summary reports rejected records and the log
explains why:

```text
WARN record rejected event="record_rejected" category="schema"
  file="/absolute/path/corpus/a.txt" row=0 field="wordcount"
  record contains undeclared field "wordcount"
```

This is the behavior to internalize: a schema mismatch doesn't crash anything.
The crawl finishes, the offending rows are dropped, the summary counts them,
and the process exits with status `1` for partial success rather than `0`. That
design keeps one bad plugin from destroying a long run, but it also means a
silent typo costs you rows, so it pays to check the summary rather than only
the exit code.

Change the key back to `words`, reinstall, and confirm you're back to a clean
run.

## Where to go next

You now have a working plugin and have seen its most common failure. From here,
[Developing a plugin](developing.md) covers the iteration loop—testing your
processor without running a crawl, debugging a worker that won't start, and
reading the structured log. When you need the precise meaning of a manifest
field or a protocol message, the [manifest](manifest.md) and
[protocol](protocol.md) references have the details.
