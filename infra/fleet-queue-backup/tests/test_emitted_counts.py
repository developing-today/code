#!/usr/bin/env python3
"""Emitted-row counts (#8584 live run 4): build_dump's counts must
describe the dump itself, so a row that lands in the source while the
export is reading (the normal case on the live queue store) is counted
-- the meta, the D1 read-back check and the restore drill then all
describe the same copy. With a separate COUNT(*) pass the counts
described the store as of dump start and the read-back check failed
on a faithful copy.

Oracle: Python's sqlite3 module, same adapter idiom as
test_fq_common.py. Run: python3 -I infra/fleet-queue-backup/tests/test_emitted_counts.py
"""

import os
import sqlite3
import sys
import unittest

BIN = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "bin"))
sys.path.insert(0, BIN)
sys.path.insert(0, os.path.dirname(__file__))

import fq_common
from test_fq_common import SqliteLibsqlAdapter


class DriftAdapter(SqliteLibsqlAdapter):
    """Inserts one extra tasks row the first time the dump probes the
    table -- a write landing mid-export, as the live store does."""

    def __init__(self, conn):
        super().__init__(conn)
        self.drifted = False

    def execute(self, sql, args=()):
        if not self.drifted and "LIMIT 0" in sql and '"tasks"' in sql:
            self.drifted = True
            self.conn.execute(
                "INSERT INTO tasks VALUES (99, 'late', '', NULL, 0.0, NULL)"
            )
            self.conn.commit()
        return super().execute(sql, args)


class TestEmittedCounts(unittest.TestCase):
    def test_counts_describe_the_dump_not_the_count_pass(self):
        conn = sqlite3.connect(":memory:")
        conn.executescript(
            """
            CREATE TABLE tasks (id INTEGER PRIMARY KEY, title TEXT, body TEXT,
                                payload BLOB, score REAL, parent INTEGER);
            """
        )
        conn.executemany(
            "INSERT INTO tasks VALUES (?,?,?,?,?,?)",
            [(1, "a", "", None, 0.0, None), (2, "b", "", None, 0.0, None)],
        )
        conn.commit()
        dump, counts, _splits = fq_common.build_dump(
            DriftAdapter(conn), "2026-11-22T00:00:00Z"
        )
        self.assertEqual(counts, {"tasks": 3})
        rebuilt = sqlite3.connect(":memory:")
        rebuilt.executescript(dump)
        self.assertEqual(
            rebuilt.execute("SELECT count(*) FROM tasks").fetchone()[0], 3
        )


if __name__ == "__main__":
    unittest.main()
