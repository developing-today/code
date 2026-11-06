#!/usr/bin/env python3
"""OpenCode-lane claimant/executor for queue tasks.

Slice 1 of #3026: discover, supervise, claim/bind/launch and record tasks
assigned to the OpenCode execution lane. Shadow mode is the safe default.
"""

from __future__ import annotations

import fcntl
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


PRIMARY = Path.home() / "git" / "code"
ROOT = Path(__file__).resolve().parent
POLL = ROOT / "poll.py"
STATE = Path.home() / ".local" / "state" / "opencode-claimant"
MODE_FILE = STATE / "mode"
FIRE_LOG = STATE / "fires.jsonl"
LOCK_FILE = STATE / "claimant.lock"
QUEUE_CLI = Path.home() / "fleet-brokers" / "bin" / "repo-queue"
SERVER = "http://127.0.0.1:13420"
QUEUES = (
    "repo-health-code-pilot",
    "repo-health-opencode-pilot",
    "repo-health-openchamber-pilot",
    "repo-health-nix-pilot",
)
DEFAULT_ROUTE = {"providerID": "opencode-route", "modelID": "workhorse", "agent": "build"}


class ClaimantError(RuntimeError):
    pass


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def read_mode() -> str:
    try:
        mode = MODE_FILE.read_text(encoding="utf-8").strip().lower()
    except OSError:
        return "shadow"
    return mode if mode in {"shadow", "live"} else "shadow"


def http_json(method: str, path: str, body: Any = None, timeout: float = 10) -> Any:
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request(
        SERVER + path,
        data=data,
        method=method,
        headers={"Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            raw = response.read()
            if not raw:
                return None
            return json.loads(raw)
    except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as exc:
        raise ClaimantError(f"OpenCode {method} {path} failed: {exc}") from exc


def queue(*args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    command = [str(QUEUE_CLI), *args, "--worker", "opencode-claimant"]
    result = subprocess.run(command, text=True, capture_output=True, check=False)
    if check and result.returncode:
        raise ClaimantError(
            f"repo-queue {' '.join(args)} exited {result.returncode}: "
            f"{result.stderr.strip() or result.stdout.strip()}"
        )
    return result


def discover(task_id: str | None = None) -> dict[str, Any]:
    command = [sys.executable, str(POLL)]
    if task_id is not None:
        command.extend(["--task-id", str(task_id)])
    result = subprocess.run(command, cwd=PRIMARY, text=True, capture_output=True, check=False)
    if result.returncode:
        raise ClaimantError(f"poll.py exited {result.returncode}: {result.stderr.strip()}")
    try:
        data = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise ClaimantError(f"poll.py returned invalid JSON: {exc}") from exc
    if not isinstance(data, dict):
        raise ClaimantError("poll.py result must be a JSON object")
    return data


def as_list(value: Any) -> list[dict[str, Any]]:
    return value if isinstance(value, list) else []


def task_id(task: dict[str, Any]) -> str:
    return str(task.get("id", task.get("task_id", "")))


def get_progress(task: dict[str, Any]) -> dict[str, Any]:
    progress = task.get("progress")
    return progress if isinstance(progress, dict) else {}


def get_bound_session(task: dict[str, Any]) -> str | None:
    progress = get_progress(task)
    value = progress.get("session_id", task.get("session_id"))
    return str(value) if value else None


def lease_expired(task: dict[str, Any]) -> bool:
    for key in ("lease_expired", "lease_is_expired"):
        if key in task:
            return bool(task[key])
    lease = task.get("lease")
    if isinstance(lease, dict):
        expiry = lease.get("expires_at") or lease.get("expiresAt")
    else:
        expiry = task.get("lease_expires_at") or task.get("lease_expires")
    if not expiry:
        return False
    try:
        parsed = datetime.fromisoformat(str(expiry).replace("Z", "+00:00"))
        return parsed <= datetime.now(timezone.utc)
    except (ValueError, TypeError):
        return False


def attempts(task: dict[str, Any]) -> tuple[int, int]:
    return int(task.get("attempts", 0)), int(task.get("max_attempts", 0))


def session_index() -> dict[str, dict[str, Any]]:
    result = http_json("GET", "/api/session")
    sessions = result if isinstance(result, list) else result.get("sessions", []) if isinstance(result, dict) else []
    return {str(row.get("id")): row for row in sessions if isinstance(row, dict) and row.get("id")}


def session_messages(session_id: str) -> list[Any]:
    result = http_json("GET", f"/api/session/{urllib.parse.quote(session_id, safe='')}/message")
    if isinstance(result, list):
        return result
    if isinstance(result, dict):
        messages = result.get("messages")
        if isinstance(messages, list):
            return messages
    raise ClaimantError(f"message list for session {session_id} is unavailable or incomplete")


def append_log(record: dict[str, Any]) -> None:
    STATE.mkdir(parents=True, exist_ok=True)
    with FIRE_LOG.open("a", encoding="utf-8") as stream:
        stream.write(json.dumps(record, sort_keys=True, separators=(",", ":")) + "\n")
        stream.flush()
        os.fsync(stream.fileno())


def queue_health(skips: list[dict[str, Any]]) -> None:
    for name in QUEUES:
        result = queue("health", "--queue", name, check=False)
        if result.returncode:
            skips.append({"queue": name, "reason": f"health failed: {result.stderr.strip()}"})


def check_issue_holds(task: dict[str, Any]) -> str | None:
    text = " ".join(
        str(task.get(key, ""))
        for key in ("issue", "title", "labels", "issue_labels", "packet", "body")
    ).lower()
    for marker in ("held", "user/owned", "do-not-merge", "do-not-auto-edit"):
        if marker in text:
            return marker
    return None


def route_for(task: dict[str, Any]) -> dict[str, str]:
    packet = task.get("packet") if isinstance(task.get("packet"), dict) else {}
    route = packet.get("route", task.get("route"))
    if not isinstance(route, dict):
        return dict(DEFAULT_ROUTE)
    return {
        "providerID": str(route.get("providerID", DEFAULT_ROUTE["providerID"])),
        "modelID": str(route.get("modelID", route.get("model", DEFAULT_ROUTE["modelID"]))),
        "agent": str(route.get("agent", DEFAULT_ROUTE["agent"])),
    }


def run_supervision(data: dict[str, Any], sessions: dict[str, dict[str, Any]], skips: list[dict[str, Any]], mode: str) -> list[dict[str, Any]]:
    recoveries: list[dict[str, Any]] = []
    bound_tasks = [task for task in as_list(data.get("tasks")) if get_bound_session(task)]
    stuck_by_id = {task_id(task): task for task in as_list(data.get("stuck_bound"))}
    for task in bound_tasks:
        tid = task_id(task)
        sid = get_bound_session(task)
        session = sessions.get(sid or "")
        if not session:
            if not lease_expired(task):
                skips.append({"task_id": tid, "session_id": sid, "reason": "bound session absent; lease not expired"})
                continue
            remaining, maximum = attempts(task)
            if mode == "shadow":
                skips.append({"task_id": tid, "session_id": sid, "reason": "would recover absent session; shadow mode"})
                continue
            if remaining >= maximum:
                terminal_fail(task, sid or "")
                recoveries.append({"task_id": tid, "session_id": sid, "action": "failed_attempts_exhausted"})
            else:
                atomic_recover(tid, sid or "", "gone-session")
                recoveries.append({"task_id": tid, "session_id": sid, "action": "reclaimed_gone_session"})
            continue

        if tid not in stuck_by_id:
            skips.append({"task_id": tid, "session_id": sid, "reason": "bound session exists; not stuck-bound"})
            continue
        if not lease_expired(task):
            skips.append({"task_id": tid, "session_id": sid, "reason": "stuck-bound lease not expired"})
            continue
        if mode == "shadow":
            skips.append({"task_id": tid, "session_id": sid, "reason": "stuck-bound; shadow mode reports only"})
            continue
        try:
            messages = session_messages(sid or "")
        except ClaimantError as exc:
            skips.append({"task_id": tid, "session_id": sid, "reason": f"unknown stuck state: {exc}"})
            continue
        if messages or has_activity(session):
            skips.append({"task_id": tid, "session_id": sid, "reason": "session has messages/activity; left bound"})
            continue
        remaining, maximum = attempts(task)
        try:
            delete_and_verify(sid or "")
        except ClaimantError as exc:
            skips.append({"task_id": tid, "session_id": sid, "reason": f"zero-message delete failed; binding kept: {exc}"})
            continue
        if remaining >= maximum:
            terminal_fail(task, sid or "")
            recoveries.append({"task_id": tid, "session_id": sid, "action": "deleted_and_failed_attempts_exhausted"})
        else:
            try:
                atomic_recover(tid, sid or "", "zero-message-deleted")
                recoveries.append({"task_id": tid, "session_id": sid, "action": "deleted_and_reclaimed"})
            except ClaimantError as exc:
                skips.append({"task_id": tid, "session_id": sid, "reason": f"compare-and-clear failed; session deleted, binding untouched: {exc}"})
    return recoveries


def has_activity(session: dict[str, Any]) -> bool:
    return bool(session.get("time", {}).get("idle") or session.get("outcome")) if isinstance(session.get("time"), dict) else bool(session.get("outcome"))


def delete_and_verify(session_id: str) -> None:
    path = f"/api/session/{urllib.parse.quote(session_id, safe='')}"
    http_json("DELETE", path)
    sessions = session_index()
    if session_id in sessions:
        raise ClaimantError("session still appears in session list after delete")


def atomic_recover(tid: str, sid: str, reason: str) -> None:
    queue("bind", tid, "--session-id", sid, "--launched-by", f"opencode-claimant-recovery:{reason}")


def terminal_fail(task: dict[str, Any], sid: str) -> None:
    tid = task_id(task)
    worker_id = str(task.get("worker_id", ""))
    execution = json.dumps({"lane": "opencode", "session_id": sid, "fallback_reason": "launch_failed_attempts_exhausted"})
    queue(
        "fail", tid, "--worker", worker_id, "--error",
        f"bound session {sid} unavailable without terminal outcome; attempts exhausted",
        "--reason-class", "deterministic", "--execution", execution,
    )


def prompt_for(task: dict[str, Any], session_id: str) -> str:
    tid = task_id(task)
    worker_id = str(task.get("worker_id", ""))
    queue_name = str(task.get("queue", task.get("queue_name", "unknown queue")))
    issue = task.get("issue", task.get("issue_url", task.get("title", "")))
    packet = task.get("packet", {})
    packet_text = json.dumps(packet, ensure_ascii=False, sort_keys=True)
    runbook = task.get("runbook", task.get("queue_runbook", ""))
    return (
        f"Work ONLY queue task {tid} from {queue_name}. This task is already claimed and lane-bound; "
        f"do not claim another task.\nIssue: {issue}\nPacket: {packet_text}\n\n"
        f"Queue runbook:\n{runbook}\n\n"
        f"Use {QUEUE_CLI} and pass --worker {worker_id} on EVERY queue CLI verb (heartbeat, checkpoint, "
        "and terminal verbs). Heartbeat while working so the claim lease stays alive. Finish using the queue "
        "CLI terminal verb (complete/park/block/fail) with --execution "
        f"'{{\"lane\":\"opencode\",\"session_id\":\"{session_id}\"}}' using this same bound session id."
    )


def create_session(task: dict[str, Any], route: dict[str, str]) -> dict[str, Any]:
    body = {
        "location": {"directory": str(PRIMARY)},
        "title": f"{task.get('queue', task.get('queue_name', 'queue task'))}: {task_id(task)} — {task.get('issue', task.get('title', ''))}",
        "model": {"id": route["modelID"], "providerID": route["providerID"]},
        "agent": route["agent"],
    }
    result = http_json("POST", "/api/session", body)
    if not isinstance(result, dict) or not result.get("id"):
        raise ClaimantError("session create returned no session id")
    if not isinstance(result.get("location"), dict) or result["location"].get("directory") != str(PRIMARY):
        try:
            http_json("DELETE", f"/api/session/{urllib.parse.quote(str(result['id']), safe='')}")
        except ClaimantError:
            pass
        raise ClaimantError(f"created session location was not verified as {PRIMARY}")
    return result


def send_prompt(session_id: str, prompt: str) -> None:
    path = f"/api/session/{urllib.parse.quote(session_id, safe='')}/message"
    http_json("POST", path, {"parts": [{"type": "text", "text": prompt}]}, timeout=30)


def add_issue_note(task: dict[str, Any], session_id: str) -> None:
    issue = task.get("issue")
    if isinstance(issue, dict):
        issue = issue.get("url", issue.get("number"))
    if not issue:
        issue = task.get("issue_url", "")
    note = f"agent-note:v1 state=handed off task={task_id(task)} session={session_id}"
    queue("note", task_id(task), "--issue", str(issue), "--note", note)


def launch(task: dict[str, Any], skips: list[dict[str, Any]]) -> dict[str, str] | None:
    tid = task_id(task)
    held = check_issue_holds(task)
    if held:
        skips.append({"task_id": tid, "reason": f"issue hold marker: {held}"})
        return None
    current = discover(tid)
    candidates = as_list(current.get("candidates"))
    if not any(task_id(candidate) == tid for candidate in candidates):
        skips.append({"task_id": tid, "reason": "candidate no longer claimed, yielded, lane-assigned, or unbound"})
        return None
    candidate = next(candidate for candidate in candidates if task_id(candidate) == tid)
    route = route_for(candidate)
    try:
        created = create_session(candidate, route)
    except ClaimantError as exc:
        checkpoint_creation_failure(candidate, str(exc))
        skips.append({"task_id": tid, "reason": f"session creation failed; left unbound: {exc}"})
        return None
    sid = str(created["id"])
    try:
        queue("bind", tid, "--session-id", sid)
    except ClaimantError as exc:
        skips.append({"task_id": tid, "session_id": sid, "reason": f"bind failed; session not prompted: {exc}"})
        return None
    try:
        progress = json.dumps({"session_id": sid, "launched_by": "opencode-claimant"})
        worker_id = str(candidate.get("worker_id", ""))
        queue("checkpoint", tid, "--worker", worker_id, "--stage", "launched", "--progress", progress)
        send_prompt(sid, prompt_for(candidate, sid))
    except ClaimantError as exc:
        skips.append({"task_id": tid, "session_id": sid, "reason": f"post-bind action failed; binding retained for supervision: {exc}"})
        return {"task_id": tid, "session_id": sid, "status": "bound_prompt_or_checkpoint_failed"}
    try:
        add_issue_note(candidate, sid)
    except ClaimantError as exc:
        skips.append({"task_id": tid, "session_id": sid, "reason": f"issue note failed; bound session retained: {exc}"})
        return {"task_id": tid, "session_id": sid, "status": "launched_note_failed"}
    return {"task_id": tid, "session_id": sid, "status": "launched", "route": route}


def checkpoint_creation_failure(task: dict[str, Any], reason: str) -> None:
    worker_id = str(task.get("worker_id", ""))
    queue(
        "checkpoint", task_id(task), "--worker", worker_id,
        "--stage", "launch_failed", "--error", reason,
    )


def main() -> int:
    STATE.mkdir(parents=True, exist_ok=True)
    with LOCK_FILE.open("w", encoding="utf-8") as lock_stream:
        try:
            fcntl.flock(lock_stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return 0

        mode = read_mode()
        skips: list[dict[str, Any]] = []
        launched: list[dict[str, Any]] = []
        recoveries: list[dict[str, Any]] = []
        try:
            data = discover()
            candidates = list(as_list(data.get("candidates")))
            tasks = as_list(data.get("tasks"))
            queue_health(skips)
            try:
                sessions = session_index()
            except ClaimantError as exc:
                sessions = {}
                skips.append({"reason": f"OpenCode session list unavailable: {exc}"})
            if mode == "live":
                recoveries = run_supervision(data, sessions, skips, mode)
            else:
                run_supervision(data, sessions, skips, mode)

            cap = data.get("cap_room")
            cap_count = data.get("active_roots", data.get("cap_count"))
            if not candidates:
                pass
            elif cap is None:
                skips.append({"reason": "cap_room null; OpenCode server unreadable, no launches"})
            elif int(cap) <= 0:
                skips.append({"reason": "36-session cap saturated; no launches"})
            elif mode == "shadow":
                task_map = {task_id(task): task for task in tasks + candidates}
                for candidate in candidates:
                    task = task_map.get(task_id(candidate), candidate)
                    held = check_issue_holds(task)
                    skips.append({
                        "task_id": task_id(candidate),
                        "reason": f"issue hold marker: {held}" if held else "shadow mode; would launch",
                        "route": route_for(candidate),
                    })
            else:
                remaining = int(cap)
                for candidate in candidates:
                    if remaining <= 0:
                        skips.append({"task_id": task_id(candidate), "reason": "cap room exhausted during fire"})
                        continue
                    result = launch(candidate, skips)
                    if result:
                        launched.append(result)
                        remaining -= 1
        except Exception as exc:
            skips.append({"reason": f"fire failed closed: {type(exc).__name__}: {exc}"})
            data = locals().get("data", {})
            candidates = locals().get("candidates", [])
            cap_count = locals().get("cap_count")
        record = {
            "time": utc_now(),
            "mode": mode,
            "candidates_seen": [task_id(task) for task in candidates],
            "sessions_launched": launched,
            "recoveries": recoveries,
            "cap_count": cap_count,
            "cap_room": data.get("cap_room") if isinstance(data, dict) else None,
            "skips": skips,
        }
        append_log(record)
        print(json.dumps(record, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
