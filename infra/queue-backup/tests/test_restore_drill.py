#!/usr/bin/env python3
"""The D1 restore drill proves the restored queue store is claimable.

Oracle: after export -> D1 copy -> restore, the restored store has the
manifest's schema fingerprint and per-table counts, and the queue's claim
shape (the conditional UPDATE in turso_queue/claim.rs, queued branch)
claims exactly the seeded claimable task — the priority-90 queued task —
once, and no second time.

Run: python3 -I infra/queue-backup/tests/test_restore_drill.py
"""

import json
import os
import sqlite3
import sys
import tempfile
import unittest

BIN = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "bin"))
sys.path.insert(0, BIN)
TESTS = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, TESTS)

import export_d1
import restore_drill
from test_export_d1 import make_fixture


class RestoreDrillTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.source = os.path.join(self.tmp.name, "queue.db")
        make_fixture(self.source)
        self.dump = os.path.join(self.tmp.name, "queue-d1.sql")
        self.manifest = os.path.join(self.tmp.name, "manifest.json")
        export_d1.export(self.source, self.dump, self.manifest)
        self.restored = os.path.join(self.tmp.name, "restored.db")

    def run_drill(self, *extra):
        return (
            restore_drill.drill(
                self.dump,
                self.manifest,
                self.restored,
                worker="drill-worker",
                allow_empty=False,
            )
            if not extra
            else restore_drill.drill(
                self.dump,
                self.manifest,
                self.restored,
                worker="drill-worker",
                allow_empty=True,
            )
        )

    def test_round_trip_counts_schema_and_claim(self):
        report = self.run_drill()
        self.assertTrue(report["ok"], report)
        self.assertEqual(report["claimed_task_id"], 1)
        conn = sqlite3.connect(self.restored)
        try:
            row = conn.execute(
                "SELECT status, worker_id, attempts, lease_until"
                " FROM tasks WHERE id = 1"
            ).fetchone()
            self.assertEqual(row[0], "claimed")
            self.assertEqual(row[1], "drill-worker")
            self.assertEqual(row[2], 1)
            self.assertIsNotNone(row[3])
            untouched = conn.execute(
                "SELECT status, attempts FROM tasks WHERE id IN (2, 3, 5) ORDER BY id"
            ).fetchall()
            self.assertEqual(
                untouched,
                [("queued", 0), ("queued", 1000), ("parked", 0)],
            )
        finally:
            conn.close()

    def test_drill_rejects_a_tampered_manifest(self):
        with open(self.manifest, "r", encoding="utf-8") as handle:
            manifest = json.load(handle)
        manifest["tables"]["tasks"] = 99
        with open(self.manifest, "w", encoding="utf-8") as handle:
            json.dump(manifest, handle)
        with self.assertRaises(restore_drill.DrillError):
            self.run_drill()

    def test_drill_rejects_a_tampered_dump(self):
        with open(self.dump, "a", encoding="utf-8") as handle:
            handle.write("INSERT INTO lanes VALUES ('evil', 'w', 'q', 1);\n")
        with self.assertRaises(restore_drill.DrillError):
            self.run_drill()

    def test_no_claimable_task_fails_unless_allowed(self):
        conn = sqlite3.connect(self.source)
        try:
            conn.execute("UPDATE tasks SET status = 'done' WHERE id = 1")
            conn.commit()
        finally:
            conn.close()
        export_d1.export(self.source, self.dump, self.manifest, force=True)
        with self.assertRaises(restore_drill.DrillError):
            self.run_drill()
        os.unlink(self.restored)  # the failed drill leaves its scratch target
        report = self.run_drill("--allow-empty")
        self.assertTrue(report["ok"], report)
        self.assertIsNone(report["claimed_task_id"])


if __name__ == "__main__":
    unittest.main()
