"""Contract tests for the Pydantic boundary models."""

from __future__ import annotations

import pytest
from pydantic import ValidationError

from crawl_plugin_sdk.models import (
    API_VERSION,
    ErrorResponse,
    PluginError,
    ReadyResponse,
    Request,
    SuccessResponse,
)


def test_process_is_the_default_operation() -> None:
    request = Request.model_validate({"id": 42, "filepath": "/data/a.txt"})
    assert request.op == "process"
    assert request.filepath == "/data/a.txt"


def test_control_requests_need_no_filepath() -> None:
    assert Request.model_validate({"id": 0, "op": "handshake"}).filepath is None


def test_unknown_request_fields_are_rejected() -> None:
    with pytest.raises(ValidationError):
        Request.model_validate({"id": 1, "filepath": "/a", "extra": True})


def test_negative_ids_are_rejected() -> None:
    with pytest.raises(ValidationError):
        Request.model_validate({"id": -1, "filepath": "/a"})


def test_unknown_operations_are_rejected() -> None:
    with pytest.raises(ValidationError):
        Request.model_validate({"id": 1, "op": "explode"})


def test_success_response_defaults_to_zero_rows() -> None:
    response = SuccessResponse(id=1)
    assert response.status == "ok"
    assert response.rows == []


def test_ready_response_declares_the_api_version() -> None:
    assert ReadyResponse(id=0).api_version == API_VERSION


def test_error_response_requires_code_and_message() -> None:
    with pytest.raises(ValidationError):
        ErrorResponse(id=1, error=PluginError.model_validate({"message": "no code"}))
