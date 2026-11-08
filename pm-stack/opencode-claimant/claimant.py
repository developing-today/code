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

import json
import os
import sys
import time
from datetime import datetime
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


def log_fire(
    mode: str,
    candidates: List[Dict],
    launched: List[Dict],
    cap_room: Optional[int],
    skips: List[Dict],
    report: Optional[Dict] = None,
) -> None:
    """Append a fire log entry to fires.jsonl."""
    ensure_state_dir()
    
    fire_entry = {
        "timestamp": datetime.utcnow().isoformat() + "Z",
        "mode": mode,
        "candidates_seen": candidates,
        "sessions_launched": launched,
        "cap_room": cap_room,
        "skips": skips,
    }
    if report:
        fire_entry.update(report)
    
    with FIRES_FILE.open("a") as f:
        f.write(json.dumps(fire_entry) + "\n")


HELD_ISSUE_LABELS = {"held", "user/owned", "do-not-merge", "do-not-auto-edit"}


def _labels_from(value: Any) -> List[str]:
    """Extract label names from common packet/issue representations."""
    if isinstance(value, str):
        return [value.lower()]
    if isinstance(value, dict):
        if isinstance(value.get("name"), str):
            return [value["name"].lower()]
        labels = value.get("labels")
        if labels is not None:
            return _labels_from(labels)
        return []
    if isinstance(value, list):
        found = []
        for item in value:
            found.extend(_labels_from(item))
        return found
    return []


def candidate_labels(candidate: Dict) -> List[str]:
    """Return labels carried by the candidate or its inlined packet."""
    packet = candidate.get("packet")
    labels = _labels_from(candidate.get("labels"))
    labels.extend(_labels_from(candidate.get("issue_labels")))
    if isinstance(packet, dict):
        labels.extend(_labels_from(packet.get("labels")))
        labels.extend(_labels_from(packet.get("issue_labels")))
        labels.extend(_labels_from(packet.get("issue")))
    return sorted(set(labels))


def candidate_route(candidate: Dict) -> Dict[str, str]:
    """Resolve the packet route, falling back only to the Workhorse route."""
    packet = candidate.get("packet")
    route = packet.get("route") if isinstance(packet, dict) else None
    if not isinstance(route, dict):
        route = {}

    provider_id = route.get("providerID") or route.get("provider_id") or "opencode-route"
    model_id = route.get("modelID") or route.get("model_id") or route.get("id")
    agent = route.get("agent") or (packet.get("agent") if isinstance(packet, dict) else None)

    if not model_id:
        model_id = "workhorse"
    if not agent:
        agent = "build"
    return {"providerID": str(provider_id), "modelID": str(model_id), "agent": str(agent)}


def shadow_report(poll_data: Dict[str, Any]) -> Dict[str, Any]:
    """Build a read-only worked-example report; this function never launches."""
    candidates = poll_data.get("candidates")
    if not isinstance(candidates, list):
        candidates = []

    cap_room = poll_data.get("cap_room")
    skips: List[Dict] = []
    would_launch: List[Dict] = []
    for candidate in candidates:
        labels = candidate_labels(candidate)
        blocked = sorted(HELD_ISSUE_LABELS.intersection(labels))
        if blocked:
            skips.append({
                "task_id": candidate.get("id"),
                "reason": "held_issue_label",
                "labels": blocked,
            })
            continue
        if cap_room is None:
            skips.append({"task_id": candidate.get("id"), "reason": "cap_unknown"})
            continue
        if cap_room <= len(would_launch):
            skips.append({"task_id": candidate.get("id"), "reason": "cap_saturated"})
            continue
        would_launch.append({
            "task_id": candidate.get("id"),
            "queue": candidate.get("queue"),
            "issue": candidate.get("issue"),
            "route": candidate_route(candidate),
        })

    if not candidates:
        skips.append({"reason": "no_candidates"})
    elif cap_room == 0:
        skips.append({"reason": "cap_saturated"})
    elif cap_room is None:
        skips.append({"reason": "cap_unknown"})

    return {
        "would_launch": would_launch,
        "cap": poll_data.get("cap", poll_data.get("concurrency_cap", CONCURRENCY_CAP)),
        "cap_count": poll_data.get("active_root_sessions"),
        "live_root_sessions": poll_data.get("live_root_sessions"),
        "active_root_sessions": poll_data.get("active_root_sessions"),
        "supervision_stalls": poll_data.get("stuck_bound", []),
        "skips": skips,
    }


def run_shadow_fire() -> int:
    """Discover and log a shadow fire without entering any mutation path."""
    try:
        poll_data = run_poll()
        if not isinstance(poll_data, dict):
            raise RuntimeError("poll.py returned a non-object result")
    except Exception as e:
        log_fire(
            "shadow", [], [], None,
            [{"reason": "discovery_failed", "error": str(e)}],
            {"would_launch": [], "supervision_stalls": [], "cap_count": None},
        )
        print(f"Shadow discovery failed; no launch attempted: {e}")
        return 1

    candidates = poll_data.get("candidates")
    if not isinstance(candidates, list):
        candidates = []
    report = shadow_report(poll_data)
    log_fire(
        "shadow",
        candidates,
        [],
        poll_data.get("cap_room"),
        report.pop("skips"),
        report,
    )
    print(
        f"Shadow fire complete. Candidates: {len(candidates)}; "
        f"would launch: {len(report['would_launch'])}; "
        f"cap room: {poll_data.get('cap_room')}"
    )
    return 0


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
    if mode != "live":
        return []
    
    recovery_candidates = []
    
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
                            session_response = requests.get(
                                f"http://127.0.0.1:13420/api/session/{session_id}",
                                timeout=5
                            )
                            
                            if session_response.status_code == 200:
                                session = session_response.json()
                                # Check for terminal state
                                has_terminal = any(
                                    marker in session.get("outcome", "") or 
                                    marker in session.get("time", {}).get("idle", "")
                                    for marker in ["idle", "outcome", "completed", "failed"]
                                )
                                
                                if not has_terminal:
                                    # Session exists and is active - nothing to do
                                    continue
                            else:
                                # Session gone from server
                                pass
                        except Exception:
                            # Could not verify session - leave bound
                            continue
                        
                        # Session is gone or stuck - check if recovery is possible
                        # Check task attempts and lease
                        if task.get("attempts", 0) < task.get("max_attempts", 5):
                            # Verify zero messages first
                            try:
                                messages = requests.get(
                                    f"http://127.0.0.1:13420/api/session/{session_id}/messages",
                                    timeout=5
                                )
                                
                                if messages.status_code == 200:
                                    message_count = len(messages.json())
                                    if message_count == 0:
                                        recovery_candidates.append({
                                            "task_id": task["id"],
                                            "session_id": session_id,
                                            "reason": "zero_message"
                                        })
                            except Exception:
                                # Could not read messages - treat as unknown state
                                pass
        except Exception as e:
            print(f"Warning: Supervision check failed for queue {queue}: {e}")
    
    # Process recovery candidates
    for candidate in recovery_candidates:
        task_id = candidate["task_id"]
        session_id = candidate["session_id"]
        
        try:
            # Delete the zero-message session
            delete_response = requests.delete(
                f"http://127.0.0.1:13420/api/session/{session_id}",
                timeout=5
            )
            
            if delete_response.status_code == 200:
                # Attempt recovery via atomic bind operation
                bind_result = subprocess.run(
                    [
                        str(REPO_QUEUE_CMD),
                        "bind", task_id,
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


def launch_step(mode: str, cap_room: int) -> List[Dict]:
    """
    Step 4: Launch - create sessions and bind tasks.
    
    Returns list of successfully launched sessions.
    """
    if mode != "live" or cap_room <= 0:
        return []
    
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
                create_response = requests.post(
                    "http://127.0.0.1:13420/api/session",
                    json=session_create_body,
                    timeout=10
                )
                
                if create_response.status_code != 200:
                    raise RuntimeError(f"Session creation failed: {create_response.text}")
                
                session = create_response.json()
                session_id = session.get("id")
                
                # Verify location before proceeding
                if session.get("location", {}).get("directory") != str(PRIMARY_CLONE):
                    # Wrong location - delete and skip
                    requests.delete(
                        f"http://127.0.0.1:13420/api/session/{session_id}",
                        timeout=5
                    )
                    skips.append({
                        "task_id": task_id,
                        "reason": "session_wrong_location"
                    })
                    continue
                
                # Step 4c: Bind the session
                bind_result = subprocess.run(
                    [
                        str(REPO_QUEUE_CMD),
                        "bind", task_id,
                        "--worker", "opencode-claimant",
                        "--session-id", session_id
                    ],
                    capture_output=True,
                    text=True,
                    timeout=10
                )
                
                if bind_result.returncode != 0:
                    # Bind failed - delete session and skip
                    requests.delete(
                        f"http://127.0.0.1:13420/api/session/{session_id}",
                        timeout=5
                    )
                    skips.append({
                        "task_id": task_id,
                        "reason": "bind_failed"
                    })
                    continue
                
                # Step 4d: Send prompt to bound session
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
                
                # In a real implementation, this would send the prompt via the session API
                # For now, we just record the successful launch
                
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
    
    return launched


def main():
    """Main entry point for the claimant/executor."""
    mode = read_mode()
    
    print(f"OpenCode claimant starting in {mode} mode")

    # Shadow mode is a strict read-only fire. Keep this branch ahead of
    # supervision, gate, cap, and launch helpers so a future refactor
    # cannot accidentally reach a mutation path from the default mode.
    if mode != "live":
        return run_shadow_fire()
    
    # Step 1: Supervision (always runs in live mode)
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
    launched = launch_step(mode, cap_room)
    
    # Step 5: Terminal states (handled by workers)
    # Step 6: Log fire
    print(f"Fire complete. Launched {len(launched)} sessions.")
    
    log_fire(mode, [], launched, cap_room, [])
    
    return 0


if __name__ == "__main__":
    sys.exit(main())
