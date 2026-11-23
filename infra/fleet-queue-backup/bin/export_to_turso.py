#!/usr/bin/env python3
"""Export a D1 database into its Turso backup home (#13458, mesh class D).

Command line: [--d1-name NAME] [--home-url URL]

The backup mesh (#9093, design doc/2026-11-23T16-34-18Z_design_backup_mesh)
gives every in-scope D1 database a dedicated Turso home
(`d1-backup-<name>`) so the D1 side gets Turso PITR plus the mesh's
restic coverage. This is the reverse direction of export_to_d1.py:
that leg protects the Turso queue store with a D1 copy; this leg
protects the D1 copy itself. The #8584 export timer is untouched.

Credentials come from the environment only and are never printed:
CLOUDFLARE_API_TOKEN (cloudflare.api-token-account-wide),
CLOUDFLARE_ACCOUNT_ID, and TURSO_HOME_TOKEN (the home's vault entry,
turso.d1-backup-fleet-queue-backup-rw-token). The systemd wrapper
fetches them by name at runtime (systemd/run-d1-home-export.sh).

Import discipline (the issue's terms):
- Wholesale replace, staged. The new copy is loaded into staging
  tables and verified against the source before anything live moves,
  so an export whose import fails leaves the previous import intact.
- Counts in the import record are the rows actually loaded, never a
  separate COUNT(*) pass (the #8584 smear lesson); a COUNT(*) cross-
  check that disagrees means the source moved mid-run and the run
  fails instead of recording a copy that never existed.
- The swap writes the incoming record as a swap marker before its
  first rename, renames the old live tables to a retiring prefix,
  renames the staging tables in, recreates indexes/views/triggers,
  verifies the live counts, and only then writes the import record
  into the home's meta table and clears the marker -- record and
  marker agreeing (by import_id) is the commit point. The retiring
  tables are dropped only after the record lands; a run that finds
  retiring tables at start reads marker and record to tell a
  never-committed swap (roll back to the retiring set) from a
  committed one whose cleanup was lost (drop the retiring set), and
  reconciles before doing anything else.
- The home is a backup target, never a live read path. Foreign keys
  are not enforced during the staged load; after the swap every
  reference resolves to the new live tables by name.
- The read path is restore_drill's proven D1 reader (keyset paging,
  CAST AS TEXT for integer columns, _cf_KV excluded): the same code
  that rebuilt a Turso store from this D1 copy in the #8584 drill.
"""

import argparse
import json
import os
import re
import sys
import uuid
from datetime import datetime, timezone

import fq_common
import restore_drill

HOME_META_TABLE = "_d1_backup_home_meta"
STAGE_PREFIX = "_d1stage_"
RETIRE_PREFIX = "_d1retire_"
DEFAULT_HOME_NAME = "d1-backup-fleet-queue-backup"
DEFAULT_HOME_URL = (
    "https://d1-backup-fleet-queue-backup-dezren39.aws-us-east-1.turso.io"
)
HOME_TOKEN_SECRET_NAME = "turso.d1-backup-fleet-queue-backup-rw-token"

# D1 keeps its own _cf_KV table in every database; it is not part of
# the backup (the query API prohibits reading it, #8584).
EXCLUDED_SOURCE_TABLES = {"_cf_KV"}


def _require_env(name):
    value = os.environ.get(name)
    if not value:
        raise RuntimeError(f"required environment variable {name} is unset or empty")
    return value


# ------------------------------------------------------------ home schema
def home_schema(home):
    """The home's schema as (type, name, tbl_name, sql), creation order.

    Like fq_common.read_schema but the exclusion is this leg's own
    meta table: the D1 source's _fleet_queue_backup_meta is ordinary
    copied data here and must stay visible.
    """
    _cols, rows = home.execute(
        "SELECT type, name, tbl_name, sql FROM sqlite_master "
        "WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' "
        "AND name <> ? ORDER BY rowid",
        (HOME_META_TABLE,),
    )
    return rows


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
      record: the swap never committed -- the live tables are a
      partial new copy. Drop the whole live set and the staging
      leftovers, rename the retiring set back, and recreate the
      recorded schema's objects (indexes were dropped before the
      renames; they and the views/triggers come back from the
      record; with no record -- a first import that never committed
      -- the tables come back and the objects are rebuilt by the
      next successful export).
    - marker absent, or the import record is the marker's record:
      the swap committed and only the retiring cleanup was lost.
      Drop the retiring tables.
    Returns True when a rollback was performed.
    """
    schema = home_schema(home)
    retiring = {
        name[len(RETIRE_PREFIX):]: name
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
    live = {name for name, _sql in _tables(schema)}
    _drop_objects(home, schema)
    _drop_tables(home, list(live))
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
    r"(\"[^\"]+\"|[A-Za-z_][\w$]*)",
    re.IGNORECASE,
)


def _staged_create(sql, table, stage):
    out, n = _CREATE_TABLE_HEAD.subn(
        lambda m: m.group(1) + f'"{stage}"', sql, count=1
    )
    if n != 1:
        raise RuntimeError(f"cannot stage {table}: CREATE head not recognized: {sql[:80]!r}")
    return out


def _load_staging(d1, db_id, home, schema):
    """Load every source table into its staging table; return the
    per-table counts of rows actually loaded."""
    tables = _tables(schema)
    for name, _sql in tables:
        home.execute(f'DROP TABLE IF EXISTS "{STAGE_PREFIX}{name}"')
    for name, sql in tables:
        home.execute(_staged_create(sql, name, f"{STAGE_PREFIX}{name}"))
    counts = {}
    for name, sql in tables:
        stage = f"{STAGE_PREFIX}{name}"
        batch = []
        placeholders = None
        loaded = 0
        for row in restore_drill.d1_read_table(d1, db_id, name, sql):
            if placeholders is None:
                placeholders = ", ".join("?" for _ in row)
            batch.append((f'INSERT INTO "{stage}" VALUES ({placeholders})', row))
            loaded += 1
            if len(batch) >= restore_drill.BATCH:
                home.execute_many(batch)
                batch = []
        if batch:
            home.execute_many(batch)
        counts[name] = loaded
    staged = fq_common.table_counts(home, home_schema(home))
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
    try:
        _write_meta(home, "swap", record)
        _drop_objects(home, current)
        for name in live_tables:
            home.execute(
                f'ALTER TABLE "{name}" RENAME TO "{RETIRE_PREFIX}{name}"'
            )
        for name, _sql in tables:
            home.execute(
                f'ALTER TABLE "{STAGE_PREFIX}{name}" RENAME TO "{name}"'
            )
        live = fq_common.table_counts(home, home_schema(home))
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
                f"fleet-queue-backup: note: retiring table for {name} "
                f"not dropped ({exc}); the next run cleans it up",
                file=sys.stderr,
            )


# ------------------------------------------------------------------ export
def export(d1_name, home_url):
    d1 = fq_common.D1Client(
        _require_env("CLOUDFLARE_API_TOKEN"), _require_env("CLOUDFLARE_ACCOUNT_ID")
    )
    db = d1.find_database(d1_name)
    if db is None:
        raise RuntimeError(f"D1 database {d1_name!r} does not exist")
    db_id = db["uuid"]
    schema = [
        tuple(r[k] for k in ("type", "name", "tbl_name", "sql"))
        for r in restore_drill.d1_schema(d1, db_id)
        if r["name"] not in EXCLUDED_SOURCE_TABLES
        and r["tbl_name"] not in EXCLUDED_SOURCE_TABLES
    ]
    if not _tables(schema):
        raise RuntimeError(f"D1 database {d1_name!r} has no tables to export")

    home = fq_common.LibsqlClient(home_url, _require_env("TURSO_HOME_TOKEN"))
    recover_interrupted_swap(home)
    try:
        counts = _load_staging(d1, db_id, home, schema)
    except Exception:
        _drop_staging(home)
        raise
    source_counts = d1.counts(db_id, [name for name, _sql in _tables(schema)])
    smeared = {
        name: (counts[name], source_counts.get(name))
        for name in counts
        if source_counts.get(name) != counts[name]
    }
    if smeared:
        _drop_staging(home)
        raise RuntimeError(
            f"source moved during the export (loaded vs counted): {smeared}; "
            "the previous import is intact -- rerun"
        )
    imported_at = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    record = {
        "import_id": uuid.uuid4().hex,
        "d1_name": d1_name,
        "d1_uuid": db_id,
        "imported_at": imported_at,
        "counts": counts,
        "schema": [list(entry) for entry in schema],
    }
    _swap(home, schema, counts, record)
    return {
        "d1_name": d1_name,
        "d1_uuid": db_id,
        "home_url": home_url,
        "imported_at": imported_at,
        "tables": len(counts),
        "rows": sum(counts.values()),
        "counts": counts,
    }


def _parser():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument(
        "--d1-name",
        default=fq_common.DEFAULT_D1_NAME,
        help="D1 database to export (default: %(default)s)",
    )
    parser.add_argument(
        "--home-url",
        default=DEFAULT_HOME_URL,
        help="libsql HTTP URL of the Turso home (default: %(default)s)",
    )
    return parser


def main(argv=None):
    args = _parser().parse_args(argv)
    try:
        result = export(args.d1_name, args.home_url)
    except (OSError, RuntimeError, ValueError) as exc:
        print(f"fleet-queue-backup: D1 home export failed: {exc}", file=sys.stderr)
        return 1
    print(
        "fleet-queue-backup: imported {tables} tables / {rows} rows from D1 "
        "{d1_name} ({d1_uuid}) into Turso home {home_url} at {imported_at}; "
        "read-back counts match".format(**result)
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
