#!/usr/bin/env python3
"""
claimant.py — OpenCode lane claimant/executor for slice 1 of #3026.

This script runs as an OpenChamber scheduled task on the laptop.
It implements the claim/bind/launch/record workflow for opencode-lane tasks.

Mode is read from ~/.local/state/opencode-claimant/mode (shadow or live).
In shadow mode: discover, verify, and log what would launch — launch nothing.
In live mode: perform supervision, candidate discovery, and launches.

Constants and flow are documented in prompt.md.
"""

import json
import os
import re
import sys
import time
import urllib.request
import urllib.error
import base64
from datetime import datetime, timezone
from pathlib import Path
import subprocess
import fcntl
from concurrent.futures import ThreadPoolExecutor, wait, FIRST_COMPLETED

# Constants from prompt.md
QUEUES = [
    "repo-health-code-pilot",
    "repo-health-opencode-pilot",
    "repo-health-openchamber-pilot",
    "repo-health-nix-pilot",
]
CAP = 36
ACTIVE_WINDOW_MS = 30 * 60 * 1000  # 30 minutes
OPENCODE_URL = os.environ.get("OPENCODE_URL", "http://127.0.0.1:13420")
AUTH_FILE = os.environ.get(
    "OPENCODE_AUTH_FILE",
    os.path.expanduser("~/.local/state/opencode/service-prod.json"),
)
BROKER_TOKEN_FILE = os.environ.get(
    "REPO_QUEUE_TOKEN_FILE",
    os.path.expanduser("~/.config/queue-broker/token"),
)
REPO_PRIMARY = "/home/user/git/code"


def read_mode():
    """Read mode from ~/.local/state/opencode-claimant/mode.
    
    Returns 'live' or 'shadow'. Missing/unreadable defaults to 'shadow'.
    """
    path = Path.home() / ".local" / "state" / "opencode-claimant" / "mode"
    try:
        return path.read_text().strip().lower()
    except Exception:
        return "shadow"


def fire_log_path():
    """Path to the fire log JSONL file."""
    return Path.home() / ".local" / "state" / "opencode-claimant" / "fires.jsonl"


def log_fire(mode, candidates, launched, cap_room, skips):
    """Append a fire log entry to the JSONL log file."""
    log_path = fire_log_path()
    log_path.parent.mkdir(parents=True, exist_ok=True)
    
    entry = {
        "timestamp": datetime.utcnow().isoformat() + "Z",
        "mode": mode,
        "candidates": candidates,
        "launched": launched,
        "cap_room": cap_room,
        "skips": skips,
    }
    
    with log_path.open("a") as f:
        f.write(json.dumps(entry, separators=(",", ":")) + "\n")


def fetch_active_ids():
    """Fetch active session IDs from OpenCode server.
    
    Returns set of active session IDs, or None if unreadable.
    """
    try:
        with open(AUTH_FILE) as f:
            password = json.load(f)["password"]
    except Exception:
        return None
    
    req = urllib.request.Request(f"{OPENCODE_URL.rstrip('/')}/api/session/active")
    token = base64.b64encode(f"opencode:{password}".encode()).decode()
    req.add_header("Authorization", f"Basic {token}")
    
    try:
        with urllib.request.urlopen(req, timeout=15) as res:
            data = json.loads(res.read() or "null")
    except Exception:
        return None
    
    if isinstance(data, dict) and "data" in data:
        data = data["data"]
    if isinstance(data, dict):
        return set(data.keys())
    if isinstance(data, list):
        return {x for x in data if isinstance(x, str)}
    return None


def fetch_sessions():
    """Fetch all sessions from OpenCode server.
    
    Returns list of session objects, or None if unreadable.
    """
    try:
        with open(AUTH_FILE) as f:
            password = json.load(f)["password"]
    except Exception:
        return None
    
    req = urllib.request.Request(f"{OPENCODE_URL.rstrip('/')}/api/session")
    token = base64.b64encode(f"opencode:{password}".encode()).decode()
    req.add_header("Authorization", f"Basic {token}")
    
    try:
        with urllib.request.urlopen(req, timeout=15) as res:
            data = json.loads(res.read() or "null")
    except Exception:
        return None
    
    if isinstance(data, dict) and "data" in data:
        data = data["data"]
    if not isinstance(data, list):
        return None
    return data


def is_root_session(s):
    """Check if a session is a root session (no parent)."""
    return isinstance(s, dict) and not s.get("parentID") and not s.get("parent_id")


def is_active_root(s, now_ms, active_ids=None):
    """Check if a root session is currently active."""
    if not is_root_session(s):
        return False
    
    # Server's active set is authoritative
    if active_ids is not None and s.get("id") in active_ids:
        return True
    
    t = s.get("time") or {}
    if t.get("idle") or s.get("outcome"):
        return False
    
    updated = t.get("updated") or t.get("created") or 0
    return now_ms - updated < ACTIVE_WINDOW_MS


def count_active_roots(sessions, now_ms, active_ids=None):
    """Count active root sessions."""
    return sum(1 for s in sessions if is_active_root(s, now_ms, active_ids))


def summarize_cap(sessions, active_ids, now_ms):
    """Summarize capacity: (live_root_sessions, active_root_sessions, cap_room).
    
    Either server read failing makes active count UNKNOWN (None), never zero.
    """
    if sessions is None:
        return None, None, None
    
    roots = [s for s in sessions if is_root_session(s)]
    if active_ids is None:
        return len(roots), None, None
    
    active = count_active_roots(roots, now_ms, active_ids)
    return len(roots), active, max(0, CAP - active)


def fetch_claimed(queue):
    """Fetch claimed tasks for a queue via queue-broker.
    
    Returns list of task rows. Raises on any broker failure.
    """
    try:
        with open(BROKER_TOKEN_FILE) as f:
            token = f.read().strip()
    except Exception as e:
        raise RuntimeError(f"Cannot read broker token: {e}")
    
    body = json.dumps({
        "queue": queue,
        "status": "claimed",
        "limit": 500,
        "rich": True
    }).encode()
    
    req = urllib.request.Request(
        os.environ.get("REPO_QUEUE_BROKER_URL", "http://100.99.113.103:4801").rstrip("/") + "/list",
        data=body,
        method="POST"
    )
    req.add_header("X-Agent-Token", token)
    req.add_header("Content-Type", "application/json")
    
    try:
        with urllib.request.urlopen(req, timeout=60) as res:
            data = json.loads(res.read() or "null")
    except Exception as e:
        raise RuntimeError(f"queue-broker /list failed for {queue}: {e}")
    
    if not isinstance(data, dict) or data.get("ok") is not True:
        raise RuntimeError(f"queue-broker /list failed for {queue}: {data!r:.200}")
    
    return data.get("rows") or []


def is_candidate(row):
    """Check if a claimed task is a claimant candidate.
    
    Candidate iff:
    - progress.lane == "opencode"
    - no session_id bound
    - progress.stage in ("handed_off", "launch_failed")
    - lane_id is not a UUID (not an OpenChamber scheduled lane)
    """
    prog = row.get("progress") or {}
    if prog.get("lane") != "opencode" or prog.get("session_id"):
        return False
    
    if prog.get("stage") not in ("handed_off", "launch_failed"):
        return False
    
    lane_id = row.get("lane_id")
    if lane_id and UUID_LANE.match(lane_id):
        return False
    
    return True


def parse_lease_ms(value):
    """Parse ISO-8601 lease_until to epoch ms, or None."""
    if not isinstance(value, str) or not value:
        return None
    
    text = value[:-1] + "+00:00" if value.endswith("Z") else value
    try:
        dt = datetime.fromisoformat(text)
    except ValueError:
        return None
    
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return int(dt.timestamp() * 1000)


def is_stuck_bound(row, session, active_ids, now_ms):
    """Check if a bound task is stuck (created-but-never-prompted and dead).
    
    Stuck iff:
    - lane == "opencode"
    - has session_id bound
    - lease expired
    - session is present as root with no terminal marker
    - NOT in active set
    - silent for >= ACTIVE_WINDOW_MS
    """
    prog = row.get("progress") or {}
    if prog.get("lane") != "opencode":
        return False
    
    session_id = prog.get("session_id")
    if not session_id:
        return False
    
    lease_ms = parse_lease_ms(row.get("lease_until"))
    if lease_ms is None or lease_ms > now_ms:
        return False
    
    if session is None or not is_root_session(session):
        return False
    
    if active_ids is not None and session_id in active_ids:
        return False
    
    t = session.get("time") or {}
    if t.get("idle") or session.get("outcome"):
        return False
    
    updated = t.get("updated") or t.get("created") or 0
    return now_ms - updated >= ACTIVE_WINDOW_MS


UUID_LANE = re.compile(
    r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
)


def discover_candidates_and_stuck():
    """Discover candidates and stuck_bound tasks.
    
    Returns {
        "queues": {queue: {"claimed": n, "candidates": [...], "stuck_bound": [...]}},
        "candidates": [...],
        "stuck_bound": [...],
        "live_root_sessions": int | None,
        "active_root_sessions": int | None,
        "cap": 36,
        "cap_room": int | None
    }
    """
    now_ms = int(time.time() * 1000)
    sessions = fetch_sessions()
    active_ids = fetch_active_ids() if sessions is not None else None
    
    session_by_id = {}
    if sessions:
        for s in sessions:
            if isinstance(s, dict) and s.get("id"):
                session_by_id[s["id"]] = s
    
    out = {"queues": {}, "candidates": [], "stuck_bound": []}
    
    for q in QUEUES:
        try:
            rows = fetch_claimed(q)
        except Exception as e:
            # Fail closed: cannot discover candidates if broker is unreachable
            raise RuntimeError(f"Failed to fetch claimed tasks for {q}: {e}")
        
        cands = []
        stuck = []
        
        for r in rows:
            prog = r.get("progress") or {}
            if is_candidate(r):
                cands.append({
                    "id": r["id"],
                    "queue": q,
                    "priority": r.get("priority"),
                    "worker_id": r.get("worker_id"),
                    "lease_until": r.get("lease_until"),
                    "lane_id": r.get("lane_id"),
                    "issue_number": prog.get("issue_number"),
                })
            
            session_id = prog.get("session_id")
            if session_id:
                s = session_by_id.get(session_id)
                if is_stuck_bound(r, s, active_ids, now_ms):
                    sess_t = s.get("time") or {}
                    stuck.append({
                        "id": r["id"],
                        "queue": q,
                        "priority": r.get("priority"),
                        "worker_id": r.get("worker_id"),
                        "session_id": session_id,
                        "lease_until": r.get("lease_until"),
                        "attempts": r.get("attempts"),
                        "max_attempts": r.get("max_attempts"),
                        "session_updated": sess_t.get("updated") or sess_t.get("created"),
                        "issue_number": prog.get("issue_number"),
                    })
        
        out["queues"][q] = {
            "claimed": len(rows),
            "candidates": cands,
            "stuck_bound": stuck,
        }
        out["candidates"].extend(cands)
        out["stuck_bound"].extend(stuck)
    
    out["candidates"].sort(key=lambda c: (c["priority"] is None, c["priority"] or 0))
    out["stuck_bound"].sort(key=lambda c: (c["priority"] is None, c["priority"] or 0))
    out["cap"] = CAP
    
    live, active, room = summarize_cap(sessions, active_ids, now_ms)
    out["live_root_sessions"] = live
    out["active_root_sessions"] = active
    out["cap_room"] = room
    
    return out


def supervision(mode, stuck_bound):
    """Perform supervision on stuck_bound tasks (step 1 of prompt.md).
    
    In shadow mode: only report, never act.
    In live mode: attempt recovery for eligible tasks.
    
    Returns list of skip reasons for stuck_bound tasks.
    """
    skips = []
    
    for task in stuck_bound:
        task_id = task["id"]
        session_id = task["session_id"]
        attempts = task.get("attempts", 0)
        max_attempts = task.get("max_attempts", 5)
        
        # Fetch session messages to verify zero-message state
        try:
            with open(AUTH_FILE) as f:
                password = json.load(f)["password"]
        except Exception:
            skips.append(f"Task {task_id}: supervision failed (cannot read auth)")
            continue
        
        req = urllib.request.Request(
            f"{OPENCODE_URL.rstrip('/')}/api/session/{session_id}/messages"
        )
        token = base64.b64encode(f"opencode:{password}".encode()).decode()
        req.add_header("Authorization", f"Basic {token}")
        
        try:
            with urllib.request.urlopen(req, timeout=15) as res:
                messages = json.loads(res.read() or "[]")
        except Exception:
            # Cannot verify state; treat as unknown
            skips.append(f"Task {task_id}: supervision unknown (cannot fetch messages)")
            continue
        
        # If session has any messages or is active, leave it bound
        if messages or isinstance(messages, list):
            skips.append(f"Task {task_id}: bound session {session_id} has activity (not stuck)")
            continue
        
        # Session has zero messages; verify it exists
        req_check = urllib.request.Request(
            f"{OPENCODE_URL.rstrip('/')}/api/session/{session_id}"
        )
        req_check.add_header("Authorization", f"Basic {token}")
        
        try:
            with urllib.request.urlopen(req_check, timeout=15):
                pass  # Session exists
        except urllib.error.HTTPError as e:
            if e.code == 404:
                # Session is gone
                if mode == "live" and attempts < max_attempts:
                    # Attempt recovery with gone-session
                    if recover_task(task_id, session_id, "gone-session"):
                        skips.append(f"Task {task_id}: recovered (session gone, attempts={attempts+1})")
                    else:
                        skips.append(f"Task {task_id}: recovery failed (session gone)")
                else:
                    skips.append(f"Task {task_id}: session gone, attempts exhausted or mode={mode}")
                continue
            else:
                skips.append(f"Task {task_id}: supervision failed (session check error)")
                continue
        except Exception:
            skips.append(f"Task {task_id}: supervision failed (cannot verify session existence)")
            continue
        
        # Session exists and has zero messages; delete it
        if mode == "live":
            try:
                req_del = urllib.request.Request(
                    f"{OPENCODE_URL.rstrip('/')}/api/session/{session_id}",
                    method="DELETE"
                )
                req_del.add_header("Authorization", f"Basic {token}")
                with urllib.request.urlopen(req_del, timeout=15):
                    pass  # Deleted successfully
            except Exception as e:
                skips.append(f"Task {task_id}: delete failed ({e}), keeping bound")
                continue
        
        # Attempt recovery
        if mode == "live" and attempts < max_attempts:
            if recover_task(task_id, session_id, "zero-message-deleted"):
                skips.append(f"Task {task_id}: recovered (zero messages, attempts={attempts+1})")
            else:
                skips.append(f"Task {task_id}: recovery failed (zero messages)")
        else:
            skips.append(f"Task {task_id}: zero messages, attempts exhausted or mode={mode}")
    
    return skips


def recover_task(task_id, session_id, reason):
    """Attempt to recover a task by clearing its binding atomically.
    
    Uses: repo-queue bind --worker opencode-claimant --session-id <old> --launched-by opencode-claimant-recovery:<reason>
    
    Returns True if recovery succeeded, False otherwise.
    """
    # Build the bind command
    cmd = [
        "repo-queue", "bind", str(task_id),
        "--worker", "opencode-claimant",
        "--session-id", session_id,
        "--launched-by", f"opencode-claimant-recovery:{reason}"
    ]
    
    # Execute the bind command
    try:
        result = subprocess.run(
            cmd,
            cwd=REPO_PRIMARY,
            capture_output=True,
            text=True,
            timeout=30
        )
        
        if result.returncode == 0:
            return True
        else:
            print(f"Recovery bind failed for task {task_id}: {result.stderr}", file=sys.stderr)
            return False
    except Exception as e:
        print(f"Recovery bind exception for task {task_id}: {e}", file=sys.stderr)
        return False


def check_queue_health():
    """Check health of all four queues.
    
    Returns True if all queues show 0 claimed-with-lane-opencode unbound candidates.
    """
    try:
        data = discover_candidates_and_stuck()
        total_candidates = len(data["candidates"])
        return total_candidates == 0
    except Exception:
        # If discovery fails, fail closed: do not gate
        return False


def create_session(task_id, queue, issue_number, route_model):
    """Create a worker session on the OpenCode server.
    
    Returns (session_id, session_object) or (None, None) on failure.
    """
    try:
        with open(AUTH_FILE) as f:
            password = json.load(f)["password"]
    except Exception as e:
        print(f"Cannot read auth for session creation: {e}", file=sys.stderr)
        return None, None
    
    # Build session creation payload
    title = f"Task {task_id} ({queue})"
    if issue_number:
        title += f" issue #{issue_number}"
    
    payload = {
        "location": {
            "directory": REPO_PRIMARY
        },
        "title": title,
        "model": {
            "id": route_model,
            "providerID": "opencode-route"
        },
        "agent": "build"
    }
    
    req = urllib.request.Request(
        f"{OPENCODE_URL.rstrip('/')}/api/session",
        data=json.dumps(payload).encode(),
        method="POST"
    )
    token = base64.b64encode(f"opencode:{password}".encode()).decode()
    req.add_header("Authorization", f"Basic {token}")
    req.add_header("Content-Type", "application/json")
    
    try:
        with urllib.request.urlopen(req, timeout=30) as res:
            session = json.loads(res.read())
    except Exception as e:
        print(f"Session creation failed for task {task_id}: {e}", file=sys.stderr)
        return None, None

    # The v2 server wraps the created session in a {"data": ...}
    # envelope; the session (id, location) lives inside it.
    if isinstance(session, dict) and isinstance(session.get("data"), dict):
        session = session["data"]
    
    # Verify location.directory matches requested
    if session.get("location", {}).get("directory") != REPO_PRIMARY:
        print(f"Session created at wrong location for task {task_id}: {session.get('location')}", file=sys.stderr)
        # Clean up the misplaced session
        try:
            req_del = urllib.request.Request(
                f"{OPENCODE_URL.rstrip('/')}/api/session/{session.get('id')}",
                method="DELETE"
            )
            req_del.add_header("Authorization", f"Basic {token}")
            urllib.request.urlopen(req_del, timeout=15)
        except Exception:
            pass  # Best effort cleanup
        return None, None
    
    return session.get("id"), session


def bind_and_checkpoint(task_id, worker_id, session_id):
    """Bind task to session and checkpoint launched stage.
    
    Uses: repo-queue bind and repo-queue checkpoint
    
    Returns True if successful, False otherwise.
    """
    # Bind: repo-queue bind <id> --worker opencode-claimant --session-id <session_id>
    bind_cmd = [
        "repo-queue", "bind", str(task_id),
        "--worker", "opencode-claimant",
        "--session-id", session_id
    ]
    
    try:
        result = subprocess.run(
            bind_cmd,
            cwd=REPO_PRIMARY,
            capture_output=True,
            text=True,
            timeout=30
        )
        
        if result.returncode != 0:
            print(f"Bind failed for task {task_id}: {result.stderr}", file=sys.stderr)
            return False
    except Exception as e:
        print(f"Bind exception for task {task_id}: {e}", file=sys.stderr)
        return False
    
    # Checkpoint: repo-queue checkpoint <id> --worker <worker_id> --stage launched
    checkpoint_cmd = [
        "repo-queue", "checkpoint", str(task_id),
        "--worker", worker_id,
        "--stage", "launched"
    ]
    
    try:
        result = subprocess.run(
            checkpoint_cmd,
            cwd=REPO_PRIMARY,
            capture_output=True,
            text=True,
            timeout=30
        )
        
        if result.returncode != 0:
            print(f"Checkpoint failed for task {task_id}: {result.stderr}", file=sys.stderr)
            return False
    except Exception as e:
        print(f"Checkpoint exception for task {task_id}: {e}", file=sys.stderr)
        return False
    
    return True


def send_prompt(task_id, worker_id, queue, issue_number, session_id):
    """Send the worker prompt to the bound session.
    
    The prompt instructs the worker to work THAT task and use proper worker ID.
    """
    # Build the prompt content
    prompt_lines = [
        f"Task {task_id} from queue {queue}",
        "",
        "Execute this task using:",
        f"  repo-queue --worker {worker_id} heartbeat {task_id}",
        f"  repo-queue --worker {worker_id} checkpoint {task_id} --stage in_progress",
        "  ... work ...",
        f"  repo-queue --worker {worker_id} complete {task_id}",
        "",
        "Ensure --worker <worker_id> is used on every repo-queue verb.",
        "Finish with terminal verbs carrying:",
        f'  --execution {{"lane": "opencode", "session_id": "{session_id}"}}'
    ]
    
    prompt_text = "\n".join(prompt_lines)
    
    # Send to OpenCode server as a user message
    try:
        with open(AUTH_FILE) as f:
            password = json.load(f)["password"]
    except Exception as e:
        print(f"Cannot read auth for prompt send: {e}", file=sys.stderr)
        return False
    
    payload = {
        "text": prompt_text
    }

    req = urllib.request.Request(
        f"{OPENCODE_URL.rstrip('/')}/api/session/{session_id}/prompt",
        data=json.dumps(payload).encode(),
        method="POST"
    )
    token = base64.b64encode(f"opencode:{password}".encode()).decode()
    req.add_header("Authorization", f"Basic {token}")
    req.add_header("Content-Type", "application/json")
    
    try:
        with urllib.request.urlopen(req, timeout=30) as res:
            result = json.loads(res.read())
        return True
    except Exception as e:
        print(f"Prompt send failed for task {task_id} session {session_id}: {e}", file=sys.stderr)
        return False


def post_agent_note(task_id, session_id):
    """Post agent-note:v1 on the issue.
    
    Note: agent-note:v1 state=handed off
    """
    # Use repo-queue signal to post the note
    cmd = [
        "repo-queue", "signal", str(task_id),
        "--worker", "opencode-claimant",
        "--kind", "agent-note:v1",
        "--state", "handed off",
        "--body", f"Opencode claimant launched task {task_id} with session {session_id}"
    ]
    
    try:
        result = subprocess.run(
            cmd,
            cwd=REPO_PRIMARY,
            capture_output=True,
            text=True,
            timeout=30
        )
        
        if result.returncode != 0:
            print(f"Agent note failed for task {task_id}: {result.stderr}", file=sys.stderr)
            return False
        return True
    except Exception as e:
        print(f"Agent note exception for task {task_id}: {e}", file=sys.stderr)
        return False




def delete_session(session_id):
    """Best-effort DELETE of a created session (cleanup after a failed
    bind/prompt, so a created-but-unbound session never lingers)."""
    try:
        with open(AUTH_FILE) as f:
            password = json.load(f)["password"]
    except Exception:
        return
    try:
        req = urllib.request.Request(
            f"{OPENCODE_URL.rstrip('/')}/api/session/{session_id}",
            method="DELETE"
        )
        token = base64.b64encode(f"opencode:{password}".encode()).decode()
        req.add_header("Authorization", f"Basic {token}")
        urllib.request.urlopen(req, timeout=15)
    except Exception:
        pass  # Best effort cleanup


def launch_candidate(candidate, route_model):
    """Launch one candidate: re-verify, create, bind, checkpoint,
    prompt, note. Returns (launched_entry | None, [skip reasons]).

    Exactly-once is anchored on the atomic DB bind
    (bind_repo_task_lane_session): the candidate list holds each task
    exactly once, and the bind itself is atomic - a second bind of
    the same task is rejected and the loser session is deleted.
    """
    task_id = candidate["id"]
    queue = candidate["queue"]
    worker_id = candidate["worker_id"]
    priority = candidate["priority"]
    issue_number = candidate.get("issue_number")
    skips = []

    print(f"Considering task {task_id} (priority {priority}) from {queue}", file=sys.stderr)

    # Re-verify candidate status
    try:
        rows = fetch_claimed(queue)
        task_row = next((r for r in rows if r["id"] == task_id), None)
        if not task_row or not is_candidate(task_row):
            return None, [f"Task {task_id}: no longer a candidate"]
    except Exception:
        return None, [f"Task {task_id}: verification failed"]

    # Create session
    session_id, session = create_session(task_id, queue, issue_number, route_model)
    if not session_id:
        return None, [f"Task {task_id}: session creation failed"]

    # Bind and checkpoint
    if not bind_and_checkpoint(task_id, worker_id, session_id):
        delete_session(session_id)
        return None, [f"Task {task_id}: bind/checkpoint failed"]

    # Send prompt
    if not send_prompt(task_id, worker_id, queue, issue_number, session_id):
        delete_session(session_id)
        return None, [f"Task {task_id}: prompt send failed"]

    # Post agent note
    if not post_agent_note(task_id, session_id):
        skips.append(f"Task {task_id}: agent note failed")

    print(f"Launched task {task_id} with session {session_id}", file=sys.stderr)
    return {
        "task_id": task_id,
        "session_id": session_id,
        "queue": queue,
        "priority": priority
    }, skips


# Concurrent launches within one fire: distinct candidates launch in
# parallel under a bounded pool; the atomic DB bind keeps every task
# exactly-once (see launch_candidate).
LAUNCH_WORKERS = 4


def launch_candidates(candidates, cap_room, route_model):
    """Launch up to cap_room candidates, LAUNCH_WORKERS at a time.

    A skip does not consume room - the next candidate backfills,
    exactly as the previous serial loop behaved. Returns (launched,
    skips) with launched in candidate (priority) order.
    """
    launched_by_id = {}
    skips = []
    pending = list(candidates)
    in_flight = {}
    with ThreadPoolExecutor(max_workers=LAUNCH_WORKERS) as pool:
        while pending or in_flight:
            while pending and len(in_flight) < LAUNCH_WORKERS \
                    and len(launched_by_id) + len(in_flight) < cap_room:
                cand = pending.pop(0)
                in_flight[pool.submit(launch_candidate, cand, route_model)] = cand
            if not in_flight:
                break
            done, _ = wait(in_flight, return_when=FIRST_COMPLETED)
            for fut in done:
                cand = in_flight.pop(fut)
                entry, cand_skips = fut.result()
                skips.extend(cand_skips)
                if entry:
                    launched_by_id[cand["id"]] = entry
    launched = [launched_by_id[cand["id"]] for cand in candidates
                if cand["id"] in launched_by_id]
    return launched, skips


def acquire_fire_lock():
    """Single-instance guard: overlapping fires must never launch
    concurrently - two claimants racing one candidate list is the one
    double-bind path the atomic bind should never be asked to
    arbitrate. Returns the open lock file object, or None when another
    fire already holds the lock."""
    lock_path = Path.home() / ".local" / "state" / "opencode-claimant" / "claimant.lock"
    try:
        lock_path.parent.mkdir(parents=True, exist_ok=True)
        fh = open(lock_path, "w")
        fcntl.flock(fh, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return fh
    except Exception:
        return None


def main():
    """Main entry point for the claimant."""
    mode = read_mode()
    print(f"OpenCode lane claimant running in {mode} mode", file=sys.stderr)

    lock_fh = acquire_fire_lock()
    if lock_fh is None:
        print("Another claimant fire holds the lock; ending without action", file=sys.stderr)
        return 0
    
    # Step 1: SUPERVISION FIRST
    try:
        data = discover_candidates_and_stuck()
        stuck_bound = data.get("stuck_bound", [])
    except Exception as e:
        print(f"Discovery failed: {e}", file=sys.stderr)
        stuck_bound = []
    
    skips = supervision(mode, stuck_bound)
    
    # Step 2: GATE
    if not check_queue_health():
        print("Gate: not all queues healthy or have candidates", file=sys.stderr)
    
    # Step 3: CAP
    cap_room = data.get("cap_room") if "data" in locals() else None
    if cap_room == 0 or cap_room is None:
        print(f"Cap: cap_room={cap_room}, launching nothing", file=sys.stderr)
        
        # Still log the fire with what we discovered
        candidates = data.get("candidates", []) if "data" in locals() else []
        launched = []
        log_fire(mode, candidates, launched, cap_room, skips)
        return 0
    
    # Step 4: LAUNCH CANDIDATES
    candidates = data.get("candidates", [])

    # Shadow mode: discover, verify, and log what WOULD launch - never
    # claim, bind, launch, or mutate a task (prompt.md MODE rule).
    if mode != "live":
        skips.append("shadow mode: no launches (discovery + report only)")
        log_fire(mode, candidates, [], cap_room, skips)
        return 0

    # Route model: use packet's route if present, else workhorse default
    route_model = "workhorse"  # Default

    launched, launch_skips = launch_candidates(candidates, cap_room, route_model)
    skips.extend(launch_skips)
    cap_room -= len(launched)

    # Step 6: FIRE LOG
    log_fire(mode, candidates, launched, cap_room, skips)
    
    return 0


if __name__ == "__main__":
    sys.exit(main())
