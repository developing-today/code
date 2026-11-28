#!/usr/bin/env python3
"""The backup docs distinguish the live queue from its D1 backup copy.

Oracle: README, runbook, and schedule registry consistently name the live
database `fleet-queue` and the backup copy `fleet-queue-backup`. The
runbook labels the current Turso source separately from the post-CF-2 D1
source; post-CF-3 (#14943) the registry records D1 `fleet-queue` as the
live source and the Turso leg as the retired former source.

Run: python3 -I tests/test_database_name_contract.py
"""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[3]


def _normalized(path):
    return " ".join(path.read_text(encoding="utf-8").split())


class DatabaseNameContractTests(unittest.TestCase):
    def test_readme_names_live_plane_and_backup_copy_separately(self):
        readme = _normalized(ROOT / "infra" / "fleet-queue-backup" / "README.md")
        self.assertTrue("live queue database is `fleet-queue`" in readme)
        self.assertTrue("separate backup D1 database is `fleet-queue-backup`" in readme)

    def test_runbook_distinguishes_current_and_target_sources(self):
        runbook = _normalized(
            ROOT / "infra" / "fleet-queue-backup" / "docs" / "RUNBOOK.md"
        )
        self.assertTrue("Current live source: Turso `fleet-queue`" in runbook)
        self.assertTrue("After CF-2: live source is D1 `fleet-queue`" in runbook)
        self.assertTrue("Backup target remains D1 `fleet-queue-backup`" in runbook)

    def test_schedule_registry_labels_source_and_backup_database(self):
        registry = _normalized(ROOT / "doc" / "fleet-schedule-registry.md")
        # Post-CF-3 (#14943): D1 `fleet-queue` is the live source; the Turso
        # leg is retired and recorded as the former source.
        self.assertTrue("former source: Turso `fleet-queue`" in registry)
        self.assertTrue("source: live D1 `fleet-queue`" in registry)
        self.assertTrue("backup D1 `fleet-queue-backup`" in registry)


if __name__ == "__main__":
    unittest.main()
