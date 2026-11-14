#!/usr/bin/env python3
import importlib.util
import json
import re
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

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


def claimed_row(task_id):
    return {
        "id": task_id,
        "queue": janitor.QUEUE,
        "status": "claimed",
        "payload": {"repo": janitor.REPO},
    }


class StubQueueModule:
    """PostgREST stub honouring limit/offset and id=in.(...) semantics."""

    def __init__(self, rows, always_full=False):
        self.rows = rows
        self.always_full = always_full
        self.queries = []

    def http(self, method, query, key):
        self.queries.append(query)
        limit = int(re.search(r"limit=(\d+)", query).group(1))
        if "id=in." in query:
            ids = {
                int(x)
                for x in re.search(r"id=in\.\%28([0-9,]+)\%29", query)
                .group(1)
                .split(",")
            }
            return [r for r in self.rows if r["id"] in ids][:limit]
        if "id=gt." in query:
            if self.always_full:
                return self.rows[:limit]
            after = int(re.search(r"id=gt\.(\d+)", query).group(1))
            keyset = sorted(
                (r for r in self.rows if r["id"] > after), key=lambda r: r["id"]
            )
            return keyset[:limit]
        offset = int(re.search(r"offset=(\d+)", query).group(1))
        if self.always_full:
            return self.rows[:limit]
        return self.rows[offset : offset + limit]


class RawQueueModule:
    """Queue-module stub returning one fixed page verbatim, so tests
    can feed the readers shapes StubQueueModule cannot express."""

    def __init__(self, page):
        self.page = page

    def http(self, method, query, key):
        return self.page


class JanitorPaginationTests(unittest.TestCase):
    """#6618: the two deletion-safety liveness reads must be complete
    or fail closed — a full page is never silently the whole set."""

    def test_queue_claims_pages_past_1000(self):
        stub = StubQueueModule([claimed_row(i) for i in range(1, 1006)])
        got = janitor.queue_claims((stub, "key"))
        self.assertEqual(got, set(range(1, 1006)))
        self.assertGreaterEqual(len(stub.queries), 2)

    def test_queue_claims_at_cap_boundary(self):
        stub = StubQueueModule([claimed_row(i) for i in range(1, 1001)])
        self.assertEqual(janitor.queue_claims((stub, "key")), set(range(1, 1001)))

    def test_queue_claims_survives_churn_behind_the_cursor(self):
        """#6790: a task leaving the claimed set mid-read must not
        shift a still-claimed task out of the never-delete set."""
        rows = [claimed_row(i) for i in range(1, 1501)]

        class ChurnStub(StubQueueModule):
            def http(self, method, query, key):
                page = super().http(method, query, key)
                if len(self.queries) == 1:
                    # Task 1 completes after page 1 is served — the
                    # ordinary churn of a live claimed set.
                    self.rows = [r for r in self.rows if r["id"] != 1]
                return page

        stub = ChurnStub(rows)
        got = janitor.queue_claims((stub, "key"))
        # Task 1001 is the row offset paging skipped in the filing's
        # probe; the keyset read returns every claimed id.
        self.assertEqual(got, set(range(1, 1501)))
        self.assertTrue(all("offset=" not in q for q in stub.queries))

    def test_queue_claims_fails_closed_when_pages_never_end(self):
        stub = StubQueueModule(
            [claimed_row(i) for i in range(1, 1001)], always_full=True
        )
        with self.assertRaises(janitor.SafetyError):
            janitor.queue_claims((stub, "key"))

    def test_queue_claims_malformed_shapes_raise_safety_error(self):
        """#6844: a malformed broker page/row must surface as a
        SafetyError naming queue_claims, never a raw shape error --
        a raw error kills the janitor with no refused run record."""
        pages = [
            {"error": "boom"},  # error-dict page
            ["not-a-row"],  # string row
            [
                {
                    "queue": janitor.QUEUE,
                    "status": "claimed",
                    "payload": {"repo": janitor.REPO},
                }
            ],  # missing row id
            [
                {
                    "id": "zz",
                    "queue": janitor.QUEUE,
                    "status": "claimed",
                    "payload": {"repo": janitor.REPO},
                }
            ],  # non-int id
            [
                {
                    "id": {"n": 1},
                    "queue": janitor.QUEUE,
                    "status": "claimed",
                    "payload": {"repo": janitor.REPO},
                }
            ],  # dict id
            [
                {
                    "id": 1,
                    "queue": janitor.QUEUE,
                    "status": "claimed",
                    "payload": "not-an-object",
                }
            ],  # string payload
        ]
        for page in pages:
            with self.subTest(page=page):
                with self.assertRaises(janitor.SafetyError) as caught:
                    janitor.queue_claims((RawQueueModule(page), "key"))
                self.assertIn("queue_claims", str(caught.exception))

    def test_queue_task_states_malformed_shapes_raise_safety_error(self):
        """#6844: same shape discipline for queue_task_states."""
        pages = [
            {"error": "boom"},  # error-dict page
            ["not-a-row"],  # string row
            [{"status": "claimed"}],  # missing row id
            [{"id": "zz", "status": "claimed"}],  # non-int id
            [{"id": 1}],  # missing status
            [{"id": 1, "status": {"s": "claimed"}}],  # dict status
        ]
        for page in pages:
            with self.subTest(page=page):
                with self.assertRaises(janitor.SafetyError) as caught:
                    janitor.queue_task_states((RawQueueModule(page), "key"), {1})
                self.assertIn("queue_task_states", str(caught.exception))

    def test_queue_readers_well_formed_controls_unchanged(self):
        """#6844 control: well-formed rows return exactly what the
        unguarded readers returned."""
        stub = StubQueueModule([claimed_row(i) for i in range(1, 6)])
        self.assertEqual(janitor.queue_claims((stub, "key")), {1, 2, 3, 4, 5})
        rows = [{"id": i, "status": "claimed"} for i in range(1, 4)]
        got = janitor.queue_task_states((RawQueueModule(rows), "key"), {1, 2, 3})
        self.assertEqual(got, {1: "claimed", 2: "claimed", 3: "claimed"})

    def fake_gh(self, total, always_full=False):
        calls = []

        def fake_run(cmd, **kwargs):
            calls.append(cmd)
            page = int(re.search(r"[?&]page=(\d+)", cmd[-1]).group(1))
            per_page = int(re.search(r"[?&]per_page=(\d+)", cmd[-1]).group(1))
            if always_full:
                rows = [{"head": {"ref": f"b{i}"}} for i in range(per_page)]
            else:
                start = (page - 1) * per_page
                rows = [
                    {"head": {"ref": f"b{i}"}}
                    for i in range(start, min(start + per_page, total))
                ]
            return subprocess.CompletedProcess(cmd, 0, stdout=json.dumps(rows))

        return fake_run, calls

    def test_open_pr_branches_pages_past_500(self):
        fake_run, calls = self.fake_gh(600)
        with mock.patch("subprocess.run", fake_run):
            got = janitor.open_pr_branches()
        self.assertEqual(got, {f"b{i}" for i in range(600)})
        self.assertGreaterEqual(len(calls), 6)

    def test_open_pr_branches_at_cap_boundary(self):
        fake_run, _ = self.fake_gh(500)
        with mock.patch("subprocess.run", fake_run):
            self.assertEqual(janitor.open_pr_branches(), {f"b{i}" for i in range(500)})

    def test_open_pr_branches_fails_closed_when_pages_never_end(self):
        fake_run, _ = self.fake_gh(0, always_full=True)
        with mock.patch("subprocess.run", fake_run):
            with self.assertRaises(janitor.SafetyError):
                janitor.open_pr_branches()

    def test_queue_task_states_chunks_beyond_1000_ids(self):
        rows = [
            {"id": i, "status": "claimed" if i % 2 else "done"} for i in range(1, 1201)
        ]
        stub = StubQueueModule(rows)
        got = janitor.queue_task_states((stub, "key"), set(range(1, 1201)))
        self.assertEqual(len(got), 1200)
        self.assertEqual(got[1200], "done")
        self.assertEqual(len(stub.queries), 3)


class JanitorFallbackSourceTests(unittest.TestCase):
    """#6812: the janitor's claims source must survive the helper's
    retirement — GitHub fallback claims protect by issue, discovery
    covers queue/ sub-worktrees and repo-root targets, and a refusal
    leaves a run record."""

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
        import os

        os.utime(data, (stamp, stamp))
        return path

    def test_issue_and_dir_task_identity(self):
        self.assertEqual(janitor.issue_id_from("queue/10528-5743-generic"), 5743)
        self.assertEqual(janitor.issue_id_from("queue/fallback-6812-janitor"), 6812)
        self.assertEqual(janitor.issue_id_from("queue/fb-6757-check"), 6757)
        self.assertEqual(janitor.issue_id_from("fb-6615-records"), 6615)
        self.assertEqual(janitor.issue_id_from("6635-issue-6635-exec"), 6635)
        self.assertEqual(janitor.dir_task_id("10747-1727-built"), 10747)
        self.assertIsNone(janitor.issue_id_from("queue/42-fix"))
        # fb-<issue>-<anything> is the fallback worktree convention,
        # so a same-issue tools dir resolves to that issue too (it can
        # only ever add protection, never remove it).
        self.assertEqual(janitor.issue_id_from("fb-5743-tools"), 5743)
        self.assertIsNone(janitor.issue_id_from("fb-tools"))

    def test_parse_fallback_scan_protects_live_and_contested_only(self):
        scan = {
            "items": [
                {"number": 1, "claim": {"state": "live", "claimable": False}},
                {"number": 2, "claim": {"state": "contested", "claimable": False}},
                {"number": 3, "claim": {"state": "stale", "claimable": True}},
                {"number": 4, "claim": {"state": "released", "claimable": True}},
                {"number": 5, "claim": {"state": "free", "claimable": True}},
            ]
        }
        self.assertEqual(janitor.parse_fallback_scan(scan), {1, 2})

    def test_parse_fallback_scan_malformed_shapes_raise_safety_error(self):
        """#6844: a malformed scan document must surface as a
        SafetyError naming parse_fallback_scan, never a raw shape
        error -- a raw error kills the janitor with no refused
        run record (the #6812 invisible-death mode)."""
        scans = [
            [],  # list scan
            {"items": {"number": 1}},  # dict items
            {"items": ["not-an-item"]},  # string item
            {"items": [{"number": 1, "claim": "live"}]},  # string claim
            {"items": [{"claim": {"state": "live"}}]},  # missing number
            {
                "items": [{"number": {"n": 1}, "claim": {"state": "live"}}]
            },  # dict number
            {"items": [{"number": "zz", "claim": {"state": "live"}}]},  # non-int number
        ]
        for scan in scans:
            with self.subTest(scan=scan):
                with self.assertRaises(janitor.SafetyError) as caught:
                    janitor.parse_fallback_scan(scan)
                self.assertIn("parse_fallback_scan", str(caught.exception))

    def test_fallback_claimed_issue_protects_fb_worktree(self):
        target = self.make_target("fb-6757/pkgs/id/target")
        found = janitor.discover_targets(
            self.root,
            {},
            set(),
            {},
            set(),
            [],
            time.time() - 48 * 3600,
            claimed_issues={6757},
        )
        self.assertEqual(len(found), 1)
        self.assertEqual(found[0].protected, "live-claimed-issue")
        self.assertEqual(found[0].issue_id, 6757)

    def test_queue_subworktree_target_is_discovered(self):
        target = self.make_target("queue/1234-9876-x/pkgs/id/target")
        # Real queue/ worktrees carry a .git file; the branch is read
        # from the checkout itself when the primary does not register
        # it. Stub that read (the tmp dir is not a git checkout).
        real = janitor.branch_for_unregistered
        janitor.branch_for_unregistered = lambda p, c: "queue/1234-9876-x"
        try:
            found = self._discover_queue_target()
        finally:
            janitor.branch_for_unregistered = real
        self.assertEqual(len(found), 1)
        self.assertEqual(found[0].task_id, 1234)
        self.assertEqual(found[0].issue_id, 9876)
        self.assertIsNone(found[0].protected)
        self.assertEqual(found[0].reason, "terminal-task")
        self.assertGreater(janitor.measure_total(self.root, {}), 0)

    def _discover_queue_target(self):
        return janitor.discover_targets(
            self.root,
            {},
            set(),
            {1234: "done"},
            set(),
            [],
            time.time() - 48 * 3600,
            claimed_issues=set(),
        )

    def test_repo_root_target_discovered_and_pr_protected(self):
        wt = self.root / "wt-1"
        (wt / "target" / "debug").mkdir(parents=True)
        (wt / "target" / "debug" / "artifact").write_bytes(b"x" * 10)
        (wt / ".git").write_text("gitdir: elsewhere")
        plain = self.root / "wt-2"
        (plain / "target").mkdir(parents=True)
        (plain / "target" / "not-cargo").write_text("x")
        found = janitor.discover_targets(
            self.root,
            {wt.resolve(): "queue/55-x"},
            set(),
            {},
            {"queue/55-x"},
            [],
            time.time() - 48 * 3600,
        )
        self.assertEqual(len(found), 1)
        self.assertEqual(found[0].path, (wt / "target").resolve())
        self.assertEqual(found[0].protected, "open-pr-worktree")
        self.assertEqual(
            janitor.worktree_root_for_target((wt / "target").resolve()), wt.resolve()
        )

    def test_load_claims_source_falls_back_when_helper_missing(self):
        source = janitor.load_claims_source(Path(self.tmp.name) / "nope.py")
        self.assertIsInstance(source, janitor.FallbackClaimsSource)

    def test_helper_source_round_trip_when_helper_exists(self):
        helper = Path(self.tmp.name) / "helper.py"
        helper.write_text(
            "def get_service_key():\n    return 'k'\n"
            "def http(method, query, key):\n"
            "    return [{'id': 7, 'queue': 'repo-health-code-pilot',"
            " 'status': 'claimed',"
            " 'payload': {'repo': 'developing-today/code'}}]\n"
        )
        source = janitor.load_claims_source(helper)
        self.assertIsInstance(source, janitor.HelperClaimsSource)
        claimed, issues = source.claims()
        self.assertEqual(claimed, {7})
        self.assertEqual(issues, set())

    def test_scratch_root_target_ages_out_without_a_checkout(self):
        # A probe/scratch dir (no .git) with a cargo-looking root
        # target: not a checkout, so no worktree-gone shortcut — it
        # becomes eligible only by age, like any loose target.
        probe = self.root / "probe-2026-10-29-x"
        target_dir = probe / "target" / "debug"
        target_dir.mkdir(parents=True)
        artifact = target_dir / "artifact"
        artifact.write_bytes(b"x" * 10)
        import os

        stamp = time.time() - 72 * 3600
        os.utime(artifact, (stamp, stamp))
        os.utime(target_dir, (stamp, stamp))
        os.utime(probe / "target", (stamp, stamp))
        found = janitor.discover_targets(
            self.root,
            {},
            set(),
            {},
            set(),
            [],
            time.time() - 48 * 3600,
        )
        self.assertEqual(len(found), 1)
        self.assertEqual(found[0].path, (probe / "target").resolve())
        self.assertIsNone(found[0].protected)
        self.assertEqual(found[0].reason, "older-than-threshold")

    def test_scratch_root_target_protected_by_build_in_owning_dir(self):
        probe = self.root / "fb-9999-probe"
        target_dir = probe / "target" / "debug"
        target_dir.mkdir(parents=True)
        (target_dir / "artifact").write_bytes(b"x" * 10)
        found = janitor.discover_targets(
            self.root,
            {},
            set(),
            {},
            set(),
            [probe],
            time.time() - 48 * 3600,
            claimed_issues=set(),
        )
        self.assertEqual(found[0].protected, "process-in-worktree")
        claimed = janitor.discover_targets(
            self.root,
            {},
            set(),
            {},
            set(),
            [],
            time.time() - 48 * 3600,
            claimed_issues={9999},
        )
        self.assertEqual(claimed[0].protected, "live-claimed-issue")

    def test_refusal_writes_a_run_record(self):
        state_dir = Path(self.tmp.name) / "state"
        argv = [
            "janitor",
            "--fleet-root",
            str(self.root),
            "--primary",
            str(Path(self.tmp.name) / "not-a-repo"),
            "--queue-helper",
            str(Path(self.tmp.name) / "nope.py"),
            "--state-dir",
            str(state_dir),
        ]
        with mock.patch.object(sys, "argv", argv):
            rc = janitor.main()
        self.assertEqual(rc, 2)
        records = list((state_dir / "runs").glob("*.jsonl"))
        self.assertEqual(len(records), 1)
        self.assertIn('"event": "refused"', records[0].read_text())


if __name__ == "__main__":
    unittest.main()
