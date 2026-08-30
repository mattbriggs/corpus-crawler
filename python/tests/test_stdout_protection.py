"""The SDK must own protocol stdout absolutely.

A plugin that calls ``print``, or a library that writes a progress bar to
stdout, must not be able to corrupt the protocol stream. This is verified in a
real subprocess because the protection works by duplicating file descriptor 1,
which cannot be observed meaningfully in-process.
"""

from __future__ import annotations

import json
import subprocess
import sys
import textwrap
from pathlib import Path

WORKER = textwrap.dedent(
    """
    import sys
    from crawl_plugin_sdk import run

    def process(path):
        # Every one of these would corrupt an unprotected protocol stream.
        print("a bare print")
        sys.stdout.write("a direct stdout write\\n")
        sys.stdout.flush()
        return [{"filename": path, "line": 1}]

    raise SystemExit(run(process, plugin="noisy"))
    """
)


def run_worker(tmp_path: Path, requests: list[dict]) -> subprocess.CompletedProcess[str]:
    """Run a worker subprocess, feeding it one request per line."""
    script = tmp_path / "noisy_worker.py"
    script.write_text(WORKER, encoding="utf-8")
    stdin = "".join(json.dumps(request) + "\n" for request in requests)
    return subprocess.run(
        [sys.executable, str(script)],
        input=stdin,
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )


def test_stray_stdout_writes_never_reach_the_protocol_stream(tmp_path: Path) -> None:
    result = run_worker(
        tmp_path,
        [
            {"id": 0, "op": "handshake"},
            {"id": 1, "filepath": "/data/a.txt"},
            {"id": 2, "op": "shutdown"},
        ],
    )
    assert result.returncode == 0, result.stderr

    # Every stdout line must be a valid protocol message, in order.
    lines = [line for line in result.stdout.splitlines() if line]
    messages = [json.loads(line) for line in lines]
    assert len(messages) == 2, f"unexpected protocol output: {lines}"
    assert messages[0]["status"] == "ready"
    assert messages[1] == {
        "id": 1,
        "status": "ok",
        "rows": [{"filename": "/data/a.txt", "line": 1}],
    }

    # The stray output was not lost - it was redirected to diagnostics.
    assert "a bare print" in result.stderr
    assert "a direct stdout write" in result.stderr


def test_a_worker_exits_cleanly_when_stdin_closes(tmp_path: Path) -> None:
    # No shutdown request: EOF alone must end the loop.
    result = run_worker(tmp_path, [{"id": 0, "op": "handshake"}])
    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout.splitlines()[0])["status"] == "ready"
