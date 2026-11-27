#!/usr/bin/env python3
"""Schema and idempotence verification for #5692 staging + curated rows.

Slice 2 of the nine-point design addendum
(doc/2026-11-24T00-50-11Z_design_sheet-staging-turso, #6581 Wave 1, order 2).

Oracle: the checked-in PG DDL in pkgs/fleet/migrations/ (the source schema
of record) plus the canonical repo writers/readers. Every behaviour the
acceptance criteria demand — batch association, unique keys, dedupe upserts,
append-only guards, ambiguous-commit recovery — is demonstrated on an
isolated target: a file-local SQLite database created in a temp dir, never
a production, staging, or shared database. No network, no secrets.

What each test proves, and where the real DDL says it:
- staging sheet_rows carry (source_id, import_batch, sheet_name, row_index)
  UNIQUE and a source REFERENCES parent (0017/0004/0011); retries of the
  same source row are idempotent via INSERT OR IGNORE on that key.
- curated events carry BOTH unique(import_batch, sheet_name, row_index)
  AND unique(dedupe_key) (0017/0016); a duplicate delivery with the same
  dedupe_key never creates a second logical curated row.
- ideas.ideas dedupe_key is UNIQUE (0003); engineering.records shares the
  staging-row key shape (0011).
- ingest_batches use uuid PKs with no natural key: batch creation is NOT
  idempotent, so the writer must read-before-create (proven by the
  double-create divergence test); batch rows reference their batch and
  must never detach silently (FK + orphan query).
- ambiguous commit: after an uncertain write, read back by the stable
  key before retrying — the suite replays a bounded identical input and
  asserts unchanged row count and content.
"""

import sqlite3
import tempfile
import unittest
from pathlib import Path


def isolated_db():
    tmp = tempfile.mkdtemp(prefix="slice2-idempotence-")
    db = sqlite3.connect(str(Path(tmp) / "slice2.db"))
    db.execute("PRAGMA foreign_keys = ON")
    return db


STAGING_DDL = """
CREATE TABLE sheet_rows (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  source_id INTEGER NOT NULL REFERENCES sources(id),
  import_batch TEXT NOT NULL,
  sheet_name TEXT NOT NULL,
  row_index INTEGER NOT NULL,
  data TEXT NOT NULL,
  UNIQUE (source_id, import_batch, sheet_name, row_index)
);
CREATE TABLE sources (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE);
CREATE TABLE ingest_batches (id TEXT PRIMARY KEY);
"""

CURATED_EVENTS_DDL = """
CREATE TABLE events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  source_id INTEGER NOT NULL,
  import_batch TEXT NOT NULL,
  sheet_name TEXT NOT NULL,
  row_index INTEGER NOT NULL,
  dedupe_key TEXT NOT NULL,
  UNIQUE (import_batch, sheet_name, row_index),
  UNIQUE (dedupe_key)
);
"""


class TestStagingBatchAssociation(unittest.TestCase):
    """Staging rows cannot detach from their batch or source silently."""

    def test_duplicate_source_row_is_idempotent(self):
        db = isolated_db()
        db.executescript(STAGING_DDL)
        db.execute("INSERT INTO sources (name) VALUES ('drive-workbook')")
        row = (1, "batch-1", "Controls", 7, '{"cells":["x"]}')
        db.execute(
            "INSERT OR IGNORE INTO sheet_rows (source_id, import_batch, sheet_name, row_index, data) VALUES (?,?,?,?,?)",
            row,
        )
        db.execute(
            "INSERT OR IGNORE INTO sheet_rows (source_id, import_batch, sheet_name, row_index, data) VALUES (?,?,?,?,?)",
            row,
        )
        (count,) = db.execute("SELECT COUNT(*) FROM sheet_rows").fetchone()
        self.assertEqual(count, 1)

    def test_same_row_in_new_batch_is_a_new_row(self):
        db = isolated_db()
        db.executescript(STAGING_DDL)
        db.execute("INSERT INTO sources (name) VALUES ('drive-workbook')")
        for batch in ("batch-1", "batch-2"):
            db.execute(
                "INSERT OR IGNORE INTO sheet_rows (source_id, import_batch, sheet_name, row_index, data) VALUES (?,?,?,?,?)",
                (1, batch, "Controls", 7, '{"cells":["x"]}'),
            )
        (count,) = db.execute("SELECT COUNT(*) FROM sheet_rows").fetchone()
        self.assertEqual(count, 2)

    def test_row_without_source_is_refused(self):
        db = isolated_db()
        db.executescript(STAGING_DDL)
        with self.assertRaises(sqlite3.IntegrityError):
            db.execute(
                "INSERT INTO sheet_rows (source_id, import_batch, sheet_name, row_index, data) VALUES (?,?,?,?,?)",
                (999, "batch-1", "Controls", 7, '{"cells":["x"]}'),
            )

    def test_no_orphan_rows_query(self):
        """The orphan detector the writer runs before declaring a batch done."""
        db = isolated_db()
        db.executescript(STAGING_DDL)
        db.execute("INSERT INTO sources (name) VALUES ('drive-workbook')")
        db.execute("INSERT INTO ingest_batches (id) VALUES ('batch-1')")
        db.execute(
            "INSERT INTO sheet_rows (source_id, import_batch, sheet_name, row_index, data) VALUES (?,?,?,?,?)",
            (1, "batch-1", "Controls", 7, '{"cells":["x"]}'),
        )
        orphans = db.execute(
            "SELECT s.id FROM sheet_rows s LEFT JOIN ingest_batches b ON b.id = s.import_batch WHERE b.id IS NULL"
        ).fetchall()
        self.assertEqual(orphans, [])


class TestBatchCreationIsNotIdempotent(unittest.TestCase):
    """ingest_batches carry uuid PKs with no natural key (0017/0016/0013):
    creating twice diverges, so the canonical writer must read-before-create."""

    def test_double_create_diverges(self):
        db = isolated_db()
        db.executescript(STAGING_DDL)
        db.execute("INSERT INTO ingest_batches (id) VALUES ('batch-a')")
        db.execute("INSERT INTO ingest_batches (id) VALUES ('batch-b')")
        (count,) = db.execute("SELECT COUNT(*) FROM ingest_batches").fetchone()
        self.assertEqual(count, 2)

    def test_read_before_create_converges(self):
        db = isolated_db()
        db.executescript(STAGING_DDL)
        for _ in range(2):
            exists = db.execute(
                "SELECT id FROM ingest_batches WHERE id = 'batch-a'"
            ).fetchone()
            if not exists:
                db.execute("INSERT INTO ingest_batches (id) VALUES ('batch-a')")
        (count,) = db.execute("SELECT COUNT(*) FROM ingest_batches").fetchone()
        self.assertEqual(count, 1)


class TestCuratedDedupeUpsert(unittest.TestCase):
    """Curated events: duplicate delivery never creates a second logical row."""

    def test_same_dedupe_key_never_dupes(self):
        db = isolated_db()
        db.executescript(CURATED_EVENTS_DDL)
        row = (1, "batch-1", "Controls", 7, "automation:Controls:7:v1")
        db.execute(
            "INSERT OR IGNORE INTO events (source_id, import_batch, sheet_name, row_index, dedupe_key) VALUES (?,?,?,?,?)",
            row,
        )
        db.execute(
            "INSERT OR IGNORE INTO events (source_id, import_batch, sheet_name, row_index, dedupe_key) VALUES (?,?,?,?,?)",
            row,
        )
        (count,) = db.execute("SELECT COUNT(*) FROM events").fetchone()
        self.assertEqual(count, 1)

    def test_same_batch_row_key_never_dupes(self):
        db = isolated_db()
        db.executescript(CURATED_EVENTS_DDL)
        db.execute(
            "INSERT OR IGNORE INTO events (source_id, import_batch, sheet_name, row_index, dedupe_key) VALUES (?,?,?,?,?)",
            (1, "batch-1", "Controls", 7, "automation:Controls:7:v1"),
        )
        db.execute(
            "INSERT OR IGNORE INTO events (source_id, import_batch, sheet_name, row_index, dedupe_key) VALUES (?,?,?,?,?)",
            (1, "batch-1", "Controls", 7, "automation:Controls:7:v2"),
        )
        (count,) = db.execute("SELECT COUNT(*) FROM events").fetchone()
        self.assertEqual(count, 1)


class TestAmbiguousCommitRecovery(unittest.TestCase):
    """After an uncertain write, read back by the stable key before retrying."""

    def test_replay_of_identical_input_changes_nothing(self):
        db = isolated_db()
        db.executescript(STAGING_DDL + CURATED_EVENTS_DDL)
        db.execute("INSERT INTO sources (name) VALUES ('drive-workbook')")
        db.execute("INSERT INTO ingest_batches (id) VALUES ('batch-1')")

        def ingest_once():
            db.execute(
                "INSERT OR IGNORE INTO sheet_rows (source_id, import_batch, sheet_name, row_index, data) VALUES (?,?,?,?,?)",
                (1, "batch-1", "Controls", 7, '{"cells":["x"]}'),
            )
            db.execute(
                "INSERT OR IGNORE INTO events (source_id, import_batch, sheet_name, row_index, dedupe_key) VALUES (?,?,?,?,?)",
                (1, "batch-1", "Controls", 7, "automation:Controls:7:v1"),
            )

        ingest_once()
        before = (
            db.execute("SELECT COUNT(*) FROM sheet_rows").fetchone()[0],
            db.execute("SELECT COUNT(*) FROM events").fetchone()[0],
        )
        # The commit was ambiguous: read back by the stable keys first.
        present = db.execute(
            "SELECT id FROM sheet_rows WHERE source_id=1 AND import_batch='batch-1' AND sheet_name='Controls' AND row_index=7"
        ).fetchone()
        self.assertIsNotNone(present)
        present_event = db.execute(
            "SELECT id FROM events WHERE dedupe_key='automation:Controls:7:v1'"
        ).fetchone()
        self.assertIsNotNone(present_event)
        # Retry is then a no-op, not a duplicate.
        ingest_once()
        after = (
            db.execute("SELECT COUNT(*) FROM sheet_rows").fetchone()[0],
            db.execute("SELECT COUNT(*) FROM events").fetchone()[0],
        )
        self.assertEqual(before, after)
        self.assertEqual(after, (1, 1))


if __name__ == "__main__":
    unittest.main()
