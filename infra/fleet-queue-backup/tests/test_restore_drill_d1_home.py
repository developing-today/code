#!/usr/bin/env python3
"""The class-D restore drill must rebuild the D1 database from its
Turso home alone, into a local SQLite file, with counts matching the
home's import record.

Oracle: Python's sqlite3 module. The home is produced by running the
real export (test_export_to_turso.py's fakes) over a sqlite3 "D1
database"; the drill then rebuilds into a temp file, and the rebuilt
tables must equal the D1 tables row for row. A home tampered with
after the import must fail the drill, not pass a wrong copy.

Run: python3 -I infra/fleet-queue-backup/tests/test_restore_drill_d1_home.py
"""

import json
import os
import sqlite3
import sys
import tempfile
import unittest

BIN = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "bin"))
sys.path.insert(0, BIN)

import export_to_turso
import fq_common
import restore_drill_d1_home


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
    """The D1Client surface the export uses, over a sqlite3 copy."""

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
                if isinstance(value, bytes):
                    value = list(value)
                elif isinstance(value, int) and abs(value) > 2**53:
                    value = int(float(value))
                record[col] = value
            out.append(record)
        return out

    def counts(self, db_id, names):
        out = {}
        for name in names:
            rows = self.query(db_id, f'SELECT count(*) AS n FROM "{name}"')
            out[name] = rows[0]["n"]
        return out


def make_d1():
    conn = sqlite3.connect(":memory:")
    conn.executescript(
        """
        CREATE TABLE tasks (id INTEGER PRIMARY KEY, title TEXT, payload BLOB);
        CREATE TABLE seq (name TEXT PRIMARY KEY, max INTEGER);
        CREATE TABLE _fleet_queue_backup_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE INDEX idx_tasks_title ON tasks(title);
        """
    )
    conn.executemany(
        "INSERT INTO tasks VALUES (?,?,?)",
        [(1, "one", b"\x01\x02"), (2, "tw'o", None), (3, "three", b"")],
    )
    conn.executemany(
        "INSERT INTO seq VALUES (?,?)",
        [("tasks", 9223372036854775807), ("events", 21500)],
    )
    conn.execute(
        "INSERT INTO _fleet_queue_backup_meta VALUES ('export', ?)",
        (json.dumps({"exported_at": "2026-11-23T00:00:00Z", "counts": {}}),),
    )
    conn.commit()
    return conn


class TestDrill(unittest.TestCase):
    def setUp(self):
        self.d1_conn = make_d1()
        self.home_conn = sqlite3.connect(":memory:")
        self.home = SqliteLibsqlAdapter(self.home_conn)
        fake = FakeD1(self.d1_conn)
        export_to_turso.fq_common.D1Client = lambda token, account: fake
        export_to_turso.fq_common.LibsqlClient = lambda url, token: self.home
        self._env = {
            name: os.environ.get(name)
            for name in (
                "CLOUDFLARE_API_TOKEN",
                "CLOUDFLARE_ACCOUNT_ID",
                "TURSO_HOME_TOKEN",
            )
        }
        for name in self._env:
            os.environ[name] = "test"
        self.tmp = tempfile.TemporaryDirectory()
        self.out = os.path.join(self.tmp.name, "rebuilt.db")

    def tearDown(self):
        for name, value in self._env.items():
            if value is None:
                del os.environ[name]
            else:
                os.environ[name] = value
        self.tmp.cleanup()

    def test_drill_rebuilds_from_home_alone(self):
        export_to_turso.export(fq_common.DEFAULT_D1_NAME, "https://home.invalid")
        result = restore_drill_d1_home.restore(
            "https://home.invalid", self.out, fq_common.DEFAULT_D1_NAME
        )
        self.assertEqual(result["tables"], 3)
        self.assertEqual(result["rows"], 3 + 2 + 1)
        rebuilt = sqlite3.connect(self.out)
        for table in ("tasks", "seq", "_fleet_queue_backup_meta"):
            self.assertEqual(
                rebuilt.execute(
                    f'SELECT * FROM "{table}" ORDER BY rowid'
                ).fetchall(),
                self.d1_conn.execute(
                    f'SELECT * FROM "{table}" ORDER BY rowid'
                ).fetchall(),
            )
        self.assertEqual(
            rebuilt.execute(
                "SELECT max FROM seq WHERE name = 'tasks'"
            ).fetchone()[0],
            9223372036854775807,
        )
        rebuilt.close()

    def test_drill_fails_on_tampered_home(self):
        export_to_turso.export(fq_common.DEFAULT_D1_NAME, "https://home.invalid")
        self.home_conn.execute("DELETE FROM tasks WHERE id = 2")
        self.home_conn.commit()
        with self.assertRaises(RuntimeError):
            restore_drill_d1_home.restore("https://home.invalid", self.out, None)

    def test_drill_fails_without_import_record(self):
        with self.assertRaises(RuntimeError):
            restore_drill_d1_home.restore("https://home.invalid", self.out, None)


if __name__ == "__main__":
    unittest.main()
