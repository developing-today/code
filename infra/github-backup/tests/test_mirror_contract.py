#!/usr/bin/env python3
"""Regression tests for the #3282 mirror contract remainder:
#3238 (validity is resolvability), #3270 (missing mirrors reach the
chronic alarm), and #3240 (atomic writes, tolerant loads, shared
state lock).

Oracles:
- #3238: the design's rule 1 — a mirror is valid iff it is bare AND
  its HEAD resolves; a bare skeleton (the state `git clone --mirror`
  creates before transport) holds zero refs by construction.
- #3270: the fetch docstring's alarm contract — 3 consecutive
  unavailable days exit non-zero; missing is an unavailable day,
  recorded through the same fail_days streak as failures (#3257).
- #3240: a reader never observes a partial file (temp + fsync +
  os.replace), and a torn file falls back loudly (last-good copy or
  empty), never a startup JSONDecodeError.

Run: python3 infra/github-backup/tests/test_mirror_contract.py
"""

import contextlib
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

BIN = os.path.abspath(
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "bin")
)
sys.path.insert(0, BIN)

import ghb_common
import clone_all
import fetch_mirrors
import manifest as manifest_mod


def git(*args, cwd=None):
    return subprocess.run(
        ["git", *args], cwd=cwd, capture_output=True, text=True, check=True
    )


def make_complete_mirror(root):
    """A small source repo + a real `git clone --mirror` of it."""
    src = os.path.join(root, "src")
    os.makedirs(src)
    git("init", "-b", "main", cwd=src)
    git("config", "user.email", "t@example.invalid", cwd=src)
    git("config", "user.name", "t", cwd=src)
    with open(os.path.join(src, "f.txt"), "w") as f:
        f.write("hello\n")
    git("add", "f.txt", cwd=src)
    git("commit", "-m", "one", cwd=src)
    dest = os.path.join(root, "owner", "repo.git")
    os.makedirs(os.path.dirname(dest))
    git("clone", "--mirror", src, dest)
    return dest


def make_skeleton(root):
    """The bare skeleton a killed mirror clone leaves (#3238)."""
    dest = os.path.join(root, "owner", "repo.git")
    os.makedirs(os.path.dirname(dest))
    git("init", "--bare", dest)
    return dest


class ValidMirrorTests(unittest.TestCase):
    def test_bare_skeleton_is_invalid(self):
        with tempfile.TemporaryDirectory() as root:
            dest = make_skeleton(root)
            self.assertFalse(ghb_common.valid_mirror(dest))
            self.assertTrue(ghb_common.is_skeleton_mirror(dest))

    def test_complete_mirror_is_valid(self):
        with tempfile.TemporaryDirectory() as root:
            dest = make_complete_mirror(root)
            self.assertTrue(ghb_common.valid_mirror(dest))
            self.assertFalse(ghb_common.is_skeleton_mirror(dest))

    def test_non_bare_and_missing_are_invalid(self):
        with tempfile.TemporaryDirectory() as root:
            self.assertFalse(ghb_common.valid_mirror(os.path.join(root, "nope")))
            plain = os.path.join(root, "plain")
            os.makedirs(plain)
            git("init", "-b", "main", cwd=plain)
            self.assertFalse(ghb_common.valid_mirror(plain))
            self.assertFalse(ghb_common.is_skeleton_mirror(plain))

    def test_skeleton_is_confirmed_partial_complete_is_not(self):
        # clone_all may clear a skeleton and re-clone (#3238 +
        # design rule 2); a complete mirror must never be cleared.
        with tempfile.TemporaryDirectory() as root:
            skeleton = make_skeleton(root)
            self.assertTrue(clone_all.dest_is_confirmed_partial(skeleton))
        with tempfile.TemporaryDirectory() as root:
            dest = make_complete_mirror(root)
            self.assertFalse(clone_all.dest_is_confirmed_partial(dest))

    def test_manifest_reports_skeleton_missing(self):
        with tempfile.TemporaryDirectory() as root:
            dest = make_skeleton(root)
            plan = [{"repo": "owner/repo", "size_kb": 1}]
            with open(os.path.join(root, "plan.json"), "w") as f:
                json.dump(plan, f)
            with (
                patch.object(manifest_mod, "load_config", return_value={}),
                patch.object(
                    manifest_mod,
                    "mirror_path",
                    side_effect=lambda _r, _repo: dest,
                ),
                contextlib.redirect_stderr(io.StringIO()),
            ):
                m = manifest_mod.build(root)
            self.assertEqual(m["mirrors"]["owner/repo"]["status"], "missing")
            self.assertEqual(m["totals"], {"mirrored": 0, "missing": 1, "disk_kb": 0})


class MissingAlarmTests(unittest.TestCase):
    def run_fetch_repo(self, state, day, valid, outcome=None):
        def fake_strftime(fmt):
            if fmt == "%Y-%m-%d":
                return day
            return day + "T12:00:00+0000"

        patches = [
            patch.object(fetch_mirrors, "mirror_path", return_value="/mirror"),
            patch.object(fetch_mirrors, "valid_mirror", return_value=valid),
            patch.object(fetch_mirrors.time, "strftime", side_effect=fake_strftime),
        ]
        if outcome is not None:
            patches.append(
                patch.object(
                    fetch_mirrors.subprocess,
                    "run",
                    return_value=SimpleNamespace(
                        returncode=0 if outcome == "ok" else 1,
                        stdout="",
                        stderr="" if outcome == "ok" else "boom",
                    ),
                )
            )
        with contextlib.ExitStack() as stack:
            for p in patches:
                stack.enter_context(p)
            return fetch_mirrors.fetch_repo("/root", "owner/repo", state)

    def test_missing_days_accumulate_on_the_failure_streak(self):
        state = {}
        for day in ("2026-01-05", "2026-01-06", "2026-01-07"):
            self.assertEqual(self.run_fetch_repo(state, day, valid=False), "missing")
        self.assertEqual(
            state["owner/repo"]["fail_days"],
            ["2026-01-05", "2026-01-06", "2026-01-07"],
        )
        self.assertIn("missing", state["owner/repo"]["last_error"])

    def test_missing_streak_reaches_chronic_exit(self):
        """End to end through main(): three missing days -> exit 1."""
        with tempfile.TemporaryDirectory() as root:
            day = [None]

            def fake_strftime(fmt):
                if fmt == "%Y-%m-%d":
                    return day[0]
                return day[0] + "T12:00:00+0000"

            codes = []
            with (
                patch.object(fetch_mirrors, "load_config", return_value={}),
                patch.object(fetch_mirrors, "backup_root", return_value=root),
                patch.object(fetch_mirrors, "mirror_path", return_value="/mirror"),
                patch.object(fetch_mirrors, "valid_mirror", return_value=False),
                patch.object(fetch_mirrors.manifest_mod, "build"),
                patch.object(fetch_mirrors.time, "strftime", side_effect=fake_strftime),
                contextlib.redirect_stdout(io.StringIO()),
            ):
                for d in ("2026-01-05", "2026-01-06", "2026-01-07"):
                    day[0] = d
                    codes.append(fetch_mirrors.main(["fetch_mirrors.py", "owner/repo"]))
            self.assertEqual(codes, [0, 0, 1])

    def test_return_to_ok_clears_the_missing_record(self):
        state = {}
        for day in ("2026-01-05", "2026-01-06"):
            self.run_fetch_repo(state, day, valid=False)
        res = self.run_fetch_repo(state, "2026-01-07", valid=True, outcome="ok")
        self.assertEqual(res, "ok")
        self.assertEqual(state["owner/repo"]["fail_days"], [])


class AtomicStateTests(unittest.TestCase):
    def test_save_json_roundtrip_and_last_good(self):
        with tempfile.TemporaryDirectory() as root:
            path = os.path.join(root, "state.json")
            ghb_common.save_json(path, {"a/b": {"fail_days": []}})
            ghb_common.save_json(path, {"a/b": {"fail_days": ["2026-01-05"]}})
            with open(path) as f:
                self.assertEqual(json.load(f), {"a/b": {"fail_days": ["2026-01-05"]}})
            with open(path + ghb_common.LAST_GOOD_SUFFIX) as f:
                self.assertEqual(json.load(f), {"a/b": {"fail_days": []}})

    def test_torn_file_falls_back_to_last_good_loudly(self):
        with tempfile.TemporaryDirectory() as root:
            path = os.path.join(root, "state.json")
            ghb_common.save_json(path, {"a/b": {"v": 1}})
            ghb_common.save_json(path, {"a/b": {"v": 2}})
            with open(path, "w") as f:
                f.write('{"a/b": {"v": ')  # torn mid-object
            value, problems = ghb_common.load_json_tolerant(path, {}, "state.json")
            self.assertEqual(value, {"a/b": {"v": 1}})
            self.assertTrue(any("torn" in p for p in problems))

    def test_torn_file_without_last_good_is_empty_not_fatal(self):
        with tempfile.TemporaryDirectory() as root:
            with open(os.path.join(root, "state.json"), "w") as f:
                f.write('{"a/b": ')
            state, problems = ghb_common.load_state(root)
            self.assertEqual(state, {})
            self.assertTrue(any("state.json" in p for p in problems))
            with open(os.path.join(root, "plan.json"), "w") as f:
                f.write('[{"repo": "a/b"')
            plan, problems = ghb_common.load_plan(root)
            self.assertEqual(plan, [])
            self.assertTrue(any("plan.json" in p for p in problems))

    def test_failed_replace_leaves_previous_file_intact(self):
        # A writer killed/replaced mid-dump never truncates the live
        # file: the temp write is separate and the replace is atomic.
        with tempfile.TemporaryDirectory() as root:
            path = os.path.join(root, "state.json")
            ghb_common.save_json(path, {"v": 1})
            with patch.object(ghb_common.os, "replace", side_effect=OSError("killed")):
                with self.assertRaises(OSError):
                    ghb_common.save_json(path, {"v": 2})
            with open(path) as f:
                self.assertEqual(json.load(f), {"v": 1})
            leftovers = [n for n in os.listdir(root) if n.endswith(".tmp")]
            self.assertEqual(leftovers, [])

    def test_state_lock_is_reacquirable(self):
        with tempfile.TemporaryDirectory() as root:
            with ghb_common.state_lock(root):
                self.assertTrue(os.path.exists(os.path.join(root, "state.lock")))
            with ghb_common.state_lock(root):
                pass


if __name__ == "__main__":
    unittest.main()
