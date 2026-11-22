#!/usr/bin/env python3
"""Offline guard tests for the github-ci Worker version workflows."""

from pathlib import Path
import unittest


REPO_ROOT = Path(__file__).resolve().parents[4]
UPLOAD = (REPO_ROOT / ".github/workflows/github-ci-worker-upload.yml").read_text()
PROMOTE = (REPO_ROOT / ".github/workflows/github-ci-worker-promote.yml").read_text()


class DeploymentWorkflowTests(unittest.TestCase):
    def test_upload_only_creates_a_version_and_keeps_runtime_secrets_out(self):
        self.assertIn("wrangler versions upload", UPLOAD)
        self.assertNotIn("wrangler versions deploy", UPLOAD)
        self.assertNotIn("wrangler deploy", UPLOAD)
        self.assertNotIn("--secrets-file", UPLOAD)
        self.assertIn("CLOUDFLARE_API_TOKEN: ${{ secrets.CLOUDFLARE_API_TOKEN }}", UPLOAD)
        self.assertIn("environment: github-ci-cloudflare-upload", UPLOAD)
        self.assertIn("if: github.ref == 'refs/heads/main'", UPLOAD)

    def test_promotion_is_manual_protected_and_uses_the_supplied_version(self):
        self.assertIn("workflow_dispatch:", PROMOTE)
        self.assertIn("version_id:", PROMOTE)
        self.assertIn("environment: github-ci-production", PROMOTE)
        self.assertIn("if: github.ref == 'refs/heads/main'", PROMOTE)
        self.assertIn('"${VERSION_ID}@100%"', PROMOTE)
        self.assertIn("--name github-ci", PROMOTE)
        self.assertIn("wrangler deployments status", PROMOTE)
        self.assertIn("versions[0].percentage !== 100", PROMOTE)
        self.assertNotRegex(PROMOTE, r"wrangler deploy(?:\s|$)")


if __name__ == "__main__":
    unittest.main()
