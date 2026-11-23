#!/usr/bin/env python3
"""The restore drill must rebuild the target from the D1 copy alone.

Oracle: Python's sqlite3 module, twice over. The "D1 copy" is a
sqlite3 database loaded from a real export dump, served through a
fake D1Client whose query() answers with JSON-shaped dicts exactly
like the D1 HTTP API; the target is a second sqlite3 database behind
the LibsqlClient contract. After restore_drill.restore, every target
table must equal the source table the export was taken from, row for
row — the drill restores data, not just counts.

Run: python3 -I infra/fleet-queue-backup/tests/test_restore_drill.py
"""

import os
import sqlite3
import sys
import unittest

BIN = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "bin"))
sys.path.insert(0, BIN)

import fq_common
import restore_drill


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


class FakeD1:
    """The D1Client surface restore_drill uses, over a sqlite3 copy.

    query() returns a list of dicts (the D1 HTTP API's shape); blob
    cells are returned as lists of byte values, as the API does.
    """

    def __init__(self, conn):
        self.conn = conn

    def find_database(self, name):
        return {"name": name, "uuid": "fake-uuid"}

    def query(self, db_id, sql, params=()):
        cur = self.conn.execute(sql, tuple(params))
        cols = [d[0] for d in cur.description]
        out = []
        for row in cur.fetchall():
            record = {}
            for col, value in zip(cols, row):
                record[col] = list(value) if isinstance(value, bytes) else value
            out.append(record)
        return out

    def read_meta(self, db_id):
        import json

        rows = self.query(
            db_id,
            f'SELECT value FROM "{fq_common.META_TABLE}" WHERE key = ?',
            ("export",),
        )
        return json.loads(rows[0]["value"]) if rows else None


def make_source():
    conn = sqlite3.connect(":memory:")
    conn.executescript(
        """
        CREATE TABLE tasks (id INTEGER PRIMARY KEY, title TEXT, payload BLOB);
        CREATE TABLE notes (task_id INTEGER, note TEXT);
        CREATE INDEX idx_notes_task ON notes(task_id);
        """
    )
    conn.executemany(
        "INSERT INTO tasks VALUES (?,?,?)",
        [(1, "one", b"\x01\x02"), (2, "tw'o", None), (3, "three", b"")],
    )
    conn.executemany("INSERT INTO notes VALUES (?,?)", [(1, "a"), (3, "b")])
    conn.commit()
    return conn


class TestD1Value(unittest.TestCase):
    def test_scalars_pass_through(self):
        for value in (None, 7, 1.5, "text"):
            self.assertEqual(restore_drill.d1_value(value), value)

    def test_byte_list_becomes_bytes(self):
        self.assertEqual(restore_drill.d1_value([0, 255, 16]), b"\x00\xff\x10")

    def test_unknown_shape_fails(self):
        with self.assertRaises(ValueError):
            restore_drill.d1_value({"surprise": True})


class TestRestore(unittest.TestCase):
    def test_restore_rebuilds_target_from_copy(self):
        source = make_source()
        dump, _counts, _splits = fq_common.build_dump(
            SqliteLibsqlAdapter(source), "2026-11-22T00:00:00Z"
        )
        copy = sqlite3.connect(":memory:")
        copy.executescript(dump)
        # D1 adds its own _cf_KV table to every database; the drill
        # must skip it (the live API prohibits even reading it).
        copy.execute("CREATE TABLE _cf_KV (key TEXT PRIMARY KEY, value BLOB)")
        copy.execute("INSERT INTO _cf_KV VALUES ('k', X'00')")
        target_conn = sqlite3.connect(":memory:")
        target = SqliteLibsqlAdapter(target_conn)

        restore_drill.fq_common.D1Client = lambda token, account: FakeD1(copy)
        restore_drill.fq_common.LibsqlClient = lambda url, token: target
        os.environ["CLOUDFLARE_API_TOKEN"] = "test"
        os.environ["CLOUDFLARE_ACCOUNT_ID"] = "test"
        os.environ["TURSO_TARGET_TOKEN"] = "test"
        try:
            result = restore_drill.restore(
                fq_common.DEFAULT_D1_NAME, "https://scratch.invalid"
            )
        finally:
            for name in (
                "CLOUDFLARE_API_TOKEN",
                "CLOUDFLARE_ACCOUNT_ID",
                "TURSO_TARGET_TOKEN",
            ):
                del os.environ[name]
        self.assertEqual(result["tables"], 2)
        self.assertEqual(result["rows"], 5)
        self.assertEqual(
            target_conn.execute(
                "SELECT count(*) FROM sqlite_master WHERE name='_cf_KV'"
            ).fetchone()[0],
            0,
        )
        for table in ("tasks", "notes"):
            self.assertEqual(
                target_conn.execute(f"SELECT * FROM {table} ORDER BY 1, 2").fetchall(),
                source.execute(f"SELECT * FROM {table} ORDER BY 1, 2").fetchall(),
            )


if __name__ == "__main__":
    unittest.main()
