#!/usr/bin/env python3
"""Contract tests for the worker preflight's origin/main shadow guard."""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
PREFLIGHT = ROOT / "scripts" / "worker-preflight.sh"


class TestWorkerPreflight(unittest.TestCase):
    def test_clean_repo_passes_and_local_origin_main_branch_fails(self):
        with tempfile.TemporaryDirectory(prefix="worker-preflight-") as tmp:
            repo = Path(tmp)
            env = os.environ.copy()
            env.update(
                GIT_AUTHOR_NAME="Worker preflight test",
                GIT_AUTHOR_EMAIL="worker-preflight@example.invalid",
                GIT_COMMITTER_NAME="Worker preflight test",
                GIT_COMMITTER_EMAIL="worker-preflight@example.invalid",
            )

            subprocess.run(["git", "init", "--quiet", str(repo)], check=True)
            subprocess.run(
                [
                    "git",
                    "-C",
                    str(repo),
                    "-c",
                    "user.name=Worker preflight test",
                    "-c",
                    "user.email=worker-preflight@example.invalid",
                    "commit",
                    "--quiet",
                    "--allow-empty",
                    "-m",
                    "fixture",
                ],
                check=True,
                env=env,
            )

            clean = subprocess.run(
                ["bash", str(PREFLIGHT)],
                cwd=repo,
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(clean.returncode, 0, clean.stderr)
            self.assertIn("worker-preflight: clean", clean.stdout)

            head = subprocess.check_output(
                ["git", "-C", str(repo), "rev-parse", "HEAD"], text=True
            ).strip()
            subprocess.run(
                ["git", "-C", str(repo), "update-ref", "refs/heads/origin/main", head],
                check=True,
            )

            shadowed = subprocess.run(
                ["bash", str(PREFLIGHT)],
                cwd=repo,
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(shadowed.returncode, 1)
            self.assertIn("refs/heads/origin/main", shadowed.stderr)
            self.assertIn("shadows refs/remotes/origin/main", shadowed.stderr)


if __name__ == "__main__":
    unittest.main()
