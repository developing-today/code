#!/usr/bin/env python3
"""Restore drill for the D1 plane: rebuild the backup copy into a
scratch D1 database and verify it (CF-3, #14943).

Command line: [--from-d1-name NAME] --scratch-name NAME [--create-scratch] [--drop-scratch]

The CF-3 acceptance drill, and the disaster path if the live D1
`fleet-queue` plane is ever lost: read the backup D1 database
(`fleet-queue-backup` by default) back out through the D1 query API
(the same build_dump the exporter uses), import it into a scratch
D1 database, and verify the scratch's per-table row counts and table
set against the counts the export recorded in the backup's
_fleet_queue_backup_meta table. A drill whose counts do not match
the export record fails loudly and the scratch is not trustworthy.

Credentials come from the environment only and are never printed:
CLOUDFLARE_API_TOKEN (cloudflare.api-token-account-wide) and
CLOUDFLARE_ACCOUNT_ID. --create-scratch permits creating the scratch
database (provisioning stays explicit); --drop-scratch deletes it
after a successful verification, which is the normal drill ending —
a disaster restore omits both flags' destructive half and keeps the
restored database.
"""

import argparse
import os
import sys

import fq_common


def _require_env(name):
    value = os.environ.get(name)
    if not value:
        raise RuntimeError(f"required environment variable {name} is unset or empty")
    return value


def drill_with_clients(d1, from_name, scratch_name, create_scratch, drop_scratch):
    """The drill over an already-built D1Client (the hermetic tests
    drive this with a SQLite-backed fake; drill() is the live path)."""
    if from_name == scratch_name:
        raise RuntimeError(
            f"backup source and scratch target are the same ({from_name!r}); "
            "a restore drill must land in a separate database"
        )
    backup_db = d1.find_database(from_name)
    if backup_db is None:
        raise RuntimeError(f"backup D1 database {from_name!r} does not exist")
    meta = d1.read_meta(backup_db["uuid"])
    if meta is None:
        raise RuntimeError(
            f"backup D1 database {from_name!r} has no export meta; "
            "there is no verified export to restore"
        )
    expected = meta.get("counts") or {}
    backup = fq_common.D1SourceClient(d1, backup_db["uuid"])
    dump, counts, _splits = fq_common.build_dump(backup, meta.get("exported_at", ""))
    # The dump re-read must describe the same copy the meta records:
    # a backup whose live counts have drifted from its export record
    # is a smear (an export ran, or is running, in between) — fail
    # here rather than verify a copy that never existed.
    if counts != expected:
        raise RuntimeError(
            f"backup re-read counts {counts} differ from the export record "
            f"{expected}; refusing to drill a smeared copy"
        )
    scratch = d1.find_database(scratch_name)
    if scratch is None:
        if not create_scratch:
            raise RuntimeError(
                f"scratch D1 database {scratch_name!r} does not exist; rerun "
                "with --create-scratch to provision it"
            )
        scratch = d1.create_database(scratch_name)
    scratch_id = scratch["uuid"]
    d1.import_sql(scratch_id, dump)
    readback = d1.counts(scratch_id, list(expected))
    mismatched = {
        name: (expected[name], readback.get(name))
        for name in expected
        if readback.get(name) != expected[name]
    }
    if mismatched:
        raise RuntimeError(f"restored count mismatch: {mismatched}")
    if drop_scratch:
        d1.delete_database(scratch_id)
    return {
        "from_d1_name": from_name,
        "scratch_name": scratch_name,
        "scratch_uuid": scratch_id,
        "exported_at": meta.get("exported_at"),
        "tables": len(expected),
        "rows": sum(expected.values()),
        "counts": expected,
        "dropped": bool(drop_scratch),
    }


def drill(from_name, scratch_name, create_scratch, drop_scratch):
    d1 = fq_common.D1Client(
        _require_env("CLOUDFLARE_API_TOKEN"), _require_env("CLOUDFLARE_ACCOUNT_ID")
    )
    return drill_with_clients(d1, from_name, scratch_name, create_scratch, drop_scratch)


def _parser():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument(
        "--from-d1-name",
        default=fq_common.DEFAULT_D1_NAME,
        help="backup D1 database to restore from (default: %(default)s)",
    )
    parser.add_argument(
        "--scratch-name",
        required=True,
        help="scratch D1 database to restore into (required; never the live or backup name)",
    )
    parser.add_argument(
        "--create-scratch",
        action="store_true",
        help="create the scratch D1 database if it does not exist yet",
    )
    parser.add_argument(
        "--drop-scratch",
        action="store_true",
        help="delete the scratch D1 database after a successful verification",
    )
    return parser


def main(argv=None):
    args = _parser().parse_args(argv)
    try:
        result = drill(
            args.from_d1_name, args.scratch_name, args.create_scratch, args.drop_scratch
        )
    except (OSError, RuntimeError, ValueError) as exc:
        print(f"fleet-queue-backup: D1 restore drill failed: {exc}", file=sys.stderr)
        return 1
    print(
        "fleet-queue-backup: restored {tables} tables / {rows} rows from D1 "
        "{from_d1_name} (export {exported_at}) into scratch D1 {scratch_name} "
        "({scratch_uuid}); restored counts match the export record".format(**result)
        + ("; scratch dropped" if result["dropped"] else "")
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
