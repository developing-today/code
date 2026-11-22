#!/usr/bin/env python3
"""The export dump must rebuild the source store exactly.

Oracle: Python's sqlite3 module. A source database is wrapped in the
LibsqlClient (cols, rows) contract; fq_common.build_dump renders it;
the dump is executed into a fresh sqlite3 database; every table's
rows must equal the source's rows exactly — including the values
chosen to break naive SQL escaping (quotes, newlines, unicode,
blobs, NULLs) — and the meta table must record the same counts.

Run: python3 -I infra/fleet-queue-backup/tests/test_fq_common.py
"""

import json
import os
import sqlite3
import sys
import unittest

BIN = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "bin"))
sys.path.insert(0, BIN)

import fq_common


class SqliteLibsqlAdapter:
    """The LibsqlClient contract over a sqlite3 connection."""

    def __init__(self, conn):
        self.conn = conn

    def execute(self, sql, args=()):
        cur = self.conn.execute(sql, tuple(args))
        cols = [d[0] for d in cur.description] if cur.description else []
        return cols, [tuple(r) for r in cur.fetchall()]

    def execute_many(self, statements):
        with self.conn:
            for sql, args in statements:
                self.conn.execute(sql, tuple(args))


def make_source():
    conn = sqlite3.connect(":memory:")
    conn.executescript(
        """
        CREATE TABLE tasks (id INTEGER PRIMARY KEY, title TEXT, body TEXT,
                            payload BLOB, score REAL, parent INTEGER);
        CREATE TABLE notes (task_id INTEGER, note TEXT);
        CREATE INDEX idx_notes_task ON notes(task_id);
        CREATE VIEW task_titles AS SELECT id, title FROM tasks;
        """
    )
    rows = [
        (1, "plain", "line one\nline two", b"\x00\xff\x10", 1.5, None),
        (2, "it's a 'quoted' title", 'semi;colon -- comment', b"", -0.25, 1),
        (3, "ünïcode ✓", "", None, 0.0, 1),
    ]
    conn.executemany("INSERT INTO tasks VALUES (?,?,?,?,?,?)", rows)
    conn.executemany(
        "INSERT INTO notes VALUES (?,?)", [(1, "first"), (1, "second"), (2, "x")]
    )
    conn.commit()
    return conn, rows


class TestBuildDump(unittest.TestCase):
    def test_dump_rebuilds_source_exactly(self):
        conn, task_rows = make_source()
        dump, counts = fq_common.build_dump(
            SqliteLibsqlAdapter(conn), "2026-11-22T00:00:00Z"
        )
        self.assertEqual(counts, {"tasks": 3, "notes": 3})
        rebuilt = sqlite3.connect(":memory:")
        rebuilt.executescript(dump)
        self.assertEqual(
            rebuilt.execute("SELECT * FROM tasks ORDER BY id").fetchall(), task_rows
        )
        self.assertEqual(
            rebuilt.execute("SELECT * FROM notes ORDER BY task_id, note").fetchall(),
            [(1, "first"), (1, "second"), (2, "x")],
        )
        self.assertEqual(
            rebuilt.execute("SELECT count(*) FROM task_titles").fetchone()[0], 3
        )
        meta = rebuilt.execute(
            f"SELECT value FROM {fq_common.META_TABLE} WHERE key='export'"
        ).fetchone()
        self.assertIsNotNone(meta)
        recorded = json.loads(meta[0])
        self.assertEqual(recorded["exported_at"], "2026-11-22T00:00:00Z")
        self.assertEqual(recorded["counts"], {"notes": 3, "tasks": 3})

    def test_dump_is_reimportable(self):
        conn, _rows = make_source()
        adapter = SqliteLibsqlAdapter(conn)
        dump, _counts = fq_common.build_dump(adapter, "2026-11-22T00:00:00Z")
        # A second export over the rebuilt copy renders the same rows:
        # DROP+CREATE makes the dump idempotent against a live target.
        rebuilt = sqlite3.connect(":memory:")
        rebuilt.executescript(dump)
        rebuilt.executescript(dump)
        self.assertEqual(
            rebuilt.execute("SELECT count(*) FROM tasks").fetchone()[0], 3
        )


class TestPagination(unittest.TestCase):
    def test_quoted_reader_pages_to_completion(self):
        conn = sqlite3.connect(":memory:")
        conn.execute("CREATE TABLE big (n INTEGER, label TEXT)")
        conn.executemany(
            "INSERT INTO big VALUES (?,?)",
            [(i, f"row-{i}") for i in range(2 * fq_common.PAGE_ROWS + 7)],
        )
        conn.commit()
        seen = list(
            fq_common.read_table_quoted(SqliteLibsqlAdapter(conn), "big", 2)
        )
        self.assertEqual(len(seen), 2 * fq_common.PAGE_ROWS + 7)
        self.assertEqual(seen[0], ("0", "'row-0'"))


class TestHranaValues(unittest.TestCase):
    def test_round_trip(self):
        corpus = [None, 0, -42, 2**62, 1.25, -0.5, "", "text ✓", b"\x00\x01\xfe"]
        for value in corpus:
            self.assertEqual(fq_common.python_value(fq_common.hrana_value(value)), value)

    def test_bool_is_integer(self):
        self.assertEqual(
            fq_common.hrana_value(True), {"type": "integer", "value": "1"}
        )


if __name__ == "__main__":
    unittest.main()
