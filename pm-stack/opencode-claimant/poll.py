#!/usr/bin/env python3
"""
poll.py - Candidate discovery for opencode-lane claimant

Discovers tasks that are:
- Claimed by a repo worker
- Assigned to lane 'opencode'
- Unbound (no session_id in progress)
- At yield stage 'handed_off' or 'launch_failed'

Also reports:
- Active root session count (cap_room)
- stuck_bound tasks (bound sessions that never received prompt)

Output: JSON with candidates, cap_room, and stuck_bound list
"""

import json
import os
import sys
import requests
import time
from typing import List, Dict, Any, Optional

# Constants
PRIMARY_REPO_DIR = "/home/user/git/code"
OPENCODE_SERVER = "http://127.0.0.1:13420"
ACTIVE_WINDOW_MS = 30 * 60 * 1000  # 30 minutes
QUEUES = [
    "repo-health-code-pilot",
    "repo-health-opencode-pilot",
    "repo-health-openchamber-pilot",
    "repo-health-nix-pilot"
]


def get_active_root_sessions() -> int:
    """Count active root sessions (no terminal marker, activity in last 30 min)"""
    try:
        response = requests.get(f"{OPENCODE_SERVER}/api/session", timeout=5)
        response.raise_for_status()
        sessions = response.json()
        
        active_count = 0
        now_ms = int(time.time() * 1000)
        
        for session in sessions:
            # Skip non-root sessions
            if session.get("isRoot") != True:
                continue
                
            # Check for terminal markers
            terminal_markers = ["idle", "outcome"]
            has_terminal = any(
                marker in session.get("time", {}) for marker in terminal_markers
            )
            if has_terminal:
                continue
                
            # Check activity timestamp
            last_active = session.get("time", {}).get("idle") or session.get("time", {}).get("prompted")
            if last_active and (now_ms - last_active) <= ACTIVE_WINDOW_MS:
                active_count += 1
                
        return active_count
    except Exception as e:
        print(f"Error getting active sessions: {e}", file=sys.stderr)
        return None


def get_stuck_bound_tasks() -> List[Dict[str, Any]]:
    """
    Get tasks that are bound but never received prompt (stuck_bound).
    These are tasks where:
    - session exists as root but has no terminal marker
    - no user messages in session
    - silent for at least ACTIVE_WINDOW_MS
    - lease has expired
    """
    stuck = []
    
    # Check each queue for bound tasks
    for queue in QUEUES:
        try:
            # Get tasks from queue
            response = requests.get(
                f"{OPENCODE_SERVER}/api/queue/{queue}/tasks",
                timeout=5
            )
            if response.status_code != 200:
                continue
                
            tasks = response.json()
            
            for task in tasks:
                # Check if bound to opencode lane
                if task.get("lane") != "opencode":
                    continue
                    
                # Check if bound (has session_id)
                session_id = task.get("progress", {}).get("session_id")
                if not session_id:
                    continue
                    
                # Check if at yield stage (handed_off or launch_failed)
                stage = task.get("progress", {}).get("stage")
                if stage not in ["handed_off", "launch_failed"]:
                    continue
                    
                # Check attempts remaining
                attempts = task.get("attempts", 0)
                max_attempts = task.get("max_attempts", 5)
                if attempts >= max_attempts:
                    continue
                    
                # Verify session exists and is stuck
                try:
                    session_response = requests.get(
                        f"{OPENCODE_SERVER}/api/session/{session_id}",
                        timeout=5
                    )
                    if session_response.status_code != 200:
                        # Session doesn't exist - eligible for recovery
                        stuck.append({
                            "task_id": task.get("id"),
                            "session_id": session_id,
                            "queue": queue,
                            "worker_id": task.get("worker_id"),
                            "attempts": attempts,
                            "max_attempts": max_attempts,
                            "reason": "session_gone"
                        })
                        continue
                    
                    session = session_response.json()
                    
                    # Check if session is root with no terminal marker
                    if session.get("isRoot") != True:
                        continue
                        
                    has_terminal = any(
                        marker in session.get("time", {}) 
                        for marker in ["idle", "outcome"]
                    )
                    if has_terminal:
                        continue
                    
                    # Check for user messages (would indicate activity)
                    messages_response = requests.get(
                        f"{OPENCODE_SERVER}/api/session/{session_id}/messages",
                        timeout=5
                    )
                    if messages_response.status_code == 200:
                        messages = messages_response.json()
                        if messages:
                            # Has messages - not stuck
                            continue
                    
                    # Check activity timestamp
                    now_ms = int(time.time() * 1000)
                    last_active = session.get("time", {}).get("idle") or session.get("time", {}).get("prompted")
                    if last_active and (now_ms - last_active) > ACTIVE_WINDOW_MS:
                        stuck.append({
                            "task_id": task.get("id"),
                            "session_id": session_id,
                            "queue": queue,
                            "worker_id": task.get("worker_id"),
                            "attempts": attempts,
                            "max_attempts": max_attempts,
                            "reason": "zero_messages"
                        })
                        
                except Exception:
                    # If we can't verify, skip this task
                    continue
                    
        except Exception:
            # If queue is unavailable, continue to next
            continue
            
    return stuck


def get_candidates() -> Dict[str, Any]:
    """Get unbound opencode-lane candidates"""
    candidates = []
    
    for queue in QUEUES:
        try:
            response = requests.get(
                f"{OPENCODE_SERVER}/api/queue/{queue}/tasks",
                timeout=5
            )
            if response.status_code != 200:
                continue
                
            tasks = response.json()
            
            for task in tasks:
                # Must be claimed by a repo worker
                if not task.get("claimed_by"):
                    continue
                    
                # Must be lane opencode
                if task.get("lane") != "opencode":
                    continue
                    
                # Must be unbound (no session_id)
                if task.get("progress", {}).get("session_id"):
                    continue
                    
                # Must be at yield stage
                stage = task.get("progress", {}).get("stage")
                if stage not in ["handed_off", "launch_failed"]:
                    continue
                    
                candidates.append({
                    "task_id": task.get("id"),
                    "issue": task.get("issue"),
                    "queue": queue,
                    "worker_id": task.get("worker_id"),
                    "lane": task.get("lane"),
                    "stage": stage,
                    "attempts": task.get("attempts", 0),
                    "max_attempts": task.get("max_attempts", 5)
                })
                
        except Exception as e:
            print(f"Error checking queue {queue}: {e}", file=sys.stderr)
            continue
            
    return {"candidates": candidates}


def main():
    """Main entry point"""
    mode = os.environ.get("MODE", "shadow")
    
    # Get candidates
    candidates_result = get_candidates()
    candidates = candidates_result["candidates"]
    
    # Get cap room
    cap_room = get_active_root_sessions()
    
    # Get stuck_bound tasks
    stuck_bound = get_stuck_bound_tasks()
    
    # Output JSON
    output = {
        "mode": mode,
        "candidates": candidates,
        "cap_room": cap_room,
        "stuck_bound": stuck_bound
    }
    
    print(json.dumps(output, indent=2))
    
    # Exit with appropriate code
    if mode == "live":
        sys.exit(0 if candidates else 1)
    else:
        sys.exit(0)


if __name__ == "__main__":
    main()
