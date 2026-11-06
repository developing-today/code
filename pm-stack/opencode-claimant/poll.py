#!/usr/bin/env python3
"""Candidate discovery for OpenCode-lane claimant.

Prints JSON to stdout with:
- candidates: tasks claimed, lane=opencode, unbound (session_id null), stage=handed_off or launch_failed
- tasks: all tasks with lane=opencode (for supervision)
- stuck_bound: tasks bound to a session that never progressed (for recovery rule)
- cap_room: integer active root sessions (None if server unreadable)
- active_roots: alias for cap_room
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path
from typing import Any

PRIMARY = Path.home() / "git" / "code"
SERVER = "http://127.0.0.1:13420"


def http_get(path: str) -> dict[str, Any] | list[Any] | None:
    try:
        import urllib.request
        with urllib.request.urlopen(SERVER + path, timeout=5) as resp:
            raw = resp.read()
            return json.loads(raw) if raw else None
    except Exception:
        return None


def queue(*args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    from pathlib import Path

    queue_cli = Path.home() / "fleet-brokers" / "bin" / "repo-queue"
    command = [str(queue_cli), *args, "--worker", "opencode-claimant"]
    result = subprocess.run(command, text=True, capture_output=True, check=False)
    if check and result.returncode:
        raise RuntimeError(f"repo-queue {' '.join(args)} exited {result.returncode}")
    return result


def as_list(value: Any) -> list[dict[str, Any]]:
    return value if isinstance(value, list) else []


def task_id(task: dict[str, Any]) -> str:
    return str(task.get("id", task.get("task_id", "")))


def get_progress(task: dict[str, Any]) -> dict[str, Any]:
    return task.get("progress") if isinstance(task.get("progress"), dict) else {}


def get_bound_session(task: dict[str, Any]) -> str | None:
    progress = get_progress(task)
    value = progress.get("session_id", task.get("session_id"))
    return str(value) if value else None


def lease_expired(task: dict[str, Any]) -> bool:
    lease = task.get("lease")
    if isinstance(lease, dict):
        expiry = lease.get("expires_at") or lease.get("expiresAt")
    else:
        expiry = task.get("lease_expires_at") or task.get("lease_expires")
    if not expiry:
        return False
    try:
        from datetime import datetime, timezone
        parsed = datetime.fromisoformat(str(expiry).replace("Z", "+00:00"))
        return parsed <= datetime.now(timezone.utc)
    except Exception:
        return False


def count_active_roots() -> int | None:
    data = http_get("/api/session")
    sessions = data if isinstance(data, list) else data.get("sessions", []) if isinstance(data, dict) else []
    count = 0
    for row in sessions:
        if not isinstance(row, dict):
            continue
        if row.get("id") is None:
            continue
        time = row.get("time")
        if isinstance(time, dict):
            idle = time.get("idle")
        else:
            idle = row.get("time_idle")
        outcome = row.get("outcome")
        if outcome is not None:
            continue
        if idle is None:
            continue
        try:
            from datetime import datetime, timezone, timedelta
            threshold = datetime.now(timezone.utc) - timedelta(minutes=30)
            if isinstance(idle, str):
                idle_dt = datetime.fromisoformat(idle.replace("Z", "+00:00"))
            elif isinstance(idle, (int, float)):
                idle_dt = datetime.fromtimestamp(idle / 1000, timezone.utc)
            else:
                idle_dt = None
            if idle_dt and idle_dt >= threshold:
                count += 1
        except Exception:
            continue
    return count


def main() -> int:
    task_id_filter = sys.argv[sys.argv.index("--task-id") + 1] if "--task-id" in sys.argv else None

    # Fetch all lane=opencode tasks
    tasks: list[dict[str, Any]] = []
    stuck_bound: list[dict[str, Any]] = []
    candidates: list[dict[str, Any]] = []

    try:
        result = queue("list", "--lane", "opencode", check=False)
        if result.returncode == 0:
            try:
                data = json.loads(result.stdout)
                tasks = as_list(data.get("tasks"))
            except Exception:
                pass
    except Exception:
        pass

    # Build candidates and stuck_bound
    for task in tasks:
        tid = task_id(task)
        if task_id_filter and tid != task_id_filter:
            continue
        bound_sid = get_bound_session(task)
        progress = get_progress(task)
        stage = progress.get("stage")
        if bound_sid:
            # Supervision candidates: bound session present
            if stage not in ("handed_off", "launch_failed"):
                # Not yielded to claimant yet
                continue
            if lease_expired(task):
                stuck_bound.append(task)
        else:
            # Launch candidates: unbound
            if stage not in ("handed_off", "launch_failed"):
                continue
            candidates.append(task)

    cap_room = count_active_roots()

    output = {
        "tasks": tasks,
        "candidates": candidates,
        "stuck_bound": stuck_bound,
        "cap_room": cap_room,
        "active_roots": cap_room,
    }
    print(json.dumps(output, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
