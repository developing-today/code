"""Structural safety guard for the standalone probe Versions staging workflow."""
import json
from pathlib import Path
import unittest


REPO = Path(__file__).resolve().parents[3]
WORKFLOW = REPO / ".github/workflows/cf-container-hello.yml"


class WorkflowDeployGuardTest(unittest.TestCase):
    def test_versions_staging_is_manual_default_off_and_main_only(self):
        workflow = WORKFLOW.read_text()
        self.assertIn("stage_versions:", workflow)
        self.assertIn("default: false", workflow)
        self.assertIn("github.event_name == 'workflow_dispatch'", workflow)
        self.assertIn("github.repository == 'developing-today/code'", workflow)
        self.assertIn("github.ref == 'refs/heads/main'", workflow)

    def test_staging_has_pinned_toolchain_and_cannot_upload_or_promote(self):
        staging = WORKFLOW.read_text().split("  stage-versions:\n", 1)[1]
        self.assertIn("node-version: 24.22.0", staging)
        self.assertIn("npm ci --no-audit --no-fund", staging)
        self.assertIn("--dry-run", staging)
        self.assertNotIn("environment:", staging)
        self.assertNotIn("secrets.", staging)
        self.assertNotIn("deployments create", staging)

    def test_operator_deploy_contract_and_container_mapping_remain(self):
        package = json.loads((REPO / "pkgs/cf-container-hello/package.json").read_text())
        self.assertEqual(package["scripts"]["deploy"], "wrangler deploy")
        config = (REPO / "pkgs/cf-container-hello/wrangler.jsonc").read_text()
        self.assertIn("lab-ops-hub-crawlercontainer:v5", config)
        self.assertIn('["HelloContainer"]', config)
        self.assertIn('["CrawlerProbe", "HelloLite"]', config)

    def test_native_build_gate_precedes_prebuilt_versions_dry_run(self):
        workflow = WORKFLOW.read_text().split("  stage-versions:\n", 1)[1]
        self.assertLess(workflow.index("node_modules/.bin/cf build"), workflow.index("node release-tools/verify-build.mjs"))
        self.assertLess(workflow.index("node release-tools/verify-build.mjs"), workflow.index("cf workers versions create"))
        versions = workflow.split("cf workers versions create", 1)[1]
        self.assertIn("--prebuilt", versions)
        self.assertIn("--dry-run", versions)
        self.assertNotIn("--versions", versions)


if __name__ == "__main__":
    unittest.main()
