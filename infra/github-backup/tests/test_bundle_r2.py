#!/usr/bin/env python3
"""R2 upload happens only after the local bundle passes its clone drill.

Oracle: the uploader's source is a verified bundle, and rclone's download
check reports byte-identical local and remote objects.

Run: python3 -I infra/github-backup/tests/test_bundle_r2.py
"""

import os
import hashlib
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

BIN = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "bin"))
sys.path.insert(0, BIN)

import bundle_cold
import bundle_r2


def git(*args, cwd=None):
    return subprocess.run(
        ["git", *args], cwd=cwd, capture_output=True, text=True, check=True
    )


def make_mirror(root):
    source = os.path.join(root, "source")
    os.mkdir(source)
    git("init", "-b", "main", cwd=source)
    git("config", "user.email", "test@example.invalid", cwd=source)
    git("config", "user.name", "test", cwd=source)
    with open(os.path.join(source, "file"), "w", encoding="utf-8") as out:
        out.write("backup oracle\n")
    git("add", "file", cwd=source)
    git("commit", "-m", "fixture", cwd=source)
    mirror = os.path.join(root, "owner", "repo.git")
    os.makedirs(os.path.dirname(mirror))
    git("clone", "--mirror", source, mirror)
    return mirror


class BundleR2Tests(unittest.TestCase):
    def test_upload_then_download_check_uses_verified_local_bundle(self):
        with tempfile.TemporaryDirectory() as root:
            mirror = make_mirror(root)
            # load_plan's normal input is the generated inventory; use the
            # same planned-repo contract without depending on the laptop tree.
            with patch.object(bundle_r2, "load_plan", return_value=([{"repo": "owner/repo"}], [])):
                self._assert_upload(root, mirror)

    def _assert_upload(self, root, mirror):
        calls = []

        def run(args):
            calls.append(args)

        with patch.object(bundle_r2, "_run_rclone", side_effect=run), patch.object(
            bundle_cold, "create_bundle", wraps=bundle_cold.create_bundle
        ):
            result = bundle_r2.upload_repo("owner/repo", root, "supabase-backups")

        self.assertEqual(result["status"], "ok")
        self.assertEqual(len(calls), 2)
        self.assertEqual(calls[0][0], "copyto")
        self.assertEqual(calls[0][1], result["bundle"])
        self.assertEqual(calls[1][0], "check")
        self.assertIn("--download", calls[1])
        self.assertEqual(
            result["head"], git("--git-dir", mirror, "rev-parse", "HEAD").stdout.strip()
        )
        with open(result["bundle"], "rb") as artifact:
            self.assertEqual(result["sha256"], hashlib.sha256(artifact.read()).hexdigest())
        self.assertIn(result["head"] + "-" + result["sha256"], result["remote"])

    def test_remote_check_failure_is_reported_after_upload(self):
        with tempfile.TemporaryDirectory() as root:
            make_mirror(root)
            with patch.object(bundle_r2, "load_plan", return_value=([{"repo": "owner/repo"}], [])):
                self._assert_remote_check_failure(root)

    def _assert_remote_check_failure(self, root):
        calls = []

        def run(args):
            calls.append(args)
            if args[0] == "check":
                raise RuntimeError("remote bytes differ")

        with patch.object(bundle_r2, "_run_rclone", side_effect=run):
            with self.assertRaisesRegex(RuntimeError, "remote bytes differ"):
                bundle_r2.upload_repo("owner/repo", root, "supabase-backups")

        self.assertEqual([call[0] for call in calls], ["copyto", "check"])


if __name__ == "__main__":
    unittest.main()
