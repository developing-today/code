#!/usr/bin/env python3
"""bench_poll_timing.py — the #13135 timing acceptance for poll.py,
runnable anywhere (CI runner included): 10 end-to-end poll.py runs
against fixture backends, p99 wall time must stay under the 30s
pre-#13112 gate budget.

The fixtures reproduce the production read shape the issue measured
(43–100s wall, ~2s CPU) at its low end, with fixed per-read delays:

  - session list: 28,000 sessions paginated at poll.py's 200/page,
    100ms per page                       -> ~14s sequential
  - active-id set: 1s
  - broker /list: 3s per read x 8 reads  -> ~24s sequential
  - ledger SELECT (stub turso-exec): 5s

Sequential total ~44s; a poll that overlaps the independent reads
lands near max(session list, broker) + ledger ~19s. Delays are
server-side sleeps in a threading fixture server, so the fixture
itself never serializes poll.py's reads.

Argument contract (#3352): bench_poll_timing.py [POLL_PY] — zero
arguments benches the sibling poll.py, one argument benches that
file; anything else is a usage error (exit 2) before any work runs.
Exit 0 iff all 10 runs exit 0 and p99 < 30s.
"""

import json
import os
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

RUNS = 10
P99_BUDGET_S = 30.0
TOTAL_SESSIONS = 28_000
PAGE_DELAY_S = 0.10
ACTIVE_DELAY_S = 1.0
BROKER_DELAY_S = 3.0
LEDGER_DELAY_S = 5.0

CANDIDATE_ROW = {
    "id": 42,
    "status": "claimed",
    "priority": 1,
    "progress": {"lane": "opencode", "stage": "handed_off"},
}


class FixtureHandler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def _json(self, payload):
        body = json.dumps(payload).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        parsed = urllib.parse.urlparse(self.path)
        if parsed.path == "/api/session":
            time.sleep(PAGE_DELAY_S)
            query = urllib.parse.parse_qs(parsed.query)
            limit = int(query["limit"][0])
            start = int(query.get("cursor", ["0"])[0])
            now_ms = int(time.time() * 1000)
            page = [
                {
                    "id": f"ses_{i}",
                    "time": {"created": now_ms - 600_000, "updated": now_ms - 60_000},
                }
                for i in range(start, min(start + limit, TOTAL_SESSIONS))
            ]
            nxt = start + limit if start + limit < TOTAL_SESSIONS else None
            self._json({"data": page, "cursor": {"next": str(nxt) if nxt else None}})
        elif parsed.path == "/api/session/active":
            time.sleep(ACTIVE_DELAY_S)
            self._json({"data": {}})
        else:
            self.send_response(404)
            self.end_headers()

    def do_POST(self):
        parsed = urllib.parse.urlparse(self.path)
        if parsed.path != "/list":
            self.send_response(404)
            self.end_headers()
            return
        time.sleep(BROKER_DELAY_S)
        length = int(self.headers.get("Content-Length") or 0)
        body = json.loads(self.rfile.read(length) or b"{}")
        rows = []
        if body.get("status") == "claimed" and body.get("queue") == (
            "repo-health-code-pilot"
        ):
            rows = [CANDIDATE_ROW]
        self._json({"ok": True, "rows": rows})


def main():
    if len(sys.argv) > 2:
        print(
            f"bench_poll_timing.py: unexpected argument {sys.argv[2]!r}; "
            "usage: python3 bench_poll_timing.py [POLL_PY]",
            file=sys.stderr,
        )
        return 2
    poll_py = (
        Path(sys.argv[1]).resolve()
        if len(sys.argv) == 2
        else Path(__file__).resolve().parent / "poll.py"
    )
    if not poll_py.is_file():
        print(f"bench_poll_timing.py: poll.py not found at {poll_py}", file=sys.stderr)
        return 2

    server = ThreadingHTTPServer(("127.0.0.1", 0), FixtureHandler)
    server.daemon_threads = True
    port = server.server_address[1]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix="poll-bench-") as tmp:
            home = Path(tmp)
            auth = home / ".local" / "state" / "opencode" / "service-prod.json"
            auth.parent.mkdir(parents=True)
            auth.write_text(json.dumps({"password": "bench"}))
            tools = home / "tools"
            tools.mkdir()
            (tools / "turso-exec.py").write_text(
                "import time\n"
                f"time.sleep({LEDGER_DELAY_S})\n"
                'print("[0] ok affected=0 rowid=None rows=[]")\n'
            )
            token = home / "token"
            token.write_text("bench-token\n")
            env = dict(
                os.environ,
                HOME=tmp,
                OPENCODE_URL=f"http://127.0.0.1:{port}",
                OPENCODE_AUTH_FILE=str(auth),
                REPO_QUEUE_BROKER_URL=f"http://127.0.0.1:{port}",
                REPO_QUEUE_TOKEN_FILE=str(token),
            )
            times = []
            for i in range(RUNS):
                start = time.monotonic()
                result = subprocess.run(
                    [sys.executable, str(poll_py)],
                    env=env,
                    capture_output=True,
                    text=True,
                    timeout=300,
                )
                elapsed = time.monotonic() - start
                times.append(elapsed)
                print(f"run {i + 1}/{RUNS}: {elapsed:.2f}s exit={result.returncode}")
                if result.returncode != 0:
                    print(result.stderr[-2000:], file=sys.stderr)
                    return 1
                out = json.loads(result.stdout)
                if len(out["candidates"]) != 1 or out["cap_room"] is None:
                    print(
                        "bench_poll_timing.py: poll output lost its fixture "
                        f"shape: candidates={len(out['candidates'])} "
                        f"cap_room={out['cap_room']}",
                        file=sys.stderr,
                    )
                    return 1
            ordered = sorted(times)
            # Nearest-rank p99 over RUNS samples.
            p99 = ordered[min(RUNS, max(1, -(-99 * RUNS // 100))) - 1]
            print(
                f"runs={RUNS} min={ordered[0]:.2f}s "
                f"median={ordered[RUNS // 2]:.2f}s p99={p99:.2f}s "
                f"budget={P99_BUDGET_S:.0f}s"
            )
            return 0 if p99 < P99_BUDGET_S else 1
    finally:
        server.shutdown()


if __name__ == "__main__":
    sys.exit(main())
