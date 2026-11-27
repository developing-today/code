#!/usr/bin/env python3
"""Hosted publisher failure isolation (#15515 slice).

Oracle: static wiring of the workflow file — the same oracle family as
test_hosted_publisher_workflow.py. Live runs 45811916838 and 45848318658
showed a failed R2/rehost leg skipping every later leg, so the run hid
independent evidence (rehost skipped after an R2 failure; live-clone
verification skipped after a rehost failure). Each publish/verify leg
must carry `if: always()` so it attempts its run even when an earlier
leg failed; the job still reports failure when any leg did (GitHub
marks the job failed if any non-continue-on-error step fails).

Run: python3 -I infra/github-backup/tests/test_hosted_publisher_failure_isolation.py
"""

import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[3]
WORKFLOW = ROOT / ".github/workflows/github-backup-hosted-publisher.yml"


class PublisherFailureIsolationTests(unittest.TestCase):
    def test_publish_and_verify_legs_run_even_after_an_earlier_leg_fails(self):
        workflow = WORKFLOW.read_text()
        steps = re.split(r"(?m)^      - name: ", workflow)
        for name in ("Publish verified rehost packs", "Verify live rehost clones"):
            with self.subTest(step=name):
                step = next(step for step in steps if step.startswith(name + "\n"))
                self.assertRegex(step, r"(?m)^        if: always\(\)$")


if __name__ == "__main__":
    unittest.main()
