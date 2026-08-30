"""Fixture worker: deliberately slow, so a crawl stays running long enough to
be cancelled or observed mid-flight."""
import json, sys, time

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
    time.sleep(0.02)
    send({"id": request["id"], "status": "ok",
          "rows": [{"filename": request["filepath"], "line": 1, "text": "row"}]})
