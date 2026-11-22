"""Hermetic dirty-refusal oracle: no source/index bytes or cargo work may change.

Run: python3 pkgs/id/tests/test_planted_bug_preflight.py
Only synthetic files in a disposable local Git repository are used.
"""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("planted-bug-check.sh")
SOURCES = (
    "pkgs/id/crates/id-core/src/membership/layer.rs",
    "pkgs/id/crates/id-core/src/membership/frontier.rs",
    "pkgs/id/crates/id-events/src/groups/delegation.rs",
    "pkgs/id/crates/id-store/src/tag_claims.rs",
)


class PreflightTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="planted-preflight-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
        self.env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull,
                        HOME=str(self.root))
        self.git("init", "-q")
        for name in SOURCES:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"synthetic baseline\n")
        script = self.root / "pkgs/id/tests/planted-bug-check.sh"
        script.parent.mkdir(parents=True)
        # Confine even a broken preflight's fixed log paths to this fixture.
        script.write_text(SCRIPT.read_text().replace(
            "/tmp/planted-bug-", str(self.root / "planted-bug-")))
        self.git("add", "pkgs")
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                 "commit", "-qm", "synthetic fixture")
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        cargo = bin_dir / "cargo"
        cargo.write_text('#!/bin/sh\nprintf invoked > "$HOME/cargo-invoked"\nexit 99\n')
        cargo.chmod(0o755)
        self.env["PATH"] = str(bin_dir) + os.pathsep + os.environ["PATH"]

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.root, env=self.env,
                              check=True, capture_output=True).stdout

    def assert_refused_unchanged(self):
        before = {name: (self.root / name).read_bytes() if (self.root / name).exists()
                  else None for name in SOURCES}
        index = (self.root / ".git/index").read_bytes()
        result = subprocess.run(["bash", "pkgs/id/tests/planted-bug-check.sh"],
                                cwd=self.root, env=self.env, capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 2, result.stderr.decode())
        self.assertIn(b"refusing", result.stderr)
        self.assertEqual((self.root / ".git/index").read_bytes(), index)
        for name, expected in before.items():
            path = self.root / name
            self.assertEqual(path.read_bytes() if path.exists() else None, expected, name)
        self.assertFalse((self.root / "cargo-invoked").exists())
        self.assertNotIn(b"baseline", result.stdout)

    def test_each_dirty_source_preserves_index_and_worktree(self):
        # Different staged and unstaged bytes ensure restoring either layer is detected.
        for name in SOURCES:
            with self.subTest(source=name):
                path = self.root / name
                path.write_bytes(b"staged fixture edit\n")
                self.git("add", name)
                path.write_bytes(b"unstaged fixture edit\n")
                self.assert_refused_unchanged()
                # Restore only our disposable synthetic fixture between cases.
                self.git("reset", "-q", "HEAD", "--", name)
                path.write_bytes(b"synthetic baseline\n")

    def test_index_flags_cannot_hide_worktree_edits(self):
        for flag in ("assume-unchanged", "skip-worktree"):
            with self.subTest(flag=flag):
                name = SOURCES[-1]
                self.git("update-index", "--" + flag, "--", name)
                (self.root / name).write_bytes(b"hidden synthetic edit\n")
                self.assert_refused_unchanged()
                self.git("update-index", "--no-" + flag, "--", name)
                (self.root / name).write_bytes(b"synthetic baseline\n")

    def test_missing_tracked_source_refuses(self):
        (self.root / SOURCES[-1]).unlink()
        self.assert_refused_unchanged()

    def test_untracked_target_refuses(self):
        self.git("rm", "--cached", "--", SOURCES[-1])
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                 "commit", "-qm", "untracked target fixture")
        self.assert_refused_unchanged()

    def test_absent_untracked_target_refuses(self):
        self.git("rm", "--", SOURCES[-1])
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                 "commit", "-qm", "missing target fixture")
        self.assert_refused_unchanged()


if __name__ == "__main__":
    unittest.main()
