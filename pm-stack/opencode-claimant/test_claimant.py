#!/usr/bin/env python3
"""Tests for claimant.py's live launch and supervision steps (#6865).

Run:  python3 -m unittest pm-stack/opencode-claimant/test_claimant.py

The transport (curl to the OpenCode server) and the queue CLI
(repo-queue) are stubbed at subprocess.run, poll discovery is
stubbed at claimant.run_poll, and supervision's claimed rows are
stubbed at the poll module's fetch_claimed_rows, so the suite
exercises the real step logic with no server and no queue:

- launch_step launches, binds and prompts a valid handed_off
  candidate; a prompt that cannot be landed flags the launch entry
  instead of reading as a clean launch;
- launch_step in shadow mode mutates nothing and records the
  would-launch window as skips (#7366);
- launch_step records a skip (not silence) when a candidate's state
  changes before launch, and main() writes those skips into the
  fires.jsonl entry;
- supervision_step recovers a bound task whose session is gone from
  the server (error body, zero messages) via delete + recovery bind;
- supervision_step treats an unreadable message read as UNKNOWN, not
  zero: no delete, no rebind, and a session that does have messages
  is never recovered; recovery also requires an expired lease (#3026).

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


def make_fake_run(binds, *, session_get="gone", messages="[]",
                  messages_rc=0, prompt_rc=0, prompts=None, deletes=None):
    """Build a subprocess.run stub dispatching on argv shape.

    binds: list collecting every repo-queue bind argv.
    session_get: "gone" -> the session GET returns an error body
    (what curl -s prints for a 404); "active" -> a live session.
    messages: body of the message read (GET /message).
    messages_rc: return code for the message read; non-zero models a
    non-2xx body (curl -f fails, as the server's 404 did).
    prompt_rc: return code for the prompt POST (curl -f fails on
    HTTP errors, so 1 models a rejected prompt send).
    prompts: optional list collecting every prompt POST argv.
    deletes: optional list collecting every session DELETE argv.
    """

    def fake_run(cmd, **kwargs):
        argv = [str(part) for part in cmd]
        joined = " ".join(argv)
        if "repo-queue" in argv[0] and "bind" in argv:
            binds.append(cmd)
            return Result(0, "{}")
        if argv[0] == "curl" and "-X" in argv and "DELETE" in argv:
            if deletes is not None:
                deletes.append(cmd)
            return Result(0, "")
        if argv[0] == "curl" and joined.rstrip().endswith("/message"):
            return Result(messages_rc, messages)
        if argv[0] == "curl" and joined.rstrip().endswith("/prompt"):
            if prompts is not None:
                prompts.append(cmd)
            return Result(prompt_rc, "")
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


BOUND_ROW = {
    "id": 4242,
    "attempts": 0,
    "max_attempts": 5,
    "lease_until": "2020-01-01T00:00:00Z",
    "progress": {"lane": "opencode", "session_id": "ses_gone"},
}


def stub_claimed_rows(rows):
    """Patch the poll module's broker read (supervision's row source)
    to serve `rows` for the first queue and [] for the rest."""
    poll_mod = claimant._load_poll_module()

    def fake_fetch(queue):
        return list(rows) if queue == claimant.QUEUES[0] else []

    return patch.object(poll_mod, "fetch_claimed_rows",
                        side_effect=fake_fetch)


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

    def test_parked_candidate_is_collected_without_naming_its_id(self):
        # #8097 Rule 1: a parked handoff candidate is collected via
        # the broker-picked handoff claim — no argv ever names the
        # task id — and then launches exactly like a claimed one.
        binds = []
        claims = []
        base_fake = make_fake_run(binds)

        def fake_run(cmd, **kwargs):
            argv = [str(part) for part in cmd]
            if "repo-queue" in argv[0] and "claim" in argv:
                claims.append(argv)
                return Result(0, json.dumps({"id": 888, "status": "claimed"}))
            return base_fake(cmd, **kwargs)

        parked = dict(CANDIDATE, id=888, status="parked",
                      queue=claimant.QUEUES[0])
        with patch.object(claimant, "run_poll",
                          return_value={"candidates": [dict(parked)]}), \
             patch.object(claimant.subprocess, "run", fake_run):
            launched, skips = claimant.launch_step("live", 3)
        self.assertEqual(skips, [])
        self.assertEqual(len(launched), 1)
        self.assertEqual(launched[0]["task_id"], 888)
        self.assertEqual(len(claims), 1)
        claim_argv = claims[0]
        self.assertIn("--handoff", claim_argv)
        self.assertIn("--platform", claim_argv)
        self.assertIn("openchamber", claim_argv)
        self.assertNotIn("--task", claim_argv)
        self.assertNotIn("888", claim_argv)

    def test_collect_parked_handoffs_drains_until_the_broker_has_none(self):
        # The collection loop claims broker-picked handoffs until
        # the broker answers {"task": null}; every claim is a
        # no-id handoff claim under the openchamber family.
        rows = [Result(0, json.dumps({"id": 501, "status": "claimed"})),
                Result(0, json.dumps({"id": 502, "status": "claimed"})),
                Result(0, json.dumps({"task": None}))]
        argvs = []

        def fake_run(cmd, **kwargs):
            argvs.append([str(part) for part in cmd])
            return rows[len(argvs) - 1]

        with patch.object(claimant.subprocess, "run", fake_run):
            collected = claimant._collect_parked_handoffs("q1", 5)
        self.assertEqual(sorted(collected), [501, 502])
        self.assertEqual(len(argvs), 3)
        for argv in argvs:
            self.assertIn("--handoff", argv)
            self.assertNotIn("--task", argv)

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

    def test_shadow_mode_launches_nothing(self):
        # #7366: a shadow fire must not create, bind or prompt — the
        # candidate is reported as a would-launch skip only.
        binds = []
        with patch.object(claimant, "run_poll",
                          return_value={"candidates": [dict(CANDIDATE)]}), \
             patch.object(claimant.subprocess, "run", make_fake_run(binds)):
            launched, skips = claimant.launch_step("shadow", 3)
        self.assertEqual(launched, [])
        self.assertEqual(skips, [{"task_id": 777,
                                  "reason": "shadow_would_launch"}])
        self.assertEqual(binds, [])

    def test_failed_prompt_flags_the_launch(self):
        # #7356: the prompt POST is rejected and no message landed —
        # the launch is recorded, but flagged, never read as clean.
        binds = []
        with patch.object(claimant, "run_poll",
                          return_value={"candidates": [dict(CANDIDATE)]}), \
             patch.object(claimant.subprocess, "run",
                          make_fake_run(binds, prompt_rc=1)):
            launched, skips = claimant.launch_step("live", 3)
        self.assertEqual(skips, [])
        self.assertEqual(len(launched), 1)
        self.assertEqual(launched[0].get("prompt_failed"), True)
        self.assertEqual(len(binds), 1)

    def test_landed_prompt_despite_post_error_is_clean(self):
        # The POST's response was lost but the message landed: the
        # reconcile read sees it and no resend/flag happens.
        binds = []
        with patch.object(claimant, "run_poll",
                          return_value={"candidates": [dict(CANDIDATE)]}), \
             patch.object(claimant.subprocess, "run",
                          make_fake_run(binds, prompt_rc=1,
                                        messages='[{"id": "m1"}]')):
            launched, skips = claimant.launch_step("live", 3)
        self.assertEqual(skips, [])
        self.assertEqual(len(launched), 1)
        self.assertNotIn("prompt_failed", launched[0])

    def test_unreadable_session_is_never_resent(self):
        # The POST failed and the reconcile read is unusable (error
        # body, not a message list): delivery state is unknown, so
        # no second POST may go out — the worker might be executing
        # the first one. The launch is flagged instead.
        binds = []
        prompts = []
        with patch.object(claimant, "run_poll",
                          return_value={"candidates": [dict(CANDIDATE)]}), \
             patch.object(claimant.subprocess, "run",
                          make_fake_run(binds, prompt_rc=1,
                                        messages='{"error": "boom"}',
                                        prompts=prompts)):
            launched, skips = claimant.launch_step("live", 3)
        self.assertEqual(skips, [])
        self.assertEqual(len(launched), 1)
        self.assertEqual(launched[0].get("prompt_failed"), True)
        self.assertEqual(len(prompts), 1)


class TestSupervisionStep(unittest.TestCase):
    def test_gone_zero_message_session_is_recovered(self):
        binds = []
        with stub_claimed_rows([BOUND_ROW]), \
             patch.object(claimant.subprocess, "run", make_fake_run(binds)):
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
        with stub_claimed_rows([BOUND_ROW]), \
             patch.object(claimant.subprocess, "run",
                          make_fake_run(binds, session_get="active")):
            recovered = claimant.supervision_step("live")
        self.assertEqual(recovered, [])
        self.assertEqual(binds, [])

    def test_unreadable_messages_are_unknown_not_zero(self):
        # #3026: the old endpoint 404s with an empty body, and the old
        # read parsed that as []. Unknown must leave the task bound:
        # no DELETE, no recovery bind, no candidate.
        for rc, body in ((1, ""), (0, "")):
            with self.subTest(returncode=rc, body=body):
                binds = []
                deletes = []
                with stub_claimed_rows([BOUND_ROW]), \
                     patch.object(claimant.subprocess, "run",
                                  make_fake_run(binds, messages=body,
                                                messages_rc=rc,
                                                deletes=deletes)):
                    recovered = claimant.supervision_step("live")
                self.assertEqual(recovered, [])
                self.assertEqual(binds, [])
                self.assertEqual(deletes, [])

    def test_wrapped_empty_message_list_is_recovered(self):
        # The live 200 shape: {"data": [...], "cursor": {...}}. A
        # dict wrapping a confirmed empty list is still zero messages.
        binds = []
        with stub_claimed_rows([BOUND_ROW]), \
             patch.object(claimant.subprocess, "run",
                          make_fake_run(binds,
                                        messages='{"data": [], "cursor": {}}')):
            recovered = claimant.supervision_step("live")
        self.assertEqual(recovered, [{
            "task_id": 4242,
            "session_id": "ses_gone",
            "reason": "zero_message",
        }])
        self.assertEqual(len(binds), 1)

    def test_bound_session_with_messages_is_not_recovered(self):
        binds = []
        deletes = []
        with stub_claimed_rows([BOUND_ROW]), \
             patch.object(claimant.subprocess, "run",
                          make_fake_run(binds, messages='[{"id": "m1"}]',
                                        deletes=deletes)):
            recovered = claimant.supervision_step("live")
        self.assertEqual(recovered, [])
        self.assertEqual(binds, [])
        self.assertEqual(deletes, [])

    def test_unexpired_lease_is_not_recovered(self):
        # A live lease is still owned by a worker: recovery would
        # double-launch over it even when the session looks gone.
        row = dict(BOUND_ROW, lease_until="2999-01-01T00:00:00Z")
        binds = []
        deletes = []
        with stub_claimed_rows([row]), \
             patch.object(claimant.subprocess, "run",
                          make_fake_run(binds, deletes=deletes)):
            recovered = claimant.supervision_step("live")
        self.assertEqual(recovered, [])
        self.assertEqual(binds, [])
        self.assertEqual(deletes, [])

    def test_unparseable_lease_is_not_recovered(self):
        # A lease we cannot read is a fact we do not have; it never
        # counts as expired.
        row = dict(BOUND_ROW, lease_until="not-a-timestamp")
        binds = []
        deletes = []
        with stub_claimed_rows([row]), \
             patch.object(claimant.subprocess, "run",
                          make_fake_run(binds, deletes=deletes)):
            recovered = claimant.supervision_step("live")
        self.assertEqual(recovered, [])
        self.assertEqual(binds, [])
        self.assertEqual(deletes, [])

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
                 stub_claimed_rows([]), \
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
