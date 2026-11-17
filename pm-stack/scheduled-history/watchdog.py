#!/usr/bin/env python3
"""watchdog.py — Gap 3 watchdog over OpenChamber scheduled-task run history (#2174).

Consumes the Gap 1 store written by collect.py (one durable row per
run) and flags, per scheduled task:

- ``missing_run``: no run was scheduled within the task's expected
  cadence window — a run should have appeared by now and did not.
- ``stale``: the newest run is older than the stale threshold
  (``--stale-multiplier`` x the window, default 2x) and nothing is in
  flight — silence well beyond a single missed beat.
- ``failed_latest``: the most recent terminal outcome is an error
  class (error / failed / failure / timeout / timed_out).

Expected-window source, stated per task in every alert: a declared
per-task window from ``--windows`` (JSON object mapping task id to
window seconds) when present, otherwise the observed schedule — the
minimum positive interval between consecutive ``scheduled_for``
instants in the task's own history (window_source
``observed-schedule``). A task whose history is younger than one
window is inside its first window and never alerts for missing/stale:
there has not been time for a second run to be due.

Alert path: the check prints one JSON document
(``scheduled-history-watchdog/v1``) on stdout and exits 1 when any
alert fires, 0 when all tasks are healthy. The fleet-watchdog cron
consumes that document — the alert lands wherever the cron files its
JSON findings; this script itself sends nothing and changes no
schedule.

Run it: ``python3 watchdog.py check [--store PATH] [--windows FILE]
[--stale-multiplier N] [--json]`` (output is JSON by default; the
exit code is the machine-readable signal either way).
"""

import argparse, json, os, sys
from datetime import datetime, timezone

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import collect  # noqa: E402  (same directory: store shape + status sets)

ERROR_OUTCOMES = {"error", "failed", "failure", "timeout", "timed_out"}
SCHEMA = "scheduled-history-watchdog/v1"


def parse_instant(value):
    """ISO string -> aware datetime, or None. Accepts a trailing Z."""
    if not value or not isinstance(value, str):
        return None
    text = value.strip()
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    try:
        parsed = datetime.fromisoformat(text)
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed


def run_instant(row):
    return (
        parse_instant(row.get("scheduled_for"))
        or parse_instant(row.get("started_at"))
        or parse_instant(row.get("first_seen_at"))
    )


def derive_window(runs):
    """Observed cadence: min positive gap between consecutive run instants."""
    instants = sorted(
        {instant for instant in (run_instant(r) for r in runs) if instant}
    )
    gaps = [
        (later - earlier).total_seconds()
        for earlier, later in zip(instants, instants[1:])
        if (later - earlier).total_seconds() > 0
    ]
    return min(gaps) if gaps else None


def check_task(task_id, runs, now, declared_windows, stale_multiplier):
    """Return (alerts, facts) for one task's rows."""
    facts = {
        "task_id": task_id,
        "task_name": None,
        "window_seconds": None,
        "window_source": None,
    }
    alerts = []
    if not runs:
        return alerts, facts
    facts["task_name"] = runs[-1].get("task_name")
    latest = max(
        runs, key=lambda r: run_instant(r) or datetime.min.replace(tzinfo=timezone.utc)
    )
    if latest.get("enabled") is False:
        facts["skipped"] = "disabled"
        return alerts, facts

    if task_id in declared_windows:
        window = float(declared_windows[task_id])
        source = "declared"
    else:
        window = derive_window(runs)
        source = "observed-schedule"
    facts["window_seconds"] = window
    facts["window_source"] = source if window else None

    latest_instant = run_instant(latest)
    age = (now - latest_instant).total_seconds() if latest_instant else None
    facts["latest_scheduled_for"] = latest.get("scheduled_for")
    facts["latest_outcome"] = latest.get("outcome")
    facts["age_seconds"] = age

    terminal_rows = [r for r in runs if r.get("outcome_terminal")]
    latest_terminal = None
    if terminal_rows:
        latest_terminal = max(
            terminal_rows,
            key=lambda r: run_instant(r) or datetime.min.replace(tzinfo=timezone.utc),
        )
    if (
        latest_terminal
        and (latest_terminal.get("outcome") or "").lower() in ERROR_OUTCOMES
    ):
        alerts.append(
            {
                "task_id": task_id,
                "task_name": facts["task_name"],
                "condition": "failed_latest",
                "evidence": {
                    "latest_terminal_scheduled_for": latest_terminal.get(
                        "scheduled_for"
                    ),
                    "latest_terminal_outcome": latest_terminal.get("outcome"),
                    "finished_at": latest_terminal.get("finished_at"),
                },
            }
        )

    if window and age is not None:
        first_seen_candidates = [parse_instant(r.get("first_seen_at")) for r in runs]
        first_seen = min((t for t in first_seen_candidates if t), default=None)
        in_first_window = (
            first_seen is not None and (now - first_seen).total_seconds() < window
        )
        in_flight = not latest.get("outcome_terminal")
        stale_after = window * stale_multiplier
        facts["stale_after_seconds"] = stale_after
        if not in_first_window and not in_flight:
            if age > stale_after:
                alerts.append(
                    {
                        "task_id": task_id,
                        "task_name": facts["task_name"],
                        "condition": "stale",
                        "evidence": {
                            "latest_scheduled_for": latest.get("scheduled_for"),
                            "latest_outcome": latest.get("outcome"),
                            "age_seconds": age,
                            "window_seconds": window,
                            "window_source": source,
                            "stale_after_seconds": stale_after,
                        },
                    }
                )
            elif age > window:
                alerts.append(
                    {
                        "task_id": task_id,
                        "task_name": facts["task_name"],
                        "condition": "missing_run",
                        "evidence": {
                            "latest_scheduled_for": latest.get("scheduled_for"),
                            "latest_outcome": latest.get("outcome"),
                            "age_seconds": age,
                            "window_seconds": window,
                            "window_source": source,
                            "stale_after_seconds": stale_after,
                        },
                    }
                )
    return alerts, facts


def run_check(rows, now, declared_windows=None, stale_multiplier=2.0):
    declared_windows = declared_windows or {}
    by_task = {}
    for row in rows.values():
        by_task.setdefault(row.get("task_id"), []).append(row)
    alerts = []
    facts = []
    for task_id in sorted(by_task, key=lambda t: str(t)):
        task_alerts, task_facts = check_task(
            task_id, by_task[task_id], now, declared_windows, stale_multiplier
        )
        alerts.extend(task_alerts)
        facts.append(task_facts)
    alerted = {a["task_id"] for a in alerts}
    return {
        "schema": SCHEMA,
        "checked_at": now.isoformat(),
        "tasks_checked": len(by_task),
        "alerts": alerts,
        "healthy_task_ids": [
            t for t in sorted(by_task, key=lambda t: str(t)) if t not in alerted
        ],
        "tasks": facts,
    }


def cmd_check(args):
    rows = collect.load_store(args.store)
    declared = {}
    if args.windows:
        with open(args.windows) as f:
            declared = json.load(f)
    now = parse_instant(args.now) if args.now else datetime.now(timezone.utc)
    report = run_check(rows, now, declared, args.stale_multiplier)
    report["store"] = args.store
    print(json.dumps(report, indent=2, sort_keys=True))
    return 1 if report["alerts"] else 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    check = sub.add_parser("check", help="evaluate the store and print alerts as JSON")
    check.add_argument("--store", default=collect.DEFAULT_STORE)
    check.add_argument(
        "--windows",
        default=None,
        help="JSON file mapping task id -> declared window seconds",
    )
    check.add_argument("--stale-multiplier", type=float, default=2.0)
    check.add_argument(
        "--now", default=None, help="override the evaluation instant (ISO; tests)"
    )
    check.set_defaults(func=cmd_check)
    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
