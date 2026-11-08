#!/usr/bin/env python3
"""Tests for claimant.py's live launch and supervision steps (#6865).

Run:  python3 -m unittest pm-stack/opencode-claimant/test_claimant.py

The transport (curl to the OpenCode server) and the queue CLI
(repo-queue) are stubbed at subprocess.run, and poll discovery is
stubbed at claimant.run_poll, so the suite exercises the real step
logic with no server and no queue:

- launch_step launches and binds a valid handed_off candidate;
- launch_step records a skip (not silence) when a candidate's state
  changes before launch, and main() writes those skips into the
  fires.jsonl entry;
- supervision_step recovers a bound task whose session is gone from
  the server (error body, zero messages) via delete + recovery bind.

Negative control (#6865 acceptance): this suite fails against the
pre-fix claimant.py (65beb42c1a, unimported requests/subprocess) and
against the 307-line Slice-1 shadow stub that replaced it
(d0703e82ce) — neither can launch or recover anything in live mode.
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
    """Minimal subprocess.run stand-in."""

    def __init__(self, returncode=0, stdout="", stderr=""):
        self.returncode = returncode
        self.stdout = stdout
        self.stderr = stderr


def make_fake_run(binds, *, session_get="gone", messages="[]"):
    """Build a subprocess.run stub dispatching on argv shape.

    binds: list collecting every repo-queue bind argv.
    session_get: "gone" -> the session GET returns an error body
    (what curl -s prints for a 404); "active" -> a live session.
    """

    def fake_run(cmd, **kwargs):
        argv = [str(part) for part in cmd]
        joined = " ".join(argv)
        if "repo-queue" in argv[0] and "list" in argv:
            queue = argv[argv.index("--queue") + 1]
            if queue == claimant.QUEUES[0]:
                rows = [{
                    "id": 4242,
                    "attempts": 0,
                    "max_attempts": 5,
                    "progress": {"lane": "opencode", "session_id": "ses_gone"},
                }]
            else:
                rows = []
            return Result(0, json.dumps(rows))
        if "repo-queue" in argv[0] and "bind" in argv:
            binds.append(cmd)
            return Result(0, "{}")
        if argv[0] == "curl" and "-X" in argv and "DELETE" in argv:
            return Result(0, "")
        if argv[0] == "curl" and joined.rstrip().endswith("/messages"):
            return Result(0, messages)
        if argv[0] == "curl" and "-X" in argv and "POST" in argv:
            return Result(0, json.dumps({
                "id": "ses_new1",
                "location": {"directory": str(claimant.PRIMARY_CLONE)},
            }))
        if argv[0] == "curl" and "/api/session/" in joined:
            if session_get == "gone":
                return Result(0, json.dumps({"error": "session not found"}))
            return Result(0, json.dumps({
                "id": "ses_gone",
                "time": {"created": 1, "updated": 2},
            }))
        return Result(0, "")

    return fake_run


CANDIDATE = {
    "id": 777,
    "lane": "opencode",
    "session_id": None,
    "stage": "handed_off",
    "worker_id": "repo-worker-code",
    "issue": "#1234",
    "packet": {},
}


class TestLaunchStep(unittest.TestCase):
    def test_valid_candidate_launches_and_binds(self):
        binds = []
        with patch.object(claimant, "run_poll",
                          return_value={"candidates": [dict(CANDIDATE)]}), \
             patch.object(claimant.subprocess, "run", make_fake_run(binds)):
            launched, skips = claimant.launch_step("live", 3)
        self.assertEqual(skips, [])
        self.assertEqual(len(launched), 1)
        self.assertEqual(launched[0]["task_id"], 777)
        self.assertEqual(launched[0]["session_id"], "ses_new1")
        self.assertEqual(len(binds), 1)
        bind_argv = binds[0]
        # Real subprocess.run rejects non-str argv: every part is a str
        # and the task id arrives stringified.
        self.assertTrue(all(isinstance(part, str) for part in bind_argv))
        self.assertIn("777", bind_argv)
        self.assertIn("ses_new1", bind_argv)

    def test_vanished_candidate_is_a_recorded_skip(self):
        calls = {"n": 0}

        def poll_stub():
            calls["n"] += 1
            if calls["n"] == 1:
                return {"candidates": [dict(CANDIDATE)]}
            return {"candidates": []}

        with patch.object(claimant, "run_poll", side_effect=poll_stub), \
             patch.object(claimant.subprocess, "run", make_fake_run([])):
            launched, skips = claimant.launch_step("live", 3)
        self.assertEqual(launched, [])
        self.assertEqual(skips, [{"task_id": 777, "reason": "task_state_changed"}])


class TestSupervisionStep(unittest.TestCase):
    def test_gone_zero_message_session_is_recovered(self):
        binds = []
        with patch.object(claimant.subprocess, "run", make_fake_run(binds)):
            recovered = claimant.supervision_step("live")
        self.assertEqual(recovered, [{
            "task_id": 4242,
            "session_id": "ses_gone",
            "reason": "zero_message",
        }])
        self.assertEqual(len(binds), 1)
        bind_argv = [str(part) for part in binds[0]]
        self.assertIn(claimant.LAUNCHED_BY_RECOVERY_ZERO_MESSAGE, bind_argv)
        self.assertIn("4242", bind_argv)

    def test_active_session_is_left_bound(self):
        binds = []
        with patch.object(claimant.subprocess, "run",
                          make_fake_run(binds, session_get="active")):
            recovered = claimant.supervision_step("live")
        self.assertEqual(recovered, [])
        self.assertEqual(binds, [])

    def test_shadow_mode_recovers_nothing(self):
        with patch.object(claimant.subprocess, "run", make_fake_run([])):
            self.assertEqual(claimant.supervision_step("shadow"), [])


class TestMainFireLog(unittest.TestCase):
    def test_main_records_launch_skips_in_fires_entry(self):
        calls = {"n": 0}

        def poll_stub():
            calls["n"] += 1
            # gate + launch discovery see the candidate; the launch
            # re-verification finds it gone -> a recorded skip.
            if calls["n"] <= 2:
                return {"candidates": [dict(CANDIDATE)]}
            return {"candidates": []}

        with tempfile.TemporaryDirectory() as tmp:
            state_dir = Path(tmp)
            fires = state_dir / "fires.jsonl"
            with patch.object(claimant, "read_mode", return_value="live"), \
                 patch.object(claimant, "run_poll", side_effect=poll_stub), \
                 patch.object(claimant, "get_active_root_session_count",
                              return_value=0), \
                 patch.object(claimant.subprocess, "run", make_fake_run([])), \
                 patch.object(claimant, "STATE_DIR", state_dir), \
                 patch.object(claimant, "FIRES_FILE", fires):
                self.assertEqual(claimant.main(), 0)
            entries = [json.loads(line)
                       for line in fires.read_text().splitlines() if line]
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0]["sessions_launched"], [])
        self.assertEqual(entries[0]["skips"],
                         [{"task_id": 777, "reason": "task_state_changed"}])


if __name__ == "__main__":
    unittest.main()
