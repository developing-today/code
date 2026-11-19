#!/usr/bin/env python3
"""Tests for scripts/dedupe_dump.py (#12000)."""

import sqlite3
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

FILTER = Path(__file__).resolve().parent.parent / "scripts" / "dedupe_dump.py"

VEC_CREATE = (
    "CREATE TABLE IF NOT EXISTS vec (chunk_id INTEGER PRIMARY KEY, v BLOB NOT NULL);\n"
)
VEC_ROWS = "INSERT INTO vec VALUES(1,X'00');\nINSERT INTO vec VALUES(4,X'01');\n"


def run(text):
    return subprocess.run(
        [sys.executable, str(FILTER)],
        input=text.encode(),
        capture_output=True,
        check=False,
    )


def rebuild(sql):
    with tempfile.TemporaryDirectory() as tmp:
        conn = sqlite3.connect(Path(tmp) / "out.db")
        try:
            conn.executescript(sql)
            return conn.execute("SELECT chunk_id FROM vec ORDER BY chunk_id").fetchall()
        finally:
            conn.close()


class TestDedupeDump(unittest.TestCase):
    def test_identical_repeated_table_is_dropped_and_rebuilds(self):
        # The atlas shape: the whole vec table, then all of it again.
        dump = (
            "BEGIN TRANSACTION;\n"
            + VEC_CREATE
            + VEC_ROWS
            + VEC_CREATE
            + VEC_ROWS
            + "CREATE TABLE IF NOT EXISTS after (x);\n"
            "INSERT INTO after VALUES(1);\n"
            "COMMIT;\n"
        )
        result = run(dump)
        self.assertEqual(result.returncode, 0, result.stderr)
        out = result.stdout.decode()
        self.assertEqual(out.count("INSERT INTO vec"), 2)
        self.assertEqual(out.count("CREATE TABLE IF NOT EXISTS vec"), 1)
        self.assertIn("INSERT INTO after VALUES(1);", out)
        self.assertTrue(out.endswith("COMMIT;\n"))
        self.assertIn(
            "dropped a repeated copy of table vec (2 rows identical to the first copy)",
            result.stderr.decode(),
        )
        self.assertNotIn("X'00'", result.stderr.decode())
        self.assertEqual(rebuild(out), [(1,), (4,)])

    def test_repeat_with_a_different_row_fails_and_drops_nothing_silently(self):
        dump = (
            VEC_CREATE
            + VEC_ROWS
            + VEC_CREATE
            + "INSERT INTO vec VALUES(1,X'00');\nINSERT INTO vec VALUES(4,X'02');\n"
        )
        result = run(dump)
        self.assertEqual(result.returncode, 1)
        self.assertIn("dumped twice with different rows", result.stderr.decode())

    def test_repeat_with_fewer_rows_fails(self):
        dump = VEC_CREATE + VEC_ROWS + VEC_CREATE + "INSERT INTO vec VALUES(1,X'00');\n"
        result = run(dump)
        self.assertEqual(result.returncode, 1)
        self.assertIn("(2 rows, then 1)", result.stderr.decode())

    def test_repeat_with_a_different_definition_fails(self):
        dump = (
            VEC_CREATE
            + VEC_ROWS
            + "CREATE TABLE IF NOT EXISTS vec (chunk_id INTEGER PRIMARY KEY, v BLOB);\n"
            + VEC_ROWS
        )
        result = run(dump)
        self.assertEqual(result.returncode, 1)
        self.assertIn("created twice with different definitions", result.stderr.decode())

    def test_multiline_create_is_matched_whole(self):
        create = "CREATE TABLE IF NOT EXISTS s(\n      id TEXT PRIMARY KEY);\n"
        row = "INSERT INTO s VALUES('a');\n"
        result = run(create + row + create + row + "COMMIT;\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.decode(), create + row + "COMMIT;\n")

    def test_quoted_table_names(self):
        create = "CREATE TABLE IF NOT EXISTS 'chunks_fts_data'(id INTEGER PRIMARY KEY, block BLOB);\n"
        row = "INSERT INTO \"chunks_fts_data\" VALUES(1,X'00');\n"
        result = run(create + row + create + row)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.decode(), create + row)

    def test_dump_without_repeats_passes_byte_identical(self):
        dump = (
            "PRAGMA foreign_keys=OFF;\n"
            "BEGIN TRANSACTION;\n"
            + VEC_CREATE
            + VEC_ROWS
            + "CREATE INDEX vec_idx ON vec (v);\n"
            "PRAGMA writable_schema=ON;\n"
            "INSERT INTO sqlite_schema(type,name,tbl_name,rootpage,sql)"
            "VALUES('table','f','f',0,'CREATE VIRTUAL TABLE f USING fts5(x)');\n"
            "PRAGMA writable_schema=OFF;\n"
            "COMMIT;\n"
        )
        result = run(dump)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.decode(), dump)
        self.assertEqual(result.stderr, b"")

    def test_repeat_at_end_of_input_is_checked(self):
        bad = VEC_CREATE + VEC_ROWS + VEC_CREATE + "INSERT INTO vec VALUES(9,X'00');\n"
        self.assertEqual(run(bad).returncode, 1)
        good = VEC_CREATE + VEC_ROWS + VEC_CREATE + VEC_ROWS
        result = run(good)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.decode(), VEC_CREATE + VEC_ROWS)


if __name__ == "__main__":
    unittest.main()
