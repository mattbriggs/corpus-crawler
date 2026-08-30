"""Framing and encoding tests."""

from __future__ import annotations

import json

import pytest

from crawl_plugin_sdk.errors import ProtocolViolationError
from crawl_plugin_sdk.models import ErrorResponse, PluginError, ReadyResponse, SuccessResponse
from crawl_plugin_sdk.protocol import decode_request, encode_response


def test_encoded_lines_are_compact_and_newline_terminated() -> None:
    line = encode_response(SuccessResponse(id=42, rows=[{"a": 1}]))
    assert line == '{"id":42,"status":"ok","rows":[{"a":1}]}\n'


def test_non_ascii_is_not_escaped() -> None:
    line = encode_response(SuccessResponse(id=1, rows=[{"path": "/data/文件.txt"}]))
    assert "文件" in line
    assert json.loads(line)["rows"][0]["path"] == "/data/文件.txt"


def test_optional_fields_are_omitted() -> None:
    assert encode_response(ReadyResponse(id=0)) == '{"id":0,"status":"ready","api_version":"1"}\n'
    encoded = encode_response(ErrorResponse(id=1, error=PluginError(code="x", message="y")))
    assert encoded == '{"id":1,"status":"error","error":{"code":"x","message":"y"}}\n'


def test_blank_and_malformed_lines_are_protocol_violations() -> None:
    for bad in ["", "   \n", "not json", "[1,2]"]:
        with pytest.raises(ProtocolViolationError):
            decode_request(bad)


def test_valid_lines_decode() -> None:
    assert decode_request('{"id":7,"filepath":"/a"}\n').id == 7
