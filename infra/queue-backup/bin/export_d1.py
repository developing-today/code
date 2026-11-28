#!/usr/bin/env python3
"""Export the fleet-queue store to a D1-loadable SQL dump, self-verified.

Command line: --source PATH --out PATH [--manifest PATH] [--force].
The source is a SQLite database file (a local replica or scratch copy of
the Turso queue store); it is opened read-only and never modified. The
export is written only after the dump loads into a fresh vanilla SQLite
database with the source's exact schema fingerprint and per-table row
counts — that load is the D1-fidelity check (D1 is SQLite). The manifest
records the counts and hashes the restore drill verifies against. This
command never reads or prints credentials.
"""

import argparse
import hashlib
import json
import os
import sqlite3
import sys
import tempfile
from datetime import datetime, timezone

import queue_backup_common as qb


def export(source, out, manifest_path, force=False):
    if not force and (os.path.exists(out) or os.path.exists(manifest_path)):
        raise FileExistsError(
            f"refusing to overwrite existing export: {out} / {manifest_path}"
            " (pass --force to replace)"
        )
    conn = qb.open_readonly(source)
    try:
        counts = qb.table_counts(conn)
        if not counts:
            raise RuntimeError(f"source has no user tables: {source}")
        fingerprint = qb.schema_sha256(conn)
        dump = qb.dump_database(conn)
    finally:
        conn.close()

    # Self-verify before anything is written: the dump must stand alone.
    fd, scratch = tempfile.mkstemp(prefix=".export-verify-", suffix=".db")
    os.close(fd)
    os.unlink(scratch)
    try:
        loaded = qb.load_dump_text(dump, scratch)
        try:
            if qb.table_counts(loaded) != counts:
                raise RuntimeError("dump load counts differ from the source")
            if qb.schema_sha256(loaded) != fingerprint:
                raise RuntimeError("dump load schema differs from the source")
        finally:
            loaded.close()
    finally:
        try:
            os.unlink(scratch)
        except FileNotFoundError:
            pass

    raw = dump.encode("utf-8")
    manifest = {
        "tool": "queue-backup/export_d1",
        "source": source,
        "exported_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "tables": counts,
        "schema_sha256": fingerprint,
        "dump_sha256": hashlib.sha256(raw).hexdigest(),
        "dump_bytes": len(raw),
    }
    with open(out, "w", encoding="utf-8") as handle:
        handle.write(dump)
    with open(manifest_path, "w", encoding="utf-8") as handle:
        json.dump(manifest, handle, indent=2, sort_keys=True)
        handle.write("\n")
    return manifest


def _parser():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--source", required=True, help="queue store DB file")
    parser.add_argument("--out", required=True, help="SQL dump to write")
    parser.add_argument(
        "--manifest", help="manifest JSON (defaults to <out>.manifest.json)"
    )
    parser.add_argument(
        "--force", action="store_true", help="replace an existing export"
    )
    return parser


def main(argv=None):
    args = _parser().parse_args(argv)
    manifest_path = args.manifest or (args.out + ".manifest.json")
    try:
        manifest = export(args.source, args.out, manifest_path, force=args.force)
    except (OSError, RuntimeError, sqlite3.Error) as exc:
        print(f"queue-backup: export: {exc}", file=sys.stderr)
        return 1
    tables = manifest["tables"]
    print(
        f"queue-backup: exported {len(tables)} tables"
        f" ({sum(tables.values())} rows) to {args.out};"
        f" manifest at {manifest_path}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
