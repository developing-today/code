#!/usr/bin/env python3
"""Offline guard checks for github-ci Worker deployment workflows."""

from pathlib import Path
import unittest


REPO_ROOT = Path(__file__).resolve().parents[4]
UPLOAD = (REPO_ROOT / ".github/workflows/github-ci-worker-upload.yml").read_text()
PROMOTE = (REPO_ROOT / ".github/workflows/github-ci-worker-promote.yml").read_text()


class DeploymentWorkflowTests(unittest.TestCase):
    def test_upload_uses_cf_and_never_promotes_traffic(self):
        self.assertIn("node-version: 24.22.0", UPLOAD)
        self.assertIn("npm run build", UPLOAD)
        self.assertIn("cf -q workers versions create", UPLOAD)
        self.assertIn("--prebuilt --dry-run", UPLOAD)
        self.assertIn("workers versions list --worker github-ci", UPLOAD)
        self.assertIn("versions get", UPLOAD)
        self.assertIn("verify-version", UPLOAD)
        self.assertIn("verify-deployable", UPLOAD)
        self.assertNotIn("cf workers deployments create", UPLOAD)
        self.assertNotRegex(UPLOAD, r"(?m)^\s*(?:npx\s+)?(?:\./node_modules/\.bin/)?wrangler(?:\s|$)")
        self.assertNotIn("--secrets-file", UPLOAD)
        for runtime_secret in [
            "GITHUB_WEBHOOK_SECRET",
            "TURSO_PLATFORM_TOKEN",
            "TURSO_QUEUE_TOKEN",
            "EVENTS_BROKER_TOKEN",
            "EVENTS_BROKER_ACCESS_CLIENT_ID",
            "EVENTS_BROKER_ACCESS_CLIENT_SECRET",
        ]:
            self.assertNotIn(runtime_secret, UPLOAD)
            self.assertNotIn(runtime_secret, PROMOTE)
        self.assertIn("environment: github-ci-cloudflare-upload", UPLOAD)
        self.assertIn("if: github.repository == 'developing-today/code' && github.ref == 'refs/heads/main'", UPLOAD)

    def test_promotion_is_manual_cf_only_and_verifies_provenance(self):
        self.assertIn("workflow_dispatch:", PROMOTE)
        self.assertIn("version_id:", PROMOTE)
        self.assertIn("environment: github-ci-production", PROMOTE)
        self.assertIn("if: github.repository == 'developing-today/code' && github.ref == 'refs/heads/main'", PROMOTE)
        self.assertIn("cf -q workers versions get", PROMOTE)
        self.assertIn("git merge-base --is-ancestor", PROMOTE)
        self.assertIn("cf -q workers deployments create", PROMOTE)
        self.assertIn("cf -q workers deployments list", PROMOTE)
        self.assertIn("verify-deployment", PROMOTE)
        self.assertNotRegex(PROMOTE, r"(?m)^\s*(?:npx\s+)?(?:\./node_modules/\.bin/)?wrangler(?:\s|$)")


if __name__ == "__main__":
    unittest.main()
