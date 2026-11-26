#!/usr/bin/env python3
"""Hermetic tests for quota-recorder's Turso pipeline acknowledgement contract."""

import importlib.util
import json
import pathlib
import subprocess
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = pathlib.Path(__file__).with_name("quota-recorder.py")
SPEC = importlib.util.spec_from_file_location("quota_recorder", SCRIPT)
quota_recorder = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(quota_recorder)


RECORD = {"ts": "2026-11-26T00:00:00Z", "source": "muse-subscription"}
NOW = "2026-11-26T01:00:00Z"
SUCCESS_RESPONSE = {
    "baton": None,
    "base_url": None,
    "results": [
        {
            "type": "ok",
            "response": {"type": "execute", "result": {"affected_row_count": 1}},
        },
        {"type": "ok", "response": {"type": "close"}},
    ],
}


class QuotaRecorderPipelineTests(unittest.TestCase):
    def run_flush_response(self, response, *, curl_status=0, curl_stderr=""):
        curl_stdout = response if isinstance(response, str) else json.dumps(response)
        with patch.object(
            quota_recorder.subprocess,
            "run",
            side_effect=[
                subprocess.CompletedProcess(["fleet", "secrets"], 0, "test-token\n", ""),
                subprocess.CompletedProcess(
                    ["curl"], curl_status, curl_stdout, curl_stderr
                ),
            ],
        ):
            return quota_recorder.run_flush([RECORD], NOW)

    def test_complete_success_response_confirms_execute_and_close(self):
        ok, error = self.run_flush_response(SUCCESS_RESPONSE)

        self.assertTrue(ok)
        self.assertEqual(error, "")

    def test_missing_or_incomplete_pipeline_results_are_rejected(self):
        incomplete = [
            {},
            {"results": []},
            {"error": "unauthorized"},
            {"results": SUCCESS_RESPONSE["results"][:1]},
            {"results": SUCCESS_RESPONSE["results"] + [SUCCESS_RESPONSE["results"][1]]},
            {"results": [SUCCESS_RESPONSE["results"][0], {"type": "ok"}]},
            {"results": [SUCCESS_RESPONSE["results"][0], {"type": "unexpected"}]},
            {"results": [SUCCESS_RESPONSE["results"][0], {"type": "ok", "response": {"type": "execute"}}]},
            {"results": [None, SUCCESS_RESPONSE["results"][1]]},
        ]
        for response in incomplete:
            with self.subTest(response=response):
                ok, error = self.run_flush_response(response)
                self.assertFalse(ok)
                self.assertTrue(error)

    def test_malformed_json_and_http_failure_are_rejected(self):
        json_ok, json_error = self.run_flush_response("not JSON")
        self.assertFalse(json_ok)
        self.assertIn("unparseable", json_error)

        http_ok, http_error = self.run_flush_response(
            {"error": "unauthorized"}, curl_status=22, curl_stderr="HTTP 401"
        )
        self.assertFalse(http_ok)
        self.assertIn("curl exit 22", http_error)

    def test_error_in_either_pipeline_result_is_rejected(self):
        responses = [
            {
                "results": [
                    {"type": "error", "error": {"message": "insert failed"}},
                    SUCCESS_RESPONSE["results"][1],
                ]
            },
            {
                "results": [
                    SUCCESS_RESPONSE["results"][0],
                    {"type": "error", "error": {"message": "close failed"}},
                ]
            },
        ]
        for response in responses:
            with self.subTest(response=response):
                ok, error = self.run_flush_response(response)
                self.assertFalse(ok)
                self.assertIn("turso:", error)

    def test_rejected_response_stays_pending_until_a_complete_success(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "records.jsonl"
            path.write_text(json.dumps(RECORD) + "\n")
            with patch.object(
                quota_recorder.subprocess,
                "run",
                side_effect=[
                    subprocess.CompletedProcess(["fleet", "secrets"], 0, "token\n", ""),
                    subprocess.CompletedProcess(["curl"], 0, "{}", ""),
                ],
            ):
                pending = quota_recorder.flush_file(str(path), NOW)

            self.assertEqual(pending, 1)
            self.assertIsNone(quota_recorder.load_file_state(path)[1])
            self.assertEqual(
                json.loads(path.read_text().splitlines()[-1])["kind"], "turso-error"
            )

            with patch.object(
                quota_recorder.subprocess,
                "run",
                side_effect=[
                    subprocess.CompletedProcess(["fleet", "secrets"], 0, "token\n", ""),
                    subprocess.CompletedProcess(
                        ["curl"], 0, json.dumps(SUCCESS_RESPONSE), ""
                    ),
                ],
            ):
                pending = quota_recorder.flush_file(str(path), NOW)

            self.assertEqual(pending, 0)
            records, acked_through = quota_recorder.load_file_state(path)
            self.assertEqual(records, [RECORD])
            self.assertEqual(acked_through, RECORD["ts"])
            self.assertEqual(
                json.loads(path.read_text().splitlines()[-1])["kind"], "turso-ack"
            )


if __name__ == "__main__":
    unittest.main()
