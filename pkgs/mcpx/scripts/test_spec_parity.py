#!/usr/bin/env python3
"""Synthetic regressions for spec-parity.py input validation and decisions."""

import datetime as dt
import json
import pathlib
import random
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
    def run_parity(
        self,
        *,
        mcpx=REVISIONS,
        suite=REVISIONS,
        now=NOW,
        success="",
        failure="",
        allow_mcpx="",
        allow_suite="",
    ):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            summary = root / "summary.md"
            out = root / "out"
            args = [
                sys.executable,
                str(SCRIPT),
                mcpx,
                suite,
                now,
                "run/1",
                success,
                failure,
                "false",
                str(summary),
                str(out),
                allow_mcpx,
                allow_suite,
            ]
            result = subprocess.run(args, capture_output=True, text=True, check=False)
            report = summary.read_text() if summary.exists() else ""
            artifacts = (
                {path.name: path.read_text() for path in out.iterdir()}
                if out.exists()
                else {}
            )
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
        for field, argument in (
            ("MCPX revisions", "mcpx"),
            ("suite revisions", "suite"),
        ):
            for value in (
                "2025-03-26",
                {"revision": "2025-03-26"},
                7,
                ["2025-03-26", ["2025-11-25"]],
                ["2025-03-26", 7],
            ):
                with self.subTest(field=field, value=value):
                    self.assert_bad_input(field, **{argument: json_arg(value)})

    def test_allow_lists_must_be_arrays_of_strings(self):
        for field, argument in (
            ("MCPX allow list", "allow_mcpx"),
            ("suite allow list", "allow_suite"),
        ):
            for value in (
                "2025-03-26",
                {"revision": "2025-03-26"},
                7,
                ["2025-03-26", 7],
            ):
                with self.subTest(field=field, value=value):
                    self.assert_bad_input(field, **{argument: json_arg(value)})

    def test_success_and_failure_records_must_be_objects_or_null(self):
        for field, argument in (
            ("success record", "success"),
            ("failure record", "failure"),
        ):
            for value in ([{"timestamp": NOW}], "record"):
                with self.subTest(field=field, value=value):
                    self.assert_bad_input(field, **{argument: json_arg(value)})

    def test_invalid_json_is_a_named_graceful_error(self):
        for field, argument in (
            ("MCPX revisions", "mcpx"),
            ("suite revisions", "suite"),
            ("MCPX allow list", "allow_mcpx"),
            ("suite allow list", "allow_suite"),
            ("success record", "success"),
            ("failure record", "failure"),
        ):
            with self.subTest(field=field):
                self.assert_bad_input(field, **{argument: "{oops"})

    def test_failure_count_must_be_nonnegative_integer_not_bool(self):
        for count in ("1", True, -1, 1.0):
            with self.subTest(count=count):
                self.assert_bad_input(
                    "failure.count", failure=json_arg({"count": count})
                )

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
        for field, argument in (
            ("success record", "success"),
            ("failure record", "failure"),
        ):
            with self.subTest(field=field):
                self.assert_bad_input(
                    field, mcpx="[]", **{argument: json_arg("record")}
                )

    def test_valid_match_writes_success_and_zero_sleep(self):
        result, report, artifacts = self.run_parity(success="null", failure="null")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Spec parity: ✅ match", report)
        self.assertEqual(artifacts["status"], "ok")
        self.assertEqual(artifacts["sleep"], "0")
        self.assertEqual(
            json.loads(artifacts["success.json"]),
            {"timestamp": NOW, "run": "run/1", "specs": ["2025-03-26"]},
        )
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
            (False, 840, "warn", 840),
            (False, 841, "error", 841),
            (True, 360, "warn", 360),
            (True, 361, "error", 361),
            (False, 1000, "error", 900),
        ):
            with self.subTest(both_sided=both_sided, since=since):
                now = dt.datetime.strptime(NOW, "%Y-%m-%dT%H:%M:%SZ")
                success_time = (now - dt.timedelta(hours=since)).strftime(
                    "%Y-%m-%dT%H:%M:%SZ"
                )
                if both_sided:
                    mcpx = json_arg(["2025-03-26", "2025-11-25"])
                    suite = json_arg(["2025-03-26", "2026-01-10"])
                else:
                    mcpx = REVISIONS
                    suite = json_arg(["2025-03-26", "2025-11-25"])
                result, report, artifacts = self.run_parity(
                    mcpx=mcpx,
                    suite=suite,
                    success=json_arg({"timestamp": success_time}),
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("Spec parity:", report)
                self.assertEqual(artifacts["status"], expected_status)
                self.assertEqual(artifacts["sleep"], str(expected_sleep))


class ShapePropertyTests(unittest.TestCase):
    """Property coverage for the input-shape invariant (Codex P1 on PR
    #7046; testing doctrine: parser invariants get property tests with a
    named oracle).

    Oracle: a value is a valid revision/allow list iff it is a list whose
    items are all strings — stated here independently of spec-parity.py's
    own validator. A record argument is shape-valid iff it is a JSON
    object or null.

    Property: for every generated JSON value, an invalid shape in any
    validated field produces the graceful error report (status=error,
    sleep=0, no traceback, no success/failure record files), and a valid
    shape is never rejected as that field's shape error.
    """

    SEED = 6852

    def run_parity(
        self,
        *,
        mcpx=REVISIONS,
        suite=REVISIONS,
        now=NOW,
        success="",
        failure="",
        allow_mcpx="",
        allow_suite="",
    ):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            summary = root / "summary.md"
            out = root / "out"
            args = [
                sys.executable,
                str(SCRIPT),
                mcpx,
                suite,
                now,
                "run/1",
                success,
                failure,
                "false",
                str(summary),
                str(out),
                allow_mcpx,
                allow_suite,
            ]
            result = subprocess.run(args, capture_output=True, text=True, check=False)
            report = summary.read_text() if summary.exists() else ""
            artifacts = (
                {path.name: path.read_text() for path in out.iterdir()}
                if out.exists()
                else {}
            )
            return result, report, artifacts

    @staticmethod
    def _random_json(rng, depth=0):
        kinds = ["null", "bool", "int", "float", "str"]
        if depth < 3:
            kinds += ["list", "list", "dict"]
        kind = rng.choice(kinds)
        if kind == "null":
            return None
        if kind == "bool":
            return rng.choice([True, False])
        if kind == "int":
            return rng.randint(-3, 100)
        if kind == "float":
            return rng.choice([0.5, -1.25, 7.0])
        if kind == "str":
            return rng.choice(["2025-03-26", "2025-11-25", "", "x", NOW])
        if kind == "list":
            return [
                ShapePropertyTests._random_json(rng, depth + 1)
                for _ in range(rng.randint(0, 3))
            ]
        return {
            rng.choice(
                ["count", "timestamp", "specs", "run", "k"]
            ): ShapePropertyTests._random_json(rng, depth + 1)
            for _ in range(rng.randint(0, 3))
        }

    @staticmethod
    def _is_string_list(value):
        return isinstance(value, list) and all(isinstance(item, str) for item in value)

    def assert_graceful_shape_error(self, result, report, artifacts):
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, "")
        self.assertNotIn("Traceback", report + result.stderr)
        self.assertIn("could not read", report)
        self.assertEqual(artifacts.get("status"), "error")
        self.assertEqual(artifacts.get("sleep"), "0")
        self.assertNotIn("success.json", artifacts)
        self.assertNotIn("failure.json", artifacts)

    def test_property_list_fields_accept_exactly_string_lists(self):
        fields = (
            ("MCPX revisions", "mcpx"),
            ("suite revisions", "suite"),
            ("MCPX allow list", "allow_mcpx"),
            ("suite allow list", "allow_suite"),
        )
        for field, argument in fields:
            rng = random.Random(self.SEED)
            valid_seen = invalid_seen = 0
            for _ in range(48):
                value = self._random_json(rng)
                # An empty revision list has its own pre-existing named
                # error ("could not read the lists"); it is shape-valid,
                # so count it on the valid side and skip the run.
                if argument in ("mcpx", "suite") and value == []:
                    valid_seen += 1
                    continue
                with self.subTest(field=field, value=value):
                    result, report, artifacts = self.run_parity(
                        **{argument: json_arg(value)}
                    )
                    if self._is_string_list(value):
                        valid_seen += 1
                        self.assertEqual(result.returncode, 0, result.stderr)
                        self.assertNotIn("Traceback", report + result.stderr)
                        self.assertNotIn(f"could not read {field}", report)
                    else:
                        invalid_seen += 1
                        self.assert_graceful_shape_error(result, report, artifacts)
            # The property must not be vacuous on either side.
            self.assertGreaterEqual(invalid_seen, 20, field)
            self.assertGreaterEqual(valid_seen, 3, field)

    def test_property_record_fields_accept_exactly_objects_or_null(self):
        for field, argument in (
            ("success record", "success"),
            ("failure record", "failure"),
        ):
            rng = random.Random(self.SEED + 1)
            valid_seen = invalid_seen = 0
            for _ in range(32):
                value = self._random_json(rng)
                with self.subTest(field=field, value=value):
                    result, report, artifacts = self.run_parity(
                        **{argument: json_arg(value)}
                    )
                    if value is None or isinstance(value, dict):
                        valid_seen += 1
                        # Inner field defects are named gracefully too;
                        # the shape invariant is only: never a traceback,
                        # never a crash, never the record-shape error.
                        self.assertEqual(result.returncode, 0, result.stderr)
                        self.assertEqual(result.stderr, "")
                        self.assertNotIn("Traceback", report + result.stderr)
                        self.assertNotIn(f"could not read {field}", report)
                    else:
                        invalid_seen += 1
                        self.assert_graceful_shape_error(result, report, artifacts)
            self.assertGreaterEqual(invalid_seen, 15, field)
            self.assertGreaterEqual(valid_seen, 5, field)


if __name__ == "__main__":
    unittest.main()
