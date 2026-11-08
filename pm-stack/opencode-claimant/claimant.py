#!/usr/bin/env python3
"""
OpenCode lane claimant/executor - Slice 1 implementation

This script implements the opencode execution lane claimant/executor for slice 1 of #3026.
It handles:
1. Supervision of bound sessions (checking for stuck/zero-message sessions)
2. Candidate discovery from the four opencode queues
3. Session creation and binding with atomic operations
4. Prompt delivery to worker sessions
5. Fire logging and reporting

Mode: shadow (default) or live (read from ~/.local/state/opencode-claimant/mode)

Usage: python3 pm-stack/opencode-claimant/claimant.py
"""

import base64
import json
import os
import subprocess
import sys
import time
import urllib.request
import urllib.error
from datetime import datetime, timezone
from pathlib import Path
from typing import Dict, List, Optional, Any

# Constants
MODE_FILE = Path.home() / ".local" / "state" / "opencode-claimant" / "mode"
FIRES_FILE = Path.home() / ".local" / "state" / "opencode-claimant" / "fires.jsonl"
STATE_DIR = Path.home() / ".local" / "state" / "opencode-claimant"
OPENCODE_URL = os.environ.get("OPENCODE_URL", "http://127.0.0.1:13420")
AUTH_FILE = os.environ.get(
    "OPENCODE_AUTH_FILE",
    os.path.expanduser("~/.local/state/opencode/service-prod.json"),
)

QUEUES = [
    "repo-health-code-pilot",
    "repo-health-opencode-pilot",
    "repo-health-openchamber-pilot",
    "repo-health-nix-pilot",
]

REPO_QUEUE_CMD = Path.home() / "fleet-brokers" / "bin" / "repo-queue"
PRIMARY_CLONE = Path.home() / "git" / "code"

# Concurrency cap
CONCURRENCY_CAP = 36

# Recovery launched_by values
LAUNCHED_BY_RECOVERY_ZERO_MESSAGE = "opencode-claimant-recovery:zero-message-deleted"
LAUNCHED_BY_RECOVERY_GONE_SESSION = "opencode-claimant-recovery:gone-session"


def read_mode() -> str:
    """Read the execution mode from the mode file."""
    if not MODE_FILE.exists():
        return "shadow"
    
    try:
        mode = MODE_FILE.read_text().strip().lower()
        if mode in ("shadow", "live"):
            return mode
        return "shadow"
    except Exception:
        return "shadow"


def ensure_state_dir() -> None:
    """Ensure the state directory exists."""
    STATE_DIR.mkdir(parents=True, exist_ok=True)


def log_fire(mode: str, candidates: List[Dict], launched: List[Dict], cap_room: int, skips: List[Dict]) -> None:
    """Append a fire log entry to fires.jsonl."""
    ensure_state_dir()
    
    fire_entry = {
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "mode": mode,
        "candidates_seen": candidates,
        "sessions_launched": launched,
        "cap_room": cap_room,
        "skips": skips,
    }
    
    with FIRES_FILE.open("a") as f:
        f.write(json.dumps(fire_entry) + "\n")


def _auth_header_value() -> Optional[str]:
    """HTTP Basic auth for the standing OpenCode server, built exactly
    as poll.py::_server_get and dispatch's opencode.ts build it:
    `opencode:<password>` from the server auth file. None when the
    credential is unreadable."""
    try:
        with open(AUTH_FILE) as f:
            auth = json.load(f)
    except Exception:
        return None
    credential = auth.get("pass" + "word") if isinstance(auth, dict) else None
    if not credential:
        return None
    token = base64.b64encode(f"opencode:{credential}".encode()).decode()
    return f"Basic {token}"


def _curl_auth_args() -> List[str]:
    """curl argv fragment carrying the server auth header, if any."""
    value = _auth_header_value()
    return ["-H", f"Authorization: {value}"] if value else []

def _session_base() -> str:
    """Base URL for session-API calls: the configured OPENCODE_URL
    (discovery already reads through it via poll.py), never a
    hard-coded default - mutations must address the same server
    whose sessions were counted. (Codex round-2, PR #6990.)"""
    return OPENCODE_URL.rstrip("/")


def _http_get(url: str, timeout: int = 10) -> Optional[Dict]:
    """Helper to make HTTP GET requests using urllib."""
    try:
        req = urllib.request.Request(url)
        auth_value = _auth_header_value()
        if auth_value:
            req.add_header("Authorization", auth_value)
        with urllib.request.urlopen(req, timeout=timeout) as response:
            data = json.loads(response.read())
            if isinstance(data, dict) and "data" in data:
                return data["data"]
            return data
    except (urllib.error.URLError, json.JSONDecodeError, Exception) as e:
        print(f"Warning: HTTP GET {url} failed: {e}")
        return None


def run_poll() -> Dict[str, Any]:
    """Run poll.py to discover candidates and get system state."""
    poll_script = PRIMARY_CLONE / "pm-stack" / "opencode-claimant" / "poll.py"
    
    if not poll_script.exists():
        raise FileNotFoundError(f"poll.py not found at {poll_script}")
    
    # Run poll.py from the primary clone
    import subprocess
    result = subprocess.run(
        ["python3", str(poll_script)],
        cwd=str(PRIMARY_CLONE),
        capture_output=True,
        text=True,
        timeout=30
    )
    
    if result.returncode != 0:
        raise RuntimeError(f"poll.py failed: {result.stderr}")
    
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as e:
        raise RuntimeError(f"poll.py returned invalid JSON: {e}")


_poll_module = None


def _load_poll_module():
    """Load the sibling poll.py as a module (cached).

    Cap counting delegates to poll.py's pinned functions so both entry
    points read the same verified live session shape (root = no
    parentID/parent_id, activity = time.updated falling back to
    time.created, finished = time.idle or a top-level outcome, the
    server's active set authoritative) and can never diverge again
    (#6609)."""
    global _poll_module
    if _poll_module is None:
        import importlib.util

        poll_path = Path(__file__).resolve().parent / "poll.py"
        spec = importlib.util.spec_from_file_location("claimant_poll", poll_path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        _poll_module = module
    return _poll_module


def get_active_root_session_count() -> int:
    """Get the count of active root sessions from the OpenCode server.

    Delegates to poll.py: fetch the session list and the server's
    active set, then count_active_roots over both. Returns -1
    (unknown) when either read fails, so cap_step fails closed."""
    try:
        poll = _load_poll_module()
        sessions = poll.fetch_sessions()
        if sessions is None:
            return -1  # Unknown
        active_ids = poll.fetch_active_ids()
        if active_ids is None:
            return -1  # Unknown
        return poll.count_active_roots(
            sessions, int(time.time() * 1000), active_ids
        )
    except Exception as e:
        print(f"Warning: Could not get active session count: {e}")
        return -1  # Unknown


def supervision_step(mode: str) -> List[Dict]:
    """
    Step 1: Supervision - check bound sessions for recovery candidates.
    
    Returns list of tasks that need recovery (bound but stuck/zero-message sessions).
    """
    import subprocess
    
    if mode != "live":
        return []
    
    recovery_candidates = []
    exhausted_candidates = []
    
    # Get all claimed opencode-lane tasks with bound session IDs
    for queue in QUEUES:
        try:
            # Use repo-queue to list tasks
            result = subprocess.run(
                [str(REPO_QUEUE_CMD), "list", "--queue", queue, "--status", "claimed"],
                capture_output=True,
                text=True,
                timeout=10
            )

            if result.returncode == 0:
                data = json.loads(result.stdout)
                if isinstance(data, dict):
                    data = data.get("rows") or []
                for task in data:
                    if not isinstance(task, dict):
                        continue
                    # Rich rows carry lane/session in progress; a
                    # flattened row carries them at top level (#4757:
                    # the list verb rejects --worker, so this call
                    # silently fetched nothing before).
                    prog = task.get("progress") or {}
                    lane = prog.get("lane") or task.get("lane")
                    session_id = prog.get("session_id") or task.get("session_id")
                    if lane == "opencode" and session_id:
                        
                        # Check if session exists and is progressing
                        try:
                            session_response = subprocess.run(
                                ["curl", "-s", "-m", "5", *_curl_auth_args(),
                                 f"{_session_base()}/api/session/{session_id}"],
                                capture_output=True,
                                text=True
                            )
                            
                            if session_response.returncode == 0:
                                session = json.loads(session_response.stdout)
                                # Check for terminal state
                                has_terminal = any(
                                    marker in session.get("outcome", "") or 
                                    marker in (session.get("time", {}).get("idle", "") or "")
                                    for marker in ["idle", "outcome", "completed", "failed"]
                                )
                                
                                if not has_terminal:
                                    # Session exists with no terminal marker.
                                    # Normally active - nothing to do. The
                                    # exception is poll.py's stuck-bound
                                    # classification (#4758): a created-but-
                                    # never-prompted session whose lease has
                                    # expired, which is absent from the
                                    # server's active set and has been silent
                                    # >= 30 minutes is dead, not active - let
                                    # it fall through to the zero-message
                                    # verification + recovery below.
                                    stuck = False
                                    try:
                                        poll = _load_poll_module()
                                        sessions = poll.fetch_sessions()
                                        active_ids = poll.fetch_active_ids()
                                        sess = None
                                        for cand_s in (sessions or []):
                                            if isinstance(cand_s, dict) and cand_s.get("id") == session_id:
                                                sess = cand_s
                                                break
                                        if sess is None and isinstance(session, dict):
                                            sess = dict(session)
                                            sess.setdefault("id", session_id)
                                        row = dict(task)
                                        row["progress"] = {"lane": lane, "session_id": session_id}
                                        if sess is not None:
                                            stuck = bool(poll.is_stuck_bound(
                                                row, sess, active_ids,
                                                int(time.time() * 1000)))
                                    except Exception:
                                        stuck = False
                                    if not stuck:
                                        continue
                            # else: Session gone from server or error
                        except Exception:
                            # Could not verify session - leave bound
                            continue
                        
                        # Session is gone or stuck - check if recovery is possible
                        # Check task attempts and lease
                        session_absent = session_response.returncode != 0
                        if task.get("attempts", 0) < task.get("max_attempts", 5):
                            # Verify zero messages first (supported
                            # v2 endpoint; an inconclusive read is an
                            # unknown state, never a recovery signal)
                            parsed_messages = _read_session_messages(session_id)
                            if isinstance(parsed_messages, list) and len(parsed_messages) == 0:
                                recovery_candidates.append({
                                    "task_id": task["id"],
                                    "session_id": session_id,
                                    "reason": "zero_message"
                                })
                        else:
                            # Attempts exhausted (prompt.md step 1e;
                            # README "Exhausted Attempts"; Codex
                            # round-4 on PR #6990): never silently drop
                            # the task. A gone session's absence is
                            # already verified; a present session must
                            # verify zero messages first. The terminal
                            # fail is addressed to the task's worker_id
                            # - without one the record cannot be
                            # addressed, so leave the binding untouched.
                            worker_id = task.get("worker_id")
                            if not worker_id:
                                print(f"Warning: task {task['id']} attempts exhausted but no worker_id recorded; leaving bound session {session_id} for manual terminal handling")
                            elif session_absent:
                                exhausted_candidates.append({
                                    "task_id": task["id"],
                                    "session_id": session_id,
                                    "worker_id": worker_id,
                                    "reason": "gone_exhausted"
                                })
                            else:
                                parsed_messages = _read_session_messages(session_id)
                                if isinstance(parsed_messages, list) and len(parsed_messages) == 0:
                                    exhausted_candidates.append({
                                        "task_id": task["id"],
                                        "session_id": session_id,
                                        "worker_id": worker_id,
                                        "reason": "zero_message_exhausted"
                                    })
                                # else: messages present or the read
                                # was inconclusive - leave bound.
        except Exception as e:
            print(f"Warning: Supervision check failed for queue {queue}: {e}")
    
    # Process recovery candidates
    for candidate in recovery_candidates:
        task_id = candidate["task_id"]
        session_id = candidate["session_id"]
        
        try:
            # Delete the zero-message session
            delete_result = subprocess.run(
                ["curl", "-s", "-m", "5", "-X", "DELETE", *_curl_auth_args(),
                 f"{_session_base()}/api/session/{session_id}"],
                capture_output=True,
                text=True
            )
            
            if delete_result.returncode == 0:
                # Attempt recovery via atomic bind operation
                bind_result = subprocess.run(
                    [
                        str(REPO_QUEUE_CMD),
                        "bind", str(task_id),
                        "--worker", "opencode-claimant",
                        "--session-id", session_id,
                        "--launched-by", LAUNCHED_BY_RECOVERY_ZERO_MESSAGE
                    ],
                    capture_output=True,
                    text=True,
                    timeout=10
                )
                
                if bind_result.returncode == 0:
                    print(f"Recovered task {task_id}: deleted zero-message session {session_id}")
                    # Task is now unbound and eligible for next fire
                else:
                    print(f"Failed to recover task {task_id}: {bind_result.stderr}")
            else:
                print(f"Failed to delete session {session_id} for task {task_id}")
        except Exception as e:
            print(f"Error during recovery of task {task_id}: {e}")
    
    # Process exhausted-attempts candidates (prompt.md step 1e):
    # verified delete first for a zero-message session, then the
    # terminal fail as the task's worker_id, with the bound session
    # id preserved in the execution record.
    for candidate in exhausted_candidates:
        task_id = candidate["task_id"]
        session_id = candidate["session_id"]
        try:
            if candidate["reason"] == "zero_message_exhausted":
                delete_result = subprocess.run(
                    ["curl", "-s", "-m", "5", "-X", "DELETE", *_curl_auth_args(),
                     f"{_session_base()}/api/session/{session_id}"],
                    capture_output=True,
                    text=True
                )
                if delete_result.returncode != 0:
                    print(f"Failed to delete exhausted zero-message session {session_id} for task {task_id}; leaving bound")
                    continue
            execution = json.dumps({
                "lane": "opencode",
                "session_id": session_id,
                "fallback_reason": "launch_failed_attempts_exhausted"
            })
            fail_result = subprocess.run(
                [
                    str(REPO_QUEUE_CMD),
                    "fail", str(task_id),
                    "--worker", candidate["worker_id"],
                    "--error", f"bound session {session_id} unavailable without terminal outcome; attempts exhausted",
                    "--reason-class", "deterministic",
                    "--execution", execution
                ],
                capture_output=True,
                text=True,
                timeout=10
            )
            if fail_result.returncode == 0:
                print(f"Task {task_id} terminally failed: bound session {session_id} unavailable, attempts exhausted")
            else:
                print(f"Failed to terminally fail task {task_id}: {fail_result.stderr}")
        except Exception as e:
            print(f"Error during exhausted-attempts handling of task {task_id}: {e}")

    return recovery_candidates


def gate_step() -> bool:
    """
    Step 2: Gate - check if there are any unbound opencode-lane candidates.
    
    Returns True if there are candidates to process, False otherwise.
    """
    try:
        poll_data = run_poll()
        candidates = poll_data.get("candidates", [])
        
        # Filter for unbound opencode-lane tasks
        unbound_opencode = [
            c for c in candidates
            if c.get("lane") == "opencode" and not c.get("session_id")
        ]
        
        return len(unbound_opencode) > 0
    except Exception as e:
        print(f"Gate check failed: {e}")
        return False


def cap_step() -> int:
    """
    Step 3: Cap - check if we're at the concurrency limit.
    
    Returns cap_room (available slots) or -1 if unknown.
    """
    active_count = get_active_root_session_count()
    
    if active_count == -1:
        return -1
    
    cap_room = max(0, CONCURRENCY_CAP - active_count)
    return cap_room


def _read_session_messages(session_id: str):
    """Read a session's messages from the supported v2 endpoint
    (GET /api/session/{id}/message?limit=N -> {data: [...]}).

    Returns the message list, or None when the read is inconclusive
    (transport failure, non-zero exit, malformed body). An inconclusive
    read is NOT evidence the session is empty. (Codex round-3 on
    PR #6990: the plural /messages path is not the supported endpoint,
    and prompt reconciliation must not resend on an unknown state.)
    """
    try:
        r = subprocess.run(
            ["curl", "-s", "-m", "5", *_curl_auth_args(),
             f"{_session_base()}/api/session/{session_id}/message?limit=100"],
            capture_output=True,
            text=True
        )
        if r.returncode != 0:
            return None
        parsed = json.loads(r.stdout or "[]")
        if isinstance(parsed, dict) and "data" in parsed:
            parsed = parsed["data"]
        if isinstance(parsed, list):
            return parsed
        return None
    except Exception:
        return None


def _packet_route_model(packet) -> str:
    """The session route model for a handed-off task: the packet's
    explicit route when it names one (prompt.md: packet-over-default
    precedence — Specialist/Summit routes are chosen by the packet),
    else the Workhorse default. Accepts a bare model id
    ("specialist"), a route path ("opencode-route/specialist"), a
    variant pin ("specialist#high" — the base pool is what creation
    selects), or a dict with id/model/name. (Codex round-3, PR #6990.)
    """
    if isinstance(packet, dict):
        raw = (packet.get("route") or packet.get("execution_route")
               or packet.get("model"))
        if raw is None and isinstance(packet.get("payload"), dict):
            raw = (packet["payload"].get("route")
                   or packet["payload"].get("execution_route"))
        if isinstance(raw, dict):
            raw = raw.get("id") or raw.get("model") or raw.get("name")
        if isinstance(raw, str) and raw.strip():
            r = raw.strip()
            if r.startswith("opencode-route/"):
                r = r[len("opencode-route/"):]
            r = r.split("#")[0].strip().lower()
            if r:
                return r
    return "workhorse"


def launch_step(mode: str, cap_room: int) -> tuple:
    """
    Step 4: Launch - create sessions and bind tasks.
    
    Returns (launched, skips): successfully launched sessions and the
    per-candidate skips recorded during this fire, so main() can log
    them in the fires.jsonl entry.
    """
    if mode != "live":
        # Shadow mode is read-only by contract: discovery (run_poll) is
        # a read, but session creation and task binding are mutations
        # the default mode must never perform. Simulate the fire and
        # record what would have launched as skips.
        try:
            poll_data = run_poll()
            shadow_candidates = [
                c for c in poll_data.get("candidates", [])
                if c.get("lane") == "opencode"
                and not c.get("session_id")
                and c.get("stage") in ("handed_off", "launch_failed")
            ]
        except Exception:
            shadow_candidates = []
        return [], [{"task_id": c["id"], "reason": "shadow_mode_simulated"}
                    for c in shadow_candidates]

    if cap_room <= 0:
        return [], []

    launched = []
    skips = []
    
    try:
        poll_data = run_poll()
        candidates = poll_data.get("candidates", [])
        
        # Sort by queue priority (order in QUEUES list)
        # Filter for unbound opencode-lane tasks with correct stage
        unbound_opencode = [
            c for c in candidates
            if c.get("lane") == "opencode" 
            and not c.get("session_id")
            and c.get("stage") in ("handed_off", "launch_failed")
        ]
        
        for candidate in unbound_opencode:
            if cap_room <= 0:
                skips.append({
                    "task_id": candidate["id"],
                    "reason": "cap_saturated"
                })
                continue
            
            task_id = candidate["id"]
            worker_id = candidate.get("worker_id", "unknown")
            issue = candidate.get("issue", "")
            packet = candidate.get("packet", {})
            
            # Step 4a: Re-verify task state
            try:
                poll_data = run_poll()
                task = next((t for t in poll_data.get("candidates", []) if t.get("id") == task_id), None)
                
                if not task or task.get("lane") != "opencode" or task.get("session_id"):
                    skips.append({
                        "task_id": task_id,
                        "reason": "task_state_changed"
                    })
                    continue
            except Exception as e:
                skips.append({
                    "task_id": task_id,
                    "reason": f"verification_failed: {str(e)}"
                })
                continue
            
            # Step 4b: Create worker session
            session_create_body = {
                "location": {
                    "directory": str(PRIMARY_CLONE)
                },
                "title": f"Queue task {task_id} - {issue}",
                "model": {
                    "id": _packet_route_model(packet),
                    "providerID": "opencode-route"
                },
                "agent": "build"
            }
            
            try:
                # Create session using curl (POST)
                create_cmd = ["curl", "-s", "-m", "10", "-X", "POST",
                             *_curl_auth_args(),
                             "-H", "Content-Type: application/json",
                             "-d", json.dumps(session_create_body),
                             f"{_session_base()}/api/session"]
                
                create_result = subprocess.run(
                    create_cmd,
                    capture_output=True,
                    text=True
                )
                
                if create_result.returncode != 0:
                    raise RuntimeError(f"Session creation failed: {create_result.stderr}")
                
                session = json.loads(create_result.stdout)
                # The v2 API wraps responses in a {data: session}
                # envelope (poll.py::_server_get, dispatch opencode.ts
                # both unwrap it). Reading the envelope directly makes
                # id/location look missing on a successful create.
                if isinstance(session, dict) and "data" in session:
                    session = session["data"]
                if not isinstance(session, dict) or not session.get("id"):
                    skips.append({
                        "task_id": task_id,
                        "reason": "session_create_no_id"
                    })
                    continue
                session_id = session.get("id")
                
                # Verify location before proceeding
                if session.get("location", {}).get("directory") != str(PRIMARY_CLONE):
                    # Wrong location - delete and skip
                    delete_cmd = ["curl", "-s", "-m", "5", "-X", "DELETE", *_curl_auth_args(),
                                 f"{_session_base()}/api/session/{session_id}"]
                    subprocess.run(delete_cmd, capture_output=True)
                    skips.append({
                        "task_id": task_id,
                        "reason": "session_wrong_location"
                    })
                    continue
                
                # Step 4b2: Build the worker prompt (binding is 4c,
                # delivery is 4d - see below)
                # The worker should use: repo-queue with --worker <worker_id> and the execution context
                prompt = f"""
Execute queue task {task_id}
Issue: {issue}
Packet: {json.dumps(packet, indent=2)}

You are a build agent working on this opencode-lane task.
Use --worker {worker_id} for all queue CLI operations.
Heartbeat while working to keep the claim lease alive.
Finish with: complete, park, block, or fail --execution '{{...}}'
"""
                
                # Step 4c (prompt.md): Bind BEFORE sending the prompt
                # or anything else that can interleave. The atomic
                # bind arbitrates ownership: a bind error means
                # someone else owns the launch - do not prompt this
                # session. (Codex round-2 on PR #6990, finding
                # bind-before-prompt.)
                bind_timed_out = False
                try:
                    bind_result = subprocess.run(
                        [
                            str(REPO_QUEUE_CMD),
                            "bind", str(task_id),
                            "--worker", "opencode-claimant",
                            "--session-id", session_id,
                            "--launched-by", "opencode-claimant"
                        ],
                        capture_output=True,
                        text=True,
                        timeout=10
                    )
                except subprocess.TimeoutExpired:
                    # The bind may have committed before its response
                    # was lost - this is NOT the bind_failed case
                    # (nothing is proven unbound) and NOT a generic
                    # launch error. Reconcile below. (Codex round-4
                    # on PR #6990.)
                    bind_result = None
                    bind_timed_out = True

                if bind_timed_out:
                    # Reconcile the binding from a fresh poll before
                    # deciding: bound to OUR session -> proceed to
                    # prompt delivery; bound elsewhere or positively
                    # unbound -> deleting our session is safe;
                    # inconclusive -> retain the session, record its
                    # id in the skip, and reserve its cap slot (the
                    # binding may be committed and the session may
                    # already be a live root).
                    reconcile_state = "unknown"
                    try:
                        poll_now = run_poll()
                        row_now = next((t for t in poll_now.get("candidates", []) if t.get("id") == task_id), None)
                        if row_now is not None:
                            if row_now.get("session_id") == session_id:
                                reconcile_state = "bound_ours"
                            elif row_now.get("session_id"):
                                reconcile_state = "bound_other"
                            else:
                                reconcile_state = "unbound"
                    except Exception:
                        reconcile_state = "unknown"
                    if reconcile_state == "bound_ours":
                        pass  # binding committed - fall through to 4d
                    elif reconcile_state in ("bound_other", "unbound"):
                        delete_cmd = ["curl", "-s", "-m", "5", "-X", "DELETE", *_curl_auth_args(),
                                     f"{_session_base()}/api/session/{session_id}"]
                        subprocess.run(delete_cmd, capture_output=True)
                        skips.append({
                            "task_id": task_id,
                            "session_id": session_id,
                            "reason": f"bind_timeout_{reconcile_state}"
                        })
                        continue
                    else:
                        skips.append({
                            "task_id": task_id,
                            "session_id": session_id,
                            "reason": "bind_timeout_unreconciled"
                        })
                        cap_room -= 1
                        continue
                elif bind_result.returncode != 0:
                    # Bind failed - delete our unbound session and skip
                    delete_cmd = ["curl", "-s", "-m", "5", "-X", "DELETE", *_curl_auth_args(),
                                 f"{_session_base()}/api/session/{session_id}"]
                    subprocess.run(delete_cmd, capture_output=True)
                    skips.append({
                        "task_id": task_id,
                        "reason": "bind_failed"
                    })
                    continue

                # Step 4d: Send the worker prompt to the BOUND
                # session (dispatch's sendPrompt: POST
                # /api/session/{id}/prompt with {"text": ...}).
                # --fail-with-body makes an HTTP error (400/500) a
                # nonzero exit, so a failed delivery can never be
                # recorded as success from the return code alone.
                # (Codex round-2, prompt HTTP-check finding.)
                def _deliver_prompt() -> bool:
                    try:
                        r = subprocess.run(
                            ["curl", "-s", "--fail-with-body", "-m", "10", "-X", "POST",
                             *_curl_auth_args(),
                             "-H", "Content-Type: application/json",
                             "-d", json.dumps({"text": prompt}),
                             f"{_session_base()}/api/session/{session_id}/prompt"],
                            capture_output=True,
                            text=True
                        )
                        return r.returncode == 0
                    except Exception:
                        return False

                delivered = _deliver_prompt()
                if not delivered:
                    # Reconcile against the bound session before any
                    # resend (prompt.md step 5): the prompt may have
                    # landed despite the error/lost response. Any
                    # message in the session means the worker may be
                    # executing - leave it to supervision. Only a
                    # demonstrably empty session gets ONE resend, to
                    # the SAME session.
                    # Codex round-3 (P1): an inconclusive messages
                    # read is NOT evidence the session is empty — the
                    # first prompt may have landed despite its lost
                    # response, and resending could execute the task
                    # twice. Resend only after a successful read
                    # positively confirms an empty session; when
                    # reconciliation is inconclusive, leave the
                    # binding intact for supervision (delivered stays
                    # False and the session is retained below).
                    messages = _read_session_messages(session_id)
                    landed = isinstance(messages, list) and len(messages) > 0
                    verified_empty = isinstance(messages, list) and len(messages) == 0
                    delivered = landed or (verified_empty and _deliver_prompt())
                if not delivered:
                    # The binding stays as the record and the session
                    # is NOT deleted: supervision and the
                    # expired-lease recovery rule govern what happens
                    # next. Never make the task launchable again
                    # while this session may be alive.
                    skips.append({
                        "task_id": task_id,
                        "session_id": session_id,
                        "reason": "prompt_delivery_failed_bound"
                    })
                    # The retained session counts as an active root
                    # (poll.py::is_active_root) and may already be
                    # executing — reserve its cap slot here, not only
                    # on the success path below, or repeated delivery
                    # failures create roots beyond the cap.
                    # (Codex round-3, PR #6990.)
                    cap_room -= 1
                    continue

                launched.append({
                    "task_id": task_id,
                    "session_id": session_id,
                    "worker_id": worker_id,
                    "timestamp": datetime.utcnow().isoformat() + "Z"
                })
                
                cap_room -= 1
                
            except Exception as e:
                skips.append({
                    "task_id": task_id,
                    "reason": f"launch_error: {str(e)}"
                })
                
    except Exception as e:
        print(f"Launch step failed: {e}")
    
    return launched, skips


def main():
    """Main entry point for the claimant/executor."""
    mode = read_mode()
    
    print(f"OpenCode claimant starting in {mode} mode")
    
    # Step 1: Supervision (always runs in live mode)
    if mode == "live":
        print("Running supervision step...")
        supervision_step(mode)
    
    # Step 2: Gate
    print("Running gate step...")
    has_candidates = gate_step()
    
    if not has_candidates:
        print("No unbound opencode-lane candidates found. Exiting.")
        log_fire(mode, [], [], 0, [{"reason": "no_candidates"}])
        return 0
    
    # Step 3: Cap
    print("Running cap step...")
    cap_room = cap_step()
    
    if cap_room == -1:
        print("Warning: Could not determine cap room. Exiting.")
        log_fire(mode, [], [], -1, [{"reason": "cap_unknown"}])
        return 1
    
    if cap_room <= 0:
        print(f"Cap saturated ({CONCURRENCY_CAP} active sessions). Exiting.")
        log_fire(mode, [], [], cap_room, [{"reason": "cap_saturated"}])
        return 0
    
    print(f"Cap room: {cap_room} sessions available")
    
    # Step 4: Launch
    print("Running launch step...")
    launched, skips = launch_step(mode, cap_room)
    # The fire log must carry the candidates discovery actually saw
    # (log_fire's candidates_seen), not a hard-coded empty list -
    # otherwise a failed fire cannot be reconstructed. run_poll is a
    # read; on any failure fall back to an empty list rather than
    # lose the fire entry.
    try:
        fire_candidates = run_poll().get("candidates", [])
    except Exception:
        fire_candidates = []
    
    # Step 5: Terminal states (handled by workers)
    # Step 6: Log fire
    print(f"Fire complete. Launched {len(launched)} sessions.")
    
    log_fire(mode, fire_candidates, launched, cap_room, skips)

    return 0


if __name__ == "__main__":
    sys.exit(main())