#!/usr/bin/env python3
"""Usage: import_config_sources.py CANDIDATE [--apply --expect-sha256 SHA]
[--target fleet-platform|fleet-platform-reconcile-stage-20261118]. Default read-only.
Creates only four missing sources tables, inserts five recovered records; D1 untouched.
"""

import argparse, json, subprocess
from pathlib import Path
from import_runtime import Client, quote, filehash
from import_platform_missing import canonical_time

TABLES = {
    "automation_sources": 2,
    "finance_sources": 1,
    "home_sources": 1,
    "research_sources": 1,
}
COLS = [
    "id",
    "name",
    "kind",
    "drive_file_id",
    "drive_modified_at",
    "import_batch",
    "imported_at",
    "note",
]


def ddl(table):
    if table not in TABLES:
        raise ValueError("table not approved")
    return (
        "CREATE TABLE "
        + quote(table)
        + " (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, kind TEXT NOT NULL, drive_file_id TEXT, drive_modified_at TEXT, import_batch TEXT NOT NULL, imported_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), note TEXT)"
    )


def validate(data):
    if set(data) != set(TABLES):
        raise ValueError("wrong tables")
    for table, rows in data.items():
        if len(rows) != TABLES[table] or len({r[0] for r in rows}) != len(rows):
            raise ValueError("wrong count/duplicate ID")
        for r in rows:
            if (
                len(r) != 8
                or type(r[0]) is not int
                or any(v is not None and not isinstance(v, str) for v in r[1:])
            ):
                raise ValueError("row type")
            if any(r[i] is None for i in [1, 2, 5, 6]):
                raise ValueError("required field")
            canonical_time(r[6])
            if r[4] is not None:
                canonical_time(r[4])


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("candidate", type=Path)
    p.add_argument("--apply", action="store_true")
    p.add_argument("--expect-sha256")
    p.add_argument(
        "--target",
        choices=["fleet-platform", "fleet-platform-reconcile-stage-20261118"],
        default="fleet-platform",
    )
    a = p.parse_args()
    digest = filehash(a.candidate)
    if a.apply and digest != a.expect_sha256:
        p.error("reviewed candidate SHA required")
    data = json.loads(a.candidate.read_text())
    validate(data)
    cmd = (
        ["fleet", "secrets", "get", "turso.fleet-platform-rw-token"]
        if a.target == "fleet-platform"
        else ["turso", "db", "tokens", "create", a.target, "--expiration", "1d"]
    )
    token = (
        subprocess.run(cmd, capture_output=True, text=True, check=True, timeout=60)
        .stdout.strip()
        .splitlines()[-1]
    )
    c = Client(token, a.target)
    begun = False
    attempted = False
    try:
        if a.apply:
            c.execute("BEGIN IMMEDIATE")
            begun = True
        existing = {r[0] for r in c.execute("SELECT name FROM sqlite_master")}
        if set(data) & existing:
            raise ValueError("target already exists; refuse duplicate/partial import")
        print(
            "apply" if a.apply else "dry-run",
            "four new tables, five rows",
            digest,
            flush=True,
        )
        if not a.apply:
            return
        for table, rows in data.items():
            c.execute(ddl(table))
            for row in rows:
                c.execute(
                    "INSERT INTO "
                    + quote(table)
                    + " ("
                    + ",".join(quote(v) for v in COLS)
                    + ") VALUES (?,?,?,?,?,?,?,?)",
                    row,
                )
            if int(c.execute("SELECT count(*) FROM " + quote(table))[0][0]) != len(
                rows
            ):
                raise ValueError("count mismatch")
        if filehash(a.candidate) != digest:
            raise ValueError("candidate changed")
        attempted = True
        c.execute("COMMIT")
        begun = False
        print("COMMIT acknowledged", flush=True)
    except Exception:
        if begun and not attempted:
            try:
                c.execute("ROLLBACK")
                print("ROLLBACK acknowledged", flush=True)
            except Exception:
                print("ROLLBACK uncertain", flush=True)
        elif attempted:
            print("COMMIT uncertain; inspect before retry", flush=True)
        raise
    finally:
        try:
            c.close()
        except Exception:
            pass


if __name__ == "__main__":
    main()
