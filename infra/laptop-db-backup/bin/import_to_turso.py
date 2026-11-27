#!/usr/bin/env python3
"""Import a laptop-local SQLite database into its Turso backup home.

Mesh class E (#13474, design #9093 §6): a qualifying laptop-local
database gets a dedicated Turso home (`laptop-backup-<name>`) filled
by a weekly wholesale-replace import. This script is that import for
one database; the systemd wrapper (systemd/run-*.sh) supplies the
source path and home, and fetches the home's token from the vault by
name at runtime.

Command line: --source PATH --home-url URL [--home-name NAME]

Credentials come from the environment only and are never printed:
TURSO_HOME_TOKEN (the home's vault entry, e.g.
turso.laptop-backup-pm-projector-rw-token).

Import discipline (the mesh's terms, mirroring the class-D leg,
infra/fleet-queue-backup/bin/export_to_turso.py, #13458):
- The source is read through a consistent snapshot taken with the
  SQLite backup API, never by copying the live file: a raw copy of
  a live WAL database is not a backup. Every count and row in the
  run comes from that one snapshot, so the source moving mid-run
  cannot smear the copy.
- Wholesale replace, staged. The new copy is loaded into staging
  tables and verified against the snapshot before anything live
  moves, so an import that fails leaves the previous import intact.
- Counts in the import record are the rows actually loaded; a
  COUNT(*) cross-check of the staging tables that disagrees fails
  the run instead of recording a copy that never existed.
- The swap writes the incoming record as a swap marker before its
  first rename, renames the old live tables to a retiring prefix,
  renames the staging tables in, recreates indexes/views/triggers,
  verifies the live counts, and only then writes the import record
  into the home's meta table and clears the marker — record and
  marker agreeing (by import_id) is the commit point. The retiring
  tables are dropped only after the record lands; a run that finds
  retiring tables at start reads marker and record to tell a
  never-committed swap (roll back to the retiring set) from a
  committed one whose cleanup was lost (drop the retiring set), and
  reconciles before doing anything else.
- The home is a backup target, never a live read path. Foreign keys
  are not enforced during the staged load; after the swap every
  reference resolves to the new live tables by name.
"""

import argparse
import json
import os
import re
import shutil
import sqlite3
import sys
import tempfile
import uuid
from datetime import datetime, timezone

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ldb_common

HOME_META_TABLE = ldb_common.HOME_META_TABLE
STAGE_PREFIX = "_ldbstage_"
RETIRE_PREFIX = "_ldbretire_"
BATCH = 250


def _require_env(name):
    value = os.environ.get(name)
    if not value:
        raise RuntimeError(f"required environment variable {name} is unset or empty")
    return value


# ------------------------------------------------------------ home schema
def home_schema(home):
    """The home's schema as (type, name, tbl_name, sql), creation order."""
    return ldb_common.read_schema(home, HOME_META_TABLE)


def _tables(schema):
    return [(name, sql) for kind, name, _t, sql in schema if kind == "table"]


def _objects(schema):
    return [
        (kind, name, sql)
        for kind, name, _t, sql in schema
        if kind in ("index", "view", "trigger")
    ]


def _read_meta(home, key):
    _cols, rows = home.execute(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?",
        (HOME_META_TABLE,),
    )
    if not rows:
        return None
    _cols, rows = home.execute(
        f'SELECT value FROM "{HOME_META_TABLE}" WHERE key = ?', (key,)
    )
    if not rows:
        return None
    return json.loads(rows[0][0])


def read_import_record(home):
    """The committed import record from the home's meta table, or None."""
    return _read_meta(home, "import")


def _read_swap_marker(home):
    """The in-flight swap's record, written before the swap's first
    rename and deleted after the import record lands. Equality of
    the two records' import_id is the commit witness recovery uses."""
    return _read_meta(home, "swap")


def _write_meta(home, key, record):
    home.execute(
        f'CREATE TABLE IF NOT EXISTS "{HOME_META_TABLE}" '
        "(key TEXT PRIMARY KEY, value TEXT NOT NULL)"
    )
    home.execute(
        f'INSERT OR REPLACE INTO "{HOME_META_TABLE}" VALUES (?, ?)',
        (key, json.dumps(record, sort_keys=True)),
    )


def _delete_meta(home, key):
    _cols, rows = home.execute(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?",
        (HOME_META_TABLE,),
    )
    if rows:
        home.execute(f'DELETE FROM "{HOME_META_TABLE}" WHERE key = ?', (key,))


def _drop_objects(home, schema):
    for kind, name, _sql in _objects(schema):
        home.execute(f'DROP {kind.upper()} IF EXISTS "{name}"')


def _drop_tables(home, names):
    for name in names:
        home.execute(f'DROP TABLE IF EXISTS "{name}"')


# ------------------------------------------------------- interrupted swap
def recover_interrupted_swap(home):
    """Reconcile retiring tables left by a swap, per the import record.

    Retiring tables are the previous live set, renamed aside during
    a swap and dropped only after the new import record lands. The
    swap marker (the incoming record, written before the first
    rename) plus the import record decide what their presence means:
    - marker present, and the import record is not the marker's
      record: the swap never committed — roll back to the retiring
      set (see the class-D leg's recovery for the full reasoning).
    - marker absent, or the import record is the marker's record:
      the swap committed and only the retiring cleanup was lost.
      Drop the retiring tables.
    Returns True when a rollback was performed.
    """
    schema = home_schema(home)
    retiring = {
        name[len(RETIRE_PREFIX) :]: name
        for _kind, name, _t, _sql in schema
        if name.startswith(RETIRE_PREFIX)
    }
    staging = [
        name for _kind, name, _t, _sql in schema if name.startswith(STAGE_PREFIX)
    ]
    marker = _read_swap_marker(home)
    if not retiring:
        _drop_tables(home, staging)
        if marker is not None:
            _delete_meta(home, "swap")
        return False
    record = read_import_record(home)
    committed = marker is None or (
        record is not None and record["import_id"] == marker["import_id"]
    )
    if committed:
        _drop_tables(home, list(retiring.values()))
        _drop_tables(home, staging)
        if marker is not None:
            _delete_meta(home, "swap")
        return False
    incoming = set(marker["counts"])
    replaced = set(marker.get("replaces", list(retiring)))
    _drop_objects(home, schema)
    for name, _sql in _tables(schema):
        if name.startswith((STAGE_PREFIX, RETIRE_PREFIX)):
            continue
        if name in retiring or (name in incoming and name not in replaced):
            home.execute(f'DROP TABLE IF EXISTS "{name}"')
    _drop_tables(home, staging)
    for base, retired in retiring.items():
        home.execute(f'ALTER TABLE "{retired}" RENAME TO "{base}"')
    if record is not None:
        recorded = [tuple(entry) for entry in record["schema"]]
        present = {name for _kind, name, _t, _sql in home_schema(home)}
        for kind, name, _t, sql in recorded:
            if kind in ("index", "view", "trigger") and name not in present:
                home.execute(sql.rstrip().rstrip(";"))
    _delete_meta(home, "swap")
    return True


# ---------------------------------------------------------------- staging
_CREATE_TABLE_HEAD = re.compile(
    r"^(\s*CREATE\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?)"
    r"(\"[^\"\n]+\"|`[^`\n]+`|[A-Za-z_][\w$]*)",
    re.IGNORECASE,
)


def _staged_create(sql, table, stage):
    out, n = _CREATE_TABLE_HEAD.subn(lambda m: m.group(1) + f'"{stage}"', sql, count=1)
    if n != 1:
        raise RuntimeError(
            f"cannot stage {table}: CREATE head not recognized: {sql[:80]!r}"
        )
    return out


def snapshot_source(source_path, staging_dir):
    """A consistent snapshot of the source via the SQLite backup API.

    The source is opened read-only; the snapshot is a standalone
    file every later step reads, so the run's counts and rows all
    describe one point in time even while the live database moves.
    """
    snap_path = os.path.join(staging_dir, "snapshot.db")
    src = sqlite3.connect(f"file:{source_path}?mode=ro", uri=True)
    try:
        dst = sqlite3.connect(snap_path)
        try:
            src.backup(dst)
        finally:
            dst.close()
    finally:
        src.close()
    return snap_path


def snapshot_schema(snap):
    """The snapshot's schema as (type, name, tbl_name, sql) rows.

    The home's meta table name is excluded: it is written by the
    import, never copied from a source.
    """
    rows = snap.execute(
        "SELECT type, name, tbl_name, sql FROM sqlite_master "
        "WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' "
        "AND name <> ? ORDER BY rowid",
        (HOME_META_TABLE,),
    ).fetchall()
    return [tuple(r) for r in rows]


def _insert_rows(home, stage, rows):
    """Insert one batch as a single multi-row INSERT statement.

    Per-statement cost dominates on the libsql wire (measured on the
    first live run, #13474: ~54 ms per single-row INSERT statement
    through the pipeline, against ~29 rows/s end to end), so a batch
    rides one statement with one value tuple per row, never one
    statement per row.

    The batch leads with PRAGMA foreign_keys=OFF: during staging the
    parent tables exist only under their staging names, so an
    enforced foreign key would fail every child insert with "no such
    table". The pragma rides the same pipeline batch as the INSERT
    (each client request is its own connection), so enforcement is
    off only for these staging writes; the swapped-in live copy
    keeps the source's full DDL, foreign keys included, and the
    post-swap count verification still gates the record.
    """
    ncols = len(rows[0])
    one = "(" + ", ".join("?" for _ in range(ncols)) + ")"
    sql = f'INSERT INTO "{stage}" VALUES ' + ", ".join(one for _ in rows)
    args = tuple(value for row in rows for value in row)
    home.execute_many([("PRAGMA foreign_keys=OFF", ()), (sql, args)])


def _load_staging(snap, home, schema):
    """Load every snapshot table into its staging table; return the
    per-table counts of rows actually loaded, cross-checked against
    a COUNT(*) of the staging tables in the home."""
    tables = _tables(schema)
    for name, _sql in tables:
        home.execute(f'DROP TABLE IF EXISTS "{STAGE_PREFIX}{name}"')
    for name, sql in tables:
        home.execute(_staged_create(sql, name, f"{STAGE_PREFIX}{name}"))
    counts = {}
    for name, _sql in tables:
        stage = f"{STAGE_PREFIX}{name}"
        batch = []
        loaded = 0
        cursor = snap.execute(f'SELECT * FROM "{name}"')
        while True:
            rows = cursor.fetchmany(ldb_common.PAGE_ROWS)
            if not rows:
                break
            for row in rows:
                batch.append(tuple(row))
                loaded += 1
                if len(batch) >= BATCH:
                    _insert_rows(home, stage, batch)
                    batch = []
        if batch:
            _insert_rows(home, stage, batch)
        counts[name] = loaded
    staged = ldb_common.table_counts(home, home_schema(home))
    mismatched = {
        name: (counts[name], staged.get(f"{STAGE_PREFIX}{name}"))
        for name in counts
        if staged.get(f"{STAGE_PREFIX}{name}") != counts[name]
    }
    if mismatched:
        raise RuntimeError(f"staging count mismatch: {mismatched}")
    return counts


def _drop_staging(home):
    schema = home_schema(home)
    _drop_tables(
        home,
        [name for _k, name, _t, _s in schema if name.startswith(STAGE_PREFIX)],
    )


# -------------------------------------------------------------------- swap
def _swap(home, schema, counts, record):
    """Replace the home's live copy with the verified staging copy.

    The incoming record is written as the swap marker before the
    first rename and as the import record after the live counts
    verify; equality of the two (by import_id) is the commit point
    recover_interrupted_swap reads. Raises with the phase named; a
    failure here leaves the retiring tables and the marker in place
    for that recovery rather than guessing.
    """
    tables = _tables(schema)
    current = home_schema(home)
    live_tables = [
        name
        for name, _sql in _tables(current)
        if not name.startswith((STAGE_PREFIX, RETIRE_PREFIX))
    ]
    marker = dict(record)
    marker["replaces"] = live_tables
    try:
        _write_meta(home, "swap", marker)
        _drop_objects(home, current)
        for name in live_tables:
            home.execute(f'ALTER TABLE "{name}" RENAME TO "{RETIRE_PREFIX}{name}"')
        for name, _sql in tables:
            home.execute(f'ALTER TABLE "{STAGE_PREFIX}{name}" RENAME TO "{name}"')
        live = ldb_common.table_counts(home, home_schema(home))
        mismatched = {
            name: (counts[name], live.get(name))
            for name in counts
            if live.get(name) != counts[name]
        }
        if mismatched:
            raise RuntimeError(f"post-swap count mismatch: {mismatched}")
        for kind, _name, _t, sql in schema:
            if kind in ("index", "view", "trigger"):
                home.execute(sql.rstrip().rstrip(";"))
        _write_meta(home, "import", record)
        _delete_meta(home, "swap")
    except (OSError, RuntimeError, ValueError) as exc:
        raise RuntimeError(
            f"swap failed after staging verified ({exc}); the previous "
            "import is recoverable from the retiring tables on the next run"
        ) from exc
    # The import record is written: the swap has committed. Dropping
    # the retiring tables is cleanup; if it is lost, the next run's
    # recovery recognizes the post-commit state and finishes it.
    for name in live_tables:
        try:
            home.execute(f'DROP TABLE IF EXISTS "{RETIRE_PREFIX}{name}"')
        except (OSError, RuntimeError, ValueError) as exc:
            print(
                f"laptop-db-backup: note: retiring table for {name} "
                f"not dropped ({exc}); the next run cleans it up",
                file=sys.stderr,
            )


# --------------------------------------------------------------------- run
def run_import(source_path, home, home_name):
    """One full import run; returns the receipt (the import record
    plus the recovery note). Raises on any verification failure."""
    recovered = recover_interrupted_swap(home)
    staging_dir = tempfile.mkdtemp(prefix="laptop-db-backup-")
    snap_path = snapshot_source(source_path, staging_dir)
    try:
        snap = sqlite3.connect(f"file:{snap_path}?mode=ro", uri=True)
        try:
            schema = snapshot_schema(snap)
            counts = _load_staging(snap, home, schema)
        finally:
            snap.close()
    finally:
        # The snapshot inherits the source's WAL journal mode, so its
        # -wal/-shm sidecars outlive the read-only connection that
        # read it; remove the whole staging dir, never just the file
        # (first live run, #13474: rmdir failed "Directory not empty"
        # after staging verified, before the swap).
        shutil.rmtree(staging_dir, ignore_errors=True)
    record = {
        "import_id": uuid.uuid4().hex,
        "imported_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "home": home_name,
        "source": {
            "path": source_path,
            "size_bytes": os.path.getsize(source_path),
        },
        "counts": counts,
        "schema": [list(entry) for entry in schema],
    }
    _swap(home, schema, counts, record)
    receipt = dict(record)
    receipt.pop("schema", None)
    receipt["recovered_interrupted_swap"] = recovered
    return receipt


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--source", required=True, help="path of the source database")
    parser.add_argument("--home-url", required=True, help="libsql URL of the home")
    parser.add_argument("--home-name", default="", help="home name for the record")
    args = parser.parse_args(argv)
    token = _require_env("TURSO_HOME_TOKEN")
    home = ldb_common.LibsqlClient(args.home_url, token)
    receipt = run_import(args.source, home, args.home_name or args.home_url)
    print(json.dumps(receipt, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
