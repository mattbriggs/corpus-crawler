"""Fixture worker: the happy path. Emits one record per line of each file."""
import json, os, sys

# When the host sets CRAWL_PID_DIR, each worker records its own process id by
# creating a file named after it. Counting those files tells a test exactly how
# many worker processes the host started, which is the only direct way to
# observe worker bounding and reuse from outside.
_pid_dir = os.environ.get("CRAWL_PID_DIR")
if _pid_dir:
    open(os.path.join(_pid_dir, str(os.getpid())), "w").close()

def send(obj):
    sys.stdout.write(json.dumps(obj, separators=(",", ":")) + "\n")
    sys.stdout.flush()

def rows_for(path):
    out = []
    with open(path, encoding="utf-8") as handle:
        for number, line in enumerate(handle, start=1):
            out.append({"filename": path, "line": number, "text": line.rstrip("\n")})
    return out

for raw in sys.stdin:
    raw = raw.strip()
    if not raw:
        continue
    request = json.loads(raw)
    op = request.get("op", "process")
    if op == "shutdown":
        break
    if op == "handshake":
        send({"id": request["id"], "status": "ready", "api_version": "1", "plugin": "fixture-echo"})
        continue
    if op == "selftest":
        send({"id": request["id"], "status": "ok", "rows": []})
        continue
    try:
        send({"id": request["id"], "status": "ok", "rows": rows_for(request["filepath"])})
    except OSError as error:
        send({"id": request["id"], "status": "error",
              "error": {"code": "file_access_error", "message": str(error)}})
