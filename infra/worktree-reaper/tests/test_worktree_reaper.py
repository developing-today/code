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

import argparse
import contextlib
import fcntl
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
from unittest import mock

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
    proc = subprocess.run(
        ["git", "-C", repo, *args],
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if check and proc.returncode != 0:
        raise AssertionError(f"git -C {repo} {' '.join(args)} failed: {proc.stderr}")
    return proc


def gitto(*args):
    proc = subprocess.run(
        ["git", *args],
        env=GIT_ENV,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
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
        rc, out, _ = wr.run_git(
            self.primary, "symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"
        )
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
            git(
                self.primary,
                "merge",
                "--quiet",
                "--no-ff",
                branch,
                "-m",
                f"merge {branch}",
            )
            git(self.primary, "push", "--quiet", "origin", "main")
        if push:
            git(self.primary, "push", "--quiet", "-u", "origin", branch)
        git(self.primary, "checkout", "--quiet", "main")
        return tree

    def branch_exists(self, branch):
        rc, _, _ = wr.run_git(
            self.primary, "rev-parse", "--verify", "--quiet", f"refs/heads/{branch}"
        )
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
            "ignored_dirt_prefixes": ["target/", "node_modules/"],
            "ignored_dirt_files": [],
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
        git(
            repo.primary,
            "cherry-pick",
            "--quiet",
            patch_tip,
            env_extra={"GIT_COMMITTER_DATE": "2020-01-01T00:00:00 +0000"},
        )
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
        self.assertFalse(
            repo.branch_exists("t/pushed-clean"),
            "merged-to-origin branch should be -d deleted",
        )
        self.assertFalse(os.path.isdir(t_merged))
        self.assertFalse(repo.branch_exists("t/merged"))
        self.assertFalse(
            os.path.isdir(t_logs), "allowlisted-dirt tree should be force-reaped"
        )
        self.assertFalse(
            os.path.isdir(t_patch), "patch-equivalent tree should be reaped"
        )
        self.assertTrue(
            decisions[t_patch]["gates"]["pushed"]["method"].startswith(
                "patch-equivalent"
            )
        )
        self.assertTrue(
            repo.branch_exists("t/patch-equiv"),
            "-d must refuse the not-locally-merged branch; reaper keeps it",
        )

        for tree, reason in (
            (t_unpushed, "unpushed"),
            (t_dirty_tracked, "dirty"),
            (t_dirty_untracked, "dirty"),
            (t_detached, "detached"),
            (t_denied, "denied:deny-path"),
        ):
            self.assertTrue(os.path.isdir(tree), f"{tree} must survive")
            self.assertEqual(decisions[tree]["action"], "keep")
            self.assertEqual(decisions[tree]["reason"], reason)
        self.assertTrue(os.path.isdir(t_occupied), "occupied tree must survive")
        self.assertEqual(decisions[t_occupied]["action"], "keep")
        # Caught either at evaluation or at the pre-removal re-scan.
        self.assertTrue(decisions[t_occupied]["reason"].startswith("occupied"))

    def _tree_with_ignored(self, branch, rule, ignored_rel):
        """A pushed tree whose branch commits a .gitignore rule, with an
        ignored file present in the working tree."""
        tree = self.repo.branch_tree(branch, push=True)
        self.repo.write(tree, ".gitignore", rule + "\n")
        git(tree, "add", ".gitignore")
        git(tree, "commit", "--quiet", "-m", "ignore rule")
        git(tree, "push", "--quiet", "origin", branch)
        self.repo.write(tree, ignored_rel, "local state\n")
        return tree

    def test_05_ignored_gate(self):
        # Ignored-only tree: the CLEAN gate must see the ignored file
        # and keep the tree (#6734 — before the fix, status_entries
        # returned ([], []) for this tree and it reaped as clean).
        t_ignored = self._tree_with_ignored(
            "t/ignored-only", "local-state.db", "local-state.db"
        )
        tracked, untracked, ignored = wr.status_entries(t_ignored)
        self.assertEqual((tracked, untracked), ([], []))
        self.assertEqual(ignored, ["local-state.db"])

        # Allowlisted untracked dirt does not launder an unlisted
        # ignored file: the tree is still kept.
        t_mixed = self._tree_with_ignored(
            "t/untracked-plus-ignored", "notes.db", "notes.db"
        )
        self.repo.write(t_mixed, "fleet-feed.json", "{}\n")

        # Ignored build output under an allowlisted prefix reaps, and
        # the record names the ignored paths it deleted.
        t_target = self._tree_with_ignored(
            "t/ignored-target", "target/", "target/build.o"
        )

        decisions = self.runner.run()
        for tree in (t_ignored, t_mixed):
            self.assertTrue(os.path.isdir(tree), f"{tree} must survive")
            self.assertEqual(decisions[tree]["action"], "keep")
            self.assertEqual(decisions[tree]["reason"], "dirty")
        self.assertIn(
            "local-state.db", decisions[t_ignored]["gates"]["clean"]["ignored_other"]
        )
        self.assertIn("notes.db", decisions[t_mixed]["gates"]["clean"]["ignored_other"])

        self.assertFalse(
            os.path.isdir(t_target), "allowlisted ignored dirt should reap"
        )
        self.assertEqual(decisions[t_target]["action"], "reaped")
        self.assertTrue(
            any("target" in p for p in decisions[t_target]["ignored_paths"]),
            f"reaped tree's record must name its ignored paths: {decisions[t_target]}",
        )

    def test_04_quiet_gate(self):
        tree = self.repo.branch_tree("t/fresh", push=True)
        decisions = self.runner.run(quiet_hours=100000)
        self.assertTrue(os.path.isdir(tree))
        self.assertEqual(decisions[tree]["reason"], "active")
        decisions = self.runner.run(quiet_hours=0)
        self.assertFalse(os.path.isdir(tree))


class RunLogCollisionTests(unittest.TestCase):
    """#5716: two runs in the same second must not share a run log."""

    def test_two_runs_same_second_both_logged(self):
        tmp = tempfile.mkdtemp(prefix="wtreaper-collision-")
        try:
            repo = SyntheticRepo(tmp)
            runner = ReaperRun(repo, os.path.join(tmp, "state"))
            tree = repo.branch_tree("t/fresh", push=True)
            first = runner.run(quiet_hours=100000)
            second = runner.run(quiet_hours=100000)
            self.assertEqual(first[tree]["reason"], "active")
            self.assertEqual(second[tree]["reason"], "active")
            logs = os.listdir(os.path.join(tmp, "state", "runs"))
            self.assertEqual(len(logs), 2, logs)
        finally:
            shutil.rmtree(tmp, ignore_errors=True)


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

    def test_ignored_allowlisted(self):
        cfg = {
            "ignored_dirt_prefixes": ["target/", "node_modules/"],
            "ignored_dirt_files": ["scratch.db"],
        }
        allow, other = wr.ignored_allowlisted(
            [
                "target/",
                "target/build.o",
                "pkgs/id/target/deps/x.o",
                "node_modules/pkg/index.js",
                "data/scratch.db",
                "pkgs/id/.identity.db",
                "local-state.db",
            ],
            cfg,
        )
        self.assertEqual(len(allow), 5)
        self.assertEqual(other, ["pkgs/id/.identity.db", "local-state.db"])

    def test_dirt_allowlisted(self):
        cfg = {
            "generated_dirt_prefixes": ["pkgs/mcpx/internal/cli/logs/"],
            "generated_dirt_files": ["fleet-feed.json"],
        }
        allow, other = wr.dirt_allowlisted(
            [
                "pkgs/mcpx/internal/cli/logs/a.log",
                "fleet-feed.json",
                "pm-stack/aggregator/fleet-feed.json",
                "src/real.py",
            ],
            cfg,
        )
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


class ConfigValidationTests(unittest.TestCase):
    """#6394: load_config validates both config files, naming the file.

    Each invalid case from the issue must yield a clean ConfigError
    (never a silent disarm, never a raw traceback): a typo'd deny
    key, a string-typed deny list, a wrong-shaped top level,
    malformed JSON, and a string-typed numeric. A valid config and
    a valid overlay merge exactly as before.
    """

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="reaper-config-test-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.state_dir = os.path.join(self.tmp, "state")
        os.makedirs(self.state_dir)

    def _args(self, config_path=None):
        return argparse.Namespace(
            config=config_path,
            state_dir=self.state_dir,
            quiet_hours=None,
            max_removals=None,
            no_fetch=False,
        )

    def _write(self, name, text):
        path = os.path.join(self.tmp, name)
        with open(path, "w") as fh:
            fh.write(text)
        return path

    def _load(self, config_path=None):
        return wr.load_config(self.tmp, self._args(config_path))

    def test_typo_deny_key_is_refused_naming_the_file(self):
        path = self._write("config.json", json.dumps({"deny_path": ["/x"]}))
        with self.assertRaises(wr.ConfigError) as ctx:
            self._load(path)
        self.assertIn(path, str(ctx.exception))
        self.assertIn("deny_path", str(ctx.exception))

    def test_string_typed_deny_paths_is_refused(self):
        path = self._write(
            "config.json", json.dumps({"deny_paths": "/home/user/git/important"})
        )
        with self.assertRaises(wr.ConfigError) as ctx:
            self._load(path)
        self.assertIn("deny_paths", str(ctx.exception))

    def test_top_level_list_is_refused_cleanly(self):
        path = self._write("config.json", json.dumps(["deny_paths"]))
        with self.assertRaises(wr.ConfigError) as ctx:
            self._load(path)
        self.assertIn("must be a JSON object", str(ctx.exception))

    def test_malformed_json_is_refused_naming_the_file(self):
        path = self._write("bad.json", "{oops")
        with self.assertRaises(wr.ConfigError) as ctx:
            self._load(path)
        self.assertIn(path, str(ctx.exception))
        self.assertIn("invalid JSON", str(ctx.exception))

    def test_string_typed_quiet_hours_is_refused_at_load(self):
        path = self._write("config.json", json.dumps({"quiet_hours": "6"}))
        with self.assertRaises(wr.ConfigError) as ctx:
            self._load(path)
        self.assertIn("quiet_hours", str(ctx.exception))

    def test_overlay_typo_key_is_refused_naming_the_overlay(self):
        overlay = os.path.join(self.state_dir, "config.local.json")
        with open(overlay, "w") as fh:
            json.dump({"deny_path": ["/x"]}, fh)
        with self.assertRaises(wr.ConfigError) as ctx:
            self._load()
        self.assertIn(overlay, str(ctx.exception))

    def test_overlay_top_level_list_is_refused_cleanly(self):
        overlay = os.path.join(self.state_dir, "config.local.json")
        with open(overlay, "w") as fh:
            json.dump(["x"], fh)
        with self.assertRaises(wr.ConfigError) as ctx:
            self._load()
        self.assertIn(overlay, str(ctx.exception))

    def test_valid_config_and_overlay_merge_unchanged(self):
        path = self._write(
            "config.json",
            json.dumps({"quiet_hours": 2, "deny_paths": ["/a"], "fetch": False}),
        )
        overlay = os.path.join(self.state_dir, "config.local.json")
        with open(overlay, "w") as fh:
            json.dump({"deny_paths": ["/b"], "max_removals": 5}, fh)
        cfg = self._load(path)
        self.assertEqual(cfg["quiet_hours"], 2)
        self.assertEqual(cfg["deny_paths"], ["/a", "/b"])
        self.assertEqual(cfg["max_removals"], 5)
        self.assertFalse(cfg["fetch"])


class ProcScanFailClosedTests(unittest.TestCase):
    """#5740: a /proc scan failure is UNKNOWN occupancy — fail closed."""

    def test_proc_cwds_raises_on_listdir_failure(self):
        with mock.patch("os.listdir", side_effect=OSError("proc gone")):
            with self.assertRaises(wr.ProcScanError):
                wr.proc_cwds()

    def test_successful_empty_scan_is_unoccupied(self):
        root = tempfile.mkdtemp(prefix="wtreaper-empty-scan-")
        try:
            with mock.patch("os.listdir", return_value=[]):
                self.assertEqual(wr.proc_cwds(), [])
                self.assertFalse(wr.tree_occupied(root))
        finally:
            shutil.rmtree(root, ignore_errors=True)

    def test_confirmed_matching_process_still_occupied(self):
        root = tempfile.mkdtemp(prefix="wtreaper-occupied-")
        try:
            self.assertTrue(wr.tree_occupied(root, cwds=[root]))
            self.assertFalse(wr.tree_occupied(root, cwds=["/nonexistent-elsewhere"]))
        finally:
            shutil.rmtree(root, ignore_errors=True)

    def test_reap_tree_retains_candidate_and_never_calls_git(self):
        root = tempfile.mkdtemp(prefix="wtreaper-failclosed-")
        try:
            rec = {"tree": root, "branch": "queue/test", "action": "candidate"}
            git_calls = []

            def fake_run_git(*args, **kwargs):
                git_calls.append(args)
                return 0, "", ""

            with (
                mock.patch("os.listdir", side_effect=OSError("proc gone")),
                mock.patch.object(wr, "run_git", fake_run_git),
                contextlib.redirect_stdout(io.StringIO()) as out,
            ):
                result = wr.reap_tree("/nonexistent-primary", rec, {})
            self.assertFalse(result)
            self.assertEqual(rec["action"], "keep")
            self.assertEqual(rec["reason"], "occupied-unknown-at-removal")
            self.assertEqual(git_calls, [], "no git worktree/branch call may run")
            self.assertTrue(os.path.isdir(root), "candidate tree must be retained")
            self.assertIn("WARNING", out.getvalue())
        finally:
            shutil.rmtree(root, ignore_errors=True)


class FdOccupancyTests(unittest.TestCase):
    """#6898: occupancy must see open fds and cmdline paths, not only cwds."""

    def _wait_occupied(self, root, want=True):
        deadline = time.time() + 10
        while time.time() < deadline:
            if wr.tree_occupied(root) == want:
                return True
            time.sleep(0.1)
        return wr.tree_occupied(root) == want

    def test_open_fd_in_tree_occupies_with_cwd_outside(self):
        root = tempfile.mkdtemp(prefix="wtreaper-fd-")
        holder = None
        try:
            held = os.path.join(root, "held.txt")
            with open(held, "w") as fh:
                fh.write("x")
            # The path reaches the child via the environment, so its
            # cmdline carries no reference to the tree: only the open
            # fd can give it away. Its cwd is outside the tree.
            holder = subprocess.Popen(
                [
                    sys.executable,
                    "-c",
                    "import os, time; fh = open(os.environ['WR_HELD']); time.sleep(60)",
                ],
                cwd="/",
                env={**os.environ, "WR_HELD": held},
            )
            self.assertTrue(
                self._wait_occupied(root),
                "a process holding an open file in the tree must occupy it",
            )
        finally:
            if holder is not None:
                holder.terminate()
                holder.wait(timeout=10)
            shutil.rmtree(root, ignore_errors=True)

    def test_cmdline_path_in_tree_occupies(self):
        root = tempfile.mkdtemp(prefix="wtreaper-cmdline-")
        holder = None
        try:
            target = os.path.join(root, "mentioned.txt")
            # The child never opens the file; the tree path appears
            # only as an argv token in its cmdline.
            holder = subprocess.Popen(
                [sys.executable, "-c", "import time; time.sleep(60)", target], cwd="/"
            )
            self.assertTrue(
                self._wait_occupied(root),
                "a cmdline path inside the tree must occupy it",
            )
        finally:
            if holder is not None:
                holder.terminate()
                holder.wait(timeout=10)
            shutil.rmtree(root, ignore_errors=True)

    def test_reap_tree_recheck_sees_fd_holder(self):
        root = tempfile.mkdtemp(prefix="wtreaper-fd-reap-")
        holder = None
        try:
            held = os.path.join(root, "held.txt")
            with open(held, "w") as fh:
                fh.write("x")
            holder = subprocess.Popen(
                [
                    sys.executable,
                    "-c",
                    "import os, time; fh = open(os.environ['WR_HELD']); time.sleep(60)",
                ],
                cwd="/",
                env={**os.environ, "WR_HELD": held},
            )
            self.assertTrue(self._wait_occupied(root))
            rec = {"tree": root, "branch": "queue/test", "action": "candidate"}
            git_calls = []

            def fake_run_git(*args, **kwargs):
                git_calls.append(args)
                return 0, "", ""

            with (
                mock.patch.object(wr, "run_git", fake_run_git),
                contextlib.redirect_stdout(io.StringIO()),
            ):
                result = wr.reap_tree("/nonexistent-primary", rec, {})
            self.assertFalse(result)
            self.assertEqual(rec["reason"], "occupied-at-removal")
            self.assertEqual(
                git_calls, [], "no git call may run while an fd holder lives"
            )
            self.assertTrue(os.path.isdir(root), "candidate tree must be retained")
        finally:
            if holder is not None:
                holder.terminate()
                holder.wait(timeout=10)
            shutil.rmtree(root, ignore_errors=True)


class RemovalRecheckTests(unittest.TestCase):
    """#6899: reap_tree re-verifies QUIET and PUSHED immediately before
    removal. Oracle: a tree whose gates changed after evaluation —
    a fresh commit (tip unpushed), or fresh activity on a pushed tip —
    must still exist, branch included, after reap_tree runs."""

    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.mkdtemp(prefix="wtreaper-recheck-")
        cls.repo = SyntheticRepo(cls.tmp)
        cls.state = os.path.join(cls.tmp, "state")

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def _cfg(self, quiet_hours=0):
        return {
            "git_root": self.repo.git_root,
            "state_dir": self.state,
            "quiet_hours": quiet_hours,
            "max_removals": 100,
            "deny_paths": [],
            "deny_prefixes": [],
            "deny_names": [],
            "generated_dirt_prefixes": ["pkgs/mcpx/internal/cli/logs/"],
            "generated_dirt_files": ["fleet-feed.json"],
            "ignored_dirt_prefixes": ["target/", "node_modules/"],
            "ignored_dirt_files": [],
            "fetch": False,
        }

    def _candidate(self, branch):
        tree = self.repo.branch_tree(branch, push=True)
        wt = next(w for w in wr.list_worktrees(self.repo.primary) if w["path"] == tree)
        rec = wr.evaluate_tree(self.repo.primary, wt, self._cfg(), time.time(), [])
        self.assertEqual(rec["action"], "candidate", rec)
        return tree, rec

    def test_commit_between_evaluation_and_removal_keeps_tree(self):
        tree, rec = self._candidate("t/late-commit")
        # The agent resumes and commits after evaluation; the working
        # tree stays clean, so only the PUSHED re-verification can
        # see that the tip moved and is no longer proven on origin.
        self.repo.write(tree, "late.txt", "new work\n")
        git(tree, "add", ".")
        git(tree, "commit", "--quiet", "-m", "late commit")
        self.assertFalse(wr.reap_tree(self.repo.primary, rec, self._cfg()))
        self.assertEqual(rec["action"], "keep")
        self.assertEqual(rec["reason"], "unpushed-at-removal")
        self.assertTrue(os.path.isdir(tree))
        self.assertTrue(self.repo.branch_exists("t/late-commit"))

    def test_pushed_but_active_at_removal_keeps_tree(self):
        tree, rec = self._candidate("t/pushed-active")
        # Commit AND push after evaluation: PUSHED still holds at
        # removal, so only the QUIET re-verification can retain it.
        self.repo.write(tree, "more.txt", "more work\n")
        git(tree, "add", ".")
        git(tree, "commit", "--quiet", "-m", "pushed follow-up")
        git(tree, "push", "--quiet", "origin", "t/pushed-active")
        cfg = self._cfg(quiet_hours=100000)
        self.assertFalse(wr.reap_tree(self.repo.primary, rec, cfg))
        self.assertEqual(rec["action"], "keep")
        self.assertEqual(rec["reason"], "active-at-removal")
        self.assertTrue(os.path.isdir(tree))


class SingleInstanceLockTests(unittest.TestCase):
    """#6899: a held flock in the state dir makes a run a no-op.
    Oracle: while the test holds the lock, main() writes no run log
    and the candidate tree survives; after release, the same run
    reaps it — the lock was the only blocker."""

    def test_held_lock_makes_second_run_noop(self):
        tmp = tempfile.mkdtemp(prefix="wtreaper-lock-")
        try:
            repo = SyntheticRepo(tmp)
            state = os.path.join(tmp, "state")
            runner = ReaperRun(repo, state)
            tree = repo.branch_tree("t/lock-held", push=True)
            decisions = runner.run(dry_run=True)
            self.assertEqual(decisions[tree]["action"], "would-reap")
            lock_path = os.path.join(state, wr.LOCK_FILENAME)
            fd = os.open(lock_path, os.O_CREAT | os.O_RDWR, 0o644)
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            try:
                decisions = runner.run()
            finally:
                fcntl.flock(fd, fcntl.LOCK_UN)
                os.close(fd)
            self.assertEqual(decisions, {}, "locked run must not even evaluate")
            self.assertTrue(os.path.isdir(tree), "locked run must remove nothing")
            decisions = runner.run()
            self.assertFalse(
                os.path.isdir(tree), "after the lock is released the run must reap"
            )
        finally:
            shutil.rmtree(tmp, ignore_errors=True)


class StaleRegistrationTests(unittest.TestCase):
    """#6547: a stale registration (worktree directory deleted out from
    under git) is prune work on its own — no same-run candidate reap
    may be required to trigger `git worktree prune`, and a dry run
    must report it as would-prune while changing nothing on disk."""

    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.mkdtemp(prefix="wtreaper-stale-")
        cls.repo = SyntheticRepo(cls.tmp)
        cls.state = os.path.join(cls.tmp, "state")
        cls.runner = ReaperRun(cls.repo, cls.state)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def registered_paths(self):
        rc, out, _ = wr.run_git(self.repo.primary, "worktree", "list", "--porcelain")
        assert rc == 0
        return [
            line.split(" ", 1)[1]
            for line in out.decode().splitlines()
            if line.startswith("worktree ")
        ]

    def last_summary(self):
        with open(os.path.join(self.state, "last-run.json")) as fh:
            return json.load(fh)

    def test_01_dry_run_reports_would_prune_and_changes_nothing(self):
        tree = self.repo.branch_tree("t/stale-dry", push=True)
        shutil.rmtree(tree)
        assert tree in self.registered_paths()
        decisions = self.runner.run(dry_run=True)
        assert decisions[tree]["action"] == "would-prune", decisions[tree]
        summary = self.last_summary()
        assert summary["stale_registrations"] == 1, summary
        assert summary["pruned"] == 0, summary
        assert "stale-registration" not in summary["kept"], summary
        assert tree in self.registered_paths()

    def test_02_non_dry_run_prunes_deleted_directory_only_primary(self):
        tree = self.repo.branch_tree("t/stale-real", push=True)
        shutil.rmtree(tree)
        assert tree in self.registered_paths()
        decisions = self.runner.run()
        assert decisions[tree]["action"] == "stale-registration", decisions[tree]
        summary = self.last_summary()
        # The dry-run test's registration is still stale too (that test
        # proved a dry run changes nothing), so the count is >= 1; this
        # run prunes the primary once, clearing both.
        assert summary["stale_registrations"] >= 1, summary
        assert summary["pruned"] == 1, summary
        assert summary["errors"] == [], summary
        assert tree not in self.registered_paths()


if __name__ == "__main__":
    unittest.main(verbosity=2)
