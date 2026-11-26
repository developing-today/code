"""Oracles: migration 0016/0017 shapes, SQLite catalog, read-only invariant."""

import hashlib
import importlib.util
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/inspect_sheet_staging_schema.py"
spec = importlib.util.spec_from_file_location("inspector", SCRIPT)
inspector = importlib.util.module_from_spec(spec)
spec.loader.exec_module(inspector)


class SchemaInspectorTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / "artifact ?#.sqlite"

    def database(self, sql=""):
        with sqlite3.connect(self.path) as db:
            db.executescript(sql)

    def test_oracle_0016_runtime_findings_not_workbook(self):
        self.database("""CREATE TABLE engineering_events (
            id INTEGER PRIMARY KEY, run_date TEXT NOT NULL, topic TEXT NOT NULL,
            headline TEXT NOT NULL, summary TEXT, source_url TEXT, source_name TEXT,
            event_date TEXT, dedupe_key TEXT NOT NULL UNIQUE,
            payload TEXT NOT NULL DEFAULT '{}', created_at TEXT NOT NULL);""")
        row = inspector.inspect_database(self.path)["tables"]["engineering_events"]
        self.assertEqual(row["gaps"], [])
        self.assertIn("guard behavior not exercised", row["unknown"])
        self.assertEqual(row["guards"], [])
        self.assertNotIn("import_batch", [c["name"] for c in row["columns"]])

    def test_oracle_missing_dedupe_or_partial_unique_fails(self):
        for suffix in (
            "",
            "CREATE UNIQUE INDEX partial ON engineering_events(dedupe_key) WHERE topic='x';",
        ):
            with self.subTest(suffix=suffix):
                if self.path.exists():
                    self.path.unlink()
                self.database(
                    """CREATE TABLE engineering_events (
                    id INTEGER PRIMARY KEY, run_date TEXT NOT NULL, topic TEXT NOT NULL,
                    headline TEXT NOT NULL, dedupe_key TEXT NOT NULL,
                    payload TEXT NOT NULL, created_at TEXT NOT NULL);"""
                    + suffix
                )
                row = inspector.inspect_database(self.path)["tables"][
                    "engineering_events"
                ]
                self.assertIn(
                    "missing unconditional unique key: dedupe_key", row["gaps"]
                )

    def test_oracle_0017_absent_source_fk_is_reported_not_invented(self):
        self.database("""CREATE TABLE automation_sheet_rows (
            id INTEGER PRIMARY KEY, source_id INTEGER NOT NULL,
            import_batch TEXT NOT NULL, sheet_name TEXT NOT NULL,
            row_index INTEGER NOT NULL, data TEXT NOT NULL,
            UNIQUE(source_id, import_batch, sheet_name, row_index));""")
        row = inspector.inspect_database(self.path)["tables"]["automation_sheet_rows"]
        self.assertIn(
            "source association unenforced: source_id -> automation_sources.id",
            row["gaps"],
        )
        self.assertEqual(row["foreign_keys"], [])

    def test_oracle_manifest_missing_and_home_unknown(self):
        self.database("CREATE TABLE home_events(id INTEGER PRIMARY KEY);")
        report = inspector.inspect_database(self.path)
        self.assertEqual(len(report["tables"]), 19)
        self.assertFalse(report["live_gate_cleared"])
        self.assertIn("missing table", report["tables"]["finance_events"]["gaps"])
        self.assertIn(
            "home contract unknown", report["tables"]["home_events"]["unknown"]
        )

    def test_oracle_sqlite_catalog_fk_and_composite_index(self):
        self.database("""CREATE TABLE research_sources(id INTEGER PRIMARY KEY);
            CREATE TABLE research_sheet_rows (
            id INTEGER PRIMARY KEY, source_id INTEGER NOT NULL REFERENCES research_sources(id),
            import_batch TEXT NOT NULL, sheet_name TEXT NOT NULL,
            row_index INTEGER NOT NULL, data TEXT NOT NULL);
            CREATE UNIQUE INDEX rows_key ON research_sheet_rows
            (source_id, import_batch, sheet_name, row_index);
            CREATE TRIGGER misleading_guard BEFORE DELETE ON research_sheet_rows
            BEGIN SELECT 1; END;""")
        row = inspector.inspect_database(self.path)["tables"]["research_sheet_rows"]
        self.assertEqual(row["gaps"], [])
        self.assertEqual(row["foreign_keys"][0]["table"], "research_sources")
        self.assertIn("CREATE UNIQUE INDEX", row["indexes"][0]["sql"])
        self.assertIn("guard behavior not exercised", row["unknown"])

    def test_oracle_read_only_bytes_and_missing_file_not_created(self):
        self.database()
        before = hashlib.sha256(self.path.read_bytes()).hexdigest()
        inspector.inspect_database(self.path)
        self.assertEqual(before, hashlib.sha256(self.path.read_bytes()).hexdigest())
        missing = self.path.parent / "absent.sqlite"
        with self.assertRaises(sqlite3.OperationalError):
            inspector.inspect_database(missing)
        self.assertFalse(missing.exists())

    def test_oracle_sqlite_identity_pk_and_nullability(self):
        for declaration, expected in (
            ("id TEXT", "missing single-column id primary key"),
            ("id TEXT UNIQUE", "missing single-column id primary key"),
            (
                "id TEXT, extra TEXT, PRIMARY KEY(id, extra)",
                "missing single-column id primary key",
            ),
            ("id TEXT PRIMARY KEY", "nullable primary identity: id"),
            ("id TEXT NOT NULL PRIMARY KEY", None),
            ("id INTEGER PRIMARY KEY", None),
        ):
            with self.subTest(declaration=declaration):
                if self.path.exists():
                    self.path.unlink()
                self.database(f"CREATE TABLE engineering_events ({declaration});")
                gaps = inspector.inspect_database(self.path)["tables"][
                    "engineering_events"
                ]["gaps"]
                identity_gaps = [g for g in gaps if "primary" in g]
                self.assertEqual(identity_gaps, [expected] if expected else [])

    def test_oracle_strict_cli_before_file_access(self):
        for extra in (["--typo"], ["stray"], ["--data", "x"], ["--database"]):
            result = subprocess.run(
                [
                    sys.executable,
                    "-I",
                    str(SCRIPT),
                    "--database",
                    str(self.path),
                    *extra,
                ],
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 2)
            self.assertIn("usage:", result.stderr)
            self.assertFalse(self.path.exists())

    def test_oracle_cli_json_is_local_not_live(self):
        self.database()
        result = subprocess.run(
            [sys.executable, "-I", str(SCRIPT), "--database=" + str(self.path)],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 1)
        self.assertFalse(json.loads(result.stdout)["live_gate_cleared"])


if __name__ == "__main__":
    unittest.main()
