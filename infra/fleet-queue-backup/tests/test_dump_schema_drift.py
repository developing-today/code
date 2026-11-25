#!/usr/bin/env python3
"""build_dump must not pair a stale CREATE with wider rows (#15081).

Oracle: Python's sqlite3 module. A wrapper adapter plays the live
store's mid-dump migration: immediately after build_dump's opening
schema capture returns, `notes` gains a column and one row is updated
to carry a value in it. The dump must still rebuild the source
exactly — each table's CREATE captured at the moment that table is
read, so schema and rows come from the same instant. On the pre-fix
code the dump pairs the captured 2-column CREATE with 3-value
INSERTs and executing it raises "table notes has 2 columns but 3
values were supplied" — the 2026-11-25 D1 import failure, reproduced
hermetically.

Run: python3 -I infra/fleet-queue-backup/tests/test_dump_schema_drift.py
"""

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


class DriftAfterSchemaCapture:
    """Adapter wrapper: the migration lands right after the opening
    full-schema read (the one ending in ORDER BY rowid) returns."""

    def __init__(self, conn):
        self.inner = SqliteLibsqlAdapter(conn)
        self.conn = conn
        self.fired = False

    def execute(self, sql, args=()):
        cols, rows = self.inner.execute(sql, args)
        if not self.fired and "FROM sqlite_master" in sql and "ORDER BY rowid" in sql:
            self.fired = True
            self.conn.execute("ALTER TABLE notes ADD COLUMN flag TEXT")
            self.conn.execute(
                "UPDATE notes SET flag = 'migrated' "
                "WHERE task_id = 1 AND note = 'first'"
            )
            self.conn.commit()
        return cols, rows

    def execute_many(self, statements):
        return self.inner.execute_many(statements)


class TestDumpSchemaDrift(unittest.TestCase):
    def test_column_added_after_schema_capture(self):
        conn = sqlite3.connect(":memory:")
        conn.executescript(
            """
            CREATE TABLE tasks (id INTEGER PRIMARY KEY, title TEXT);
            CREATE TABLE notes (task_id INTEGER, note TEXT);
            """
        )
        conn.executemany(
            "INSERT INTO tasks VALUES (?,?)", [(1, "one"), (2, "two")]
        )
        conn.executemany(
            "INSERT INTO notes VALUES (?,?)",
            [(1, "first"), (1, "second"), (2, "x")],
        )
        conn.commit()
        adapter = DriftAfterSchemaCapture(conn)
        dump, counts, _splits = fq_common.build_dump(
            adapter, "2026-11-25T00:00:00Z"
        )
        self.assertTrue(adapter.fired)
        self.assertEqual(counts, {"tasks": 2, "notes": 3})
        rebuilt = sqlite3.connect(":memory:")
        rebuilt.executescript(dump)
        self.assertEqual(
            rebuilt.execute("SELECT * FROM tasks ORDER BY id").fetchall(),
            [(1, "one"), (2, "two")],
        )
        self.assertEqual(
            rebuilt.execute(
                "SELECT * FROM notes ORDER BY task_id, note"
            ).fetchall(),
            [(1, "first", "migrated"), (1, "second", None), (2, "x", None)],
        )


if __name__ == "__main__":
    unittest.main()
