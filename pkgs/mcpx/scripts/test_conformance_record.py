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


if __name__ == "__main__":
    unittest.main()
