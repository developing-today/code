"""Contract tests for check_transport.py (code#6369).

Offline: every pass pins --probe-result and --now, and GitHub is a
recording fake, so the whole alert / dedupe / recovery path runs
without a broker or a network.
"""

import contextlib
import importlib.util
import io
import json
import socket
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "check_transport.py"
spec = importlib.util.spec_from_file_location("check_transport", SCRIPT)
ct = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ct)


class FakeGitHub:
    def __init__(self, existing=None, fail_on=None):
        self.existing = existing
        self.fail_on = fail_on or set()
        self.calls = []
        self.next_number = 9000

    def _maybe_fail(self, op):
        if op in self.fail_on:
            raise RuntimeError(f"{op} failed")

    def find(self, repo):
        self.calls.append(("find", repo))
        self._maybe_fail("find")
        return self.existing

    def file(self, repo, title, body):
        self.calls.append(("file", title))
        self._maybe_fail("file")
        self.next_number += 1
        return self.next_number

    def comment(self, repo, number, body):
        self.calls.append(("comment", number, body))
        self._maybe_fail("comment")

    def close(self, repo, number):
        self.calls.append(("close", number))
        self._maybe_fail("close")

    def ops(self):
        return {
            "find": self.find,
            "file": self.file,
            "comment": self.comment,
            "close": self.close,
        }

    def kinds(self):
        return [c[0] for c in self.calls]


class Harness(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.state = Path(self.tmp.name) / "state" / "state.json"

    def tearDown(self):
        self.tmp.cleanup()

    def run_pass(self, result, now, gh=None, apply=True, extra=()):
        argv = ["--state", str(self.state), "--probe-result", result, "--now", now]
        if apply:
            argv.append("--apply")
        argv += list(extra)
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = ct.main(argv, github=gh.ops() if gh else None)
        return code, json.loads(out.getvalue())


class AlertTests(Harness):
    def test_healthy_pass_is_quiet(self):
        gh = FakeGitHub()
        code, rep = self.run_pass("ok", "2026-11-18T23:00:00Z", gh)
        self.assertEqual(code, 0)
        self.assertEqual(rep["event"], "ok")
        self.assertEqual(rep["message"], "")
        self.assertEqual(gh.calls, [])

    def test_single_failure_does_not_alert(self):
        gh = FakeGitHub()
        _, rep = self.run_pass("fail:cannot connect", "2026-11-18T23:00:00Z", gh)
        self.assertEqual(rep["event"], "failing")
        self.assertEqual(rep["message"], "")
        self.assertEqual(gh.calls, [])

    def test_second_failure_files_exactly_one_incident(self):
        gh = FakeGitHub()
        self.run_pass("fail:cannot connect", "2026-11-18T23:00:00Z", gh)
        _, rep = self.run_pass("fail:cannot connect", "2026-11-18T23:10:00Z", gh)
        self.assertEqual(rep["event"], "alert")
        self.assertEqual(gh.kinds(), ["find", "file"])
        self.assertEqual(rep["incident"], 9001)
        self.assertIn("unreachable for 10m", rep["message"])
        self.assertIn("#9001", rep["message"])
        title = gh.calls[1][1]
        self.assertTrue(title.startswith(ct.INCIDENT_TITLE_PREFIX))
        self.assertIn("2026-11-18T23:00:00Z", title)

    def test_continuing_outage_stays_quiet_after_alert(self):
        gh = FakeGitHub()
        for minute in ("00", "10", "20", "30", "40"):
            _, rep = self.run_pass("fail", f"2026-11-18T23:{minute}:00Z", gh)
        self.assertEqual(rep["event"], "down")
        self.assertEqual(rep["message"], "")
        self.assertEqual(gh.kinds(), ["find", "file"])
        self.assertEqual(rep["state"]["consecutive_failures"], 5)

    def test_open_incident_is_reused_not_duplicated(self):
        gh = FakeGitHub(existing=5708)
        self.run_pass("fail", "2026-11-18T23:00:00Z", gh)
        _, rep = self.run_pass("fail", "2026-11-18T23:10:00Z", gh)
        self.assertEqual(gh.kinds(), ["find", "comment"])
        self.assertEqual(gh.calls[1][1], 5708)
        self.assertEqual(rep["incident"], 5708)
        self.assertFalse(rep["state"]["incident_filed_by_probe"])

    def test_threshold_is_configurable(self):
        gh = FakeGitHub()
        _, rep = self.run_pass(
            "fail", "2026-11-18T23:00:00Z", gh, extra=["--threshold", "1"]
        )
        self.assertEqual(rep["event"], "alert")
        self.assertEqual(gh.kinds(), ["find", "file"])

    def test_failed_filing_exits_1_and_retries_next_pass(self):
        gh = FakeGitHub(fail_on={"file"})
        self.run_pass("fail", "2026-11-18T23:00:00Z", gh)
        code, rep = self.run_pass("fail", "2026-11-18T23:10:00Z", gh)
        self.assertEqual(code, 1)
        self.assertTrue(rep["apply_errors"])
        self.assertFalse(rep["state"]["alerted"])
        gh.fail_on = set()
        code, rep = self.run_pass("fail", "2026-11-18T23:20:00Z", gh)
        self.assertEqual(code, 0)
        self.assertEqual(rep["event"], "alert")
        self.assertEqual(gh.kinds(), ["find", "file", "find", "file"])


class RecoveryTests(Harness):
    def alerted(self, gh):
        self.run_pass("fail", "2026-11-18T23:00:00Z", gh)
        self.run_pass("fail", "2026-11-18T23:10:00Z", gh)

    def test_recovery_comments_once_and_never_closes(self):
        # #10136: automation may comment but must never close an issue,
        # even one it filed itself.
        gh = FakeGitHub()
        self.alerted(gh)
        _, rep = self.run_pass("ok", "2026-11-19T01:15:00Z", gh)
        self.assertEqual(rep["event"], "recovered")
        self.assertIn("after about 2h15m", rep["message"])
        self.assertEqual(gh.kinds(), ["find", "file", "comment"])
        self.assertEqual(gh.calls[2][1], 9001)
        self.assertNotIn("close", gh.kinds())
        _, rep = self.run_pass("ok", "2026-11-19T01:25:00Z", gh)
        self.assertEqual(rep["event"], "ok")
        self.assertEqual(rep["message"], "")
        self.assertEqual(len(gh.calls), 3)

    def test_recovery_leaves_hand_filed_incident_open(self):
        gh = FakeGitHub(existing=5708)
        self.alerted(gh)
        self.run_pass("ok", "2026-11-18T23:30:00Z", gh)
        self.assertEqual(gh.kinds(), ["find", "comment", "comment"])

    def test_blip_below_threshold_recovers_quietly(self):
        gh = FakeGitHub()
        self.run_pass("fail", "2026-11-18T23:00:00Z", gh)
        _, rep = self.run_pass("ok", "2026-11-18T23:10:00Z", gh)
        self.assertEqual(rep["event"], "ok")
        self.assertEqual(rep["message"], "")
        self.assertEqual(rep["state"]["consecutive_failures"], 0)
        self.assertIsNone(rep["state"]["window_start"])
        self.assertEqual(gh.calls, [])

    def test_failed_recovery_comment_is_retried(self):
        gh = FakeGitHub()
        self.alerted(gh)
        gh.fail_on = {"comment"}
        code, _ = self.run_pass("ok", "2026-11-18T23:20:00Z", gh)
        self.assertEqual(code, 1)
        gh.fail_on = set()
        code, rep = self.run_pass("ok", "2026-11-18T23:30:00Z", gh)
        self.assertEqual(code, 0)
        self.assertEqual(rep["event"], "recovered")
        self.assertEqual(gh.kinds()[-1], "comment")
        self.assertNotIn("close", gh.kinds())

    def test_new_outage_after_recovery_alerts_again(self):
        gh = FakeGitHub()
        self.alerted(gh)
        self.run_pass("ok", "2026-11-18T23:20:00Z", gh)
        self.run_pass("fail", "2026-11-19T02:00:00Z", gh)
        _, rep = self.run_pass("fail", "2026-11-19T02:10:00Z", gh)
        self.assertEqual(rep["event"], "alert")
        self.assertEqual(rep["incident"], 9002)


class ObserveModeTests(Harness):
    def test_observe_mode_never_touches_github_but_still_reports(self):
        gh = FakeGitHub()
        self.run_pass("fail", "2026-11-18T23:00:00Z", gh, apply=False)
        _, rep = self.run_pass("fail", "2026-11-18T23:10:00Z", gh, apply=False)
        self.assertEqual(rep["event"], "alert")
        self.assertEqual(rep["mode"], "observe")
        self.assertTrue(rep["message"])
        _, rep = self.run_pass("ok", "2026-11-18T23:20:00Z", gh, apply=False)
        self.assertEqual(rep["event"], "recovered")
        self.assertTrue(rep["message"])
        self.assertEqual(gh.calls, [])


class StateAndProbeTests(Harness):
    def test_corrupt_state_starts_fresh(self):
        self.state.parent.mkdir(parents=True)
        self.state.write_text("{not json")
        code, rep = self.run_pass("fail", "2026-11-18T23:00:00Z", FakeGitHub())
        self.assertEqual(code, 0)
        self.assertEqual(rep["state"]["consecutive_failures"], 1)

    def test_text_format_prints_only_the_message(self):
        gh = FakeGitHub()
        argv = [
            "--state",
            str(self.state),
            "--probe-result",
            "fail",
            "--now",
            "2026-11-18T23:00:00Z",
            "--format",
            "text",
            "--threshold",
            "1",
            "--apply",
        ]
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            ct.main(argv, github=gh.ops())
        lines = out.getvalue().strip().splitlines()
        self.assertEqual(len(lines), 1)
        self.assertTrue(
            lines[0].startswith("The repo-queue broker has been unreachable")
        )

    def test_command_probe_reports_exit_and_tail(self):
        ok, detail = ct.probe_command(
            [
                sys.executable,
                "-c",
                "import sys; sys.stderr.write('cannot connect to queue-broker'); sys.exit(3)",
            ],
            10,
        )
        self.assertFalse(ok)
        self.assertIn("exited 3", detail)
        self.assertIn("cannot connect to queue-broker", detail)

    def test_command_probe_timeout_is_a_failure(self):
        ok, detail = ct.probe_command(
            [sys.executable, "-c", "import time; time.sleep(5)"], 0.2
        )
        self.assertFalse(ok)
        self.assertIn("timed out", detail)

    def test_missing_probe_binary_is_a_failure(self):
        ok, detail = ct.probe_command(["/nonexistent/fleet", "queue", "health"], 5)
        self.assertFalse(ok)
        self.assertIn("could not start", detail)

    def test_tcp_probe(self):
        srv = socket.socket()
        srv.bind(("127.0.0.1", 0))
        srv.listen(1)
        port = srv.getsockname()[1]
        try:
            self.assertEqual(ct.probe_tcp(f"127.0.0.1:{port}", 2), (True, ""))
        finally:
            srv.close()
        ok, detail = ct.probe_tcp(f"127.0.0.1:{port}", 2)
        self.assertFalse(ok)
        self.assertIn("TCP connect", detail)

    def test_bad_probe_result_is_usage_error(self):
        with (
            self.assertRaises(SystemExit) as cm,
            contextlib.redirect_stderr(io.StringIO()),
        ):
            ct.main(["--state", str(self.state), "--probe-result", "maybe"])
        self.assertEqual(cm.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
