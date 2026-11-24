#!/usr/bin/env python3
"""Tests for scripts/turso-mesh-class-a.py — #13455 slice 1.

Planner + export-manifest builder for the backup-mesh class-A leg
(design doc/2026-11-23T16-34-18Z_design_backup_mesh/note.md §§2-4, 9).

Oracles:
- Planner (property, stdlib random with fixed seeds — the repo's
  Python tooling is stdlib-only, like test_repair_dump_property.py):
  for ANY name set, assignments are injective (distinct weekdays),
  total (every name has a weekday), and identical under every input
  permutation. A deterministic assignment is its own oracle: two
  calls with the same set must agree byte-for-byte.
- Manifest (property): for ANY generated SQLite database (random
  tables, random row contents, random insertion order), the
  manifest's per-table row counts equal a recount through a fresh
  connection (the SQL count(*) oracle), and the content hash is
  invariant to physical row insertion order — rebuild the same
  logical table with rows inserted in a different order and the
  hash must not change. Example tests pin the three class-A
  databases through the plan CLI and a small fixture dump through
  the manifest builder and CLI.

Run:  python3 -m pytest pkgs/cold-store-reaper/tests/test_turso_mesh_class_a.py
  or: python3 -m unittest pkgs.cold-store-reaper.tests.test_turso_mesh_class_a
"""

import hashlib
import importlib.util
import json
import os
import random
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

PKG = Path(__file__).resolve().parent.parent
SCRIPT = PKG / "scripts" / "turso-mesh-class-a.py"

sys.path.insert(0, str(PKG / "tests"))
spec = importlib.util.spec_from_file_location("turso_mesh_class_a", SCRIPT)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)

WEEKDAYS = set(mod.WEEKDAYS)


# ---------------------------------------------------------------------------
# Planner
# ---------------------------------------------------------------------------

class PlannerProperties(unittest.TestCase):
    def run_case(self, names):
        plan = mod.plan_class_a_week(names)
        self.assertEqual(len(plan), len(names))
        self.assertEqual(set(plan), set(names))
        self.assertEqual(len(set(plan.values())), len(names))
        self.assertTrue(set(plan.values()) <= WEEKDAYS)

    def test_default_class_a_set_is_distinct_and_weekly(self):
        plan = mod.plan_class_a_week(list(mod.CLASS_A_DATABASES))
        self.assertEqual(len(plan), 3)
        self.assertEqual(len(set(plan.values())), 3)
        self.assertTrue(set(plan.values()) <= WEEKDAYS)

    def test_assignments_are_stable_and_permutation_invariant(self):
        rng = random.Random(20261123)
        for case in range(120):
            size = rng.randint(1, 7)
            names = ["db-%d-%d" % (case, i) for i in range(size)]
            rng.shuffle(names)
            self.run_case(names)
            reference = mod.plan_class_a_week(names)
            for _ in range(5):
                shuffled = list(names)
                rng.shuffle(shuffled)
                self.assertEqual(mod.plan_class_a_week(shuffled), reference)

    def test_same_inputs_same_plan(self):
        for _ in range(50):
            names = ["fleet-queue", "fleet-platform", "atlas", "x-%d" % _]
            self.assertEqual(
                mod.plan_class_a_week(names), mod.plan_class_a_week(list(names))
            )

    def test_plan_covers_all_seven_days_at_capacity(self):
        names = ["cap-%d" % i for i in range(7)]
        plan = mod.plan_class_a_week(names)
        self.assertEqual(set(plan.values()), WEEKDAYS)

    def test_duplicates_are_refused(self):
        with self.assertRaises(ValueError):
            mod.plan_class_a_week(["atlas", "atlas"])

    def test_more_than_seven_is_refused(self):
        with self.assertRaises(ValueError):
            mod.plan_class_a_week(["d%d" % i for i in range(8)])


# ---------------------------------------------------------------------------
# Manifest — generated-database properties
# ---------------------------------------------------------------------------

def create_database(path, tables, rng, insertion_order=True):
    """Create a SQLite database from a generated schema+content spec.

    tables: list of (name, columns, rows) where rows is a list of
    tuples. When insertion_order is False the rows of every table are
    inserted in a shuffled order (the logical content is unchanged).
    """
    if os.path.exists(path):
        os.unlink(path)
    conn = sqlite3.connect(path)
    try:
        for name, columns, rows in tables:
            conn.execute("CREATE TABLE %s (%s)" % (name, ", ".join(columns)))
            to_insert = list(rows)
            if not insertion_order:
                rng.shuffle(to_insert)
            placeholders = ", ".join("?" * 1 for _ in columns)
            for row in to_insert:
                conn.execute(
                    "INSERT INTO %s (%s) VALUES (%s)"
                    % (name, ", ".join(c.split()[0] for c in columns), placeholders),
                    row,
                )
        conn.commit()
    finally:
        conn.close()


def generated_tables(rng):
    tables = []
    for t in range(rng.randint(1, 4)):
        ncols = rng.randint(1, 3)
        col_types = ["TEXT"] * ncols
        for i in rng.sample(range(ncols), rng.randint(0, ncols)):
            col_types[i] = "INTEGER"
        columns = ["c%d %s" % (i, col_types[i]) for i in range(ncols)]
        rows = []
        for _ in range(rng.randint(0, 12)):
            row = []
            for c in columns:
                if c.endswith("INTEGER"):
                    row.append(rng.randint(-1000, 1000))
                else:
                    row.append("v%d" % rng.randint(0, 999))
            rows.append(tuple(row))
        tables.append(("tbl%d" % t, columns, rows))
    return tables


class ManifestProperties(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mesh-a-test-")
        self.addCleanup(_rmtree_silent, self.tmp)
        self.rng = random.Random(987654321)

    def test_row_counts_match_sql_recount(self):
        for case in range(40):
            tables = generated_tables(self.rng)
            path = os.path.join(self.tmp, "gen-%d.sqlite" % case)
            create_database(path, tables, self.rng)
            dump = b"BEGIN;\n" + b"generated-%d\n" % case + b"COMMIT;\n"
            manifest = mod.build_manifest("db-%d" % case, path, dump, "2026-11-23T16:00:00Z")
            self.assertEqual(manifest["database"], "db-%d" % case)
            self.assertEqual(manifest["exported_at"], "2026-11-23T16:00:00Z")
            self.assertEqual(
                manifest["dump_sha256"], hashlib.sha256(dump).hexdigest()
            )
            conn = sqlite3.connect(path)
            try:
                by_name = {e["name"]: e for e in manifest["tables"]}
                self.assertEqual(len(by_name), len(manifest["tables"]))
                for name, _columns, rows in tables:
                    self.assertIn(name, by_name)
                    self.assertEqual(by_name[name]["rows"], len(rows))
                    recount = conn.execute(
                        "SELECT count(*) FROM %s" % name
                    ).fetchone()[0]
                    self.assertEqual(by_name[name]["rows"], recount)
                sqlite_tables = {
                    r[0]
                    for r in conn.execute(
                        "SELECT name FROM sqlite_master WHERE type = 'table'"
                    )
                }
                for entry in manifest["tables"]:
                    self.assertIn(entry["name"], sqlite_tables)
            finally:
                conn.close()

    def test_content_hash_is_insertion_order_invariant(self):
        for case in range(40):
            tables = generated_tables(self.rng)
            path_a = os.path.join(self.tmp, "ord-a-%d.sqlite" % case)
            path_b = os.path.join(self.tmp, "ord-b-%d.sqlite" % case)
            create_database(path_a, tables, self.rng, insertion_order=True)
            create_database(path_b, tables, self.rng, insertion_order=False)
            dump_a = b"dump-a-%d" % case
            dump_b = b"dump-b-%d" % case
            manifest_a = mod.build_manifest("x", path_a, dump_a, "t")
            manifest_b = mod.build_manifest("x", path_b, dump_b, "t")
            entries_a = {e["name"]: e for e in manifest_a["tables"]}
            entries_b = {e["name"]: e for e in manifest_b["tables"]}
            self.assertEqual(set(entries_a), set(entries_b))
            for name, entry in entries_a.items():
                self.assertEqual(entry["rows"], entries_b[name]["rows"])
                self.assertEqual(entry["sha256"], entries_b[name]["sha256"])
            # The dump hash is content-derived too: same bytes, same hash.
            self.assertNotEqual(manifest_a["dump_sha256"], manifest_b["dump_sha256"])

    def test_content_hash_separates_different_content(self):
        rng = random.Random(42)
        tables_a = [("t", ["c TEXT"], [("a",), ("b",)])]
        tables_b = [("t", ["c TEXT"], [("a",), ("c",)])]
        path_a = os.path.join(self.tmp, "sep-a.sqlite")
        path_b = os.path.join(self.tmp, "sep-b.sqlite")
        create_database(path_a, tables_a, rng)
        create_database(path_b, tables_b, rng)
        manifest_a = mod.build_manifest("x", path_a, b"d", "t")
        manifest_b = mod.build_manifest("x", path_b, b"d", "t")
        self.assertNotEqual(
            manifest_a["tables"][0]["sha256"], manifest_b["tables"][0]["sha256"]
        )

    def test_internal_sqlite_tables_are_excluded(self):
        path = os.path.join(self.tmp, "internal.sqlite")
        if os.path.exists(path):
            os.unlink(path)
        conn = sqlite3.connect(path)
        try:
            conn.execute("CREATE TABLE keepme (x INTEGER)")
            conn.execute("CREATE INDEX keepme_idx ON keepme(x)")
            conn.execute("INSERT INTO keepme VALUES (1)")
            conn.commit()
        finally:
            conn.close()
        manifest = mod.build_manifest("x", path, b"d", "t")
        self.assertEqual([e["name"] for e in manifest["tables"]], ["keepme"])

    def test_manifest_lists_tables_sorted_by_name(self):
        path = os.path.join(self.tmp, "sorted.sqlite")
        if os.path.exists(path):
            os.unlink(path)
        conn = sqlite3.connect(path)
        try:
            for name in ("zeta", "alpha", "mid"):
                conn.execute("CREATE TABLE %s (x INTEGER)" % name)
            conn.commit()
        finally:
            conn.close()
        manifest = mod.build_manifest("x", path, b"d", "t")
        self.assertEqual(
            [e["name"] for e in manifest["tables"]], ["alpha", "mid", "zeta"]
        )


# ---------------------------------------------------------------------------
# CLI example tests
# ---------------------------------------------------------------------------

class CliExamples(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mesh-a-cli-")
        self.addCleanup(_rmtree_silent, self.tmp)

    def run_cli(self, *argv):
        return subprocess.run(
            [sys.executable, "-I", str(SCRIPT), *argv],
            capture_output=True,
            text=True,
        )

    def test_default_plan_cli_names_three_class_a_databases(self):
        result = self.run_cli("plan")
        self.assertEqual(result.returncode, 0, result.stderr)
        plan = json.loads(result.stdout)
        self.assertEqual(set(plan), set(mod.CLASS_A_DATABASES))
        self.assertEqual(len(set(plan.values())), 3)

    def test_plan_cli_refuses_duplicates_with_usage_error(self):
        result = self.run_cli("plan", "--db", "atlas", "--db", "atlas")
        self.assertEqual(result.returncode, 2)
        self.assertIn("duplicate", result.stderr)

    def test_manifest_cli_fixture_dump(self):
        sqlite_path = os.path.join(self.tmp, "fixture.sqlite")
        conn = sqlite3.connect(sqlite_path)
        try:
            conn.execute("CREATE TABLE t (x INTEGER, y TEXT)")
            conn.executemany(
                "INSERT INTO t VALUES (?, ?)", [(1, "a"), (2, "b"), (3, "c")]
            )
            conn.execute("CREATE TABLE empty (z TEXT)")
            conn.commit()
        finally:
            conn.close()
        dump_path = os.path.join(self.tmp, "fixture.dump")
        dump_bytes = b"BEGIN;\nINSERT INTO t VALUES (1, 'a');\nCOMMIT;\n"
        with open(dump_path, "wb") as fh:
            fh.write(dump_bytes)
        out_path = os.path.join(self.tmp, "manifest.json")
        result = self.run_cli(
            "manifest",
            "--db",
            "fleet-queue",
            "--sqlite",
            sqlite_path,
            "--dump",
            dump_path,
            "--exported-at",
            "2026-11-23T16:00:00Z",
            "--out",
            out_path,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        with open(out_path, encoding="utf-8") as fh:
            manifest = json.load(fh)
        self.assertEqual(manifest["database"], "fleet-queue")
        self.assertEqual(manifest["exported_at"], "2026-11-23T16:00:00Z")
        self.assertEqual(manifest["dump_sha256"], hashlib.sha256(dump_bytes).hexdigest())
        by_name = {e["name"]: e for e in manifest["tables"]}
        self.assertEqual(by_name["t"]["rows"], 3)
        self.assertEqual(by_name["empty"]["rows"], 0)
        self.assertEqual(len(by_name), 2)
        # stdout stays clean when --out is used
        self.assertEqual(result.stdout, "")

    def test_manifest_cli_rebuild_round_trip_from_fixture_dump(self):
        """The design's restore shape: the dump is applied to a fresh
        SQLite file, and the manifest built over that rebuild reports
        exactly what the dump carries."""
        dump_path = os.path.join(self.tmp, "round.dump")
        dump_bytes = (
            b"BEGIN TRANSACTION;\n"
            b"CREATE TABLE t (x INTEGER);\n"
            b"INSERT INTO t VALUES (1);\n"
            b"INSERT INTO t VALUES (2);\n"
            b"COMMIT;\n"
        )
        with open(dump_path, "wb") as fh:
            fh.write(dump_bytes)
        rebuilt = os.path.join(self.tmp, "rebuilt.sqlite")
        conn = sqlite3.connect(rebuilt)
        try:
            conn.executescript(dump_bytes.decode("utf-8"))
        finally:
            conn.close()
        result = self.run_cli(
            "manifest",
            "--db",
            "atlas",
            "--sqlite",
            rebuilt,
            "--dump",
            dump_path,
            "--exported-at",
            "2026-11-23T16:30:00Z",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        manifest = json.loads(result.stdout)
        self.assertEqual(manifest["database"], "atlas")
        self.assertEqual(manifest["tables"][0]["name"], "t")
        self.assertEqual(manifest["tables"][0]["rows"], 2)


def _rmtree_silent(path):
    import shutil

    shutil.rmtree(path, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()
