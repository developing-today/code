#!/usr/bin/env python3
"""The cutover delta must bring a stale D1 copy level with the source.

Oracle: Python's sqlite3 module stands in for both ends. The source is
wrapped in the LibsqlClient (cols, rows) contract; the "D1" side is a
second sqlite3 database with foreign keys on, whose import runs the
delta's SQL as one transaction, the way D1's import does. The cases
are the ones a live cutover hit: a table that gained a column after
the bulk copy (github_mirror_issues, 2026-11-25), an append table that
only grew, an append table whose columns changed, and a child table
whose parent key is only a UNIQUE index.

Run: python3 -I infra/fleet-queue-backup/tests/test_delta_to_d1.py
"""

import os
import sqlite3
import sys
import unittest

BIN = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "bin"))
sys.path.insert(0, BIN)

import delta_to_d1
import fq_common


class Source:
    def __init__(self, conn):
        self.conn = conn

    def execute(self, sql, args=()):
        cur = self.conn.execute(sql, tuple(args))
        cols = [d[0] for d in cur.description] if cur.description else []
        return cols, [tuple(r) for r in cur.fetchall()]


class FakeD1:
    def __init__(self, conn):
        self.conn = conn
        self.conn.execute("PRAGMA foreign_keys = ON")
        self.imports = []

    def find_database(self, name):
        return {"uuid": "db-1", "name": name}

    def query(self, db_id, sql, params=()):
        cur = self.conn.execute(sql, tuple(params))
        cols = [d[0] for d in cur.description] if cur.description else []
        return [dict(zip(cols, r)) for r in cur.fetchall()]

    def import_sql(self, db_id, sql_text):
        self.imports.append(sql_text)
        body = "\n".join(
            line for line in sql_text.splitlines() if not line.startswith("PRAGMA defer_foreign_keys")
        )
        try:
            self.conn.executescript("BEGIN;\nPRAGMA defer_foreign_keys = true;\n" + body + "\nCOMMIT;")
        except Exception:
            if self.conn.in_transaction:
                self.conn.execute("ROLLBACK")
            raise

    def counts(self, db_id, names):
        return {n: self.query(db_id, f'SELECT count(*) AS n FROM "{n}"')[0]["n"] for n in names}


BASE = """
CREATE TABLE parents (id INTEGER PRIMARY KEY, key TEXT NOT NULL);
CREATE UNIQUE INDEX parents_key ON parents(key);
CREATE TABLE children (id INTEGER PRIMARY KEY, parent_key TEXT REFERENCES parents(key), v TEXT);
CREATE TABLE issues (id INTEGER PRIMARY KEY, title TEXT);
CREATE INDEX issues_title ON issues(title);
CREATE TABLE events (id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT);
CREATE TABLE history (id INTEGER PRIMARY KEY AUTOINCREMENT, note TEXT);
"""


def seed(conn):
    conn.executescript(BASE)
    conn.executemany("INSERT INTO parents VALUES (?, ?)", [(1, "a"), (2, "b")])
    conn.executemany("INSERT INTO children VALUES (?, ?, ?)", [(1, "a", "x"), (2, "b", "y")])
    conn.executemany("INSERT INTO issues VALUES (?, ?)", [(1, "one"), (2, "two")])
    conn.executemany("INSERT INTO events (kind) VALUES (?)", [("e1",), ("e2",)])
    conn.executemany("INSERT INTO history (note) VALUES (?)", [("h1",)])
    conn.commit()


def rows(conn, table):
    return conn.execute(f'SELECT * FROM "{table}" ORDER BY rowid').fetchall()


class TestDelta(unittest.TestCase):
    def setUp(self):
        self.src = sqlite3.connect(":memory:", isolation_level=None)
        self.dst = sqlite3.connect(":memory:", isolation_level=None)
        seed(self.src)
        seed(self.dst)
        # Writes after the bulk copy: an update, a new row, appends, a
        # new column on a replaced table and on an append table.
        self.src.executescript(
            """
            ALTER TABLE issues ADD COLUMN state TEXT;
            UPDATE issues SET title = 'one!', state = 'open' WHERE id = 1;
            INSERT INTO issues VALUES (3, 'three', 'closed');
            INSERT INTO parents VALUES (3, 'c');
            INSERT INTO children VALUES (3, 'c', 'z');
            INSERT INTO events (kind) VALUES ('e3');
            ALTER TABLE history ADD COLUMN who TEXT;
            INSERT INTO history (note, who) VALUES ('h2', 'me');
            CREATE TABLE brand_new (k TEXT PRIMARY KEY, v TEXT);
            INSERT INTO brand_new VALUES ('k', 'v');
            """
        )
        self.d1 = FakeD1(self.dst)
        self._orig = (fq_common.LibsqlClient, fq_common.D1Client)
        fq_common.LibsqlClient = lambda url, token: Source(self.src)
        fq_common.D1Client = lambda token, account: self.d1
        for k in ("TURSO_QUEUE_TOKEN", "CLOUDFLARE_API_TOKEN", "CLOUDFLARE_ACCOUNT_ID"):
            os.environ[k] = "test"

    def tearDown(self):
        fq_common.LibsqlClient, fq_common.D1Client = self._orig

    def test_brings_every_table_level(self):
        report, _total = delta_to_d1.delta("libsql://x", "db", ["events", "history"])
        for t in ("parents", "children", "issues", "events", "history", "brand_new"):
            self.assertEqual(rows(self.dst, t), rows(self.src, t), t)
        self.assertEqual(report["events"], "+1")
        self.assertEqual(report["history"], "=2")
        self.assertEqual(report["issues"], "=3")
        # Indexes come back after the replaced table was dropped.
        idx = {r[0] for r in self.dst.execute("SELECT name FROM sqlite_master WHERE type = 'index'")}
        self.assertTrue({"issues_title", "parents_key"} <= idx)
        self.assertEqual(self.dst.execute("PRAGMA foreign_key_check").fetchall(), [])

    def test_second_run_is_a_no_op(self):
        delta_to_d1.delta("libsql://x", "db", ["events", "history"])
        report, _ = delta_to_d1.delta("libsql://x", "db", ["events", "history"])
        self.assertEqual(report["events"], "+0")
        self.assertEqual(report["history"], "+0")
        for t in ("issues", "events", "history"):
            self.assertEqual(rows(self.dst, t), rows(self.src, t), t)


if __name__ == "__main__":
    unittest.main()
