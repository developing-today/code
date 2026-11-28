#!/usr/bin/env python3
"""Restore drill: rebuild a D1 database from its Turso backup home (#13458).

Command line: --out PATH [--home-url URL] [--d1-name NAME]

The acceptance proof for the mesh class-D leg (D1 -> Turso home,
export_to_turso.py): read the home's committed import record, load
every recorded table out of the home over the libsql HTTP API into a
local SQLite file (the issue's "local SQLite equivalent" scratch
target), and verify the rebuilt per-table row counts against the
record. A drill whose counts differ exits non-zero. The drill never
writes to the home or to any production store; --out must name a
scratch file, and an existing file there is replaced.

This is deliberately not a backup-write check: it proves the home
alone suffices to rebuild the D1 database's content, which is the
property the mesh buys (§9 of the design: backup-verified is not
restore-verified).
"""

import argparse
import os
import sqlite3
import sys

import export_to_turso
import fq_common


def _require_env(name):
    value = os.environ.get(name)
    if not value:
        raise RuntimeError(f"required environment variable {name} is unset or empty")
    return value


def _read_table_values(client, table):
    """Yield every row of `table` as value tuples, paged with the
    #6622 bound (fq_common.read_table_quoted's shape, unquoted)."""
    offset = 0
    for _page in range(fq_common.MAX_PAGES):
        _cols, rows = client.execute(
            f'SELECT * FROM "{table}" LIMIT {fq_common.PAGE_ROWS} OFFSET {offset}'
        )
        yield from rows
        if len(rows) < fq_common.PAGE_ROWS:
            return
        offset += fq_common.PAGE_ROWS
    raise RuntimeError(
        f"{table}: pagination exceeded {fq_common.MAX_PAGES} pages; "
        "refusing to page unboundedly"
    )


def restore(home_url, out_path, d1_name):
    home = fq_common.LibsqlClient(home_url, _require_env("TURSO_HOME_TOKEN"))
    record = export_to_turso.read_import_record(home)
    if record is None:
        raise RuntimeError("Turso home has no import record; nothing to verify")
    if d1_name is not None and record["d1_name"] != d1_name:
        raise RuntimeError(
            f"home holds an import of {record['d1_name']!r}, not {d1_name!r}"
        )
    expected = {k: int(v) for k, v in record["counts"].items()}
    schema = [tuple(entry) for entry in record["schema"]]
    tables = [(name, sql) for kind, name, _t, sql in schema if kind == "table"]

    if os.path.exists(out_path):
        os.unlink(out_path)
    conn = sqlite3.connect(out_path)
    try:
        for _name, sql in tables:
            conn.execute(sql.rstrip().rstrip(";"))
        for name, _sql in tables:
            batch = []
            placeholders = None
            for row in _read_table_values(home, name):
                if placeholders is None:
                    placeholders = ", ".join("?" for _ in row)
                batch.append(tuple(row))
                if len(batch) >= 1000:
                    conn.executemany(
                        f'INSERT INTO "{name}" VALUES ({placeholders})', batch
                    )
                    batch = []
            if batch:
                conn.executemany(
                    f'INSERT INTO "{name}" VALUES ({placeholders})', batch
                )
        for kind, _name, _t, sql in schema:
            if kind in ("index", "view", "trigger"):
                conn.execute(sql.rstrip().rstrip(";"))
        conn.commit()
        actual = {
            name: conn.execute(f'SELECT count(*) FROM "{name}"').fetchone()[0]
            for name, _sql in tables
        }
    finally:
        conn.close()
    mismatched = {
        name: (expected[name], actual.get(name))
        for name in expected
        if actual.get(name) != expected[name]
    }
    if mismatched:
        raise RuntimeError(f"rebuilt count mismatch: {mismatched}")
    return {
        "d1_name": record["d1_name"],
        "imported_at": record["imported_at"],
        "tables": len(expected),
        "rows": sum(expected.values()),
        "out": out_path,
    }


def _parser():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument(
        "--out",
        required=True,
        help="scratch SQLite file to rebuild into (replaced if present)",
    )
    parser.add_argument(
        "--home-url",
        default=export_to_turso.DEFAULT_HOME_URL,
        help="libsql HTTP URL of the Turso home (default: %(default)s)",
    )
    parser.add_argument(
        "--d1-name",
        default=None,
        help="expected source D1 database name (default: whatever the "
        "home's import record names)",
    )
    return parser


def main(argv=None):
    args = _parser().parse_args(argv)
    try:
        result = restore(args.home_url, args.out, args.d1_name)
    except (OSError, RuntimeError, ValueError, sqlite3.Error) as exc:
        print(f"fleet-queue-backup: D1 home drill failed: {exc}", file=sys.stderr)
        return 1
    print(
        "fleet-queue-backup: rebuilt {tables} tables / {rows} rows of D1 "
        "{d1_name} from its Turso home (import {imported_at}) into {out}; "
        "counts match the import record".format(**result)
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
