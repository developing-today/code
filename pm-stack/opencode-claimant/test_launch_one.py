#!/usr/bin/env python3
"""Tests for launch_one.py, the per-task claimant launch trigger (#8187).

Run:  python3 -m unittest pm-stack/opencode-claimant/test_launch_one.py

Everything external is stubbed at launch_one's own seams: poll.py's
broker fetches serve fixture rows, claimant.py's cap/mode/transport
functions are patched on the module objects launch_one loaded, the
repo-queue CLI is launch_one.queue, and `gh` is reached only through
launch_one.hard_stop_reason / post_agent_note, also patched where a
launch is exercised. No server, broker, or GitHub is touched.

Covered:
- repo-queue resolution: ~/.local/bin/repo-queue by default (never
  the retired ~/fleet-brokers/bin tree), env override wins;
- candidate discovery delegates to poll.py: claimed handed_off rows
  and parked handoff rows are candidates; bound, unyielded, and
  OpenChamber-lane rows are not, with the reason named;
- preflight is read-only in effect: it reports candidate, blockers
  (exhausted, held labels, cap, mode) and the would-steps, and a
  parked candidate's steps start with the by-id claim;
- --launch refuses in shadow mode, on a saturated/unknown cap, and
  on held labels — before creating any session;
- a claimed candidate launches end to end: create (location
  verified) → bind → checkpoint as the claiming worker → prompt,
  and the record carries the bound session id;
- a bind failure deletes the created session (no orphan);
- a location mismatch deletes the session and checkpoints
  launch_failed instead of binding;
- a parked candidate whose by-id claim fails launches nothing.
"""

import importlib.util
import json
import os
import unittest
from pathlib import Path
from unittest.mock import patch

_spec = importlib.util.spec_from_file_location(
    "launch_one", Path(__file__).resolve().parent / "launch_one.py"
)
launch_one = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(launch_one)

poll = launch_one.poll
claimant = launch_one.claimant
QUEUE = "repo-health-code-pilot"
PRIMARY = "/home/user/git/code"


def claimed_row(**over):
    row = {
        "id": 777, "queue": QUEUE, "status": "claimed",
        "title": "Fix the thing", "dedupe_key": "repo-health:v1:fix:developing-today/code:issue:4242",
        "priority": 10, "attempts": 1, "max_attempts": 5,
        "worker_id": "repo-worker-code", "lane_id": "code-a",
        "progress": {"lane": "opencode", "stage": "handed_off"},
        "payload": {"repo": "developing-today/code", "issue": {"number": 4242}},
    }
    row.update(over)
    return row


def parked_row(**over):
    row = claimed_row(status="parked", worker_id=None)
    row.update(over)
    return row


def serve(claimed=(), parked=()):
    """Patch poll's broker fetches to serve the fixture rows."""

    def fetch_claimed(queue):
        return list(claimed) if queue == QUEUE else []

    def fetch_parked(queue):
        return list(parked) if queue == QUEUE else []

    return patch.multiple(
        poll, fetch_claimed_rows=fetch_claimed, fetch_parked_rows=fetch_parked
    )


def live_and_room(room=3):
    return patch.multiple(
        claimant, read_mode=lambda: "live", cap_step=lambda: room
    )


class TestRepoQueueResolution(unittest.TestCase):
    def test_default_is_local_bin_never_fleet_brokers(self):
        with patch.dict(os.environ, {}, clear=True):
            cmd = launch_one.repo_queue_cmd()
        self.assertEqual(cmd, str(Path.home() / ".local" / "bin" / "repo-queue"))
        self.assertNotIn("fleet-brokers", cmd)

    def test_env_override_wins(self):
        with patch.dict(os.environ, {"REPO_QUEUE_BIN": "/tmp/rq-test"}):
            self.assertEqual(launch_one.repo_queue_cmd(), "/tmp/rq-test")


class TestFindRow(unittest.TestCase):
    def test_claimed_handed_off_is_candidate(self):
        with serve(claimed=[claimed_row()]):
            row, status, cand, reasons = launch_one.find_row(QUEUE, 777)
        self.assertTrue(cand)
        self.assertEqual(status, "claimed")
        self.assertEqual(reasons, [])

    def test_parked_handoff_is_candidate(self):
        with serve(parked=[parked_row()]):
            row, status, cand, reasons = launch_one.find_row(QUEUE, 777)
        self.assertTrue(cand)
        self.assertEqual(status, "parked")

    def test_bound_row_is_not_candidate_and_says_why(self):
        row = claimed_row(progress={"lane": "opencode", "stage": "handed_off",
                                    "session_id": "ses_bound"})
        with serve(claimed=[row]):
            _r, _s, cand, reasons = launch_one.find_row(QUEUE, 777)
        self.assertFalse(cand)
        self.assertTrue(any("already bound" in r for r in reasons))

    def test_unyielded_row_is_not_candidate(self):
        row = claimed_row(progress={"lane": "opencode", "stage": "verified"})
        with serve(claimed=[row]):
            _r, _s, cand, reasons = launch_one.find_row(QUEUE, 777)
        self.assertFalse(cand)
        self.assertTrue(any("stage" in r for r in reasons))

    def test_openchamber_lane_row_is_not_candidate(self):
        row = claimed_row(lane_id="8ac61616-3815-4e7b-b33d-840a3829154a")
        with serve(claimed=[row]):
            _r, _s, cand, reasons = launch_one.find_row(QUEUE, 777)
        self.assertFalse(cand)
        self.assertTrue(any("OpenChamber" in r for r in reasons))

    def test_genuine_wait_park_is_not_candidate(self):
        row = parked_row(progress={"lane": "opencode", "stage": "awaiting_ci"})
        with serve(parked=[row]):
            _r, _s, cand, _reasons = launch_one.find_row(QUEUE, 777)
        self.assertFalse(cand)

    def test_missing_task(self):
        with serve():
            row, status, cand, reasons = launch_one.find_row(QUEUE, 999)
        self.assertIsNone(row)
        self.assertFalse(cand)


class TestPreflight(unittest.TestCase):
    def test_parked_candidate_reports_would_steps(self):
        with serve(parked=[parked_row()]), live_and_room(2), \
             patch.object(launch_one, "hard_stop_reason", return_value=None):
            out = launch_one.preflight(QUEUE, 777)
        self.assertTrue(out["candidate"])
        self.assertTrue(out["ok"])
        self.assertEqual(out["cap_room"], 2)
        self.assertIn("--task 777", out["would"][0])
        self.assertTrue(any("bind" in step for step in out["would"]))

    def test_cap_saturated_blocks(self):
        with serve(claimed=[claimed_row()]), live_and_room(0), \
             patch.object(launch_one, "hard_stop_reason", return_value=None):
            out = launch_one.preflight(QUEUE, 777)
        self.assertTrue(out["candidate"])
        self.assertFalse(out["ok"])
        self.assertTrue(any("cap_saturated" in b for b in out["blockers"]))

    def test_held_label_blocks(self):
        with serve(claimed=[claimed_row()]), live_and_room(2), \
             patch.object(launch_one, "hard_stop_reason",
                          return_value="hard-stop:held"):
            out = launch_one.preflight(QUEUE, 777)
        self.assertFalse(out["ok"])
        self.assertEqual(out["hard_stop"], "hard-stop:held")

    def test_not_candidate(self):
        with serve(), live_and_room(2):
            out = launch_one.preflight(QUEUE, 999)
        self.assertFalse(out["candidate"])
        self.assertFalse(out["ok"])
        self.assertEqual(out["would"], [])


class FakeQueue:
    """launch_one.queue stand-in dispatching on the verb."""

    def __init__(self, bind_rc=0, claim_row=None, claim_rc=0):
        self.calls = []
        self.bind_rc = bind_rc
        self.claim_row = claim_row
        self.claim_rc = claim_rc

    def __call__(self, *args, timeout=120):
        self.calls.append(args)
        verb = args[0]
        if verb == "bind":
            return self.bind_rc, "", "bind boom" if self.bind_rc else ""
        if verb == "claim":
            out = json.dumps(self.claim_row) if self.claim_row is not None else '{"task": null}'
            return self.claim_rc, out, ""
        return 0, "", ""

    def verbs(self):
        return [c[0] for c in self.calls]


def fake_server(create_directory=PRIMARY, prompt_ok=True):
    """Patch claimant's transport seams on the loaded module."""
    deleted = []

    def server_request(method, path, body=None, timeout=5):
        if method == "POST" and path == "/api/session":
            return 201, {"id": "ses_new1",
                         "location": {"directory": create_directory}}
        raise AssertionError(f"unexpected server call {method} {path}")

    def delete_session(sid):
        deleted.append(sid)
        return True

    patches = patch.multiple(
        claimant,
        _server_request=server_request,
        _delete_session=delete_session,
        _send_prompt=lambda sid, prompt: prompt_ok,
    )
    return patches, deleted


def no_journal():
    return patch.multiple(launch_one, log=lambda rec: None,
                          journal=lambda rec: None)


class TestLaunch(unittest.TestCase):
    def run_launch(self, *, rows_kw, fq, server_kw=None, room=3,
                   mode="live", hs=None):
        server_patches, deleted = fake_server(**(server_kw or {}))
        with serve(**rows_kw), \
             patch.object(claimant, "read_mode", return_value=mode), \
             patch.object(claimant, "cap_step", return_value=room), \
             patch.object(launch_one, "hard_stop_reason", return_value=hs), \
             patch.object(launch_one, "queue", fq), \
             patch.object(launch_one, "post_agent_note",
                          return_value=(True, None)), \
             server_patches, no_journal():
            rec = launch_one.launch(QUEUE, 777)
        return rec, deleted

    def test_claimed_candidate_launches_end_to_end(self):
        fq = FakeQueue()
        rec, deleted = self.run_launch(rows_kw={"claimed": [claimed_row()]}, fq=fq)
        self.assertTrue(rec["ok"], rec)
        self.assertEqual(rec["session_id"], "ses_new1")
        self.assertEqual(deleted, [])
        self.assertEqual(fq.verbs(), ["bind", "checkpoint"])
        bind = fq.calls[0]
        self.assertIn("ses_new1", bind)
        self.assertIn("opencode-claimant", bind)
        checkpoint = fq.calls[1]
        self.assertIn("repo-worker-code", checkpoint)  # claiming worker owns it
        self.assertIn("launched", checkpoint)

    def test_shadow_mode_refuses_before_any_mutation(self):
        fq = FakeQueue()
        rec, _d = self.run_launch(rows_kw={"claimed": [claimed_row()]},
                                  fq=fq, mode="shadow")
        self.assertFalse(rec["ok"])
        self.assertIn("mode_not_live", rec["reason"])
        self.assertEqual(fq.calls, [])

    def test_cap_saturated_refuses(self):
        fq = FakeQueue()
        rec, _d = self.run_launch(rows_kw={"claimed": [claimed_row()]},
                                  fq=fq, room=0)
        self.assertFalse(rec["ok"])
        self.assertEqual(rec["reason"], "cap_saturated")
        self.assertEqual(fq.calls, [])

    def test_cap_unknown_refuses(self):
        fq = FakeQueue()
        rec, _d = self.run_launch(rows_kw={"claimed": [claimed_row()]},
                                  fq=fq, room=-1)
        self.assertFalse(rec["ok"])
        self.assertIn("cap_unknown", rec["reason"])

    def test_held_label_refuses(self):
        fq = FakeQueue()
        rec, _d = self.run_launch(rows_kw={"claimed": [claimed_row()]},
                                  fq=fq, hs="hard-stop:do-not-merge")
        self.assertFalse(rec["ok"])
        self.assertEqual(rec["reason"], "hard-stop:do-not-merge")
        self.assertEqual(fq.calls, [])

    def test_bind_failure_deletes_the_created_session(self):
        fq = FakeQueue(bind_rc=1)
        rec, deleted = self.run_launch(rows_kw={"claimed": [claimed_row()]}, fq=fq)
        self.assertFalse(rec["ok"])
        self.assertEqual(rec["phase"], "bind")
        self.assertEqual(deleted, ["ses_new1"])
        self.assertEqual(rec["session_cleanup"], "deleted")

    def test_location_mismatch_deletes_and_checkpoints_launch_failed(self):
        fq = FakeQueue()
        rec, deleted = self.run_launch(
            rows_kw={"claimed": [claimed_row()]}, fq=fq,
            server_kw={"create_directory": "/home/user"})
        self.assertFalse(rec["ok"])
        self.assertEqual(rec["phase"], "location")
        self.assertEqual(deleted, ["ses_new1"])
        self.assertNotIn("bind", fq.verbs())
        checkpoint = fq.calls[0]
        self.assertIn("launch_failed", checkpoint)

    def test_prompt_failure_reports_failure_but_keeps_the_binding(self):
        fq = FakeQueue()
        rec, deleted = self.run_launch(
            rows_kw={"claimed": [claimed_row()]}, fq=fq,
            server_kw={"prompt_ok": False})
        self.assertFalse(rec["ok"])
        self.assertIn("prompt_failed", rec["reason"])
        self.assertEqual(deleted, [])  # bound session is supervision's, not deleted

    def test_parked_candidate_claims_by_id_then_launches(self):
        fq = FakeQueue(claim_row={"task": {"id": 777, "worker_id": "opencode-claimant"}})
        rec, _d = self.run_launch(rows_kw={"parked": [parked_row()]}, fq=fq)
        self.assertTrue(rec["ok"], rec)
        self.assertEqual(fq.verbs(), ["claim", "bind", "checkpoint"])
        claim = fq.calls[0]
        self.assertIn("--task", claim)
        self.assertIn("777", claim)
        self.assertIn("openchamber", claim)
        checkpoint = fq.calls[2]
        self.assertIn("opencode-claimant", checkpoint)

    def test_parked_claim_failure_launches_nothing(self):
        fq = FakeQueue(claim_row=None, claim_rc=1)
        rec, deleted = self.run_launch(rows_kw={"parked": [parked_row()]}, fq=fq)
        self.assertFalse(rec["ok"])
        self.assertEqual(rec["reason"], "parked_claim_failed")
        self.assertEqual(fq.verbs(), ["claim"])
        self.assertEqual(deleted, [])


class TestCli(unittest.TestCase):
    def test_bad_spec_is_usage_error(self):
        self.assertEqual(launch_one.main(["launch_one.py", "--preflight", "bogus"]), 2)
        self.assertEqual(launch_one.main(["launch_one.py", "--launch", "nope:1"]), 2)

    def test_launch_takes_exactly_one_spec(self):
        self.assertEqual(
            launch_one.main(["launch_one.py", "--launch",
                             f"{QUEUE}:1", f"{QUEUE}:2"]), 2)

    def test_ref_for_dedupe_and_packet(self):
        row = claimed_row(progress={"lane": "opencode", "stage": "handed_off"},
                          payload={})
        self.assertEqual(launch_one.ref_for(row), ("issue", 4242))
        row2 = claimed_row()
        self.assertEqual(launch_one.ref_for(row2), ("issue", 4242))


if __name__ == "__main__":
    unittest.main()
