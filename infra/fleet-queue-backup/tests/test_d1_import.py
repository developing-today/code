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
caught hermetically next time. Resume oracle: Cloudflare's official
workers-sdk/packages/wrangler/src/d1/execute.ts and types.ts (checked
2026-11-27). An init without upload_url is already validated server-side
using the supplied MD5 etag and ingestion has started. It returns the
first import response (complete, active with at_bookmark, or failure),
not speculative stored-object length/hash metadata or an ingest filename.

Regression (#16070): after an accepted PUT loses its response, retry
init may return a complete or active import without upload_url. Never
PUT or ingest again in this branch; reject failures, unknown states,
and active responses without a bookmark with a retryable RuntimeError.

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


    def _lost_response_resume(self, resume):
        """Oracle: Wrangler's init shortcut is already server-validated/ingested."""
        calls = []
        put_bytes = []

        def api(method, url, token, payload=None, timeout=None):
            calls.append(payload)
            action = payload["action"]
            if action == "init":
                self.assertEqual(payload["etag"], self.digest)
                if len(calls) == 1:
                    return {"success": True, "result": {
                        "upload_url": "https://upload.example/put",
                        "filename": "import.sql",
                    }}
                return {"success": True, "result": resume}
            self.assertEqual(action, "poll", "resume must not ingest again")
            self.assertEqual(payload["current_bookmark"], "bm-resume")
            return {"success": True, "result": {"status": "complete"}}

        def accepted_put(req, timeout=None):
            put_bytes.append(req.data)
            raise fq_common.urllib.error.URLError("accepted PUT response lost")

        with mock.patch.object(fq_common, "http_json", api), mock.patch.object(
            fq_common.urllib.request, "urlopen", accepted_put
        ):
            try:
                return fq_common.D1Client("token", "acct").import_sql("db-id", self.sql)
            finally:
                self.assertEqual(put_bytes, [self.sql.encode()])
                self.assertEqual([c["action"] for c in calls[:2]], ["init", "init"])

    def test_lost_put_response_resume_complete_returns_without_ingest_or_poll(self):
        complete = {"status": "complete", "result": {"num_queries": 2}}
        self.assertEqual(self._lost_response_resume(complete), complete)

    def test_lost_put_response_resume_pending_polls_without_ingest(self):
        result = self._lost_response_resume({
            "status": "active", "at_bookmark": "bm-resume",
        })
        self.assertEqual(result["status"], "complete")

    def test_resume_errors_and_unrecognized_results_are_retryable(self):
        # Error beats status/bookmark; unknown states and absent bookmarks
        # cannot prove completion. No hypothetical hash/size metadata needed.
        for resume in (
            {"status": "error", "error": "stored object etag mismatch"},
            {"status": "error", "at_bookmark": "bm-resume"},
            {"status": "complete", "error": "validation failed"},
            {"status": "complete", "success": False},
            {"status": "complete", "errors": ["stored object length mismatch"]},
            {"status": "active"},
            {"status": "mystery", "at_bookmark": "bm-resume"},
            {"filename": "import.sql"},
            {}, None, [],
        ):
            with self.subTest(resume=resume):
                with self.assertRaisesRegex(RuntimeError, "retryable"):
                    self._lost_response_resume(resume)

    def test_resume_poll_error_cannot_report_success(self):
        for result in (
            {"status": "error", "errors": ["import failed"]},
            {"status": "complete", "success": False},
            {"status": "mystery"}, None,
        ):
            with self.subTest(result=result), mock.patch.object(
                fq_common.D1Client, "_call", side_effect=[
                    {"status": "active", "at_bookmark": "bm-resume"}, result,
                ],
            ), self.assertRaisesRegex(RuntimeError, "retryable"):
                fq_common.D1Client("token", "acct").import_sql("db-id", self.sql)

    def test_normal_ingest_can_complete_immediately(self):
        complete = {"status": "complete"}
        with mock.patch.object(fq_common.D1Client, "_call", side_effect=[
            {"upload_url": "https://upload.example/put", "filename": "import.sql"},
            complete,
        ]) as api:
            self.assertEqual(
                fq_common.D1Client("token", "acct").import_sql("db-id", self.sql),
                complete,
            )
        self.assertEqual(api.call_count, 2)

    def test_upload_etag_mismatch_retries_then_rejects(self):
        with mock.patch.object(
            fq_common.urllib.request, "urlopen",
            return_value=FakeUploadResponse("wrong-etag"),
        ) as put:
            with self.assertRaisesRegex(RuntimeError, "etag mismatch"):
                fq_common.D1Client("token", "acct").import_sql("db-id", self.sql)
        self.assertEqual(put.call_count, 3)
        self.assertEqual([c[2]["action"] for c in self.calls], ["init"] * 3)


if __name__ == "__main__":
    unittest.main()
