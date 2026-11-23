#!/usr/bin/env python3
"""fleet-sync-probe.py — judge one fork's sync-upstream freshness (#12235).

Reads the sync-upstream workflow runs for one fleet fork (as returned by
``gh api repos/<owner>/<repo>/actions/workflows/sync-upstream.yml/runs``,
either the ``{"workflow_runs": [...]}`` object or a bare list) and decides
whether that fork's upstream sync is healthy.

Verdict FAIL (exit 1) when either half of #12235 step 4's "visible within
a day" holds:

- the latest run overall failed (conclusion failure/cancelled/timed_out)
  — the fork's sync is currently broken, e.g. a merge conflict like the
  2026-11-21 org-fork failure (run 44900082037); or
- there is no successful *scheduled* run within --max-age-hours — the
  schedule never fired or was dropped, the silent gap 2026-11-06..21 in
  both forks. Dispatch successes do not satisfy this half: a manual
  dispatch proves the fork is level, not that its schedule works.

Verdict PASS (exit 0) otherwise. A run still in progress (conclusion
null) is neither a failure nor a success.

Oracle: the GitHub Actions runs API for the fork (event, conclusion,
created_at of each run, newest first) plus the wall clock. The contract
tests in .github/scripts/fleet-sync-probe.test.sh feed canned runs
shapes, including the real 2026-11-22 shapes of both forks.

Argument contract (#3352): the whole command line is declared below
and parsed strictly by argparse — an unknown flag, a stray positional,
or a missing required option is a usage error (exit 2). --opt=value and
--opt value are equivalent; a present-but-valueless option is a usage
error. A --now value pins the clock for tests; without it the script
uses the current UTC time.

Exit codes: 0 pass, 1 fail (or unreadable/invalid runs input, named on
stderr), 2 usage error.
"""

import argparse
import json
import sys
from datetime import datetime, timezone

FAILURE_CONCLUSIONS = {"failure", "cancelled", "timed_out"}


def parse_time(value, what):
    """Parse an ISO 8601 timestamp (trailing Z allowed) to an aware datetime."""
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        print(f"fleet-sync-probe: {what} {value!r} is not an ISO 8601 timestamp",
              file=sys.stderr)
        sys.exit(1)


def load_runs(path):
    try:
        with open(path, encoding="utf-8") as fh:
            doc = json.load(fh)
    except OSError as exc:
        print(f"fleet-sync-probe: cannot read runs file: {exc}", file=sys.stderr)
        sys.exit(1)
    except json.JSONDecodeError as exc:
        print(f"fleet-sync-probe: invalid JSON in runs file: {exc}", file=sys.stderr)
        sys.exit(1)
    if isinstance(doc, dict):
        runs = doc.get("workflow_runs")
    else:
        runs = doc
    if not isinstance(runs, list):
        print("fleet-sync-probe: runs file must hold a workflow_runs list "
              "or a bare list of runs", file=sys.stderr)
        sys.exit(1)
    return runs


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Judge one fleet fork's sync-upstream freshness (#12235).")
    parser.add_argument("--repo", required=True,
                        help="fork under test, e.g. developing-today/fleet")
    parser.add_argument("--runs-json", required=True,
                        help="path to a runs-API JSON file (workflow_runs "
                             "object or bare list, newest first)")
    parser.add_argument("--max-age-hours", type=float, default=30.0,
                        help="a scheduled success older than this is stale "
                             "(default 30: sync cron 04:23 UTC plus the "
                             "~8h load delay observed 2026-11-05..22)")
    parser.add_argument("--now", default=None,
                        help="ISO 8601 clock override (tests); default: now UTC")
    args = parser.parse_args(argv)

    now = parse_time(args.now, "--now") if args.now else datetime.now(timezone.utc)
    runs = load_runs(args.runs_json)

    if not runs:
        print(f"repo={args.repo} verdict=fail "
              f"detail=no sync-upstream runs at all")
        return 1

    latest = runs[0]
    latest_id = latest.get("id", "?")
    latest_event = latest.get("event", "?")
    latest_conclusion = latest.get("conclusion")
    latest_created = latest.get("created_at", "?")

    schedule_successes = [r for r in runs
                          if r.get("event") == "schedule"
                          and r.get("conclusion") == "success"]
    if schedule_successes:
        last_ok = parse_time(schedule_successes[0].get("created_at", ""),
                             "created_at")
        age_hours = (now - last_ok).total_seconds() / 3600.0
        schedule_detail = (f"last scheduled success "
                           f"{schedule_successes[0].get('created_at')} "
                           f"({age_hours:.1f}h ago)")
    else:
        age_hours = None
        schedule_detail = "no successful scheduled sync ever"

    if latest_conclusion in FAILURE_CONCLUSIONS:
        print(f"repo={args.repo} verdict=fail "
              f"detail=latest sync failed: run {latest_id} "
              f"({latest_event}, {latest_conclusion}, {latest_created}); "
              f"{schedule_detail}")
        return 1
    if age_hours is None or age_hours > args.max_age_hours:
        print(f"repo={args.repo} verdict=fail "
              f"detail=no successful scheduled sync within "
              f"{args.max_age_hours:g}h: {schedule_detail}; latest run "
              f"{latest_id} ({latest_event}, "
              f"{latest_conclusion or latest.get('status', '?')}, "
              f"{latest_created})")
        return 1

    print(f"repo={args.repo} verdict=pass "
          f"detail={schedule_detail}; latest run {latest_id} "
          f"({latest_event}, {latest_conclusion or latest.get('status', '?')}, "
          f"{latest_created})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
