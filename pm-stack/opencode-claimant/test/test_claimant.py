#!/usr/bin/env python3
"""Offline tests for claimant ordering and fail-closed behavior.

Ported to the live claimant API for #7670 (task 14501). The Slice-1
suite at this path patched a claimant API that no longer exists
(prepare_candidate / launch_one / _POLL / _current_bound_row /
_poll-shaped supervision_step(data, mode)) and errored 4/4 while no
workflow ran it. This port pins the surviving behaviors against the
live surface - launch_step(mode, cap_room) and supervision_step(mode):

- bind precedes prompt: a session is created, then bound to the task,
  then prompted (the Slice-1 checkpoint-between-bind-and-prompt pin is
  superseded: the live flow has no checkpoint step; prompt delivery is
  verified by _send_prompt's reconcile-before-resend instead);
- a credential-shaped packet value never enters the prompt text (the
  live claimant redacts credential-shaped keys when it dumps the
  packet into the prompt);
- a held issue label (held / user/owned / do-not-merge /
  do-not-auto-edit) prevents launch, and an unreadable label set
  fails closed the same way - this gate was genuinely missing in the
  live claimant and is implemented in launch_step for this issue;
- session-create failure binds nothing and is a recorded skip;
- supervision: a confirmed zero-message read precedes the session
  DELETE, which precedes the atomic recovery bind carrying
  LAUNCHED_BY_RECOVERY_ZERO_MESSAGE.

The Slice-1 per-packet route/model pin is superseded: the live
claimant launches every session on the fixed workhorse route.
"""

import copy
import importlib.util
import json
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("claimant", ROOT / "claimant.py")
claimant = importlib.util.module_from_spec(spec)
spec.loader.exec_module(claimant)


class Result:
    """Minimal subprocess.run stand-in."""

    def __init__(self, returncode=0, stdout="", stderr=""):
        self.returncode = returncode
        self.stdout = stdout
        self.stderr = stderr


def make_fake_run(events, *, create_rc=0, messages="[]"):
    """subprocess.run stub dispatching on argv shape, recording an
    ordered event log: ("create",) / ("bind", argv) / ("prompt", argv)
    / ("messages",) / ("delete",) / ("session_get",).

    Stub-shape audit (#7772, #7770 Phase 4): every curl reply mirrors
    the writer it stands in for — the claimant's one transport
    (`_server_request`) invokes curl with `-w "\\n%{http_code}"`, so a
    real reply is the body plus a trailing status line, and curl -s
    exits 0 even on a 401/404. This stub therefore appends the status
    line to every curl body and answers rc 0 for HTTP errors; only a
    transport-level create failure uses create_rc. (The pre-audit stub
    returned bare bodies with no status line, which the verified
    transport correctly read as UNKNOWN — the reader's-assumption
    shape #7760 classed as a defect.)"""

    def reply(body, status):
        return Result(0, body + "\n" + str(status))

    def fake_run(cmd, **kwargs):
        argv = [str(part) for part in cmd]
        joined = " ".join(argv)
        if "repo-queue" in argv[0] and "bind" in argv:
            events.append(("bind", argv))
            return Result(0, "{}")
        if argv[0] == "curl" and "-X" in argv and "DELETE" in argv:
            events.append(("delete",))
            return reply("", 200)
        if argv[0] == "curl" and joined.rstrip().endswith("/message"):
            events.append(("messages",))
            return reply(messages, 200)
        if argv[0] == "curl" and joined.rstrip().endswith("/prompt"):
            events.append(("prompt", argv))
            return reply("", 200)
        if argv[0] == "curl" and "-X" in argv and "POST" in argv:
            events.append(("create",))
            if create_rc != 0:
                return Result(create_rc, "", "server unavailable")
            return reply(
                json.dumps(
                    {
                        "id": "ses_new1",
                        "location": {"directory": str(claimant.PRIMARY_CLONE)},
                    }
                ),
                200,
            )
        if argv[0] == "curl" and "/api/session/" in joined:
            events.append(("session_get",))
            # The v2 server's shape for a missing session: a 404 with
            # an error body (the "gone" verdict's only lawful source).
            return reply(json.dumps({"error": "session not found"}), 404)
        return Result(0, "")

    return fake_run


CANDIDATE = {
    "id": 73001,
    "queue": "repo-health-code-pilot",
    "status": "claimed",
    "priority": 1,
    "worker_id": "repo-worker-code",
    "lane_id": "code-a",
    "lease_until": "2999-01-01T00:00:00Z",
    "lane": "opencode",
    "stage": "handed_off",
    "session_id": None,
    "issue": 3026,
    "issue_number": 3026,
    "packet": {
        "repo": "developing-today/code",
        "issue": {"number": 3026},
        "route": {"providerID": "opencode-route", "modelID": "summit"},
        "password": "must-not-enter-prompt",
        "nested": {"api_token": "must-not-enter-prompt-either"},
    },
    "attempts": 0,
    "max_attempts": 3,
    "title": "Queue task 73001",
    "dedupe_key": "test",
}


def run_launch(events, *, labels=set(), create_rc=0):
    with (
        patch.object(
            claimant,
            "run_poll",
            side_effect=lambda: {"candidates": [copy.deepcopy(CANDIDATE)]},
        ),
        patch.object(claimant, "_github_issue_labels", return_value=set(labels)),
        patch.object(
            claimant.subprocess, "run", make_fake_run(events, create_rc=create_rc)
        ),
    ):
        return claimant.launch_step("live", 3)


class TestLaunch(unittest.TestCase):
    def test_session_is_bound_before_prompt_and_password_stays_out(self):
        events = []
        launched, skips = run_launch(events)
        self.assertEqual(skips, [])
        self.assertEqual(len(launched), 1)
        self.assertEqual(launched[0]["session_id"], "ses_new1")
        kinds = [e[0] for e in events]
        self.assertLess(kinds.index("create"), kinds.index("bind"))
        self.assertLess(kinds.index("bind"), kinds.index("prompt"))
        prompt_argv = next(e[1] for e in events if e[0] == "prompt")
        body = json.loads(next(a for a in prompt_argv if a.startswith("{")))
        prompt = body["text"]
        self.assertIn("73001", prompt)
        self.assertIn("repo-worker-code", prompt)
        self.assertIn("ses_new1", prompt)
        self.assertNotIn("must-not-enter-prompt", prompt)
        self.assertIn("[redacted]", prompt)

    def test_create_failure_binds_nothing(self):
        events = []
        launched, skips = run_launch(events, create_rc=1)
        self.assertEqual(launched, [])
        self.assertEqual(len(skips), 1)
        self.assertEqual(skips[0]["task_id"], 73001)
        self.assertTrue(skips[0]["reason"].startswith("launch_error"))
        self.assertNotIn("bind", [e[0] for e in events])

    def test_held_issue_label_prevents_launch(self):
        events = []
        launched, skips = run_launch(events, labels={"do-not-merge"})
        self.assertEqual(launched, [])
        self.assertEqual(len(skips), 1)
        self.assertTrue(skips[0]["reason"].startswith("held_issue_labels"))
        self.assertIn("do-not-merge", skips[0]["reason"])
        self.assertNotIn("create", [e[0] for e in events])
        self.assertNotIn("bind", [e[0] for e in events])

    def test_unreadable_labels_fail_closed(self):
        events = []
        with (
            patch.object(
                claimant,
                "run_poll",
                side_effect=lambda: {"candidates": [copy.deepcopy(CANDIDATE)]},
            ),
            patch.object(
                claimant, "_github_issue_labels", side_effect=RuntimeError("gh down")
            ),
            patch.object(claimant.subprocess, "run", make_fake_run(events)),
        ):
            launched, skips = claimant.launch_step("live", 3)
        self.assertEqual(launched, [])
        self.assertEqual(len(skips), 1)
        self.assertTrue(skips[0]["reason"].startswith("held_label_check_failed"))
        self.assertNotIn("create", [e[0] for e in events])


BOUND_ROW = {
    "id": 4242,
    "attempts": 0,
    "max_attempts": 5,
    "lease_until": "2020-01-01T00:00:00Z",
    "progress": {"lane": "opencode", "session_id": "ses_gone"},
}


class TestSupervision(unittest.TestCase):
    def test_zero_message_delete_precedes_atomic_recovery(self):
        events = []
        poll_mod = claimant._load_poll_module()

        def fake_fetch(queue):
            return [dict(BOUND_ROW)] if queue == claimant.QUEUES[0] else []

        with (
            patch.object(poll_mod, "fetch_claimed_rows", side_effect=fake_fetch),
            patch.object(claimant.subprocess, "run", make_fake_run(events)),
        ):
            recovered = claimant.supervision_step("live")

        self.assertEqual(
            recovered,
            [
                {
                    "task_id": 4242,
                    "session_id": "ses_gone",
                    "reason": "zero_message",
                }
            ],
        )
        kinds = [e[0] for e in events]
        self.assertLess(kinds.index("messages"), kinds.index("delete"))
        self.assertLess(kinds.index("delete"), kinds.index("bind"))
        bind_argv = next(e[1] for e in events if e[0] == "bind")
        self.assertIn(claimant.LAUNCHED_BY_RECOVERY_ZERO_MESSAGE, bind_argv)


if __name__ == "__main__":
    unittest.main()
