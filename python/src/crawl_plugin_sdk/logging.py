"""Structured diagnostic logging for plugin workers.

Worker ``stdout`` is reserved for protocol messages, so every diagnostic goes
to ``stderr``. The host captures it separately and never parses it as
protocol.

:module: crawl_plugin_sdk.logging
"""

from __future__ import annotations

import json
import logging
import sys
from typing import Any


class JsonFormatter(logging.Formatter):
    """Render log records as one JSON object per line."""

    def format(self, record: logging.LogRecord) -> str:
        """Return the record as a compact JSON line.

        :param record: The record to render.
        :returns: A single-line JSON document.
        """
        payload: dict[str, Any] = {
            "level": record.levelname.lower(),
            "logger": record.name,
            "message": record.getMessage(),
        }
        if record.exc_info:
            payload["exception"] = self.formatException(record.exc_info)
        return json.dumps(payload, separators=(",", ":"), ensure_ascii=False)


def configure(level: int = logging.INFO, *, name: str = "crawl.plugin") -> logging.Logger:
    """Configure a worker logger that writes structured lines to ``stderr``.

    :param level: Minimum level to emit.
    :param name: Logger name.
    :returns: The configured logger.
    """
    logger = logging.getLogger(name)
    logger.setLevel(level)
    logger.handlers.clear()
    handler = logging.StreamHandler(sys.stderr)
    handler.setFormatter(JsonFormatter())
    logger.addHandler(handler)
    logger.propagate = False
    return logger
