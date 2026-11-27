"""orchestration-pass retirement record (issue #2136, slice 1).

Slice 1 records where every orchestration-pass duty lives and who owns
it, before the pass is disabled. The duty table below is the assessment
#2136 asks for: the absorbed duties stay where they are, and each
residue duty (cross-queue priority calls, stuck-work escalation,
goal-measurement rollup) names its Tend owner -- operational residue to
stewardship-cycle, macro/alignment residue to program-tending.

--check verifies three things and nothing else:
  1. every duty has exactly one known owner;
  2. every residue duty is owned by stewardship-cycle or program-tending,
     with the class/owner split the issue orders;
  3. the schedule registry agrees with the RETIRED flag below
     (not retired -> the registry row still says enabled; retired ->
     it must say disabled, which is the later slice's job to flip).

Oracle: the duty names and the owner split come from the #2136 issue
text; the enabled/disabled ground truth is the orchestration-pass row
in doc/fleet-schedule-registry.md at the checked-out tree.

Usage: python3 -I scripts/orchestration_pass_retirement.py --check [--json] [--root PATH]
Contract (#3352): the whole command line is --check, --json and
--root; anything else is a usage error with exit 2. --root takes a
value in both --root PATH and --root=PATH forms; a valueless --root
is a usage error, never the default. The command is read-only: it
parses one Markdown table and exits 0 (clean) or 1 (violations).
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

RETIRED = False

REGISTRY_REL = Path("doc/fleet-schedule-registry.md")
PASS_NAME = "orchestration-pass"

KNOWN_OWNERS = (
    "stewardship-cycle",
    "program-tending",
    "fleet-monitor",
    "dispatcher",
    "queue-lanes",
)

# Tend roles per #2136: operational residue -> stewardship-cycle,
# macro/alignment residue -> program-tending.
RESIDUE_OWNERS = ("stewardship-cycle", "program-tending")

# status is "absorbed" (already lives with its owner) or "residue"
# (uniquely the pass's own; folded into Tend by this retirement).
DUTIES = (
    {
        "duty": "queue triage",
        "status": "absorbed",
        "owner": "fleet-monitor",
        "class": "operational",
    },
    {
        "duty": "assignment",
        "status": "absorbed",
        "owner": "dispatcher",
        "class": "operational",
    },
    {
        "duty": "lane self-claim",
        "status": "absorbed",
        "owner": "queue-lanes",
        "class": "operational",
    },
    {
        "duty": "hourly queue health review",
        "status": "absorbed",
        "owner": "stewardship-cycle",
        "class": "operational",
    },
    {
        "duty": "daily roadmap pass look-back",
        "status": "absorbed",
        "owner": "program-tending",
        "class": "macro",
    },
    {
        "duty": "cross-queue priority calls",
        "status": "residue",
        "owner": "stewardship-cycle",
        "class": "operational",
    },
    {
        "duty": "stuck-work escalation",
        "status": "residue",
        "owner": "stewardship-cycle",
        "class": "operational",
    },
    {
        "duty": "hourly goal measurement",
        "status": "residue",
        "owner": "stewardship-cycle",
        "class": "operational",
    },
    {
        "duty": "goal-measurement rollup vs roadmaps and priorities",
        "status": "residue",
        "owner": "program-tending",
        "class": "macro",
    },
)


def _default_root() -> Path:
    return Path(__file__).resolve().parents[1]


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Check the orchestration-pass retirement record (#2136)."
    )
    parser.add_argument("--check", action="store_true", help="run the record check")
    parser.add_argument("--json", action="store_true", help="print the report as JSON")
    parser.add_argument("--root", default=str(_default_root()), help="repository root")
    args = parser.parse_args(argv)
    if not args.check:
        parser.print_usage(sys.stderr)
        parser.exit(2, "error: --check is required\n")
    return args


def registry_state(root: Path) -> str | None:
    """The enabled/disabled cell of the orchestration-pass registry row."""
    text = (root / REGISTRY_REL).read_text()
    for line in text.splitlines():
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if cells and cells[0].strip("`") == PASS_NAME:
            return cells[3] if len(cells) > 3 else None
    return None


def check(root: Path) -> dict:
    violations: list[str] = []
    for entry in DUTIES:
        owner = entry.get("owner")
        if owner not in KNOWN_OWNERS:
            violations.append(
                f"duty {entry.get('duty')!r} has no known owner: {owner!r}"
            )
            continue
        if entry.get("status") == "residue":
            if owner not in RESIDUE_OWNERS:
                violations.append(
                    f"residue duty {entry['duty']!r} is not Tend-owned: {owner!r}"
                )
            elif (entry.get("class") == "operational") != (
                owner == "stewardship-cycle"
            ):
                violations.append(
                    f"residue duty {entry['duty']!r}: class/owner split "
                    f"violated (class={entry.get('class')!r} owner={owner!r})"
                )
    state = registry_state(root)
    if state is None:
        violations.append("orchestration-pass row missing from the registry")
    elif RETIRED and state != "disabled":
        violations.append(f"record says retired but the registry row is {state!r}")
    elif not RETIRED and state != "enabled":
        violations.append(f"record says active but the registry row is {state!r}")
    residue = sorted(e["duty"] for e in DUTIES if e.get("status") == "residue")
    return {
        "retired": RETIRED,
        "registry_state": state,
        "duties": len(DUTIES),
        "residue": residue,
        "violations": violations,
    }


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    report = check(Path(args.root))
    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        for duty in sorted(
            DUTIES, key=lambda e: (e.get("status") != "residue", e["duty"])
        ):
            print(
                f"{duty['status']:8} {duty['class']:11} "
                f"{duty['duty']} -> {duty['owner']}"
            )
        print(f"registry: {PASS_NAME} is {report['registry_state']}")
        for violation in report["violations"]:
            print(f"VIOLATION: {violation}")
    return 1 if report["violations"] else 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
