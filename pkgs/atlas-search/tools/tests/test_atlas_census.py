#!/usr/bin/env python3
"""Oracle: counts and store profiles in synthetic databases, including a WAL-only row."""

import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "atlas_census.py"
SECRET_SENTINEL = "SYNTHETIC_ATLAS_PAYLOAD_MUST_STAY_LOCAL"
VECTOR_TABLE = "vec_snowflake_m_v2_768_f32"


class CensusSmokeTest(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="atlas-private-fixture-")
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)

    def run_census(self, *arguments, expected_code):
        result = subprocess.run(
            [sys.executable, str(SCRIPT), *map(str, arguments)],
            capture_output=True,
            text=True,
            timeout=90,
            check=False,
        )
        for output in (result.stdout, result.stderr):
            self.assertNotIn(SECRET_SENTINEL, output)
            self.assertNotIn(str(self.root), output)
        self.assertEqual(result.returncode, expected_code, result.stderr)
        self.assertEqual(result.stderr, "")
        return json.loads(result.stdout)

    def create_database(self, name, schema):
        path = self.root / name
        connection = sqlite3.connect(path)
        try:
            connection.executescript(schema)
            connection.commit()
        finally:
            connection.close()
        return path

    def test_three_store_profiles_and_committed_wal(self):
        canonical = self.create_database(
            "canonical.db",
            """
            CREATE TABLE sessions (id INTEGER PRIMARY KEY, title TEXT);
            CREATE TABLE messages (id INTEGER PRIMARY KEY);
            CREATE TABLE chunks (id INTEGER PRIMARY KEY);
            INSERT INTO sessions VALUES (1, 'initial row');
        """,
        )
        shard = self.create_database(
            "private-shard.db",
            f"""
            CREATE TABLE chunks (id INTEGER PRIMARY KEY);
            CREATE TABLE {VECTOR_TABLE} (chunk_id INTEGER PRIMARY KEY, vec BLOB);
            CREATE TABLE atlas_manifest (key TEXT PRIMARY KEY, value TEXT);
            INSERT INTO atlas_manifest VALUES ('shard', 'synthetic'),
                ('embedding_model', 'synthetic'), ('expected_chunks', '0');
        """,
        )
        content = self.create_database(
            "private-content.db",
            """
            CREATE TABLE sessions (id INTEGER PRIMARY KEY);
            CREATE TABLE messages (id INTEGER PRIMARY KEY);
            INSERT INTO messages VALUES (1);
        """,
        )

        writer = sqlite3.connect(canonical)
        try:
            self.assertEqual(
                writer.execute("PRAGMA journal_mode=WAL").fetchone(), ("wal",)
            )
            writer.execute("PRAGMA wal_autocheckpoint=0")
            writer.execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchone()
            writer.execute("INSERT INTO sessions VALUES (?, ?)", (2, SECRET_SENTINEL))
            writer.commit()
            self.assertGreater(Path(str(canonical) + "-wal").stat().st_size, 0)
            # This fixture intentionally reads *only* the known checkpointed
            # base so the control cannot see the second, WAL-resident session.
            base = sqlite3.connect(canonical.as_uri() + "?immutable=1", uri=True)
            try:
                self.assertEqual(
                    base.execute("SELECT COUNT(*) FROM sessions").fetchone(), (1,)
                )
            finally:
                base.close()
            report = self.run_census(
                "--canonical",
                canonical,
                "--shard",
                shard,
                "--content-store",
                content,
                expected_code=0,
            )
        finally:
            writer.close()

        self.assertTrue(report["complete_required_counts"])
        main, scan, hydration = report["stores"]
        self.assertEqual(
            [s["role"] for s in report["stores"]],
            ["canonical", "scan_shard", "content_store"],
        )
        self.assertEqual([s["status"] for s in report["stores"]], ["ok"] * 3)
        self.assertEqual(main["snapshot_row_counts"]["sessions"], 2)
        self.assertEqual(main["snapshot_row_counts"]["chunks"], 0)
        self.assertEqual(main["table_dispositions"]["chunks"], "counted")
        self.assertIsNone(main["snapshot_row_counts"][VECTOR_TABLE])
        self.assertEqual(
            main["table_dispositions"][VECTOR_TABLE], "optional_table_absent"
        )
        self.assertEqual(scan["snapshot_row_counts"][VECTOR_TABLE], 0)
        self.assertEqual(scan["table_dispositions"]["sessions"], "not_applicable")
        self.assertEqual(hydration["snapshot_row_counts"]["messages"], 1)
        self.assertEqual(hydration["table_dispositions"]["chunks"], "not_applicable")
        with sqlite3.connect(shard) as changed:
            changed.execute(
                "UPDATE atlas_manifest SET value='4' WHERE key='expected_chunks'"
            )
        mismatch = self.run_census(
            "--canonical", canonical, "--shard", shard, expected_code=1
        )
        self.assertEqual(
            mismatch["stores"][1]["status"], "shard_manifest_count_mismatch"
        )

    def test_missing_table_missing_path_and_non_file(self):
        partial = self.create_database(
            "partial.db",
            """
            CREATE TABLE sessions (id INTEGER PRIMARY KEY);
            CREATE TABLE messages (id INTEGER PRIMARY KEY);
        """,
        )
        absent = self.root / "absent.db"
        directory = self.root / "a-directory"
        directory.mkdir()
        report = self.run_census(
            "--canonical",
            partial,
            "--shard",
            absent,
            "--content-store",
            directory,
            expected_code=1,
        )
        self.assertFalse(report["complete_required_counts"])
        self.assertEqual(
            [s["status"] for s in report["stores"]],
            ["required_table_missing", "file_missing", "not_a_file"],
        )
        self.assertEqual(
            report["stores"][0]["table_dispositions"]["chunks"],
            "required_table_missing",
        )
        self.assertIsNone(report["stores"][0]["snapshot_row_counts"]["chunks"])
        self.assertFalse(absent.exists())
        self.assertFalse(list(self.root.glob("absent.db-*")))

    def test_usage_error_names_offending_argument(self):
        for arguments, offender in [
            (["--seconds-per-store", "wrong"], "--seconds-per-store"),
            (["--bogus-flag"], "--bogus-flag"),
            (["--seconds-per-store"], "--seconds-per-store"),
        ]:
            result = subprocess.run(
                [sys.executable, str(SCRIPT), *arguments],
                capture_output=True,
                text=True,
                timeout=90,
                check=False,
            )
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertNotIn(str(self.root), result.stderr)
            self.assertIn("Invalid census arguments", result.stderr)
            if offender is not None:
                self.assertIn(offender, result.stderr)


if __name__ == "__main__":
    unittest.main()
