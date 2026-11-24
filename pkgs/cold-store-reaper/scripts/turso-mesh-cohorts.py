#!/usr/bin/env python3
"""Deterministic class-B cohort planner for the Turso backup mesh (#13456).

The mesh design (``doc/2026-11-23T16-34-18Z_design_backup_mesh/note.md``
§4) divides the general Turso databases (class B — everything except
the platform set) into 14 daily cohorts by a stable hash of the
database name; each database is snapshotted once per 14 days. This
script is the planner only: it takes an explicit runtime inventory
and a date and emits the due cohort as a machine-readable dry-run
plan. It performs no export, writes nothing to R2, touches no
credential, and schedules nothing — those are follow-up slices.

Rules (all deterministic; the same inventory and date always produce
the same plan, regardless of input ordering):

* Class A — ``fleet-queue``, ``fleet-platform``, ``atlas`` — is
  excluded from the rotation (design §4: weekly, double the mesh
  cadence, leg #13455). Excluded names are reported in the plan, never
  silently dropped.
* Cohort of a database: ``int(sha256(name)[:8]) % 14`` — SHA-256 of
  the UTF-8 name, first 8 bytes big-endian. A name hash is stable
  across runs, hosts, and Python versions, so a database never moves
  cohort when the inventory grows; new databases join automatically.
* Cohort due on a date: ``(date - 1970-01-01).days % 14`` — days
  since the Unix epoch, so the rotation is anchored to a fixed date
  and every cohort comes due exactly once per 14-day cycle.

Inventory format: one database name per line (blank lines ignored,
surrounding whitespace stripped, duplicates collapsed). Feed it from
``turso db list`` at run time — never hard-code the inventory here.

Usage:
    turso db list | python3 scripts/turso-mesh-cohorts.py --date 2026-11-24
    python3 scripts/turso-mesh-cohorts.py --inventory dbs.txt --all
    python3 scripts/turso-mesh-cohorts.py --inventory dbs.txt --cohort 3

Output is JSON on stdout. Exit 0 on success, 2 on a usage error
(bad date, cohort out of range, unreadable inventory).

Argument contract (#3352): the whole command line is declared in
``main()`` and parsed strictly with argparse before any inventory
read; an unknown flag, stray positional, or typo'd flag is a usage
error naming the offending argument, exit 2.
"""

import argparse
import hashlib
import json
import sys
from datetime import date

COHORTS = 14
EPOCH = date(1970, 1, 1)

CLASS_A = frozenset({"fleet-queue", "fleet-platform", "atlas"})


def cohort_of(name):
    """The 14-day cohort index of one database name (0..13)."""
    digest = hashlib.sha256(name.encode("utf-8")).digest()
    return int.from_bytes(digest[:8], "big") % COHORTS


def cohort_for_date(day):
    """The cohort due on a date: days since the epoch, mod 14."""
    return (day - EPOCH).days % COHORTS


def read_inventory(lines):
    """Parse one-name-per-line inventory text into a sorted unique list."""
    names = set()
    for line in lines:
        name = line.strip()
        if name:
            names.add(name)
    return sorted(names)


def split_classes(inventory):
    """Split an inventory into (class-B names, excluded class-A names)."""
    class_b = [name for name in inventory if name not in CLASS_A]
    excluded = sorted(name for name in inventory if name in CLASS_A)
    return class_b, excluded


def assign(inventory):
    """Assign every class-B database to exactly one cohort.

    Returns (cohorts, excluded) where cohorts is a list of 14 sorted
    name lists and excluded is the sorted class-A list.
    """
    class_b, excluded = split_classes(inventory)
    cohorts = [[] for _ in range(COHORTS)]
    for name in class_b:
        cohorts[cohort_of(name)].append(name)
    return [sorted(members) for members in cohorts], excluded


def plan(inventory, day):
    """The dry-run plan for one date: the due cohort and its databases."""
    cohorts, excluded = assign(inventory)
    due = cohort_for_date(day)
    return {
        "date": day.isoformat(),
        "cohort": due,
        "databases": cohorts[due],
        "excluded_class_a": excluded,
        "class_b_total": sum(len(members) for members in cohorts),
        "cohorts": COHORTS,
    }


def full_plan(inventory, day):
    """The plan for all 14 cohorts on one date (audit / --all output)."""
    cohorts, excluded = assign(inventory)
    return {
        "date": day.isoformat(),
        "cohort": cohort_for_date(day),
        "cohorts": [
            {"cohort": index, "databases": members}
            for index, members in enumerate(cohorts)
        ],
        "excluded_class_a": excluded,
        "class_b_total": sum(len(members) for members in cohorts),
        "cohort_count": COHORTS,
    }


def parse_date(text):
    """Parse a YYYY-MM-DD date, raising ValueError on anything else."""
    year, month, day = text.split("-")
    if len(year) != 4 or len(month) != 2 or len(day) != 2:
        raise ValueError("date must be YYYY-MM-DD, got %r" % text)
    return date(int(year), int(month), int(day))


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Plan the class-B Turso mesh cohort due on a date (#13456)."
    )
    parser.add_argument(
        "--inventory",
        metavar="FILE",
        help="one database name per line; default reads stdin",
    )
    parser.add_argument(
        "--date",
        default=date.today().isoformat(),
        help="the run date, YYYY-MM-DD (default: today)",
    )
    group = parser.add_mutually_exclusive_group()
    group.add_argument(
        "--all",
        action="store_true",
        help="emit all 14 cohorts instead of only the due one",
    )
    group.add_argument(
        "--cohort",
        type=int,
        metavar="N",
        help="emit cohort N (0..13) instead of the date's due cohort",
    )
    args = parser.parse_args(argv)

    try:
        day = parse_date(args.date)
    except ValueError as exc:
        print("turso-mesh-cohorts: %s" % exc, file=sys.stderr)
        return 2
    if args.cohort is not None and not 0 <= args.cohort < COHORTS:
        print(
            "turso-mesh-cohorts: cohort must be 0..%d, got %d"
            % (COHORTS - 1, args.cohort),
            file=sys.stderr,
        )
        return 2

    try:
        if args.inventory:
            with open(args.inventory, encoding="utf-8") as handle:
                inventory = read_inventory(handle)
        else:
            inventory = read_inventory(sys.stdin)
    except OSError as exc:
        print("turso-mesh-cohorts: cannot read inventory: %s" % exc, file=sys.stderr)
        return 2

    if args.all:
        result = full_plan(inventory, day)
    elif args.cohort is not None:
        cohorts, excluded = assign(inventory)
        result = {
            "date": day.isoformat(),
            "cohort": args.cohort,
            "databases": cohorts[args.cohort],
            "excluded_class_a": excluded,
            "class_b_total": sum(len(members) for members in cohorts),
            "cohorts": COHORTS,
        }
    else:
        result = plan(inventory, day)
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
