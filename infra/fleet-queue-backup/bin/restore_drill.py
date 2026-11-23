#!/usr/bin/env python3
"""Restore drill: rebuild a Turso store from the D1 backup copy (#8584).

Command line: --target-url URL [--d1-name NAME]

Credentials come from the environment only and are never printed:
TURSO_TARGET_TOKEN (a write token for the SCRATCH target store —
never the production queue token), CLOUDFLARE_API_TOKEN
(cloudflare.api-token-account-wide), and CLOUDFLARE_ACCOUNT_ID.

The drill is the acceptance proof for the Turso -> D1 backup, and it
is deliberately not a backup-write check: it reads the schema and
every row out of the D1 copy through the D1 query API, loads them
into the target store with parameterized libsql statements (row
values never pass through SQL text on this path), and then verifies
the target's per-table row counts against the counts the export
recorded in the copy's meta table. A drill whose counts differ exits
non-zero. The target must be a scratch store: the drill drops and
recreates every table it restores.

After this script passes, the runbook's live-claim step
(docs/RUNBOOK.md) starts a queue broker against the restored store
and claims a task through the real CLI, proving the restored store
serves the production code path — counts alone do not close #8584.
"""

import argparse
import os
import sys

import fq_common

# Rows per D1 read page and per pipeline write batch. Sized for the
# measured per-request latency of the laptop-to-Turso path (seconds
# per request, #8584): at 100 the restore ran at ~3 rows/s.
READ_PAGE = 1000
BATCH = 1000


def _require_env(name):
    value = os.environ.get(name)
    if not value:
        raise RuntimeError(f"required environment variable {name} is unset or empty")
    return value


def d1_value(value):
    """Decode one value from the D1 query API's JSON results.

    D1 returns integers/floats/strings/null natively; a blob arrives
    as a list of byte values. Anything else is a shape this drill does
    not understand — fail loudly rather than restore a mangled row.
    """
    if value is None or isinstance(value, (int, float, str)):
        return value
    if isinstance(value, list) and all(
        isinstance(b, int) and 0 <= b <= 255 for b in value
    ):
        return bytes(value)
    raise ValueError(f"unsupported D1 value shape: {fq_common._shape(value)}")


def d1_schema(d1, db_id):
    return d1.query(
        db_id,
        "SELECT type, name, tbl_name, sql FROM sqlite_master "
        "WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY rowid",
    )


def d1_columns(create_sql):
    """Column names in table order, parsed from the CREATE TABLE --
    the drill never trusts JSON object key order for column position,
    and the D1 query API refuses PRAGMA statements (SQLITE_AUTH, seen
    live on the first drill run, #8584), so PRAGMA table_info cannot
    be the source."""
    return fq_common.table_columns(create_sql)


def d1_read_table(d1, db_id, table, create_sql):
    columns = d1_columns(create_sql)
    offset = 0
    for _page in range(fq_common.MAX_PAGES):
        rows = d1.query(
            db_id,
            f'SELECT * FROM "{table}" LIMIT {READ_PAGE} OFFSET {offset}',
        )
        for row in rows:
            yield tuple(d1_value(row[c]) for c in columns)
        if len(rows) < READ_PAGE:
            return
        offset += READ_PAGE
    raise RuntimeError(
        f"{table}: D1 pagination exceeded {fq_common.MAX_PAGES} pages; "
        "refusing to page unboundedly"
    )


def restore(d1_name, target_url):
    d1 = fq_common.D1Client(
        _require_env("CLOUDFLARE_API_TOKEN"), _require_env("CLOUDFLARE_ACCOUNT_ID")
    )
    db = d1.find_database(d1_name)
    if db is None:
        raise RuntimeError(f"D1 database {d1_name!r} does not exist")
    db_id = db["uuid"]
    meta = d1.read_meta(db_id)
    if meta is None:
        raise RuntimeError("D1 copy has no export meta; nothing trustworthy to restore")
    expected = {k: int(v) for k, v in meta["counts"].items()}

    target = fq_common.LibsqlClient(target_url, _require_env("TURSO_TARGET_TOKEN"))
    schema = [
        (r["type"], r["name"], r["tbl_name"], r["sql"]) for r in d1_schema(d1, db_id)
    ]
    # D1 keeps its own _cf_KV table in every database: it is not part
    # of the backup, the query API prohibits reading it ("access to
    # _cf_KV.key is prohibited: SQLITE_AUTH", seen live on the first
    # drill run, #8584), and the target must not recreate it.
    schema = [entry for entry in schema if entry[1] != "_cf_KV"]
    tables = [(name, sql) for kind, name, _t, sql in schema if kind == "table"]
    for name, _sql in tables:
        target.execute(f'DROP TABLE IF EXISTS "{name}"')
    for _name, sql in tables:
        target.execute(sql.rstrip().rstrip(";"))
    for name, sql in tables:
        batch = []
        placeholders = None
        for row in d1_read_table(d1, db_id, name, sql):
            if placeholders is None:
                placeholders = ", ".join("?" for _ in row)
            batch.append((f'INSERT INTO "{name}" VALUES ({placeholders})', row))
            if len(batch) >= BATCH:
                target.execute_many(batch)
                batch = []
        if batch:
            target.execute_many(batch)
    for kind, _name, _t, sql in schema:
        if kind in ("index", "view", "trigger"):
            target.execute(sql.rstrip().rstrip(";"))

    actual = fq_common.table_counts(target, fq_common.read_schema(target))
    mismatched = {
        name: (expected[name], actual.get(name))
        for name in expected
        if actual.get(name) != expected[name]
    }
    if mismatched:
        raise RuntimeError(f"restored count mismatch: {mismatched}")
    return {
        "d1_name": d1_name,
        "exported_at": meta["exported_at"],
        "tables": len(expected),
        "rows": sum(expected.values()),
    }


def _parser():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument(
        "--target-url",
        required=True,
        help="libsql HTTP URL of the SCRATCH target store",
    )
    parser.add_argument(
        "--d1-name",
        default=fq_common.DEFAULT_D1_NAME,
        help="D1 database name (default: %(default)s)",
    )
    return parser


def main(argv=None):
    args = _parser().parse_args(argv)
    try:
        result = restore(args.d1_name, args.target_url)
    except (OSError, RuntimeError, ValueError) as exc:
        print(f"fleet-queue-backup: restore drill failed: {exc}", file=sys.stderr)
        return 1
    print(
        "fleet-queue-backup: restored {tables} tables / {rows} rows from D1 "
        "{d1_name} (export {exported_at}); target counts match the export".format(
            **result
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
