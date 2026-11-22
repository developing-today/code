#!/usr/bin/env python3
"""R2 upload happens only after the local bundle passes its clone drill.

Oracle: the uploader's source is a verified bundle, and rclone's download
check reports byte-identical local and remote objects.

Run: python3 -I infra/github-backup/tests/test_bundle_r2.py
"""

import os
import hashlib
import shutil
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
    def test_upload_then_read_back_compares_exact_remote_bytes(self):
        with tempfile.TemporaryDirectory() as root:
            mirror = make_mirror(root)
            calls = []
            readbacks = []
            with patch.object(bundle_r2, "load_plan", return_value=([{"repo": "owner/repo"}], [])):
                with patch.object(bundle_r2, "_run_rclone", side_effect=self._local_rclone(root, calls, readbacks)):
                    result = bundle_r2.upload_repo("owner/repo", root, "supabase-backups")
            expected_head = git("--git-dir", mirror, "rev-parse", "HEAD").stdout.strip()

        self.assertEqual(result["status"], "ok")
        self.assertEqual(len(calls), 2)
        self.assertEqual([call[0] for call in calls], ["copyto", "copyto"])
        self.assertEqual(calls[1][1], result["remote"])
        self.assertNotEqual(calls[1][2], calls[0][1])
        self.assertEqual(
            result["head"], expected_head
        )
        self.assertEqual(readbacks, [result["sha256"]])
        self.assertFalse(os.path.exists(calls[0][1]), "verified local staging bundle is removed")
        self.assertIn(result["head"] + "-" + result["sha256"], result["remote"])

    def test_remote_byte_mismatch_is_reported_after_upload(self):
        with tempfile.TemporaryDirectory() as root:
            make_mirror(root)
            calls = []
            readbacks = []
            with patch.object(bundle_r2, "load_plan", return_value=([{"repo": "owner/repo"}], [])):
                with patch.object(
                    bundle_r2,
                    "_run_rclone",
                    side_effect=self._local_rclone(root, calls, readbacks, corrupt_readback=True),
                ):
                    with self.assertRaisesRegex(RuntimeError, "downloaded R2 bundle SHA-256"):
                        bundle_r2.upload_repo("owner/repo", root, "supabase-backups")

            self.assertEqual([call[0] for call in calls], ["copyto", "copyto"])
            self.assertFalse(os.path.exists(calls[0][1]), "failed verification removes local staging bundle")

    def test_upload_failure_removes_local_staging_bundle(self):
        with tempfile.TemporaryDirectory() as root:
            make_mirror(root)
            source = []

            def failed_upload(args):
                source.append(args[1])
                raise RuntimeError("upload unavailable")

            with patch.object(bundle_r2, "load_plan", return_value=([{"repo": "owner/repo"}], [])):
                with patch.object(bundle_r2, "_run_rclone", side_effect=failed_upload):
                    with self.assertRaisesRegex(RuntimeError, "upload unavailable"):
                        bundle_r2.upload_repo("owner/repo", root, "supabase-backups")

            self.assertEqual(len(source), 1)
            self.assertFalse(os.path.exists(source[0]), "failed upload removes local staging bundle")

    def test_each_upload_uses_a_distinct_bundle_snapshot(self):
        with tempfile.TemporaryDirectory() as root:
            make_mirror(root)
            calls = []
            readbacks = []
            with patch.object(bundle_r2, "load_plan", return_value=([{"repo": "owner/repo"}], [])):
                with patch.object(bundle_r2, "_run_rclone", side_effect=self._local_rclone(root, calls, readbacks)):
                    bundle_r2.upload_repo("owner/repo", root, "supabase-backups")
                    bundle_r2.upload_repo("owner/repo", root, "supabase-backups")

        # The per-invocation input files stay isolated while hashing and both
        # rclone copyto calls run; the oracle is that their source paths differ.
        self.assertNotEqual(calls[0][1], calls[2][1])
        self.assertEqual([call[0] for call in calls], ["copyto"] * 4)

    def _local_rclone(self, root, calls, readbacks, corrupt_readback=False):
        remote_root = os.path.join(root, "remote-store")

        def run(args):
            calls.append(args)
            command, source, destination = args[:3]
            if command != "copyto":
                raise AssertionError(f"expected file-compatible copyto, got {command}")

            if source.startswith("r2bak:"):
                remote_path = os.path.join(remote_root, source.removeprefix("r2bak:").replace("/", os.sep))
                shutil.copyfile(remote_path, destination)
                if corrupt_readback:
                    with open(destination, "ab") as downloaded:
                        downloaded.write(b"corrupt")
                with open(destination, "rb") as downloaded:
                    readbacks.append(hashlib.sha256(downloaded.read()).hexdigest())
                return

            remote_path = os.path.join(remote_root, destination.removeprefix("r2bak:").replace("/", os.sep))
            os.makedirs(os.path.dirname(remote_path), exist_ok=True)
            shutil.copyfile(source, remote_path)

        return run


if __name__ == "__main__":
    unittest.main()
