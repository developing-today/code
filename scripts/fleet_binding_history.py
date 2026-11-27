#!/usr/bin/env python3
"""Durable queue-bound launch/retry binding history (#2140 Part 3).

The native ledger upserts its session binding and native sources are
pruned, so a raw latest-binding export cannot prove complete retry cost.
This collector monotonically merges offline task/ledger/session/event
exports (the same four arrays scripts/fleet_completion_usage.py consumes)
into a durable history file that outlives pruning and overwrites:

  record    Merge one export into the history file. Never deletes: session
            bindings union in first-seen order, session input counters keep
            the latest non-null value (null never overwrites known), and a
            task first seen already terminal -- or gaining a new session
            after completing -- is marked history_gap (earlier retries may
            predate the collector). The gap flag is sticky by design.
  snapshot  Emit a fleet_completion_usage.py input document for [start,end)
            assembled from retained history. binding_history_complete is
            true only when no completed-in-window task carries history_gap;
            per-session null handling stays with the usage report.
  checkin   Render the daily check-in markdown: per-queue completed-cohort
            usage from the usage report plus explicit source coverage and
            retry-history completeness. Ratios stay WITHHELD unless the
            inputs are complete; this never claims window spend or a
            cadence before/after result.

Argument contract (#3352): the whole command line is declared below and
parsed strictly by argparse before any file I/O. An unknown flag, a stray
positional or a valueless option is a usage error with exit 2.
`--opt=value` and `--opt value` are the same. Read-only against live
systems: this tool never queries a broker, never touches a schedule, and
only writes the history/snapshot files it is told to.

Exit codes: 0 done; 2 usage error or unreadable/invalid input.
"""

import argparse
import json
from datetime import datetime, timezone
from pathlib import Path
import sys

VERSION = 1
TERMINAL = {"done"}
COMPLETION_KINDS = {"completed", "staleness_autocomplete"}
SOURCE_KEYS = ("tasks", "events", "ledger", "sessions")


def timestamp(value):
    if not isinstance(value, str):
        raise ValueError("timestamp must be an ISO string with timezone")
    result = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if result.utcoffset() is None:
        raise ValueError("timestamp must include timezone")
    return result


def now_iso():
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def blank_history():
    return {
        "version": VERSION,
        "record_calls": 0,
        "first_record_at": None,
        "last_record_at": None,
        "sources": [],
        "tasks": {},
        "sessions": {},
    }


def load_history(path):
    if not path.exists():
        return blank_history()
    try:
        data = json.loads(path.read_text())
    except (OSError, ValueError) as error:
        raise ValueError(f"history file {path} is unreadable: {error}")
    if not isinstance(data, dict) or data.get("version") != VERSION:
        raise ValueError(f"history file {path} has unsupported version")
    for key in ("tasks", "sessions"):
        if not isinstance(data.get(key), dict):
            raise ValueError(f"history file {path}: {key} must be an object")
    if not isinstance(data.get("sources"), list):
        raise ValueError(f"history file {path}: sources must be an array")
    return data


def load_export(path):
    try:
        data = json.loads(path.read_text())
    except (OSError, ValueError) as error:
        raise ValueError(f"export file {path} is unreadable: {error}")
    if not isinstance(data, dict):
        raise ValueError(f"export file {path} must hold one object")
    for key in SOURCE_KEYS:
        if not isinstance(data.get(key), list):
            raise ValueError(f"export file {path}: {key} must be an array")
    return data


def task_sessions(task):
    """Session ids bound on the task row itself (progress or normalized)."""
    found = []
    progress = task.get("progress") or {}
    if isinstance(progress, str):
        try:
            progress = json.loads(progress)
        except ValueError:
            raise ValueError(f"task {task.get('id')}: progress is not an object")
    if not isinstance(progress, dict):
        raise ValueError(f"task {task.get('id')}: progress must be an object")
    for sid in (progress.get("session_id"), task.get("session_id")):
        if sid:
            if not isinstance(sid, str):
                raise ValueError(f"task {task.get('id')}: session_id must be a string")
            if sid not in found:
                found.append(sid)
    return found


def record(history, export, at):
    """Monotonically merge one export. Mutates and returns history."""
    counts = {key: len(export[key]) for key in SOURCE_KEYS}

    for task in export["tasks"]:
        task_id = task.get("id")
        if type(task_id) is not int:
            raise ValueError("task IDs must be integers")
        queue = task.get("queue")
        if not isinstance(queue, str) or not queue:
            raise ValueError(f"task {task_id}: queue must be a nonempty string")
        status = task.get("status")
        if not isinstance(status, str) or not status:
            raise ValueError(f"task {task_id}: status must be a nonempty string")
        key = str(task_id)
        known = history["tasks"].get(key)
        if known is not None and known["queue"] != queue:
            raise ValueError(f"task {task_id}: conflicting queue across exports")
        if known is None:
            known = {
                "queue": queue,
                "status": status,
                "sessions": [],
                "completed_at": None,
                "history_gap": False,
                "first_seen_at": at,
            }
            history["tasks"][key] = known
            if status in TERMINAL:
                # First seen already terminal: earlier retries may predate us.
                known["history_gap"] = True
        else:
            known["status"] = status
        for sid in task_sessions(task):
            if sid not in known["sessions"]:
                if known["completed_at"] is not None:
                    # New session after completion: a retry we had missed.
                    known["history_gap"] = True
                known["sessions"].append(sid)

    for row in export["ledger"]:
        task_id = row.get("task_id")
        if type(task_id) is not int or str(task_id) not in history["tasks"]:
            raise ValueError("ledger binding references unknown task")
        known = history["tasks"][str(task_id)]
        if row.get("queue") != known["queue"]:
            raise ValueError(f"task {task_id}: ledger queue conflicts with task queue")
        sid = row.get("session_id")
        if sid:
            if not isinstance(sid, str):
                raise ValueError(f"task {task_id}: ledger session_id must be a string")
            if sid not in known["sessions"]:
                if known["completed_at"] is not None:
                    known["history_gap"] = True
                known["sessions"].append(sid)

    for event in export["events"]:
        task_id = event.get("task_id")
        if type(task_id) is not int or str(task_id) not in history["tasks"]:
            raise ValueError("completion event references unknown task")
        if event.get("kind") not in COMPLETION_KINDS:
            continue
        when = timestamp(event.get("created_at"))
        known = history["tasks"][str(task_id)]
        if known["completed_at"] is None or when > timestamp(known["completed_at"]):
            known["completed_at"] = event["created_at"]

    for session in export["sessions"]:
        sid = session.get("id")
        if not isinstance(sid, str) or not sid:
            raise ValueError("session id must be a nonempty string")
        counts_map = session.get("tokens")
        value = counts_map.get("input") if isinstance(counts_map, dict) else None
        if value is not None and (type(value) is not int or value < 0):
            raise ValueError(
                f"session {sid}: input must be a nonnegative integer or null"
            )
        known = history["sessions"].get(sid)
        if known is None:
            history["sessions"][sid] = {"input": value, "last_seen": at}
        else:
            # Cumulative counters only move forward; null never erases known.
            if value is not None:
                known["input"] = value
            known["last_seen"] = at

    history["record_calls"] = history.get("record_calls", 0) + 1
    if history.get("first_record_at") is None:
        history["first_record_at"] = at
    history["last_record_at"] = at
    history["sources"].append({"at": at, **counts})
    history["sources"] = history["sources"][-50:]
    return history


def completed_in_window(history, start, end):
    lower, upper = timestamp(start), timestamp(end)
    if lower >= upper:
        raise ValueError("start must be before end")
    cohort = []
    for key, task in history["tasks"].items():
        if task["status"] != "done" or task["completed_at"] is None:
            continue
        when = timestamp(task["completed_at"])
        if lower <= when < upper:
            cohort.append((int(key), task))
    return cohort


def snapshot(history, start, end):
    """Assemble a fleet_completion_usage.py input document."""
    cohort = completed_in_window(history, start, end)
    gap = [task_id for task_id, task in cohort if task["history_gap"]]
    ledger, events, sessions = [], [], []
    seen_sessions = set()
    for task_id, task in cohort:
        for sid in task["sessions"]:
            ledger.append(
                {"task_id": task_id, "queue": task["queue"], "session_id": sid}
            )
            if sid not in seen_sessions:
                seen_sessions.add(sid)
                stored = history["sessions"].get(sid, {"input": None})
                sessions.append({"id": sid, "tokens": {"input": stored["input"]}})
        events.append(
            {
                "task_id": task_id,
                "kind": "completed",
                "created_at": task["completed_at"],
            }
        )
    tasks = [
        {"id": int(key), "queue": task["queue"], "status": task["status"]}
        for key, task in history["tasks"].items()
    ]
    return {
        "binding_history_complete": not gap,
        "tasks": tasks,
        "events": events,
        "ledger": ledger,
        "sessions": sessions,
        "_collector": {
            "scope": "retained launch/retry bindings assembled for [start,end)",
            "start": start,
            "end": end,
            "completed_in_window": len(cohort),
            "gap_task_ids": sorted(gap),
            "record_calls": history.get("record_calls", 0),
            "history_covers_from": history.get("first_record_at"),
        },
    }


def load_usage_module():
    import importlib.util

    path = Path(__file__).with_name("fleet_completion_usage.py")
    spec = importlib.util.spec_from_file_location("fleet_completion_usage", path)
    if spec is None or spec.loader is None:
        return None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def checkin(history, start, end):
    """Render the daily check-in markdown for one window."""
    snap = snapshot(history, start, end)
    collector = snap.pop("_collector")
    lines = [
        f"# Queue input check-in: {start} to {end}",
        "",
        "Completed-cohort cumulative bound-session input tokens; not window spend.",
        "",
        f"Retry-history complete: **{snap['binding_history_complete']}**"
        + (
            ""
            if snap["binding_history_complete"]
            else f" (gap tasks: {collector['gap_task_ids']})"
        ),
        f"Completed in window: {collector['completed_in_window']} "
        f"(history covers from {collector['history_covers_from']}, "
        f"{collector['record_calls']} record calls)",
        "",
    ]
    usage = load_usage_module()
    if usage is None:
        lines += [
            "Usage report unavailable (fleet_completion_usage.py not importable);",
            "bindings retained but no per-queue totals are claimed.",
            "",
        ]
    else:
        try:
            report = usage.report(snap, start, end)
        except (ValueError, KeyError, TypeError, AttributeError) as error:
            raise ValueError(f"usage report rejected the snapshot: {error}")
        if not report["queues"]:
            lines += ["No completed cohort in this window; nothing to report.", ""]
        for queue in report["queues"]:
            ratio = queue["input_tokens_per_completed_task"]
            lines += [
                f"## {queue['queue']}",
                f"- completed: {queue['completed_tasks']}, "
                f"measured: {queue['measured_tasks']}",
                f"- known input tokens: {queue['known_input_tokens']}",
                "- input per completed task: "
                + (f"{ratio:.2f}" if ratio is not None else "WITHHELD (incomplete)"),
                "- missing tasks: "
                + (", ".join(map(str, queue["missing_task_ids"])) or "none"),
                "",
            ]
        unlocated = report.get("done_tasks_without_completion_event", [])
        if unlocated:
            lines += [
                "Done without a completion event (window unestablished): "
                + ", ".join(map(str, sorted(unlocated))),
                "",
            ]
    lines += [
        "Do not infer window spend or a cadence before/after result from this "
        "check-in; it covers only completed cohorts with complete history.",
        "",
    ]
    return "\n".join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    sub = parser.add_subparsers(dest="command", required=True)

    record_p = sub.add_parser("record", help="merge one export into the history")
    record_p.add_argument("--history", required=True, type=Path)
    record_p.add_argument("--export", required=True, type=Path)
    record_p.add_argument("--now", default=None)

    snap_p = sub.add_parser("snapshot", help="emit a usage-report input document")
    snap_p.add_argument("--history", required=True, type=Path)
    snap_p.add_argument("--out", required=True, type=Path)
    snap_p.add_argument("--start", required=True)
    snap_p.add_argument("--end", required=True)

    check_p = sub.add_parser("checkin", help="render the daily check-in markdown")
    check_p.add_argument("--history", required=True, type=Path)
    check_p.add_argument("--start", required=True)
    check_p.add_argument("--end", required=True)

    args = parser.parse_args(argv)
    try:
        if args.command == "record":
            history = load_history(args.history)
            export = load_export(args.export)
            at = args.now or now_iso()
            timestamp(at)
            record(history, export, at)
            args.history.write_text(json.dumps(history, indent=2, sort_keys=True))
        else:
            history = load_history(args.history)
            if args.command == "snapshot":
                snap = snapshot(history, args.start, args.end)
                args.out.write_text(json.dumps(snap, indent=2, sort_keys=True))
            else:
                sys.stdout.write(checkin(history, args.start, args.end))
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
