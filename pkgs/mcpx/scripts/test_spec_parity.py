#!/usr/bin/env python3
"""Synthetic regressions for spec-parity.py input validation and decisions."""

import datetime as dt
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest


SCRIPT = pathlib.Path(__file__).with_name("spec-parity.py")
NOW = "2026-11-06T00:00:00Z"
REVISIONS = '["2025-03-26"]'


def json_arg(value):
    return json.dumps(value, separators=(",", ":"))


class SpecParityTests(unittest.TestCase):
    def run_parity(self, *, mcpx=REVISIONS, suite=REVISIONS, now=NOW,
                   success="", failure="", allow_mcpx="", allow_suite=""):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            summary = root / "summary.md"
            out = root / "out"
            args = [
                sys.executable, str(SCRIPT), mcpx, suite, now, "run/1",
                success, failure, "false", str(summary), str(out),
                allow_mcpx, allow_suite,
            ]
            result = subprocess.run(args, capture_output=True, text=True, check=False)
            report = summary.read_text() if summary.exists() else ""
            artifacts = {
                path.name: path.read_text()
                for path in out.iterdir()
            } if out.exists() else {}
            return result, report, artifacts

    def assert_bad_input(self, field, **kwargs):
        result, report, artifacts = self.run_parity(**kwargs)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, "")
        self.assertNotIn("Traceback", report + result.stderr)
        self.assertIn(f"could not read {field}", report)
        self.assertNotIn("| revision |", report)
        self.assertNotIn("Spec parity: ⚠️ mismatch", report)
        self.assertEqual(artifacts.get("status"), "error")
        self.assertEqual(artifacts.get("sleep"), "0")
        self.assertNotIn("success.json", artifacts)
        self.assertNotIn("failure.json", artifacts)

    def test_revision_lists_must_be_arrays_of_strings(self):
        for field, argument in (("MCPX revisions", "mcpx"),
                                ("suite revisions", "suite")):
            for value in ("2025-03-26", {"revision": "2025-03-26"}, 7,
                          ["2025-03-26", ["2025-11-25"]],
                          ["2025-03-26", 7]):
                with self.subTest(field=field, value=value):
                    self.assert_bad_input(field, **{argument: json_arg(value)})

    def test_allow_lists_must_be_arrays_of_strings(self):
        for field, argument in (("MCPX allow list", "allow_mcpx"),
                                ("suite allow list", "allow_suite")):
            for value in ("2025-03-26", {"revision": "2025-03-26"}, 7,
                          ["2025-03-26", 7]):
                with self.subTest(field=field, value=value):
                    self.assert_bad_input(field, **{argument: json_arg(value)})

    def test_success_and_failure_records_must_be_objects_or_null(self):
        for field, argument in (("success record", "success"),
                                ("failure record", "failure")):
            for value in ([{"timestamp": NOW}], "record"):
                with self.subTest(field=field, value=value):
                    self.assert_bad_input(field, **{argument: json_arg(value)})

    def test_invalid_json_is_a_named_graceful_error(self):
        for field, argument in (("MCPX revisions", "mcpx"),
                                ("suite revisions", "suite"),
                                ("MCPX allow list", "allow_mcpx"),
                                ("suite allow list", "allow_suite"),
                                ("success record", "success"),
                                ("failure record", "failure")):
            with self.subTest(field=field):
                self.assert_bad_input(field, **{argument: "{oops"})

    def test_failure_count_must_be_nonnegative_integer_not_bool(self):
        for count in ("1", True, -1, 1.0):
            with self.subTest(count=count):
                self.assert_bad_input(
                    "failure.count", failure=json_arg({"count": count}))

    def test_present_timestamps_must_be_parseable(self):
        cases = (
            ("success.timestamp", "success", {"timestamp": 123}),
            ("success.timestamp", "success", {"timestamp": "not-a-time"}),
            ("failure.first_timestamp", "failure", {"first_timestamp": 123}),
            ("failure.latest_timestamp", "failure", {"latest_timestamp": "bad"}),
        )
        for field, argument, value in cases:
            with self.subTest(field=field, value=value):
                self.assert_bad_input(field, **{argument: json_arg(value)})

    def test_specs_and_run_fields_must_have_expected_types(self):
        cases = (
            ("success.specs", "success", {"specs": "2025-03-26"}),
            ("success.specs", "success", {"specs": ["2025-03-26", 7]}),
            ("success.run", "success", {"run": 7}),
            ("failure.first_run", "failure", {"first_run": 7}),
            ("failure.latest_run", "failure", {"latest_run": None}),
        )
        for field, argument, value in cases:
            with self.subTest(field=field, value=value):
                self.assert_bad_input(field, **{argument: json_arg(value)})

    def test_active_failure_record_requires_first_timestamp(self):
        self.assert_bad_input("failure.first_timestamp", failure=json_arg({"count": 1}))

    def test_empty_revision_list_keeps_the_existing_error_behavior(self):
        result, report, artifacts = self.run_parity(mcpx="[]")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("could not read the lists", report)
        self.assertEqual(artifacts["status"], "error")
        self.assertEqual(artifacts["sleep"], "0")
        self.assertNotIn("success.json", artifacts)
        self.assertNotIn("failure.json", artifacts)

    def test_malformed_record_is_named_even_when_revision_list_is_empty(self):
        for field, argument in (("success record", "success"),
                                ("failure record", "failure")):
            with self.subTest(field=field):
                self.assert_bad_input(field, mcpx="[]", **{argument: json_arg("record")})

    def test_valid_match_writes_success_and_zero_sleep(self):
        result, report, artifacts = self.run_parity(success="null", failure="null")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Spec parity: ✅ match", report)
        self.assertEqual(artifacts["status"], "ok")
        self.assertEqual(artifacts["sleep"], "0")
        self.assertEqual(json.loads(artifacts["success.json"]), {
            "timestamp": NOW, "run": "run/1", "specs": ["2025-03-26"]})
        self.assertNotIn("failure.json", artifacts)

    def test_valid_mismatch_warns_and_starts_failure_record(self):
        result, report, artifacts = self.run_parity(suite='["2025-03-26","2025-11-25"]')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Spec parity: ⚠️ mismatch", report)
        self.assertEqual(artifacts["status"], "warn")
        self.assertEqual(artifacts["sleep"], "0")
        failure = json.loads(artifacts["failure.json"])
        self.assertEqual(failure["count"], 1)
        self.assertEqual(failure["first_timestamp"], NOW)
        self.assertEqual(failure["latest_timestamp"], NOW)
        self.assertNotIn("success.json", artifacts)

    def test_dead_allowance_remains_an_error(self):
        result, report, artifacts = self.run_parity(allow_mcpx=json_arg(["2025-03-26"]))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("allowance is dead code", report)
        self.assertEqual(artifacts["status"], "error")
        self.assertEqual(artifacts["sleep"], "0")

    def test_one_and_two_sided_deadlines_and_sleep_are_unchanged(self):
        for both_sided, since, expected_status, expected_sleep in (
            (False, 840, "warn", 840), (False, 841, "error", 841),
            (True, 360, "warn", 360), (True, 361, "error", 361),
            (False, 1000, "error", 900),
        ):
            with self.subTest(both_sided=both_sided, since=since):
                now = dt.datetime.strptime(NOW, "%Y-%m-%dT%H:%M:%SZ")
                success_time = (now - dt.timedelta(hours=since)).strftime("%Y-%m-%dT%H:%M:%SZ")
                if both_sided:
                    mcpx = json_arg(["2025-03-26", "2025-11-25"])
                    suite = json_arg(["2025-03-26", "2026-01-10"])
                else:
                    mcpx = REVISIONS
                    suite = json_arg(["2025-03-26", "2025-11-25"])
                result, report, artifacts = self.run_parity(
                    mcpx=mcpx, suite=suite,
                    success=json_arg({"timestamp": success_time}))
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("Spec parity:", report)
                self.assertEqual(artifacts["status"], expected_status)
                self.assertEqual(artifacts["sleep"], str(expected_sleep))


if __name__ == "__main__":
    unittest.main()
