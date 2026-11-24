#!/usr/bin/env python3
"""Take aggregate SQLite counts on the Atlas laptop. Requires Python 3.9+.

Usage: atlas_census.py [--canonical PATH] [--shard PATH ...] [--content-store PATH ...]
                       [--seconds-per-store N]
Repeat --shard or --content-store for each additional store.

Reads the canonical database by default. Supply --shard PATH once per approved
scan shard, and --content-store PATH once per separately approved content store.
The report labels stores by role and argument position. Keep the path-to-position
mapping on the laptop. Exit 0 means the required tables for all requested roles
were counted, 1 means partial or unsuccessful, and 2 means invalid input.
"""

import argparse
from datetime import datetime, timezone
import json
import multiprocessing
from pathlib import Path
import sqlite3
import sys
from time import monotonic


# The profile comes from the operator's explicit source selection. Missing role
# markers mean incomplete evidence. Never guess a role from filenames or data.
PROFILES = {
    "canonical": {
        "required": ("sessions", "messages", "chunks"),
        "optional": ("cas_registry", "chunk_occurrences", "vec_snowflake_m_v2_768_f32",
                     "vec_bge_small_384_f32"),
    },
    "scan_shard": {
        "required": ("atlas_manifest", "chunks", "vec_snowflake_m_v2_768_f32"),
        "optional": ("chunk_occurrences",),
    },
    "content_store": {
        "required": ("sessions", "messages"),
        "optional": (),
    },
}
TABLES = tuple(dict.fromkeys(
    table for profile in PROFILES.values()
    for names in profile.values() for table in names
))


def utc_now():
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds")


class SafeParser(argparse.ArgumentParser):
    def error(self, message):
        # Name the offending option (operator-script argument contract)
        # without echoing values: a value may be a local path and this
        # tool never prints paths.
        offenders = []
        for token in message.replace(":", " ").replace(",", " ").split():
            token = token.split("=")[0].rstrip(".,;")
            if token.startswith("-") and len(token) > 1 and token not in offenders:
                offenders.append(token)
        detail = " (" + ", ".join(sorted(offenders)) + ")" if offenders else ""
        self.exit(2, f"Invalid census arguments{detail}. See --help.\n")


def file_size(path):
    try:
        return path.stat().st_size
    except OSError:
        return None


def inspect_store(path, seconds, role):
    path = path.expanduser().resolve(strict=True)
    if not path.is_file():
        return {"status": "not_a_file"}
    deadline = monotonic() + seconds
    database = sqlite3.connect(
        path.as_uri() + "?mode=ro", uri=True, timeout=min(seconds, 1.0)
    )
    try:
        database.execute("PRAGMA query_only=ON")
        database.execute("PRAGMA trusted_schema=OFF")
        database.set_progress_handler(lambda: int(monotonic() >= deadline), 1000)
        database.execute("BEGIN")
        version = database.execute("PRAGMA schema_version").fetchone()[0]
        user_version = database.execute("PRAGMA user_version").fetchone()[0]
        profile = PROFILES[role]
        counts = {}
        dispositions = {}
        for table in TABLES:
            if table not in profile["required"] and table not in profile["optional"]:
                counts[table] = None
                dispositions[table] = "not_applicable"
                continue
            entry = database.execute(
                "SELECT type FROM sqlite_master WHERE name = ?", (table,)
            ).fetchone()
            if entry is None or entry[0] != "table":
                counts[table] = None
                dispositions[table] = (
                    "required_table_missing" if table in profile["required"]
                    else "optional_table_absent"
                )
                continue
            # The identifier comes exclusively from the constants above.
            counts[table] = database.execute(
                f'SELECT COUNT(*) FROM "{table}"'
            ).fetchone()[0]
            dispositions[table] = "counted"
        status = (
            "ok" if all(counts[x] is not None for x in profile["required"])
            else "required_table_missing"
        )
        if role == "scan_shard" and status == "ok":
            # Return only the count of required keys, never manifest values.
            matched = database.execute(
                'SELECT COUNT(DISTINCT "key") FROM atlas_manifest '
                'WHERE "key" IN (?, ?, ?) AND value IS NOT NULL '
                'AND length(value) > 0', ("shard", "embedding_model", "expected_chunks")
            ).fetchone()[0]
            if matched != 3:
                status = "shard_manifest_incomplete"
        if role == "scan_shard" and status == "ok":
            expected = database.execute(
                "SELECT value FROM atlas_manifest WHERE key = 'expected_chunks'"
            ).fetchone()[0]
            if str(expected) != str(counts["chunks"]):
                status = "shard_manifest_count_mismatch"
        database.rollback()
    finally:
        database.close()
    return {
        "status": status,
        "sqlite_schema_version": version,
        "sqlite_user_version": user_version,
        "snapshot_row_counts": counts,
        "table_dispositions": dispositions,
        "db_bytes_after_snapshot": file_size(path),
        "wal_bytes_after_snapshot": file_size(Path(str(path) + "-wal")),
    }


def count_and_send(path, seconds, role, sender):
    try:
        result = inspect_store(path, seconds, role)
    except FileNotFoundError:
        result = {"status": "file_missing"}
    except PermissionError:
        result = {"status": "access_denied"}
    except sqlite3.Error as error:
        # SQLite codes are optional on older Python builds. Do not print exception
        # messages: they may include paths or database-specific identifiers.
        code = getattr(error, "sqlite_errorcode", 0) or 0
        result = {"status": {
            5: "database_busy", 6: "database_locked", 9: "interrupted",
            14: "open_failed", 26: "unrecognized_database",
        }.get(code & 255, "sqlite_error")}
    except Exception:
        result = {"status": "unavailable"}
    try:
        sender.send(result)
    finally:
        sender.close()


def bounded_count(ctx, path, seconds, role):
    # A SQL progress callback alone cannot bound a single long sqlite opcode.
    receiver, sender = ctx.Pipe(duplex=False)
    worker = ctx.Process(target=count_and_send, args=(path, seconds, role, sender))
    try:
        worker.start()
        sender.close()
        if not receiver.poll(seconds):
            return {"status": "deadline_exceeded"}
        try:
            return receiver.recv()
        except EOFError:
            return {"status": "worker_failed"}
    finally:
        receiver.close()
        sender.close()
        if worker.pid is not None:
            worker.join(timeout=0.1)
            if worker.is_alive():
                worker.kill()
                worker.join(timeout=1)


def main():
    parser = SafeParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--canonical", type=Path,
                        default=Path("~/atlas/turso-atlas/data/atlas.db"))
    parser.add_argument("--shard", type=Path, action="append", default=[],
                        help="Explicit authorized scan shard, in local ledger order")
    parser.add_argument("--content-store", type=Path, action="append", default=[],
                        help="Explicit authorized content store, in local ledger order")
    parser.add_argument("--seconds-per-store", type=int, default=20,
                        help="Maximum worker wait per store, from 1 to 120 seconds")
    args = parser.parse_args()
    if not 1 <= args.seconds_per_store <= 120:
        parser.error("invalid deadline")
    ctx = multiprocessing.get_context("spawn")
    report = {
        "format": "atlas-census/v1",
        "started_at_utc": utc_now(),
        "route": "local_sqlite_readonly",
        "snapshot_scope": "one read transaction per store",
        "size_scope": "best-effort filesystem observations after each read transaction",
        "null_count": "see role-specific table_dispositions",
        "null_size": "unavailable or absent",
        "freshness": "ingest and published watermarks were not measured",
        "identity_scope": "per-store physical rows, including retained revisions or tombstones",
        "stores": [],
    }
    targets = [("canonical", args.canonical)]
    targets.extend(("scan_shard", path) for path in args.shard)
    targets.extend(("content_store", path) for path in args.content_store)
    for index, (role, path) in enumerate(targets):
        store = {"role": role, "ordinal": index, "started_at_utc": utc_now()}
        try:
            store.update(bounded_count(ctx, path, args.seconds_per_store, role))
        except Exception:
            store["status"] = "worker_failed"
        store["finished_at_utc"] = utc_now()
        report["stores"].append(store)
    report["finished_at_utc"] = utc_now()
    report["complete_required_counts"] = all(s["status"] == "ok" for s in report["stores"])
    print(json.dumps(report, indent=2))
    return 0 if report["complete_required_counts"] else 1


if __name__ == "__main__":
    sys.exit(main())

