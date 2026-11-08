#!/usr/bin/env python3
"""
OpenCode lane claimant/executor - Slice 1 implementation

This script implements the opencode execution lane claimant/executor for slice 1 of #3026.
It handles:
1. Supervision of bound sessions (checking for stuck/zero-message sessions) - READ-ONLY in shadow mode
2. Candidate discovery from the four opencode queues - READ-ONLY in shadow mode  
3. Session creation and binding with atomic operations - NOT EXECUTED in shadow mode
4. Prompt delivery to worker sessions - NOT EXECUTED in shadow mode
5. Fire logging and reporting - ALWAYS EXECUTED

Mode: shadow (default) or live (read from ~/.local/state/opencode-claimant/mode)

Usage: python3 pm-stack/opencode-claimant/claimant.py

Slice-1 scope: claim/bind/launch/record for opencode-lane tasks.
In SHADOW mode (the default), this script discovers, verifies, and reports
what it WOULD launch, and launches nothing.
"""

import json
import os
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
        f.write(json.dumps(fire_entry, sort_keys=True) + "\n")


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
    """Get the count of active root sessions from the OpenCode server.
    
    Returns -1 (unknown) when the read fails, so cap_step fails closed.
    """
    try:
        # Try to get active root count via poll.py's method
        poll_data = run_poll()
        return poll_data.get("cap_room", -1)
    except Exception as e:
        print(f"Warning: Could not get active session count: {e}")
        return -1  # Unknown


def supervision_step(mode: str) -> List[Dict]:
    """
    Step 1: Supervision - check bound sessions for recovery candidates.
    
    In shadow mode: reports stuck_bound tasks but does not act on them.
    Returns list of tasks that would need recovery in live mode.
    """
    if mode != "live":
        # In shadow mode, just report what supervision would find
        try:
            poll_data = run_poll()
            return poll_data.get("stuck_bound", [])
        except Exception:
            return []
    
    # Live mode supervision logic would go here
    # For slice-1, we only implement shadow mode
    return []


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
    
    In shadow mode: simulates what would be launched without actually launching.
    Returns list of would-be launched sessions.
    """
    if cap_room <= 0:
        return []
    
    would_launch = []
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
            queue = candidate.get("queue", "unknown")
            
            # Simulate session creation
            session_id = f"sim-{task_id[:8]}-{int(time.time())}"
            
            # Record what would be launched
            would_launch.append({
                "task_id": task_id,
                "session_id": session_id,
                "worker_id": worker_id,
                "queue": queue,
                "issue": issue,
                "packet": packet,
                "timestamp": datetime.utcnow().isoformat() + "Z"
            })
            
            cap_room -= 1
            
    except Exception as e:
        print(f"Launch simulation failed: {e}")
    
    return would_launch


def main():
    """Main entry point for the claimant/executor."""
    mode = read_mode()
    
    print(f"OpenCode claimant starting in {mode} mode")
    
    # Step 1: Supervision (always runs in live mode, reports in shadow)
    print("Running supervision step...")
    supervision_candidates = supervision_step(mode)
    if supervision_candidates:
        print(f"  Found {len(supervision_candidates)} stuck_bound tasks that would be recovered in live mode")
    
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
    
    # Step 4: Launch (simulation in shadow mode)
    print("Running launch step (simulation in shadow mode)...")
    would_launch = launch_step(mode, cap_room)
    
    # Step 6: Log fire
    print(f"Fire complete. Would launch {len(would_launch)} sessions in live mode.")
    
    # Prepare fire log entry
    candidates_seen = []
    try:
        poll_data = run_poll()
        candidates_seen = poll_data.get("candidates", [])
    except Exception:
        pass
    
    log_fire(mode, candidates_seen, would_launch, cap_room, [])
    
    return 0


if __name__ == "__main__":
    sys.exit(main())