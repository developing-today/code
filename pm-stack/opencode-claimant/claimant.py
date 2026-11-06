# opencode-lane claimant/executor - Slice 1 implementation (#3026)
# Deterministic execution per specification

import json
import os
import sys
import time
import requests
from datetime import datetime
from typing import List, Dict, Any, Optional

# Constants
QUEUES = [
    "repo-health-code-pilot",
    "repo-health-opencode-pilot",
    "repo-health-openchamber-pilot",
    "repo-health-nix-pilot"
]

REPO_QUEUE_PATH = "/home/user/fleet-brokers/bin/repo-queue"
OPENCODE_SERVER = "http://127.0.0.1:13420"
ACTIVE_WINDOW_MS = 30 * 60 * 1000  # 30 minutes
CONCURRENCY_CAP = 36  # Active root sessions cap

MODE_FILE = os.path.expanduser("~/.local/state/opencode-claimant/mode")
FIRES_LOG = os.path.expanduser("~/.local/state/opencode-claimant/fires.jsonl")

class ClaimantError(Exception):
    """Base exception for claimant operations"""
    pass

def read_mode() -> str:
    """Read the execution mode from file. Defaults to 'shadow' if missing/unreadable."""
    try:
        with open(MODE_FILE, 'r') as f:
            mode = f.read().strip()
            if mode in ['shadow', 'live']:
                return mode
    except Exception:
        pass
    return 'shadow'

def log_fire(mode: str, candidates: List[Dict], launched: List[Dict], cap_room: int, skips: List[Dict]) -> None:
    """Append a fire log entry to fires.jsonl"""
    fire_record = {
        "timestamp": datetime.utcnow().isoformat(),
        "mode": mode,
        "candidates_seen": len(candidates),
        "sessions_launched": len(launched),
        "cap_room": cap_room,
        "candidates": candidates,
        "launched": launched,
        "skips": skips
    }
    
    # Ensure directory exists
    os.makedirs(os.path.dirname(FIRES_LOG), exist_ok=True)
    
    with open(FIRES_LOG, 'a') as f:
        f.write(json.dumps(fire_record) + "\n")

def run_repo_queue_command(args: List[str]) -> Dict[str, Any]:
    """Run a repo-queue command and return parsed JSON output"""
    cmd = [REPO_QUEUE_PATH, "--worker", "opencode-claimant"] + args
    
    try:
        import subprocess
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=30
        )
        
        if result.returncode != 0:
            raise ClaimantError(f"repo-queue failed: {result.stderr}")
        
        try:
            return json.loads(result.stdout)
        except json.JSONDecodeError:
            raise ClaimantError(f"repo-queue returned non-JSON: {result.stdout}")
    except subprocess.TimeoutExpired:
        raise ClaimantError("repo-queue command timed out")
    except Exception as e:
        raise ClaimantError(f"repo-queue execution error: {str(e)}")

def get_active_root_sessions() -> int:
    """Get count of active root sessions from OpenCode server"""
    try:
        response = requests.get(f"{OPENCODE_SERVER}/api/session", timeout=5)
        response.raise_for_status()
        sessions = response.json()
        
        active_count = 0
        now_ms = int(time.time() * 1000)
        
        for session in sessions:
            # Check if it's a root session (no parent)
            if session.get("parent_id") is None:
                # Check for terminal markers
                has_terminal = session.get("time", {}).get("idle") is not None
                has_outcome = session.get("outcome") is not None
                
                if not has_terminal and not has_outcome:
                    # Check activity within ACTIVE_WINDOW_MS
                    last_activity = session.get("time", {}).get("last_message") or session.get("time", {}).get("created")
                    if last_activity and (now_ms - last_activity) < ACTIVE_WINDOW_MS:
                        active_count += 1
        
        return active_count
    except Exception as e:
        print(f"Warning: Could not get active sessions: {e}", file=sys.stderr)
        return -1  # Indicates error

def get_task_candidates() -> List[Dict[str, Any]]:
    """Discover unbound opencode-lane tasks using poll.py logic"""
    candidates = []
    
    for queue in QUEUES:
        try:
            result = run_repo_queue_command(["health", queue, "--worker", "opencode-claimant"])
            
            if result.get("claimed_with_lane_opencode_unbound"):
                candidates.extend(result["claimed_with_lane_opencode_unbound"])
        except Exception as e:
            print(f"Warning: Could not check queue {queue}: {e}", file=sys.stderr)
    
    return candidates

def verify_session_location(session_id: str, expected_dir: str) -> bool:
    """Verify a session is at the expected directory"""
    try:
        response = requests.get(f"{OPENCODE_SERVER}/api/session/{session_id}", timeout=5)
        response.raise_for_status()
        session = response.json()
        
        actual_dir = session.get("location", {}).get("directory")
        return actual_dir == expected_dir
    except Exception:
        return False

def create_worker_session(task_title: str, model_id: str) -> Optional[str]:
    """Create a worker session on the OpenCode server"""
    payload = {
        "location": {"directory": "/home/user/git/code"},
        "title": task_title,
        "model": {"id": model_id, "providerID": "opencode-route"},
        "agent": "build"
    }
    
    try:
        response = requests.post(f"{OPENCODE_SERVER}/api/session", json=payload, timeout=10)
        response.raise_for_status()
        session = response.json()
        
        if not verify_session_location(session.get("id"), "/home/user/git/code"):
            try:
                requests.delete(f"{OPENCODE_SERVER}/api/session/{session.get('id')}", timeout=5)
            except Exception:
                pass
            return None
        
        return session.get("id")
    except Exception as e:
        print(f"Warning: Session creation failed: {e}", file=sys.stderr)
        return None

def supervision_recovery(mode: str) -> List[Dict[str, Any]]:
    """Step 1: Supervision and recovery of stuck_bound sessions (live mode only)"""
    recovered = []
    
    if mode != "live":
        return recovered
    
    bound_tasks = []
    for queue in QUEUES:
        try:
            result = run_repo_queue_command(["health", queue, "--worker", "opencode-claimant"])
            if result.get("claimed_with_lane_opencode_bound"):
                bound_tasks.extend(result["claimed_with_lane_opencode_bound"])
        except Exception as e:
            print(f"Warning: Could not check bound tasks in {queue}: {e}", file=sys.stderr)
    
    for task in bound_tasks:
        session_id = task.get("progress", {}).get("session_id")
        if not session_id:
            continue
        
        try:
            response = requests.get(f"{OPENCODE_SERVER}/api/session/{session_id}", timeout=5)
            if response.status_code == 404:
                if task.get("attempts", 0) < task.get("max_attempts", 5):
                    try:
                        result = run_repo_queue_command([
                            "bind", task.get("id"), "--worker", "opencode-claimant",
                            "--session-id", session_id, "--launched-by", "opencode-claimant-recovery:gone-session"
                        ])
                        if result.get("success"):
                            recovered.append({"task_id": task.get("id"), "session_id": session_id, "action": "recovered_gone_session"})
                    except Exception as recovery_error:
                        print(f"Recovery failed for task {task.get('id')}: {recovery_error}", file=sys.stderr)
                else:
                    try:
                        run_repo_queue_command([
                            "fail", task.get("id"), "--worker", task.get("worker_id"),
                            "--error", "bound session unavailable without terminal outcome; attempts exhausted",
                            "--reason-class", "deterministic",
                            "--execution", json.dumps({"lane": "opencode", "session_id": session_id, "fallback_reason": "launch_failed_attempts_exhausted"})
                        ])
                    except Exception as fail_error:
                        print(f"Failed to terminally fail task {task.get('id')}: {fail_error}", file=sys.stderr)
            else:
                session = response.json()
                messages_response = requests.get(f"{OPENCODE_SERVER}/api/session/{session_id}/messages", timeout=5)
                if messages_response.status_code == 200:
                    messages = messages_response.json()
                    if len(messages) == 0:
                        if task.get("attempts", 0) < task.get("max_attempts", 5):
                            try:
                                requests.delete(f"{OPENCODE_SERVER}/api/session/{session_id}", timeout=5)
                                result = run_repo_queue_command([
                                    "bind", task.get("id"), "--worker", "opencode-claimant",
                                    "--session-id", session_id, "--launched-by", "opencode-claimant-recovery:zero-message-deleted"
                                ])
                                if result.get("success"):
                                    recovered.append({"task_id": task.get("id"), "session_id": session_id, "action": "recovered_zero_message"})
                            except Exception as recovery_error:
                                print(f"Zero-message recovery failed for task {task.get('id')}: {recovery_error}", file=sys.stderr)
                        else:
                            try:
                                run_repo_queue_command([
                                    "fail", task.get("id"), "--worker", task.get("worker_id"),
                                    "--error", "bound session unavailable without terminal outcome; attempts exhausted",
                                    "--reason-class", "deterministic",
                                    "--execution", json.dumps({"lane": "opencode", "session_id": session_id, "fallback_reason": "launch_failed_attempts_exhausted"})
                                ])
                            except Exception as fail_error:
                                print(f"Failed to terminally fail task {task.get('id')}: {fail_error}", file=sys.stderr)
        except Exception as e:
            print(f"Warning: Could not verify session {session_id}: {e}", file=sys.stderr)
    
    return recovered

def launch_tasks(mode: str, candidates: List[Dict[str, Any]], cap_room: int) -> List[Dict[str, Any]]:
    """Step 4: Launch tasks within cap constraints"""
    launched = []
    skips = []
    
    if cap_room <= 0:
        skips.append({"reason": "cap_saturated", "cap_room": cap_room, "candidates_skipped": len(candidates)})
        return launched
    
    for queue in QUEUES:
        queue_candidates = [c for c in candidates if c.get("queue") == queue]
        
        for candidate in queue_candidates:
            if cap_room <= 0:
                skips.append({"task_id": candidate.get("id"), "reason": "cap_exhausted_during_launch", "remaining_cap": cap_room})
                continue
            
            task_id = candidate.get("id")
            worker_id = candidate.get("worker_id")
            
            try:
                result = run_repo_queue_command(["health", queue, "--worker", "opencode-claimant"])
                task_found = False
                for check_candidate in result.get("claimed_with_lane_opencode_unbound", []):
                    if check_candidate.get("id") == task_id:
                        task_found = True
                        candidate = check_candidate
                        break
                
                if not task_found:
                    skips.append({"task_id": task_id, "reason": "task_state_changed", "details": "No longer unbound or lane changed"})
                    continue
                
                issue_labels = candidate.get("issue_labels", [])
                skip_conditions = ["held", "user/owned", "do-not-merge", "do-not-auto-edit"]
                if any(cond in issue_labels for cond in skip_conditions):
                    skips.append({"task_id": task_id, "reason": "skip_label", "labels": issue_labels})
                    continue
                
                task_title = f"Task {task_id}: {candidate.get('title', 'Untitled')}"
                model_id = candidate.get("route_model", "workhorse")
                
                session_id = create_worker_session(task_title, model_id)
                if not session_id:
                    skips.append({"task_id": task_id, "reason": "session_creation_failed"})
                    continue
                
                try:
                    bind_result = run_repo_queue_command(["bind", task_id, "--worker", "opencode-claimant", "--session-id", session_id])
                    
                    if not bind_result.get("success"):
                        try:
                            requests.delete(f"{OPENCODE_SERVER}/api/session/{session_id}", timeout=5)
                        except Exception:
                            pass
                        skips.append({"task_id": task_id, "reason": "bind_failed", "details": "Session already bound"})
                        continue
                    
                    checkpoint_result = run_repo_queue_command(["checkpoint", task_id, "--worker", worker_id, "--stage", "launched"])
                    
                    if not checkpoint_result.get("success"):
                        skips.append({"task_id": task_id, "reason": "checkpoint_failed"})
                        try:
                            requests.delete(f"{OPENCODE_SERVER}/api/session/{session_id}", timeout=5)
                        except Exception:
                            pass
                        continue
                    
                    launched.append({"task_id": task_id, "session_id": session_id, "queue": queue, "worker_id": worker_id, "timestamp": datetime.utcnow().isoformat()})
                    cap_room -= 1
                    
                except Exception as e:
                    try:
                        requests.delete(f"{OPENCODE_SERVER}/api/session/{session_id}", timeout=5)
                    except Exception:
                        pass
                    skips.append({"task_id": task_id, "reason": "launch_error", "error": str(e)})
            
            except Exception as e:
                skips.append({"task_id": task_id, "reason": "verification_error", "error": str(e)})
    
    return launched

def main():
    """Main execution entry point"""
    mode = read_mode()
    print(f"opencode-claimant running in {mode} mode")
    
    if mode == "live":
        print("Running supervision/recovery...")
        recovered = supervision_recovery(mode)
        for rec in recovered:
            print(f"  Recovered: {rec}")
    
    print("Discovering candidates...")
    candidates = get_task_candidates()
    
    if not candidates:
        print("No candidates found - exiting")
        log_fire(mode, candidates, [], 0, [{"reason": "no_candidates"}])
        return 0
    
    print("Checking concurrency cap...")
    cap_room = get_active_root_sessions()
    
    if cap_room == -1:
        print("Warning: Could not determine cap room - exiting")
        log_fire(mode, candidates, [], -1, [{"reason": "cap_check_error"}])
        return 1
    
    if cap_room <= 0:
        print(f"Cap saturated ({cap_room} active sessions) - exiting")
        log_fire(mode, candidates, [], cap_room, [{"reason": "cap_saturated", "cap_room": cap_room}])
        return 0
    
    # In shadow mode, do not launch - only discover and report
    if mode == "shadow":
        print(f"Shadow mode: discovered {len(candidates)} candidates, cap_room={cap_room}, launching nothing")
        log_fire(mode, candidates, [], cap_room, [{"reason": "shadow_no_launches"}])
        return 0
    
    print(f"Cap room available: {cap_room}")
    print(f"Launching up to {cap_room} tasks...")
    launched = launch_tasks(mode, candidates, cap_room)
    
    remaining_cap = cap_room - len(launched)
    log_fire(mode, candidates, launched, remaining_cap, [])
    
    print(f"Fire complete. Launched: {len(launched)}, Candidates: {len(candidates)}")
    return 0

if __name__ == "__main__":
    sys.exit(main())
