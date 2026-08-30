"""JSONL framing and encoding.

:module: crawl_plugin_sdk.protocol
"""

from __future__ import annotations

import json
from typing import Any

from .errors import ProtocolViolationError
from .models import Request, Response


def decode_request(line: str) -> Request:
    """Decode one protocol line into a :class:`~crawl_plugin_sdk.models.Request`.

    :param line: One complete line read from stdin, newline optional.
    :returns: The validated request.
    :raises ProtocolViolationError: If the line is not a JSON object matching the
        request contract. The worker treats this as fatal, because a
        misunderstood request cannot be answered safely.
    """
    text = line.strip()
    if not text:
        raise ProtocolViolationError("received an empty protocol line")
    try:
        payload: Any = json.loads(text)
    except json.JSONDecodeError as error:  # pragma: no cover - defensive
        raise ProtocolViolationError(f"host sent malformed JSON: {error}") from error
    if not isinstance(payload, dict):
        raise ProtocolViolationError("host message must be a JSON object")
    try:
        return Request.model_validate(payload)
    except Exception as error:
        raise ProtocolViolationError(
            f"host message violates the request contract: {error}"
        ) from error


def encode_response(response: Response) -> str:
    """Encode a response as one newline-terminated protocol line.

    Uses the compact separators and non-ASCII passthrough that the host's
    canonical encoding expects, so cross-language contract fixtures compare
    byte for byte.

    :param response: The message to send.
    :returns: The encoded line, including its trailing newline.
    """
    payload = response.model_dump(exclude_none=True)
    return json.dumps(payload, separators=(",", ":"), ensure_ascii=False) + "\n"
