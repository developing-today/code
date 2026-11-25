#!/usr/bin/env python3
"""Hermetic tests for restore_drill.py (mesh class E, #13474).

The drill's oracle is the import record: a home filled by
import_to_turso (against the sqlite-backed FakeHome) must rebuild
into a scratch file whose per-table counts match the record, and
whose rows match the source.

Run: python3 -I tests/test_restore_drill.py
"""

import os
import sqlite3
import sys
import tempfile

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "bin"))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import import_to_turso as imp
import restore_drill
from test_import_to_turso import FakeHome, make_source


def test_drill_rebuilds_and_verifies():
    with tempfile.TemporaryDirectory() as td:
        src = os.path.join(td, "source.db")
        make_source(src, 7)
        home = FakeHome()
        receipt = imp.run_import(src, home, "laptop-backup-test")
        out = os.path.join(td, "restored.db")
        drill = restore_drill.run_drill(home, out)
        assert drill["verified"] is True
        assert drill["import_id"] == receipt["import_id"]
        assert drill["counts"] == {"events": 7, "snapshots": 1}, drill
        db = sqlite3.connect(out)
        rows = db.execute("SELECT * FROM events ORDER BY id").fetchall()
        assert len(rows) == 7 and rows[1][2] == b"\x00\x01"
        kinds = db.execute("SELECT * FROM event_kinds ORDER BY 1").fetchall()
        assert kinds == [("k0",), ("k1",), ("k2",)], kinds
        db.close()


def test_drill_without_import_record_fails():
    home = FakeHome()
    with tempfile.TemporaryDirectory() as td:
        try:
            restore_drill.run_drill(home, os.path.join(td, "x.db"))
        except RuntimeError as exc:
            assert "no import record" in str(exc)
        else:
            raise AssertionError("drill should fail without an import record")


def main():
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    for t in tests:
        t()
        print(f"ok {t.__name__}")
    print(f"{len(tests)} tests passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
