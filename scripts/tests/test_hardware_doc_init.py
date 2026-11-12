#!/usr/bin/env python3
"""Regression tests for scripts/hardware-doc-init.sh branch routing (issue #6890).

Oracle: fixture git repositories. The script promises (header) that an
existing, clean, behind checkout is fast-forwarded and every unsafe state
warns and is left byte-identical. Before the fix the update block was
unreachable (the else required $TARGET/.git to both exist and not exist),
so a behind checkout was adopted as-is with no fetch, warning, or update.
"""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "hardware-doc-init.sh"

ENV_EXTRA = {
    "GIT_AUTHOR_NAME": "hardware-doc-init test",
    "GIT_AUTHOR_EMAIL": "hardware-doc-init@example.invalid",
    "GIT_COMMITTER_NAME": "hardware-doc-init test",
    "GIT_COMMITTER_EMAIL": "hardware-doc-init@example.invalid",
}


def git(cwd, *args, check=True):
    env = os.environ.copy()
    env.update(ENV_EXTRA)
    return subprocess.run(
        ["git", "-C", str(cwd), *args],
        check=check,
        text=True,
        capture_output=True,
        env=env,
    )


def commit_file(repo, name, content, message):
    (Path(repo) / name).write_text(content)
    git(repo, "add", name)
    git(repo, "commit", "--quiet", "-m", message)
    return git(repo, "rev-parse", "HEAD").stdout.strip()


class TestHardwareDocInit(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="hardware-doc-init-")
        self.addCleanup(self._tmp.cleanup)
        base = Path(self._tmp.name)
        # The repo the script is run from ("this repo").
        self.main_repo = base / "code"
        self.main_repo.mkdir()
        subprocess.run(
            ["git", "init", "--quiet", "-b", "main", str(self.main_repo)],
            check=True,
            env={**os.environ, **ENV_EXTRA},
        )
        commit_file(self.main_repo, "README.md", "fixture\n", "main fixture")
        # Upstream hardware-doc repo.
        self.origin = base / "hw-origin"
        self.origin.mkdir()
        subprocess.run(
            ["git", "init", "--quiet", "-b", "main", str(self.origin)],
            check=True,
            env={**os.environ, **ENV_EXTRA},
        )
        self.head_a = commit_file(self.origin, "doc.md", "a\n", "A")
        self.target = base / "hardware-doc"

    def clone_target(self):
        env = os.environ.copy()
        env.update(ENV_EXTRA)
        subprocess.run(
            ["git", "clone", "--quiet", str(self.origin), str(self.target)],
            check=True,
            capture_output=True,
            env=env,
        )

    def advance_origin(self):
        return commit_file(self.origin, "doc.md", "b\n", "B")

    def run_script(self):
        env = os.environ.copy()
        env.update(ENV_EXTRA)
        env["HARDWARE_DOC_URL"] = str(self.origin)
        return subprocess.run(
            ["bash", str(SCRIPT)],
            cwd=self.main_repo,
            text=True,
            capture_output=True,
            check=False,
            env=env,
        )

    def target_head(self):
        return git(self.target, "rev-parse", "HEAD").stdout.strip()

    def test_behind_checkout_is_fast_forwarded(self):
        self.clone_target()
        head_b = self.advance_origin()
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("fast-forwarded", result.stdout + result.stderr)
        self.assertEqual(self.target_head(), head_b)

    def test_up_to_date_checkout_reports_up_to_date(self):
        self.clone_target()
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("up to date", result.stdout + result.stderr)
        self.assertEqual(self.target_head(), self.head_a)

    def test_dirty_checkout_warns_and_is_untouched(self):
        self.clone_target()
        self.advance_origin()
        (self.target / "doc.md").write_text("dirty\n")
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("uncommitted changes", result.stderr)
        self.assertEqual(self.target_head(), self.head_a)

    def test_diverged_checkout_warns_and_is_untouched(self):
        self.clone_target()
        self.advance_origin()
        local = commit_file(self.target, "local.md", "local\n", "local")
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("diverged", result.stderr)
        self.assertEqual(self.target_head(), local)

    def test_unpushed_checkout_warns_and_is_untouched(self):
        self.clone_target()
        local = commit_file(self.target, "local.md", "local\n", "local")
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("unpushed commits", result.stderr)
        self.assertEqual(self.target_head(), local)

    def test_detached_head_warns_and_is_untouched(self):
        self.clone_target()
        self.advance_origin()
        git(self.target, "checkout", "--quiet", "--detach", "HEAD")
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("detached HEAD", result.stderr)
        self.assertEqual(self.target_head(), self.head_a)

    def test_branch_without_upstream_warns(self):
        self.clone_target()
        git(self.target, "checkout", "--quiet", "-b", "topic")
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("no upstream", result.stderr)
        self.assertEqual(self.target_head(), self.head_a)

    def test_existing_non_repo_target_errors(self):
        self.target.mkdir()
        (self.target / "plain.txt").write_text("not a repo\n")
        result = self.run_script()
        self.assertEqual(result.returncode, 1)
        self.assertIn("not a git repository", result.stderr)

    def test_absent_target_is_cloned(self):
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.target / ".git").is_dir())
        self.assertEqual(self.target_head(), self.head_a)
        # Symlink behaviour unchanged: doc/hardware resolves to the target.
        link = self.main_repo / "doc" / "hardware"
        self.assertTrue(link.is_symlink())
        self.assertEqual(os.path.realpath(link), os.path.realpath(self.target))


if __name__ == "__main__":
    unittest.main()
