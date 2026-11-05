#!/usr/bin/env python3
"""poll.py — read-only candidate discovery for the opencode claimant.

Prints one JSON object:
  {
    "queues": {<queue>: {"claimed": n, "candidates": [...]}, ...},
    "candidates": [ {id, queue, priority, worker_id, lease_until,
                     issue_number} ... ],   # claimed, progress.lane ==
                                              # "opencode", no session_id
    "stuck_bound": [ {id, queue, priority, worker_id, session_id,
                      lease_until, attempts, max_attempts,
                      session_updated, issue_number} ... ],
                                              # bound tasks whose session
                                              # looks never-prompted and
                                              # dead: lease expired, no
                                              # terminal marker, not in the
                                              # server's active set, silent
                                              # >= ACTIVE_WINDOW_MS (#4758).
                                              # Candidates for the prompt.md
                                              # recovery rule ONLY -- the
                                              # claimant must still verify
                                              # zero messages before acting.
    "live_root_sessions": int | null,        # all root sessions;
                                              # null = server unreadable
    "active_root_sessions": int | null,      # roots still active
                                              # (see is_active_root)
    "cap": 36,
    "cap_room": int | null                   # CAP - active roots;
                                              # null when count unknown
  }

Read-only by construction: it issues only queue-broker /list reads
(the broker holds the queue credentials; poll.py presents only the
broker's client token) and GETs to the laptop OpenCode server. The
claimant treats a null cap_room as NO launch room: an unreadable
session list must never become a launch decision.
"""

import json
import os
import re
import sys
import time
import urllib.request
import base64
from datetime import datetime, timezone

QUEUES = [
    "repo-health-code-pilot",
    "repo-health-opencode-pilot",
    "repo-health-openchamber-pilot",
    "repo-health-nix-pilot",
]
CAP = 36
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


UUID_LANE = re.compile(
    r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
)


def is_candidate(row):
    """A claimed task is a claimant candidate iff the opencode execution
    lane was assigned (progress.lane, stamped by assign_repo_task_lane
    since migration 0021), no session is bound yet, and the claiming
    lane is not itself an OpenChamber scheduled lane. OpenChamber lanes
    ARE opencode sessions executing their own claims, so a claimant
    launch for one would double-execute; they are registered in
    agent.lanes under their scheduled-task UUIDs, while Muse shift
    lanes use short ids (code-a, opencode-a, ...) and legacy claims
    carry no lane id at all."""
    prog = row.get("progress") or {}
    if prog.get("lane") != "opencode" or prog.get("session_id"):
        return False
    # The claiming lane must have RECORDED its yield (instruction
    # section 1b: checkpoint stage "handed_off"). Assignment alone is
    # not a handoff: lanes on pre-1b instructions assign and then work
    # the task themselves, and a launch into a task whose lane is
    # mid-work double-executes it (first live batch, 2026-11-02:
    # tasks 5565/5867/6397 were launched while their claiming lanes
    # worked/parked/failed them in parallel). The recorded yield is
    # the only launch-safe signal. Stage "launch_failed" is the other
    # launch-safe yield: the claimant's own recovery rule (#4758)
    # atomically cleared both progress and ledger bindings after
    # verifying/removing the old session, which is exactly the state a
    # relaunch needs.
    if prog.get("stage") not in ("handed_off", "launch_failed"):
        return False
    lane_id = row.get("lane_id")
    if lane_id and UUID_LANE.match(lane_id):
        return False
    return True


def parse_lease_ms(value):
    """ISO-8601 lease_until -> epoch ms, or None when unparseable.

    An unparseable lease is never 'expired': the recovery rule below
    acts on a row only when every fact about it is verified, and a
    lease it cannot read is a fact it does not have."""
    if not isinstance(value, str) or not value:
        return None
    text = value[:-1] + "+00:00" if value.endswith("Z") else value
    try:
        dt = datetime.fromisoformat(text)
    except ValueError:
        return None
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return int(dt.timestamp() * 1000)


def is_stuck_bound(row, session, active_ids, now_ms):
    """True iff a claimed opencode-lane task is bound to a session that
    looks created-but-never-prompted and dead (#4758): the task's lease
    has expired, the bound session is present in the server's list as a
    root with NO terminal marker (no time.idle, no outcome), it is NOT
    in the server's active set, and it has been silent for at least
    ACTIVE_WINDOW_MS (the same 30-minute window the cap counter uses
    for zombies -- one threshold, not two).

    Fail closed on every unknown: a session missing from the list is
    the gone-session case (step 1's relaunch rule owns it, not this
    one), and an unreadable active set (active_ids None) can never
    prove the session is not live. Zero messages is NOT decidable from
    the session list; poll.py only surfaces the candidate and the
    claimant must verify zero messages before acting (prompt.md step
    1). A live, progressing bound session never satisfies this: recent
    activity, active-set membership, or any terminal marker vetoes."""
    prog = row.get("progress") or {}
    if prog.get("lane") != "opencode":
        return False
    session_id = prog.get("session_id")
    if not session_id:
        return False
    lease_ms = parse_lease_ms(row.get("lease_until"))
    if lease_ms is None or lease_ms > now_ms:
        return False
    if session is None or not is_root_session(session):
        return False
    if active_ids is not None and session_id in active_ids:
        return False
    t = session.get("time") or {}
    if t.get("idle") or session.get("outcome"):
        return False
    updated = t.get("updated") or t.get("created") or 0
    return now_ms - updated >= ACTIVE_WINDOW_MS


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


BROKER_URL = os.environ.get("REPO_QUEUE_BROKER_URL", "http://100.99.113.103:4801")
BROKER_TOKEN_FILE = os.environ.get(
    "REPO_QUEUE_TOKEN_FILE",
    os.path.expanduser("~/.config/queue-broker/token"),
)
OPENCODE_URL = os.environ.get("OPENCODE_URL", "http://127.0.0.1:13420")
AUTH_FILE = os.environ.get(
    "OPENCODE_AUTH_FILE",
    os.path.expanduser("~/.local/state/opencode/service-prod.json"),
)


def fetch_claimed(queue):
    """Claimed rows for one queue via the queue-broker's rich /list
    (read-only). Raises on any broker failure: a claimant that cannot
    see the ledger must fail closed, never guess."""
    with open(BROKER_TOKEN_FILE) as f:
        token = f.read().strip()
    body = json.dumps(
        {"queue": queue, "status": "claimed", "limit": 500, "rich": True}
    ).encode()
    req = urllib.request.Request(
        BROKER_URL.rstrip("/") + "/list", data=body, method="POST"
    )
    req.add_header("X-Agent-Token", token)
    req.add_header("Content-Type", "application/json")
    with urllib.request.urlopen(req, timeout=60) as res:
        data = json.loads(res.read() or "null")
    if not isinstance(data, dict) or data.get("ok") is not True:
        raise RuntimeError(f"queue-broker /list failed for {queue}: {data!r:.200}")
    return data.get("rows") or []


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
    now_ms = int(time.time() * 1000)
    # Sessions first: stuck-bound classification (#4758) needs the
    # session list and the server's active set, and the cap summary
    # below reuses the same two reads.
    sessions = fetch_sessions()
    active_ids = fetch_active_ids() if sessions is not None else None
    session_by_id = {}
    if sessions:
        for s in sessions:
            if isinstance(s, dict) and s.get("id"):
                session_by_id[s["id"]] = s
    out = {"queues": {}, "candidates": [], "stuck_bound": []}
    for q in QUEUES:
        rows = fetch_claimed(q)
        cands = []
        stuck = []
        for r in rows:
            prog = r.get("progress") or {}
            if is_candidate(r):
                cands.append(
                    {
                        "id": r["id"],
                        "queue": q,
                        "priority": r.get("priority"),
                        "worker_id": r.get("worker_id"),
                        "lease_until": r.get("lease_until"),
                        "lane_id": r.get("lane_id"),
                        "issue_number": prog.get("issue_number"),
                    }
                )
            session_id = prog.get("session_id")
            if session_id and is_stuck_bound(
                r, session_by_id.get(session_id), active_ids, now_ms
            ):
                sess_t = session_by_id[session_id].get("time") or {}
                stuck.append(
                    {
                        "id": r["id"],
                        "queue": q,
                        "priority": r.get("priority"),
                        "worker_id": r.get("worker_id"),
                        "session_id": session_id,
                        "lease_until": r.get("lease_until"),
                        "attempts": r.get("attempts"),
                        "max_attempts": r.get("max_attempts"),
                        "session_updated": sess_t.get("updated")
                        or sess_t.get("created"),
                        "issue_number": prog.get("issue_number"),
                    }
                )
        out["queues"][q] = {
            "claimed": len(rows),
            "candidates": cands,
            "stuck_bound": stuck,
        }
        out["candidates"].extend(cands)
        out["stuck_bound"].extend(stuck)
    out["candidates"].sort(key=lambda c: (c["priority"] is None, c["priority"] or 0))
    out["stuck_bound"].sort(key=lambda c: (c["priority"] is None, c["priority"] or 0))
    out["cap"] = CAP
    live, active, room = summarize_cap(
        sessions,
        active_ids,
        now_ms,
    )
    out["live_root_sessions"] = live
    out["active_root_sessions"] = active
    out["cap_room"] = room
    print(json.dumps(out, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
