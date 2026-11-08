#!/usr/bin/env python3
"""Synthetic regressions for conformance-record.py (#6392).

The stored record mirrors the probe in #6392: 10 passed / 0 failed /
12 total / 1 warning / 1 skipped, skipped=["check-A"], as both the
best and the largest record for the leg.
"""

import json
import pathlib
import subprocess
import tempfile
import unittest


SCRIPT = pathlib.Path(__file__).with_name("conformance-record.py")
NOW = "2026-11-06T00:00:00Z"
BEFORE = "2026-01-01T00:00:00Z"


def stored_record(passed=10, failed=0, total=12, warnings=1, skipped=1,
                  names=("check-A",), run="previous/1", first=BEFORE, latest=BEFORE):
    r = {
        "first_timestamp": first,
        "latest_timestamp": latest,
        "run": run,
        "passed_count": passed,
        "failed_count": failed,
        "total_count": total,
        "warning_count": warnings,
        "skipped_count": skipped,
        "info_count": 0,
    }
    if names is not None:
        r["skipped"] = sorted(names)
    return r


STORED = json.dumps({"best": stored_record(), "largest": stored_record()})


class ConformanceRecordTests(unittest.TestCase):
    def run_record(self, passed=10, failed=0, total=12, warnings=1, skipped=1,
                   names=("check-A",), stored=STORED):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            summary = root / "summary.md"
            out = root / "new.json"
            args = [
                "python3", str(SCRIPT), "leg", str(passed), str(failed),
                "run/2", NOW, stored, str(summary), str(out),
                str(total), str(warnings), str(skipped), "0",
            ]
            if names is not None:
                args.append(json.dumps(sorted(names)))
            result = subprocess.run(args, capture_output=True, text=True, check=False)
            return (
                result,
                summary.read_text(),
                json.loads(out.read_text()) if out.exists() else None,
            )

    def test_equal_count_swap_is_an_error_and_moves_neither_record(self):
        # check-A runs again (as a warning); check-B, which used to
        # run, is now skipped. The count never rose, so the old guard
        # passed this silently and saved best/largest disagreeing
        # about which check is skipped.
        result, summary, state = self.run_record(names=("check-B",))
        self.assertEqual(result.returncode, 3)
        self.assertIn("newly skipped", summary)
        self.assertIn("check-B", summary)
        self.assertIn("records unchanged", summary)
        self.assertIsNone(state)

    def test_swap_plus_pass_is_an_error_and_does_not_rebaseline_best(self):
        # check-A un-skips and passes; check-B becomes skipped. The
        # passed increase used to replace best wholesale, absorbing
        # check-B into the baseline with no alert.
        result, summary, state = self.run_record(passed=11, names=("check-B",))
        self.assertEqual(result.returncode, 3)
        self.assertIn("check-B", summary)
        self.assertNotIn("**improved**", summary)
        self.assertIsNone(state)

    def test_count_increase_still_errors_and_names_the_newly_skipped(self):
        result, summary, state = self.run_record(
            skipped=2, names=("check-A", "check-B"))
        self.assertEqual(result.returncode, 3)
        self.assertIn("skipped checks rose from 1 to 2", summary)
        self.assertIn("newly skipped: check-B", summary)
        self.assertIsNone(state)

    def test_identical_run_only_moves_timestamps_and_records_agree(self):
        result, summary, state = self.run_record()
        self.assertEqual(result.returncode, 0)
        self.assertIsNotNone(state)
        self.assertEqual(state["best"]["latest_timestamp"], NOW)
        self.assertEqual(state["best"]["first_timestamp"], BEFORE)
        self.assertEqual(state["best"]["skipped"], ["check-A"])
        self.assertEqual(state["largest"]["skipped"], ["check-A"])
        self.assertEqual(state["largest"]["first_timestamp"], BEFORE)

    def test_pure_unskip_with_more_passed_replaces_both_records(self):
        # Nothing newly skipped: the improvement path still works and
        # both records take this run's (empty) skipped set.
        result, summary, state = self.run_record(passed=11, skipped=0, names=())
        self.assertEqual(result.returncode, 0)
        self.assertIsNotNone(state)
        self.assertEqual(state["best"]["skipped"], [])
        self.assertEqual(state["largest"]["skipped"], [])

    def test_retained_best_with_contradicting_skipped_list_is_reported(self):
        # Same not-passed share, same passed: best is retained. The
        # stored skipped name no longer matches this run (check-A
        # failed instead of skipping; nothing is newly skipped), so
        # the divergence from largest's fresh names must be reported,
        # not silently kept.
        result, summary, state = self.run_record(failed=1, skipped=0, names=())
        self.assertEqual(result.returncode, 0)
        self.assertIn("differs from this run's", summary)
        self.assertIsNotNone(state)
        self.assertEqual(state["best"]["skipped"], ["check-A"])
        self.assertEqual(state["largest"]["skipped"], [])

    def test_legacy_record_without_counts_is_replaced_not_compared(self):
        legacy = {"passed_count": 9, "failed_count": 1, "run": "old/1",
                  "first_timestamp": BEFORE, "latest_timestamp": BEFORE}
        stored = json.dumps({"best": legacy, "largest": legacy})
        result, summary, state = self.run_record(names=("check-B",), stored=stored)
        self.assertEqual(result.returncode, 0)
        self.assertIsNotNone(state)
        self.assertEqual(state["best"]["passed_count"], 10)
        self.assertEqual(state["best"]["skipped"], ["check-B"])

    def test_run_without_skipped_names_is_compared_on_counts_only(self):
        # No names argument: the set guard cannot fire; behavior is
        # the pre-#6392 count-only comparison.
        result, summary, state = self.run_record(names=None)
        self.assertEqual(result.returncode, 0)
        self.assertIsNotNone(state)


class StoredShapeTests(unittest.TestCase):
    """#6850: wrong-shape stored values and skipped-names arguments are
    one named error (leg + field, exit 1, no traceback) -- never a raw
    TypeError/AttributeError, and never silently iterated as
    characters/keys the way a string or object names value was."""

    def run_raw(self, stored, names_arg=None):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            summary = root / "summary.md"
            out = root / "new.json"
            args = [
                "python3", str(SCRIPT), "leg", "10", "0",
                "run/2", NOW, stored, str(summary), str(out),
                "12", "1", "1", "0",
            ]
            if names_arg is not None:
                args.append(names_arg)
            result = subprocess.run(args, capture_output=True, text=True, check=False)
            return result

    def assert_named_error(self, result, *needles):
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertNotIn("Traceback", result.stderr)
        for needle in needles:
            self.assertIn(needle, result.stderr)

    def test_stored_top_level_string_is_a_named_error(self):
        self.assert_named_error(
            self.run_raw(json.dumps("hello")),
            "stored record for leg 'leg'", "must be a JSON object")

    def test_stored_top_level_list_is_a_named_error(self):
        self.assert_named_error(
            self.run_raw(json.dumps([1, 2])),
            "stored record for leg 'leg'", "must be a JSON object")

    def test_stored_invalid_json_is_a_named_error_not_a_traceback(self):
        self.assert_named_error(
            self.run_raw("{oops"),
            "stored record for leg 'leg'", "not valid JSON")

    def test_best_as_string_is_a_named_error(self):
        stored = json.dumps({"best": "x", "largest": stored_record()})
        self.assert_named_error(
            self.run_raw(stored), "field 'best'", "must be an object")

    def test_count_field_as_string_is_a_named_error(self):
        bad = stored_record()
        bad["passed_count"] = "10"
        stored = json.dumps({"best": bad, "largest": stored_record()})
        self.assert_named_error(
            self.run_raw(stored), "'best.passed_count'", "non-negative integer")

    def test_skipped_count_as_string_is_a_named_error(self):
        bad = stored_record()
        bad["skipped_count"] = "1"
        stored = json.dumps({"best": stored_record(), "largest": bad})
        self.assert_named_error(
            self.run_raw(stored), "'largest.skipped_count'", "non-negative integer")

    def test_bool_and_negative_counts_are_named_errors(self):
        for value in (True, -1):
            bad = stored_record()
            bad["failed_count"] = value
            stored = json.dumps({"best": bad, "largest": stored_record()})
            self.assert_named_error(
                self.run_raw(stored), "'best.failed_count'", "non-negative integer")

    def test_skipped_as_int_is_a_named_error(self):
        bad = stored_record()
        bad["skipped"] = 3
        stored = json.dumps({"best": bad, "largest": stored_record()})
        self.assert_named_error(
            self.run_raw(stored), "'best.skipped'", "list of strings")

    def test_skipped_as_string_is_a_named_error_not_a_character_set(self):
        # The silent case: best.skipped as a string used to be compared
        # as a set of characters by the #6392 guard.
        bad = stored_record()
        bad["skipped"] = "abcde"
        stored = json.dumps({"best": bad, "largest": stored_record()})
        self.assert_named_error(
            self.run_raw(stored), "'best.skipped'", "list of strings")

    def test_non_string_timestamp_is_a_named_error(self):
        bad = stored_record()
        bad["run"] = 42
        stored = json.dumps({"best": bad, "largest": stored_record()})
        self.assert_named_error(self.run_raw(stored), "'best.run'", "must be a string")

    def test_names_as_string_is_a_named_error_not_characters(self):
        # The silent case: skipped became ['a', 'b', 'c'].
        self.assert_named_error(
            self.run_raw(STORED, json.dumps("abc")),
            "skipped names for leg 'leg'", "list of strings")

    def test_names_as_object_is_a_named_error_not_keys(self):
        self.assert_named_error(
            self.run_raw(STORED, json.dumps({"a": 1})),
            "skipped names for leg 'leg'", "list of strings")

    def test_names_as_int_and_list_with_int_are_named_errors(self):
        for arg in ("123", json.dumps(["check-A", 1])):
            self.assert_named_error(
                self.run_raw(STORED, arg),
                "skipped names for leg 'leg'", "list of strings")

    def test_names_invalid_json_is_a_named_error(self):
        self.assert_named_error(
            self.run_raw(STORED, "{oops"), "skipped names for leg 'leg'", "not valid JSON")


if __name__ == "__main__":
    unittest.main()
