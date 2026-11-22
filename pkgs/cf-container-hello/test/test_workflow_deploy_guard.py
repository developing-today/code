"""Structural safety guard for the manual standalone probe deploy workflow."""
from pathlib import Path
import unittest


WORKFLOW = Path(__file__).resolve().parents[3] / ".github/workflows/cf-container-hello.yml"


class WorkflowDeployGuardTest(unittest.TestCase):
    def test_manual_mutations_are_main_only_and_use_dedicated_environment(self):
        workflow = WORKFLOW.read_text()
        self.assertIn("github.ref == 'refs/heads/main' && inputs.operation == 'publish-image'", workflow)
        self.assertIn("github.ref == 'refs/heads/main' && inputs.operation == 'activate'", workflow)
        self.assertIn("environment: cf-container-hello", workflow)
        self.assertIn("secrets.CF_CONTAINER_HELLO_API_TOKEN", workflow)
        self.assertNotIn("secrets.CLOUDFLARE_API_TOKEN", workflow)
        activation = workflow.split("  activate:\n", 1)[1]

    def test_version_input_is_env_mapped_validated_before_credentialed_step(self):
        workflow = WORKFLOW.read_text()
        activation = workflow.split("  activate:\n", 1)[1]
        self.assertNotIn('${{ inputs.version_id }}"', activation)
        self.assertIn("CF_VERSION_ID: ${{ inputs.version_id }}", activation)
        self.assertIn("version_id must be a UUID", activation)
        self.assertLess(activation.index("- name: Validate version ID"), activation.index("- name: Activate Worker"))
        self.assertIn('activate --version "$CF_VERSION_ID"', activation)
        self.assertIn("CF_VERSION_TAG: git-${{ github.sha }}", activation)


if __name__ == "__main__":
    unittest.main()
