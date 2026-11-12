#!/usr/bin/env python3
"""Tests for dispatch_batch.py (#8217).

The batch claim + per-item dispatch driver: one batch claim, then
every item handed to a DEDICATED sub-task session in the same
touch (opencode via send_to_opencode.send, Muse via a lane-spawned
sub-session bound by id), every bind verified from live queue
state, and any failed item never left claimed-idle (released back
to queued with --release-failures, else held by the lane direct).

Effects are faked at dispatch_batch's own seams (queue, run_send,
worktree_for, bound_session_id), so the suite runs anywhere:

    python3 -m unittest test_dispatch_batch -v
"""

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent


def _load(name: str):
    path = HERE / f"{name}.py"
    spec = importlib.util.spec_from_file_location(f"dbt_{name}", path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = mod
    spec.loader.exec_module(mod)
    return mod


db = _load("dispatch_batch")

QUEUE = "repo-health-code-pilot"
WORKER = "repo-worker-code"


def row(task_id: int, issue: int) -> dict:
    return {
        "id": task_id,
        "status": "claimed",
        "payload": {
            "repo": "developing-today/code",
            "issue": {"number": issue},
        },
    }


class FakeQueue:
    """db.queue stand-in: the claim answers scripted rows, other
    verbs are recorded. Returns (rc, stdout, stderr) like the real
    launch_one.queue."""

    def __init__(self, claim_rows):
        self.claim_rows = claim_rows
        self.calls: list[tuple] = []

    def __call__(self, *args, timeout=120):
        self.calls.append(args)
        if args[0] == "claim":
            body = {
                "tasks": self.claim_rows,
                "claimed": len(self.claim_rows),
                "requested": 5,
                "guard_held": [],
            }
            return 0, json.dumps(body), ""
        return 0, "{}", ""

    def verbs(self, verb: str) -> list[tuple]:
        return [c for c in self.calls if c[0] == verb]


class BareRowQueue(FakeQueue):
    """The client's count==1 shape: a claim answers with the bare
    row (the single claim), not the batch {"tasks": [...]} body."""

    def __call__(self, *args, timeout=120):
        self.calls.append(args)
        if args[0] == "claim":
            return 0, json.dumps(self.claim_rows[0]), ""
        return 0, "{}", ""


class DispatchBatchTests(unittest.TestCase):
    def setUp(self):
        self.worktree_calls: list[tuple] = []
        self.bound: dict[int, str | None] = {}

        def fake_worktree(primary, task_id, issue):
            self.worktree_calls.append((primary, task_id, issue))
            return f"/tmp/fleet-wt/queue-{task_id}-{issue}-dispatch", True, None

        def fake_bound(queue_name, task_id):
            return self.bound.get(task_id), None

        self._orig = {
            "queue": db.queue,
            "run_send": db.run_send,
            "worktree_for": db.worktree_for,
            "bound_session_id": db.bound_session_id,
        }
        db.worktree_for = fake_worktree
        db.bound_session_id = fake_bound

    def tearDown(self):
        db.queue = self._orig["queue"]
        db.run_send = self._orig["run_send"]
        db.worktree_for = self._orig["worktree_for"]
        db.bound_session_id = self._orig["bound_session_id"]

    def run_main(self, argv, claim_rows, send_impl=None):
        fake_q = FakeQueue(claim_rows)
        db.queue = fake_q
        if send_impl is not None:
            db.run_send = send_impl
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = db.main(["dispatch_batch.py", *argv])
        records = [
            json.loads(line)
            for line in out.getvalue().splitlines()
            if line.startswith("{")
        ]
        return rc, records, fake_q

    def base_argv(self, *extra):
        return [
            "--queue",
            QUEUE,
            "--worker",
            WORKER,
            "--count",
            "2",
            "--reason",
            "batch dispatch for the test bundle",
            "--notes",
            "lane notes for the batch",
            *extra,
        ]

    def items(self, records):
        return [r for r in records if r.get("kind") != "dispatch_batch"]

    # -- dry run ----------------------------------------------------

    def test_dry_run_claims_and_dispatches_nothing(self):
        rows = [row(101, 555), row(102, 556)]
        rc, records, fake_q = self.run_main(
            self.base_argv("--to", "opencode", "--session-id", "ses_lane", "--dry-run"),
            rows,
            send_impl=lambda *a: self.fail("dry run must not send"),
        )
        self.assertEqual(rc, 0)
        self.assertEqual(len(records), 1, "dry run prints the summary only")
        summary = records[0]
        self.assertTrue(summary["ok"])
        self.assertTrue(summary["dry_run"])
        self.assertEqual(summary["tasks"], [101, 102])
        claim = fake_q.verbs("claim")[0]
        self.assertIn("--count", claim)
        self.assertIn("openchamber", claim, "the claim declares the family")
        self.assertEqual(self.worktree_calls, [])

    # -- opencode dispatch ------------------------------------------

    def test_opencode_items_bound_to_dedicated_sessions(self):
        rows = [row(101, 555), row(102, 556)]
        sends = []

        def send_impl(
            queue_name, task_id, worker, from_session, reason, notes, worktree
        ):
            sends.append(
                {
                    "task": task_id,
                    "from": from_session,
                    "reason": reason,
                    "notes": notes,
                    "worktree": worktree,
                }
            )
            sid = f"ses_item_{task_id}"
            self.bound[task_id] = sid
            return {"ok": True, "session_id": sid, "task": task_id}

        rc, records, _ = self.run_main(
            self.base_argv("--to", "opencode", "--session-id", "ses_lane"),
            rows,
            send_impl=send_impl,
        )
        self.assertEqual(rc, 0)
        items = self.items(records)
        self.assertEqual([r["outcome"] for r in items], ["bound", "bound"])
        sessions = [r["session_id"] for r in items]
        self.assertEqual(len(set(sessions)), 2, "one session per item")
        worktrees = [r["worktree"] for r in items]
        self.assertEqual(len(set(worktrees)), 2, "one worktree per item")
        for s in sends:
            self.assertEqual(s["from"], "ses_lane")
            self.assertEqual(s["reason"], "batch dispatch for the test bundle")
            self.assertEqual(s["notes"], "lane notes for the batch")
            self.assertIn(str(s["task"]), s["worktree"])

    def test_failed_send_releases_item_with_flag(self):
        rows = [row(101, 555), row(102, 556)]

        def send_impl(queue_name, task_id, *rest):
            if task_id == 102:
                return {
                    "ok": False,
                    "phase": "create",
                    "reason": "session create refused",
                }
            sid = "ses_item_101"
            self.bound[task_id] = sid
            return {"ok": True, "session_id": sid}

        rc, records, fake_q = self.run_main(
            self.base_argv(
                "--to", "opencode", "--session-id", "ses_lane", "--release-failures"
            ),
            rows,
            send_impl=send_impl,
        )
        self.assertEqual(rc, 1)
        by_task = {r["task"]: r for r in self.items(records)}
        self.assertEqual(by_task[101]["outcome"], "bound")
        self.assertEqual(by_task[102]["outcome"], "released_to_queue")
        self.assertEqual(by_task[102]["phase"], "create")
        releases = fake_q.verbs("release")
        self.assertEqual(len(releases), 1)
        self.assertEqual(releases[0][1], "102")

    def test_failed_send_held_by_lane_by_default(self):
        rows = [row(101, 555)]

        def send_impl(queue_name, task_id, *rest):
            return {
                "ok": False,
                "phase": "pool",
                "reason": "opencode session pool is full",
            }

        rc, records, fake_q = self.run_main(
            self.base_argv(
                "--to", "opencode", "--session-id", "ses_lane", "--count", "1"
            ),
            rows,
            send_impl=send_impl,
        )
        self.assertEqual(rc, 1)
        item = self.items(records)[0]
        self.assertEqual(item["outcome"], "held_by_lane_direct")
        self.assertEqual(fake_q.verbs("release"), [], "no release without the flag")

    def test_unverified_bind_releases_item(self):
        rows = [row(101, 555)]

        # The send reports a session, but live state never shows it.
        def send_impl(queue_name, task_id, *rest):
            return {"ok": True, "session_id": "ses_ghost"}

        rc, records, fake_q = self.run_main(
            self.base_argv(
                "--to",
                "opencode",
                "--session-id",
                "ses_lane",
                "--count",
                "1",
                "--release-failures",
            ),
            rows,
            send_impl=send_impl,
        )
        self.assertEqual(rc, 1)
        item = self.items(records)[0]
        self.assertEqual(item["outcome"], "released_to_queue")
        self.assertEqual(item["phase"], "verify")
        self.assertEqual(len(fake_q.verbs("release")), 1)

    # -- Muse dispatch ----------------------------------------------

    def test_claude_item_with_bind_is_bound(self):
        rows = [row(101, 555)]
        self.bound[101] = "ses_claude_1"
        rc, records, fake_q = self.run_main(
            self.base_argv(
                "--to", "Muse", "--count", "1", "--bind", "101:ses_claude_1"
            ),
            rows,
            send_impl=lambda *a: self.fail("Muse items never send"),
        )
        self.assertEqual(rc, 0)
        item = self.items(records)[0]
        self.assertEqual(item["outcome"], "bound")
        self.assertEqual(item["session_id"], "ses_claude_1")
        # Lane-direct assignment + dispatch checkpoint precede the bind.
        self.assertEqual(len(fake_q.verbs("lane")), 1)
        self.assertEqual(len(fake_q.verbs("checkpoint")), 1)
        self.assertEqual(len(fake_q.verbs("bind")), 1)

    def test_claude_item_without_bind_is_prepared_unbound(self):
        rows = [row(101, 555)]
        rc, records, fake_q = self.run_main(
            self.base_argv("--to", "Muse", "--count", "1"), rows
        )
        self.assertEqual(rc, 1, "an unbound Muse item fails the batch")
        item = self.items(records)[0]
        self.assertEqual(item["outcome"], "prepared_unbound")
        self.assertIn("bind_command", item)
        self.assertEqual(fake_q.verbs("bind"), [])

    def test_claude_claim_carries_the_muse_lane_stamp(self):
        # #8176: a Muse-family batch claim stamps the muse-lane
        # marker (the lane surfaces carry no session model of
        # their own); an opencode claim must NOT carry it — its
        # stamp is derived from the named session by the queue
        # client instead.
        rows = [row(101, 555)]
        _rc, _records, fake_q = self.run_main(
            self.base_argv("--to", "Muse", "--count", "1"), rows
        )
        claim = fake_q.verbs("claim")[0]
        self.assertEqual(claim[claim.index("--model-provider") + 1], "muse-lane")
        self.assertEqual(claim[claim.index("--model-id") + 1], "muse-lane")

        rows = [row(101, 555), row(102, 556)]
        _rc, _records, fake_q = self.run_main(
            self.base_argv("--to", "opencode", "--session-id", "ses_lane", "--dry-run"),
            rows,
        )
        claim = fake_q.verbs("claim")[0]
        self.assertNotIn("--model-provider", claim)
        self.assertIn("ses_lane", claim)

    def test_single_item_batch_accepts_bare_row_response(self):
        # Regression (live acceptance, #8217): --count 1 is the
        # single claim, whose response is the bare row; dispatch
        # must normalize it instead of failing the claim phase.
        fake_q = BareRowQueue([row(101, 555)])
        db.queue = fake_q
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = db.main(
                [
                    "dispatch_batch.py",
                    *self.base_argv(
                        "--to",
                        "opencode",
                        "--session-id",
                        "ses_lane",
                        "--count",
                        "1",
                        "--dry-run",
                    ),
                ]
            )
        self.assertEqual(rc, 0)
        summary = json.loads(out.getvalue().strip().splitlines()[-1])
        self.assertEqual(summary["tasks"], [101])

    # -- validation ---------------------------------------------------

    def test_count_over_cap_refused_before_claiming(self):
        called = []
        db.queue = lambda *a, timeout=120: called.append(a) or (0, "{}", "")
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = db.main(
                [
                    "dispatch_batch.py",
                    *self.base_argv(
                        "--to", "opencode", "--session-id", "ses_lane", "--count", "6"
                    ),
                ]
            )
        self.assertEqual(rc, 2)
        self.assertEqual(called, [], "over-cap never reaches the broker")

    def test_opencode_requires_session_id(self):
        rc, records, fake_q = self.run_main(
            self.base_argv("--to", "opencode"), [row(101, 555)]
        )
        self.assertEqual(rc, 2)
        self.assertEqual(fake_q.calls, [])


if __name__ == "__main__":
    unittest.main()
