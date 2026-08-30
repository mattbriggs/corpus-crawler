"""Fixture worker: writes diagnostics to stderr while succeeding on stdout."""
import json, sys

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
        send({"id": request["id"], "status": "ready", "api_version": "1"})
        continue
    sys.stderr.write("fixture-stderr: noisy diagnostic for %s\n" % request.get("filepath"))
    sys.stderr.flush()
    send({"id": request["id"], "status": "ok", "rows": rows_for(request["filepath"])})
