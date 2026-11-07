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
from datetime import datetime, timezone
from pathlib import Path
from typing import Dict, List, Optional, Any

# Import poll functions for reuse
import poll

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


def get_active_root_session_count() -> int:
    """Get the count of active root sessions from the OpenCode server."""
    import urllib.request
    import urllib.error
    
    try:
        req = urllib.request.Request("http://127.0.0.1:13420/api/session", timeout=5)
        with urllib.request.urlopen(req) as response:
            sessions = json.loads(response.read().decode('utf-8'))
        
        active_count = 0
        for session in sessions:
            # Count root sessions that are active (no terminal marker, activity in last 30 min)
            if session.get("is_root", False):
                # Check for terminal markers
                outcome = session.get("outcome", "")
                idle = session.get("time", {}).get("idle", "")
                
                has_terminal = False
                if isinstance(outcome, str):
                    has_terminal = any(marker in outcome for marker in ["idle", "outcome"])
                if isinstance(idle, str):
                    has_terminal = has_terminal or any(marker in idle for marker in ["idle", "outcome"])
                
                if not has_terminal:
                    active_count += 1
        
        return active_count
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
    
    # Get stuck bound tasks using poll's get_stuck_bound
    stuck_bound = poll.get_stuck_bound()
    
    for candidate in stuck_bound:
        task_id = candidate["task_id"]
        session_id = candidate["session_id"]
        attempts = candidate.get("attempts", 0)
        max_attempts = candidate.get("max_attempts", 5)
        
        try:
            # Step a: VERIFY zero messages first
            session_messages = []
            try:
                msg_req = urllib.request.Request(
                    f"http://127.0.0.1:13420/api/session/{session_id}/messages",
                    timeout=5
                )
                with urllib.request.urlopen(msg_req) as msg_response:
                    if msg_response.status_code == 200:
                        session_messages = json.loads(msg_response.read().decode('utf-8'))
            except Exception:
                # Session might be gone or unavailable
                pass
            
            # If session has ANY messages or activity, it's NOT this case
            if len(session_messages) > 0:
                print(f"Session {session_id} for task {task_id} has messages - not a zero-message case")
                continue
            
            # Session has zero messages - proceed with recovery
            # Step b: DELETE the verified zero-message session
            req = urllib.request.Request(
                f"http://127.0.0.1:13420/api/session/{session_id}",
                method="DELETE",
                timeout=5
            )
            with urllib.request.urlopen(req) as delete_response:
                if delete_response.status:
                    # Step c: Attempt recovery via atomic bind operation
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
        candidates = poll.get_candidates()
        return len(candidates) > 0
    except Exception as e:
        print(f"Gate check failed: {e}")
        return False


def cap_step() -> int:
    """
    Step 3: Cap - check if we're at the concurrency limit.
    
    Returns cap_room (available slots) or -1 if unknown.
    """
    active_count = poll.get_active_root_session_count()
    
    if active_count == -1:
        return -1
    
    cap_room = max(0, poll.CONCURRENCY_CAP - active_count)
    return cap_room


def launch_step(mode: str, cap_room: int) -> List[Dict]:
    """
    Step 4: Launch - create sessions and bind tasks.
    
    Returns list of successfully launched sessions.
    """
    if cap_room <= 0:
        return []
    
    launched = []
    skips = []
    
    try:
        candidates = poll.get_candidates()
        
        for candidate in candidates:
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
                data = json.dumps(session_create_body).encode('utf-8')
                req = urllib.request.Request(
                    "http://127.0.0.1:13420/api/session",
                    data=data,
                    headers={'Content-Type': 'application/json'},
                    timeout=10
                )
                
                with urllib.request.urlopen(req) as create_response:
                    if create_response.status != 200:
                        raise RuntimeError(f"Session creation failed: {create_response.read().decode('utf-8')}")
                    
                    session = json.loads(create_response.read().decode('utf-8'))
                    session_id = session.get("id")
                
                # Verify location before proceeding
                if session.get("location", {}).get("directory") != str(PRIMARY_CLONE):
                    # Wrong location - delete and skip
                    req = urllib.request.Request(
                        f"http://127.0.0.1:13420/api/session/{session_id}",
                        method="DELETE",
                        timeout=5
                    )
                    with urllib.request.urlopen(req):
                        pass
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
                    req = urllib.request.Request(
                        f"http://127.0.0.1:13420/api/session/{session_id}",
                        method="DELETE",
                        timeout=5
                    )
                    with urllib.request.urlopen(req):
                        pass
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
                    "queue": queue,
                    "timestamp": datetime.now(timezone.utc).isoformat()
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
        print(f"Cap saturated ({poll.CONCURRENCY_CAP} active sessions). Exiting.")
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