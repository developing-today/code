#!/usr/bin/env python3
"""poll.py — read-only candidate discovery for the opencode claimant.

Prints one JSON object:
  {
    "queues": {<queue>: {"claimed": n, "candidates": [...]}, ...},
    "candidates": [ {id, queue, priority, worker_id, lease_until,
                     issue_number} ... ],   # claimed, progress.lane ==
                                              # "opencode", no session_id
    "live_root_sessions": int | null,        # all root sessions;
                                              # null = server unreadable
    "active_root_sessions": int | null,      # roots still active
                                              # (see is_active_root)
    "cap": 12,
    "cap_room": int | null                   # CAP - active roots;
                                              # null when count unknown
  }

Read-only by construction: it issues only PostgREST GETs (through the
queue helper module, which fetches the service-role key transiently —
the value is never printed or stored) and one GET to the laptop
OpenCode server. The claimant treats a null cap_room as NO launch room:
an unreadable session list must never become a launch decision.
"""

import importlib.util
import json
import os
import sys
import time
import urllib.request
import base64

QUEUES = [
    "repo-health-code-pilot",
    "repo-health-opencode-pilot",
    "repo-health-openchamber-pilot",
    "repo-health-nix-pilot",
]
CAP = 12
# A root session counts toward the cap only while it is active: no
# terminal marker and recent activity. GET /api/session returns every
# finished-but-unarchived session forever (50 roots at the #3026 gate
# check, 46 of them finished), so counting all roots pins cap_room at
# 0 and deadlocks the claimant. A finished session carries time.idle
# and an outcome; a running one carries neither. A marker-less session
# silent longer than this window is indistinguishable from a zombie
# whose client died mid-run, so it stops counting too — the claimant
# fires every 10 min and a working session updates far more often.
ACTIVE_WINDOW_MS = 30 * 60 * 1000


def is_root_session(s):
    return isinstance(s, dict) and not s.get("parentID") and not s.get("parent_id")


def is_active_root(s, now_ms, active_ids=None):
    if not is_root_session(s):
        return False
    # The server's active set is authoritative: a RESUMED session keeps
    # its stored outcome/idle from the earlier run while it is mid-run
    # again (the aggregator's mapper overrides stored outcomes the same
    # way, pm-stack/aggregator/src/sources/opencode.ts). Membership
    # counts the root no matter what its stored markers say.
    if active_ids is not None and s.get("id") in active_ids:
        return True
    t = s.get("time") or {}
    if t.get("idle") or s.get("outcome"):
        return False
    updated = t.get("updated") or t.get("created") or 0
    return now_ms - updated < ACTIVE_WINDOW_MS


def count_active_roots(sessions, now_ms, active_ids=None):
    return sum(1 for s in sessions if is_active_root(s, now_ms, active_ids))


def summarize_cap(sessions, active_ids, now_ms):
    """(live_root_sessions, active_root_sessions, cap_room).

    Either server read failing makes the active count UNKNOWN, never
    zero: a resumed root's stored markers read as finished, so falling
    back to markers alone when /api/session/active is unreadable could
    report cap_room 12 over 12 running sessions. Unknown propagates as
    None and the claimant launches nothing — the same fail-closed rule
    the session list itself has always had.
    """
    if sessions is None:
        return None, None, None
    roots = [s for s in sessions if is_root_session(s)]
    if active_ids is None:
        return len(roots), None, None
    active = count_active_roots(roots, now_ms, active_ids)
    return len(roots), active, max(0, CAP - active)


HELPER = os.environ.get(
    "REPO_QUEUE_HELPER", os.path.expanduser("~/hatch/tools/repo-queue.py")
)
OPENCODE_URL = os.environ.get("OPENCODE_URL", "http://127.0.0.1:13420")
AUTH_FILE = os.environ.get(
    "OPENCODE_AUTH_FILE",
    os.path.expanduser("~/.local/state/opencode/service-prod.json"),
)


def load_helper():
    spec = importlib.util.spec_from_file_location("repo_queue", HELPER)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def fetch_active_ids():
    """Session ids the server currently holds active (GET
    /api/session/active), or None when unreadable. The response's data
    is a map keyed by session id (a list shape is accepted too)."""
    try:
        with open(AUTH_FILE) as f:
            password = json.load(f)["password"]
    except Exception:
        return None
    req = urllib.request.Request(OPENCODE_URL.rstrip("/") + "/api/session/active")
    token = base64.b64encode(f"opencode:{password}".encode()).decode()
    req.add_header("Authorization", f"Basic {token}")
    try:
        with urllib.request.urlopen(req, timeout=15) as res:
            data = json.loads(res.read() or "null")
    except Exception:
        return None
    if isinstance(data, dict) and "data" in data:
        data = data["data"]
    if isinstance(data, dict):
        return set(data.keys())
    if isinstance(data, list):
        return {x for x in data if isinstance(x, str)}
    return None


def fetch_sessions():
    try:
        with open(AUTH_FILE) as f:
            password = json.load(f)["password"]
    except Exception:
        return None
    req = urllib.request.Request(OPENCODE_URL.rstrip("/") + "/api/session")
    token = base64.b64encode(f"opencode:{password}".encode()).decode()
    req.add_header("Authorization", f"Basic {token}")
    try:
        with urllib.request.urlopen(req, timeout=15) as res:
            data = json.loads(res.read() or "null")
    except Exception:
        return None
    if isinstance(data, dict) and "data" in data:
        data = data["data"]
    if not isinstance(data, list):
        return None
    return data


def main():
    helper = load_helper()
    key = helper.get_service_key()
    out = {"queues": {}, "candidates": []}
    for q in QUEUES:
        rows = (
            helper.http(
                "GET",
                f"tasks?queue=eq.{q}&status=eq.claimed"
                "&select=id,queue,status,worker_id,priority,progress,lease_until"
                "&limit=500",
                key,
            )
            or []
        )
        cands = []
        for r in rows:
            prog = r.get("progress") or {}
            if prog.get("lane") != "opencode" or prog.get("session_id"):
                continue
            cands.append(
                {
                    "id": r["id"],
                    "queue": q,
                    "priority": r.get("priority"),
                    "worker_id": r.get("worker_id"),
                    "lease_until": r.get("lease_until"),
                    "issue_number": prog.get("issue_number"),
                }
            )
        out["queues"][q] = {"claimed": len(rows), "candidates": cands}
        out["candidates"].extend(cands)
    out["candidates"].sort(key=lambda c: (c["priority"] is None, c["priority"] or 0))
    sessions = fetch_sessions()
    out["cap"] = CAP
    live, active, room = summarize_cap(
        sessions,
        fetch_active_ids() if sessions is not None else None,
        int(time.time() * 1000),
    )
    out["live_root_sessions"] = live
    out["active_root_sessions"] = active
    out["cap_room"] = room
    print(json.dumps(out, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
