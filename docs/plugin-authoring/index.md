# Plugin authoring

A plugin is the part of `crawl` that you write. The host already knows how to
walk a directory tree, run work in parallel, supervise crashes, validate
output, and stream a CSV report; what it can't know is what your files mean.
That last question—what to extract from one file—is the whole of a plugin's
job, and it's deliberately the only question you have to answer.

Because the boundary is that narrow, a plugin is smaller than you might
expect. In Python, the minimum is one function that takes a file path and
returns a list of dictionaries, plus a short YAML file that declares the
plugin's name, the file extensions it wants, and the columns it produces. You
don't write a worker loop, a retry policy, or a CSV writer, and you don't need
to know anything about the Rust host to write a conforming plugin.

The trade for that simplicity is a contract. You declare your output columns up
front, and the host holds you to them: a row that doesn't match the schema is
rejected and logged rather than written. This feels strict the first time it
happens to you, and it's worth knowing why the strictness exists. A crawl may
process millions of files across many worker processes, and the only way to
guarantee a coherent CSV at the end is to agree on the columns before the first
file is opened.

## Where to start

The three pages below are meant to be read in order, though you can skip
ahead if you already know what you need.

| Page | Read it when |
|---|---|
| [Build your first plugin](tutorial.md) | You're starting out. It walks from an empty directory to a finished crawl. |
| [Developing a plugin](developing.md) | You have something running and want to test it, debug it, and iterate quickly. |
| [The manifest reference](manifest.md) | You need the exact meaning of a `plugin.yaml` field. |
| [The protocol reference](protocol.md) | You're writing a worker without the Python SDK, or diagnosing a protocol error. |

If you'd rather read code than prose, the
[`plugins/reference/entity-lines`](https://github.com/mattbriggs/corpus-crawler/tree/main/plugins/reference/entity-lines)
directory is a complete, working plugin with a self-test, and it's a reasonable
thing to copy as a starting point.

## What the host guarantees

Knowing what you can rely on makes it easier to decide what your plugin
shouldn't bother doing. The host promises the following, and none of it is
your responsibility.

- **Your processor is called once per matching file**, with an absolute path.
  Files that don't match your declared extensions never reach you.
- **A crash or a hang costs you one file, not the crawl.** The host replaces
  the worker and keeps going.
- **You may return zero, one, or many records per file.** Returning nothing is
  a success, not an error.
- **Your worker process is reused** across many files, so expensive setup at
  import time is paid once, not per file.
- **Only the host writes the report.** You never open, lock, or coordinate
  access to the CSV.

## What you're responsible for

The corresponding short list is what the host can't do for you. Keep these in
mind as you write, and the rest of the system will stay out of your way.

- **Declaring an accurate schema**, because it becomes the CSV columns.
- **Returning flat records**—strings, integers, numbers, booleans, and nulls.
  Nested objects and lists are rejected.
- **Keeping `stdout` clean.** The Python SDK handles this for you, and
  [Developing a plugin](developing.md) explains what happens if you write a
  worker by hand.
- **Failing honestly.** Raise `PluginProcessingError` when you can't process a
  file, so the host can attribute the failure to it.

## A note on trust

Plugins run as trusted local code with the privileges of whoever ran the
command. The subprocess boundary is there to contain crashes and hangs, not to
contain hostile code, so treat installing a plugin with the same care you'd
give to running any other program. This matters more than it might seem,
because a plugin's manifest names a command that the host will execute.
