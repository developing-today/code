#!/usr/bin/env python3
"""Tests for the conductor-silence heartbeat (code#15123).

Everything runs offline against fixtures: `--fixture` replaces the
conductor-log read, so no live GitHub access and no issue writes are
needed. The acceptance demonstrations from code#15123 are here by
name:

- the conductor's actual 2026-11-24 silence (last cycle
  2026-11-24T04:25Z) fires exactly one tracking issue at the
  meta-review's detection time — the probe would have fired ~31h
  earlier, at the 2h threshold;
- a fresh cycle timestamp stays quiet;
- a silence that already has a tracking issue is updated, not
  duplicated.

Oracle: the verdict is a pure function of (cycle timestamps, now,
threshold) — age arithmetic against the threshold, nothing else.
"""

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "check_conductor.py"
sys.path.insert(0, str(HERE))

import check_conductor  # noqa: E402

NOW = "2026-11-25T13:29:00Z"
# The last conductor cycle before the silence in code#15123.
LAST_CYCLE = "2026-11-24T04:25:00Z"


def run_fixture(fixture: dict, now: str = NOW) -> dict:
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as fh:
        json.dump(fixture, fh)
        path = fh.name
    cmd = [sys.executable, str(SCRIPT), "--fixture", path, "--now", now]
    out = subprocess.run(cmd, capture_output=True, text=True, check=True).stdout
    return json.loads(out)


class ConductorSilenceTests(unittest.TestCase):
    def test_the_15123_silence_fires_exactly_one_tracking_issue(self):
        result = run_fixture({"cycles": [LAST_CYCLE], "tracking": None})
        actions = [a for a in result["actions"] if a["action"] != "none"]
        self.assertEqual(len(actions), 1)
        self.assertEqual(actions[0]["action"], "file")
        ev = result["evaluation"]
        self.assertEqual(ev["status"], "silent")
        self.assertEqual(ev["age_hours"], 33.067)

    def test_silence_would_have_fired_at_threshold_not_at_33h(self):
        # Two hours after the last cycle the probe already verdicts
        # silent; the 33h detection lag in code#15123 is what the
        # threshold exists to remove.
        result = run_fixture(
            {"cycles": [LAST_CYCLE], "tracking": None},
            now="2026-11-24T06:25:01Z",
        )
        self.assertEqual(result["evaluation"]["status"], "silent")

    def test_fresh_cycle_stays_quiet(self):
        result = run_fixture({"cycles": ["2026-11-25T12:45:00Z"], "tracking": None})
        self.assertEqual(result["actions"], [{"action": "none", "issue": None}])
        self.assertEqual(result["evaluation"]["status"], "ok")

    def test_existing_tracking_issue_is_updated_not_duplicated(self):
        result = run_fixture({"cycles": [LAST_CYCLE], "tracking": 15123})
        self.assertEqual(result["actions"], [{"action": "update", "issue": 15123}])

    def test_no_cycles_at_all_fires(self):
        result = run_fixture({"cycles": [], "tracking": None})
        self.assertEqual(result["evaluation"]["status"], "silent")
        self.assertIsNone(result["evaluation"]["last_cycle"])
        self.assertEqual(result["actions"][0]["action"], "file")

    def test_threshold_boundary(self):
        just_under = check_conductor.evaluate(
            [check_conductor.parse_ts("2026-11-25T11:29:01Z")],
            check_conductor.parse_ts(NOW),
            2.0,
        )
        just_over = check_conductor.evaluate(
            [check_conductor.parse_ts("2026-11-25T11:28:59Z")],
            check_conductor.parse_ts(NOW),
            2.0,
        )
        self.assertEqual(just_under["status"], "ok")
        self.assertEqual(just_over["status"], "silent")

    def test_newest_cycle_wins_regardless_of_order(self):
        cycles = [
            check_conductor.parse_ts("2026-11-20T04:25:00Z"),
            check_conductor.parse_ts("2026-11-25T12:45:00Z"),
            check_conductor.parse_ts(LAST_CYCLE),
        ]
        ev = check_conductor.evaluate(cycles, check_conductor.parse_ts(NOW), 2.0)
        self.assertEqual(ev["status"], "ok")
        self.assertEqual(ev["last_cycle"], "2026-11-25T12:45:00+00:00")

    def test_future_timestamps_do_not_mask_silence(self):
        cycles = [
            check_conductor.parse_ts(LAST_CYCLE),
            check_conductor.parse_ts("2027-01-01T00:00:00Z"),
        ]
        ev = check_conductor.evaluate(cycles, check_conductor.parse_ts(NOW), 2.0)
        self.assertEqual(ev["status"], "silent")
        self.assertEqual(ev["last_cycle"], "2026-11-24T04:25:00+00:00")

    def test_cycle_timestamps_parse_from_log_text(self):
        log = (
            "cycle 2026-11-24T03:25Z census 11/30\n"
            "cycle 2026-11-24T04:25Z census 12/30, dispatch 0\n"
            "filed 2026-11-20 for window 2026-11-19 (dates, not cycles)\n"
        )
        cycles = check_conductor.cycle_timestamps(log)
        self.assertEqual(
            cycles,
            [
                check_conductor.parse_ts("2026-11-24T03:25:00Z"),
                check_conductor.parse_ts("2026-11-24T04:25:00Z"),
            ],
        )

    def test_missing_log_file_is_an_error_not_a_verdict(self):
        cmd = [
            sys.executable,
            str(SCRIPT),
            "--log-file",
            str(HERE / "does-not-exist.md"),
            "--now",
            NOW,
        ]
        proc = subprocess.run(cmd, capture_output=True, text=True)
        self.assertEqual(proc.returncode, 1)
        result = json.loads(proc.stdout)
        self.assertEqual(result["evaluation"]["status"], "error")
        self.assertEqual(result["actions"], [{"action": "none", "issue": None}])


if __name__ == "__main__":
    unittest.main()
