#!/usr/bin/env python3
"""#11980 transport oracle: real HTTP over a disposable Unix socket.
Run separately from run_hermetic_tests.py (whose no-socket guard is unchanged).
No live credentials, queue operations, subprocesses, or TCP connections.
"""
import contextlib
import http.server
import importlib.util
import io
import json
import os
from pathlib import Path
import socket
import socketserver
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch, Mock
import urllib.error

sys.dont_write_bytecode = True
_spec = importlib.util.spec_from_file_location("poll_transport", Path(__file__).with_name("poll.py"))
poll = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(poll)


class TransportTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="poll-uds-")
        self.addCleanup(self.tmp.cleanup)
        self.base = Path(self.tmp.name)
        self.path = self.base / "broker.sock"
        self.token = self.base / "token"
        self.token.write_text("fixture-only-token\n")
        self.env = patch.dict(os.environ, {
            "HOME": self.tmp.name,
            "REPO_QUEUE_BROKER_URL": "unix://" + str(self.path),
            "REPO_QUEUE_TOKEN_FILE": str(self.token),
        }, clear=True)
        self.env.start()
        self.addCleanup(self.env.stop)
        poll._READ_ERRORS.clear()
        # Hard-bound every real connect to this test's disposable socket.
        real_connect = socket.socket.connect
        allowed = str(self.path)
        def fixture_connect(sock, address):
            if sock.family != socket.AF_UNIX or address != allowed:
                raise AssertionError("test attempted a non-fixture connection")
            return real_connect(sock, address)
        connect_guard = patch.object(socket.socket, "connect", fixture_connect)
        connect_guard.start()
        self.addCleanup(connect_guard.stop)

    @contextlib.contextmanager
    def broker(self, body=b'{"rows":[]}', status=200, chunked=False):
        seen = []
        class Handler(http.server.BaseHTTPRequestHandler):
            def do_POST(self):
                if self.path != "/list":
                    raise AssertionError("fixture only serves the read-only /list route")
                seen.append((self.path, dict(self.headers), self.rfile.read(int(self.headers["Content-Length"]))))
                self.send_response(status)
                if chunked:
                    self.send_header("Transfer-Encoding", "chunked")
                else:
                    self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                if chunked:
                    self.wfile.write(("%x\r\n" % len(body)).encode() + body + b"\r\n0\r\n\r\n")
                else:
                    self.wfile.write(body)
            def log_message(self, *args):
                pass
        with socketserver.UnixStreamServer(str(self.path), Handler) as server:
            thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": .01}, daemon=True)
            thread.start()
            try:
                yield seen
            finally:
                server.shutdown()
                thread.join(timeout=2)
                self.assertFalse(thread.is_alive())

    def test_unix_http_auth_body_and_exact_read_only_path(self):
        body = {"queue": "fixture", "status": "claimed", "rich": True, "limit": 500}
        with self.broker() as seen:
            self.assertEqual(poll._broker_post("/list", body), {"rows": []})
        self.assertEqual(len(seen), 1)
        path, headers, received = seen[0]
        self.assertEqual(path, "/list")
        self.assertEqual(headers["X-Agent-Token"], "fixture-only-token")
        self.assertEqual(headers["Content-Type"], "application/json")
        self.assertEqual(json.loads(received), body)

    def test_real_unix_read_discovers_and_normalizes_candidate_rows(self):
        # Oracle: recorded handoff checkpoint becomes normalized stage, while
        # unrelated statuses cannot enter claimed discovery. Pure classifier
        # only: no claimant main, claims, ledger calls, or launches.
        raw = {
            "id": 42, "queue": "fixture", "status": "claimed",
            "worker_id": "fixture-worker", "lane_id": "fixture-lane",
            "progress": {"lane": "opencode",
                         "last_checkpoint": {"stage": "handed_off"}},
        }
        payload = {"ok": True, "rows": [raw, {"id": 43, "status": "done"}]}
        with self.broker(body=json.dumps(payload).encode()) as seen:
            rows = poll.fetch_claimed_rows("fixture")
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["id"], 42)
        self.assertEqual(rows[0]["worker_id"], "fixture-worker")
        self.assertEqual(rows[0]["progress"]["stage"], "handed_off")
        self.assertTrue(poll.is_candidate(rows[0]))
        self.assertEqual(poll.queue_read_errors("fixture"), {})
        self.assertEqual(len(seen), 1)
        self.assertEqual(seen[0][0], "/list")
        self.assertEqual(json.loads(seen[0][2]), {
            "queue": "fixture", "status": "claimed",
            "rich": True, "limit": poll.LIST_LIMIT,
        })

    def test_chunked_http_response(self):
        with self.broker(chunked=True):
            self.assertEqual(poll._broker_post("/list", {}), {"rows": []})

    def test_non_success_and_redirect_fail_closed(self):
        for status in (301, 401, 500):
            with self.subTest(status=status):
                with self.broker(status=status):
                    with self.assertRaises(urllib.error.HTTPError) as err:
                        poll._broker_post("/list", {})
                    self.assertEqual(err.exception.code, status)
                self.path.unlink()

    def test_invalid_json_and_application_error_fail_closed(self):
        for body in (b"not json", b'{"ok":false,"error":"fixture failure"}'):
            with self.subTest(body=body):
                with self.broker(body=body), contextlib.redirect_stderr(io.StringIO()):
                    self.assertEqual(poll.fetch_claimed_rows("fixture"), [])
                    self.assertTrue(poll.queue_read_errors("fixture"))
                self.path.unlink()

    def test_unreachable_socket_is_not_empty_success_or_tcp_fallback(self):
        with patch.object(poll.urllib.request, "urlopen", side_effect=AssertionError("TCP fallback forbidden")), contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(poll.fetch_claimed_rows("fixture"), [])
            self.assertTrue(poll.queue_read_errors("fixture"))

    def test_config_file_resolution(self):
        config = self.base / "broker-url"
        config.write_text("unix://" + str(self.path) + "\n")
        with patch.dict(os.environ, {"REPO_QUEUE_BROKER_URL": ""}), patch.object(poll, "BROKER_URL_FILE", config), self.broker():
            self.assertEqual(poll._broker_post("/list", {}), {"rows": []})

    def test_tcp_urllib_request_and_timeout_unchanged(self):
        response = io.BytesIO(b'{"rows":[]}')
        with patch.dict(os.environ, {"REPO_QUEUE_BROKER_URL": "http://fixture.invalid:9911/"}), patch.object(poll.urllib.request, "urlopen", return_value=response) as opened:
            self.assertEqual(poll._broker_post("/list", {"rich": True}), {"rows": []})
        req = opened.call_args.args[0]
        self.assertEqual(req.full_url, "http://fixture.invalid:9911/list")
        self.assertEqual(req.method, "POST")
        self.assertEqual(req.get_header("X-agent-token"), "fixture-only-token")
        self.assertEqual(opened.call_args.kwargs, {"timeout": 30})

    def test_unix_timeout_is_30_seconds_and_failed_connect_closes_socket(self):
        sock = Mock()
        sock.connect.side_effect = TimeoutError("fixture timeout")
        with patch.object(poll.socket, "socket", return_value=sock) as created:
            with self.assertRaises(TimeoutError):
                poll._broker_post("/list", {})
        created.assert_called_once_with(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.settimeout.assert_called_once_with(30)
        sock.connect.assert_called_once_with(str(self.path))
        sock.close.assert_called_once()

    def test_invalid_unix_paths_fail_before_connect(self):
        for url in ("unix://", "unix://relative/path"):
            with self.subTest(url=url), patch.dict(os.environ, {"REPO_QUEUE_BROKER_URL": url}), patch.object(socket.socket, "connect", side_effect=AssertionError("invalid path connected")):
                with self.assertRaises(ValueError):
                    poll._broker_post("/list", {})


if __name__ == "__main__":
    unittest.main()
