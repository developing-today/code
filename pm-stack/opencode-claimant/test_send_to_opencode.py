#!/usr/bin/env python3
"""Tests for send_to_opencode.py, the Muse→opencode push (#8210).

Run:  python3 -m unittest pm-stack/opencode-claimant/test_send_to_opencode.py

Everything external is stubbed at the send module's own seams:
poll's broker fetches serve fixture rows, claimant's cap/mode/
transport functions are patched on the module objects the send
module loaded, the repo-queue CLI is launch_one.queue behind the
send module's queue_json, and gh is reached only through
launch_one.hard_stop_reason / post_agent_note. No server, broker,
or GitHub is touched.

Covered:
- validation gates: no reason / no notes / no from-session -> the
  send refuses before anything is read or mutated (#8187: a
  pass-off without a stated reason does not happen);
- the sender must HOLD the task, and hard-stop labels refuse;
- the primary checkout itself is refused as the worktree;
- happy path: assignment recorded with the reason, sender
  checkpoint carries from_session, session created in the
  worktree, the transfer claim binds it (--from-session +
  --session-id), the prompt carries the sender notes and the
  worktree, and the notes file lands in the worktree;
- a failed transfer deletes the created session and flips the
  assignment back to direct (the sender keeps the item);
- a failed assignment creates no session at all;
- a prompt failure after a successful bind does NOT roll back
  (the bound session is supervision's, claimant rule) and the
  record says so.
"""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

_spec = importlib.util.spec_from_file_location(
    "send_to_opencode", Path(__file__).resolve().parent / "send_to_opencode.py"
)
send = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(send)

poll = send.poll
claimant = send.claimant
launch_one = send.launch_one
QUEUE = "repo-health-code-pilot"
PRIMARY = "/home/user/git/code"
SENDER = "repo-worker-code-m"
FROM_SESSION = "11111111-2222-3333-4444-555555555555"
NEW_SESSION = "ses_new_8210"
NOTES = "Fix the widget counter first; the flaky test is a red herring."


def sender_row(**over):
    row = {
        "id": 777, "queue": QUEUE, "status": "claimed",
        "title": "Fix the thing",
        "dedupe_key": "repo-health:v1:fix:developing-today/code:issue:4242",
        "priority": 10, "attempts": 1, "max_attempts": 5,
        "worker_id": SENDER, "lane_id": "code-m",
        "progress": {"lane": "direct"},
        "payload": {"repo": "developing-today/code", "issue": {"number": 4242}},
    }
    row.update(over)
    return row


def serve_claimed(rows):
    def fetch_claimed(queue):
        return list(rows) if queue == QUEUE else []

    return patch.object(poll, "fetch_claimed_rows", fetch_claimed)


def live_and_room(room=3):
    return patch.multiple(
        claimant, read_mode=lambda: "live", cap_step=lambda: room
    )


def quiet_logs():
    return patch.multiple(
        send,
        log=lambda rec: rec,
        journal=lambda rec: None,
    )


class FakeQueue:
    """launch_one.queue stand-in dispatching on the verb."""

    def __init__(self, claim_rc=0, assign_lane="opencode"):
        self.calls = []
        self.claim_rc = claim_rc
        self.assign_lane = assign_lane

    def __call__(self, *args, timeout=120):
        self.calls.append(args)
        verb = args[0]
        if verb == "lane":
            if "--opencode-reason" in args:
                return 0, json.dumps({"assigned_lane": self.assign_lane}), ""
            return 0, json.dumps({"assigned_lane": "direct"}), ""
        if verb == "checkpoint":
            return 0, "{}", ""
        if verb == "claim":
            if self.claim_rc != 0:
                return self.claim_rc, "", "transfer refused: holder mismatch"
            sid = args[args.index("--session-id") + 1]
            row = sender_row(
                worker_id="opencode-claimant",
                progress={"lane": "opencode", "session_id": sid},
            )
            return 0, json.dumps({"task": row}), ""
        return 0, "{}", ""

    def verbs(self):
        return [c[0] for c in self.calls]

    def claim_args(self):
        for c in self.calls:
            if c[0] == "claim":
                return c
        return None


def fake_server(worktree, prompt_ok=True, create_directory=None):
    deleted = []
    prompts = []

    def server_request(method, path, body=None, timeout=5):
        return 201, {
            "id": NEW_SESSION,
            "location": {"directory": create_directory or worktree},
        }

    def send_prompt(sid, prompt):
        prompts.append(prompt)
        return prompt_ok

    patches = patch.multiple(
        claimant,
        _server_request=server_request,
        _send_prompt=send_prompt,
        _delete_session=lambda sid: deleted.append(sid) or True,
    )
    return patches, deleted, prompts


def run_send(worktree, fq=None, server_kw=None, **send_kw):
    fq = fq or FakeQueue()
    server_patches, deleted, prompts = fake_server(worktree, **(server_kw or {}))
    kw = dict(
        queue_name=QUEUE, task_id=777, sender_worker=SENDER,
        from_session=FROM_SESSION, reason="sender at its time limit",
        notes=NOTES, worktree_arg=worktree,
    )
    kw.update(send_kw)
    with serve_claimed([sender_row()]), live_and_room(), quiet_logs(), \
            server_patches, \
            patch.object(launch_one, "queue", fq), \
            patch.object(launch_one, "hard_stop_reason", return_value=None), \
            patch.object(launch_one, "post_agent_note",
                         return_value=(True, None)), \
            patch.object(send, "is_worktree_of", return_value=True):
        rec = send.send(**kw)
    return rec, fq, deleted, prompts


class TestValidationGates(unittest.TestCase):
    def test_no_reason_refuses(self):
        with quiet_logs():
            rec = send.send(QUEUE, 777, SENDER, FROM_SESSION, "  ", NOTES, None)
        self.assertFalse(rec["ok"])
        self.assertEqual(rec["phase"], "validate")

    def test_no_notes_refuses(self):
        with quiet_logs():
            rec = send.send(QUEUE, 777, SENDER, FROM_SESSION, "a reason", "", None)
        self.assertFalse(rec["ok"])
        self.assertEqual(rec["phase"], "validate")

    def test_no_from_session_refuses(self):
        with quiet_logs():
            rec = send.send(QUEUE, 777, SENDER, "", "a reason", NOTES, None)
        self.assertFalse(rec["ok"])
        self.assertEqual(rec["phase"], "validate")


class TestClaimAndLabelGates(unittest.TestCase):
    def test_sender_must_hold_the_task(self):
        with serve_claimed([sender_row(worker_id="repo-worker-code-x")]), \
                live_and_room(), quiet_logs():
            rec = send.send(QUEUE, 777, SENDER, FROM_SESSION,
                            "a reason", NOTES, None)
        self.assertFalse(rec["ok"])
        self.assertEqual(rec["phase"], "verify_claim")

    def test_hard_stop_label_refuses_before_any_queue_write(self):
        fq = FakeQueue()
        with serve_claimed([sender_row()]), live_and_room(), quiet_logs(), \
                patch.object(launch_one, "queue", fq), \
                patch.object(launch_one, "hard_stop_reason",
                             return_value="hard-stop:held"):
            rec = send.send(QUEUE, 777, SENDER, FROM_SESSION,
                            "a reason", NOTES, "/tmp/wt")
        self.assertFalse(rec["ok"])
        self.assertEqual(rec["phase"], "labels")
        self.assertEqual(fq.calls, [])

    def test_primary_checkout_is_not_a_worktree(self):
        with serve_claimed([sender_row()]), live_and_room(), quiet_logs(), \
                patch.object(launch_one, "hard_stop_reason", return_value=None):
            rec = send.send(QUEUE, 777, SENDER, FROM_SESSION,
                            "a reason", NOTES, PRIMARY)
        self.assertFalse(rec["ok"])
        self.assertEqual(rec["phase"], "worktree")


class TestHappyPath(unittest.TestCase):
    def test_send_end_to_end(self):
        with tempfile.TemporaryDirectory() as wt:
            rec, fq, deleted, prompts = run_send(wt)
            self.assertTrue(rec["ok"], rec)
            self.assertEqual(rec["session_id"], NEW_SESSION)
            self.assertEqual(rec["worktree"], wt)
            self.assertEqual(deleted, [])
            # Order: assignment -> sender checkpoint -> transfer
            # claim -> launched checkpoint.
            self.assertEqual(
                fq.verbs(), ["lane", "checkpoint", "claim", "checkpoint"])
            claim = fq.claim_args()
            self.assertIn("--from-session", claim)
            self.assertEqual(claim[claim.index("--from-session") + 1],
                             FROM_SESSION)
            self.assertEqual(claim[claim.index("--session-id") + 1],
                             NEW_SESSION)
            # The prompt carries the sender's notes and the
            # worktree, and says who sent it.
            self.assertEqual(len(prompts), 1)
            self.assertIn("SENDER NOTES", prompts[0])
            self.assertIn(NOTES, prompts[0])
            self.assertIn(wt, prompts[0])
            self.assertIn(FROM_SESSION, prompts[0])
            # The notes file landed in the worktree.
            notes_file = Path(wt) / "SEND_NOTES.md"
            self.assertTrue(notes_file.exists())
            self.assertIn(NOTES, notes_file.read_text())

    def test_dry_run_mutates_nothing(self):
        with tempfile.TemporaryDirectory() as wt:
            rec, fq, deleted, prompts = run_send(wt, dry_run=True)
            self.assertTrue(rec["ok"], rec)
            self.assertTrue(rec["dry_run"])
            self.assertEqual(fq.calls, [])
            self.assertEqual(prompts, [])


class TestRollback(unittest.TestCase):
    def test_failed_transfer_deletes_session_and_flips_back(self):
        with tempfile.TemporaryDirectory() as wt:
            rec, fq, deleted, _prompts = run_send(
                wt, fq=FakeQueue(claim_rc=4))
            self.assertFalse(rec["ok"])
            self.assertEqual(rec["phase"], "transfer")
            self.assertTrue(rec["rolled_back"])
            self.assertTrue(rec["assignment_flipped_back"])
            self.assertEqual(deleted, [NEW_SESSION])
            # lane assign, sender checkpoint, claim, lane flip-back.
            self.assertEqual(
                fq.verbs(), ["lane", "checkpoint", "claim", "lane"])

    def test_failed_assignment_creates_no_session(self):
        created = []
        orig = claimant._server_request

        def spy(method, path, body=None, timeout=5):
            created.append(path)
            return orig(method, path, body=body, timeout=timeout)

        with tempfile.TemporaryDirectory() as wt:
            fq = FakeQueue(assign_lane="direct")
            server_patches, _d, _p = fake_server(wt)
            with serve_claimed([sender_row()]), live_and_room(), \
                    quiet_logs(), server_patches, \
                    patch.object(claimant, "_server_request", spy), \
                    patch.object(launch_one, "queue", fq), \
                    patch.object(launch_one, "hard_stop_reason",
                                 return_value=None), \
                    patch.object(send, "is_worktree_of", return_value=True):
                rec = send.send(QUEUE, 777, SENDER, FROM_SESSION,
                                "a reason", NOTES, wt)
            self.assertFalse(rec["ok"])
            self.assertEqual(rec["phase"], "assign")
            self.assertEqual(created, [])

    def test_prompt_failure_keeps_the_binding(self):
        with tempfile.TemporaryDirectory() as wt:
            rec, _fq, deleted, _prompts = run_send(
                wt, server_kw={"prompt_ok": False})
            self.assertFalse(rec["ok"])
            self.assertFalse(rec["rolled_back"])
            self.assertIn("prompt_failed", rec["reason"])
            self.assertEqual(deleted, [])


class TestBuildPromptSenderNotes(unittest.TestCase):
    def test_notes_section_present_only_when_given(self):
        row = sender_row()
        with_notes = launch_one.build_prompt(
            row, ("issue", 4242), "developing-today/code", "/tmp/wt",
            NEW_SESSION, "opencode-claimant", sender_notes="do the thing")
        self.assertIn("SENDER NOTES", with_notes)
        self.assertIn("do the thing", with_notes)
        without = launch_one.build_prompt(
            row, ("issue", 4242), "developing-today/code", "/tmp/wt",
            NEW_SESSION, "opencode-claimant")
        self.assertNotIn("SENDER NOTES", without)


if __name__ == "__main__":
    unittest.main()
