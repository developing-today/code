#!/usr/bin/env python3
"""Oracle: controlled HTTP server responses, not a live outage or service health."""
import http.server
import json
from pathlib import Path
import subprocess
import sys
import threading
import unittest

SCRIPT = Path(__file__).with_name("tier-drill-probe.py")


class Handler(http.server.BaseHTTPRequestHandler):
    requests = []

    def do_GET(self):
        self.requests.append(self.path)
        if self.path == "/broken":
            self.send_response(200)
            self.send_header("Transfer-Encoding", "chunked")
            self.end_headers()
            self.wfile.write(b"b\r\nshort")
            self.close_connection = True
            return
        if self.path == "/large":
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b"x" * 70000)
            return
        if self.path == "/redirect":
            self.send_response(302)
            self.send_header("Location", "/login")
            self.end_headers()
            return
        self.send_response(503 if self.path == "/unavailable" else 200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"ok":true}')

    def log_message(self, *args):
        pass


class ProbeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.base = "http://127.0.0.1:%s" % cls.server.server_port

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()
        cls.thread.join()

    def run_probe(self, *args):
        return subprocess.run([sys.executable, str(SCRIPT), *args], capture_output=True, text=True)

    def test_baseline_is_not_drill_evidence(self):
        result = self.run_probe("--base-url", self.base, "--path", "/health")
        self.assertEqual(result.returncode, 0, result.stderr)
        record = json.loads(result.stdout)
        self.assertEqual(record["kind"], "tier-probe-observations")
        self.assertFalse(record["provesDrill"])
        self.assertEqual(record["observations"][0]["status"], 200)
        self.assertEqual(record["observations"][0]["bodyBytes"], 11)
        self.assertEqual(len(record["observations"][0]["bodySha256"]), 64)

    def test_redirect_is_not_followed_or_counted_as_serving(self):
        result = self.run_probe("--base-url", self.base, "--path", "/redirect")
        self.assertEqual(result.returncode, 1)
        row = json.loads(result.stdout)["observations"][0]
        self.assertEqual(row["status"], 302)
        self.assertEqual(row["location"], "/login")
        self.assertFalse(row["httpSuccess"])

    def test_failure_does_not_discard_other_surfaces(self):
        result = self.run_probe("--base-url", self.base, "--path", "/unavailable", "--path", "/health")
        self.assertEqual(result.returncode, 1)
        self.assertEqual([r["status"] for r in json.loads(result.stdout)["observations"]], [503, 200])

    def test_transport_failure_is_recorded(self):
        result = self.run_probe("--base-url", "http://127.0.0.1:1", "--path", "/health", "--timeout", "0.1")
        self.assertEqual(result.returncode, 1)
        row = json.loads(result.stdout)["observations"][0]
        self.assertIsNone(row["status"])
        self.assertTrue(row["error"])

    def test_protocol_failure_preserves_other_observations(self):
        result = self.run_probe("--base-url", self.base, "--path", "/health", "--path", "/broken", "--path", "/health")
        self.assertEqual(result.returncode, 1, result.stderr)
        rows = json.loads(result.stdout)["observations"]
        self.assertEqual([r["httpSuccess"] for r in rows], [True, False, True])
        self.assertEqual(rows[1]["error"], "IncompleteRead")

    def test_large_body_is_explicitly_sampled(self):
        result = self.run_probe("--base-url", self.base, "--path", "/large")
        self.assertEqual(result.returncode, 0, result.stderr)
        row = json.loads(result.stdout)["observations"][0]
        self.assertEqual(row["bodyBytes"], 65536)
        self.assertTrue(row["bodyTruncated"])

    def test_strict_arguments_before_network(self):
        # Invalid options and path escapes must fail before any observation output.
        cases = [
            ["--typo"], ["extra"], ["--time", "1"], ["--timeout", "nan"],
            ["--timeout", "0"], ["--path", "//evil.example/"],
            ["--path", "https://evil.example/"], ["--path", "/ok#fragment"],
            ["--base-url", self.base + "/prefix"], ["--base-url", "file:///etc/passwd"],
        ]
        before = len(Handler.requests)
        for args in cases:
            with self.subTest(args=args):
                result = self.run_probe("--base-url", self.base, "--path", "/health", *args)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertEqual(result.stdout, "")
        self.assertEqual(len(Handler.requests), before)


if __name__ == "__main__":
    unittest.main()
