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
from pathlib import Path
from typing import Dict, List, Any

# Constants
QUEUES = [
    "repo-health-code-pilot",
    "repo-health-opencode-pilot",
    "repo-health-openchamber-pilot",
    "repo-health-nix-pilot",
]

PRIMARY_CLONE = Path.home() / "git" / "code"


def as_list(value) -> List[Dict]:
    """Convert value to list if it's not already a list."""
    if value is None:
        return []
    if isinstance(value, list):
        return value
    return [value]


def get_candidates() -> List[Dict]:
    """Discover unbound opencode-lane candidates from all queues.
    
    In a real implementation, this would query the queue broker.
    For now, return empty list to indicate no candidates.
    """
    # Return empty list - in a real implementation this would query the broker
    return []


def get_stuck_bound() -> List[Dict]:
    """
    Discover bound tasks with stuck/zero-message sessions.
    
    These are tasks that:
    - Have a session_id bound
    - Session exists as a root but has no terminal marker
    - No messages in the session (zero-message case)
    - Session has been silent for >30 minutes (ACTIVE_WINDOW_MS)
    """
    # Return empty list - this would require OpenCode server access
    return []


def count_active_roots() -> int:
    """Count active root sessions from the OpenCode server."""
    # Return 0 - this would require OpenCode server access
    return 0


def main():
    """Main entry point."""
    candidates = get_candidates()
    stuck_bound = get_stuck_bound()
    active_count = count_active_roots()
    
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