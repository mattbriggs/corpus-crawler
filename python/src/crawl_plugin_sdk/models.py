"""Pydantic models for the host-worker JSONL contract.

Pydantic is the authoritative runtime boundary validator on the Python side:
nothing reaches plugin code without having satisfied these models, and nothing
reaches the host without having been produced by them.

:module: crawl_plugin_sdk.models
"""

from __future__ import annotations

from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field

#: The protocol version this SDK implements. It must equal the ``api_version``
#: declared by the plugin manifest and implemented by the host.
API_VERSION = "1"

#: Values a flat record may carry. Nested containers are deliberately absent:
#: v1 records are flat, and a nested value is a contract error the host
#: rejects during schema validation.
ScalarValue = str | int | float | bool | None

#: One output record: a flat mapping of field name to scalar value.
Record = dict[str, ScalarValue]


class Request(BaseModel):
    """One host-to-worker request.

    ``op`` defaults to ``process`` so the minimal documented request,
    ``{"id": 42, "filepath": "/data/a.txt"}``, remains valid.

    :ivar id: Correlation identifier the response must echo.
    :ivar op: Requested operation.
    :ivar filepath: Target file, present exactly for ``process`` requests.
    """

    model_config = ConfigDict(extra="forbid")

    id: int = Field(ge=0)
    op: Literal["process", "handshake", "selftest", "shutdown"] = "process"
    filepath: str | None = None


class PluginError(BaseModel):
    """A plugin-declared processing failure.

    :ivar code: Stable machine-readable error code.
    :ivar message: Human-readable description.
    :ivar detail: Optional plugin-specific diagnostic payload.
    """

    model_config = ConfigDict(extra="forbid")

    code: str
    message: str
    detail: Any | None = None


class ReadyResponse(BaseModel):
    """Handshake acknowledgement, sent once before any file is processed.

    :ivar id: Correlation identifier of the handshake request.
    :ivar status: Always ``ready``.
    :ivar api_version: Protocol version this worker implements.
    :ivar plugin: Optional plugin identity, echoed for diagnostics.
    :ivar sdk: Optional SDK identity, echoed for diagnostics.
    """

    model_config = ConfigDict(extra="forbid")

    id: int = Field(ge=0)
    status: Literal["ready"] = "ready"
    api_version: str = API_VERSION
    plugin: str | None = None
    sdk: str | None = None


class SuccessResponse(BaseModel):
    """Successful processing, carrying zero or more records.

    :ivar id: Correlation identifier.
    :ivar status: Always ``ok``.
    :ivar rows: Zero or more flat records.
    """

    model_config = ConfigDict(extra="forbid")

    id: int = Field(ge=0)
    status: Literal["ok"] = "ok"
    rows: list[Record] = Field(default_factory=list)


class ErrorResponse(BaseModel):
    """Plugin-declared failure for one request.

    :ivar id: Correlation identifier.
    :ivar status: Always ``error``.
    :ivar error: The declared failure.
    """

    model_config = ConfigDict(extra="forbid")

    id: int = Field(ge=0)
    status: Literal["error"] = "error"
    error: PluginError


#: Any message a worker may send.
Response = ReadyResponse | SuccessResponse | ErrorResponse
