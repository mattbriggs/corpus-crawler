# The manifest reference

Every plugin is described by a `plugin.yaml` file that sits in the plugin's root
directory. The manifest is how the host learns everything it needs to know about
your plugin without reading your code: what to call it, how to start it, which
files to send it, and what columns its output will fill. This page documents
every field; if you're writing your first manifest, the
[tutorial](tutorial.md) is a gentler introduction.

## A complete manifest

The example below uses every available field, with the optional ones marked.
Most plugins need considerably less than this.

```yaml
api_version: "1"              # required; only "1" is supported

plugin:
  name: entity-lines          # required; [a-z0-9][a-z0-9._-]*, max 64 bytes
  version: "1.0.0"            # required; any non-empty token without whitespace
  description: Extract entities and source line numbers   # optional

runtime:
  type: python                # required; only "python" in v1
  command: ["python3", "worker.py"]   # required; argv, first element is the program
  working_directory: .        # optional; relative to the plugin root
  env:                        # optional
    - { name: MY_SETTING, value: "1" }
  selftest: true              # optional; declares support for the selftest request

input:
  extensions: [".txt", ".md"] # required; at least one
  follow_symlinks: false      # optional; defaults to false

output:
  format: records             # optional; only "records" in v1
  extra_fields: reject        # optional; reject (default) or ignore
  schema:                     # required; declaration order is CSV column order
    filename: { type: string,  required: true }
    line:     { type: integer, required: true }
    entity:   { type: string,  required: false }

execution:                    # all optional; the CLI overrides these
  workers: auto               # auto, or a positive integer
  timeout_seconds: 30         # positive
  max_retries: 1              # zero or more
  queue_capacity: 1024        # positive
```

Before working through the blocks individually, it's worth knowing one thing
about how the file is read.

## Unknown keys are rejected

The host rejects any manifest containing a field it doesn't recognize. A
misspelled key is a failed installation rather than a setting that silently does
nothing, which is deliberate: a manifest is a contract, and a contract that
quietly ignores half of what you wrote isn't one. If installation fails with a
message about an unknown field, check your spelling and indentation before
looking anywhere else.

With that in mind, the blocks below can be taken one at a time.

## `plugin`: identity

The `plugin` block gives your plugin the name the registry knows it by. Names
are restricted to lowercase letters, digits, hyphens, underscores, and periods,
and must begin with a letter or digit—a restriction that keeps names usable as
registry keys and filesystem paths on every supported platform.

Versions are deliberately unconstrained beyond being non-empty and
whitespace-free. Semantic versioning is a good idea, but the host doesn't
require it, so `2024-03-release` is as acceptable as `1.2.3`. The registry is
keyed by name alone, which means installing a plugin whose name is already
registered replaces the existing entry rather than creating a second one; the
host asks you to confirm that with `--force`.

## `runtime`: how to start a worker

The `runtime` block tells the host what command to execute. The `command` field
is an argument vector, not a shell string, so no quoting or shell expansion
happens: the first element is the program and the rest are its arguments,
passed through exactly as written.

The interpreter you name here is the one your worker runs under, and it must be
able to import your dependencies. Because the host neither creates nor manages
Python environments, pointing `command` at a virtual environment's interpreter
is how a plugin with dependencies gets them:

```yaml
command: ["/path/to/.venv/bin/python", "worker.py"]
```

Workers start with the plugin's root directory as their working directory, which
is why the relative `worker.py` above resolves correctly. Set
`working_directory` only if you need something else; a relative value is
resolved against the plugin root, and an absolute one is used as-is. The host
also sets two environment variables for you—`PYTHONUNBUFFERED=1`, so responses
aren't held in a buffer, and `CRAWL_API_VERSION`—and `env` adds any others your
plugin needs.

Setting `selftest: true` declares that your worker implements the optional
self-test request, which `crawl plugin validate` will then run. See
[Developing a plugin](developing.md#add-a-self-test) for what to do with it.

## `input`: which files you receive

The `input` block decides which discovered files reach your processor. You must
declare at least one extension, and the host normalizes what you write: leading
periods are optional and case is ignored, so `TXT`, `txt`, and `.txt` all become
`.txt`.

Matching then follows one rule that surprises people exactly once. Only the
final extension is considered, so `archive.tar.gz` matches `.gz` and never
`.tar.gz`, and a file with no extension—`README`, or a dotfile like
`.gitignore`—never matches anything. If your file counts come back at zero, this
rule is the first thing to check.

The `follow_symlinks` field controls whether traversal descends through symbolic
links, and it defaults to `false`. With the default, symlinks are skipped
entirely rather than being handed to your plugin as files, so you never receive
a path that turns out to be a directory.

## `output`: the schema that becomes your CSV

The `output.schema` block is the most consequential part of the manifest,
because it defines the report's columns in the order you declare them. Every
key must match a key your processor returns, and the type must match the value.

Four types are available, and none of them coerce:

| Type | Accepts | Rendered in CSV as |
|---|---|---|
| `string` | JSON strings only | the string itself |
| `integer` | JSON integers, and floats with no fractional part | digits, no decimal point |
| `number` | JSON integers and finite floats | shortest round-trippable form |
| `boolean` | JSON booleans only | `true` or `false` |

The absence of coercion is worth stating plainly: `"3"` will not satisfy an
`integer` column, and `1` will not satisfy a `boolean` one. The single exception
is that a float with a zero fractional part satisfies `integer`, because JSON
encoders legitimately render `7` as `7.0`.

Nullability uses one convention throughout. A field marked `required: true` must
be present and non-null in every record; the default, `required: false`, means
the field may be absent or null, and renders as an empty CSV field. Records are
also flat—a nested object or list as a value is rejected, so serialize structure
into a string and declare that column as `string`.

Finally, `extra_fields` decides what happens when a record carries a key the
schema doesn't declare. The default, `reject`, discards the whole record and
logs why, which keeps the output contract exact. Setting it to `ignore` drops
the undeclared keys and keeps the record, which is useful when your processor
legitimately returns more than you want to report.

## `execution`: defaults you can override

The `execution` block lets you suggest operational defaults suited to your
plugin—a slow parser might want a longer timeout, a memory-hungry one fewer
workers. Every field is optional, and each is only a default.

Whoever runs the crawl has the final say, following one precedence rule:

```text
CLI option > manifest execution block > host default
```

That ordering means you should set these values to what your plugin normally
needs, not to what you want to force. An operator who passes `--workers 2` has
a reason, and the manifest doesn't override it.

## Validation rules at a glance

The host checks all of the following before registering a plugin, so a manifest
that installs successfully is one the crawl engine can rely on.

| Field | Rule |
|---|---|
| `api_version` | Must be `"1"` |
| `plugin.name` | Non-empty, at most 64 bytes, `[a-z0-9][a-z0-9._-]*` |
| `plugin.version` | Non-empty, no whitespace |
| `runtime.type` | Must be `python` |
| `runtime.command` | At least one element, first non-empty |
| `input.extensions` | At least one; no whitespace, path separators, or interior periods |
| `output.schema` | At least one field; no duplicate names; every type supported |
| `execution.workers` | `auto`, or an integer greater than zero |
| `execution.timeout_seconds` | Greater than zero |
| `execution.max_retries` | Zero or more |
| `execution.queue_capacity` | Greater than zero |
