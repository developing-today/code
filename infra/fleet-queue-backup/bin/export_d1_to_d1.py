#!/usr/bin/env python3
"""Export the live D1 fleet-queue plane into its D1 backup copy (CF-3, #14943).

Command line: [--source-d1-name NAME] [--d1-name NAME] [--create]

This is the CF-2-successor of export_to_d1.py: once CF-2 makes D1
`fleet-queue` the live queue plane, the backup's source is that D1
database, not the Turso store. The target stays the separate backup
D1 database `fleet-queue-backup` — source and target names must
differ, and this tool refuses to run when they do not, so the live
plane can never be "backed up" onto itself.

Credentials come from the environment only and are never printed:
CLOUDFLARE_API_TOKEN (cloudflare.api-token-account-wide) and
CLOUDFLARE_ACCOUNT_ID. The systemd wrapper fetches the token by name
at runtime (systemd/run-export-d1.sh); an interactive caller exports
it the same way.

The export is the same full logical copy as the Turso leg
(fq_common.build_dump over a D1SourceClient): schema + every row,
literals rendered by quote() at the source, then a read-back
verification of per-table row counts against the counts emitted in
the same run — an export that cannot verify does not report success.
--create permits creating the target D1 database when it does not
exist yet; without it a missing target is an error, so provisioning
stays an explicit act.
"""

import argparse
import os
import sys
from datetime import datetime, timezone

import fq_common


def _require_env(name):
    value = os.environ.get(name)
    if not value:
        raise RuntimeError(f"required environment variable {name} is unset or empty")
    return value


def export_with_clients(d1, source_name, target_name, create):
    """The export over an already-built D1Client (the hermetic tests
    drive this with a SQLite-backed fake; export() is the live path)."""
    if source_name == target_name:
        raise RuntimeError(
            f"source and target D1 databases are the same ({source_name!r}); "
            "the live plane must never be its own backup target"
        )
    source_db = d1.find_database(source_name)
    if source_db is None:
        raise RuntimeError(f"source D1 database {source_name!r} does not exist")
    source = fq_common.D1SourceClient(d1, source_db["uuid"])
    exported_at = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    dump, counts, splits = fq_common.build_dump(source, exported_at)
    db = d1.find_database(target_name)
    if db is None:
        if not create:
            raise RuntimeError(
                f"D1 database {target_name!r} does not exist; rerun with --create "
                "to provision it in the fleet Cloudflare account"
            )
        db = d1.create_database(target_name)
    db_id = db["uuid"]
    d1.import_sql(db_id, dump)
    for table, col, where, expected in splits:
        rows = d1.query(
            db_id,
            f'SELECT length("{col}") AS n FROM "{table}" WHERE {where}',
        )
        if len(rows) != 1 or rows[0].get("n") != expected:
            raise RuntimeError(
                f"split-row verification failed for {table}.{col}: "
                f"expected one row of length {expected}, got {rows}"
            )
    meta = d1.read_meta(db_id)
    if meta is None:
        raise RuntimeError("D1 copy has no export meta after import; import is suspect")
    readback = d1.counts(db_id, list(counts))
    mismatched = {
        name: (counts[name], readback.get(name))
        for name in counts
        if readback.get(name) != counts[name]
    }
    if mismatched:
        raise RuntimeError(f"D1 read-back count mismatch: {mismatched}")
    return {
        "source_d1_name": source_name,
        "d1_name": target_name,
        "d1_uuid": db_id,
        "exported_at": exported_at,
        "tables": len(counts),
        "rows": sum(counts.values()),
        "counts": counts,
    }


def export(source_name, target_name, create):
    d1 = fq_common.D1Client(
        _require_env("CLOUDFLARE_API_TOKEN"), _require_env("CLOUDFLARE_ACCOUNT_ID")
    )
    return export_with_clients(d1, source_name, target_name, create)


def _parser():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument(
        "--source-d1-name",
        default=fq_common.LIVE_D1_NAME,
        help="live D1 database to export from (default: %(default)s)",
    )
    parser.add_argument(
        "--d1-name",
        default=fq_common.DEFAULT_D1_NAME,
        help="backup D1 database to export into (default: %(default)s)",
    )
    parser.add_argument(
        "--create",
        action="store_true",
        help="create the target D1 database if it does not exist yet",
    )
    return parser


def main(argv=None):
    args = _parser().parse_args(argv)
    try:
        result = export(args.source_d1_name, args.d1_name, args.create)
    except (OSError, RuntimeError, ValueError) as exc:
        print(f"fleet-queue-backup: D1 export failed: {exc}", file=sys.stderr)
        return 1
    print(
        "fleet-queue-backup: exported {tables} tables / {rows} rows from D1 "
        "{source_d1_name} to D1 {d1_name} ({d1_uuid}) at {exported_at}; "
        "read-back counts match".format(**result)
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
