#!/usr/bin/env python3
"""All four fleet mirrors are attempted, even when one repository fails.

Oracle: every fleet repo reaches upload_repo, unrelated planned repos do not,
and any failure yields nonzero.
Run: python3 -I infra/github-backup/tests/test_bundle_r2_all.py
"""

import os
import subprocess
import sys
import unittest
from unittest.mock import patch

BIN = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "bin"))
sys.path.insert(0, BIN)

import bundle_r2_all


class BundleR2AllTests(unittest.TestCase):
    def test_attempts_every_repo_and_reports_failure(self):
        plan = (
            [{"repo": repo} for repo in bundle_r2_all.FLEET_REPOS]
            + [{"repo": "unrelated/repo"}],
            [],
        )
        attempted = []

        def upload(repo, _root, _bucket):
            attempted.append(repo)
            if repo == bundle_r2_all.FLEET_REPOS[1]:
                raise RuntimeError("upload failed")
            return {"head": "abc"}

        with (
            patch.object(bundle_r2_all, "load_plan", return_value=plan),
            patch.object(bundle_r2_all.bundle_r2, "upload_repo", side_effect=upload),
        ):
            self.assertEqual(bundle_r2_all.run_all("/backup", "supabase-backups"), 1)
        self.assertEqual(attempted, list(bundle_r2_all.FLEET_REPOS))

    def test_missing_fleet_repo_fails_before_upload(self):
        plan = ([{"repo": bundle_r2_all.FLEET_REPOS[0]}], [])
        with (
            patch.object(bundle_r2_all, "load_plan", return_value=plan),
            patch.object(bundle_r2_all.bundle_r2, "upload_repo") as upload,
        ):
            self.assertEqual(bundle_r2_all.run_all("/backup", "supabase-backups"), 1)
        upload.assert_not_called()

    def test_unknown_cli_argument_is_usage_error(self):
        script = os.path.join(BIN, "bundle_r2_all.py")
        result = subprocess.run(
            [sys.executable, script, "--unexpected"], capture_output=True, text=True
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("unrecognized arguments", result.stderr)


if __name__ == "__main__":
    unittest.main()
