#!/usr/bin/env python3
"""Restore drill for a laptop-db backup home (#13474, mesh §9).

Backup-verified is not restore-verified. This drill reads a Turso
backup home (laptop-backup-<name>) back out over the libsql HTTP
API, rebuilds it into a scratch local SQLite file, and verifies the
restored per-table row counts against the counts the import recorded
in the home's meta table. It never writes to the source database or
to the home: the home is opened read-only in effect (SELECTs only)
and the only file written is the scratch target.

Command line: --home-url URL --out PATH [--home-name NAME]

Credentials come from the environment only and are never printed:
TURSO_HOME_TOKEN (the home's vault entry). The drill prints a JSON
receipt: the import record's id and timestamp, and per-table
expected vs restored counts.
"""

import argparse
import json
import os
import sqlite3
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ldb_common
import import_to_turso


def run_drill(home, out_path):
    """Rebuild the home into out_path; return the drill receipt.

    Raises on any mismatch between the restored copy and the home's
    committed import record.
    """
    record = import_to_turso.read_import_record(home)
    if record is None:
        raise RuntimeError("home has no import record; nothing to verify against")
    schema = [tuple(entry) for entry in record["schema"]]
    if os.path.exists(out_path):
        os.unlink(out_path)
    dst = sqlite3.connect(out_path)
    try:
        for kind, _name, _tbl, sql in schema:
            if kind == "table":
                dst.execute(sql.rstrip().rstrip(";"))
        restored = {}
        for name in ldb_common.table_names(schema):
            loaded = 0
            batch = []
            placeholders = None
            for row in ldb_common.read_table_paged(home, name):
                if placeholders is None:
                    placeholders = ", ".join("?" for _ in row)
                batch.append(tuple(row))
                loaded += 1
                if len(batch) >= import_to_turso.BATCH:
                    dst.executemany(
                        f'INSERT INTO "{name}" VALUES ({placeholders})', batch
                    )
                    batch = []
            if batch:
                dst.executemany(
                    f'INSERT INTO "{name}" VALUES ({placeholders})', batch
                )
            restored[name] = loaded
        for kind, _name, _tbl, sql in schema:
            if kind in ("index", "view", "trigger"):
                dst.execute(sql.rstrip().rstrip(";"))
        dst.commit()
    finally:
        dst.close()
    expected = record["counts"]
    mismatched = {
        name: (expected.get(name), restored.get(name))
        for name in set(expected) | set(restored)
        if expected.get(name) != restored.get(name)
    }
    if mismatched:
        raise RuntimeError(f"restore count mismatch: {mismatched}")
    return {
        "import_id": record["import_id"],
        "imported_at": record["imported_at"],
        "home": record.get("home", ""),
        "out": out_path,
        "counts": restored,
        "verified": True,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--home-url", required=True, help="libsql URL of the home")
    parser.add_argument("--home-name", default="", help="home name for the receipt")
    parser.add_argument("--out", required=True, help="scratch SQLite file to build")
    args = parser.parse_args(argv)
    token = import_to_turso._require_env("TURSO_HOME_TOKEN")
    home = ldb_common.LibsqlClient(args.home_url, token)
    receipt = run_drill(home, args.out)
    if args.home_name:
        receipt["home"] = args.home_name
    print(json.dumps(receipt, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
