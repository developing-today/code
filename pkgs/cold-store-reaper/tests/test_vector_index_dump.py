#!/usr/bin/env python3
"""Tests for scripts/vector_index_dump.py (#12000)."""

import sqlite3
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

FILTER = Path(__file__).resolve().parent.parent / "scripts" / "vector_index_dump.py"

INDEX = "CREATE INDEX vec_idx ON vec (libsql_vector_idx(v));"
# The atlas tail's shape: shadow tables are ordinary tables, the index
# statement comes last, inside the writable_schema block.
DUMP = (
    "PRAGMA foreign_keys=OFF;\n"
    "BEGIN TRANSACTION;\n"
    "CREATE TABLE IF NOT EXISTS vec (chunk_id INTEGER PRIMARY KEY, v F32_BLOB(2) NOT NULL);\n"
    "INSERT INTO vec VALUES(1,X'0000803f00000040');\n"
    "CREATE TABLE IF NOT EXISTS libsql_vector_meta_shadow ( name TEXT PRIMARY KEY, metadata BLOB ) WITHOUT ROWID;\n"
    "INSERT INTO libsql_vector_meta_shadow VALUES('vec_idx',X'01');\n"
    "CREATE TABLE IF NOT EXISTS vec_idx_shadow (index_key  INTEGER , data BLOB, PRIMARY KEY (index_key));\n"
    "INSERT INTO vec_idx_shadow VALUES(1,X'02');\n"
    "PRAGMA writable_schema=ON;\n"
    "CREATE INDEX vec_idx_shadow_idx ON vec_idx_shadow (index_key);\n" + INDEX + "\n"
    "PRAGMA writable_schema=OFF;\n"
    "COMMIT;\n"
)


def run(text):
    return subprocess.run(
        [sys.executable, str(FILTER)],
        input=text.encode(),
        capture_output=True,
        check=False,
    )


class TestVectorIndexDump(unittest.TestCase):
    def test_vector_index_is_recorded_and_the_dump_rebuilds(self):
        result = run(DUMP)
        self.assertEqual(result.returncode, 0, result.stderr)
        out = result.stdout.decode()
        self.assertNotIn("\n" + INDEX + "\n", out)
        self.assertIn("kept libsql vector index vec_idx", result.stderr.decode())
        with tempfile.TemporaryDirectory() as tmp:
            conn = sqlite3.connect(Path(tmp) / "out.db")
            try:
                conn.executescript(out)
                self.assertEqual(
                    conn.execute("PRAGMA integrity_check").fetchone(), ("ok",)
                )
                self.assertEqual(
                    conn.execute(
                        "SELECT name, sql FROM cold_store_libsql_vector_indexes"
                    ).fetchall(),
                    [("vec_idx", INDEX)],
                )
                # The index data itself is kept in the shadow tables.
                self.assertEqual(
                    conn.execute("SELECT count(*) FROM vec_idx_shadow").fetchone(), (1,)
                )
                self.assertEqual(
                    conn.execute(
                        "SELECT name FROM libsql_vector_meta_shadow"
                    ).fetchall(),
                    [("vec_idx",)],
                )
            finally:
                conn.close()

    def test_stock_sqlite_cannot_build_the_original(self):
        # Guards the premise: without the filter the rebuild fails.
        with tempfile.TemporaryDirectory() as tmp:
            conn = sqlite3.connect(Path(tmp) / "out.db")
            try:
                with self.assertRaises(sqlite3.OperationalError):
                    conn.executescript(DUMP)
            finally:
                conn.close()

    def test_quotes_in_the_statement_are_escaped(self):
        index = "CREATE INDEX \"it's\" ON vec (libsql_vector_idx(v, 'metric=cosine'));"
        result = run(index + "\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        conn = sqlite3.connect(":memory:")
        try:
            conn.executescript(result.stdout.decode())
            self.assertEqual(
                conn.execute(
                    "SELECT name, sql FROM cold_store_libsql_vector_indexes"
                ).fetchall(),
                [("it's", index)],
            )
        finally:
            conn.close()

    def test_two_vector_indexes_share_one_table(self):
        a = "CREATE INDEX a ON vec (libsql_vector_idx(v));"
        b = "CREATE INDEX b ON vec (libsql_vector_idx(v));"
        result = run(a + "\n" + b + "\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.decode().count("CREATE TABLE"), 1)

    def test_dump_without_a_vector_index_passes_byte_identical(self):
        dump = DUMP.replace(INDEX + "\n", "")
        result = run(dump)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.decode(), dump)
        self.assertEqual(result.stderr, b"")

    def test_unterminated_statement_fails(self):
        result = run("CREATE INDEX vec_idx ON vec (libsql_vector_idx(v)\n);\n")
        self.assertEqual(result.returncode, 1)


if __name__ == "__main__":
    unittest.main()
