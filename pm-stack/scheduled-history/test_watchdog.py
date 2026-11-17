#!/usr/bin/env python3
"""test_watchdog.py — checks for watchdog.py (#2174 Gap 3).

Plain-script style mirroring test_collect.py: run
``python3 test_watchdog.py``; every check prints into the PASS list
and the summary line reports n/n.
"""

import os, sys
from datetime import datetime, timedelta, timezone

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import watchdog  # noqa: E402

PASS = []


def check(name, ok):
    PASS.append((name, bool(ok)))
    print(("ok  " if ok else "FAIL"), name)


NOW = datetime(2026, 11, 17, 12, 0, 0, tzinfo=timezone.utc)


def iso(dt):
    return dt.isoformat()


def row(task_id, when, outcome="success", terminal=True, first_seen=None, enabled=True):
    return {
        "run_key": f"{task_id}@{when}",
        "task_id": task_id,
        "task_name": f"name-{task_id}",
        "scheduled_for": iso(when),
        "started_at": iso(when),
        "finished_at": iso(when + timedelta(minutes=5)) if terminal else None,
        "outcome": outcome,
        "outcome_terminal": terminal,
        "enabled": enabled,
        "first_seen_at": iso(first_seen or when),
        "last_seen_at": iso(when),
    }


def hourly(task_id, last_age_hours, outcome="success", runs=6, first_seen_age_hours=None):
    latest = NOW - timedelta(hours=last_age_hours)
    rows = {}
    for i in range(runs):
        when = latest - timedelta(hours=(runs - 1 - i))
        r = row(task_id, when, outcome=outcome if i == runs - 1 else "success",
                first_seen=NOW - timedelta(hours=first_seen_age_hours) if first_seen_age_hours else None)
        rows[r["run_key"]] = r
    return rows


def conditions(report, task_id):
    return [a["condition"] for a in report["alerts"] if a["task_id"] == task_id]


# 1. healthy task: hourly cadence, latest run 20 minutes ago -> no alerts
report = watchdog.run_check(hourly("t-healthy", 1 / 3), NOW)
check("healthy: no alerts", conditions(report, "t-healthy") == [])
check("healthy: listed healthy", "t-healthy" in report["healthy_task_ids"])

# 2. missing run: hourly cadence, latest run 90 minutes ago -> missing_run only
report = watchdog.run_check(hourly("t-missing", 1.5), NOW)
check("missing: missing_run fires", conditions(report, "t-missing") == ["missing_run"])
ev = report["alerts"][0]["evidence"]
check("missing: evidence carries window + source",
      ev["window_seconds"] == 3600.0 and ev["window_source"] == "observed-schedule")

# 3. stale: hourly cadence, latest run 5 hours ago -> stale (past 2x threshold)
report = watchdog.run_check(hourly("t-stale", 5), NOW)
check("stale: stale fires", conditions(report, "t-stale") == ["stale"])
check("stale: threshold named", report["alerts"][0]["evidence"]["stale_after_seconds"] == 7200.0)

# 4. failed latest: latest terminal outcome error -> failed_latest, even when fresh
report = watchdog.run_check(hourly("t-failed", 0.25, outcome="error"), NOW)
check("failed: failed_latest fires", conditions(report, "t-failed") == ["failed_latest"])
check("failed: outcome in evidence",
      report["alerts"][0]["evidence"]["latest_terminal_outcome"] == "error")

# 5. first window: brand-new task, one run seen 10 minutes ago inside its
#    first (declared) window -> no false alert
rows = {"k": row("t-new", NOW - timedelta(minutes=10))}
report = watchdog.run_check(rows, NOW, declared_windows={"t-new": 3600})
check("first-window: no alert inside first window", conditions(report, "t-new") == [])

# 6. declared window overrides the observed one: hourly history but a
#    declared daily window -> a 90-minute-old latest run is healthy
report = watchdog.run_check(hourly("t-declared", 1.5), NOW, declared_windows={"t-declared": 86400})
check("declared: window source wins", conditions(report, "t-declared") == [])
report2 = watchdog.run_check(hourly("t-declared", 1.5), NOW)
check("declared: same rows alert without the declaration",
      conditions(report2, "t-declared") == ["missing_run"])

# 7. disabled task never alerts, however old its latest run
rows = hourly("t-off", 72)
for r in rows.values():
    r["enabled"] = False
report = watchdog.run_check(rows, NOW)
check("disabled: skipped", conditions(report, "t-off") == [])

# 8. in-flight latest run (non-terminal) suppresses missing/stale
rows = hourly("t-running", 5)
latest_key = max(rows)
rows[latest_key]["outcome"] = "running"
rows[latest_key]["outcome_terminal"] = False
report = watchdog.run_check(rows, NOW)
check("in-flight: no missing/stale while a run is in flight",
      conditions(report, "t-running") == [])

print(f"\n{sum(1 for _, ok in PASS if ok)}/{len(PASS)} checks passed")
sys.exit(0 if all(ok for _, ok in PASS) else 1)
