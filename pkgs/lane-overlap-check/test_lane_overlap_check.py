"""Tests for lane_overlap_check against a fake OpenChamber server.

Run: python3 pkgs/lane-overlap-check/test_lane_overlap_check.py
"""

import contextlib
import http.server
import io
import json
import os
import stat
import sys
import tempfile
import threading
import time
import unittest
import urllib.parse
from contextlib import redirect_stdout

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lane_overlap_check as loc  # noqa: E402

NOW = int(time.time() * 1000)
MIN = 60_000


def session(sid, title, created_min_ago, updated_min_ago=None):
    updated = updated_min_ago if updated_min_ago is not None else created_min_ago
    return {
        "id": sid,
        "title": title,
        "location": {"directory": "/home/user/git/code"},
        "time": {
            "created": NOW - created_min_ago * MIN,
            "updated": NOW - updated * MIN,
        },
    }


class FakeServer:
    def __init__(self, sessions, page_size=2):
        # Newest-updated first, like the real server.
        self.sessions = sorted(sessions, key=lambda s: -s["time"]["updated"])
        self.page_size = page_size
        self.requests = 0
        outer = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):  # noqa: N802
                outer.requests += 1
                q = urllib.parse.parse_qs(urllib.parse.urlparse(self.path).query)
                search = q.get("search", [""])[0]
                start = int(q.get("cursor", ["0"])[0])
                hits = [s for s in outer.sessions if search in s["title"]]
                page = hits[start : start + outer.page_size]
                nxt = start + outer.page_size
                body = {
                    "data": page,
                    "cursor": {"next": str(nxt) if nxt < len(hits) else None},
                }
                data = json.dumps(body).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(data)

            def log_message(self, *a):
                pass

        self.httpd = http.server.HTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()
        self.url = "http://127.0.0.1:%d" % self.httpd.server_port

    def close(self):
        self.httpd.shutdown()


def fake_openchamber(dirpath, statuses):
    script = os.path.join(dirpath, "openchamber")
    with open(script, "w") as f:
        f.write("#!%s\nimport json, sys\nS = %r\n" % (sys.executable, statuses))
        f.write("sid = sys.argv[sys.argv.index('--session') + 1]\n")
        f.write("print(json.dumps({'sessionStatus': {'type': S.get(sid, 'idle')}}))\n")
    os.chmod(script, os.stat(script).st_mode | stat.S_IEXEC)


class OverlapTest(unittest.TestCase):
    def run_check(self, sessions, statuses, *extra):
        server = FakeServer(sessions)
        self.addCleanup(server.close)
        tmp = tempfile.mkdtemp()
        fake_openchamber(tmp, statuses)
        old = os.environ["PATH"]
        os.environ["PATH"] = tmp + os.pathsep + old
        self.addCleanup(os.environ.__setitem__, "PATH", old)
        out = io.StringIO()
        with redirect_stdout(out):
            rc = loc.main(
                ["code-f", "--server", server.url, "--page-size", "2", *extra]
            )
        return rc, json.loads(out.getvalue()), server

    def test_clear_when_earlier_occurrences_are_idle(self):
        rc, res, _ = self.run_check(
            [
                session("s3", "Queue lane (scheduled) - code-f 22:40", 1),
                session("s2", "Queue lane (scheduled) - code-f 22:00", 40),
                session("s1", "Queue lane (scheduled) - code-f 21:00", 100),
            ],
            {},
        )
        self.assertEqual((rc, res["verdict"]), (0, "clear"))
        self.assertEqual(res["self"]["id"], "s3")
        self.assertEqual([e["id"] for e in res["earlier"]], ["s1", "s2"])

    def test_overlap_when_an_earlier_occurrence_is_busy(self):
        rc, res, _ = self.run_check(
            [
                session("s3", "Queue lane (scheduled) - code-f 22:40", 1),
                session("s2", "Queue lane (scheduled) - code-f 22:00", 40, 0),
            ],
            {"s2": "busy"},
        )
        self.assertEqual((rc, res["verdict"]), (3, "overlap"))

    def test_other_lanes_with_the_same_stem_are_ignored(self):
        rc, res, _ = self.run_check(
            [
                session("s3", "Queue lane (scheduled) - code-f 22:40", 1),
                session("x", "Queue lane (scheduled) - code-f2 22:30", 10, 0),
            ],
            {"x": "busy"},
        )
        self.assertEqual((rc, res["verdict"]), (0, "clear"))
        self.assertEqual(res["earlier"], [])

    def test_paging_stops_once_past_the_window(self):
        old = [
            session("o%d" % i, "Queue lane (scheduled) - code-f old%d" % i, 300 + i)
            for i in range(20)
        ]
        rc, res, server = self.run_check(
            [session("s3", "Queue lane (scheduled) - code-f 22:40", 1)] + old, {}
        )
        self.assertEqual(rc, 0)
        self.assertLessEqual(server.requests, 2)

    def test_unknown_when_the_server_is_unreachable(self):
        out = io.StringIO()
        with redirect_stdout(out):
            rc = loc.main(
                ["code-f", "--server", "http://127.0.0.1:9", "--http-timeout", "1"]
            )
        self.assertEqual((rc, json.loads(out.getvalue())["verdict"]), (4, "unknown"))

    def test_self_id_picks_this_occurrence(self):
        rc, res, _ = self.run_check(
            [
                session("s3", "Queue lane (scheduled) - code-f 22:40", 1, 0),
                session("s2", "Queue lane (scheduled) - code-f 22:00", 40),
            ],
            {"s3": "busy"},
            "--self",
            "s2",
        )
        # s3 is newer than this occurrence, so it is not an earlier one.
        self.assertEqual((rc, res["verdict"]), (0, "clear"))

    def test_abbreviated_options_are_usage_errors(self):
        for flag in ("--serv", "--window"):
            with (
                self.assertRaises(SystemExit) as cm,
                redirect_stdout(io.StringIO()),
                contextlib.redirect_stderr(io.StringIO()),
            ):
                loc.main(["code-f", flag, "x"])
            self.assertEqual(cm.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
