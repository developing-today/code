#!/usr/bin/env python3
"""Bring a D1 copy of a Turso store up to date for a cutover.

Command line: --source-url URL --d1-name NAME [--append T1,T2,...]
[--lagging T1,T2,...]

Run it after a full export_to_d1.py copy, with the store's writers
stopped. Append tables (the --append list, plus any table guarded by a
<table>_no_delete trigger) get only the rows past D1's max(rowid);
every other table is dropped and recreated from the source's current
CREATE, then refilled, so updates and schema changes made since the
bulk copy land too. An append table whose columns changed since the
bulk copy is recreated the same way. Tables new since the bulk copy
are created. Turso's own __turso_internal_* tables are skipped. Row
counts are read back from D1 and must match the source.

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


_REFERENCES = re.compile(r'REFERENCES\s+["`\[]?([A-Za-z_][A-Za-z0-9_]*)', re.IGNORECASE)


def _parents(sql):
    """Tables a CREATE TABLE's foreign keys point at."""
    return set(_REFERENCES.findall(sql or ""))


def _parents_first(tables):
    """Table names ordered so every FK parent comes before its children.

    D1 does not hold a deferred foreign-key check across a whole import
    (a delta with a parent dropped under kept children failed with
    FOREIGN KEY constraint failed on 2026-11-26), so the import must
    never leave a child pointing at a missing parent between statements.
    Self references and cycles keep source order.
    """
    names = [n for n, _ in tables]
    deps = {n: _parents(sql) & set(names) - {n} for n, sql in tables}
    out, done = [], set()
    while len(out) < len(names):
        ready = [n for n in names if n not in done and deps[n] <= done]
        if not ready:
            ready = [next(n for n in names if n not in done)]
        for n in ready:
            out.append(n)
            done.add(n)
    return out


def _emit(lines, splits, table, colnames, pk_cols, row):
    stmt = f'INSERT INTO "{table}" VALUES (' + ", ".join(row) + ");"
    if len(stmt.encode("utf-8")) <= fq_common.MAX_IMPORT_STATEMENT_BYTES:
        lines.append(stmt)
    else:
        lines.extend(fq_common._split_row_insert(table, colnames, row, pk_cols, splits))


def delta(source_url, d1_name, append, lagging=()):
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
    d1_tables = {
        r["name"]: r["sql"]
        for r in d1.query(
            db_id, "SELECT name, sql FROM sqlite_master WHERE type = 'table'"
        )
    }
    # An append table keeps its D1 rows only while its columns still
    # match the source; any drift sends it down the full-copy path.
    keep = set()
    for name, sql in tables:
        if name in append and name in d1_tables:
            src_cols, _ = src.execute(f'SELECT * FROM "{name}" LIMIT 0')
            if fq_common.table_columns(d1_tables[name] or "") == list(src_cols):
                keep.add(name)
    # A kept table whose parent is recreated would point at missing
    # rows between the parent's drop and its refill; recreate it too.
    sql_of = dict(tables)
    changed = True
    while changed:
        changed = False
        for name in sorted(keep):
            if _parents(sql_of[name]) - keep - {name}:
                keep.discard(name)
                changed = True
    recreate = {name for name, _sql in tables if name not in keep}
    order = _parents_first(tables)
    # D1 enforces foreign keys; check them at the end of the import.
    # UNIQUE indexes go in before any row, because a child insert whose
    # parent key has no unique index fails as a "foreign key mismatch"
    # even with checks deferred.
    lines, splits, report = ["PRAGMA defer_foreign_keys = true;"], [], {}
    # Children are dropped before their parents and created after them.
    for name in reversed(order):
        if name in recreate and name in d1_tables:
            lines.append(f'DROP TABLE IF EXISTS "{name}";')
    for name in order:
        if name in recreate:
            lines.append(sql_of[name].rstrip().rstrip(";") + ";")
    deferred_schema = []
    for kind, name, tbl, sql in schema:
        if kind not in ("index", "trigger") or tbl not in recreate:
            continue
        if kind == "index" and fq_common._is_unique_index(sql):
            lines.append(sql.rstrip().rstrip(";") + ";")
        else:
            deferred_schema.append(sql.rstrip().rstrip(";") + ";")
    for name in order:
        sql = sql_of[name]
        colnames, _ = src.execute(f'SELECT * FROM "{name}" LIMIT 0')
        pk_cols = fq_common._pk_columns(sql)
        n = 0
        if name in keep:
            m = d1.query(db_id, f'SELECT coalesce(max(rowid), 0) AS m FROM "{name}"')
            after = m[0]["m"] if m else 0
            for row in _rows_after(src, name, colnames, after):
                _emit(lines, splits, name, colnames, pk_cols, row)
                n += 1
            report[name] = f"+{n}"
        else:
            for row in fq_common.read_table_quoted(src, name, len(colnames)):
                _emit(lines, splits, name, colnames, pk_cols, row)
                n += 1
            report[name] = f"={n}"
    lines.extend(deferred_schema)
    if len(lines) > 1:
        d1.import_sql(db_id, "\n".join(lines) + "\n")
    for table, col, where, expected in splits:
        rows = d1.query(
            db_id, f'SELECT length("{col}") AS n FROM "{table}" WHERE {where}'
        )
        if len(rows) != 1 or rows[0].get("n") != expected:
            raise RuntimeError(f"split-row verification failed for {table}.{col}")
    names = [n for n, _ in tables]
    source_counts = {n: fq_common.count_rows(src, n) for n in names}
    d1_counts = d1.counts(db_id, names)
    # A lagging table has a writer that cannot be held for the cutover
    # (a Worker, another host); rows it lands during the delta are
    # copied afterwards, so D1 may trail the source there, never lead.
    lagging = set(lagging)
    bad = {
        n: (source_counts[n], d1_counts.get(n))
        for n in names
        if d1_counts.get(n) != source_counts[n]
        and not (n in lagging and (d1_counts.get(n) or 0) <= source_counts[n])
    }
    if bad:
        raise RuntimeError(f"count mismatch after delta (source, d1): {bad}")
    return report, sum(source_counts.values())


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    p.add_argument("--source-url", required=True)
    p.add_argument("--d1-name", required=True)
    p.add_argument("--append", default="", help="comma-separated append-only tables")
    p.add_argument(
        "--lagging",
        default="",
        help="comma-separated tables whose writers keep running; D1 may trail the source there",
    )
    a = p.parse_args(argv)
    try:
        report, total = delta(
            a.source_url,
            a.d1_name,
            [t for t in a.append.split(",") if t],
            [t for t in a.lagging.split(",") if t],
        )
    except (OSError, RuntimeError, ValueError) as exc:
        print(f"delta_to_d1: failed: {exc}", file=sys.stderr)
        return 1
    print(
        f"delta_to_d1: {a.d1_name} up to date; {total} rows; "
        + " ".join(f"{k}{v}" for k, v in report.items())
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
