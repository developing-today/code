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

QUEUES = [
    "repo-health-code-pilot",
    "repo-health-opencode-pilot",
    "repo-health-openchamber-pilot",
    "repo-health-nix-pilot",
]

# Packaged fleet binary via its repo-queue argv0 alias
# (~/.local/bin/repo-queue -> ~/.local/bin/fleet); the hand-built
# ~/fleet-brokers/bin tree was retired by the code#5362 cutover
# (task 12731).
REPO_QUEUE_CMD = Path.home() / ".local" / "bin" / "repo-queue"
PRIMARY_CLONE = Path.home() / "git" / "code"

# OpenCode server coordinates: the standing server requires HTTP
# Basic auth (user "opencode", password in its state file — the same
# credential poll.py and the pm-dispatch client read). Calls made
# without it are rejected, so every session call below carries it.
OPENCODE_URL = os.environ.get("OPENCODE_URL", "http://127.0.0.1:13420")
AUTH_FILE = os.environ.get(
    "OPENCODE_AUTH_FILE",
    os.path.expanduser("~/.local/state/opencode/service-prod.json"),
)


def _auth_header_args() -> List[str]:
    """curl args carrying the server's Basic auth, or [] when the
    credential cannot be read (the server then rejects the call and
    the caller fails closed). The value is never logged."""
    try:
        with open(AUTH_FILE) as f:
            auth = json.load(f)
        password = auth.get("password") if isinstance(auth, dict) else None
        if not password:
            return []
        token = base64.b64encode(f"opencode:{password}".encode()).decode()
        return ["-H", f"Authorization: Basic {token}"]
    except Exception:
        return []

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


def _http_get(url: str, timeout: int = 10) -> Optional[Dict]:
    """Helper to make HTTP GET requests using urllib."""
    try:
        req = urllib.request.Request(url)
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


# OpenCode server reads used by supervision. The v2 server answers
# messages at /api/session/<id>/message — SINGULAR. /messages is a 404
# with an EMPTY body, so a status-blind reader reads every session as
# "zero messages" and deletes work that actually ran (#8038). Every
# read below distinguishes "zero" from "could not read".
MESSAGES_PATH = "/api/session/{id}/message"


def _server_call(path: str, method: str = "GET", timeout: int = 10):
    """(status, decoded_body) for one authenticated server call.

    status is None when the call never completed (transport failure,
    no status line); decoded_body is None for an empty or non-JSON
    body. The credential never leaves this function."""
    try:
        result = subprocess.run(
            ["curl", "-s", "-m", str(timeout), "-X", method,
             *_auth_header_args(), "-w", "\n%{http_code}",
             f"{OPENCODE_URL}{path}"],
            capture_output=True,
            text=True,
        )
    except Exception:
        return None, None
    if result.returncode != 0:
        return None, None
    body, _, code = result.stdout.rpartition("\n")
    try:
        status = int(code.strip())
    except ValueError:
        return None, None
    payload = None
    if body.strip():
        try:
            payload = json.loads(body)
        except ValueError:
            payload = None
        if isinstance(payload, dict) and "data" in payload:
            payload = payload["data"]
    return status, payload


def _session_messages(session_id: str) -> Optional[List]:
    """The session's messages, or None when the read is inconclusive.

    None means UNKNOWN, never zero: a non-200 status, a transport
    failure or a non-list body leaves the binding alone (step 1a)."""
    status, payload = _server_call(MESSAGES_PATH.format(id=session_id))
    if status != 200 or not isinstance(payload, list):
        return None
    return payload


def _delete_session_verified(session_id: str) -> bool:
    """Delete the session and prove it is gone (step 1b). False on any
    doubt: an un-deleted session with a cleared binding is the
    duplicate-execution defect this ordering prevents."""
    status, _ = _server_call(f"/api/session/{session_id}",
                             method="DELETE", timeout=5)
    if status not in (200, 202, 204, 404):
        return False
    status, _ = _server_call(f"/api/session/{session_id}", timeout=5)
    return status == 404


def _lease_expired(row: Dict, now_ms: int) -> bool:
    """Only an expired lease makes a bound task the claimant's
    business; an unreadable lease is never 'expired'."""
    poll = _load_poll_module()
    lease_ms = poll.parse_lease_ms(row.get("lease_until"))
    return lease_ms is not None and lease_ms <= now_ms


def _queue_verb(argv: List[str]):
    """Run one queue CLI verb, or None when it cannot be run."""
    try:
        return subprocess.run(argv, capture_output=True, text=True, timeout=15)
    except Exception as e:
        print(f"Supervision: queue verb failed ({argv[1]}): {e}")
        return None


def supervision_step(mode: str) -> List[Dict]:
    """Step 1: supervise bound opencode-lane sessions.

    Candidates come from poll.py's classifiers, not from a local
    reimplementation: `stuck_bound` (lease expired, session present as
    a root with NO terminal marker, not in the server's active set,
    silent for ACTIVE_WINDOW_MS — #4758) plus the gone-session case
    (bound id absent from the COMPLETE session list, lease expired).

    Each candidate passes three guards before anything is mutated: the
    message read must succeed and be empty, the delete must be
    verified, and only then is the binding cleared atomically. Any
    unknown (session list, active set, message read) leaves every
    binding exactly as it is. Shadow mode reports only.
    """
    poll = _load_poll_module()
    now_ms = int(time.time() * 1000)
    sessions = poll.fetch_sessions()
    active_ids = poll.fetch_active_ids() if sessions is not None else None
    if sessions is None or active_ids is None:
        print("Supervision: session list or active set unreadable; "
              "leaving every binding as it is.")
        return []
    session_by_id = {
        s["id"]: s for s in sessions if isinstance(s, dict) and s.get("id")
    }

    stuck, gone = [], []
    for queue in QUEUES:
        try:
            rows = poll.fetch_claimed_rows(queue)
        except Exception as e:
            print(f"Supervision: could not read queue {queue}: {e}")
            continue
        stuck.extend(poll.get_stuck_bound(rows, session_by_id, active_ids, now_ms))
        for row in rows:
            prog = row.get("progress") or {}
            session_id = prog.get("session_id")
            if prog.get("lane") != "opencode" or not session_id:
                continue
            if session_id in session_by_id:
                continue  # present on the server: the stuck rule owns it
            if _lease_expired(row, now_ms):
                gone.append(row)

    if mode != "live":
        for row in stuck:
            print(f"Supervision (shadow): task {row.get('id')} bound session "
                  f"{(row.get('progress') or {}).get('session_id')} looks stuck")
        for row in gone:
            print(f"Supervision (shadow): task {row.get('id')} bound session "
                  f"{(row.get('progress') or {}).get('session_id')} is gone")
        return []

    recovered = []
    todo = [(row, "zero_message") for row in stuck] + \
           [(row, "gone_session") for row in gone]
    for row, reason in todo:
        task_id = row.get("id")
        prog = row.get("progress") or {}
        session_id = prog.get("session_id")
        worker_id = row.get("worker_id") or "repo-worker-code"
        attempts = row.get("attempts") or 0
        max_attempts = row.get("max_attempts") or 5

        if reason == "zero_message":
            messages = _session_messages(session_id)
            if messages is None:
                print(f"Supervision: messages unreadable for session "
                      f"{session_id} (task {task_id}); state unknown, "
                      f"leaving it bound.")
                continue
            if messages:
                print(f"Supervision: task {task_id} session {session_id} "
                      f"holds {len(messages)} messages; leaving it bound.")
                continue
            if not _delete_session_verified(session_id):
                print(f"Supervision: could not verify deletion of session "
                      f"{session_id} for task {task_id}; keeping the binding.")
                continue
        # gone_session: absence from the complete session list is the
        # verification; there is nothing left to delete.

        if attempts >= max_attempts:
            execution = json.dumps({
                "lane": "opencode",
                "session_id": session_id,
                "fallback_reason": "launch_failed_attempts_exhausted",
            })
            result = _queue_verb([
                str(REPO_QUEUE_CMD), "fail", str(task_id),
                "--worker", worker_id,
                "--error", f"bound session {session_id} unavailable without "
                           f"terminal outcome; attempts exhausted",
                "--reason-class", "deterministic",
                "--execution", execution,
            ])
            if result is not None and result.returncode == 0:
                print(f"Supervision: task {task_id} failed after {attempts} "
                      f"attempts (bound session {session_id} removed)")
                recovered.append({"task_id": task_id,
                                  "session_id": session_id,
                                  "reason": reason})
            elif result is not None:
                print(f"Supervision: fail verb refused for task {task_id}: "
                      f"{(result.stderr or result.stdout).strip()[:200]}")
            continue

        launched_by = (LAUNCHED_BY_RECOVERY_ZERO_MESSAGE
                       if reason == "zero_message"
                       else LAUNCHED_BY_RECOVERY_GONE_SESSION)
        result = _queue_verb([
            str(REPO_QUEUE_CMD), "bind", str(task_id),
            "--worker", "opencode-claimant",
            "--session-id", session_id,
            "--launched-by", launched_by,
        ])
        if result is None:
            continue
        if result.returncode == 0:
            print(f"Recovered task {task_id}: cleared {reason} "
                  f"session {session_id}")
            recovered.append({"task_id": task_id,
                              "session_id": session_id,
                              "reason": reason})
        else:
            print(f"Supervision: recovery bind refused for task {task_id}: "
                  f"{(result.stderr or result.stdout).strip()[:200]}")

    return recovered


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


def _post_prompt(session_id: str, prompt: str) -> bool:
    """One prompt POST; True iff the server accepted it."""
    try:
        result = subprocess.run(
            ["curl", "-s", "-f", "-m", "15", "-X", "POST",
             "-H", "Content-Type: application/json",
             *_auth_header_args(),
             "-d", json.dumps({"text": prompt}),
             f"{OPENCODE_URL}/api/session/{session_id}/prompt"],
            capture_output=True,
            text=True,
        )
        return result.returncode == 0
    except Exception:
        return False


def _get_messages(session_id: str) -> Optional[List]:
    """The session's message list, or None when it cannot be read
    (transport failure, non-JSON body, error object): an unreadable
    session is an UNKNOWN delivery state, never an empty one."""
    try:
        result = subprocess.run(
            ["curl", "-s", "-m", "5", *_auth_header_args(),
             f"{OPENCODE_URL}/api/session/{session_id}/messages"],
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            return None
        data = json.loads(result.stdout or "[]")
        if isinstance(data, dict) and "data" in data:
            data = data["data"]
        return data if isinstance(data, list) else None
    except Exception:
        return None


def _send_prompt(session_id: str, prompt: str) -> bool:
    """Deliver the execution prompt to a bound session. Returns True
    iff the prompt is confirmed delivered. A failed POST is
    reconciled before any resend (prompt.md step 4): if a message
    already landed, the worker may be executing and must never be
    double-prompted; only a successful reconcile that proves the
    message list is EMPTY earns one resend to the same session. An
    inconclusive reconcile (unreadable session) never resends — the
    binding stands and the caller flags the launch instead."""
    if _post_prompt(session_id, prompt):
        return True
    messages = _get_messages(session_id)
    if messages is None:
        return False
    if len(messages) > 0:
        return True
    return _post_prompt(session_id, prompt)


def launch_step(mode: str, cap_room: int) -> List[Dict]:
    """
    Step 4: Launch - create sessions and bind tasks.

    Returns (launched, skips): the successfully launched sessions and
    a skip record for every candidate that was not launched, so the
    fire log can state why a zero-launch fire launched nothing
    (#6865).
    """
    if mode != "live":
        # Shadow fires observe only (#7366): report the candidates
        # that WOULD launch, bounded by cap room, and mutate nothing —
        # no session created, nothing bound, nothing prompted.
        try:
            poll_data = run_poll()
            candidates = [
                c for c in poll_data.get("candidates", [])
                if c.get("lane") == "opencode"
                and not c.get("session_id")
                and c.get("stage") in ("handed_off", "launch_failed")
            ]
        except Exception as e:
            print(f"Shadow launch scan failed: {e}")
            return [], []
        skips = []
        room = cap_room
        for candidate in candidates:
            if room > 0:
                skips.append({"task_id": candidate["id"],
                              "reason": "shadow_would_launch"})
                room -= 1
            else:
                skips.append({"task_id": candidate["id"],
                              "reason": "shadow_beyond_cap"})
        return [], skips

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
                    "id": "workhorse",
                    "providerID": "opencode-route"
                },
                "agent": "build"
            }
            
            try:
                # Create session using curl (POST), with the server's
                # Basic auth — unauthenticated calls are rejected.
                create_cmd = ["curl", "-s", "-m", "10", "-X", "POST",
                             "-H", "Content-Type: application/json",
                             *_auth_header_args(),
                             "-d", json.dumps(session_create_body),
                             f"{OPENCODE_URL}/api/session"]

                create_result = subprocess.run(
                    create_cmd,
                    capture_output=True,
                    text=True
                )

                if create_result.returncode != 0:
                    raise RuntimeError(f"Session creation failed: {create_result.stderr}")

                session = json.loads(create_result.stdout)
                if isinstance(session, dict) and "data" in session:
                    session = session["data"]
                session_id = session.get("id") if isinstance(session, dict) else None
                if not session_id:
                    raise RuntimeError(
                        f"Session creation returned no id: {create_result.stdout[:200]}")

                # Verify location before proceeding
                if session.get("location", {}).get("directory") != str(PRIMARY_CLONE):
                    # Wrong location - delete and skip
                    delete_cmd = ["curl", "-s", "-m", "5", "-X", "DELETE",
                                 *_auth_header_args(),
                                 f"{OPENCODE_URL}/api/session/{session_id}"]
                    subprocess.run(delete_cmd, capture_output=True)
                    skips.append({
                        "task_id": task_id,
                        "reason": "session_wrong_location"
                    })
                    continue
                
                # Step 4c: Bind the session (atomic operation)
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
                
                if bind_result.returncode != 0:
                    # Bind failed - delete session and skip
                    delete_cmd = ["curl", "-s", "-m", "5", "-X", "DELETE",
                                 *_auth_header_args(),
                                 f"{OPENCODE_URL}/api/session/{session_id}"]
                    subprocess.run(delete_cmd, capture_output=True)
                    skips.append({
                        "task_id": task_id,
                        "reason": "bind_failed"
                    })
                    continue

                # Step 4d: Send the execution prompt to the bound
                # session (POST /api/session/{id}/prompt — the shape
                # pm-dispatch's OpenCode client uses). A bound session
                # that never receives its prompt is #4758's
                # zero-message case (#7356), so the send is verified,
                # never assumed: _send_prompt reconciles against the
                # session's messages before any resend. The binding
                # stands even if the prompt cannot be landed —
                # supervision owns a bound-but-unprompted session,
                # and unbinding here could double-launch over a
                # worker that is in fact executing.
                prompt = f"""
Execute queue task {task_id}
Issue: {issue}
Packet: {json.dumps(packet, indent=2)}

You are a build agent working on this opencode-lane task.
Use --worker {worker_id} for all queue CLI operations.
Heartbeat while working to keep the claim lease alive.
Finish with a queue terminal verb (complete, park, block, or fail) carrying --worker {worker_id} and --execution '{{"lane": "opencode", "session_id": "{session_id}"}}'.
"""

                entry = {
                    "task_id": task_id,
                    "session_id": session_id,
                    "worker_id": worker_id,
                    "timestamp": datetime.utcnow().isoformat() + "Z"
                }
                if not _send_prompt(session_id, prompt):
                    entry["prompt_failed"] = True
                launched.append(entry)

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
    
    # Step 5: Terminal states (handled by workers)
    # Step 6: Log fire
    print(f"Fire complete. Launched {len(launched)} sessions.")
    
    log_fire(mode, [], launched, cap_room, skips)
    
    return 0


if __name__ == "__main__":
    sys.exit(main())