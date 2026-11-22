#!/usr/bin/env python3
"""Export the fleet-queue Turso store into its D1 backup copy (#8584).

Command line: [--d1-name NAME] [--source-url URL] [--create]

Credentials come from the environment only and are never printed:
TURSO_QUEUE_TOKEN (the turso.fleet-queue-rw-token vault entry),
CLOUDFLARE_API_TOKEN (cloudflare.api-token-account-wide), and
CLOUDFLARE_ACCOUNT_ID. The systemd wrapper fetches them by name at
runtime (systemd/run-export.sh); an interactive caller exports them
the same way.

The export is a full logical copy: schema + every row, with literals
rendered by SQLite's quote() at the source (fq_common.build_dump).
After the D1 import, the copy is verified by reading the per-table
row counts back out of D1 and comparing them with the source counts
taken in the same run — an export that cannot verify does not report
success. --create permits creating the D1 database when it does not
exist yet; without it a missing database is an error, so provisioning
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


def export(source_url, d1_name, create):
    source = fq_common.LibsqlClient(source_url, _require_env("TURSO_QUEUE_TOKEN"))
    d1 = fq_common.D1Client(
        _require_env("CLOUDFLARE_API_TOKEN"), _require_env("CLOUDFLARE_ACCOUNT_ID")
    )
    exported_at = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    dump, counts = fq_common.build_dump(source, exported_at)
    db = d1.find_database(d1_name)
    if db is None:
        if not create:
            raise RuntimeError(
                f"D1 database {d1_name!r} does not exist; rerun with --create "
                "to provision it in the fleet Cloudflare account"
            )
        db = d1.create_database(d1_name)
    db_id = db["uuid"]
    d1.import_sql(db_id, dump)
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
        "d1_name": d1_name,
        "d1_uuid": db_id,
        "exported_at": exported_at,
        "tables": len(counts),
        "rows": sum(counts.values()),
        "counts": counts,
    }


def _parser():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument(
        "--d1-name",
        default=fq_common.DEFAULT_D1_NAME,
        help="D1 database name (default: %(default)s)",
    )
    parser.add_argument(
        "--source-url",
        default=fq_common.DEFAULT_TURSO_URL,
        help="libsql HTTP URL of the source store (default: the fleet-queue store)",
    )
    parser.add_argument(
        "--create",
        action="store_true",
        help="create the D1 database if it does not exist yet",
    )
    return parser


def main(argv=None):
    args = _parser().parse_args(argv)
    try:
        result = export(args.source_url, args.d1_name, args.create)
    except (OSError, RuntimeError, ValueError) as exc:
        print(f"fleet-queue-backup: export failed: {exc}", file=sys.stderr)
        return 1
    print(
        "fleet-queue-backup: exported {tables} tables / {rows} rows to D1 "
        "{d1_name} ({d1_uuid}) at {exported_at}; read-back counts match".format(
            **result
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
