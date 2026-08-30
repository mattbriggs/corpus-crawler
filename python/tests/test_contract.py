"""Cross-language protocol contract tests.

Reads the same fixture file as the Rust contract test, so the two
implementations cannot drift apart silently.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from pydantic import ValidationError

from crawl_plugin_sdk.errors import ProtocolViolationError
from crawl_plugin_sdk.models import ErrorResponse, PluginError, ReadyResponse, SuccessResponse
from crawl_plugin_sdk.protocol import decode_request, encode_response

FIXTURES = Path(__file__).resolve().parents[2] / "tests/fixtures/protocol/cases.json"
CASES = json.loads(FIXTURES.read_text(encoding="utf-8"))


@pytest.mark.parametrize("case", CASES["requests"], ids=lambda c: c["name"])
def test_python_validates_every_host_produced_request(case: dict) -> None:
    """Requests the Rust host encodes must satisfy the Pydantic contract."""
    request = decode_request(case["encoded"])
    expected = case["decoded"]
    assert request.id == expected["id"]
    assert request.op == expected["op"]
    assert request.filepath == expected["filepath"]


@pytest.mark.parametrize("case", CASES["valid_responses"], ids=lambda c: c["name"])
def test_python_reproduces_the_canonical_response_bytes(case: dict) -> None:
    """Responses Python encodes must be byte-identical to what Rust decodes."""
    if case["kind"] == "ready":
        response = ReadyResponse(id=case["id"])
    elif case["kind"] == "ok":
        response = SuccessResponse(id=case["id"], rows=case["rows"])
    else:
        response = ErrorResponse(id=case["id"], error=PluginError(**case["error"]))
    assert encode_response(response).rstrip("\n") == case["encoded"]


# Every payload the Rust host rejects is prevented on the Python side by exactly
# one mechanism. Naming which one per fixture keeps this test specific: a bare
# "somehow prevented" check would pass even if Pydantic silently started
# accepting a malformed message and the SDK began emitting it.
#
#   unparseable   - not JSON at all, so no model could produce it
#   not_an_object - valid JSON, but not a mapping
#   no_model      - no `status` this SDK can emit, so there is no model for it
#   rejected      - a model exists and raises ValidationError
#   normalised    - a model accepts it but encodes something else
PREVENTION = {
    "not_json": "unparseable",
    "empty_line": "unparseable",
    "truncated_json": "unparseable",
    "json_array": "not_an_object",
    "json_scalar": "not_an_object",
    "missing_status": "no_model",
    "unknown_status": "no_model",
    "missing_id": "rejected",
    "missing_error_code": "rejected",
    "negative_id": "rejected",
    "rows_not_an_array": "rejected",
    "extra_protocol_field": "rejected",
    "row_not_an_object": "rejected",
    "missing_rows": "normalised",
    "string_id": "normalised",
}

MODELS = {"ok": SuccessResponse, "error": ErrorResponse, "ready": ReadyResponse}


def test_every_invalid_fixture_has_a_declared_prevention_mechanism() -> None:
    """Adding a fixture must mean deciding how Python is stopped from emitting it."""
    fixtures = {case["name"] for case in CASES["invalid_responses"]}
    assert fixtures == set(PREVENTION), (
        "PREVENTION and the fixture file disagree; "
        f"only in fixtures: {fixtures - set(PREVENTION)}, "
        f"only in PREVENTION: {set(PREVENTION) - fixtures}"
    )


@pytest.mark.parametrize("case", CASES["invalid_responses"], ids=lambda c: c["name"])
def test_python_is_prevented_from_emitting_host_rejected_bytes(case: dict) -> None:
    """The SDK cannot put a host-rejected line on the wire, for the stated reason."""
    expected = PREVENTION[case["name"]]

    try:
        payload = json.loads(case["encoded"])
    except json.JSONDecodeError:
        assert expected == "unparseable"
        return

    if not isinstance(payload, dict):
        assert expected == "not_an_object"
        return

    model = MODELS.get(payload.get("status"))
    if model is None:
        assert expected == "no_model", (
            f"{case['name']} has no model, but was expected to be {expected}"
        )
        return

    try:
        validated = model.model_validate(payload)
    except ValidationError:
        assert expected == "rejected", (
            f"{case['name']} was rejected, but was expected to be {expected}"
        )
        return

    assert expected == "normalised", (
        f"{case['name']} validated cleanly, but was expected to be {expected}"
    )
    encoded = encode_response(validated).rstrip("\n")
    assert encoded != case["encoded"], "the SDK would emit bytes the host rejects"
    # What it emits instead must be something the host accepts.
    assert json.loads(encoded)["id"] >= 0


def test_protocol_version_matches_the_fixture_file() -> None:
    from crawl_plugin_sdk.models import API_VERSION

    assert CASES["protocol_version"] == API_VERSION


def test_blank_protocol_lines_are_rejected_in_both_languages() -> None:
    with pytest.raises(ProtocolViolationError):
        decode_request("")
