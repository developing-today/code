#!/usr/bin/env python3
"""Tests for #15793 criterion 1 — lab-ops-hub GET /version.

The edge worker-checks probe (pm-stack/console/edge/src/
worker-checks.ts) asks every in-service Worker for the commit it was
deployed from and reads the `version` field of its /version answer
(commitOf). lab-ops-hub named no commit: /version fell through to the
static-assets fallthrough and /api/health names no commit either, so
the probe recorded it answering with commit null.

Oracle: #15793 criterion 1 plus the probe contract — an
unauthenticated GET /version answering {ok, service, version} from
the VERSION var stamped per deploy (the pm-service / github-ci /
fleet-git-rehost shape), "unknown" when unstamped. The behavioral
answer shape is pinned by test/version.test.ts; these tests pin the
wiring in src/index.ts (which plain Node cannot import — it pulls in
Worker bindings):

- the route sits ahead of the #441 API_TOKEN gate and ahead of the
  D1 usage ledger (it touches no binding, so the gate rationale is
  untouched), and non-GET gets a 405,
- the Env type declares the VERSION var,
- both deploy paths stamp VERSION (deploy.mjs --var when the commit
  is known, and the lab-ops-hub row of
  cf-worker-versions-release.yml).

Run: npm test  (python3 -m unittest discover -s test -p "test_*.py")
"""

import re
import unittest
from pathlib import Path

PKG = Path(__file__).resolve().parent.parent
SRC = (PKG / "src" / "index.ts").read_text()
DEPLOY = (PKG / "deploy.mjs").read_text()
WORKFLOW = (PKG.parent.parent / ".github" / "workflows" / "cf-worker-versions-release.yml").read_text()


class VersionRouteWiring(unittest.TestCase):
    def test_route_sits_ahead_of_auth_gate_and_ledger(self):
        route = SRC.index('p === "/version"')
        gate = SRC.index('p.startsWith("/api/")')
        ledger = SRC.index('logUsage(env, "worker_request"')
        self.assertLess(route, gate)
        self.assertLess(route, ledger)

    def test_route_answers_with_version_answer_helper(self):
        m = re.search(
            r'p === "/version"\) \{\s.*?return json\(versionAnswer\(env\.VERSION\)\);',
            SRC,
            re.DOTALL,
        )
        self.assertIsNotNone(m, "no /version branch answering versionAnswer(env.VERSION) found")

    def test_non_get_is_405(self):
        m = re.search(
            r'p === "/version"\) \{\s.*?if \(req\.method !== "GET"\) \{\s*'
            r'return json\(\{ error: "method not allowed" \}, 405\);',
            SRC,
            re.DOTALL,
        )
        self.assertIsNotNone(m, "the /version branch does not refuse non-GET with 405")

    def test_env_declares_version_var(self):
        self.assertIn("VERSION?: string;", SRC)


class DeployPathsStampVersion(unittest.TestCase):
    def test_deploy_mjs_stamps_known_commit(self):
        self.assertIn("process.env.LOH_VERSION", DEPLOY)
        self.assertIn('["deploy", "--var", `VERSION:${versionSha}`]', DEPLOY)

    def test_versions_release_workflow_stamps_lab_ops_hub(self):
        m = re.search(
            r'\[ "\$WORKER" = "lab-ops-hub" \]; then\s*'
            r'upload_args\+=\(--var "VERSION:\$\{GITHUB_SHA\}"\)',
            WORKFLOW,
        )
        self.assertIsNotNone(m, "the versions-release stamp does not cover lab-ops-hub")


if __name__ == "__main__":
    unittest.main()
