#!/usr/bin/env python3
"""The class-D export must land a faithful, wholesale copy of the D1
database in its Turso home -- and a failed import must leave the
previous import intact.

Oracle: Python's sqlite3 module, twice over. The "D1 database" is a
sqlite3 database served through a fake D1Client whose query() answers
with JSON-shaped dicts exactly like the D1 HTTP API (including its
int64 rounding); the "Turso home" is a second sqlite3 database behind
the LibsqlClient contract. After export_to_turso.export, every home
table must equal the D1 table row for row, and the home's import
record must describe exactly that copy.

Run: python3 -I infra/fleet-queue-backup/tests/test_export_to_turso.py
"""

import json
import os
import sqlite3
import sys
import unittest

BIN = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "bin"))
sys.path.insert(0, BIN)

import export_to_turso
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


class FakeD1:
    """The D1Client surface export_to_turso uses, over a sqlite3 copy.

    query() returns a list of dicts (the D1 HTTP API's shape); blob
    cells are returned as lists of byte values, and integers beyond
    2^53 are rounded through a float, exactly as the API does.
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


class FailingD1(FakeD1):
    """Fails while reading one named table, mid-import."""

    def __init__(self, conn, fail_table):
        super().__init__(conn)
        self.fail_table = fail_table

    def query(self, db_id, sql, params=()):
        if f'FROM "{self.fail_table}"' in sql and "count(*)" not in sql:
            raise RuntimeError("simulated D1 read failure")
        return super().query(db_id, sql, params)


class SmearedD1(FakeD1):
    """count(*) disagrees with the rows actually read (source moved)."""

    def counts(self, db_id, names):
        out = super().counts(db_id, names)
        if out:
            out[names[0]] += 1
        return out


def make_d1(variant=1):
    conn = sqlite3.connect(":memory:")
    conn.executescript(
        """
        CREATE TABLE tasks (id INTEGER PRIMARY KEY, title TEXT, payload BLOB);
        CREATE TABLE notes (task_id INTEGER, note TEXT);
        CREATE TABLE seq (name TEXT PRIMARY KEY, max INTEGER);
        CREATE TABLE _cf_KV (key TEXT PRIMARY KEY, value BLOB);
        CREATE TABLE _fleet_queue_backup_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE INDEX idx_notes_task ON notes(task_id);
        CREATE VIEW task_titles AS SELECT id, title FROM tasks;
        """
    )
    if variant == 1:
        conn.executemany(
            "INSERT INTO tasks VALUES (?,?,?)",
            [(1, "one", b"\x01\x02"), (2, "tw'o", None), (3, "three", b"")],
        )
        conn.executemany("INSERT INTO notes VALUES (?,?)", [(1, "a"), (3, "b")])
    else:
        conn.executemany(
            "INSERT INTO tasks VALUES (?,?,?)",
            [(1, "one", b"\x01\x02"), (4, "four", b"\xff")],
        )
        conn.execute("DROP INDEX idx_notes_task")
        conn.execute("DROP TABLE notes")
        conn.execute("CREATE TABLE events (id INTEGER PRIMARY KEY, kind TEXT)")
        conn.executemany("INSERT INTO events VALUES (?,?)", [(1, "boot")])
    conn.executemany(
        "INSERT INTO seq VALUES (?,?)",
        [("tasks", 9223372036854775807), ("events", 21500)],
    )
    conn.execute("INSERT INTO _cf_KV VALUES ('k', X'00')")
    conn.execute(
        "INSERT INTO _fleet_queue_backup_meta VALUES ('export', ?)",
        (json.dumps({"exported_at": "2026-11-23T00:00:00Z", "counts": {}}),),
    )
    conn.commit()
    return conn


def table_rows(conn, table):
    return conn.execute(f'SELECT * FROM "{table}" ORDER BY rowid').fetchall()


def schema_names(conn, kind=None):
    sql = "SELECT name FROM sqlite_master WHERE sql IS NOT NULL"
    if kind:
        sql += f" AND type = '{kind}'"
    return {r[0] for r in conn.execute(sql)}


class ExportCase(unittest.TestCase):
    def setUp(self):
        self.d1_conn = make_d1(1)
        self.home_conn = sqlite3.connect(":memory:")
        self.home = SqliteLibsqlAdapter(self.home_conn)
        self._patch(FakeD1(self.d1_conn))
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

    def tearDown(self):
        for name, value in self._env.items():
            if value is None:
                del os.environ[name]
            else:
                os.environ[name] = value

    def _patch(self, fake_d1):
        export_to_turso.fq_common.D1Client = lambda token, account: fake_d1
        export_to_turso.fq_common.LibsqlClient = lambda url, token: self.home

    def run_export(self):
        return export_to_turso.export(
            fq_common.DEFAULT_D1_NAME, "https://home.invalid"
        )


class TestExport(ExportCase):
    def test_copies_row_for_row_and_records_import(self):
        result = self.run_export()
        self.assertEqual(result["tables"], 4)
        self.assertEqual(result["rows"], 3 + 2 + 2 + 1)
        for table in ("tasks", "notes", "seq", "_fleet_queue_backup_meta"):
            self.assertEqual(
                table_rows(self.home_conn, table), table_rows(self.d1_conn, table)
            )
        # The int64 sequence value survived exactly (CAST AS TEXT read).
        self.assertEqual(
            self.home_conn.execute(
                "SELECT max FROM seq WHERE name = 'tasks'"
            ).fetchone()[0],
            9223372036854775807,
        )
        # D1's own _cf_KV is not part of the backup.
        self.assertNotIn("_cf_KV", schema_names(self.home_conn, "table"))
        # Objects came across.
        self.assertIn("idx_notes_task", schema_names(self.home_conn, "index"))
        self.assertIn("task_titles", schema_names(self.home_conn, "view"))
        # The import record describes exactly this copy.
        record = export_to_turso.read_import_record(self.home)
        self.assertEqual(record["d1_name"], fq_common.DEFAULT_D1_NAME)
        self.assertEqual(record["counts"], result["counts"])
        self.assertEqual(record["counts"]["tasks"], 3)

    def test_second_export_replaces_wholesale(self):
        self.run_export()
        self.d1_conn = make_d1(2)
        self._patch(FakeD1(self.d1_conn))
        result = self.run_export()
        self.assertEqual(
            schema_names(self.home_conn, "table"),
            {
                "tasks",
                "seq",
                "events",
                "_fleet_queue_backup_meta",
                export_to_turso.HOME_META_TABLE,
            },
        )
        for table in ("tasks", "seq", "events"):
            self.assertEqual(
                table_rows(self.home_conn, table), table_rows(self.d1_conn, table)
            )
        record = export_to_turso.read_import_record(self.home)
        self.assertEqual(record["counts"], result["counts"])
        self.assertNotIn("notes", record["counts"])

    def test_failed_import_leaves_previous_import_intact(self):
        first = self.run_export()
        self._patch(FailingD1(self.d1_conn, "notes"))
        with self.assertRaises(RuntimeError):
            self.run_export()
        for table in ("tasks", "notes", "seq"):
            self.assertEqual(
                table_rows(self.home_conn, table), table_rows(self.d1_conn, table)
            )
        record = export_to_turso.read_import_record(self.home)
        self.assertEqual(record["imported_at"], first["imported_at"])
        self.assertEqual(record["counts"], first["counts"])
        leftovers = {
            n
            for n in schema_names(self.home_conn, "table")
            if n.startswith(
                (export_to_turso.STAGE_PREFIX, export_to_turso.RETIRE_PREFIX)
            )
        }
        self.assertEqual(leftovers, set())

    def test_source_smear_is_refused(self):
        self.run_export()
        self._patch(SmearedD1(self.d1_conn))
        with self.assertRaises(RuntimeError):
            self.run_export()
        record = export_to_turso.read_import_record(self.home)
        self.assertEqual(record["counts"]["tasks"], 3)


class TestSwapRecovery(ExportCase):
    def _crash_mid_swap(self):
        """Leave the home as a swap that died after the renames:
        old tables retiring, one partial new table live, objects
        gone, and the swap marker naming an incoming record that
        never became the import record."""
        record = export_to_turso.read_import_record(self.home)
        export_to_turso._write_meta(
            self.home,
            "swap",
            {**record, "import_id": "incoming-never-committed"},
        )
        for (name,) in self.home_conn.execute(
            "SELECT name FROM sqlite_master WHERE type = 'table' "
            "AND name NOT LIKE 'sqlite_%' AND name <> ?",
            (export_to_turso.HOME_META_TABLE,),
        ).fetchall():
            self.home_conn.execute(
                f'ALTER TABLE "{name}" RENAME TO '
                f'"{export_to_turso.RETIRE_PREFIX}{name}"'
            )
        self.home_conn.execute("DROP VIEW task_titles")
        self.home_conn.execute("DROP INDEX idx_notes_task")
        self.home_conn.execute(
            "CREATE TABLE tasks (id INTEGER PRIMARY KEY, title TEXT, payload BLOB)"
        )
        self.home_conn.execute("INSERT INTO tasks VALUES (99, 'partial', NULL)")
        self.home_conn.commit()

    def test_interrupted_swap_rolls_back_to_recorded_import(self):
        self.run_export()
        before = {
            t: table_rows(self.d1_conn, t)
            for t in ("tasks", "notes", "seq", "_fleet_queue_backup_meta")
        }
        self._crash_mid_swap()
        self.assertTrue(export_to_turso.recover_interrupted_swap(self.home))
        for table, rows in before.items():
            self.assertEqual(table_rows(self.home_conn, table), rows)
        self.assertIn("idx_notes_task", schema_names(self.home_conn, "index"))
        self.assertIn("task_titles", schema_names(self.home_conn, "view"))
        # And the next export proceeds normally from the recovered state.
        result = self.run_export()
        self.assertEqual(result["rows"], 8)

    def test_early_crash_keeps_untouched_live_tables(self):
        """A swap that died during the rename-aside phase: one old
        table retiring, the rest still live and untouched, one
        incoming-only table already renamed in. Recovery must keep
        the untouched old tables, drop the incoming-only one, and
        rename the retiring table back."""
        self.run_export()
        before = {
            t: table_rows(self.d1_conn, t)
            for t in ("tasks", "notes", "seq", "_fleet_queue_backup_meta")
        }
        record = export_to_turso.read_import_record(self.home)
        export_to_turso._write_meta(
            self.home,
            "swap",
            {
                **record,
                "import_id": "incoming-never-committed",
                "counts": {**record["counts"], "events": 1},
                "replaces": ["tasks", "notes", "seq", "_fleet_queue_backup_meta"],
            },
        )
        self.home_conn.execute(
            f'ALTER TABLE "seq" RENAME TO "{export_to_turso.RETIRE_PREFIX}seq"'
        )
        self.home_conn.execute(
            "CREATE TABLE events (id INTEGER PRIMARY KEY, kind TEXT)"
        )
        self.home_conn.execute("INSERT INTO events VALUES (1, 'partial')")
        self.home_conn.commit()
        self.assertTrue(export_to_turso.recover_interrupted_swap(self.home))
        for table, rows in before.items():
            self.assertEqual(table_rows(self.home_conn, table), rows)
        self.assertNotIn("events", schema_names(self.home_conn, "table"))

    def test_committed_swap_leftover_retiring_is_cleaned_not_rolled_back(self):
        self.run_export()
        self.d1_conn = make_d1(2)
        self._patch(FakeD1(self.d1_conn))
        self.run_export()
        # Simulate a committed swap whose retiring cleanup was lost:
        # a stale retiring table beside the new live copy, plus the
        # swap marker still naming the record that did commit.
        self.home_conn.execute(
            f'CREATE TABLE "{export_to_turso.RETIRE_PREFIX}tasks" '
            "(id INTEGER PRIMARY KEY, title TEXT, payload BLOB)"
        )
        self.home_conn.commit()
        record = export_to_turso.read_import_record(self.home)
        export_to_turso._write_meta(self.home, "swap", record)
        self.assertFalse(export_to_turso.recover_interrupted_swap(self.home))
        self.assertIsNone(export_to_turso._read_swap_marker(self.home))
        self.assertNotIn(
            f"{export_to_turso.RETIRE_PREFIX}tasks",
            schema_names(self.home_conn, "table"),
        )
        self.assertEqual(
            table_rows(self.home_conn, "tasks"), table_rows(self.d1_conn, "tasks")
        )


if __name__ == "__main__":
    unittest.main()
