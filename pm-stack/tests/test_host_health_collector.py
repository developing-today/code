"""Offline regression tests for the host-health systemd snapshot projection."""

import importlib.util
import subprocess
import unittest
from datetime import UTC, datetime
from pathlib import Path
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / "host-health-collector.py"
SPEC = importlib.util.spec_from_file_location("host_health_collector", SCRIPT)
COLLECTOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(COLLECTOR)

UNIT = "pm-dashboard-backup"
EARLIER = "Fri 2026-11-20 20:00:00 UTC"
START = "Fri 2026-11-20 21:00:00 UTC"
DONE = "Fri 2026-11-20 21:00:10 UTC"
EARLIER_ISO = "2026-11-20T20:00:00+00:00"
DONE_ISO = "2026-11-20T21:00:10+00:00"


class HostHealthCollectorTests(unittest.TestCase):
    def read(self, props=None, *, returncode=0):
        lines = "\n".join(f"{key}={value}" for key, value in (props or {}).items())
        response = subprocess.CompletedProcess([], returncode, lines, "fixture error")
        with (
            patch.object(COLLECTOR.subprocess, "run", return_value=response) as run,
            patch.object(
                COLLECTOR.time,
                "mktime",
                side_effect=lambda value: datetime(*value[:6], tzinfo=UTC).timestamp(),
            ),
        ):
            record = COLLECTOR.show_unit(UNIT)
        call = run.call_args.args[0]
        self.assertEqual(call[:4], ["systemctl", "--user", "show", UNIT])
        return record, call[-1]

    def sample(self, **overrides):
        return self.read(
            {
                "Id": f"{UNIT}.service",
                "ActiveState": "inactive",
                "SubState": "dead",
                "Result": "success",
                "ExecMainCode": 1,
                "ExecMainStatus": 0,
                "ExecMainExitTimestamp": DONE,
                "InactiveEnterTimestamp": DONE,
                "StateChangeTimestamp": DONE,
                **overrides,
            }
        )[0]

    def test_failed_retry_and_recovery(self):
        failed = self.sample(
            ActiveState="failed",
            SubState="failed",
            Result="exit-code",
            ExecMainStatus=45,
            ExecMainExitTimestamp=EARLIER,
        )
        retry = self.sample(
            ActiveState="activating",
            SubState="start",
            ExecMainStatus=45,
            ExecMainExitTimestamp=EARLIER,
            StateChangeTimestamp=START,
        )
        recovered = self.sample()
        self.assertEqual(
            [(r["result"], r["lastSuccessAt"]) for r in (failed, retry, recovered)],
            [("exit-code", None), ("exit-code", None), ("success", DONE_ISO)],
        )

    def test_retry_retains_a_demonstrably_prior_success(self):
        prior = self.sample(
            ActiveState="activating",
            SubState="start",
            ExecMainExitTimestamp=EARLIER,
            StateChangeTimestamp=START,
        )
        self.assertEqual(
            (prior["result"], prior["lastSuccessAt"]), ("success", EARLIER_ISO)
        )

    def test_retry_ambiguous_or_pending_is_not_completed(self):
        for details in (
            {"ExecMainExitTimestamp": START},
            {"ExecMainExitTimestamp": EARLIER, "SubState": "start-post"},
            {"ExecMainExitTimestamp": DONE},
            {"ExecMainExitTimestamp": EARLIER, "StateChangeTimestamp": ""},
            {"ExecMainExitTimestamp": EARLIER, "ExecMainCode": 0, "ExecMainStatus": 0},
        ):
            with self.subTest(details=details):
                retry = self.sample(
                    **{
                        "ActiveState": "activating",
                        "SubState": "start",
                        "StateChangeTimestamp": START,
                        **details,
                    }
                )
                self.assertEqual(
                    (retry["result"], retry["lastSuccessAt"]), (None, None)
                )

    def test_no_success_without_affirmative_completion(self):
        for details in (
            {"ExecMainStatus": ""},
            {"ExecMainStatus": "n/a"},
            {"ExecMainCode": ""},
            {"ExecMainExitTimestamp": ""},
            {"ExecMainExitTimestamp": "bad timestamp"},
            {"ExecMainExitTimestamp": "Fri 2026-11-31 21:00:10 UTC"},
            {"ActiveState": "active", "SubState": "running"},
        ):
            with self.subTest(details=details):
                observed = self.sample(**details)
                self.assertEqual(
                    (observed["result"], observed["lastSuccessAt"]), (None, None)
                )
        self.assertIsNone(self.sample(ExecMainStatus="")["execMainStatus"])

    def test_retained_signalled_process_is_not_success(self):
        retained = self.sample(
            ActiveState="activating",
            SubState="start",
            ExecMainCode=2,
            ExecMainStatus=15,
            ExecMainExitTimestamp=EARLIER,
            StateChangeTimestamp=START,
        )
        self.assertEqual(
            (retained["result"], retained["lastSuccessAt"]), ("signal", None)
        )

    def test_post_exit_unit_failure_wins_over_clean_process(self):
        failed = self.sample(Result="timeout", ActiveState="failed")
        self.assertEqual((failed["result"], failed["lastSuccessAt"]), ("timeout", None))

    def test_remain_after_exit_is_a_completed_success(self):
        completed = self.sample(ActiveState="active", SubState="exited")
        self.assertEqual(
            (completed["result"], completed["lastSuccessAt"]), ("success", DONE_ISO)
        )

    def test_missing_show_and_required_code_property(self):
        missing, props = self.read()
        self.assertEqual(missing, {"state": "unknown", "detail": "unit not found"})
        self.assertIn("ExecMainCode", props.removeprefix("--property=").split(","))
        unavailable, _ = self.read(returncode=1)
        self.assertEqual(unavailable["state"], "unknown")
