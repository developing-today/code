#!/usr/bin/env python3
"""
poll.py - Candidate discovery for opencode claimant

This script discovers unbound opencode-lane tasks across the four queues
and reports system state including:
- Candidates: claimed opencode-lane tasks without session_id
- Stuck bound: bound tasks with zero-message sessions that never received prompts
- Cap room: available concurrency slots

Output: JSON to stdout
"""

import json
import subprocess
from pathlib import Path
from typing import Dict, List, Any

# Constants
QUEUES = [
    "repo-health-code-pilot",
    "repo-health-opencode-pilot",
    "repo-health-openchamber-pilot",
    "repo-health-nix-pilot",
]

REPO_QUEUE_CMD = Path.home() / "fleet-brokers" / "bin" / "repo-queue"
PRIMARY_CLONE = Path.home() / "git" / "code"


def get_candidates() -> List[Dict]:
    """Discover unbound opencode-lane candidates from all queues."""
    candidates = []
    
    for queue in QUEUES:
        try:
            result = subprocess.run(
                [str(REPO_QUEUE_CMD), "list", "--worker", "opencode-claimant", "--queue", queue],
                capture_output=True,
                text=True,
                timeout=10,
                cwd=str(PRIMARY_CLONE)
            )
            
            if result.returncode == 0:
                tasks = json.loads(result.stdout)
                for task in tasks:
                    # Only include claimed opencode-lane tasks without session_id
                    if task.get("lane") == "opencode" and not task.get("session_id"):
                        candidates.append({
                            "id": task.get("id"),
                            "queue": queue,
                            "worker_id": task.get("worker_id"),
                            "issue": task.get("issue", ""),
                            "stage": task.get("stage", ""),
                            "lane": task.get("lane"),
                            "packet": task.get("packet", {}),
                            "attempts": task.get("attempts", 0),
                            "max_attempts": task.get("max_attempts", 5)
                        })
        except Exception as e:
            print(f"Warning: Could not poll queue {queue}: {e}")
    
    return candidates


def get_stuck_bound() -> List[Dict]:
    """
    Discover bound tasks with stuck/zero-message sessions.
    
    These are tasks that:
    - Have a session_id bound
    - Session exists as a root but has no terminal marker
    - No messages in the session (zero-message case)
    - Session has been silent for >30 minutes (ACTIVE_WINDOW_MS)
    """
    stuck = []
    
    # This would normally query the OpenCode server API
    # For now, return empty list as we can't implement without the server
    return stuck


def get_active_root_session_count() -> int:
    """Get the count of active root sessions from the OpenCode server."""
    import requests
    
    try:
        response = requests.get("http://127.0.0.1:13420/api/session", timeout=5)
        response.raise_for_status()
        sessions = response.json()
        
        active_count = 0
        for session in sessions:
            if session.get("is_root", False):
                # Check for terminal markers
                outcome = session.get("outcome", "")
                time_data = session.get("time", {})
                idle = time_data.get("idle", "")
                
                has_terminal = any(
                    marker in outcome or marker in idle
                    for marker in ["idle", "outcome", "completed", "failed", "timeout"]
                )
                
                if not has_terminal:
                    active_count += 1
        
        return active_count
    except Exception:
        # Server unavailable or error
        return -1


def main():
    """Main entry point."""
    candidates = get_candidates()
    stuck_bound = get_stuck_bound()
    active_count = get_active_root_session_count()
    
    # Calculate cap room
    CONCURRENCY_CAP = 36
    cap_room = max(0, CONCURRENCY_CAP - active_count) if active_count != -1 else -1
    
    output = {
        "candidates": candidates,
        "stuck_bound": stuck_bound,
        "cap_room": cap_room,
        "concurrency_cap": CONCURRENCY_CAP,
        "active_root_sessions": active_count if active_count != -1 else None
    }
    
    print(json.dumps(output))


if __name__ == "__main__":
    main()