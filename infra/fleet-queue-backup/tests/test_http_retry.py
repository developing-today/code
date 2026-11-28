#!/usr/bin/env python3
"""http_json transport retry (#8584): a connection reset must not be
fatal to a multi-hour export or restore; an HTTP error must not be
retried (the API answered; it does not heal).

Run: python3 -I infra/fleet-queue-backup/tests/test_http_retry.py
"""

import io
import json
import os
import sys
import unittest
import urllib.error
import urllib.request

BIN = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "bin"))
sys.path.insert(0, BIN)

import fq_common


class FakeResponse:
    def __init__(self, body):
        self._body = body

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False

    def read(self):
        return self._body


class TestHttpRetry(unittest.TestCase):
    def setUp(self):
        self.calls = 0
        self._real_urlopen = urllib.request.urlopen
        self._real_sleep = fq_common.time.sleep
        fq_common.time.sleep = lambda _s: None

    def tearDown(self):
        urllib.request.urlopen = self._real_urlopen
        fq_common.time.sleep = self._real_sleep

    def test_transport_failures_are_retried(self):
        def flaky(req, timeout=None):
            self.calls += 1
            if self.calls < 3:
                raise urllib.error.URLError("connection reset by peer")
            return FakeResponse(json.dumps({"ok": True}).encode())

        urllib.request.urlopen = flaky
        out = fq_common.http_json("GET", "https://example.invalid/x", "tok")
        self.assertEqual(out, {"ok": True})
        self.assertEqual(self.calls, 3)

    def test_http_errors_are_not_retried(self):
        def broken(req, timeout=None):
            self.calls += 1
            raise urllib.error.HTTPError(
                req.full_url, 400, "bad", {}, io.BytesIO(b'{"e":1}')
            )

        urllib.request.urlopen = broken
        with self.assertRaises(RuntimeError):
            fq_common.http_json("GET", "https://example.invalid/x", "tok")
        self.assertEqual(self.calls, 1)

    def test_transport_failure_eventually_raises(self):
        def dead(req, timeout=None):
            self.calls += 1
            raise urllib.error.URLError("unreachable")

        urllib.request.urlopen = dead
        with self.assertRaises(RuntimeError):
            fq_common.http_json("GET", "https://example.invalid/x", "tok")
        self.assertEqual(self.calls, fq_common.TRANSPORT_ATTEMPTS)


if __name__ == "__main__":
    unittest.main()
