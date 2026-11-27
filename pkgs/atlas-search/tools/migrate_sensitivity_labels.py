#!/usr/bin/env python3
"""Migrate rung-typed published sensitivity labels, never staging tiers.

Command line contract: migrate_sensitivity_labels.py DATABASE [--apply
--backup PATH] [--reverse]. With no --apply, inspect and print deterministic
JSON without writing. --backup is required with --apply and must name a new
file. --reverse maps canonical published labels back to legacy spellings.
"""

import argparse
from collections import Counter
import json
from pathlib import Path
import sqlite3
import sys


FORWARD = {
    "shareable": "public",
    "personal": "internal",
    "local-only": "private",
    "secret-bearing": "sensitive",
}
REVERSE = {canonical: legacy for legacy, canonical in FORWARD.items()}
TARGETS = {"chunks": "sensitivity", "session_labels": "sensitivity"}
STAGING_TABLES = {"_schema_version", "items", "chunks_raw", "notes", "kv"}


class SafeParser(argparse.ArgumentParser):
    def error(self, message):
        self.exit(2, "Invalid migration arguments. See --help.\n")


def parser():
    result = SafeParser(description=__doc__)
    result.add_argument("database", type=Path)
    result.add_argument("--apply", action="store_true")
    result.add_argument("--backup", type=Path)
    result.add_argument("--reverse", action="store_true")
    return result


def table_names(connection):
    return {
        row[0]
        for row in connection.execute(
            "SELECT name FROM sqlite_master WHERE type='table'"
        )
    }


def columns(connection, table):
    return {row[1] for row in connection.execute(f'PRAGMA table_info("{table}")')}


def table_counts(connection, table, column, mapping):
    rows = connection.execute(
        f'SELECT "{column}", COUNT(*) FROM "{table}" GROUP BY "{column}"'
    )
    counts = Counter()
    counts.update({("<NULL>" if value is None else str(value)): count for value, count in rows})
    return {
        "values": dict(sorted(counts.items())),
        "migratable": sum(counts.get(key, 0) for key in mapping),
        "already_canonical": sum(counts.get(value, 0) for value in mapping.values()),
    }


def inspect_schema(connection):
    names = table_names(connection)
    present = names.intersection(TARGETS)
    if present:
        if present != set(TARGETS):
            raise ValueError("published schema is missing a target table")
        for table, column in TARGETS.items():
            if column not in columns(connection, table):
                raise ValueError("published schema is missing a sensitivity column")
        return "published"

    if names.intersection(STAGING_TABLES):
        if not STAGING_TABLES.issubset(names):
            raise ValueError("staging schema is incomplete")
        if "sensitivity" not in columns(connection, "items"):
            raise ValueError("staging schema is missing its tier column")
        version = connection.execute("SELECT version FROM _schema_version").fetchall()
        if version != [(1,)]:
            raise ValueError("unsupported staging schema version")
        return "staging_v1"
    raise ValueError("database is neither a supported published store nor staging v1")


def make_backup(source, destination):
    if destination.exists():
        raise ValueError("backup path already exists")
    destination.parent.mkdir(parents=True, exist_ok=True)
    target = sqlite3.connect(destination)
    try:
        source.backup(target)
        target.commit()
    except Exception:
        target.close()
        destination.unlink(missing_ok=True)
        raise
    finally:
        if target:
            try:
                target.close()
            except sqlite3.Error:
                pass


def run(database_path, apply=False, backup_path=None, reverse=False):
    if database_path.is_symlink():
        raise ValueError("database path must not be a symlink")
    database_path = database_path.expanduser().resolve(strict=True)
    if not database_path.is_file():
        raise ValueError("database path is not a regular file")
    mapping = REVERSE if reverse else FORWARD

    if apply and backup_path is None:
        raise ValueError("--apply requires --backup")
    if not apply and backup_path is not None:
        raise ValueError("--backup requires --apply")

    connection = sqlite3.connect(
        database_path.as_uri() + ("?mode=rw" if apply else "?mode=ro"),
        uri=True,
        timeout=10,
    )
    try:
        connection.execute("PRAGMA query_only=ON" if not apply else "PRAGMA trusted_schema=OFF")
        profile = inspect_schema(connection)
        direction = "reverse" if reverse else "forward"
        if profile == "staging_v1":
            # The v1 sensitivity field is a physical corpus tier, not a rung.
            tiers = table_counts(connection, "items", "sensitivity", mapping)
            return {
                "profile": profile,
                "direction": direction,
                "action": "skipped_non_rung_tier_store",
                "staging_tiers": tiers["values"],
                "changed": 0,
            }

        before = {
            table: table_counts(connection, table, column, mapping)
            for table, column in TARGETS.items()
        }
        if apply:
            backup_path = backup_path.expanduser()
            if backup_path.resolve() == database_path:
                raise ValueError("backup path must differ from database path")
            make_backup(connection, backup_path)
            try:
                connection.execute("BEGIN IMMEDIATE")
                for table, column in TARGETS.items():
                    for old, new in mapping.items():
                        connection.execute(
                            f'UPDATE "{table}" SET "{column}"=? WHERE "{column}"=?',
                            (new, old),
                        )
                connection.commit()
            except Exception:
                connection.rollback()
                raise
        after = {
            table: table_counts(connection, table, column, mapping)
            for table, column in TARGETS.items()
        }
        return {
            "profile": profile,
            "direction": direction,
            "action": "applied" if apply else "dry_run",
            "backup_created": str(backup_path) if apply else None,
            "before": before,
            "after": after,
        }
    finally:
        connection.close()


def main(argv=None):
    args = parser().parse_args(argv)
    try:
        report = run(args.database, args.apply, args.backup, args.reverse)
    except (OSError, sqlite3.Error, ValueError) as error:
        print(f"Migration refused: {error}", file=sys.stderr)
        return 1
    print(json.dumps(report, sort_keys=True, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
