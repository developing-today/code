#!/usr/bin/env python3
"""CF-3 (#14943): the D1 -> D1 backup leg and its restore drill.

Oracle: real SQLite. The fake D1Client below is backed by in-memory
SQLite databases, so the D1 query API's SQL behaviour (sqlite_master
reads, quote() literal rendering, paged row reads) is exercised by
the same engine D1 runs, and the import is a real executescript of
the generated dump. What is faked is only the HTTP transport and the
database registry.

Run: python3 -I tests/test_d1_plane.py
"""

import os
import sqlite3
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "bin"))

import fq_common
import export_d1_to_d1
import restore_drill_d1


class FakeD1:
    """The slice of fq_common.D1Client the CF-3 tools use, over SQLite."""

    def __init__(self):
        self.dbs = {}  # uuid -> {"name": str, "conn": sqlite3.Connection}
        self._seq = 0

    def _register(self, name):
        self._seq += 1
        uuid = f"fake-{self._seq}"
        self.dbs[uuid] = {"name": name, "conn": sqlite3.connect(":memory:")}
        return {"name": name, "uuid": uuid}

    def find_database(self, name):
        for uuid, db in self.dbs.items():
            if db["name"] == name:
                return {"name": name, "uuid": uuid}
        return None

    def create_database(self, name):
        return self._register(name)

    def delete_database(self, db_id):
        self.dbs.pop(db_id, None)
        return {}

    def query(self, db_id, sql, params=()):
        cur = self.dbs[db_id]["conn"].execute(sql, list(params))
        cols = [d[0] for d in cur.description or []]
        return [dict(zip(cols, row)) for row in cur.fetchall()]

    def import_sql(self, db_id, sql_text):
        conn = self.dbs[db_id]["conn"]
        conn.executescript(sql_text)
        conn.commit()
        return {"status": "complete"}

    def read_meta(self, db_id):
        rows = self.query(
            db_id,
            f'SELECT value FROM "{fq_common.META_TABLE}" WHERE key = ?',
            ("export",),
        )
        if not rows:
            return None
        import json

        return json.loads(rows[0]["value"])

    def counts(self, db_id, names):
        out = {}
        for name in names:
            rows = self.query(db_id, f'SELECT count(*) AS n FROM "{name}"')
            out[name] = rows[0]["n"]
        return out


def _seed_live_plane(fake):
    db = fake.create_database(fq_common.LIVE_D1_NAME)
    conn = fake.dbs[db["uuid"]]["conn"]
    conn.executescript(
        """
        CREATE TABLE tasks (id INTEGER PRIMARY KEY, kind TEXT, payload TEXT);
        CREATE TABLE task_events (id INTEGER PRIMARY KEY, task_id INTEGER, note TEXT);
        CREATE INDEX idx_events_task ON task_events(task_id);
        INSERT INTO tasks VALUES (1, 'claim', 'it''s a payload'), (2, 'complete', NULL), (3, 'checkpoint', 'x');
        INSERT INTO task_events VALUES (1, 1, 'claimed'), (2, 1, 'done');
        """
    )
    conn.commit()
    return db


class D1PlaneTests(unittest.TestCase):
    def setUp(self):
        self.fake = FakeD1()
        _seed_live_plane(self.fake)

    def test_export_copies_live_d1_into_backup_d1(self):
        result = export_d1_to_d1.export_with_clients(
            self.fake, fq_common.LIVE_D1_NAME, fq_common.DEFAULT_D1_NAME, create=True
        )
        self.assertEqual(result["counts"], {"tasks": 3, "task_events": 2})
        backup = self.fake.find_database(fq_common.DEFAULT_D1_NAME)
        rows = self.fake.query(backup["uuid"], "SELECT payload FROM tasks WHERE id = 1")
        self.assertEqual(rows[0]["payload"], "it's a payload")

    def test_export_refuses_live_plane_as_its_own_target(self):
        with self.assertRaises(RuntimeError):
            export_d1_to_d1.export_with_clients(
                self.fake, fq_common.LIVE_D1_NAME, fq_common.LIVE_D1_NAME, create=True
            )

    def test_restore_drill_rebuilds_backup_into_scratch(self):
        export_d1_to_d1.export_with_clients(
            self.fake, fq_common.LIVE_D1_NAME, fq_common.DEFAULT_D1_NAME, create=True
        )
        result = restore_drill_d1.drill_with_clients(
            self.fake,
            fq_common.DEFAULT_D1_NAME,
            "fleet-queue-restore-drill",
            create_scratch=True,
            drop_scratch=True,
        )
        self.assertEqual(result["counts"], {"tasks": 3, "task_events": 2})
        self.assertTrue(result["dropped"])
        self.assertIsNone(self.fake.find_database("fleet-queue-restore-drill"))

    def test_restore_drill_refuses_backup_as_its_own_scratch(self):
        with self.assertRaises(RuntimeError):
            restore_drill_d1.drill_with_clients(
                self.fake,
                fq_common.DEFAULT_D1_NAME,
                fq_common.DEFAULT_D1_NAME,
                create_scratch=True,
                drop_scratch=False,
            )

    def test_export_skips_turso_internal_bookkeeping(self):
        # Turso-internal tables ride along when the plane is seeded
        # from Turso; they are not queue user data and must never
        # enter the dump, the meta record, or the backup copy (#14943).
        live = self.fake.find_database(fq_common.LIVE_D1_NAME)
        conn = self.fake.dbs[live["uuid"]]["conn"]
        conn.execute(
            "CREATE TABLE __turso_internal_seq_tasks (seq INTEGER)"
        )
        conn.execute("INSERT INTO __turso_internal_seq_tasks VALUES (3)")
        conn.commit()
        result = export_d1_to_d1.export_with_clients(
            self.fake, fq_common.LIVE_D1_NAME, fq_common.DEFAULT_D1_NAME, create=True
        )
        self.assertEqual(result["counts"], {"tasks": 3, "task_events": 2})
        backup = self.fake.find_database(fq_common.DEFAULT_D1_NAME)
        rows = self.fake.query(
            backup["uuid"], "SELECT name FROM sqlite_master WHERE type = 'table'"
        )
        names = [row["name"] for row in rows]
        self.assertNotIn("__turso_internal_seq_tasks", names)

    def test_restore_drill_ignores_post_export_internal_tables(self):
        # Live failure signature, #14943 (2026-11-27): the backup gains
        # two one-row __turso_internal_seq_* tables after the export
        # (import side-effect / stale leftover), so a re-read
        # enumerates 34 tables against a 32-table export record while
        # every shared table matches exactly. The drill must verify
        # the faithful copy, not refuse it as smeared.
        export_d1_to_d1.export_with_clients(
            self.fake, fq_common.LIVE_D1_NAME, fq_common.DEFAULT_D1_NAME, create=True
        )
        backup = self.fake.find_database(fq_common.DEFAULT_D1_NAME)
        conn = self.fake.dbs[backup["uuid"]]["conn"]
        for table in (
            "__turso_internal_seq___turso_internal_autoincrement_github_issue_history",
            "__turso_internal_seq___turso_internal_autoincrement_github_label_history",
        ):
            conn.execute(f'CREATE TABLE "{table}" (seq INTEGER)')
            conn.execute(f'INSERT INTO "{table}" VALUES (1)')
        conn.commit()
        result = restore_drill_d1.drill_with_clients(
            self.fake,
            fq_common.DEFAULT_D1_NAME,
            "fleet-queue-restore-drill",
            create_scratch=True,
            drop_scratch=True,
        )
        self.assertEqual(result["counts"], {"tasks": 3, "task_events": 2})
        self.assertTrue(result["dropped"])
        self.assertIsNone(self.fake.find_database("fleet-queue-restore-drill"))


if __name__ == "__main__":
    unittest.main()
