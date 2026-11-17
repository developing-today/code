#!/usr/bin/env python3
"""Subprocess bounds for the epic-invariant sweeper (#11416).

Oracle: the issue's acceptance shape. Each helper that shells out
(gh_graphql, gh_run, kaneo_links) must pass a timeout to subprocess.run,
and a timed-out call must surface as a RuntimeError naming the call,
which the sweeper's existing error paths already report.

Run:  python3 pm-stack/tests/test_invariant_sweeper_timeouts.py
"""

import importlib.util
import subprocess
import unittest
from pathlib import Path
from unittest import mock

SWEEPER_PATH = Path(__file__).resolve().parent.parent / "invariant-sweeper.py"
spec = importlib.util.spec_from_file_location("invariant_sweeper", SWEEPER_PATH)
sweeper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sweeper)


def timing_out(cmd, **kwargs):
    raise subprocess.TimeoutExpired(cmd, kwargs.get("timeout"))


class SubprocessBoundsTest(unittest.TestCase):
    def assert_times_out(self, call, label):
        seen = {}

        def fake(cmd, **kwargs):
            seen.update(kwargs)
            return timing_out(cmd, **kwargs)

        with mock.patch.object(sweeper.subprocess, "run", side_effect=fake):
            with self.assertRaises(RuntimeError) as ctx:
                call()
        self.assertEqual(seen.get("timeout"), sweeper.SUBPROCESS_TIMEOUT_SECS)
        self.assertIn(label, str(ctx.exception))
        self.assertIn("timed out", str(ctx.exception))

    def test_gh_graphql_is_bounded(self):
        self.assert_times_out(lambda: sweeper.gh_graphql("query{viewer{login}}"), "gh api graphql")

    def test_gh_run_is_bounded(self):
        self.assert_times_out(lambda: sweeper.gh_run("api", "rate_limit"), "gh api rate_limit")

    def test_kaneo_psql_is_bounded(self):
        self.assert_times_out(sweeper.kaneo_links, "kaneo psql")

    def test_completed_calls_still_return_output(self):
        done = subprocess.CompletedProcess([], 0, stdout="ok\n", stderr="")
        with mock.patch.object(sweeper.subprocess, "run", return_value=done) as run:
            self.assertEqual(sweeper.gh_run("api", "rate_limit"), "ok\n")
        self.assertEqual(run.call_args.kwargs["timeout"], sweeper.SUBPROCESS_TIMEOUT_SECS)

    def test_the_timer_unit_has_an_outer_bound(self):
        unit = (SWEEPER_PATH.parent / "systemd" / "pm-invariant-sweeper.service").read_text()
        self.assertIn("TimeoutStartSec=", unit)


if __name__ == "__main__":
    unittest.main()
