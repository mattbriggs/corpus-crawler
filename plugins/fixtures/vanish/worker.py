"""Fixture worker: deletes its sibling files as it processes.

This makes the filesystem race of REQ-171 deterministic. Every file is
discovered and queued before any is processed, so whichever file this worker is
given first deletes the rest while they are still queued. Each remaining file
then fails when the worker tries to open it.
"""
import json, os, sys


def send(obj):
    sys.stdout.write(json.dumps(obj, separators=(",", ":")) + "\n")
    sys.stdout.flush()


for raw in sys.stdin:
    raw = raw.strip()
    if not raw:
        continue
    request = json.loads(raw)
    op = request.get("op", "process")
    if op == "shutdown":
        break
    if op == "handshake":
        send({"id": request["id"], "status": "ready", "api_version": "1"})
        continue

    path = request["filepath"]
    directory = os.path.dirname(path)
    for sibling in os.listdir(directory):
        candidate = os.path.join(directory, sibling)
        if candidate != path and candidate.endswith(".txt"):
            os.remove(candidate)

    try:
        with open(path, encoding="utf-8") as handle:
            rows = [
                {"filename": path, "line": number, "text": line.rstrip("\n")}
                for number, line in enumerate(handle, start=1)
            ]
        send({"id": request["id"], "status": "ok", "rows": rows})
    except OSError as error:
        send({"id": request["id"], "status": "error",
              "error": {"code": "file_access_error", "message": str(error)}})
