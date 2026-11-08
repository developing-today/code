#!/usr/bin/env python3
"""Tests for claimant.py's launch and supervision steps (#6865 / task 11370).

Run:  python3 -m unittest pm-stack/opencode-claimant/test_claimant.py

Oracle: the documented claimant contract — a valid handed_off candidate
is launched (session created via the session API, task bound via the
repo-queue CLI) and reported in ; every candidate that cannot
launch is reported in  with its reason, and main() records those
skips in the fires.jsonl entry. A bound session that is gone from the
server with zero messages is a supervision recovery candidate.
All transport (subprocess: curl + repo-queue CLI) and poll discovery are
stubbed; nothing touches a server or the queue.
"""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

_spec = importlib.util.spec_from_file_location(
    "claimant", Path(__file__).resolve().parent / "claimant.py"
)
claimant = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(claimant)


class Result:
    def __init__(self, returncode=0, stdout="", stderr=""):
        self.returncode = returncode
        self.stdout = stdout
        self.stderr = stderr


def candidate(task_id=123, **over):
    c = {
        "id": task_id,
        "lane": "opencode",
        "session_id": None,
        "stage": "handed_off",
        "worker_id": "repo-worker-code",
        "issue": "#9999",
        "packet": {},
    }
    c.update(over)
    return c


class TestLaunchStep(unittest.TestCase):
    def run_launch(self, candidates, cap_room=1, create_stdout=None):
        calls = []

        def fake_run(argv, **kwargs):
            calls.append(argv)
            if "bind" in argv:
                return Result(0)
            if any("api/session" in str(a) for a in argv):
                body = create_stdout if create_stdout is not None else {
                    "id": "ses_test1",
                    "location": {"directory": str(claimant.PRIMARY_CLONE)},
                }
                return Result(0, json.dumps(body))
            return Result(0)

        with patch.object(claimant, "run_poll",
                          return_value={"candidates": candidates}), \
             patch.object(claimant.subprocess, "run", side_effect=fake_run):
            launched, skips = claimant.launch_step("live", cap_room)
        return launched, skips, calls

    def test_valid_handed_off_candidate_launches_and_binds(self):
        launched, skips, calls = self.run_launch([candidate()])
        self.assertEqual(skips, [])
        self.assertEqual(len(launched), 1)
        self.assertEqual(launched[0]["task_id"], 123)
        self.assertEqual(launched[0]["session_id"], "ses_test1")
        bind_calls = [a for a in calls if "bind" in a]
        self.assertEqual(len(bind_calls), 1)
        self.assertIn("123", bind_calls[0])
        self.assertIn("ses_test1", bind_calls[0])

    def test_wrong_location_session_is_skipped_not_launched(self):
        launched, skips, _ = self.run_launch(
            [candidate()],
            create_stdout={"id": "ses_bad",
                           "location": {"directory": "/somewhere/else"}},
        )
        self.assertEqual(launched, [])
        self.assertEqual([s["reason"] for s in skips], ["session_wrong_location"])

    def test_cap_saturation_is_recorded_as_a_skip(self):
        launched, skips, _ = self.run_launch(
            [candidate(1), candidate(2)], cap_room=1)
        self.assertEqual(len(launched), 1)
        self.assertEqual([s["reason"] for s in skips], ["cap_saturated"])
        self.assertEqual(skips[0]["task_id"], 2)

    def test_no_cap_room_launches_and_skips_nothing(self):
        launched, skips, _ = self.run_launch([candidate()], cap_room=0)
        self.assertEqual((launched, skips), ([], []))

    def test_main_records_launch_skips_in_the_fire_entry(self):
        with tempfile.TemporaryDirectory() as d:
            state = Path(d)
            with patch.object(claimant, "STATE_DIR", state), \
                 patch.object(claimant, "FIRES_FILE", state / "fires.jsonl"), \
                 patch.object(claimant, "read_mode", return_value="live"), \
                 patch.object(claimant, "supervision_step", return_value=[]), \
                 patch.object(claimant, "gate_step", return_value=True), \
                 patch.object(claimant, "cap_step", return_value=1), \
                 patch.object(claimant, "launch_step",
                              return_value=([], [{"task_id": 9, "reason": "bind_failed"}])):
                rc = claimant.main()
            self.assertEqual(rc, 0)
            entry = json.loads((state / "fires.jsonl").read_text().splitlines()[-1])
            self.assertEqual(entry["skips"], [{"task_id": 9, "reason": "bind_failed"}])


class TestSupervisionStep(unittest.TestCase):
    TASK = {
        "id": 555,
        "status": "claimed",
        "attempts": 1,
        "max_attempts": 5,
        "progress": {"lane": "opencode", "session_id": "ses_gone"},
    }

    def run_supervision(self, mode="live"):
        calls = []

        def fake_run(argv, **kwargs):
            calls.append(argv)
            argv_s = [str(a) for a in argv]
            if "list" in argv_s:
                queue = argv_s[argv_s.index("--queue") + 1]
                rows = [self.TASK] if queue == "repo-health-code-pilot" else []
                return Result(0, json.dumps({"rows": rows}))
            if "DELETE" in argv_s:
                return Result(0)
            if any("/messages" in a for a in argv_s):
                return Result(0, "[]")
            if any("api/session" in a for a in argv_s):
                return Result(1, "")  # session gone from the server
            if "bind" in argv_s:
                return Result(0)
            return Result(0)

        with patch.object(claimant.subprocess, "run", side_effect=fake_run):
            recovered = claimant.supervision_step(mode)
        return recovered, calls

    def test_gone_zero_message_session_is_recovered(self):
        recovered, calls = self.run_supervision()
        self.assertEqual(
            recovered,
            [{"task_id": 555, "session_id": "ses_gone", "reason": "zero_message"}],
        )
        bind_calls = [a for a in calls if "bind" in [str(x) for x in a]]
        self.assertEqual(len(bind_calls), 1)
        flat = [str(x) for x in bind_calls[0]]
        self.assertIn("555", flat)
        self.assertIn(claimant.LAUNCHED_BY_RECOVERY_ZERO_MESSAGE, flat)

    def test_shadow_mode_recovers_nothing(self):
        recovered, calls = self.run_supervision(mode="shadow")
        self.assertEqual(recovered, [])
        self.assertEqual(calls, [])


if __name__ == "__main__":
    unittest.main()
