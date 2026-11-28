"""Offline regression tests for the host-health systemd snapshot projection."""

import importlib.util
import json
import random
import subprocess
import unittest
from datetime import UTC, datetime, timedelta
from itertools import product
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

    def test_property_missing_completion_witness_never_claims_success(self):
        # Invariant oracle: removing or invalidating any one witness from the
        # completed fixture makes its completion unknown, regardless of the
        # sampled timestamp or whether it is a remain-after-exit service.
        rng = random.Random(12138)
        epoch = datetime(2026, 1, 1, tzinfo=UTC)
        for sample in range(160):
            exited = self.wire_time(epoch + timedelta(seconds=rng.randrange(31536000)))
            invalid_witnesses = (
                ("ExecMainExitTimestamp", ""),
                ("ExecMainExitTimestamp", f"invalid {rng.getrandbits(32)}"),
                ("ExecMainCode", ""),
                ("ExecMainCode", rng.randint(4, 65535)),
                ("ExecMainStatus", ""),
                ("ExecMainStatus", f"invalid {rng.getrandbits(32)}"),
                ("ExecMainStatus", -rng.randint(1, 65535)),
            )
            for state, (key, value) in product(
                (("inactive", "dead"), ("active", "exited")), invalid_witnesses
            ):
                with self.subTest(sample=sample, state=state, witness=key, value=value):
                    record = self.sample(
                        **{
                            "ActiveState": state[0],
                            "SubState": state[1],
                            "ExecMainExitTimestamp": exited,
                            key: value,
                        }
                    )
                    self.assertEqual(
                        (record["result"], record["lastSuccessAt"]), (None, None)
                    )

    def test_property_retry_only_reuses_a_strictly_prior_exit_while_starting(self):
        # Model oracle: only the PRIOR/START cell can report the historical
        # process outcome. Text timestamps are built from offsets with known
        # ordering, not compared using the collector's timestamp parser.
        rng = random.Random(12139)
        epoch = datetime(2026, 1, 2, tzinfo=UTC)
        for sample in range(96):
            started = epoch + timedelta(seconds=rng.randrange(31536000))
            interval = timedelta(seconds=rng.randint(1, 7200))
            prior, later = started - interval, started + interval
            exits = ((prior, True), (started, False), (later, False), (None, False))
            outcomes = (
                (1, 0, ("success", prior.isoformat())),
                (1, rng.randint(1, 255), ("exit-code", None)),
                (2, rng.randint(1, 64), ("signal", None)),
                (3, rng.randint(1, 64), ("core-dump", None)),
            )
            for (exited, is_prior), substate, (code, status, history) in product(
                exits, ("start", "start-post", "start-pre"), outcomes
            ):
                with self.subTest(
                    sample=sample,
                    exited=exited,
                    substate=substate,
                    code=code,
                    status=status,
                ):
                    record = self.sample(
                        ActiveState="activating",
                        SubState=substate,
                        ExecMainCode=code,
                        ExecMainStatus=status,
                        ExecMainExitTimestamp=self.wire_time(exited) if exited else "",
                        StateChangeTimestamp=self.wire_time(started),
                    )
                    expected = (
                        history
                        if (is_prior, substate) == (True, "start")
                        else (None, None)
                    )
                    self.assertEqual(
                        (record["result"], record["lastSuccessAt"]), expected
                    )

    @staticmethod
    def wire_time(value):
        return value.strftime("%Y-%m-%d %H:%M:%S UTC")

    def test_missing_show_and_required_code_property(self):
        missing, props = self.read()
        self.assertEqual(missing, {"state": "unknown", "detail": "unit not found"})
        self.assertIn("ExecMainCode", props.removeprefix("--property=").split(","))
        unavailable, _ = self.read(returncode=1)
        self.assertEqual(unavailable["state"], "unknown")


class ReadOpenCodeRuntimeTests(unittest.TestCase):
    REV = "abcdef1234567890abcdef1234567890abcdef12"

    def write_lock(self, tmp_path):
        lock = Path(tmp_path) / "flake.lock"
        lock.write_text(json.dumps({
            "root": "root",
            "nodes": {
                "root": {"inputs": {"opencode": "opencode"}},
                "opencode": {"locked": {"rev": self.REV}},
            },
        }))
        return lock

    def runtime(self, tmp_path, executable, *, props="ActiveState=active\nMainPID=321\n"):
        lock = self.write_lock(tmp_path)
        proc = Path(tmp_path) / "proc"
        (proc / "321").mkdir(parents=True, exist_ok=True)
        with (
            patch.object(COLLECTOR.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, props, "")),
            patch.object(COLLECTOR.os, "readlink", return_value=executable),
        ):
            return COLLECTOR.read_opencode_runtime(proc_root=proc, flake_lock=lock)

    def test_running_executable_revision_matches_or_mismatches_pin(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            match = self.runtime(tmp, f"/nix/store/opencode-1.2.3+{self.REV[:12]}/bin/opencode")
            self.assertEqual(match["state"], "match")
            self.assertEqual(match["runtimeRevision"], self.REV[:12])
            mismatch = self.runtime(tmp, "/nix/store/opencode-1.2.3+1234567/bin/opencode")
            self.assertEqual(mismatch["state"], "mismatch")

    def test_deleted_or_unrecognizable_executable_is_unknown(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            deleted = self.runtime(tmp, f"/nix/store/opencode-1.2.3+{self.REV[:12]}/bin/opencode (deleted)")
            self.assertEqual(deleted["state"], "unknown")
            self.assertTrue(deleted["deleted"])
            unknown = self.runtime(tmp, "/nix/store/bin/opencode")
            self.assertEqual(unknown["state"], "unknown")
            self.assertIsNone(unknown["runtimeRevision"])

    def test_inactive_service_reports_down_without_reading_process(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            lock = self.write_lock(tmp)
            with patch.object(
                COLLECTOR.subprocess,
                "run",
                return_value=subprocess.CompletedProcess([], 0, "ActiveState=inactive\nMainPID=0\n", ""),
            ), patch.object(COLLECTOR.os, "readlink") as readlink:
                result = COLLECTOR.read_opencode_runtime(flake_lock=lock)
            self.assertEqual(result["state"], "down")
            readlink.assert_not_called()


class ReadBackupRecordTests(unittest.TestCase):
    """The run-record half of the snapshot (#3989 criterion 6): the
    collector passes the dashboard backup's newest verified generation
    through, and anything unreadable reads as unknown, never as health."""

    def write_record(self, tmp_path, payload):
        record = Path(tmp_path) / ".last-run.json"
        if isinstance(payload, (dict, list)):
            record.write_text(json.dumps(payload) + "\n")
        else:
            record.write_text(payload)
        return record

    def complete_record(self):
        return {
            "job": "pm-dashboard-backup",
            "date": "2026-11-22",
            "started_at": "2026-11-22T10:30:00+00:00",
            "finished_at": "2026-11-22T10:41:01+00:00",
            "result": "complete",
            "exit_code": 0,
            "failing_step": None,
            "steps": [{"step": "local-archive", "outcome": "ok"}],
            "archive_sha256": "8f4cec6c",
            "destinations": {"local": "ok", "r2": "ok", "drive": "ok"},
        }

    def test_complete_generation_passes_through(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            record = self.write_record(tmp, self.complete_record())
            backup = COLLECTOR.read_backup_record(str(record))
        self.assertEqual(backup["result"], "complete")
        self.assertEqual(backup["date"], "2026-11-22")
        self.assertEqual(backup["finishedAt"], "2026-11-22T10:41:01+00:00")
        self.assertEqual(backup["archiveSha256"], "8f4cec6c")
        self.assertEqual(
            backup["destinations"], {"local": "ok", "r2": "ok", "drive": "ok"}
        )
        self.assertIsNone(backup["failingStep"])

    def test_failed_generation_names_its_step(self):
        import tempfile

        payload = self.complete_record()
        payload.update(
            {"result": "failed", "exit_code": 1, "failing_step": "r2-upload"}
        )
        with tempfile.TemporaryDirectory() as tmp:
            record = self.write_record(tmp, payload)
            backup = COLLECTOR.read_backup_record(str(record))
        self.assertEqual(backup["result"], "failed")
        self.assertEqual(backup["failingStep"], "r2-upload")

    def test_missing_corrupt_or_shapeless_record_is_unknown(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            missing = COLLECTOR.read_backup_record(str(Path(tmp) / "absent.json"))
            self.assertEqual(missing["state"], "unknown")
            corrupt = self.write_record(tmp, "{not json")
            self.assertEqual(
                COLLECTOR.read_backup_record(str(corrupt))["state"], "unknown"
            )
            shapeless = self.write_record(tmp, {"job": "pm-dashboard-backup"})
            self.assertEqual(
                COLLECTOR.read_backup_record(str(shapeless))["state"], "unknown"
            )
            bad_result = self.write_record(
                tmp, {**self.complete_record(), "result": "complete-ish"}
            )
            self.assertEqual(
                COLLECTOR.read_backup_record(str(bad_result))["state"], "unknown"
            )
