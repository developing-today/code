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


def log_fire(mode: str, candidates: List[Dict], launched: List[Dict], cap_room: "int | None", skips: List[Dict]) -> None:
    """Append a fire log entry to fires.jsonl.

    cap_room is the measured available-slot count, or None when the
    fire exited before the cap step ran (#7732): null means unknown,
    never a placeholder 0, which would read as a saturated fleet.
    """
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


def supervision_step(mode: str) -> List[Dict]:
    """
    Step 1: Supervision - check bound sessions for recovery candidates.
    
    Returns list of tasks that need recovery (bound but stuck/zero-message sessions).
    """
    if mode != "live":
        return []
    
    recovery_candidates = []
    
    # Get all claimed opencode-lane tasks with bound session IDs.
    # Rows come from poll.py's broker rich read: the repo-queue
    # binary's list verb prints a human table, and json-parsing it
    # silently supervised nothing (#7325).
    poll = _load_poll_module()
    for queue in QUEUES:
        try:
            for task in poll.fetch_claimed_rows(queue):
                # Rich rows carry lane/session in progress; a
                # flattened row carries them at top level.
                prog = task.get("progress") or {}
                lane = prog.get("lane") or task.get("lane")
                session_id = prog.get("session_id") or task.get("session_id")
                if lane == "opencode" and session_id:

                    # Check if session exists and is progressing
                    try:
                        session_response = subprocess.run(
                            ["curl", "-s", "-m", "5", *_auth_header_args(),
                             f"{OPENCODE_URL}/api/session/{session_id}"],
                            capture_output=True,
                            text=True
                        )

                        if session_response.returncode == 0:
                            session = json.loads(session_response.stdout)
                            if isinstance(session, dict) and "data" in session:
                                session = session["data"]
                            if not isinstance(session, dict) or not session.get("id"):
                                # Error body, not a session object:
                                # the session is gone from the server.
                                session = None
                            else:
                                # Check for terminal state
                                has_terminal = any(
                                    marker in str(session.get("outcome", "") or "") or
                                    marker in str((session.get("time") or {}).get("idle", "") or "")
                                    for marker in ["idle", "outcome", "completed", "failed"]
                                )

                                if not has_terminal:
                                    # Session exists and is active - nothing to do
                                    continue
                        # else: Session gone from server or error
                    except Exception:
                        # Could not verify session - leave bound
                        continue

                    # Session is gone or stuck - check if recovery is possible
                    # Check task attempts and lease
                    if task.get("attempts", 0) < task.get("max_attempts", 5):
                        # Only an expired lease is recoverable: a live
                        # lease is still owned by a worker that may be
                        # executing, and recovery would double-launch
                        # over it. An unreadable lease is not expired
                        # (poll.py's rule: never act on a fact we lack).
                        lease_ms = poll.parse_lease_ms(task.get("lease_until"))
                        if lease_ms is None or lease_ms > int(time.time() * 1000):
                            continue

                        # Verify zero messages first. An unreadable
                        # session is UNKNOWN, never empty: only a
                        # confirmed [] may be deleted and rebound, or a
                        # session that actually had messages gets
                        # deleted and its task relaunched (#3026).
                        messages = _get_messages(session_id)
                        if messages is None:
                            print(
                                f"Supervision: message read for session "
                                f"{session_id} was unknown; leaving task "
                                f"{task['id']} bound"
                            )
                            continue
                        if len(messages) == 0:
                            recovery_candidates.append({
                                "task_id": task["id"],
                                "session_id": session_id,
                                "reason": "zero_message"
                            })
        except Exception as e:
            print(f"Warning: Supervision check failed for queue {queue}: {e}")
    
    # Process recovery candidates
    for candidate in recovery_candidates:
        task_id = candidate["task_id"]
        session_id = candidate["session_id"]
        
        try:
            # Delete the zero-message session
            delete_result = subprocess.run(
                ["curl", "-s", "-m", "5", "-X", "DELETE", *_auth_header_args(),
                 f"{OPENCODE_URL}/api/session/{session_id}"],
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
    (transport failure, non-2xx status, non-JSON body, error object,
    or any body without a list of messages): an unreadable session
    is an UNKNOWN delivery state, never an empty one.

    The v2 endpoint is /message, not /messages: on the standing
    server the latter 404s with an empty body, which without -f used
    to parse as [] and turn every unknown read into "zero messages"
    (#3026). -f makes that non-2xx a failure and strict parsing keeps
    an empty or error body from masquerading as a confirmed list."""
    try:
        result = subprocess.run(
            ["curl", "-s", "-f", "-m", "5", *_auth_header_args(),
             f"{OPENCODE_URL}/api/session/{session_id}/message"],
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            return None
        data = json.loads(result.stdout)
        if isinstance(data, dict):
            data = data.get("data")
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


CLAIMANT_WORKER = "opencode-claimant"


def _collect_parked_handoffs(queue: str, want: int) -> Dict[int, Dict]:
    """Collect up to `want` parked recorded handoffs from `queue`
    WITHOUT naming a task id (#8097 Rule 1): each
    `repo-queue claim --handoff` lets the BROKER pick the next
    parked handoff (its ordinary pick order), claimed as the
    claimant, so the launch flow below has a live claim to bind
    against. Returns {task_id: claimed row}.

    The handing-off lane parks the task as its handoff sink, so
    without collection the task is never in the claimed state the
    launch flow binds against — it waited out the park timer, was
    re-claimed by the next shift wave, handed off, and parked again
    (#7907/#6533). Collection used to name the task
    (`claim --task <id>`); that by-id claim is now the special-case
    verb (one-off revivals), and this default path does not depend
    on it — the broker's handoff pick only ever returns a parked
    recorded opencode handoff (unbound yield progress), never a
    live claim or a genuine wait park.

    The claim declares the claimant's family, --platform
    openchamber (#8097 Rule 2): a parked handoff whose item
    carries the `muse` selector is not the opencode claimant's
    to collect.
    """
    collected: Dict[int, Dict] = {}
    for _ in range(max(want, 0)):
        try:
            result = subprocess.run(
                [
                    str(REPO_QUEUE_CMD),
                    "claim",
                    "--queue", queue,
                    "--worker", CLAIMANT_WORKER,
                    "--platform", "openchamber",
                    "--handoff",
                ],
                capture_output=True,
                text=True,
                timeout=15,
            )
            data = json.loads(result.stdout) if result.stdout.strip() else None
        except Exception:
            break
        # cmd_claim prints the claimed row itself ({"task": null}
        # when nothing was claimed).
        row = data if isinstance(data, dict) else None
        claimed_id = row.get("id") if isinstance(row, dict) else None
        if claimed_id is None:
            break
        collected[claimed_id] = row
    return collected


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
        
        # Parked handoff sink (#8097 Rule 1): collect this fire's
        # parked candidates' claims up front, grouped by queue,
        # without naming any task id — one broker-picked handoff
        # claim per discovered parked candidate. A parked
        # candidate whose id is not in `collected` when its turn
        # comes was not collected (its pick lost a race or the
        # broker had nothing claimable) and is skipped below —
        # the binding is never improvised.
        parked_by_queue: Dict[str, int] = {}
        for c in unbound_opencode:
            if c.get("status") == "parked":
                parked_by_queue[c["queue"]] = parked_by_queue.get(c["queue"], 0) + 1
        collected: Dict[int, Dict] = {}
        for queue_name, want in parked_by_queue.items():
            collected.update(_collect_parked_handoffs(queue_name, want))

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

            # Parked handoff sink: the candidate was handed off and
            # then parked by its claiming lane, so no live claim
            # existed to bind against until the collection above
            # claimed it (as the claimant; the launched session
            # then works under the claimant's worker identity). A
            # candidate the collection did not claim is skipped —
            # the binding is never improvised.
            if candidate.get("status") == "parked":
                if task_id not in collected:
                    skips.append({
                        "task_id": task_id,
                        "reason": "parked_claim_failed"
                    })
                    continue
                candidate["status"] = "claimed"
                candidate["worker_id"] = CLAIMANT_WORKER
                worker_id = CLAIMANT_WORKER

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
        log_fire(mode, [], [], None, [{"reason": "no_candidates"}])
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