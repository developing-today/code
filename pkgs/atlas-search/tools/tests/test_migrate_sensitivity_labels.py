"""Oracle: SQLite row values and a pre-migration backup define correctness."""

import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "migrate_sensitivity_labels.py"
PUBLISHED_SCHEMA = """
CREATE TABLE chunks (id INTEGER PRIMARY KEY, sensitivity TEXT);
CREATE TABLE session_labels (session_id TEXT PRIMARY KEY, sensitivity TEXT);
INSERT INTO chunks VALUES
  (1, 'shareable'), (2, 'personal'), (3, 'local-only'),
  (4, 'secret-bearing'), (5, 'public'), (6, 'sensitive'), (7, NULL), (8, 'unknown');
INSERT INTO session_labels VALUES
  ('a', 'shareable'), ('b', 'internal'), ('c', 'local-only'), ('d', 'secret-bearing');
"""
STAGING_SCHEMA = """
CREATE TABLE _schema_version (version INTEGER NOT NULL);
INSERT INTO _schema_version VALUES (1);
CREATE TABLE items (id INTEGER PRIMARY KEY, sensitivity TEXT NOT NULL DEFAULT 'internal');
CREATE TABLE chunks_raw (id INTEGER PRIMARY KEY);
CREATE TABLE notes (id INTEGER PRIMARY KEY);
CREATE TABLE kv (key TEXT PRIMARY KEY, value TEXT);
INSERT INTO items VALUES (1, 'public'), (2, 'internal'), (3, 'secret');
"""


class SensitivityMigrationTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="sensitivity-migration-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def create(self, name, schema=PUBLISHED_SCHEMA):
        path = self.root / name
        with sqlite3.connect(path) as connection:
            connection.executescript(schema)
        return path

    def run_cli(self, path, *arguments, code=0):
        result = subprocess.run(
            [sys.executable, str(SCRIPT), str(path), *map(str, arguments)],
            capture_output=True,
            text=True,
            timeout=20,
            check=False,
        )
        self.assertEqual(result.returncode, code, result.stderr)
        if code:
            self.assertEqual(result.stdout, "")
            return result.stderr
        self.assertEqual(result.stderr, "")
        return json.loads(result.stdout)

    def values(self, path):
        with sqlite3.connect(path) as connection:
            return {
                table: connection.execute(
                    f"SELECT id, sensitivity FROM {table} ORDER BY 1"
                    if table == "chunks"
                    else "SELECT session_id, sensitivity FROM session_labels ORDER BY 1"
                ).fetchall()
                for table in ("chunks", "session_labels")
            }

    def test_dry_run_reports_only_rung_typed_target_fields(self):
        database = self.create("dry-run.db")
        original = self.values(database)
        report = self.run_cli(database)

        self.assertEqual(report["profile"], "published")
        self.assertEqual(report["action"], "dry_run")
        self.assertEqual(report["before"]["chunks"]["migratable"], 4)
        self.assertEqual(report["before"]["session_labels"]["migratable"], 3)
        self.assertEqual(self.values(database), original)

    def test_apply_is_idempotent_and_reverse_restores_legacy_labels(self):
        database = self.create("reversible.db")
        original = self.values(database)
        backup = self.root / "before.db"

        applied = self.run_cli(database, "--apply", "--backup", backup)
        self.assertEqual(applied["action"], "applied")
        canonical = self.values(database)
        self.assertEqual(canonical["chunks"][0], (1, "public"))
        self.assertEqual(canonical["chunks"][1], (2, "internal"))
        self.assertEqual(canonical["chunks"][2], (3, "private"))
        self.assertEqual(canonical["chunks"][3], (4, "sensitive"))
        self.assertEqual(canonical["chunks"][4:], original["chunks"][4:])
        self.assertEqual(canonical["session_labels"][1][1], "internal")

        rerun_backup = self.root / "second.db"
        rerun = self.run_cli(database, "--apply", "--backup", rerun_backup)
        self.assertEqual(rerun["before"], rerun["after"])
        self.assertEqual(self.values(database), canonical)

        reverse_backup = self.root / "before-reverse.db"
        reversed_report = self.run_cli(
            database, "--apply", "--backup", reverse_backup, "--reverse"
        )
        self.assertEqual(reversed_report["action"], "applied")
        restored_values = {
            "chunks": [
                (row_id, {"public": "shareable", "internal": "personal", "private": "local-only", "sensitive": "secret-bearing"}.get(value, value))
                for row_id, value in canonical["chunks"]
            ],
            "session_labels": [
                (session_id, {"public": "shareable", "internal": "personal", "private": "local-only", "sensitive": "secret-bearing"}.get(value, value))
                for session_id, value in canonical["session_labels"]
            ],
        }
        self.assertEqual(self.values(database), restored_values)
        # The independently created pre-migration backup is the exact rollback
        # oracle, including canonical labels that were already present.
        self.assertEqual(self.values(backup), original)

    def test_failed_apply_rolls_back_and_keeps_valid_backup(self):
        database = self.create("failure.db")
        original = self.values(database)
        with sqlite3.connect(database) as connection:
            connection.execute(
                "CREATE TRIGGER reject_sensitivity BEFORE UPDATE OF sensitivity ON chunks "
                "BEGIN SELECT RAISE(ABORT, 'injected failure'); END"
            )
        backup = self.root / "rollback-copy.db"
        self.run_cli(database, "--apply", "--backup", backup, code=1)

        self.assertTrue(backup.is_file())
        with sqlite3.connect(backup) as connection:
            self.assertEqual(
                connection.execute("SELECT sensitivity FROM chunks ORDER BY id").fetchall(),
                [(row[1],) for row in original["chunks"]],
            )
        self.assertEqual(self.values(database), original)

    def test_staging_v1_is_reported_and_never_reinterpreted_or_changed(self):
        database = self.create("staging.db", STAGING_SCHEMA)
        before = sqlite3.connect(database).execute(
            "SELECT id, sensitivity FROM items ORDER BY id"
        ).fetchall()
        dry_run = self.run_cli(database)
        self.assertEqual(dry_run["profile"], "staging_v1")
        self.assertEqual(dry_run["action"], "skipped_non_rung_tier_store")
        self.assertEqual(dry_run["staging_tiers"], {"internal": 1, "public": 1, "secret": 1})

        backup = self.root / "staging-copy.db"
        applied = self.run_cli(database, "--apply", "--backup", backup)
        self.assertEqual(applied["changed"], 0)
        with sqlite3.connect(database) as connection:
            self.assertEqual(
                connection.execute("SELECT id, sensitivity FROM items ORDER BY id").fetchall(),
                before,
            )
        self.assertFalse(backup.exists())

    def test_refuses_partial_schemas_missing_backup_and_existing_backup(self):
        partial = self.create(
            "partial.db", "CREATE TABLE chunks (id INTEGER, sensitivity TEXT);"
        )
        self.run_cli(partial, code=1)

        database = self.create("guard.db")
        self.run_cli(database, "--apply", code=1)
        backup = self.root / "existing.db"
        backup.touch()
        error = self.run_cli(database, "--apply", "--backup", backup, code=1)
        self.assertIn("already exists", error)
        self.assertTrue(backup.is_file())


if __name__ == "__main__":
    unittest.main()
