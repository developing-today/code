#!/usr/bin/env python3
"""Oversized rows: D1's import rejects any statement over 100,000
bytes (SQLITE_TOOBIG, seen live on the first #8584 export attempt).
A row whose quote()-rendered INSERT exceeds the exporter's cap is
split -- its largest cells chunked across an INSERT plus keyed UPDATE
appends -- and the rebuilt values must equal the source byte-for-byte,
with every emitted statement under the cap and every split recorded
for the exporter's post-import verification.

Oracle: Python's sqlite3 module, same adapter idiom as
test_fq_common.py. Run: python3 -I infra/fleet-queue-backup/tests/test_statement_split.py
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


def statements(dump):
    """Split a dump into statements on top-level semicolons (a ';'
    inside a quoted literal does not split; '' is an escaped quote)."""
    out, buf, i, in_str = [], [], 0, False
    while i < len(dump):
        ch = dump[i]
        buf.append(ch)
        if in_str:
            if ch == "'":
                if i + 1 < len(dump) and dump[i + 1] == "'":
                    buf.append("'")
                    i += 1
                else:
                    in_str = False
        elif ch == "'":
            in_str = True
        elif ch == ";":
            out.append("".join(buf))
            buf = []
        i += 1
    tail = "".join(buf).strip()
    if tail:
        out.append(tail)
    return out


class TestStatementSplit(unittest.TestCase):
    def test_oversized_text_and_blob_row_rebuilds_exactly(self):
        big_text = "data 'quoted' and \"dq\" ✓ line\n" * 5000  # ~165K chars
        big_blob = bytes(range(256)) * 700  # 179,200 bytes
        conn = sqlite3.connect(":memory:")
        conn.execute("CREATE TABLE wide (id INTEGER PRIMARY KEY, note TEXT, data BLOB)")
        conn.executemany(
            "INSERT INTO wide VALUES (?,?,?)",
            [(1, big_text, big_blob), (2, "small", b"\x00\xff")],
        )
        conn.commit()
        dump, counts, splits = fq_common.build_dump(
            SqliteLibsqlAdapter(conn), "2026-11-22T00:00:00Z"
        )
        self.assertEqual(counts, {"wide": 2})
        for stmt in statements(dump):
            self.assertLessEqual(
                len(stmt.encode("utf-8")),
                fq_common.MAX_IMPORT_STATEMENT_BYTES,
                stmt[:60],
            )
        rebuilt = sqlite3.connect(":memory:")
        rebuilt.executescript(dump)
        self.assertEqual(
            rebuilt.execute("SELECT * FROM wide ORDER BY id").fetchall(),
            [(1, big_text, big_blob), (2, "small", b"\x00\xff")],
        )
        recorded = {(t, c): (w, n) for t, c, w, n in splits}
        self.assertEqual(set(recorded), {("wide", "note"), ("wide", "data")})
        self.assertEqual(recorded[("wide", "note")][1], len(big_text))
        self.assertEqual(recorded[("wide", "data")][1], len(big_blob))
        self.assertIn('"id" IS 1', recorded[("wide", "note")][0])

    def test_oversized_row_without_pk_keys_appends_on_other_columns(self):
        big_text = "z" * 150_000
        conn = sqlite3.connect(":memory:")
        conn.execute("CREATE TABLE nopk (a TEXT, b TEXT)")
        conn.executemany(
            "INSERT INTO nopk VALUES (?,?)", [("k1", big_text), ("k2", "tiny")]
        )
        conn.commit()
        dump, _counts, splits = fq_common.build_dump(
            SqliteLibsqlAdapter(conn), "2026-11-22T00:00:00Z"
        )
        rebuilt = sqlite3.connect(":memory:")
        rebuilt.executescript(dump)
        self.assertEqual(
            rebuilt.execute("SELECT * FROM nopk ORDER BY a").fetchall(),
            [("k1", big_text), ("k2", "tiny")],
        )
        self.assertEqual(len(splits), 1)
        self.assertEqual(splits[0][3], len(big_text))

    def test_normal_rows_record_no_splits(self):
        conn = sqlite3.connect(":memory:")
        conn.execute("CREATE TABLE plain (id INTEGER PRIMARY KEY, v TEXT)")
        conn.execute("INSERT INTO plain VALUES (1, 'x')")
        conn.commit()
        _dump, _counts, splits = fq_common.build_dump(
            SqliteLibsqlAdapter(conn), "2026-11-22T00:00:00Z"
        )
        self.assertEqual(splits, [])


if __name__ == "__main__":
    unittest.main()
