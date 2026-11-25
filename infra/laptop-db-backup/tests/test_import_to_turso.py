#!/usr/bin/env python3
"""Hermetic tests for import_to_turso.py (mesh class E, #13474).

sqlite3 is the oracle: the home is a FakeHome backed by an in-memory
SQLite database exposing the LibsqlClient interface, so the staged
load, the marker-witnessed swap, and the interrupted-swap recovery
all run against the real SQL semantics with no network.

Run: python3 -I tests/test_import_to_turso.py
"""

import os
import sqlite3
import sys
import tempfile

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "bin"))
import import_to_turso as imp


class FakeHome:
    """The LibsqlClient interface over a local SQLite connection."""

    def __init__(self):
        self.conn = sqlite3.connect(":memory:")

    def execute(self, sql, args=()):
        cur = self.conn.execute(sql, tuple(args))
        rows = cur.fetchall()
        cols = [d[0] for d in cur.description] if cur.description else []
        self.conn.commit()
        return cols, [tuple(r) for r in rows]

    def execute_many(self, statements):
        for sql, args in statements:
            self.conn.execute(sql, tuple(args))
        self.conn.commit()


def make_source(path, events_rows, with_extra=True):
    if os.path.exists(path):
        os.unlink(path)
    db = sqlite3.connect(path)
    # WAL like the live sources: the snapshot inherits the journal
    # mode, and its -wal/-shm sidecars must not break the run's
    # staging cleanup (the first live run failed exactly there).
    db.execute("PRAGMA journal_mode=WAL")
    db.execute("CREATE TABLE events (id INTEGER PRIMARY KEY, kind TEXT, payload BLOB, score REAL)")
    db.execute("CREATE INDEX events_kind ON events (kind)")
    db.execute("CREATE TABLE snapshots (id INTEGER PRIMARY KEY, note TEXT)")
    db.execute("CREATE VIEW event_kinds AS SELECT DISTINCT kind FROM events")
    for i in range(events_rows):
        db.execute(
            "INSERT INTO events VALUES (?, ?, ?, ?)",
            (i + 1, f"k{i % 3}", b"\x00\x01" if i % 2 else None, i * 0.5 if i % 5 else None),
        )
    if with_extra:
        db.execute("INSERT INTO snapshots VALUES (1, 'first')")
    db.commit()
    db.close()


def live_rows(home, table):
    _cols, rows = home.execute(f'SELECT * FROM "{table}" ORDER BY 1')
    return rows


def test_first_import_lands_verified_copy():
    with tempfile.TemporaryDirectory() as td:
        src = os.path.join(td, "source.db")
        make_source(src, 10)
        home = FakeHome()
        receipt = imp.run_import(src, home, "laptop-backup-test")
        assert receipt["counts"] == {"events": 10, "snapshots": 1}, receipt
        assert receipt["recovered_interrupted_swap"] is False
        rows = live_rows(home, "events")
        assert len(rows) == 10 and rows[0][1] == "k0" and rows[1][2] == b"\x00\x01"
        record = imp.read_import_record(home)
        assert record["import_id"] == receipt["import_id"]
        assert imp._read_swap_marker(home) is None
        names = {r[0] for r in home.execute(
            "SELECT name FROM sqlite_master WHERE type='table'")[1]}
        assert not any(n.startswith((imp.STAGE_PREFIX, imp.RETIRE_PREFIX)) for n in names), names


def test_second_import_wholesale_replaces():
    with tempfile.TemporaryDirectory() as td:
        src = os.path.join(td, "source.db")
        # 600 rows crosses two full insert batches plus a remainder,
        # so the multi-row INSERT batching is exercised end to end.
        make_source(src, 600)
        home = FakeHome()
        first = imp.run_import(src, home, "laptop-backup-test")
        assert first["counts"] == {"events": 600, "snapshots": 1}, first
        assert len(live_rows(home, "events")) == 600
        make_source(src, 4, with_extra=False)
        receipt = imp.run_import(src, home, "laptop-backup-test")
        assert receipt["counts"] == {"events": 4, "snapshots": 0}, receipt
        assert len(live_rows(home, "events")) == 4
        assert live_rows(home, "snapshots") == []


def test_uncommitted_swap_rolls_back():
    with tempfile.TemporaryDirectory() as td:
        src = os.path.join(td, "source.db")
        make_source(src, 3)
        home = FakeHome()
        first = imp.run_import(src, home, "laptop-backup-test")
        # Simulate a swap that renamed the live set aside, renamed a
        # partial new set in, and died before the record landed.
        home.execute('ALTER TABLE "events" RENAME TO "_ldbretire_events"')
        home.execute('CREATE TABLE "events" (id INTEGER PRIMARY KEY, kind TEXT, payload BLOB, score REAL)')
        home.execute("INSERT INTO events VALUES (99, 'partial', NULL, NULL)")
        marker = dict(first)
        marker["import_id"] = "different-import"
        marker["counts"] = {"events": 1, "snapshots": 1}
        marker["replaces"] = ["events", "snapshots"]
        imp._write_meta(home, "swap", marker)
        rolled = imp.recover_interrupted_swap(home)
        assert rolled is True
        rows = live_rows(home, "events")
        assert len(rows) == 3 and rows[0][0] == 1, rows
        assert imp._read_swap_marker(home) is None


def test_committed_swap_cleanup_is_finished_by_recovery():
    with tempfile.TemporaryDirectory() as td:
        src = os.path.join(td, "source.db")
        make_source(src, 2)
        home = FakeHome()
        record = imp.run_import(src, home, "laptop-backup-test")
        # Simulate a committed swap whose retiring cleanup was lost.
        home.execute('CREATE TABLE "_ldbretire_events" (id INTEGER PRIMARY KEY)')
        marker = dict(record)
        marker["replaces"] = ["events", "snapshots"]
        imp._write_meta(home, "swap", marker)
        rolled = imp.recover_interrupted_swap(home)
        assert rolled is False
        names = {r[0] for r in home.execute(
            "SELECT name FROM sqlite_master WHERE type='table'")[1]}
        assert "_ldbretire_events" not in names, names
        assert len(live_rows(home, "events")) == 2
        assert imp._read_swap_marker(home) is None


def main():
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    for t in tests:
        t()
        print(f"ok {t.__name__}")
    print(f"{len(tests)} tests passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
