"""Python SDK for writing ``crawl`` plugin workers.

A minimal plugin is one processor function and one call to :func:`run`::

    from crawl_plugin_sdk import run

    def process(path: str) -> list[dict[str, object]]:
        with open(path, encoding="utf-8") as handle:
            return [{"filename": path, "lines": sum(1 for _ in handle)}]

    if __name__ == "__main__":
        raise SystemExit(run(process, plugin="line-count"))

:module: crawl_plugin_sdk
"""

from .errors import PluginProcessingError, ProtocolViolationError
from .models import (
    API_VERSION,
    ErrorResponse,
    PluginError,
    ReadyResponse,
    Record,
    Request,
    Response,
    ScalarValue,
    SuccessResponse,
)
from .protocol import decode_request, encode_response
from .worker import handle_request, main, run

__all__ = [
    "API_VERSION",
    "ErrorResponse",
    "PluginError",
    "PluginProcessingError",
    "ProtocolViolationError",
    "ReadyResponse",
    "Record",
    "Request",
    "Response",
    "ScalarValue",
    "SuccessResponse",
    "decode_request",
    "encode_response",
    "handle_request",
    "main",
    "run",
]

__version__ = "0.1.0"
