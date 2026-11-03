#!/usr/bin/env python3
"""test_worktree_reaper.py — gate-logic tests for the worktree reaper.

Builds synthetic repos (bare origin + primary clone + worktrees) in a
temp dir and drives the real reaper module end-to-end:

  pushed + clean + quiet + unoccupied   -> reaped, branch -d'd
  tip contained in origin default only  -> reaped (contained-in-default)
  patch-equivalent tip (cherry-picked)  -> reaped, branch -d refuses (kept+logged)
  unpushed branch                       -> kept (unpushed)
  dirty tracked file                    -> kept (dirty)
  untracked non-allowlisted file        -> kept (dirty)
  allowlisted generated dirt only       -> reaped via --force
  detached HEAD                         -> kept (detached, v1 rule)
  denylisted tree                       -> kept (denied)
  process cwd inside tree               -> kept (occupied)
  fresh tree with huge quiet_hours      -> kept (active)
  --dry-run                             -> would-reap listed, tree intact
  DISABLED kill switch                  -> exit 0, nothing touched

Plus unit tests for the porcelain parser, the dirt allowlist, and the
activity walk. Quiet is controlled via the config's quiet_hours (0 makes
any age quiet, since HEAD reflog timestamps are always "now" in a fresh
synthetic repo; 100000 makes everything active).

Run on the laptop:  python3 tests/test_worktree_reaper.py
"""

import contextlib
import importlib.util
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.join(HERE, "..", "bin", "worktree_reaper.py")
spec = importlib.util.spec_from_file_location("worktree_reaper", BIN)
wr = importlib.util.module_from_spec(spec)
spec.loader.exec_module(wr)

GIT_ENV = {
    **os.environ,
    "GIT_AUTHOR_NAME": "Reaper Test",
    "GIT_AUTHOR_EMAIL": "reaper-test@example.invalid",
    "GIT_COMMITTER_NAME": "Reaper Test",
    "GIT_COMMITTER_EMAIL": "reaper-test@example.invalid",
}


def git(repo, *args, check=True, env_extra=None):
    env = {**GIT_ENV, **env_extra} if env_extra else GIT_ENV
    proc = subprocess.run(["git", "-C", repo, *args], env=env,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if check and proc.returncode != 0:
        raise AssertionError(f"git -C {repo} {' '.join(args)} failed: {proc.stderr}")
    return proc


def gitto(*args):
    proc = subprocess.run(["git", *args], env=GIT_ENV,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if proc.returncode != 0:
        raise AssertionError(f"git {' '.join(args)} failed: {proc.stderr}")
    return proc


class SyntheticRepo:
    def __init__(self, root):
        self.origin = os.path.join(root, "origin.git")
        self.git_root = os.path.join(root, "git")
        self.primary = os.path.join(self.git_root, "primary")
        self.trees = os.path.join(root, "trees")
        os.makedirs(self.git_root)
        os.makedirs(self.trees)
        gitto("init", "--bare", "-b", "main", self.origin)
        seed = os.path.join(root, "seed")
        gitto("clone", "--quiet", self.origin, seed)
        git(seed, "symbolic-ref", "HEAD", "refs/heads/main")
        self.write(seed, "base.txt", "base\n")
        git(seed, "add", "base.txt")
        git(seed, "commit", "--quiet", "-m", "base")
        git(seed, "push", "--quiet", "origin", "main")
        shutil.rmtree(seed)
        gitto("clone", "--quiet", self.origin, self.primary)
        # Sanity: the default-ref machinery the pushed gate relies on.
        rc, out, _ = wr.run_git(self.primary, "symbolic-ref", "--quiet",
                                "refs/remotes/origin/HEAD")
        assert rc == 0 and out.decode().strip() == "refs/remotes/origin/main"

    def write(self, repo, rel, content):
        path = os.path.join(repo, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as fh:
            fh.write(content)

    def branch_tree(self, branch, *, push=False, merge_to_main=False):
        """Create a branch with one unique commit and a worktree for it."""
        git(self.primary, "branch", branch)
        tree = os.path.join(self.trees, branch.replace("/", "-"))
        git(self.primary, "worktree", "add", "--quiet", tree, branch)
        self.write(tree, f"{branch.replace('/', '-')}.txt", f"work for {branch}\n")
        git(tree, "add", ".")
        git(tree, "commit", "--quiet", "-m", f"commit on {branch}")
        if merge_to_main:
            git(self.primary, "merge", "--quiet", "--no-ff", branch,
                "-m", f"merge {branch}")
            git(self.primary, "push", "--quiet", "origin", "main")
        if push:
            git(self.primary, "push", "--quiet", "-u", "origin", branch)
        git(self.primary, "checkout", "--quiet", "main")
        return tree

    def branch_exists(self, branch):
        rc, _, _ = wr.run_git(self.primary, "rev-parse", "--verify", "--quiet",
                              f"refs/heads/{branch}")
        return rc == 0


class ReaperRun:
    def __init__(self, repo, state_dir):
        self.repo = repo
        self.state_dir = state_dir

    def run(self, *, quiet_hours=0, dry_run=False, deny_paths=()):
        cfg = {
            "git_root": self.repo.git_root,
            "state_dir": self.state_dir,
            "quiet_hours": quiet_hours,
            "max_removals": 100,
            "deny_paths": list(deny_paths),
            "generated_dirt_prefixes": ["pkgs/mcpx/internal/cli/logs/"],
            "generated_dirt_files": ["fleet-feed.json"],
        }
        cfg_path = os.path.join(self.state_dir, "test-config.json")
        os.makedirs(self.state_dir, exist_ok=True)
        with open(cfg_path, "w") as fh:
            json.dump(cfg, fh)
        argv = ["--config", cfg_path] + (["--dry-run"] if dry_run else [])
        runs_dir = os.path.join(self.state_dir, "runs")
        before = set(os.listdir(runs_dir)) if os.path.isdir(runs_dir) else set()
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = wr.main(argv)
        assert rc == 0
        decisions = {}
        if os.path.isdir(runs_dir):
            new_logs = sorted(set(os.listdir(runs_dir)) - before)
            if new_logs:
                with open(os.path.join(runs_dir, new_logs[-1])) as fh:
                    for line in fh:
                        obj = json.loads(line)
                        if obj.get("type") == "decision":
                            decisions[obj["tree"]] = obj
        return decisions


class GateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.mkdtemp(prefix="wtreaper-test-")
        cls.repo = SyntheticRepo(cls.tmp)
        cls.state = os.path.join(cls.tmp, "state")
        cls.runner = ReaperRun(cls.repo, cls.state)
        cls.tree_pushed_clean = cls.repo.branch_tree("t/pushed-clean", push=True)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def test_01_dry_run_lists_but_keeps(self):
        tree = self.tree_pushed_clean
        decisions = self.runner.run(dry_run=True)
        self.assertEqual(decisions[tree]["action"], "would-reap")
        self.assertTrue(os.path.isdir(tree))

    def test_02_kill_switch(self):
        open(os.path.join(self.state, "DISABLED"), "w").close()
        try:
            decisions = self.runner.run()
        finally:
            os.unlink(os.path.join(self.state, "DISABLED"))
        self.assertEqual(decisions, {})  # no run log written at all
        self.assertTrue(os.path.isdir(self.tree_pushed_clean))

    def test_03_reap_matrix(self):
        repo, run = self.repo, self.runner.run
        t_unpushed = repo.branch_tree("t/unpushed")
        t_merged = repo.branch_tree("t/merged", merge_to_main=True)
        t_dirty_tracked = repo.branch_tree("t/dirty-tracked", push=True)
        repo.write(t_dirty_tracked, "base.txt", "tampered\n")
        t_dirty_untracked = repo.branch_tree("t/dirty-untracked", push=True)
        repo.write(t_dirty_untracked, "notes.txt", "real work\n")
        t_logs = repo.branch_tree("t/logs-only", push=True)
        repo.write(t_logs, "pkgs/mcpx/internal/cli/logs/run.log", "log\n")
        repo.write(t_logs, "fleet-feed.json", "{}\n")
        repo.write(t_logs, "pm-stack/aggregator/fleet-feed.json", "{}\n")
        # Patch-equivalent: branch commit cherry-picked onto main, branch
        # itself never pushed and not an ancestor anywhere. The pick gets
        # a pinned, distinct committer date: with the default "now" the
        # pick can land in the same second as the original commit and
        # produce a byte-identical SHA (a fast-forward in disguise),
        # which would prove containment instead of patch-equivalence.
        t_patch = repo.branch_tree("t/patch-equiv")
        patch_tip = git(repo.primary, "rev-parse", "t/patch-equiv").stdout.strip()
        git(repo.primary, "cherry-pick", "--quiet", patch_tip,
            env_extra={"GIT_COMMITTER_DATE": "2020-01-01T00:00:00 +0000"})
        picked = git(repo.primary, "rev-parse", "main").stdout.strip()
        self.assertNotEqual(picked, patch_tip)
        git(repo.primary, "push", "--quiet", "origin", "main")
        t_detached = os.path.join(repo.trees, "detached")
        git(repo.primary, "worktree", "add", "--quiet", "--detach", t_detached, "main")
        t_denied = repo.branch_tree("t/denied", push=True)
        t_occupied = repo.branch_tree("t/occupied", push=True)
        sleeper = subprocess.Popen(["sleep", "60"], cwd=t_occupied)
        try:
            decisions = run(deny_paths=[t_denied])
        finally:
            sleeper.terminate()

        reap = self.tree_pushed_clean
        self.assertFalse(os.path.isdir(reap), "pushed+clean tree should be reaped")
        self.assertFalse(repo.branch_exists("t/pushed-clean"),
                         "merged-to-origin branch should be -d deleted")
        self.assertFalse(os.path.isdir(t_merged))
        self.assertFalse(repo.branch_exists("t/merged"))
        self.assertFalse(os.path.isdir(t_logs), "allowlisted-dirt tree should be force-reaped")
        self.assertFalse(os.path.isdir(t_patch), "patch-equivalent tree should be reaped")
        self.assertTrue(decisions[t_patch]["gates"]["pushed"]["method"].startswith("patch-equivalent"))
        self.assertTrue(repo.branch_exists("t/patch-equiv"),
                        "-d must refuse the not-locally-merged branch; reaper keeps it")

        for tree, reason in ((t_unpushed, "unpushed"), (t_dirty_tracked, "dirty"),
                             (t_dirty_untracked, "dirty"), (t_detached, "detached"),
                             (t_denied, "denied:deny-path")):
            self.assertTrue(os.path.isdir(tree), f"{tree} must survive")
            self.assertEqual(decisions[tree]["action"], "keep")
            self.assertEqual(decisions[tree]["reason"], reason)
        self.assertTrue(os.path.isdir(t_occupied), "occupied tree must survive")
        self.assertEqual(decisions[t_occupied]["action"], "keep")
        # Caught either at evaluation or at the pre-removal re-scan.
        self.assertTrue(decisions[t_occupied]["reason"].startswith("occupied"))

    def test_04_quiet_gate(self):
        tree = self.repo.branch_tree("t/fresh", push=True)
        decisions = self.runner.run(quiet_hours=100000)
        self.assertTrue(os.path.isdir(tree))
        self.assertEqual(decisions[tree]["reason"], "active")
        decisions = self.runner.run(quiet_hours=0)
        self.assertFalse(os.path.isdir(tree))


class UnitTests(unittest.TestCase):
    def test_porcelain_parser(self):
        raw = (
            b"worktree /a/primary\0HEAD aaa\0branch refs/heads/main\0\0"
            b"worktree /a/fleet/x\0HEAD bbb\0detached\0\0"
            b"worktree /a/fleet/y\0HEAD ccc\0branch refs/heads/queue/1-2-z\0locked reason\0"
        )
        entries = wr.parse_worktree_porcelain(raw)
        self.assertEqual(len(entries), 3)
        self.assertEqual(entries[0]["branch"], "main")
        self.assertTrue(entries[1]["detached"])
        self.assertIsNone(entries[1]["branch"])
        self.assertEqual(entries[2]["branch"], "queue/1-2-z")
        self.assertTrue(entries[2]["locked"])

    def test_dirt_allowlisted(self):
        cfg = {"generated_dirt_prefixes": ["pkgs/mcpx/internal/cli/logs/"],
               "generated_dirt_files": ["fleet-feed.json"]}
        allow, other = wr.dirt_allowlisted(
            ["pkgs/mcpx/internal/cli/logs/a.log", "fleet-feed.json",
             "pm-stack/aggregator/fleet-feed.json", "src/real.py"], cfg)
        self.assertEqual(len(allow), 3)
        self.assertEqual(other, ["src/real.py"])

    def test_walk_activity(self):
        root = tempfile.mkdtemp(prefix="wtreaper-walk-")
        try:
            path = os.path.join(root, "f.txt")
            with open(path, "w") as fh:
                fh.write("x")
            now = time.time()
            mtime, _size, complete = wr.walk_activity(root, now + 10)
            self.assertTrue(complete)
            self.assertLessEqual(mtime, now)
            _m, _s, complete2 = wr.walk_activity(root, now - 10)
            self.assertFalse(complete2, "fresh file must early-exit as not-quiet")
            old = now - 100000
            os.utime(path, (old, old))
            os.utime(root, (old, old))
            mtime3, _s3, complete3 = wr.walk_activity(root, now - 1000)
            self.assertTrue(complete3)
            self.assertAlmostEqual(mtime3, old, delta=2)
        finally:
            shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    unittest.main(verbosity=2)
