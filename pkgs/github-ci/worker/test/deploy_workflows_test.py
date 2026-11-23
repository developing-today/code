#!/usr/bin/env python3
"""Offline guard checks for github-ci Worker deployment workflows."""

import json
from pathlib import Path
import unittest


REPO_ROOT = Path(__file__).resolve().parents[4]
UPLOAD = (REPO_ROOT / ".github/workflows/github-ci-worker-upload.yml").read_text()
PROMOTE = (REPO_ROOT / ".github/workflows/github-ci-worker-promote.yml").read_text()
TOOLCHAIN = json.loads((REPO_ROOT / ".github/worker-deploy-ci/package.json").read_text())
WORKER_PACKAGE = json.loads((REPO_ROOT / "pkgs/github-ci/worker/package.json").read_text())


class DeploymentWorkflowTests(unittest.TestCase):
    def test_upload_is_manual_default_off_cf_only_and_never_promotes(self):
        self.assertIn("workflow_dispatch:", UPLOAD)
        self.assertIn("default: false", UPLOAD)
        self.assertNotIn("push:", UPLOAD)
        self.assertIn("inputs.confirm_upload == true", UPLOAD)
        self.assertIn("environment: github-ci-cloudflare-upload", UPLOAD)
        self.assertIn("check-environment", UPLOAD)
        self.assertIn("node-version: 24.22.0", UPLOAD)
        self.assertIn("npm run build", UPLOAD)
        self.assertIn("cf workers versions create", UPLOAD)
        self.assertIn("--prebuilt --dry-run", UPLOAD)
        self.assertIn("workers versions list --worker github-ci", UPLOAD)
        self.assertIn("versions get", UPLOAD)
        self.assertIn("select-baseline", UPLOAD)
        self.assertIn("verify-baseline", UPLOAD)
        self.assertIn("verify-version", UPLOAD)
        self.assertIn("verify-deployable", UPLOAD)
        self.assertIn("write-upload-receipt", UPLOAD)
        self.assertIn("actions/upload-artifact@v7", UPLOAD)
        self.assertNotIn("cf workers deployments create", UPLOAD)
        self.assertNotRegex(UPLOAD, r"(?m)^\s*(?:npx\s+)?(?:\./node_modules/\.bin/)?wrangler(?:\s|$)")
        self.assertNotIn("--secrets-file", UPLOAD)
        self.assertIn("GITHUB_TOKEN: ${{ github.token }}", UPLOAD)
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

    def test_promotion_requires_exact_upload_attempt_and_readback_receipts(self):
        self.assertIn("workflow_dispatch:", PROMOTE)
        self.assertIn("default: false", PROMOTE)
        self.assertNotIn("push:", PROMOTE)
        self.assertIn("inputs.confirm_promotion == true", PROMOTE)
        self.assertIn("environment: github-ci-production", PROMOTE)
        self.assertIn("check-environment", PROMOTE)
        for receipt_input in ("upload_run_id:", "upload_run_attempt:", "version_id:", "baseline_deployment_id:", "baseline_version_id:"):
            self.assertIn(receipt_input, PROMOTE)
        self.assertIn("actions/download-artifact@v8", PROMOTE)
        self.assertIn("actions/runs/$UPLOAD_RUN_ID/attempts/$UPLOAD_RUN_ATTEMPT", PROMOTE)
        self.assertIn("verify-upload-receipt", PROMOTE)
        self.assertIn("cf workers versions get", PROMOTE)
        self.assertIn("verify-baseline-current", PROMOTE)
        self.assertIn("git merge-base --is-ancestor", PROMOTE)
        self.assertIn("cf workers deployments create", PROMOTE)
        self.assertIn("cf workers deployments list", PROMOTE)
        self.assertIn("verify-promotion-readback", PROMOTE)
        self.assertIn("write-promotion-receipt", PROMOTE)
        self.assertIn("actions/upload-artifact@v7", PROMOTE)
        self.assertNotIn("wrangler deploy", PROMOTE)
        self.assertNotRegex(PROMOTE, r"(?m)^\s*(?:npx\s+)?(?:\./node_modules/\.bin/)?wrangler(?:\s|$)")

    def test_pinned_toolchain_is_separate_from_operator_package(self):
        self.assertEqual(TOOLCHAIN["engines"]["node"], "24.22.0")
        self.assertEqual(TOOLCHAIN["devDependencies"]["cf"], "1.0.0-rc.1")
        self.assertEqual(TOOLCHAIN["devDependencies"]["wrangler"], "4.166.0")
        self.assertEqual(WORKER_PACKAGE["scripts"]["deploy"], "wrangler deploy")
        self.assertEqual(WORKER_PACKAGE["devDependencies"]["wrangler"], "4.59.1")
        self.assertNotIn("cf", WORKER_PACKAGE["devDependencies"])


if __name__ == "__main__":
    unittest.main()
