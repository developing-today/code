#!/usr/bin/env python3
"""Report completed-cohort bound-session input usage (#2140).

Contract: --snapshot FILE --start ISO_TIMESTAMP --end ISO_TIMESTAMP; no
positionals/unknown options/abbreviations. Parse all arguments before file I/O.
Read-only, offline: no queue queries, network calls or schedule changes.
"""

import argparse
from datetime import datetime
import json
from pathlib import Path
import sys


def timestamp(value):
    if not isinstance(value, str):
        raise ValueError("timestamp must be an ISO string with timezone")
    result = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if result.utcoffset() is None:
        raise ValueError("timestamp must include timezone")
    return result


def report(snapshot, start, end):
    """Latest finishing event/current done state is the console trends oracle.

    Tokens are cumulative for the completed cohort, NOT spend in [start,end).
    Missing usage/bindings never become zero; retries from the ledger count once.
    """
    lower, upper = timestamp(start), timestamp(end)
    if lower >= upper:
        raise ValueError("start must be before end")
    for key in ("tasks", "events", "ledger", "sessions"):
        if not isinstance(snapshot.get(key), list):
            raise ValueError(f"snapshot.{key} must be an array")
    history_complete = snapshot.get("binding_history_complete", False)
    if type(history_complete) is not bool:
        raise ValueError("binding_history_complete must be boolean")

    tasks, bindings, owners = {}, {}, {}

    def bind(task_id, session_id):
        if not isinstance(session_id, str) or not session_id:
            raise ValueError("session_id must be a nonempty string")
        if session_id in owners and owners[session_id] != task_id:
            raise ValueError(f"session {session_id} binds multiple tasks")
        owners[session_id] = task_id
        bindings[task_id].add(session_id)

    for task in snapshot["tasks"]:
        task_id = task["id"]
        if type(task_id) is not int or task_id in tasks:
            raise ValueError("task IDs must be unique integers")
        if not isinstance(task.get("queue"), str) or not task["queue"]:
            raise ValueError("task queue must be a nonempty string")
        tasks[task_id] = task
        bindings[task_id] = set()
        progress = task.get("progress") or {}
        if isinstance(progress, str):
            progress = json.loads(progress)
        if not isinstance(progress, dict):
            raise ValueError("task progress must be an object")
        sid = progress.get("session_id") or task.get("session_id")
        if sid:
            bind(task_id, sid)

    for row in snapshot["ledger"]:
        task_id = row["task_id"]
        if task_id not in tasks or row["queue"] != tasks[task_id]["queue"]:
            raise ValueError("ledger binding has unknown task or conflicting queue")
        if row.get("session_id"):
            bind(task_id, row["session_id"])

    latest = {}
    for event in snapshot["events"]:
        if event["kind"] not in {"completed", "staleness_autocomplete"}:
            continue
        task_id = event["task_id"]
        if task_id not in tasks:
            raise ValueError("completion event references unknown task")
        when = timestamp(event["created_at"])
        latest[task_id] = max(when, latest.get(task_id, when))

    tokens = {}
    for session in snapshot["sessions"]:
        sid = session["id"]
        if not isinstance(sid, str) or not sid:
            raise ValueError("session id must be a nonempty string")
        if sid in tokens:
            raise ValueError(f"duplicate session {sid}")
        counts = session.get("tokens")
        value = counts.get("input") if isinstance(counts, dict) else None
        if value is not None and (type(value) is not int or value < 0):
            raise ValueError("input tokens must be a nonnegative integer or null")
        tokens[sid] = value

    queues = {}
    unlocated = []
    for task_id, task in tasks.items():
        if task.get("status") != "done":
            continue
        if task_id not in latest:
            unlocated.append(task_id)
            continue
        if not lower <= latest[task_id] < upper:
            continue
        queue = queues.setdefault(
            task["queue"],
            {
                "queue": task["queue"],
                "completed_tasks": 0,
                "measured_tasks": 0,
                "missing_task_ids": [],
                "known_input_tokens": 0,
                "input_tokens_per_completed_task": None,
            },
        )
        queue["completed_tasks"] += 1
        sessions = bindings[task_id]
        known = [tokens[s] for s in sessions if tokens.get(s) is not None]
        queue["known_input_tokens"] += sum(known)
        if history_complete and sessions and len(known) == len(sessions):
            queue["measured_tasks"] += 1
        else:
            queue["missing_task_ids"].append(task_id)

    for queue in queues.values():
        queue["missing_task_ids"].sort()
        unknown_completion = any(tasks[t]["queue"] == queue["queue"] for t in unlocated)
        if not queue["missing_task_ids"] and not unknown_completion:
            queue["input_tokens_per_completed_task"] = (
                queue["known_input_tokens"] / queue["completed_tasks"]
            )
    return {
        "scope": "completed-cohort cumulative bound-session input tokens; not window spend",
        "start": start,
        "end": end,
        "binding_history_complete": history_complete,
        "done_tasks_without_completion_event": sorted(unlocated),
        "queues": [queues[q] for q in sorted(queues)],
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--snapshot", required=True, type=Path)
    parser.add_argument("--start", required=True)
    parser.add_argument("--end", required=True)
    args = parser.parse_args(argv)
    try:
        result = report(json.loads(args.snapshot.read_text()), args.start, args.end)
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
        parser.error(str(error))
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
