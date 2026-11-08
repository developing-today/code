#!/usr/bin/env python3
"""Tests for the yard-reaper whole-entry classification (code#7093).

Oracle: Drew's disposition rule (2026-11-07) -- KEEP text, scripts,
and markdown (provenance beats extension); a whole entry is reaped
only when preservation is proven (clone clean + HEAD == live remote
tip, or scratch tree with zero keep-set files) and every liveness
gate (claimed task/issue, process cwd, 4h idle) is clear.
"""
import importlib.util
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "bin/yard_reaper.py"
spec = importlib.util.spec_from_file_location("yard_reaper", SCRIPT)
reaper = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = reaper
spec.loader.exec_module(reaper)


def age(path: Path, hours: float) -> None:
    stamp = time.time() - hours * 3600
    for root, dirs, files in os.walk(path):
        for name in dirs + files:
            os.utime(Path(root) / name, (stamp, stamp))
    os.utime(path, (stamp, stamp))


class YardEntryTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name) / "fleet"
        self.root.mkdir()
        self.idle_before = time.time() - 4 * 3600

    def tearDown(self):
        self.tmp.cleanup()

    def discover(self, **kw):
        return reaper.discover_entries(
            self.root,
            kw.get("registered", set()),
            kw.get("claimed", set()),
            kw.get("claimed_issues", set()),
            kw.get("prs", set()),
            kw.get("cwds", []),
            self.idle_before,
        )

    def by_name(self, entries):
        return {e.path.name: e for e in entries}

    def test_scratch_without_keep_set_is_reapable(self):
        d = self.root / "probe-x"
        d.mkdir()
        (d / "blob.bin").write_bytes(b"x" * 64)
        age(d, 10)
        found = self.by_name(self.discover())
        self.assertEqual(found["probe-x"].reason, "scratch-no-keep-set")
        self.assertIsNone(found["probe-x"].protected)

    def test_scratch_with_authored_note_is_protected(self):
        d = self.root / "probe-y"
        d.mkdir()
        (d / "notes.md").write_text("human triage notes")
        (d / "blob.bin").write_bytes(b"x" * 64)
        age(d, 10)
        found = self.by_name(self.discover())
        self.assertEqual(found["probe-y"].protected, "holds-keep-set")
        self.assertEqual(found["probe-y"].keep_files, 1)

    def test_fixture_txt_flood_is_not_keep_set(self):
        d = self.root / "watcher-bench-run1"
        d.mkdir()
        for i in range(5):
            (d / f"fixture-{i}.txt").write_text("generated")
        age(d, 10)
        found = self.by_name(self.discover())
        self.assertEqual(found["watcher-bench-run1"].keep_files, 0)
        self.assertEqual(found["watcher-bench-run1"].reason, "scratch-no-keep-set")

    def test_keep_set_inside_target_is_bulk_not_authored(self):
        d = self.root / "probe-z"
        t = d / "target" / "debug"
        t.mkdir(parents=True)
        (t / "build.log").write_text("cargo output")
        (d / "blob.bin").write_bytes(b"x" * 8)
        age(d, 10)
        found = self.by_name(self.discover())
        self.assertEqual(found["probe-z"].keep_files, 0)
        self.assertEqual(found["probe-z"].reason, "scratch-no-keep-set")

    def test_recent_activity_protects(self):
        d = self.root / "probe-live"
        d.mkdir()
        (d / "blob.bin").write_bytes(b"x" * 8)
        age(d, 1)
        found = self.by_name(self.discover())
        self.assertEqual(found["probe-live"].protected, "active-within-4h")

    def test_claimed_task_protects(self):
        d = self.root / "1234-5678-some-work"
        d.mkdir()
        (d / "blob.bin").write_bytes(b"x" * 8)
        age(d, 10)
        found = self.by_name(self.discover(claimed={1234}))
        self.assertEqual(found["1234-5678-some-work"].protected, "live-claimed-task")

    def test_process_cwd_protects(self):
        d = self.root / "probe-busy"
        d.mkdir()
        (d / "blob.bin").write_bytes(b"x" * 8)
        age(d, 10)
        found = self.by_name(self.discover(cwds=[d.resolve()]))
        self.assertEqual(found["probe-busy"].protected, "process-in-entry")

    def test_registered_worktree_is_not_an_entry(self):
        d = self.root / "task-99"
        d.mkdir()
        (d / "blob.bin").write_bytes(b"x" * 8)
        age(d, 10)
        found = self.by_name(self.discover(registered={d.resolve()}))
        self.assertNotIn("task-99", found)

    def test_examination_hold_by_name(self):
        for name in ("7780-1931-slice1", "11081"):
            d = self.root / name
            d.mkdir()
            (d / "blob.bin").write_bytes(b"x" * 8)
            age(d, 10)
        found = self.by_name(self.discover())
        self.assertEqual(found["7780-1931-slice1"].protected, "examination-hold")
        self.assertEqual(found["11081"].protected, "examination-hold")

    def test_root_files_are_never_entries(self):
        (self.root / "precious-notes.md").write_text("loose root file")
        found = self.discover()
        self.assertEqual(found, [])

    def make_clone(self, name: str) -> tuple[Path, Path]:
        origin = Path(self.tmp.name) / f"{name}-origin.git"
        subprocess.run(
            ["git", "init", "--bare", str(origin)], check=True, capture_output=True
        )
        seed = Path(self.tmp.name) / f"{name}-seed"
        subprocess.run(
            ["git", "clone", str(origin), str(seed)], check=True, capture_output=True
        )
        (seed / "a.txt").write_text("seed")
        env = {
            **os.environ,
            "GIT_AUTHOR_NAME": "t",
            "GIT_AUTHOR_EMAIL": "t@t",
            "GIT_COMMITTER_NAME": "t",
            "GIT_COMMITTER_EMAIL": "t@t",
        }
        subprocess.run(["git", "-C", str(seed), "add", "."], check=True, env=env)
        subprocess.run(
            ["git", "-C", str(seed), "commit", "-m", "seed"],
            check=True,
            env=env,
            capture_output=True,
        )
        subprocess.run(
            ["git", "-C", str(seed), "push", "origin", "HEAD"],
            check=True,
            env=env,
            capture_output=True,
        )
        clone = self.root / name
        subprocess.run(
            ["git", "clone", str(origin), str(clone)], check=True, capture_output=True
        )
        return clone, origin

    def test_clone_clean_at_remote_tip_is_reapable(self):
        clone, _origin = self.make_clone("nested-a")
        age(clone, 10)
        found = self.by_name(self.discover())
        self.assertEqual(found["nested-a"].reason, "clone-verified-clean")
        self.assertIsNone(found["nested-a"].protected)

    def test_clone_with_untracked_file_is_protected(self):
        clone, _origin = self.make_clone("nested-b")
        (clone / "unlanded.md").write_text("not pushed")
        age(clone, 10)
        found = self.by_name(self.discover())
        self.assertEqual(found["nested-b"].protected, "clone-unlanded-state")


if __name__ == "__main__":
    unittest.main()
