#!/usr/bin/env python3
"""
OpenCode lane claimant/executor - Slice 1 implementation (#3026)

This script implements the deterministic claim/bind/launch/record process
for opencode-lane tasks. It runs as an OpenChamber scheduled task on the
laptop and launches worker sessions itself.

MODE: shadow (default) or live
- shadow: Discovery and verification only, no execution
- live: Full execution including claiming, binding, and launching
"""

import argparse
import json
import os
import subprocess
import sys
import time
from datetime import datetime
from pathlib import Path
from typing import Dict, List, Optional, Tuple

# Constants
QUEUES = [
    "repo-health-code-pilot",
    "repo-health-opencode-pilot",
    "repo-health-openchamber-pilot",
    "repo-health-nix-pilot"
]
REPO_QUEUE_CLI = "/home/user/fleet-brokers/bin/repo-queue"
OPENCODE_SERVER = "http://127.0.0.1:13420"
ACTIVE_WINDOW_MS = 30 * 60 * 1000  # 30 minutes
CONCURRENCY_CAP = 36  # Active root sessions across the fleet
STATE_DIR = Path.home() / ".local" / "state" / "opencode-claimant"
FIRE_LOG = STATE_DIR / "fires.jsonl"
MODE_FILE = STATE_DIR / "mode"

# Recovery launched_by values
LAUNCHED_BY_ZERO_MESSAGE = "opencode-claimant-recovery:zero-message-deleted"
LAUNCHED_BY_GONE_SESSION = "opencode-claimant-recovery:gone-session"


def get_mode() -> str:
    """Read the mode from the mode file. Default to 'shadow' if missing/unreadable."""
    try:
        if MODE_FILE.exists():
            mode = MODE_FILE.read_text().strip()
            if mode in ("shadow", "live"):
                return mode
    except Exception:
        pass
    return "shadow"


def run_command(cmd: List[str], check: bool = True) -> subprocess.CompletedProcess:
    """Run a shell command and return the result."""
    try:
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            check=False
        )
        if check and result.returncode != 0:
            raise subprocess.CalledProcessError(
                result.returncode, cmd, result.stdout, result.stderr
            )
        return result
    except subprocess.CalledProcessError as e:
        print(f"Command failed: {' '.join(cmd)}", file=sys.stderr)
        print(f"stdout: {e.stdout}", file=sys.stderr)
        print(f"stderr: {e.stderr}", file=sys.stderr)
        raise


def get_active_root_sessions() -> Tuple[int, List[Dict]]:
    """
    Get the count of active root sessions and their details.
    Returns: (cap_room, active_sessions)
    """
    try:
        # Call poll.py to get the session count and details
        result = run_command([
            sys.executable,
            str(Path(__file__).parent / "poll.py"),
            "--cap-only"
        ])
        
        data = json.loads(result.stdout)
        cap_room = data.get("cap_room", 0)
        active_sessions = data.get("active_sessions", [])
        return cap_room, active_sessions
    except Exception as e:
        print(f"Failed to get active sessions: {e}", file=sys.stderr)
        return None, []


def get_candidates() -> List[Dict]:
    """
    Discover candidate tasks using poll.py.
    Returns list of unbound opencode-lane tasks.
    """
    try:
        result = run_command([
            sys.executable,
            str(Path(__file__).parent / "poll.py"),
            "--candidates"
        ])
        data = json.loads(result.stdout)
        return data.get("candidates", [])
    except Exception as e:
        print(f"Failed to get candidates: {e}", file=sys.stderr)
        return []


def get_stuck_bound() -> List[Dict]:
    """
    Get stuck_bound tasks from poll.py.
    Returns list of tasks that need recovery.
    """
    try:
        result = run_command([
            sys.executable,
            str(Path(__file__).parent / "poll.py"),
            "--stuck-bound"
        ])
        data = json.loads(result.stdout)
        return data.get("stuck_bound", [])
    except Exception as e:
        print(f"Failed to get stuck_bound tasks: {e}", file=sys.stderr)
        return []


def verify_session_exists(session_id: str) -> Optional[Dict]:
    """
    Verify a session exists and is active via OpenCode server API.
    Returns session info if exists, None otherwise.
    """
    try:
        import requests
        url = f"{OPENCODE_SERVER}/api/session/{session_id}"
        response = requests.get(url, timeout=5)
        
        if response.status_code == 200:
            return response.json()
        elif response.status_code == 404:
            return None
        else:
            print(f"Unexpected status {response.status_code} checking session {session_id}", 
                  file=sys.stderr)
            return None
    except Exception as e:
        print(f"Failed to verify session {session_id}: {e}", file=sys.stderr)
        return None


def get_session_messages(session_id: str) -> List[Dict]:
    """
    Get messages for a session via OpenCode server API.
    Returns list of messages, empty if none.
    """
    try:
        import requests
        url = f"{OPENCODE_SERVER}/api/session/{session_id}/messages"
        response = requests.get(url, timeout=5)
        
        if response.status_code == 200:
            return response.json().get("messages", [])
        else:
            print(f"Failed to get messages for session {session_id}: status {response.status_code}",
                  file=sys.stderr)
            return []
    except Exception as e:
        print(f"Failed to get messages for session {session_id}: {e}", file=sys.stderr)
        return []


def delete_session(session_id: str) -> bool:
    """
    Delete a session via OpenCode server API.
    Returns True if successful, False otherwise.
    """
    try:
        import requests
        url = f"{OPENCODE_SERVER}/api/session/{session_id}"
        response = requests.delete(url, timeout=5)
        
        if response.status_code == 200:
            return True
        else:
            print(f"Failed to delete session {session_id}: status {response.status_code}",
                  file=sys.stderr)
            return False
    except Exception as e:
        print(f"Failed to delete session {session_id}: {e}", file=sys.stderr)
        return False


def recover_stuck_bound_task(task_id: str, session_id: str, attempts: int, max_attempts: int) -> bool:
    """
    Recover a stuck_bound task by deleting the session and reclaiming the task.
    Returns True if recovery succeeded, False otherwise.
    """
    print(f"Attempting to recover stuck_bound task {task_id} with session {session_id}")
    
    # Step a: Verify zero messages
    messages = get_session_messages(session_id)
    if messages:
        print(f"Session {session_id} has {len(messages)} messages, not recovering", file=sys.stderr)
        return False
    
    # Step b: Delete the session
    if not delete_session(session_id):
        print(f"Failed to delete session {session_id}, aborting recovery", file=sys.stderr)
        return False
    
    # Determine launched_by value based on attempts
    if attempts < max_attempts:
        launched_by = LAUNCHED_BY_ZERO_MESSAGE
        recovery_type = "zero-message-deleted"
    else:
        launched_by = LAUNCHED_BY_GONE_SESSION
        recovery_type = "gone-session"
    
    # Step c: Reclaim the task atomically
    try:
        cmd = [
            REPO_QUEUE_CLI,
            "bind", task_id,
            "--worker", "opencode-claimant",
            "--session-id", session_id,
            "--launched-by", launched_by
        ]
        result = run_command(cmd)
        
        if result.returncode == 0:
            print(f"Successfully recovered task {task_id} via {recovery_type}")
            return True
        else:
            print(f"Failed to reclaim task {task_id}: {result.stderr}", file=sys.stderr)
            return False
    except Exception as e:
        print(f"Exception during task recovery: {e}", file=sys.stderr)
        return False


def supervision_step() -> List[Dict]:
    """
    Step 1: Supervision - Check bound sessions and recover stuck_bound tasks.
    Returns list of tasks that were recovered.
    """
    recovered_tasks = []
    
    # Get stuck_bound tasks
    stuck_bound = get_stuck_bound()
    
    for task in stuck_bound:
        task_id = task.get("id")
        session_id = task.get("session_id")
        attempts = task.get("attempts", 0)
        max_attempts = task.get("max_attempts", 5)
        
        if not task_id or not session_id:
            print(f"Skipping invalid stuck_bound entry: {task}", file=sys.stderr)
            continue
        
        # Verify session exists
        session_info = verify_session_exists(session_id)
        
        if session_info is None:
            # Session is gone, attempt recovery if attempts remain
            if attempts < max_attempts:
                if recover_stuck_bound_task(task_id, session_id, attempts, max_attempts):
                    recovered_tasks.append({
                        "task_id": task_id,
                        "session_id": session_id,
                        "reason": "gone-session-recovered"
                    })
            else:
                # Attempts exhausted, terminally fail the task
                fail_cmd = [
                    REPO_QUEUE_CLI,
                    "fail", task_id,
                    "--worker", task.get("worker_id", "unknown"),
                    "--error", f"bound session {session_id} unavailable without terminal outcome; attempts exhausted",
                    "--reason-class", "deterministic",
                    "--execution", json.dumps({
                        "lane": "opencode",
                        "session_id": session_id,
                        "fallback_reason": "launch_failed_attempts_exhausted"
                    })
                ]
                try:
                    run_command(fail_cmd)
                    print(f"Terminally failed task {task_id} due to exhausted attempts")
                except Exception as e:
                    print(f"Failed to terminally fail task {task_id}: {e}", file=sys.stderr)
        else:
            # Session exists but has no messages - check if it's active
            messages = get_session_messages(session_id)
            
            # Check if session is in active set (no terminal marker, recent activity)
            is_active = session_info.get("time", {}).get("idle") is None and \
                       session_info.get("outcome") is None
            
            if not is_active and not messages:
                # Stuck session with no activity and no messages - recover if attempts remain
                if attempts < max_attempts:
                    if recover_stuck_bound_task(task_id, session_id, attempts, max_attempts):
                        recovered_tasks.append({
                            "task_id": task_id,
                            "session_id": session_id,
                            "reason": "zero-message-stuck-recovered"
                        })
                else:
                    # Attempts exhausted
                    if delete_session(session_id):
                        fail_cmd = [
                            REPO_QUEUE_CLI,
                            "fail", task_id,
                            "--worker", task.get("worker_id", "unknown"),
                            "--error", f"bound session {session_id} stuck without progress; attempts exhausted",
                            "--reason-class", "deterministic",
                            "--execution", json.dumps({
                                "lane": "opencode",
                                "session_id": session_id,
                                "fallback_reason": "launch_failed_attempts_exhausted"
                            })
                        ]
                        try:
                            run_command(fail_cmd)
                            print(f"Terminally failed task {task_id} due to exhausted attempts (stuck session)")
                        except Exception as e:
                            print(f"Failed to terminally fail task {task_id}: {e}", file=sys.stderr)
            elif is_active:
                # Session is active, leave it bound
                print(f"Session {session_id} is active, leaving task {task_id} bound")
            else:
                # Has messages or other activity, leave it bound
                print(f"Session {session_id} has activity/messages, leaving task {task_id} bound")
    
    return recovered_tasks


def create_worker_session(task: Dict) -> Optional[str]:
    """
    Step 4b: Create a worker session for the task.
    Returns session_id if successful, None otherwise.
    """
    task_id = task.get("id")
    issue = task.get("issue")
    packet = task.get("packet", {})
    route = packet.get("route") or "workhorse"  # Default route
    
    # Determine model based on route
    model_map = {
        "workhorse": {"providerID": "opencode-route", "id": "workhorse"},
        "specialist": {"providerID": "opencode-route", "id": "specialist"},
        "summit": {"providerID": "opencode-route", "id": "summit"}
    }
    model = model_map.get(route, model_map["workhorse"])
    
    # Create session payload
    payload = {
        "location": {
            "directory": "/home/user/git/code"  # Primary repo directory
        },
        "title": f"opencode-lane: {issue}",
        "model": model,
        "agent": "build"
    }
    
    try:
        import requests
        url = f"{OPENCODE_SERVER}/api/session"
        headers = {"Content-Type": "application/json"}
        response = requests.post(url, json=payload, timeout=10)
        
        if response.status_code == 200:
            session_data = response.json()
            session_id = session_data.get("id")
            
            # Verify location before proceeding
            location = session_data.get("location", {})
            if location.get("directory") == "/home/user/git/code":
                print(f"Created session {session_id} for task {task_id}")
                return session_id
            else:
                # Wrong location - delete and fail
                print(f"Session {session_id} created at wrong location: {location.get('directory')}",
                      file=sys.stderr)
                delete_url = f"{OPENCODE_SERVER}/api/session/{session_id}"
                requests.delete(delete_url, timeout=5)
                return None
        else:
            print(f"Failed to create session for task {task_id}: status {response.status_code}",
                  file=sys.stderr)
            print(f"Response: {response.text}", file=sys.stderr)
            return None
    except Exception as e:
        print(f"Exception creating session for task {task_id}: {e}", file=sys.stderr)
        return None


def bind_and_launch(task: Dict, session_id: str) -> bool:
    """
    Steps 4c-4e: Bind the session to the task and send the worker prompt.
    Returns True if successful, False otherwise.
    """
    task_id = task.get("id")
    worker_id = task.get("worker_id")
    issue = task.get("issue")
    packet = task.get("packet", {})
    
    if not worker_id:
        print(f"Task {task_id} has no worker_id, cannot bind", file=sys.stderr)
        return False
    
    # Step 4c: Bind the session
    try:
        bind_cmd = [
            REPO_QUEUE_CLI,
            "bind", task_id,
            "--worker", "opencode-claimant",
            "--session-id", session_id
        ]
        result = run_command(bind_cmd)
        
        if result.returncode != 0:
            print(f"Failed to bind session {session_id} to task {task_id}: {result.stderr}",
                  file=sys.stderr)
            return False
        
        # Checkpoint as launched
        checkpoint_cmd = [
            REPO_QUEUE_CLI,
            "checkpoint", task_id,
            "--worker", worker_id,
            "--stage", "launched",
            "--execution", json.dumps({
                "lane": "opencode",
                "session_id": session_id
            })
        ]
        result = run_command(checkpoint_cmd)
        
        if result.returncode != 0:
            print(f"Failed to checkpoint task {task_id}: {result.stderr}", file=sys.stderr)
            return False
        
        print(f"Bound and checkpointed task {task_id} to session {session_id}")
        
        # Step 4d: Send worker prompt
        # The prompt should instruct the worker to work THIS task
        prompt = f"""
You are a worker for queue task {task_id} (issue: {issue}).

This task is claimed and lane-bound to opencode execution lane.

WORK ON THIS TASK ONLY. Do not claim or work on other tasks.

Use the repo-queue CLI with --worker {worker_id} for all operations:
- repo-queue heartbeat <id> --worker {worker_id}
- repo-queue checkpoint <id> --worker {worker_id} --stage <stage>
- repo-queue complete <id> --worker {worker_id}
- etc.

When finished, use the appropriate terminal verb with:
--execution '{{"lane": "opencode", "session_id": "{session_id}"}}'

Task details:
{json.dumps(packet, indent=2)}
"""
        
        import requests
        url = f"{OPENCODE_SERVER}/api/session/{session_id}/message"
        payload = {
            "text": prompt,
            "type": "user"
        }
        response = requests.post(url, json=payload, timeout=10)
        
        if response.status_code == 200:
            print(f"Sent prompt to session {session_id} for task {task_id}")
            
            # Step 4e: Post agent-note
            note_cmd = [
                REPO_QUEUE_CLI,
                "agent-note", task_id,
                "--state", "handed off",
                "--text", f"Task {task_id} handed off to session {session_id}"
            ]
            result = run_command(note_cmd)
            
            if result.returncode == 0:
                print(f"Posted agent-note for task {task_id}")
                return True
            else:
                print(f"Failed to post agent-note: {result.stderr}", file=sys.stderr)
                return False
        else:
            print(f"Failed to send prompt to session {session_id}: status {response.status_code}",
                  file=sys.stderr)
            return False
            
    except Exception as e:
        print(f"Exception during bind/launch for task {task_id}: {e}", file=sys.stderr)
        return False


def log_fire(mode: str, candidates: List[Dict], launched: List[Dict], 
             cap_room: Optional[int], skips: List[Dict]) -> None:
    """
    Step 6: Append fire log entry.
    """
    try:
        STATE_DIR.mkdir(parents=True, exist_ok=True)
        
        entry = {
            "timestamp": datetime.utcnow().isoformat() + "Z",
            "mode": mode,
            "candidates_seen": len(candidates),
            "sessions_launched": len(launched),
            "cap_room": cap_room,
            "launched": launched,
            "skips": skips
        }
        
        with FIRE_LOG.open("a") as f:
            f.write(json.dumps(entry) + "\n")
        
        print(f"Fire logged: {json.dumps(entry, indent=2)}")
    except Exception as e:
        print(f"Failed to log fire: {e}", file=sys.stderr)


def should_skip_task(task: Dict) -> Tuple[bool, str]:
    """
    Check if a task should be skipped based on issue labels.
    Returns (should_skip, reason)
    """
    issue = task.get("issue", "")
    
    # Check for labels that should prevent auto-launch
    skip_labels = ["held", "user/owned", "do-not-merge", "do-not-auto-edit"]
    
    # In a real implementation, we would fetch issue details
    # For now, we'll skip based on common patterns in the issue field
    issue_lower = issue.lower()
    
    for label in skip_labels:
        if label in issue_lower:
            return True, f"issue contains '{label}' label"
    
    return False, ""


def main():
    parser = argparse.ArgumentParser(description="OpenCode lane claimant/executor")
    parser.add_argument("--dry-run", action="store_true", 
                        help="Run in dry-run mode (equivalent to shadow)")
    args = parser.parse_args()
    
    # Determine mode
    mode = "live" if not args.dry_run else "shadow"
    actual_mode = get_mode()
    
    if actual_mode == "shadow":
        print("Running in SHADOW mode (discovery only)")
        mode = "shadow"
    elif mode == "live" and actual_mode != "live":
        print(f"Mode file specifies '{actual_mode}', running in {actual_mode} mode")
        mode = actual_mode
    
    print(f"Starting fire in {mode} mode")
    
    # Step 1: Supervision (always runs in live mode)
    recovered_tasks = []
    if mode == "live":
        print("\n=== Step 1: Supervision ===")
        recovered_tasks = supervision_step()
        print(f"Recovered {len(recovered_tasks)} stuck_bound tasks")
    
    # Step 2: Gate - Check if we have any candidates
    print("\n=== Step 2: Gate ===")
    candidates = get_candidates()
    
    if not candidates:
        print("No unbound opencode-lane candidates found")
        log_fire(mode, candidates, [], None, [])
        return 0
    
    print(f"Found {len(candidates)} unbound opencode-lane candidates")
    
    # Step 3: Cap - Check concurrency limit
    print("\n=== Step 3: Cap ===")
    cap_room, active_sessions = get_active_root_sessions()
    
    if cap_room is None:
        print("Failed to determine cap_room, aborting fire")
        log_fire(mode, candidates, [], None, [])
        return 1
    
    if cap_room <= 0:
        print(f"Cap reached (cap_room={cap_room}), aborting fire")
        log_fire(mode, candidates, [], cap_room, [])
        return 0
    
    print(f"Cap room available: {cap_room} sessions")
    
    # Filter candidates by priority (highest queue priority first)
    # QUEUES list defines priority order
    prioritized = []
    for queue in QUEUES:
        queue_candidates = [c for c in candidates if c.get("queue") == queue]
        prioritized.extend(queue_candidates)
    
    # Also include any candidates not in our queues (shouldn't happen but be safe)
    prioritized.extend([c for c in candidates if c.get("queue") not in QUEUES])
    
    print(f"Prioritized {len(prioritized)} candidates")
    
    # Step 4: Launch candidates while cap room remains
    print("\n=== Step 4: Launch ===")
    launched = []
    skips = []
    
    for task in prioritized:
        if len(launched) >= cap_room:
            print(f"Cap reached, stopping after {len(launched)} launches")
            break
        
        task_id = task.get("id")
        
        # Re-verify task is still valid candidate
        current_candidates = get_candidates()
        current_task = next((c for c in current_candidates if c.get("id") == task_id), None)
        
        if not current_task:
            reason = "no longer a candidate (claimed by another lane)"
            print(f"Skipping task {task_id}: {reason}")
            skips.append({
                "task_id": task_id,
                "reason": reason
            })
            continue
        
        # Check for skip labels
        should_skip, skip_reason = should_skip_task(current_task)
        if should_skip:
            print(f"Skipping task {task_id}: {skip_reason}")
            skips.append({
                "task_id": task_id,
                "reason": skip_reason
            })
            continue
        
        # Mode-specific handling
        if mode == "shadow":
            # In shadow mode, we would-launch but don't actually launch
            session_id = f"would-be-session-{task_id}"
            print(f"[SHADOW] Would launch task {task_id} as session {session_id}")
            launched.append({
                "task_id": task_id,
                "session_id": session_id,
                "mode": "shadow"
            })
            continue
        
        # Live mode: actual execution
        print(f"\nProcessing task {task_id}")
        
        # Step 4b: Create worker session
        session_id = create_worker_session(current_task)
        if not session_id:
            reason = "session_creation_failed"
            print(f"Failed to create session for task {task_id}")
            skips.append({
                "task_id": task_id,
                "reason": reason
            })
            continue
        
        # Steps 4c-4e: Bind and launch
        if bind_and_launch(current_task, session_id):
            launched.append({
                "task_id": task_id,
                "session_id": session_id,
                "mode": "live"
            })
        else:
            reason = "bind_or_launch_failed"
            print(f"Failed to bind/launch task {task_id}")
            skips.append({
                "task_id": task_id,
                "reason": reason
            })
            # Clean up the created session
            delete_session(session_id)
    
    # Step 6: Log fire
    print("\n=== Step 6: Fire Log ===")
    log_fire(mode, candidates, launched, cap_room, skips)
    
    print(f"\nFire completed in {mode} mode:")
    print(f"  Candidates seen: {len(candidates)}")
    print(f"  Sessions launched: {len(launched)}")
    print(f"  Tasks skipped: {len(skips)}")
    print(f"  Cap room: {cap_room}")
    
    return 0


if __name__ == "__main__":
    sys.exit(main())