#!/usr/bin/env python3
"""The D1 import flow must follow the live four-step contract.

Oracle: a fake Cloudflare API. fq_common.http_json is replaced with a
recorder that answers init/ingest/poll per the documented contract
(developers.cloudflare.com, D1 database import method): init takes
{action, etag} and returns upload_url + filename; ingest takes
{action, etag, filename} and returns at_bookmark; poll takes
{action, current_bookmark} and completes when the result status is
"complete". The upload PUT is faked at urllib.request.urlopen and
answers with the md5 etag of the bytes it received.

An earlier revision of import_sql skipped the ingest step and polled
with the filename; the live API rejects that with
"current_bookmark => Required" (first live run, 2026-11-22, #8584).
These tests pin the step order and the poll payload so the drift is
caught hermetically next time.

Run: python3 -I infra/fleet-queue-backup/tests/test_d1_import.py
"""

import hashlib
import os
import sys
import unittest
from unittest import mock

BIN = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "bin"))
sys.path.insert(0, BIN)

import fq_common


class FakeUploadResponse:
    def __init__(self, etag):
        self.headers = {"etag": etag}

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False


class ImportFlowTest(unittest.TestCase):
    def setUp(self):
        self.calls = []
        self.poll_count = 0
        self.sql = "CREATE TABLE t (x);\nINSERT INTO t VALUES (1);\n"
        self.digest = hashlib.md5(self.sql.encode()).hexdigest()

        def fake_http_json(method, url, token, payload=None, timeout=None):
            self.calls.append((method, url, payload))
            action = payload["action"]
            if action == "init":
                self.assertEqual(payload, {"action": "init", "etag": self.digest})
                return {"success": True, "result": {
                    "upload_url": "https://upload.example/put",
                    "filename": "import.sql"}}
            if action == "ingest":
                self.assertEqual(payload, {
                    "action": "ingest", "etag": self.digest,
                    "filename": "import.sql"})
                return {"success": True, "result": {"at_bookmark": "bm-1"}}
            if action == "poll":
                self.assertEqual(payload, {
                    "action": "poll", "current_bookmark": "bm-1"})
                self.poll_count += 1
                if self.poll_count < 2:
                    return {"success": True, "result": {"status": "running"}}
                return {"success": True, "result": {
                    "status": "complete", "result": {"num_queries": 2}}}
            raise AssertionError("unexpected action " + action)

        def fake_urlopen(req, timeout=None):
            self.assertEqual(req.full_url, "https://upload.example/put")
            return FakeUploadResponse(hashlib.md5(req.data).hexdigest())

        self._patches = [
            mock.patch.object(fq_common, "http_json", fake_http_json),
            mock.patch.object(fq_common.urllib.request, "urlopen", fake_urlopen),
            mock.patch.object(fq_common.time, "sleep", lambda _s: None),
        ]
        for p in self._patches:
            p.start()

    def tearDown(self):
        for p in self._patches:
            p.stop()

    def test_import_follows_init_upload_ingest_poll(self):
        client = fq_common.D1Client("token", "acct")
        result = client.import_sql("db-id", self.sql)
        self.assertEqual(result["status"], "complete")
        actions = [c[2]["action"] for c in self.calls]
        self.assertEqual(actions, ["init", "ingest", "poll", "poll"])

    def test_import_error_aborts(self):
        def failing_http_json(method, url, token, payload=None, timeout=None):
            action = payload["action"]
            if action == "init":
                return {"success": True, "result": {
                    "upload_url": "https://upload.example/put",
                    "filename": "import.sql"}}
            if action == "ingest":
                return {"success": True, "result": {"at_bookmark": "bm-1"}}
            return {"success": True, "result": {
                "status": "error", "error": "near line 1: no such table"}}

        with mock.patch.object(fq_common, "http_json", failing_http_json):
            client = fq_common.D1Client("token", "acct")
            with self.assertRaises(RuntimeError):
                client.import_sql("db-id", self.sql)


if __name__ == "__main__":
    unittest.main()
