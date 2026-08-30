"""The persistent worker loop plugin authors build on.

A conforming plugin is a module whose ``__main__`` calls :func:`run` with a
processor function. Everything else - framing, correlation, error translation,
and stdout protection - is the SDK's responsibility.

:module: crawl_plugin_sdk.worker
"""

from __future__ import annotations

import os
import sys
import traceback
from collections.abc import Callable, Iterable, Sequence
from typing import TextIO

from .errors import PluginProcessingError, ProtocolViolationError
from .models import (
    API_VERSION,
    ErrorResponse,
    PluginError,
    ReadyResponse,
    Record,
    Request,
    Response,
    SuccessResponse,
)
from .protocol import decode_request, encode_response

#: A processor receives one absolute file path and returns zero or more records.
Processor = Callable[[str], Iterable[Record]]

#: A self-test takes no arguments and raises on failure.
SelfTest = Callable[[], None]


def _protect_stdout() -> TextIO:
    """Take exclusive ownership of the protocol stream.

    A plugin that calls :func:`print`, or a library that writes a progress bar
    to stdout, would otherwise corrupt the protocol. The real stdout file
    descriptor is duplicated for protocol use and ``sys.stdout`` is redirected
    to stderr, so stray output becomes a harmless diagnostic instead of a
    protocol violation.

    :returns: A text stream writing to the original stdout.
    """
    protocol_fd = os.dup(1)
    protocol = os.fdopen(protocol_fd, "w", encoding="utf-8", newline="\n")
    os.dup2(2, 1)
    sys.stdout = sys.stderr
    return protocol


def _to_records(returned: Iterable[Record] | None) -> list[Record]:
    """Normalize a processor's return value into a list of records.

    :param returned: What the processor returned; ``None`` means no records.
    :returns: A list of records.
    :raises PluginProcessingError: If the value is not a sequence of mappings.
    """
    if returned is None:
        return []
    if isinstance(returned, dict):
        raise PluginProcessingError(
            "invalid_return",
            "processor returned a single mapping; return a list of records",
        )
    rows = list(returned)
    for index, row in enumerate(rows):
        if not isinstance(row, dict):
            raise PluginProcessingError(
                "invalid_return",
                f"record {index} is a {type(row).__name__}, expected a mapping",
            )
    return rows


def handle_request(
    request: Request,
    processor: Processor,
    *,
    plugin: str | None = None,
    selftest: SelfTest | None = None,
) -> Response | None:
    """Produce the response for one request.

    Exposed separately from :func:`run` so tests can exercise dispatch without
    driving real pipes.

    :param request: The decoded request.
    :param processor: The plugin's per-file processor.
    :param plugin: Optional plugin identity echoed at handshake.
    :param selftest: Optional registration-time self-test.
    :returns: The response to send, or ``None`` for ``shutdown``.
    """
    if request.op == "shutdown":
        return None

    if request.op == "handshake":
        return ReadyResponse(
            id=request.id,
            api_version=API_VERSION,
            plugin=plugin,
            sdk="crawl-plugin-sdk/0.1.0",
        )

    if request.op == "selftest":
        if selftest is None:
            return SuccessResponse(id=request.id, rows=[])
        try:
            selftest()
        except Exception as error:  # noqa: BLE001 - reported, never raised
            return ErrorResponse(
                id=request.id,
                error=PluginError(
                    code="selftest_failed",
                    message=str(error),
                    detail={"traceback": traceback.format_exc()},
                ),
            )
        return SuccessResponse(id=request.id, rows=[])

    if request.filepath is None:
        return ErrorResponse(
            id=request.id,
            error=PluginError(
                code="missing_filepath",
                message="process request carried no filepath",
            ),
        )

    try:
        rows = _to_records(processor(request.filepath))
    except PluginProcessingError as error:
        return ErrorResponse(
            id=request.id,
            error=PluginError(code=error.code, message=error.message, detail=error.detail),
        )
    except FileNotFoundError as error:
        # A file discovered by the host may vanish before the worker opens it.
        # That is an ordinary, attributable failure, not a plugin defect.
        #
        # This clause must stay above the OSError one below: FileNotFoundError
        # is a subclass of OSError, so reordering them would silently collapse
        # the two into one less specific error code.
        return ErrorResponse(
            id=request.id,
            error=PluginError(code="file_not_found", message=str(error)),
        )
    except OSError as error:
        return ErrorResponse(
            id=request.id,
            error=PluginError(code="file_access_error", message=str(error)),
        )
    except Exception as error:  # noqa: BLE001 - any plugin bug becomes one failed file
        return ErrorResponse(
            id=request.id,
            error=PluginError(
                code="unhandled_exception",
                message=f"{type(error).__name__}: {error}",
                detail={"traceback": traceback.format_exc()},
            ),
        )

    try:
        return SuccessResponse(id=request.id, rows=rows)
    except Exception as error:  # noqa: BLE001 - a record that cannot be serialized
        return ErrorResponse(
            id=request.id,
            error=PluginError(code="invalid_record", message=str(error)),
        )


def run(
    processor: Processor,
    *,
    plugin: str | None = None,
    selftest: SelfTest | None = None,
    stdin: TextIO | None = None,
    stdout: TextIO | None = None,
) -> int:
    """Run the persistent worker loop until the host closes stdin.

    The loop is the whole plugin lifecycle: one interpreter serves many files,
    which is what amortizes Python startup across a crawl.

    :param processor: Called once per file with an absolute path.
    :param plugin: Optional plugin identity echoed at handshake.
    :param selftest: Optional registration-time self-test.
    :param stdin: Override the input stream, for tests.
    :param stdout: Override the protocol stream, for tests.
    :returns: The process exit status: ``0`` normally, ``1`` after a protocol
        violation.
    """
    protocol = stdout if stdout is not None else _protect_stdout()
    source = stdin if stdin is not None else sys.stdin

    for line in source:
        if not line.strip():
            continue
        try:
            request = decode_request(line)
        except ProtocolViolationError as error:
            print(f"crawl-plugin-sdk: {error}", file=sys.stderr, flush=True)
            return 1

        response = handle_request(request, processor, plugin=plugin, selftest=selftest)
        if response is None:
            break
        protocol.write(encode_response(response))
        protocol.flush()
    return 0


def main(processor: Processor, argv: Sequence[str] | None = None, **kwargs: object) -> int:
    """Entry-point helper for ``if __name__ == "__main__":`` blocks.

    :param processor: The plugin's per-file processor.
    :param argv: Unused; accepted so the helper matches the usual signature.
    :param kwargs: Forwarded to :func:`run`.
    :returns: The process exit status.
    """
    del argv
    return run(processor, **kwargs)  # type: ignore[arg-type]
