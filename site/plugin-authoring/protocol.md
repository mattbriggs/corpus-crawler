# The protocol reference

The host and each worker talk over a small line-based protocol: one UTF-8 JSON
object per line, sent on `stdin` and received on `stdout`. If you write Python
plugins with the SDK, you never touch this directly—`run` implements all of it.
You'll want this page for one of two reasons: you're implementing a worker in
another language, or you're trying to understand a protocol error the host
reported.

## The shape of a session

A worker's life follows the same sequence every time, and knowing it makes the
individual messages easier to place. The host starts your process, confirms it
works, sends it files one at a time for as long as there's work, and then asks
it to stop.

```text
host                              worker
  |-- {"id":0,"op":"handshake"} ---->|
  |<-- {"id":0,"status":"ready",...}-|
  |-- {"id":1,"filepath":"/a.txt"} ->|
  |<-- {"id":1,"status":"ok",...} ---|
  |-- {"id":2,"filepath":"/b.txt"} ->|
  |<-- {"id":2,"status":"ok",...} ---|
  |-- {"id":N,"op":"shutdown"} ----->|
  |                        (exits)   |
```

Two properties of that exchange are guarantees you can build on. Only one
request is ever in flight per worker, so you never need to handle concurrent
requests or match responses out of order. And every response must echo the
`id` of the request it answers, which is how the host correlates them.

## Requests

The host sends four kinds of request. Only `process` carries a file path, and
because `op` defaults to `process`, the common case stays as small as possible.

```json
{"id": 0, "op": "handshake"}
{"id": 42, "filepath": "/data/a.txt"}
{"id": 1, "op": "selftest"}
{"id": 2, "op": "shutdown"}
```

| Operation | Meaning | Expected response |
|---|---|---|
| `handshake` | Confirm the worker started, loaded the plugin, and speaks this protocol version. Sent once, before any file. | `ready` |
| `process` | Process the file at `filepath`. The default when `op` is absent. | `ok` or `error` |
| `selftest` | Run the plugin's optional self-test. | `ok` or `error` |
| `shutdown` | Exit cleanly. | none; the worker exits |

Paths in `process` requests are always absolute, so a worker can open them
regardless of its own working directory.

## Responses

Your worker replies with one of three message types, distinguished by `status`.

```json
{"id": 0, "status": "ready", "api_version": "1", "plugin": "entity-lines"}
{"id": 42, "status": "ok", "rows": [{"filename": "/data/a.txt", "line": 7}]}
{"id": 42, "status": "ok", "rows": []}
{"id": 42, "status": "error", "error": {"code": "unreadable", "message": "permission denied"}}
```

The `ready` response answers the handshake and must declare the `api_version`
the worker implements. If it doesn't match the host's, the worker is rejected
before any file is dispatched—which is the whole point of having a handshake.

An `ok` response carries a `rows` array, and the array may be empty. This is the
detail most worth internalizing: zero rows is a **success**, meaning the file
was processed and produced nothing. It is not an error, and it doesn't count
against anything. Returning many rows for one file is equally normal.

An `error` response reports a failure your plugin identified in that specific
file. It requires a `code` and a `message`, and may include a `detail` object
for anything else useful. Error information never reaches the CSV; it becomes a
log event and a statistic, and the crawl carries on with the next file.

## Strictness, and why it exists

The host validates every message and rejects anything that doesn't match the
contract exactly. It rejects all of the following:

- malformed JSON, or a line that isn't a JSON object;
- a missing `id`, `status`, `rows`, `code`, or `message`;
- an `id` that isn't a non-negative integer;
- an unrecognized `status`;
- any top-level field the protocol doesn't define;
- a row that isn't a JSON object; and
- a response whose `id` doesn't match the outstanding request.

That last item, and the rejection of undefined fields, can feel unforgiving. The
reasoning is that a stream carrying an unexpected message is a stream the host
can no longer trust: if a worker replies to a request the host didn't send, the
host has no way to tell which response belongs to which file. Rather than guess,
it treats the worker as unusable, replaces it, and retries the file.

This is also why a protocol failure is a different thing from a schema failure.
A schema failure rejects one row and leaves the worker healthy, because the
worker is behaving correctly and only its data is wrong. A protocol failure
invalidates the worker itself.

## Writing a worker by hand

If you're implementing this protocol directly, four rules cover nearly
everything that goes wrong in practice.

- **Write one line, then flush**, for every response. A buffered reply is
  indistinguishable from a hang, and the host will eventually time it out.
- **Never write anything but protocol messages to `stdout`.** Send logs,
  warnings, and progress to `stderr`, which the host captures separately and
  never parses.
- **Run unbuffered.** The host sets `PYTHONUNBUFFERED=1` for Python workers;
  other runtimes need their own equivalent.
- **Echo the request `id`** on every response.

The first two rules are the ones people break. Any library that prints to
`stdout`—a progress bar, a warning, a stray debug statement—will corrupt the
stream, which is precisely why the Python SDK takes ownership of the descriptor
and redirects `sys.stdout` to `stderr` on your behalf.

## Versioning

Compatibility between a plugin and the host is governed by a single
`api_version` token, shared by the manifest and the handshake. The current
version is `"1"`. A worker declaring anything else is rejected at handshake,
which means an incompatible plugin fails immediately and visibly rather than
producing subtly wrong output partway through a crawl.
