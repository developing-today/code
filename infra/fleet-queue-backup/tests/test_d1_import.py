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

Regression (#16070): a PUT accepted with its response lost is
recovered idempotently -- the retry init may omit upload_url when
the stored object verifies against the expected etag/length, and
import_sql must finalize it without a second PUT; a resume naming
a mismatched stored object, or no stored object at all, must raise
a clear retryable RuntimeError, never KeyError or false success.
Oracle: the same fake API; a resume is verified only by the fake's
reported etag/length matching the digest/length of the SQL text.

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
                return {
                    "success": True,
                    "result": {
                        "upload_url": "https://upload.example/put",
                        "filename": "import.sql",
                    },
                }
            if action == "ingest":
                self.assertEqual(
                    payload,
                    {"action": "ingest", "etag": self.digest, "filename": "import.sql"},
                )
                return {"success": True, "result": {"at_bookmark": "bm-1"}}
            if action == "poll":
                self.assertEqual(
                    payload, {"action": "poll", "current_bookmark": "bm-1"}
                )
                self.poll_count += 1
                if self.poll_count < 2:
                    return {"success": True, "result": {"status": "running"}}
                return {
                    "success": True,
                    "result": {"status": "complete", "result": {"num_queries": 2}},
                }
            raise AssertionError("unexpected action " + action)

        def fake_urlopen(req, timeout=None):
            self.assertEqual(req.full_url, "https://upload.example/put")
            return FakeUploadResponse(hashlib.md5(req.data).hexdigest())

        self._good_urlopen = fake_urlopen

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
                return {
                    "success": True,
                    "result": {
                        "upload_url": "https://upload.example/put",
                        "filename": "import.sql",
                    },
                }
            if action == "ingest":
                return {"success": True, "result": {"at_bookmark": "bm-1"}}
            return {
                "success": True,
                "result": {"status": "error", "error": "near line 1: no such table"},
            }

        with mock.patch.object(fq_common, "http_json", failing_http_json):
            client = fq_common.D1Client("token", "acct")
            with self.assertRaises(RuntimeError):
                client.import_sql("db-id", self.sql)

    def test_upload_failure_reinits_and_retries(self):
        attempts = {"n": 0}

        def flaky_urlopen(req, timeout=None):
            attempts["n"] += 1
            if attempts["n"] == 1:
                raise fq_common.urllib.error.URLError("write operation timed out")
            return self._good_urlopen(req, timeout=timeout)

        with mock.patch.object(fq_common.urllib.request, "urlopen", flaky_urlopen):
            client = fq_common.D1Client("token", "acct")
            result = client.import_sql("db-id", self.sql)
        self.assertEqual(result["status"], "complete")
        actions = [c[2]["action"] for c in self.calls]
        self.assertEqual(actions, ["init", "init", "ingest", "poll", "poll"])


    def _resume_http_json(self, resume_result, calls):
        """Fake API: first init offers an upload, later inits resume."""
        state = {"init_count": 0, "poll_count": 0}

        def fake(method, url, token, payload=None, timeout=None):
            calls.append((method, url, payload))
            action = payload["action"]
            if action == "init":
                state["init_count"] += 1
                if state["init_count"] == 1:
                    return {
                        "success": True,
                        "result": {
                            "upload_url": "https://upload.example/put",
                            "filename": "import.sql",
                        },
                    }
                return {"success": True, "result": resume_result}
            if action == "ingest":
                self.assertEqual(
                    payload,
                    {"action": "ingest", "etag": self.digest,
                     "filename": "import.sql"},
                )
                return {"success": True, "result": {"at_bookmark": "bm-1"}}
            if action == "poll":
                state["poll_count"] += 1
                return {"success": True,
                        "result": {"status": "complete",
                                   "result": {"num_queries": 2}}}
            raise AssertionError("unexpected action " + action)

        return fake

    def test_retry_after_accepted_put_lost_response_recovers(self):
        # Accepted PUT + dropped response + retry init without
        # upload_url, naming the stored object with matching etag.
        calls = []
        puts = {"n": 0}

        def lost_response_urlopen(req, timeout=None):
            puts["n"] += 1
            # Storage accepted these bytes; only the response was lost.
            self.assertEqual(
                hashlib.md5(req.data).hexdigest(), self.digest)
            raise fq_common.urllib.error.URLError("response lost")

        resume = {"filename": "import.sql", "etag": self.digest,
                  "size": len(self.sql.encode())}
        with mock.patch.object(
            fq_common, "http_json", self._resume_http_json(resume, calls)
        ), mock.patch.object(
            fq_common.urllib.request, "urlopen", lost_response_urlopen
        ):
            client = fq_common.D1Client("token", "acct")
            result = client.import_sql("db-id", self.sql)
        self.assertEqual(result["status"], "complete")
        # Exactly one PUT: recovery must not re-upload or duplicate.
        self.assertEqual(puts["n"], 1)
        actions = [c[2]["action"] for c in calls]
        self.assertEqual(actions, ["init", "init", "ingest", "poll"])

    def test_resume_with_mismatched_stored_object_is_retryable_error(self):
        calls = []

        def lost_response_urlopen(req, timeout=None):
            raise fq_common.urllib.error.URLError("response lost")

        resume = {"filename": "import.sql", "etag": "0" * 32}
        with mock.patch.object(
            fq_common, "http_json", self._resume_http_json(resume, calls)
        ), mock.patch.object(
            fq_common.urllib.request, "urlopen", lost_response_urlopen
        ):
            client = fq_common.D1Client("token", "acct")
            with self.assertRaises(RuntimeError) as ctx:
                client.import_sql("db-id", self.sql)
        self.assertIn("mismatch", str(ctx.exception))
        self.assertIn("retryable", str(ctx.exception))
        actions = [c[2]["action"] for c in calls]
        self.assertNotIn("ingest", actions)
        self.assertNotIn("poll", actions)

    def test_resume_without_verified_object_is_retryable_not_keyerror(self):
        calls = []

        def lost_response_urlopen(req, timeout=None):
            raise fq_common.urllib.error.URLError("response lost")

        with mock.patch.object(
            fq_common, "http_json", self._resume_http_json({}, calls)
        ), mock.patch.object(
            fq_common.urllib.request, "urlopen", lost_response_urlopen
        ):
            client = fq_common.D1Client("token", "acct")
            with self.assertRaises(RuntimeError) as ctx:
                client.import_sql("db-id", self.sql)
        self.assertNotIsInstance(ctx.exception, KeyError)
        self.assertIn("retryable", str(ctx.exception))
        actions = [c[2]["action"] for c in calls]
        self.assertNotIn("ingest", actions)


if __name__ == "__main__":
    unittest.main()
