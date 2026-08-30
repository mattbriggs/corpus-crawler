# crawl-plugin-sdk

Python SDK for writing plugin workers for the [`crawl`](../README.md) file
processing runtime.

```bash
python3 -m venv .venv
.venv/bin/python -m pip install -e "python[dev]"
.venv/bin/python -m pytest python/tests
```

A plugin is a directory with a `plugin.yaml` manifest and a worker module:

```python
from crawl_plugin_sdk import PluginProcessingError, run


def process(path: str) -> list[dict[str, object]]:
    """Return zero or more flat records for one file."""
    with open(path, encoding="utf-8") as handle:
        return [
            {"filename": path, "line": number, "text": line.rstrip("\n")}
            for number, line in enumerate(handle, start=1)
        ]


if __name__ == "__main__":
    raise SystemExit(run(process, plugin="line-text"))
```

The SDK owns JSONL framing, request correlation, stdout protection, and the
translation of exceptions into protocol error responses. Raise
`PluginProcessingError` to fail one file without ending the crawl.
