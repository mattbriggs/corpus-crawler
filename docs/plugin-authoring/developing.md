# Developing a plugin

Once your plugin runs, the question becomes how to change it quickly and find
out what went wrong when it misbehaves. This page covers the working loop:
which edits need a reinstall and which don't, how to test your code without
running a crawl, how to read what the host tells you, and what the common
failures actually look like.

## Know which edits need a reinstall

The single most useful thing to know about the development loop is that the
registry stores your manifest, not your code. When you install a plugin, the
host validates the manifest and records it along with the plugin's directory;
when you run a crawl, it launches the command that manifest names. Your worker
file is read fresh by the interpreter every time.

That split has a practical consequence worth committing to memory:

| What you changed | What you need to do |
|---|---|
| `worker.py` or any Python your plugin imports | Nothing. The next crawl picks it up. |
| `plugin.yaml`—schema, extensions, command, execution defaults | `crawl plugin install ./your-plugin --force` |

Forgetting the second row is a genuinely confusing experience, because the
crawl runs cleanly and simply ignores the change you made. If you add an
extension and the file counts don't move, or you add a schema column and the
CSV header doesn't change, a stale registration is almost always the reason.

## Test the processor as an ordinary function

Because your processor is a plain function that takes a path and returns a
list, most of your testing needs nothing from `crawl` at all. Write tests the
way you'd write them for any other Python code, and keep the fast feedback loop
where the logic actually lives.

```python
from pathlib import Path

from worker import process


def test_counts_words(tmp_path: Path) -> None:
    sample = tmp_path / "a.txt"
    sample.write_text("one two three\n", encoding="utf-8")

    assert process(str(sample)) == [{"filename": str(sample), "words": 3}]


def test_an_empty_file_produces_a_zero_count(tmp_path: Path) -> None:
    sample = tmp_path / "empty.txt"
    sample.write_text("", encoding="utf-8")

    assert process(str(sample))[0]["words"] == 0
```

Tests like these catch the errors you'll make most often—wrong counts, wrong
keys, mishandled edge cases—long before a crawl is involved. What they can't
catch is a mismatch between what your function returns and what your manifest
declares, since the manifest isn't in play. For that, you need the worker or a
real crawl.

## Talk to the worker directly

When you want to see exactly what your plugin puts on the wire, you can drive
the worker by hand. It reads JSON Lines on `stdin` and writes them on `stdout`,
so a pipe is enough:

```bash
cd your-plugin
printf '{"id":0,"op":"handshake"}\n{"id":1,"filepath":"/absolute/path/a.txt"}\n' \
  | /path/to/.venv/bin/python worker.py
```

```json
{"id":0,"status":"ready","api_version":"1","plugin":"word-count","sdk":"crawl-plugin-sdk/0.1.0"}
{"id":1,"status":"ok","rows":[{"filename":"/absolute/path/a.txt","words":3}]}
```

This is the fastest way to answer "what is my plugin actually returning?"
because you see the literal keys and types, not a summary of them. Compare
those keys against your `output.schema` block and most schema rejections
explain themselves. Note the two conventions the probe makes visible: paths are
absolute, and the handshake comes first.

## Debug with print, and know where it goes

You can use `print` for debugging, but you should know what the SDK does with
it. Worker `stdout` is reserved for protocol messages, so the SDK takes
ownership of it at startup and redirects `sys.stdout` to `stderr`. Anything you
print therefore becomes a diagnostic rather than corrupting the stream—which is
why your output seems to vanish from `stdout` and appear somewhere else.

To see those diagnostics during a crawl, raise the verbosity:

```bash
crawl run word-count --input ./corpus --output ./report.csv -vv
```

Worker output arrives as `worker_diagnostic` events alongside the host's own
per-file tracing. If you'd rather have structured output to search or archive,
add `--log crawl.log`, which writes every event as JSON regardless of what the
terminal shows.

Two flags make debugging noticeably easier when something fails intermittently.
Running with `--workers 1` removes interleaving, so the log reads in order, and
`--fail-fast` stops at the first error instead of burying it under thousands of
later lines.

## Add a self-test

A self-test gives your plugin a way to check itself at registration time, which
is useful when the plugin depends on something outside its own code—a model
file, a compiled parser, a locale. Declare it in the manifest and pass it to
`run`:

```yaml
runtime:
  type: python
  command: ["/path/to/.venv/bin/python", "worker.py"]
  selftest: true
```

```python
def selftest() -> None:
    """Verify the extractor against a known input."""
    import tempfile

    with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as handle:
        handle.write("one two three\n")
    assert process(handle.name)[0]["words"] == 3


if __name__ == "__main__":
    raise SystemExit(run(process, plugin="word-count", selftest=selftest))
```

`crawl plugin validate word-count` now runs that function inside a real worker
process and reports the result. Because it runs in the same environment the
crawl will use, it catches environment problems that a unit test on your own
machine would miss.

## Common failures and what they mean

The host tries to tell you what happened rather than that something happened.
The table below maps the messages you're most likely to see to their usual
cause.

| What you see | What it usually means |
|---|---|
| `ModuleNotFoundError` inside a handshake failure | `runtime.command` names an interpreter that can't import your dependencies. Point it at the virtual environment where you installed them. |
| `record contains undeclared field "x"` | Your processor returns a key the schema doesn't declare. Add it to `output.schema`, remove it from the record, or set `extra_fields: ignore`. |
| `required field "x" is missing` | A declared required field is absent from a record. Return it, or mark it `required: false` if it's genuinely optional. |
| `field "x" expected integer but received string` | A type mismatch. The host never silently coerces, so `"3"` won't satisfy an `integer` column. |
| `contains a nested object; v1 records must be flat` | You returned a dict or list as a value. Serialize it to a string, and declare that column as `string`. |
| `plugin exceeded the Ns per-file timeout` | One file took too long. Raise `--timeout`, or find the pathological input—the log names the file. |
| Files matched is `0` | Your extensions don't match. Matching is case-insensitive and uses only the final extension, so `.tar.gz` never matches; use `.gz`. |
| Nothing changed after editing `plugin.yaml` | The registry still holds the old manifest. Reinstall with `--force`. |

## Fail one file without failing the crawl

When your plugin genuinely can't process a file, say so explicitly. Raising
`PluginProcessingError` gives the host a code and a message it can attribute to
that file, and the crawl continues with everything else.

```python
raise PluginProcessingError("not_utf8_text", f"{path} is not UTF-8 text")
```

You don't have to catch everything, though. Any exception you don't handle is
caught by the SDK, reported as `unhandled_exception` with a traceback, and
costs you the same one file. The difference is diagnostic quality rather than
crash safety: a named error tells you and your users something specific,
whereas a traceback tells them your plugin has a bug. Reserve explicit errors
for conditions you expect, and let genuine bugs surface as tracebacks.

## Write for reuse across many files

One last habit pays off at scale. Your worker process is reused for many
files—that's the reason a crawl doesn't pay Python's startup cost per file—so
work you do at import time happens once, while work inside `process` happens
per file.

```python
import re

# Compiled once, when the worker starts.
ENTITY = re.compile(r"\b[A-Z][a-zA-Z]+\b")


def process(path: str) -> list[dict[str, object]]:
    # Runs once per file.
    ...
```

Load models, compile patterns, and open shared resources at module level. Just
remember that anything you keep between calls persists for the life of the
worker, so avoid accumulating state that grows with every file processed—that
is exactly the unbounded memory growth the host's streaming design is built to
avoid.

## Where to go next

For the exact meaning of every manifest field, see the
[manifest reference](manifest.md). If you're implementing a worker in another
language, or want to understand a protocol error in detail, the
[protocol reference](protocol.md) documents the wire format the host and worker
share.
