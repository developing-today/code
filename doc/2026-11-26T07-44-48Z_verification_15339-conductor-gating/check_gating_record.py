#!/usr/bin/env python3
"""Machine check for the #15339 gating verification record.

Command-line contract (operator script argument contract, #3352): this
script takes NO arguments. Any argument is a usage error (exit 2),
reported before any file is read. Paths resolve relative to this
file's location in the repo, never the caller's cwd.

Oracle: the record beside this file and `doc/fleet-schedule-registry.md`
must agree —

1. the registry records `openchamber-dispatcher`,
   `openchamber-babysitter`, and `openchamber-conductor` as disabled;
2. the record's gap table states gap 1 closed (PR #8437), gap 2 OPEN
   (#2072), gap 3 closed (PR #11158), gap 4 OPEN / unverified;
3. the record carries the gating verdict that the port must not
   proceed while gaps 2 and 4 are open.

If a later change closes a gap or changes a job state, this check
fails until the record is updated in the same pass — that failure is
the point: the record must not silently go stale.

Exit 0: record and registry agree. Exit 1: a disagreement (each is
printed). Exit 2: usage error.
"""

import sys
from pathlib import Path

JOBS = ("openchamber-dispatcher", "openchamber-babysitter", "openchamber-conductor")

EXPECTED_GAP_ROWS = (
    "| 1 — per-run history | **closed** |",
    "| 2 — token accounting | **OPEN** |",
    "| 3 — failure alerting | **closed** |",
    "| 4 — server residency | **OPEN / unverified** |",
)

VERDICT_MARKER = "The #2174 gate is not satisfied: gaps 2 and 4 remain open."


def registry_row(registry_text: str, job: str) -> str | None:
    for line in registry_text.splitlines():
        if line.startswith("| `") and f"`{job}`" in line.split("|")[1]:
            return line
    return None


def main(argv: list[str]) -> int:
    if argv:
        print(f"usage error: check_gating_record.py takes no arguments, got {argv!r}")
        return 2

    here = Path(__file__).resolve().parent
    repo = here.parent.parent
    record = (here / "2026-11-26T07-44-48Z_verification_15339-conductor-gating.md").read_text()
    registry = (repo / "doc" / "fleet-schedule-registry.md").read_text()

    failures: list[str] = []

    for job in JOBS:
        row = registry_row(registry, job)
        if row is None:
            failures.append(f"registry has no Surface 1 row for {job}")
        elif "| disabled |" not in row:
            failures.append(f"registry row for {job} is not recorded disabled: {row.strip()[:120]}")

    for expected in EXPECTED_GAP_ROWS:
        if expected not in record:
            failures.append(f"record gap table is missing or changed: {expected}")

    if VERDICT_MARKER not in record:
        failures.append("record is missing the gating verdict marker")

    if failures:
        for failure in failures:
            print(f"FAIL: {failure}")
        return 1

    print("ok: registry records all three conductor jobs disabled; "
          "record gap states (1 closed, 2 open, 3 closed, 4 open) and verdict agree")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
