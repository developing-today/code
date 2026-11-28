#!/usr/bin/env python3
"""Tests for #15793 criterion 1 — cf-container-hello GET /version.

The edge worker-checks probe (pm-stack/console/edge/src/
worker-checks.ts) asks every in-service Worker for the commit it was
deployed from and reads the `version` field of its /version answer
(commitOf). cf-container-hello named no commit: every GET fell into
the #955 API_TOKEN gate, so the probe recorded it answering with
commit null.

Oracle: #15793 criterion 1 plus the probe contract — an
unauthenticated GET /version answering {ok, service, version} from
the VERSION var stamped per deploy (the pm-service / github-ci /
fleet-git-rehost shape), "unknown" when unstamped. These tests pin:

- the route sits with /worker-marker ahead of the API_TOKEN gate
  (it touches no container, so the #955 gate rationale is untouched),
- the answer carries the version field from env.VERSION with the
  "unknown" fallback, and non-GET gets a 405,
- both deploy paths stamp VERSION (deploy.mjs --var, and the
  cf-container-hello row of cf-worker-versions-release.yml).

Run: npm test  (python3 -m unittest discover -s test -p "test_*.py")
"""

import re
import unittest
from pathlib import Path

PKG = Path(__file__).resolve().parent.parent
SRC = (PKG / "src" / "index.ts").read_text()
DEPLOY = (PKG / "deploy.mjs").read_text()
WORKFLOW = (PKG.parent.parent / ".github" / "workflows" / "cf-worker-versions-release.yml").read_text()


class VersionRouteAnswersCommit(unittest.TestCase):
    def test_route_sits_ahead_of_the_auth_gate(self):
        route = SRC.index('pathname === "/version"')
        marker = SRC.index('pathname === "/worker-marker"')
        gate = SRC.index("await isAuthorized(request, env)")
        self.assertLess(marker, route)
        self.assertLess(route, gate)

    def test_answer_shape_names_service_and_version(self):
        m = re.search(
            r'pathname === "/version"\) \{\s.*?return json\(\{ ok: true, service: "cf-container-hello", '
            r"version: env\.VERSION \?\? \"unknown\" \}\);",
            SRC,
            re.DOTALL,
        )
        self.assertIsNotNone(m, "no /version branch answering the probe shape found")

    def test_non_get_is_405(self):
        m = re.search(
            r'pathname === "/version"\) \{\s.*?if \(request\.method !== "GET"\) \{\s*'
            r'return json\(\{ error: "method not allowed" \}, 405\);',
            SRC,
            re.DOTALL,
        )
        self.assertIsNotNone(m, "the /version branch does not refuse non-GET with 405")

    def test_env_declares_version_var(self):
        self.assertIn("VERSION?: string;", SRC)


class DeployPathsStampVersion(unittest.TestCase):
    def test_deploy_mjs_stamps_known_commit(self):
        self.assertIn("process.env.CFH_VERSION ?? gitHead()", DEPLOY)
        self.assertIn('wranglerArgs.push("--var", `VERSION:${version}`)', DEPLOY)

    def test_versions_release_workflow_stamps_cf_container_hello(self):
        m = re.search(
            r'if \[ "\$WORKER" = "fleet-git-rehost" \] \|\| \[ "\$WORKER" = "cf-container-hello" \]; then\s*'
            r'upload_args\+=\(--var "VERSION:\$\{GITHUB_SHA\}"\)',
            WORKFLOW,
        )
        self.assertIsNotNone(m, "the versions-release stamp does not cover cf-container-hello")


if __name__ == "__main__":
    unittest.main()
