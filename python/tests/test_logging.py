"""Structured diagnostic logging tests."""

from __future__ import annotations

import json
import logging

from crawl_plugin_sdk.logging import JsonFormatter, configure


def test_records_render_as_single_line_json() -> None:
    record = logging.LogRecord("crawl.plugin", logging.WARNING, "f.py", 1, "careful", None, None)
    payload = json.loads(JsonFormatter().format(record))
    assert payload == {"level": "warning", "logger": "crawl.plugin", "message": "careful"}


def test_exceptions_are_included() -> None:
    try:
        raise ValueError("boom")
    except ValueError:
        import sys

        record = logging.LogRecord(
            "crawl.plugin", logging.ERROR, "f.py", 1, "failed", None, sys.exc_info()
        )
    payload = json.loads(JsonFormatter().format(record))
    assert "ValueError: boom" in payload["exception"]


def test_configure_writes_to_stderr_only(capsys) -> None:
    logger = configure(level=logging.INFO, name="crawl.test")
    logger.info("hello %s", "world")
    captured = capsys.readouterr()
    assert captured.out == "", "diagnostics must never reach protocol stdout"
    assert json.loads(captured.err.strip())["message"] == "hello world"


def test_configure_is_idempotent() -> None:
    first = configure(name="crawl.idempotent")
    second = configure(name="crawl.idempotent")
    assert first is second
    assert len(second.handlers) == 1, "reconfiguring must not duplicate handlers"
