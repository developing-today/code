#!/usr/bin/env python3
"""Report-only weekly monitor adapter for the native epic closure gate.

Usage: python3 -I pm-stack/fleet-monitor/epic_close_check.py --root owner/repo#123 [--root owner/repo#456]

Runs the aggregator's read-only complete-subtree reporter and emits one JSON
record per epic. A blocked epic is a finding, not a process failure: GitHub
does not hard-block closes. Fetch/parse failures exit non-zero and never appear
as an empty or clear report.
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path


ROOT_RE = re.compile(r"^[^/#]+/[^/#]+#[1-9][0-9]*$")


def parse_args(argv):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        action="append",
        required=True,
        help="program or maintenance epic ref (owner/repo#number); repeatable",
    )
    args = parser.parse_args(argv)
    invalid = [root for root in args.root if not ROOT_RE.fullmatch(root)]
    if invalid:
        parser.error(f"invalid --root ref: {invalid[0]}")
    if len(set(args.root)) != len(args.root):
        parser.error("duplicate --root refs are not allowed")
    return args


def monitor_records(payload):
    """Turn the gate's machine report into one weekly-monitor record per root."""
    reports = payload.get("reports")
    if not isinstance(reports, list):
        raise ValueError("closure gate output omitted reports")
    records = []
    for report in reports:
        if not isinstance(report, dict) or not isinstance(report.get("root"), str):
            raise ValueError("closure gate output contains a malformed report")
        complete = report.get("factsComplete")
        closable = report.get("closable")
        root_state = report.get("rootState")
        blockers = report.get("blockers")
        if not isinstance(complete, bool) or not isinstance(closable, bool) or not isinstance(blockers, list):
            raise ValueError(f"closure gate output is incomplete for {report['root']}")
        if root_state not in ("OPEN", "CLOSED"):
            raise ValueError(f"closure gate output omitted root state for {report['root']}")
        status = "unavailable" if not complete else "clear" if closable else "blocked"
        unfinished_children = sorted(
            {
                ref
                for blocker in blockers
                if isinstance(blocker, dict)
                and blocker.get("condition") == "unfinished_children_without_reparent_destination"
                for ref in blocker.get("refs", [])
                if isinstance(ref, str)
            }
        )
        records.append(
            {
                "check": "epic_closure_gate",
                "root": report["root"],
                "status": status,
                "root_state": root_state,
                "descendants": report.get("descendants"),
                "facts_complete": complete,
                "closable": closable,
                "blockers": blockers,
                "closed_with_unfinished_children": root_state == "CLOSED" and bool(unfinished_children),
                "skipped_reparent_refs": unfinished_children if root_state == "CLOSED" else [],
            }
        )
    return records


def run_gate(roots):
    repo = Path(__file__).resolve().parents[2]
    aggregator = repo / "pm-stack" / "aggregator"
    command = ["bun", "run", "scripts/epic-closure-gate.ts"]
    for root in roots:
        command.extend(("--root", root))
    result = subprocess.run(command, cwd=aggregator, capture_output=True, text=True, check=False)
    if result.returncode != 0:
        detail = result.stderr.strip() or f"closure gate exited {result.returncode}"
        raise RuntimeError(detail)
    try:
        payload = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError("closure gate returned invalid JSON") from exc
    return monitor_records(payload)


def main(argv=None):
    # Parse and validate the complete command line before any subprocess call.
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        for record in run_gate(args.root):
            print(json.dumps(record, sort_keys=True))
    except Exception as exc:  # report source/runner failures, never fake a clear gate
        print(f"epic_close_check: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
