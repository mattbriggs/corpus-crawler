"""Exceptions a plugin raises to report a file-level failure.

:module: crawl_plugin_sdk.errors
"""

from __future__ import annotations

from typing import Any


class PluginProcessingError(Exception):
    """A file could not be processed, reported to the host as a plugin error.

    Raising this from a processor is the supported way to fail one file
    without ending the crawl. The host records the failure, attributes it to
    the file, and continues with the remaining work.

    :param code: Stable machine-readable code, for example ``unreadable_file``.
    :param message: Human-readable description of the failure.
    :param detail: Optional JSON-serializable diagnostic payload.
    """

    def __init__(self, code: str, message: str, detail: Any | None = None) -> None:
        """Build a failure report for one file. See the class docstring."""
        super().__init__(message)
        self.code = code
        self.message = message
        self.detail = detail


class ProtocolViolationError(Exception):
    """The host sent a message this SDK version cannot interpret.

    Raised only by the SDK itself. A plugin author should never need to catch
    it: it means the host and worker disagree about the protocol contract.
    """
