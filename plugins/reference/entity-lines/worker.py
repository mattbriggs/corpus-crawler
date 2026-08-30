"""Reference plugin: extract capitalised entities with their line numbers.

Demonstrates the whole plugin contract: a processor returning many records per
file, a declared self-test, and failure reporting through the SDK.

:module: entity_lines.worker
"""

from __future__ import annotations

import re

from crawl_plugin_sdk import PluginProcessingError, Record, run

#: Matches one or more consecutive capitalised words.
ENTITY = re.compile(r"\b[A-Z][a-zA-Z]+(?:\s+[A-Z][a-zA-Z]+)*\b")


def process(path: str) -> list[Record]:
    """Return one record per entity occurrence in the file.

    :param path: Absolute path supplied by the host.
    :returns: Zero or more flat records matching the declared schema.
    :raises PluginProcessingError: If the file is not valid UTF-8 text.
    """
    records: list[Record] = []
    try:
        with open(path, encoding="utf-8") as handle:
            for number, line in enumerate(handle, start=1):
                for match in ENTITY.finditer(line):
                    records.append({"filename": path, "line": number, "entity": match.group(0)})
    except UnicodeDecodeError as error:
        raise PluginProcessingError("not_utf8_text", f"{path} is not UTF-8 text") from error
    return records


def selftest() -> None:
    """Verify the extractor against a known input.

    :raises AssertionError: If extraction does not behave as documented.
    """
    import tempfile

    with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False, encoding="utf-8") as handle:
        handle.write("Alice met Bob Smith.\nnothing here\n")
        name = handle.name
    rows = process(name)
    assert [row["entity"] for row in rows] == ["Alice", "Bob Smith"], rows


if __name__ == "__main__":
    raise SystemExit(run(process, plugin="entity-lines", selftest=selftest))
