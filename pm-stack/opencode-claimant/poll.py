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
import time
from pathlib import Path
from typing import Dict, List, Any, Optional, Set

# Constants
QUEUES = [
    "repo-health-code-pilot",
    "repo-health-opencode-pilot",
    "repo-health-openchamber-pilot",
    "repo-health-nix-pilot",
]

REPO_QUEUE_CMD = Path.home() / "fleet-brokers" / "bin" / "repo-queue"
PRIMARY_CLONE = Path.home() / "git" / "code"

# Concurrency cap
CAP = 36

# Active window threshold (30 minutes in milliseconds)
ACTIVE_WINDOW_MS = 30 * 60 * 1000


def is_active_root(session: Dict, now_ms: int, active_set: Optional[Set[str]] = None) -> bool:
    """
    Check if a session is an active root session.
    
    A root session is active if:
    - It has no terminal markers (idle, outcome, completed, failed, timeout)
    - It's within the active window (updated within 30 minutes) OR in the active set
    - It's not a child session (no parentID/parent_id)
    """
    # Must be a dict
    if not isinstance(session, dict):
        return False
    
    # Check if it's a root session
    if not session.get("is_root", False):
        return False
    
    # Check for child session markers
    parent_id = session.get("parentID") or session.get("parent_id")
    if parent_id:
        return False
    
    # Check for terminal markers
    outcome = session.get("outcome", "")
    time_data = session.get("time", {})
    idle = time_data.get("idle", "")
    
    # Handle case where idle/outcome might be integers
    has_terminal = False
    if isinstance(outcome, str):
        has_terminal = any(marker in outcome for marker in ["idle", "outcome", "completed", "failed", "timeout"])
    if isinstance(idle, str):
        has_terminal = has_terminal or any(marker in idle for marker in ["idle", "outcome", "completed", "failed", "timeout"])
    
    if has_terminal:
        return False
    
    # Check if in active set (resumed sessions)
    if active_set is not None and session.get("id") in active_set:
        return True
    
    # Check active window (updated within 30 minutes)
    updated = time_data.get("updated", 0)
    created = time_data.get("created", 0)
    
    # Use updated if available, otherwise created
    last_activity = updated if updated else created
    
    if last_activity <= 0:
        return False
    
    # Session is active if updated within the window
    return (now_ms - last_activity) < ACTIVE_WINDOW_MS


def count_active_roots(sessions: List[Dict], now_ms: int, active_set: Optional[Set[str]] = None) -> int:
    """
    Count active root sessions from a list.
    """
    if not isinstance(sessions, list):
        return 0
    
    return sum(
        1 for session in sessions
        if is_active_root(session, now_ms, active_set)
    )


def summarize_cap(sessions: Optional[List[Dict]], active_set: Optional[Set[str]], now_ms: int) -> tuple:
    """
    Summarize capacity information.
    
    Returns: (live_root_count, active_root_count, cap_room)
    - live_root_count: total root sessions (including finished)
    - active_root_count: active root sessions
    - cap_room: available slots (36 - active) or None if unknown
    """
    if sessions is None:
        return (None, None, None)
    
    live_roots = sum(1 for s in sessions if isinstance(s, dict) and s.get("is_root", False))
    active_roots = count_active_roots(sessions, now_ms, active_set)
    
    if active_set is None:
        # Active set unreadable - fail closed
        return (live_roots, None, None)
    
    cap_room = max(0, CAP - active_roots) if active_roots is not None else None
    return (live_roots, active_roots, cap_room)


def is_candidate(task: Dict) -> bool:
    """
    Check if a task is a valid launch candidate.
    
    A candidate must:
    - Have lane == "opencode"
    - Have stage == "handed_off" (or "launch_failed" for recovery)
    - Not have a session_id bound
    """
    progress = task.get("progress", {})
    lane = progress.get("lane")
    stage = progress.get("stage")
    session_id = progress.get("session_id")
    
    # Must be opencode lane
    if lane != "opencode":
        return False
    
    # Must be unbound
    if session_id:
        return False
    
    # Must be handed_off or launch_failed (recovery case)
    if stage not in ("handed_off", "launch_failed"):
        return False
    
    return True


def parse_lease_ms(lease_until: Optional[str]) -> Optional[int]:
    """
    Parse lease timestamp to milliseconds since epoch.
    
    Returns None if unparseable.
    """
    if not lease_until or not isinstance(lease_until, str):
        return None
    
    # Try various ISO 8601 formats
    for fmt in [
        "%Y-%m-%dT%H:%M:%SZ",
        "%Y-%m-%dT%H:%M:%S.%fZ",
        "%Y-%m-%dT%H:%M:%S+00:00",
        "%Y-%m-%dT%H:%M:%S.%f+00:00",
    ]:
        try:
            import datetime
            dt = datetime.datetime.fromisoformat(lease_until.replace("Z", "+00:00"))
            return int(dt.timestamp() * 1000)
        except (ValueError, AttributeError):
            continue
    
    return None


def is_stuck_bound(task: Dict, session: Optional[Dict], active_set: Optional[Set[str]], now_ms: int) -> bool:
    """
    Check if a bound task has a stuck session (zero-message recovery case).
    
    A stuck bound task has:
    - lane == "opencode"
    - session_id bound
    - session exists but is a zombie (no activity for >30 minutes)
    - lease expired
    - no terminal markers in session
    """
    progress = task.get("progress", {})
    lane = progress.get("lane")
    session_id = progress.get("session_id")
    lease_until = task.get("lease_until")
    
    # Must be opencode lane
    if lane != "opencode":
        return False
    
    # Session must be bound
    if not session_id:
        return False
    
    # Session must exist
    if not session or not isinstance(session, dict):
        return False
    
    # Session must be a root (not child)
    if session.get("parentID") or session.get("parent_id"):
        return False
    
    # Check for terminal markers
    outcome = session.get("outcome", "")
    time_data = session.get("time", {})
    idle = time_data.get("idle", "")
    
    # Handle case where idle/outcome might be integers
    has_terminal = False
    if isinstance(outcome, str):
        has_terminal = any(marker in outcome for marker in ["idle", "outcome", "completed", "failed", "timeout"])
    if isinstance(idle, str):
        has_terminal = has_terminal or any(marker in idle for marker in ["idle", "outcome", "completed", "failed", "timeout"])
    
    if has_terminal:
        return False
    
    # Check lease expiration
    lease_expires = parse_lease_ms(lease_until)
    if lease_expires is None:
        # Unparseable lease - fail closed
        return False
    
    if now_ms < lease_expires:
        return False
    
    # Check active window (silent for >30 minutes)
    updated = time_data.get("updated", 0)
    created = time_data.get("created", 0)
    last_activity = updated if updated else created
    
    if last_activity <= 0:
        return False
    
    if (now_ms - last_activity) < ACTIVE_WINDOW_MS:
        return False
    
    # Check active set membership
    if active_set is not None and session_id in active_set:
        return False
    
    return True


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
                    if is_candidate(task):
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
    - Lease has expired
    """
    stuck = []
    now_ms = int(time.time() * 1000)
    
    # Get all bound opencode-lane tasks
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
                    progress = task.get("progress", {})
                    
                    # Check if bound to opencode lane
                    if progress.get("lane") == "opencode" and progress.get("session_id"):
                        session_id = progress["session_id"]
                        
                        # Try to get session info
                        try:
                            import urllib.request
                            import urllib.error
                            
                            req = urllib.request.Request(
                                f"http://127.0.0.1:13420/api/session/{session_id}",
                                timeout=5
                            )
                            with urllib.request.urlopen(req) as response:
                                if response.status_code == 200:
                                    session = json.loads(response.read().decode('utf-8'))
                                    
                                    # Check if session is stuck (zero-message case)
                                    if is_stuck_bound(task, session, None, now_ms):
                                        # Additional zero-message verification
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
                                            # Could not fetch messages - skip this task
                                            continue
                    
                                        # Only include if truly zero messages
                                        if len(session_messages) == 0:
                                            stuck.append({
                                                "task_id": task.get("id"),
                                                "session_id": session_id,
                                                "attempts": task.get("attempts", 0),
                                                "max_attempts": task.get("max_attempts", 5)
                                            })
                        except Exception:
                            # Could not verify session - skip
                            pass
        except Exception:
            pass
    
    return stuck


def get_active_root_session_count() -> int:
    """Get the count of active root sessions from the OpenCode server."""
    import urllib.request
    import urllib.error
    
    try:
        req = urllib.request.Request("http://127.0.0.1:13420/api/session", timeout=5)
        with urllib.request.urlopen(req) as response:
            sessions = json.loads(response.read().decode('utf-8'))
            
        active_count = count_active_roots(sessions, int(time.time() * 1000), None)
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
    cap_room = max(0, CAP - active_count) if active_count != -1 else -1
    
    output = {
        "candidates": candidates,
        "stuck_bound": stuck_bound,
        "cap_room": cap_room,
        "concurrency_cap": CAP,
        "active_root_sessions": active_count if active_count != -1 else None
    }
    
    print(json.dumps(output))


if __name__ == "__main__":
    main()
