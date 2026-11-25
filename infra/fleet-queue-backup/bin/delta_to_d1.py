#!/usr/bin/env python3
"""Bring a D1 copy of a Turso store up to date for a cutover.

Command line: --source-url URL --d1-name NAME [--append T1,T2,...]

Run it after a full export_to_d1.py copy, with the store's writers
stopped. Append tables (the --append list, plus any table guarded by a
<table>_no_delete trigger) get only the rows past D1's max(rowid);
every other table is replaced whole (DELETE then INSERT), so updates
made since the bulk copy land too. Tables new since the bulk copy are
created. Turso's own __turso_internal_* tables are skipped. Row counts
are read back from D1 and must match the source.

Credentials come from the environment: TURSO_QUEUE_TOKEN (the source
database's token), CLOUDFLARE_API_TOKEN, CLOUDFLARE_ACCOUNT_ID.
"""

import argparse
import os
import re
import sys

import fq_common


def _env(name):
    v = os.environ.get(name)
    if not v:
        raise RuntimeError(f"required environment variable {name} is unset or empty")
    return v


def _rows_after(client, table, colnames, after):
    quoted = ", ".join(f'quote("{c}")' for c in colnames)
    last = after
    for _page in range(fq_common.MAX_PAGES):
        _cols, rows = client.execute(
            f'SELECT rowid, {quoted} FROM "{table}" WHERE rowid > ? '
            f"ORDER BY rowid LIMIT {fq_common.PAGE_ROWS}",
            (last,),
        )
        for row in rows:
            yield row[1:]
        if len(rows) < fq_common.PAGE_ROWS:
            return
        last = rows[-1][0]
    raise RuntimeError(f"{table}: rowid paging exceeded its bound")


def _emit(lines, splits, table, colnames, pk_cols, row):
    stmt = f'INSERT INTO "{table}" VALUES (' + ", ".join(row) + ");"
    if len(stmt.encode("utf-8")) <= fq_common.MAX_IMPORT_STATEMENT_BYTES:
        lines.append(stmt)
    else:
        lines.extend(fq_common._split_row_insert(table, colnames, row, pk_cols, splits))


def delta(source_url, d1_name, append):
    src = fq_common.LibsqlClient(source_url, _env("TURSO_QUEUE_TOKEN"))
    d1 = fq_common.D1Client(_env("CLOUDFLARE_API_TOKEN"), _env("CLOUDFLARE_ACCOUNT_ID"))
    db = d1.find_database(d1_name)
    if db is None:
        raise RuntimeError(f"D1 database {d1_name!r} does not exist")
    db_id = db["uuid"]
    schema = fq_common.read_schema(src)
    tables = [
        (name, sql)
        for kind, name, _t, sql in schema
        if kind == "table" and not name.startswith("__turso_internal")
    ]
    guarded = {
        name[: -len("_no_delete")]
        for kind, name, _t, _sql in schema
        if kind == "trigger" and name.endswith("_no_delete")
    }
    append = set(append) | guarded
    existing = {
        r["name"]
        for r in d1.query(db_id, "SELECT name FROM sqlite_master WHERE type = 'table'")
    }
    # D1 enforces foreign keys; check them at the end of the import.
    lines, splits, report = ["PRAGMA defer_foreign_keys = true;"], [], {}
    for name, sql in tables:
        if name not in existing:
            lines.append(sql.rstrip().rstrip(";") + ";")
    for name, sql in tables:
        colnames, _ = src.execute(f'SELECT * FROM "{name}" LIMIT 0')
        pk_cols = fq_common._pk_columns(sql)
        n = 0
        if name in append and name in existing:
            m = d1.query(db_id, f'SELECT coalesce(max(rowid), 0) AS m FROM "{name}"')
            after = m[0]["m"] if m else 0
            for row in _rows_after(src, name, colnames, after):
                _emit(lines, splits, name, colnames, pk_cols, row)
                n += 1
            report[name] = f"+{n}"
        else:
            if name in existing:
                lines.append(f'DELETE FROM "{name}";')
            for row in fq_common.read_table_quoted(src, name, len(colnames)):
                _emit(lines, splits, name, colnames, pk_cols, row)
                n += 1
            report[name] = f"={n}"
    for kind, name, tbl, sql in schema:
        if kind in ("index", "trigger") and tbl not in existing and not tbl.startswith("__turso_internal"):
            lines.append(sql.rstrip().rstrip(";") + ";")
    if len(lines) > 1:
        d1.import_sql(db_id, "\n".join(lines) + "\n")
    for table, col, where, expected in splits:
        rows = d1.query(db_id, f'SELECT length("{col}") AS n FROM "{table}" WHERE {where}')
        if len(rows) != 1 or rows[0].get("n") != expected:
            raise RuntimeError(f"split-row verification failed for {table}.{col}")
    names = [n for n, _ in tables]
    source_counts = {n: fq_common.count_rows(src, n) for n in names}
    d1_counts = d1.counts(db_id, names)
    bad = {n: (source_counts[n], d1_counts.get(n)) for n in names if d1_counts.get(n) != source_counts[n]}
    if bad:
        raise RuntimeError(f"count mismatch after delta (source, d1): {bad}")
    return report, sum(source_counts.values())


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    p.add_argument("--source-url", required=True)
    p.add_argument("--d1-name", required=True)
    p.add_argument("--append", default="", help="comma-separated append-only tables")
    a = p.parse_args(argv)
    try:
        report, total = delta(a.source_url, a.d1_name, [t for t in a.append.split(",") if t])
    except (OSError, RuntimeError, ValueError) as exc:
        print(f"delta_to_d1: failed: {exc}", file=sys.stderr)
        return 1
    print(f"delta_to_d1: {a.d1_name} up to date; {total} rows; " + " ".join(f"{k}{v}" for k, v in report.items()))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
