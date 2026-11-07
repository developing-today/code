#!/usr/bin/env python3
import importlib.util
import json
import types
from unittest import mock
import sys
import tempfile
import time
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "bin/cargo_target_janitor.py"
spec = importlib.util.spec_from_file_location("janitor", SCRIPT)
janitor = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = janitor
spec.loader.exec_module(janitor)


class JanitorTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name) / "fleet"
        self.root.mkdir()

    def tearDown(self):
        self.tmp.cleanup()

    def make_target(self, name, size=10, age=72):
        path = self.root / name
        path.mkdir(parents=True)
        data = path / "artifact"
        data.write_bytes(b"x" * size)
        stamp = time.time() - age * 3600
        data.touch()
        import os

        os.utime(data, (stamp, stamp))
        return path

    def test_scans_worktree_target_and_protects_claimed_task(self):
        target = self.make_target("task-42/pkgs/id/target", size=11)
        worktree = target.parents[2]
        found = janitor.discover_targets(
            self.root,
            {worktree.resolve(): "queue/42-fix"},
            {42},
            {},
            set(),
            [],
            time.time() - 48 * 3600,
        )
        self.assertEqual(len(found), 1)
        self.assertEqual(found[0].protected, "live-claimed-task")

    def test_open_pr_target_is_protected(self):
        target = self.make_target("task-43/pkgs/id/target")
        worktree = target.parents[2]
        found = janitor.discover_targets(
            self.root,
            {worktree.resolve(): "queue/43-fix"},
            set(),
            {},
            {"queue/43-fix"},
            [],
            time.time() - 48 * 3600,
        )
        self.assertEqual(found[0].protected, "open-pr-worktree")

    def test_recent_and_unmapped_targets_are_never_candidates(self):
        recent = self.make_target("task-44/pkgs/id/target", age=1)
        unknown = self.make_target("target-not-a-task")
        found = janitor.discover_targets(
            self.root,
            {recent.parents[2].resolve(): "queue/44-fix"},
            set(),
            {},
            set(),
            [],
            time.time() - 48 * 3600,
        )
        # "target-not-a-task" is not the documented target-<task-id> shape
        # and holds no pkgs/id/target, so it is not a target at all
        # (PR #4175 Codex P1: non-numeric target-* names are checkouts or
        # stray directories, never disposable targets).
        self.assertEqual({t.protected for t in found}, {"younger-than-48h"})
        self.assertNotIn(unknown, {t.path for t in found})

    def test_terminal_task_is_eligible_before_age_threshold(self):
        target = self.make_target("task-45/pkgs/id/target", age=1)
        worktree = target.parents[2]
        found = janitor.discover_targets(
            self.root,
            {worktree.resolve(): "queue/45-fix"},
            set(),
            {45: "done"},
            set(),
            [],
            time.time() - 48 * 3600,
        )
        self.assertIsNone(found[0].protected)
        self.assertEqual(found[0].reason, "terminal-task")

    def test_adhoc_target_is_protected_during_any_fleet_build(self):
        target = self.make_target("target-46", age=72)
        active_worktree = self.root / "task-88"
        active_worktree.mkdir()
        found = janitor.discover_targets(
            self.root,
            {},
            set(),
            {46: "done"},
            set(),
            [active_worktree],
            time.time() - 48 * 3600,
        )
        self.assertEqual(found[0].protected, "fleet-build-active")

    def test_adhoc_target_for_gone_worktree_is_eligible(self):
        target = self.make_target("target-47", age=1)
        found = janitor.discover_targets(
            self.root,
            {},
            set(),
            {47: "parked"},
            set(),
            [],
            time.time() - 48 * 3600,
        )
        self.assertIsNone(found[0].protected)
        self.assertEqual(found[0].reason, "worktree-gone")

    def test_plan_is_oldest_first_and_stops_at_cap(self):
        old = self.make_target("target-1", size=7)
        newer = self.make_target("target-2", size=5)
        targets = [
            janitor.Target(old, 7, 100, None, 1, reason="older-than-threshold"),
            janitor.Target(newer, 5, 200, None, 2, reason="older-than-threshold"),
        ]
        selected, projected = janitor.build_plan(targets, 8)
        self.assertEqual([t.path for t in selected], [old])
        self.assertEqual(projected, 5)

    def test_registered_worktree_named_like_target_is_not_adhoc(self):
        # Codex P1 on PR #4175: a registered worktree called
        # "target-42-worktree" was classified as a disposable ad-hoc
        # target and its whole checkout became removable.
        target = self.make_target("target-42-worktree/pkgs/id/target")
        worktree = target.parents[2]
        found = janitor.discover_targets(
            self.root,
            {worktree.resolve(): "feature/my-current-work"},
            set(),
            {},
            {"feature/my-current-work"},
            [],
            time.time() - 48 * 3600,
        )
        self.assertEqual(len(found), 1)
        self.assertEqual(found[0].path, target.resolve())
        self.assertEqual(found[0].protected, "open-pr-worktree")

    def test_hardlinked_artifacts_counted_once(self):
        # Codex P2 on PR #4175: Cargo hard-links artifacts; summing
        # st_size per path double-charges the same inode.
        import os

        single = self.root / "single"
        single.mkdir()
        (single / "bin").write_bytes(b"x" * 100_000)
        doubled = self.root / "doubled"
        doubled.mkdir()
        (doubled / "bin").write_bytes(b"x" * 100_000)
        os.link(doubled / "bin", doubled / "bin-hash")
        self.assertEqual(
            janitor.target_stats(doubled)[0], janitor.target_stats(single)[0]
        )

    def test_worktree_process_guard_helpers(self):
        target = self.root / "task-9" / "pkgs" / "id" / "target"
        worktree = janitor.worktree_root_for_target(target)
        self.assertEqual(worktree, self.root / "task-9")
        self.assertTrue(janitor.cwd_inside([worktree / "pkgs"], worktree))
        self.assertFalse(janitor.cwd_inside([self.root / "other"], worktree))
        self.assertIsNone(janitor.worktree_root_for_target(self.root / "target-9"))

    def test_default_logs_plan_without_deleting(self):
        path = self.make_target("target-7")
        log = Path(self.tmp.name) / "runs" / "run.jsonl"
        janitor.append_log(log, {"event": "plan", "selected": [str(path)]})
        self.assertTrue(path.exists())
        self.assertIn('"event": "plan"', log.read_text())


    def test_owner_changed_when_worktree_switched_branch(self):
        target = self.make_target("task-43/pkgs/id/target")
        recorded = janitor.Target(target.resolve(), 10, 0, "queue/42-old", 42)
        worktree = target.parents[2].resolve()
        self.assertTrue(janitor.owner_changed(recorded, {worktree: "queue/43-new"}))
        self.assertFalse(
            janitor.owner_changed(
                janitor.Target(target.resolve(), 10, 0, "queue/43-new", 43),
                {worktree: "queue/43-new"},
            )
        )
        # The worktree was unregistered entirely since the plan.
        self.assertTrue(janitor.owner_changed(recorded, {}))

    def test_owner_changed_ignores_adhoc_targets(self):
        target = self.make_target("target-42")
        recorded = janitor.Target(target.resolve(), 10, 0, None, 42)
        self.assertFalse(janitor.owner_changed(recorded, {}))


if __name__ == "__main__":
    unittest.main()


class FakeQueueModule:
    """Stand-in for the queue helper module: serves scripted rows."""

    def __init__(self, rows):
        self.rows = rows

    def http(self, method, query, key):
        return self.rows


def claimed_row(task_id):
    return {
        "id": task_id,
        "status": "claimed",
        "queue": janitor.QUEUE,
        "payload": {"repo": janitor.REPO},
    }


class LivenessPageTests(unittest.TestCase):
    def test_queue_claims_full_page_fails_closed(self):
        rows = [claimed_row(i) for i in range(janitor.CLAIMS_PAGE_LIMIT)]
        with self.assertRaises(janitor.SafetyError):
            janitor.queue_claims((FakeQueueModule(rows), "key"))

    def test_queue_claims_below_cap_returns_ids(self):
        limit = janitor.CLAIMS_PAGE_LIMIT
        rows = [claimed_row(i) for i in range(limit - 1)]
        self.assertEqual(
            janitor.queue_claims((FakeQueueModule(rows), "key")),
            set(range(limit - 1)),
        )

    def test_open_pr_branches_full_page_fails_closed(self):
        rows = [{"headRefName": f"b{i}"} for i in range(janitor.PR_PAGE_LIMIT)]
        proc = types.SimpleNamespace(stdout=json.dumps(rows))
        with mock.patch.object(janitor.subprocess, "run", return_value=proc):
            with self.assertRaises(janitor.SafetyError):
                janitor.open_pr_branches()

    def test_open_pr_branches_below_cap_returns_branches(self):
        rows = [{"headRefName": "a"}, {"headRefName": "b"}]
        proc = types.SimpleNamespace(stdout=json.dumps(rows))
        with mock.patch.object(janitor.subprocess, "run", return_value=proc):
            self.assertEqual(janitor.open_pr_branches(), {"a", "b"})

    def test_queue_task_states_oversized_id_set_fails_closed(self):
        ids = set(range(janitor.TASK_STATES_PAGE_LIMIT + 1))
        with self.assertRaises(janitor.SafetyError):
            janitor.queue_task_states((FakeQueueModule([]), "key"), ids)

    def test_queue_task_states_at_cap_reads_all(self):
        ids = set(range(janitor.TASK_STATES_PAGE_LIMIT))
        rows = [{"id": i, "status": "done"} for i in sorted(ids)]
        states = janitor.queue_task_states((FakeQueueModule(rows), "key"), ids)
        self.assertEqual(len(states), janitor.TASK_STATES_PAGE_LIMIT)
