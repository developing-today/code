#!/usr/bin/env python3
"""Class-A stagger planner and export-manifest builder (#13455, slice 1).

The first foundational step of the backup-mesh class-A leg (design
``doc/2026-11-23T16-34-18Z_design_backup_mesh/note.md`` §§2-4, 9): the
three platform Turso databases (``fleet-queue``, ``fleet-platform``,
canonical ``atlas``) back up weekly, staggered across the week, and
every snapshot carries an export manifest of per-table row counts and
content hashes — the manifest is what restore verification checks
against.

Two pure, local pieces; no network, no credentials, no restic, no
schedule. Live Turso export wiring, the backup script, and the
watchdog are later slices (their scope is excluded on #13455 step 1).

Planner contract (``plan_class_a_week``):

- The input is a collection of database names. Names are processed in
  sorted order; each name takes the first free weekday starting from
  its stable slot (sha256 of a domain-separated name, mod 7, linear
  probe). The result is therefore deterministic and independent of
  input ordering, and every database lands on a distinct weekday.
- Duplicate names are a usage error; more than seven databases do not
  fit a week and are refused. Class A has exactly three members, so
  the probe never travels far — the design's "staggered across the
  week" with room to spare.

Manifest contract (``build_manifest``):

- The export artifact set is a rebuilt SQLite file plus the source
  dump bytes it was rebuilt from. The manifest names the database,
  the export timestamp (caller-supplied ISO-8601, so the builder
  stays pure), the sha256 of the dump bytes, and per-table row
  counts with per-table content hashes.
- The content hash of a table is the sha256 over the sorted
  serializations of its rows (``\\x1f``-joined columns, one row per
  line), so it is independent of physical row order: the same
  logical table hashes the same wherever it was rebuilt. Only user
  tables are listed (``sqlite_%`` internals excluded), ordered by
  name. The database file is opened read-only and never modified.

Usage:

    turso-mesh-class-a.py plan [--db NAME ...]
    turso-mesh-class-a.py manifest --db NAME --sqlite PATH --dump PATH \\
        --exported-at 2026-11-23T16:00:00Z [--out PATH]

``plan`` with no ``--db`` plans the three class-A databases.
``manifest`` prints the manifest JSON to stdout (or writes ``--out``).
"""

import argparse
import hashlib
import json
import sqlite3
import sys

CLASS_A_DATABASES = ("atlas", "fleet-platform", "fleet-queue")

WEEKDAYS = (
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
)

_PLANNER_DOMAIN = b"turso-mesh-class-a:v1:"


def _start_slot(name):
    digest = hashlib.sha256(_PLANNER_DOMAIN + name.encode("utf-8")).digest()
    return int.from_bytes(digest[:8], "big") % len(WEEKDAYS)


def plan_class_a_week(databases):
    """Assign each database a distinct weekday, deterministically.

    Returns a dict mapping database name to weekday name. The same
    set of names always yields the same mapping whatever order the
    names arrive in. Raises ValueError on duplicates or on more than
    seven databases (a week holds no more).
    """
    names = list(databases)
    if len(set(names)) != len(names):
        raise ValueError("duplicate database names in stagger plan")
    if len(names) > len(WEEKDAYS):
        raise ValueError(
            "cannot stagger %d databases across %d weekdays"
            % (len(names), len(WEEKDAYS))
        )
    taken = set()
    plan = {}
    for name in sorted(names):
        slot = _start_slot(name)
        for _ in range(len(WEEKDAYS)):
            if WEEKDAYS[slot] not in taken:
                taken.add(WEEKDAYS[slot])
                plan[name] = WEEKDAYS[slot]
                break
            slot = (slot + 1) % len(WEEKDAYS)
    return plan


def _quote_ident(name):
    return '"' + name.replace('"', '""') + '"'


def _serialize_value(value):
    if value is None:
        return "NULL"
    if isinstance(value, bytes):
        return "x'" + value.hex() + "'"
    if isinstance(value, float):
        return repr(value)
    return repr(value)


def _table_names(conn):
    return [
        row[0]
        for row in conn.execute(
            "SELECT name FROM sqlite_master WHERE type = 'table'"
            " AND name NOT LIKE 'sqlite_%' ORDER BY name"
        )
    ]


def _table_rows_and_hash(conn, table):
    cur = conn.execute("SELECT * FROM %s" % _quote_ident(table))
    serials = sorted(
        "\x1f".join(_serialize_value(v) for v in row) for row in cur.fetchall()
    )
    digest = hashlib.sha256()
    for serial in serials:
        digest.update(serial.encode("utf-8"))
        digest.update(b"\n")
    return len(serials), digest.hexdigest()


def build_manifest(database, sqlite_path, dump_bytes, exported_at):
    """Build the export manifest for one database snapshot.

    ``dump_bytes`` is the source dump the SQLite file was rebuilt
    from; ``exported_at`` is the export timestamp as ISO-8601 text.
    Returns a JSON-serializable dict with the database name, the
    timestamp, the dump sha256, and the per-table entries.
    """
    manifest = {
        "database": database,
        "exported_at": exported_at,
        "dump_sha256": hashlib.sha256(dump_bytes).hexdigest(),
        "tables": [],
    }
    conn = sqlite3.connect("file:%s?mode=ro" % sqlite_path, uri=True)
    try:
        for table in _table_names(conn):
            rows, content = _table_rows_and_hash(conn, table)
            manifest["tables"].append({"name": table, "rows": rows, "sha256": content})
    finally:
        conn.close()
    return manifest


def _cmd_plan(args):
    plan = plan_class_a_week(args.db or list(CLASS_A_DATABASES))
    print(json.dumps(plan, indent=2, sort_keys=True))
    return 0


def _cmd_manifest(args):
    with open(args.dump, "rb") as fh:
        dump_bytes = fh.read()
    manifest = build_manifest(args.db, args.sqlite, dump_bytes, args.exported_at)
    text = json.dumps(manifest, indent=2, sort_keys=True) + "\n"
    if args.out:
        with open(args.out, "w", encoding="utf-8") as fh:
            fh.write(text)
    else:
        sys.stdout.write(text)
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Class-A stagger planner and export-manifest builder (#13455)."
    )
    sub = parser.add_subparsers(dest="command", required=True)
    plan = sub.add_parser("plan", help="print the weekday stagger plan as JSON")
    plan.add_argument(
        "--db",
        action="append",
        default=[],
        help="database to plan (repeatable; default: the three class-A databases)",
    )
    manifest = sub.add_parser(
        "manifest", help="build the export manifest for one snapshot"
    )
    manifest.add_argument("--db", required=True, help="database name")
    manifest.add_argument("--sqlite", required=True, help="rebuilt SQLite file")
    manifest.add_argument("--dump", required=True, help="source dump file")
    manifest.add_argument(
        "--exported-at", required=True, help="export timestamp, ISO-8601"
    )
    manifest.add_argument("--out", default=None, help="write JSON here, not stdout")
    args = parser.parse_args(argv)
    try:
        if args.command == "plan":
            return _cmd_plan(args)
        return _cmd_manifest(args)
    except ValueError as exc:
        print("turso-mesh-class-a: %s" % exc, file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
