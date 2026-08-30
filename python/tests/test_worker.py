"""Worker dispatch and exception-translation tests."""

from __future__ import annotations

import io

from crawl_plugin_sdk.errors import PluginProcessingError
from crawl_plugin_sdk.models import ErrorResponse, ReadyResponse, Request, SuccessResponse
from crawl_plugin_sdk.worker import handle_request, run


def ok_processor(path: str) -> list[dict[str, object]]:
    return [{"filename": path, "line": 1}]


def test_handshake_reports_readiness() -> None:
    response = handle_request(Request(id=0, op="handshake"), ok_processor, plugin="p")
    assert isinstance(response, ReadyResponse)
    assert response.plugin == "p"


def test_shutdown_ends_the_loop() -> None:
    assert handle_request(Request(id=1, op="shutdown"), ok_processor) is None


def test_zero_rows_is_a_success() -> None:
    response = handle_request(Request(id=1, filepath="/a"), lambda _: [])
    assert isinstance(response, SuccessResponse)
    assert response.rows == []


def test_declared_failures_become_error_responses() -> None:
    def failing(_: str) -> list[dict[str, object]]:
        raise PluginProcessingError("bad_input", "cannot parse", {"hint": "check encoding"})

    response = handle_request(Request(id=2, filepath="/a"), failing)
    assert isinstance(response, ErrorResponse)
    assert response.error.code == "bad_input"
    assert response.error.detail == {"hint": "check encoding"}


def test_unexpected_exceptions_fail_only_that_file() -> None:
    def exploding(_: str) -> list[dict[str, object]]:
        raise ValueError("boom")

    response = handle_request(Request(id=3, filepath="/a"), exploding)
    assert isinstance(response, ErrorResponse)
    assert response.error.code == "unhandled_exception"
    assert "traceback" in (response.error.detail or {})


def test_missing_files_are_attributable_errors() -> None:
    def opener(path: str) -> list[dict[str, object]]:
        with open(path, encoding="utf-8"):
            return []

    response = handle_request(Request(id=4, filepath="/definitely/not/here.txt"), opener)
    assert isinstance(response, ErrorResponse)
    assert response.error.code == "file_not_found"


def test_selftest_failure_is_reported_not_raised() -> None:
    def broken() -> None:
        raise AssertionError("expected 2, got 3")

    response = handle_request(Request(id=5, op="selftest"), ok_processor, selftest=broken)
    assert isinstance(response, ErrorResponse)
    assert response.error.code == "selftest_failed"


def test_bad_return_shapes_are_rejected() -> None:
    response = handle_request(Request(id=6, filepath="/a"), lambda p: {"not": "a list"})
    assert isinstance(response, ErrorResponse)
    assert response.error.code == "invalid_return"


def test_run_drives_a_full_session() -> None:
    stdin = io.StringIO(
        '{"id":0,"op":"handshake"}\n{"id":1,"filepath":"/a.txt"}\n\n{"id":2,"op":"shutdown"}\n'
    )
    stdout = io.StringIO()
    assert run(ok_processor, plugin="t", stdin=stdin, stdout=stdout) == 0
    lines = stdout.getvalue().strip().split("\n")
    assert len(lines) == 2
    assert '"status":"ready"' in lines[0]
    assert '"status":"ok"' in lines[1]


def test_protocol_violation_from_the_host_is_fatal() -> None:
    stdin = io.StringIO("not json\n")
    assert run(ok_processor, stdin=stdin, stdout=io.StringIO()) == 1


def test_none_and_empty_returns_are_both_zero_row_successes() -> None:
    for processor in (lambda _: None, lambda _: []):
        response = handle_request(Request(id=9, filepath="/a"), processor)
        assert isinstance(response, SuccessResponse)
        assert response.rows == []


def test_non_mapping_records_are_rejected_with_their_index() -> None:
    response = handle_request(Request(id=10, filepath="/a"), lambda _: [{"ok": 1}, "bad"])
    assert isinstance(response, ErrorResponse)
    assert response.error.code == "invalid_return"
    assert "record 1" in response.error.message


def test_process_request_without_a_filepath_is_an_error() -> None:
    response = handle_request(Request(id=11, op="process"), ok_processor)
    assert isinstance(response, ErrorResponse)
    assert response.error.code == "missing_filepath"


def test_selftest_without_a_declared_hook_succeeds() -> None:
    response = handle_request(Request(id=12, op="selftest"), ok_processor)
    assert isinstance(response, SuccessResponse)


def test_passing_selftest_reports_success() -> None:
    calls: list[int] = []
    response = handle_request(
        Request(id=13, op="selftest"), ok_processor, selftest=lambda: calls.append(1)
    )
    assert isinstance(response, SuccessResponse)
    assert calls == [1]


def test_main_forwards_to_run() -> None:
    from crawl_plugin_sdk.worker import main

    stdin = io.StringIO('{"id":1,"op":"shutdown"}\n')
    assert main(ok_processor, None, stdin=stdin, stdout=io.StringIO()) == 0


def test_os_errors_other_than_missing_files_are_reported_as_access_errors(tmp_path) -> None:
    def opener(path: str) -> list[dict[str, object]]:
        # Opening a directory raises IsADirectoryError, an OSError subclass.
        with open(path, encoding="utf-8"):
            return []

    response = handle_request(Request(id=20, filepath=str(tmp_path)), opener)
    assert isinstance(response, ErrorResponse)
    assert response.error.code == "file_access_error"
