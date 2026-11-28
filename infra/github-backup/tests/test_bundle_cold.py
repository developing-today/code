#!/usr/bin/env python3
"""Local cold-bundle restore drill; oracle: clone HEAD equals mirror HEAD.

Run: python3 infra/github-backup/tests/test_bundle_cold.py
"""

import os
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr
from io import StringIO
from unittest.mock import patch

BIN = os.path.abspath(
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "bin")
)
sys.path.insert(0, BIN)

import bundle_cold


def git(*args, cwd=None):
    return subprocess.run(
        ["git", *args], cwd=cwd, capture_output=True, text=True, check=True
    )


def make_complete_mirror(root):
    src = os.path.join(root, "src")
    os.makedirs(src)
    git("init", "-b", "main", cwd=src)
    git("config", "user.email", "t@example.invalid", cwd=src)
    git("config", "user.name", "t", cwd=src)
    with open(os.path.join(src, "f.txt"), "w") as f:
        f.write("hello\n")
    git("add", "f.txt", cwd=src)
    git("commit", "-m", "one", cwd=src)
    mirror = os.path.join(root, "owner", "repo.git")
    os.makedirs(os.path.dirname(mirror))
    git("clone", "--mirror", src, mirror)
    return mirror


class ColdBundleTests(unittest.TestCase):
    def test_parser_rejects_abbreviated_options(self):
        # Oracle: only the declared --repo and --root spellings parse.
        with redirect_stderr(StringIO()):
            with self.assertRaises(SystemExit) as raised:
                bundle_cold._parser().parse_args(
                    ["--rep", "owner/repo", "--roo", "/backup"]
                )

        self.assertEqual(raised.exception.code, 2)

    def test_bundle_round_trip_matches_mirror_head(self):
        with tempfile.TemporaryDirectory() as root:
            mirror = make_complete_mirror(root)
            bundle = os.path.join(root, "cold", "owner", "repo.bundle")

            result = bundle_cold.create_bundle(mirror, bundle)

            self.assertEqual(result["status"], "ok")
            self.assertEqual(
                result["head"],
                git("--git-dir", mirror, "rev-parse", "HEAD").stdout.strip(),
            )
            self.assertTrue(os.path.isfile(bundle))

    def test_bare_skeleton_is_refused_without_creating_bundle(self):
        with tempfile.TemporaryDirectory() as root:
            mirror = os.path.join(root, "owner", "empty.git")
            os.makedirs(os.path.dirname(mirror))
            git("init", "--bare", mirror)
            bundle = os.path.join(root, "cold", "owner", "empty.bundle")

            with self.assertRaisesRegex(ValueError, "not a valid mirror"):
                bundle_cold.create_bundle(mirror, bundle)

            self.assertFalse(os.path.exists(bundle))

    def test_failed_verification_preserves_previous_bundle(self):
        with tempfile.TemporaryDirectory() as root:
            mirror = make_complete_mirror(root)
            bundle = os.path.join(root, "cold", "owner", "repo.bundle")
            os.makedirs(os.path.dirname(bundle))
            with open(bundle, "wb") as f:
                f.write(b"previous bundle")
            real_run_git = bundle_cold._run_git

            def reject_verify(args):
                if "verify" in args:
                    raise RuntimeError("injected verification failure")
                return real_run_git(args)

            with patch.object(bundle_cold, "_run_git", side_effect=reject_verify):
                with self.assertRaisesRegex(RuntimeError, "verification failure"):
                    bundle_cold.create_bundle(mirror, bundle)

            with open(bundle, "rb") as f:
                self.assertEqual(f.read(), b"previous bundle")


if __name__ == "__main__":
    unittest.main()
