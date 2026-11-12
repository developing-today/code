#!/usr/bin/env python3
"""Regression tests for consecutive-day mirror fetch failures (#3257).

Run: python3 infra/github-backup/tests/test_fetch_mirrors.py
"""

import contextlib
import io
import json
import os
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

BIN = os.path.abspath(
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "bin")
)
sys.path.insert(0, BIN)

import fetch_mirrors


class FetchMirrorsFailureStreakTests(unittest.TestCase):
    def run_sequence(self, sequence, initial_state=None):
        """Run fetch_mirrors.main() repeatedly against persisted temp state."""
        with tempfile.TemporaryDirectory(prefix="ghb-fetch-test-") as root:
            day = [None]
            if initial_state is not None:
                with open(os.path.join(root, "state.json"), "w") as state_file:
                    json.dump(initial_state, state_file)

            def fake_strftime(fmt):
                if fmt == "%Y-%m-%d":
                    return day[0]
                return day[0] + "T12:00:00+0000"

            results = [
                SimpleNamespace(
                    returncode=0 if outcome == "ok" else 1,
                    stdout="",
                    stderr="fetch failed" if outcome == "fail" else "",
                )
                for _, outcome in sequence
            ]
            exit_codes = []
            with (
                patch.object(fetch_mirrors, "load_config", return_value={}),
                patch.object(fetch_mirrors, "backup_root", return_value=root),
                patch.object(fetch_mirrors, "mirror_path", return_value="/mirror"),
                patch.object(fetch_mirrors, "valid_mirror", return_value=True),
                patch.object(fetch_mirrors.manifest_mod, "build"),
                patch.object(fetch_mirrors.time, "strftime", side_effect=fake_strftime),
                patch.object(fetch_mirrors.subprocess, "run", side_effect=results),
                contextlib.redirect_stdout(io.StringIO()),
            ):
                for date, _ in sequence:
                    day[0] = date
                    exit_codes.append(
                        fetch_mirrors.main(["fetch_mirrors.py", "owner/repo"])
                    )

            with open(os.path.join(root, "state.json")) as state_file:
                state = json.load(state_file)
            return exit_codes, state["owner/repo"].get("fail_days", [])

    def test_failures_weeks_apart_do_not_accumulate(self):
        exit_codes, fail_days = self.run_sequence(
            [
                ("2026-01-05", "fail"),
                ("2026-02-10", "fail"),
                ("2026-03-20", "fail"),
            ]
        )
        self.assertEqual(exit_codes, [0, 0, 0])
        self.assertEqual(fail_days, ["2026-03-20"])

    def test_three_consecutive_failure_days_are_chronic(self):
        exit_codes, fail_days = self.run_sequence(
            [
                ("2026-01-05", "fail"),
                ("2026-01-06", "fail"),
                ("2026-01-07", "fail"),
            ]
        )
        self.assertEqual(exit_codes, [0, 0, 1])
        self.assertEqual(fail_days, ["2026-01-05", "2026-01-06", "2026-01-07"])

    def test_success_resets_the_streak(self):
        exit_codes, fail_days = self.run_sequence(
            [
                ("2026-01-05", "fail"),
                ("2026-01-06", "fail"),
                ("2026-01-07", "ok"),
                ("2026-01-08", "fail"),
                ("2026-01-09", "fail"),
            ]
        )
        self.assertEqual(exit_codes, [0, 0, 0, 0, 0])
        self.assertEqual(fail_days, ["2026-01-08", "2026-01-09"])

    def test_success_only_is_a_non_chronic_control(self):
        exit_codes, fail_days = self.run_sequence([("2026-01-05", "ok")])
        self.assertEqual(exit_codes, [0])
        self.assertEqual(fail_days, [])

    def test_repeated_failure_on_one_day_counts_once(self):
        exit_codes, fail_days = self.run_sequence(
            [("2026-01-05", "fail"), ("2026-01-05", "fail")]
        )
        self.assertEqual(exit_codes, [0, 0])
        self.assertEqual(fail_days, ["2026-01-05"])

    def test_legacy_history_keeps_only_the_latest_consecutive_suffix(self):
        exit_codes, fail_days = self.run_sequence(
            [("2026-03-21", "fail")],
            {
                "owner/repo": {
                    "fail_days": [
                        "2026-01-05",
                        "2026-02-10",
                        "2026-03-19",
                        "2026-03-20",
                    ]
                }
            },
        )
        self.assertEqual(exit_codes, [1])
        self.assertEqual(fail_days, ["2026-03-19", "2026-03-20", "2026-03-21"])

    def test_timeout_continues_and_records_chronic_failure(self):
        with tempfile.TemporaryDirectory(prefix="ghb-fetch-timeout-test-") as root:
            initial_state = {
                "owner/first": {
                    "fail_days": ["2026-01-05", "2026-01-06"],
                }
            }
            with open(os.path.join(root, "state.json"), "w") as state_file:
                json.dump(initial_state, state_file)

            def fake_strftime(fmt):
                if fmt == "%Y-%m-%d":
                    return "2026-01-07"
                return "2026-01-07T12:00:00+0000"

            timeout = fetch_mirrors.subprocess.TimeoutExpired("git fetch", 3600)
            later_success = SimpleNamespace(returncode=0, stdout="", stderr="")
            with (
                patch.object(fetch_mirrors, "load_config", return_value={}),
                patch.object(fetch_mirrors, "backup_root", return_value=root),
                patch.object(
                    fetch_mirrors,
                    "mirror_path",
                    side_effect=lambda _root, repo: f"/mirror/{repo}",
                ),
                patch.object(fetch_mirrors, "valid_mirror", return_value=True),
                patch.object(fetch_mirrors.manifest_mod, "build") as build_manifest,
                patch.object(fetch_mirrors.time, "strftime", side_effect=fake_strftime),
                patch.object(
                    fetch_mirrors.subprocess,
                    "run",
                    side_effect=[timeout, later_success],
                ) as run,
                contextlib.redirect_stdout(io.StringIO()),
            ):
                exit_code = fetch_mirrors.main(
                    ["fetch_mirrors.py", "owner/first", "owner/later"]
                )

            self.assertEqual(exit_code, 1)
            self.assertEqual(run.call_count, 2)
            self.assertEqual(run.call_args_list[1].args[0][2], "/mirror/owner/later")
            build_manifest.assert_called_once_with(root)

            with open(os.path.join(root, "state.json")) as state_file:
                state = json.load(state_file)
            self.assertEqual(
                state["owner/first"]["fail_days"],
                ["2026-01-05", "2026-01-06", "2026-01-07"],
            )
            self.assertIn("timed out", state["owner/first"]["last_error"].lower())
            self.assertEqual(state["owner/later"]["fail_days"], [])
            self.assertIn("last_fetch", state["owner/later"])

            with open(os.path.join(root, "fetch.log")) as log_file:
                log = log_file.read()
            self.assertIn("owner/first fail", log)
            self.assertIn("timed out", log.lower())


class FetchMirrorsLastFetchShapeTests(unittest.TestCase):
    def test_wrong_shape_last_fetch_does_not_crash_the_loop(self):
        # Oracle (#6821): a string last_fetch on one repo must not
        # stop the daily fetch — the poisoned repo is fetched, the
        # fresh repo coalesce-skips, state is rewritten, and the
        # manifest is regenerated.
        import time as time_mod

        with tempfile.TemporaryDirectory(prefix="ghb-fetch-shape-") as root:
            with open(os.path.join(root, "plan.json"), "w") as plan_file:
                json.dump(
                    [{"repo": "owner/poison"}, {"repo": "owner/fresh"}],
                    plan_file,
                )
            initial_state = {
                "owner/poison": {"last_fetch": "2026-11-01T00:00:00Z"},
                "owner/fresh": {"last_fetch": time_mod.time()},
            }
            with open(os.path.join(root, "state.json"), "w") as state_file:
                json.dump(initial_state, state_file)
            success = SimpleNamespace(returncode=0, stdout="", stderr="")
            with (
                patch.object(fetch_mirrors, "load_config", return_value={}),
                patch.object(fetch_mirrors, "backup_root", return_value=root),
                patch.object(
                    fetch_mirrors,
                    "mirror_path",
                    side_effect=lambda _root, repo: f"/mirror/{repo}",
                ),
                patch.object(fetch_mirrors, "valid_mirror", return_value=True),
                patch.object(fetch_mirrors.manifest_mod, "build") as build_manifest,
                patch.object(
                    fetch_mirrors.subprocess, "run", return_value=success
                ) as run,
                contextlib.redirect_stdout(io.StringIO()),
                contextlib.redirect_stderr(io.StringIO()) as err,
            ):
                exit_code = fetch_mirrors.main(["fetch_mirrors.py"])
            self.assertEqual(exit_code, 0)
            self.assertIn("last_fetch", err.getvalue())
            self.assertEqual(run.call_count, 1)  # only the poisoned repo fetched
            self.assertEqual(run.call_args_list[0].args[0][2], "/mirror/owner/poison")
            build_manifest.assert_called_once_with(root)
            with open(os.path.join(root, "state.json")) as state_file:
                state = json.load(state_file)
            self.assertIsInstance(state["owner/poison"]["last_fetch"], float)
            self.assertIsInstance(state["owner/fresh"]["last_fetch"], float)


if __name__ == "__main__":
    unittest.main()
