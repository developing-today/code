#!/usr/bin/env python3
"""The backup docs distinguish the live queue from its D1 backup copy.

Oracle: README, runbook, and schedule registry consistently name the live
database `fleet-queue` and the backup copy `fleet-queue-backup`, and the
runbook labels the current Turso source separately from the post-CF-2 D1
source.

Run: python3 -I tests/test_database_name_contract.py
"""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[3]


class DatabaseNameContractTests(unittest.TestCase):
    def test_readme_names_live_plane_and_backup_copy_separately(self):
        readme = (ROOT / "infra" / "fleet-queue-backup" / "README.md").read_text(
            encoding="utf-8"
        )
        self.assertTrue("Live queue database: `fleet-queue`" in readme)
        self.assertTrue("Backup D1 database: `fleet-queue-backup`" in readme)

    def test_runbook_distinguishes_current_and_target_sources(self):
        runbook = (
            ROOT / "infra" / "fleet-queue-backup" / "docs" / "RUNBOOK.md"
        ).read_text(encoding="utf-8")
        self.assertTrue("Current live source: Turso `fleet-queue`" in runbook)
        self.assertTrue("After CF-2: live source is D1 `fleet-queue`" in runbook)
        self.assertTrue("Backup target remains D1 `fleet-queue-backup`" in runbook)

    def test_schedule_registry_labels_source_and_backup_database(self):
        registry = (ROOT / "doc" / "fleet-schedule-registry.md").read_text(
            encoding="utf-8"
        )
        self.assertTrue("current source: Turso `fleet-queue`" in registry)
        self.assertTrue("CF-3 target source after CF-2: D1 `fleet-queue`" in registry)
        self.assertTrue("backup D1 `fleet-queue-backup`" in registry)


if __name__ == "__main__":
    unittest.main()
