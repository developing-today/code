"""Tests for the report-only content-creation gap monitor adapter.

Run:  cd pm-stack/fleet-monitor && python3 -m unittest test_content_creation_gap

Oracle: golden corpus — the CLI lines below are verbatim from the
throttle side (content_throttle.py gap-check print statements), and
the exit-code contract (0 clear, 2 alert, anything else detector
failure) is the integration surface its README documents. The tests
pin that the monitor honors the signal and never fakes a clear
record.
"""

import io
import json
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

import content_creation_gap as gap


ALERT_LINE = (
    "ALERT gap: queued filing work {'stuck_reservations': 2, "
    "'waiting_slots': 1, 'outbox_prepared': 0} and no creation for "
    "312.4s (threshold 300s; last creation 2026-11-28T05:40:00Z)"
)
OK_LINE = (
    "ok: no filing gap (queued {'stuck_reservations': 0, "
    "'waiting_slots': 0, 'outbox_prepared': 0}, "
    "last creation 2026-11-28T05:40:00Z)"
)


def stub_throttle_file(testcase):
    """A placeholder throttle path so stubbed-runner tests pass is_file."""
    stub = tempfile.NamedTemporaryFile(
        prefix="content_throttle_stub_", suffix=".py", delete=False)
    stub.close()
    testcase.addCleanup(Path(stub.name).unlink)
    return stub.name


def stub_runner(returncode, stdout):
    """A subprocess.run stand-in returning fixed (returncode, stdout)."""
    def run(command, **kwargs):
        result = type("Result", (), {})()
        result.returncode = returncode
        result.stdout = stdout
        run.seen = getattr(run, "seen", [])
        run.seen.append(command)
        return result

    run.seen = []
    return run


class MonitorRecordTests(unittest.TestCase):
    def test_exit_2_is_an_alert_with_parsed_detail(self):
        record = gap.monitor_record(2, ALERT_LINE + "\n")

        self.assertEqual(record["check"], "content_creation_gap")
        self.assertEqual(record["status"], "alert")
        self.assertEqual(
            record["queued"],
            {"stuck_reservations": 2, "waiting_slots": 1,
             "outbox_prepared": 0},
        )
        self.assertEqual(record["gap_seconds"], 312.4)
        self.assertEqual(record["threshold_seconds"], 300)
        self.assertEqual(record["last_creation"], "2026-11-28T05:40:00Z")

    def test_exit_0_is_clear_with_parsed_detail(self):
        record = gap.monitor_record(0, OK_LINE + "\n")

        self.assertEqual(record["status"], "clear")
        self.assertEqual(
            record["queued"],
            {"stuck_reservations": 0, "waiting_slots": 0,
             "outbox_prepared": 0},
        )
        self.assertEqual(record["last_creation"], "2026-11-28T05:40:00Z")
        self.assertIsNone(record["gap_seconds"])

    def test_detector_failure_is_never_a_clear_record(self):
        for rc in (1, 5, 6, 7):
            with self.assertRaises(ValueError, msg=f"exit {rc}"):
                gap.monitor_record(rc, "")

    def test_unparsable_detail_keeps_the_signal_not_fake_fields(self):
        record = gap.monitor_record(2, "ALERT gap: something changed\n")

        self.assertEqual(record["status"], "alert")
        self.assertIsNone(record["queued"])
        self.assertIn("detail_raw", record)


class RunGapCheckTests(unittest.TestCase):
    def test_missing_throttle_script_fails_before_any_subprocess(self):
        runner = stub_runner(0, OK_LINE)
        with self.assertRaises(RuntimeError):
            gap.run_gap_check("/nonexistent/content_throttle.py",
                              runner=runner)
        self.assertEqual(runner.seen, [])

    def test_outbox_flag_is_passed_through(self):
        throttle_py = stub_throttle_file(self)
        runner = stub_runner(0, OK_LINE + "\n")

        rc, out = gap.run_gap_check(
            throttle_py, outbox="/tmp/outbox.md", runner=runner)

        self.assertEqual(rc, 0)
        self.assertIn("gap-check", runner.seen[0])
        self.assertIn("--outbox", runner.seen[0])
        self.assertIn("/tmp/outbox.md", runner.seen[0])
        self.assertIn(OK_LINE, out)

    def test_no_outbox_flag_by_default(self):
        throttle_py = stub_throttle_file(self)
        runner = stub_runner(0, OK_LINE + "\n")

        gap.run_gap_check(throttle_py, runner=runner)

        self.assertNotIn("--outbox", runner.seen[0])


class MainTests(unittest.TestCase):
    def test_main_prints_the_alert_record(self):
        throttle_py = stub_throttle_file(self)
        runner = stub_runner(2, ALERT_LINE + "\n")

        buf = io.StringIO()
        with redirect_stdout(buf):
            rc = gap.main(["--throttle-py", throttle_py], runner=runner)

        self.assertEqual(rc, 0)
        record = json.loads(buf.getvalue())
        self.assertEqual(record["status"], "alert")
        self.assertEqual(record["gap_seconds"], 312.4)

    def test_main_exits_nonzero_on_detector_failure(self):
        throttle_py = stub_throttle_file(self)
        runner = stub_runner(1, "boom\n")

        rc = gap.main(["--throttle-py", throttle_py], runner=runner)

        self.assertEqual(rc, 1)


if __name__ == "__main__":
    unittest.main()
