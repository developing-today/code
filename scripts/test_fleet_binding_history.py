"""Oracle: monotonic binding retention + honest check-in rendering.

No network, service, credentials, or non-stdlib dependencies are needed.
The collector's snapshot feeds scripts/fleet_completion_usage.py, whose
exact-sum oracle is reused end to end here.
"""

import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "fleet_binding_history", Path(__file__).with_name("fleet_binding_history.py")
)
collector = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(collector)
START = "2026-11-01T00:00:00Z"
END = "2026-11-02T00:00:00Z"
NOW = "2026-11-02T00:00:00Z"


def export(tasks=None, events=None, ledger=None, sessions=None):
    return {
        "tasks": tasks or [],
        "events": events or [],
        "ledger": ledger or [],
        "sessions": sessions or [],
    }


def task(task_id, queue="code", status="done", session=None):
    row = {"id": task_id, "queue": queue, "status": status, "progress": {}}
    if session is not None:
        row["progress"] = {"session_id": session}
    return row


def done_event(task_id, at=START):
    return {"task_id": task_id, "kind": "completed", "created_at": at}


def session(sid, tokens):
    return {"id": sid, "tokens": {"input": tokens}}


class BindingHistoryTest(unittest.TestCase):
    def history(self):
        return collector.blank_history()

    def record(self, history, data, at=NOW):
        return collector.record(history, data, at)

    def test_retry_survives_native_ledger_upsert(self):
        history = self.history()
        self.record(
            history,
            export(
                tasks=[task(1, status="claimed", session="s1")],
                ledger=[{"task_id": 1, "queue": "code", "session_id": "s1"}],
                sessions=[session("s1", 10)],
            ),
        )
        # Native retry: ledger upserts to the new session, progress follows.
        self.record(
            history,
            export(
                tasks=[task(1, session="retry")],
                events=[done_event(1)],
                ledger=[{"task_id": 1, "queue": "code", "session_id": "retry"}],
                sessions=[session("s1", 10), session("retry", 5)],
            ),
        )
        snap = collector.snapshot(history, START, END)
        bound = sorted(r["session_id"] for r in snap["ledger"])
        self.assertEqual(bound, ["retry", "s1"])
        self.assertTrue(snap["binding_history_complete"])

    def test_pruned_task_and_session_are_retained(self):
        history = self.history()
        self.record(
            history,
            export(
                tasks=[task(1, session="s1")],
                events=[done_event(1)],
                ledger=[{"task_id": 1, "queue": "code", "session_id": "s1"}],
                sessions=[session("s1", 10)],
            ),
        )
        # Later export pruned the task and its session entirely.
        self.record(history, export())
        snap = collector.snapshot(history, START, END)
        self.assertEqual(len(snap["ledger"]), 1)
        self.assertEqual(snap["sessions"], [{"id": "s1", "tokens": {"input": 10}}])

    def test_null_counter_never_overwrites_known(self):
        history = self.history()
        self.record(history, export(sessions=[session("s1", 10)]))
        self.record(history, export(sessions=[session("s1", None)]))
        self.assertEqual(history["sessions"]["s1"]["input"], 10)
        self.record(history, export(sessions=[session("s1", 40)]))
        self.assertEqual(history["sessions"]["s1"]["input"], 40)

    def test_first_seen_done_marks_history_gap(self):
        history = self.history()
        self.record(
            history,
            export(
                tasks=[task(1, session="s1")],
                events=[done_event(1)],
                sessions=[session("s1", 10)],
            ),
        )
        snap = collector.snapshot(history, START, END)
        self.assertFalse(snap["binding_history_complete"])
        self.assertEqual(snap["_collector"]["gap_task_ids"], [1])

    def test_late_retry_after_completion_marks_gap(self):
        history = self.history()
        self.record(
            history,
            export(
                tasks=[task(1, status="claimed", session="s1")],
                sessions=[session("s1", 10)],
            ),
        )
        self.record(
            history,
            export(tasks=[task(1, session="s1")], events=[done_event(1)]),
        )
        # A retry surfaces only after the task completed: we had missed it.
        self.record(
            history,
            export(
                tasks=[task(1, session="s1")],
                ledger=[{"task_id": 1, "queue": "code", "session_id": "late"}],
                sessions=[session("late", 3)],
            ),
        )
        snap = collector.snapshot(history, START, END)
        self.assertFalse(snap["binding_history_complete"])
        self.assertIn("late", [r["session_id"] for r in snap["ledger"]])

    def test_snapshot_feeds_usage_report_exact_sum(self):
        usage_spec = importlib.util.spec_from_file_location(
            "fleet_completion_usage",
            Path(__file__).with_name("fleet_completion_usage.py"),
        )
        usage = importlib.util.module_from_spec(usage_spec)
        usage_spec.loader.exec_module(usage)
        history = self.history()
        self.record(
            history,
            export(
                tasks=[task(1, status="claimed", session="s1")],
                sessions=[session("s1", 10)],
            ),
        )
        self.record(
            history,
            export(
                tasks=[task(1, session="retry")],
                events=[done_event(1)],
                ledger=[
                    {"task_id": 1, "queue": "code", "session_id": "s1"},
                    {"task_id": 1, "queue": "code", "session_id": "retry"},
                ],
                sessions=[session("s1", 10), session("retry", 5)],
            ),
        )
        snap = collector.snapshot(history, START, END)
        snap.pop("_collector")
        queue = usage.report(snap, START, END)["queues"][0]
        self.assertEqual(queue["known_input_tokens"], 15)
        self.assertEqual(queue["input_tokens_per_completed_task"], 15)
        self.assertEqual(queue["measured_tasks"], 1)

    def test_incomplete_session_withholds_ratio_downstream(self):
        usage_spec = importlib.util.spec_from_file_location(
            "fleet_completion_usage2",
            Path(__file__).with_name("fleet_completion_usage.py"),
        )
        usage = importlib.util.module_from_spec(usage_spec)
        usage_spec.loader.exec_module(usage)
        history = self.history()
        self.record(
            history,
            export(
                tasks=[task(1, status="claimed", session="s1")],
                sessions=[session("s1", 10)],
            ),
        )
        self.record(
            history,
            export(
                tasks=[task(1, session="s2")],
                events=[done_event(1)],
                ledger=[{"task_id": 1, "queue": "code", "session_id": "s2"}],
                sessions=[session("s2", None)],
            ),
        )
        snap = collector.snapshot(history, START, END)
        self.assertTrue(snap["binding_history_complete"])
        snap.pop("_collector")
        queue = usage.report(snap, START, END)["queues"][0]
        self.assertEqual(queue["missing_task_ids"], [1])
        self.assertIsNone(queue["input_tokens_per_completed_task"])

    def test_checkin_complete_renders_ratio(self):
        history = self.history()
        self.record(
            history,
            export(
                tasks=[task(1, status="claimed", session="s1")],
                sessions=[session("s1", 10)],
            ),
        )
        self.record(
            history,
            export(
                tasks=[task(1, session="s1")],
                events=[done_event(1)],
                sessions=[session("s1", 10)],
            ),
        )
        text = collector.checkin(history, START, END)
        self.assertIn("Retry-history complete: **True**", text)
        self.assertIn("input per completed task: 10.00", text)
        self.assertIn("missing tasks: none", text)
        self.assertIn("Do not infer window spend", text)

    def test_checkin_incomplete_withholds_and_names_gaps(self):
        history = self.history()
        self.record(
            history,
            export(
                tasks=[task(1, session="s1")],
                events=[done_event(1)],
                sessions=[session("s1", None)],
            ),
        )
        text = collector.checkin(history, START, END)
        self.assertIn("Retry-history complete: **False** (gap tasks: [1])", text)
        self.assertIn("WITHHELD (incomplete)", text)
        self.assertIn("missing tasks: 1", text)

    def test_conflicting_queue_is_an_error(self):
        history = self.history()
        self.record(history, export(tasks=[task(1, queue="code")]))
        with self.assertRaises(ValueError):
            self.record(history, export(tasks=[task(1, queue="other")]))

    def test_cli_round_trip(self):
        script = str(Path(__file__).with_name("fleet_binding_history.py"))
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            history_path, export_path, out_path = (
                tmp / "h.json",
                tmp / "e.json",
                tmp / "s.json",
            )
            export_path.write_text(
                json.dumps(
                    export(
                        tasks=[task(1, status="claimed", session="s1")],
                        sessions=[session("s1", 7)],
                    )
                )
            )
            script = str(Path(__file__).with_name("fleet_binding_history.py"))
            self.assertEqual(
                subprocess.run(
                    [
                        sys.executable,
                        "-I",
                        script,
                        "record",
                        f"--history={history_path}",
                        f"--export={export_path}",
                        f"--now={NOW}",
                    ],
                    capture_output=True,
                    text=True,
                ).returncode,
                0,
            )
            done = export(
                tasks=[task(1, session="s1")],
                events=[done_event(1)],
                sessions=[session("s1", 7)],
            )
            export_path.write_text(json.dumps(done))
            self.assertEqual(
                subprocess.run(
                    [
                        sys.executable,
                        "-I",
                        script,
                        "record",
                        f"--history={history_path}",
                        f"--export={export_path}",
                        f"--now={NOW}",
                    ],
                    capture_output=True,
                    text=True,
                ).returncode,
                0,
            )
            self.assertEqual(
                subprocess.run(
                    [
                        sys.executable,
                        "-I",
                        script,
                        "snapshot",
                        f"--history={history_path}",
                        f"--out={out_path}",
                        f"--start={START}",
                        f"--end={END}",
                    ],
                    capture_output=True,
                    text=True,
                ).returncode,
                0,
            )
            snap = json.loads(out_path.read_text())
            self.assertTrue(snap["binding_history_complete"])
            proc = subprocess.run(
                [
                    sys.executable,
                    "-I",
                    script,
                    "checkin",
                    f"--history={history_path}",
                    f"--start={START}",
                    f"--end={END}",
                ],
                capture_output=True,
                text=True,
            )
            self.assertEqual(proc.returncode, 0)
            self.assertIn("input per completed task: 7.00", proc.stdout)

    def test_unknown_flag_is_usage_error(self):
        script = str(Path(__file__).with_name("fleet_binding_history.py"))
        proc = subprocess.run(
            [sys.executable, "-I", script, "record", "--bogus"],
            capture_output=True,
            text=True,
        )
        self.assertEqual(proc.returncode, 2)


if __name__ == "__main__":
    unittest.main()
