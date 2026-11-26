#!/usr/bin/env python3
"""Hosted plain-mirror workflow contract.

Oracle: the workflow file and schedule registry must agree on the exact
daily cadence; the only triggers are manual dispatch and that schedule; the
publisher uses an immutable, repository-scoped App token and the strict
hosted-source and plain-mirror drivers. This is a wiring check, not proof of
a live publish (which is recorded from the Actions run).

Run: python3 -I infra/github-backup/tests/test_hosted_publisher_workflow.py
"""

import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[3]
WORKFLOW = ROOT / ".github/workflows/github-backup-hosted-publisher.yml"
REGISTRY = ROOT / "doc/fleet-schedule-registry.md"


class HostedPublisherWorkflowTests(unittest.TestCase):
    def test_workflow_is_dispatchable_scheduled_and_registered(self):
        workflow = WORKFLOW.read_text()
        registry = REGISTRY.read_text()

        self.assertRegex(
            workflow,
            r'(?m)^on:\n  workflow_dispatch:\n  schedule:\n    - cron: "17 4 \* \* \*"$',
        )
        self.assertNotRegex(workflow, r"(?m)^\s*(pull_request|pull_request_target):")
        self.assertRegex(
            registry,
            r"(?m)^\| `github-backup-hosted-publisher` \(`\.github/workflows/github-backup-hosted-publisher\.yml`\) \| developing-today/code \| `17 4 \* \* \*` \(daily 04:17 UTC\) \+ `workflow_dispatch` \|",
        )

    def test_publisher_uses_scoped_immutable_token_and_verified_drivers(self):
        workflow = WORKFLOW.read_text()

        self.assertIn(
            "actions/create-github-app-token@bcd2ba49218906704ab6c1aa796996da409d3eb1",
            workflow,
        )
        self.assertIn(
            "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
            workflow,
        )
        self.assertIn('app-id: "5562183"', workflow)
        self.assertIn("secrets.BACKUP_APP_PRIVATE_KEY", workflow)
        self.assertIn(
            "if: github.repository == 'developing-today/code' && github.ref == 'refs/heads/main'",
            workflow,
        )
        self.assertIn("owner: developing-today", workflow)
        self.assertIn("permission-contents: write", workflow)
        for repo in (
            "code-mirror",
            "fleet-mirror",
            "openchamber-mirror",
            "opencode-mirror",
        ):
            self.assertRegex(workflow, rf"(?m)^\s+{re.escape(repo)}$")
        self.assertIn("contents: read", workflow)
        self.assertIn("gh auth setup-git", workflow)
        self.assertIn("hosted_seed_fleet.py --root", workflow)
        self.assertIn("plain_mirror_all.py --root", workflow)
        self.assertIn("cancel-in-progress: false", workflow)


if __name__ == "__main__":
    unittest.main()
