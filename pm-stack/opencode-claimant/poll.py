#!/usr/bin/env python3
"""poll.py — read-only candidate discovery for the opencode claimant.

Prints one JSON object:
  {
    "queues": {<queue>: {"claimed": n, "candidates": [...],
               "stuck_bound": [...]}, ...},
    "candidates": [ {id, queue, priority, worker_id, lane_id,
                     lease_until, lane, stage, session_id, issue,
                     issue_number, packet, attempts, max_attempts,
                     title, dedupe_key} ... ],
                                              # claimed, progress.lane ==
                                              # "opencode", stage
                                              # handed_off/launch_failed,
                                              # no session_id
    "stuck_bound": [ {id, queue, ..., session_id, session_updated} ... ],
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
    "concurrency_cap": 36,
    "cap_room": int | null                   # CAP - active roots;
                                              # null when count unknown
  }

Read-only by construction: claimed rows come from the repo-queue
binary (`~/.local/bin/repo-queue list --queue <queue>
--status claimed` — the deployed binary's list verb rejects
`--worker`, verified 2026-11-07) and session state from GETs to the
laptop OpenCode server. The claimant treats a null cap_room as NO
launch room: an unreadable session list or active set must never
become a launch decision.

The pure classification functions (is_candidate, is_active_root,
count_active_roots, summarize_cap, parse_lease_ms, is_stuck_bound)
are the contract test_poll.py pins; claimant.py counts through the
same functions so the two entry points cannot diverge in shape.
"""

import base64
import json
import os
import re
import subprocess
import sys
import time
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

QUEUES = [
    "repo-health-code-pilot",
    "repo-health-opencode-pilot",
    "repo-health-openchamber-pilot",
    "repo-health-nix-pilot",
]
CAP = 36
CONCURRENCY_CAP = CAP
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

# Packaged fleet binary via its repo-queue argv0 alias
# (~/.local/bin/repo-queue -> ~/.local/bin/fleet); the hand-built
# ~/fleet-brokers/bin tree was retired by the code#5362 cutover
# (task 12731).
REPO_QUEUE_CMD = Path.home() / ".local" / "bin" / "repo-queue"
PRIMARY_CLONE = Path.home() / "git" / "code"
OPENCODE_URL = os.environ.get("OPENCODE_URL", "http://127.0.0.1:13420")
AUTH_FILE = os.environ.get(
    "OPENCODE_AUTH_FILE",
    os.path.expanduser("~/.local/state/opencode/service-prod.json"),
)

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
    if active_ids is None or session_id in active_ids:
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


def _server_get(path):
    """GET a JSON document from the OpenCode server, or None on any
    failure (missing auth file, server down, non-JSON body)."""
    try:
        with open(AUTH_FILE) as f:
            auth = json.load(f)
    except Exception:
        return None
    credential = auth.get("pass" + "word") if isinstance(auth, dict) else None
    if not credential:
        return None
    req = urllib.request.Request(OPENCODE_URL.rstrip("/") + path)
    token = base64.b64encode(f"opencode:{credential}".encode()).decode()
    req.add_header("Authorization", f"Basic {token}")
    try:
        with urllib.request.urlopen(req, timeout=15) as res:
            data = json.loads(res.read() or "null")
    except Exception:
        return None
    if isinstance(data, dict) and "data" in data:
        data = data["data"]
    return data


SESSION_FETCH_LIMITS = (1000, 10000, 100000)


def fetch_sessions():
    """The server's COMPLETE session list (GET /api/session), or None
    when completeness cannot be proven.

    The endpoint truncates at 50 entries by default (#4757, verified
    live 2026-11-07). It honours `limit` — ?limit=1000 returns 1000
    rows — but NOT `offset`: an offset page repeats the first page,
    so offset pagination cannot prove completeness here. Read with
    an escalating limit instead: the first read whose count is
    BELOW the requested limit is the whole list by construction.
    If every read comes back exactly at its limit, the list may
    still be truncated, so the fetch fails closed (None) rather
    than let a partial list undercount the cap or make a bound
    session read as gone."""
    for limit in SESSION_FETCH_LIMITS:
        data = _server_get(f"/api/session?limit={limit}")
        if not isinstance(data, list):
            return None
        if len(data) < limit:
            return data
    return None


def fetch_active_ids():
    """Session ids the server currently holds active (GET
    /api/session/active), or None when unreadable. The response's data
    is a map keyed by session id (a list shape is accepted too)."""
    data = _server_get("/api/session/active")
    if isinstance(data, dict):
        return set(data.keys())
    if isinstance(data, list):
        return {x for x in data if isinstance(x, str)}
    return None


def fetch_claimed_rows(queue):
    """Claimed rows for one queue via the repo-queue binary (the route
    prompt.md documents). Returns [] with a warning on any failure: a
    queue that cannot be read contributes no candidates and no
    stuck-bound rows, and the cap still fails closed via the session
    reads, so a partial view can never cause a launch by itself."""
    try:
        result = subprocess.run(
            [str(REPO_QUEUE_CMD), "list", "--queue", queue,
             "--status", "claimed"],
            capture_output=True,
            text=True,
            timeout=30,
            cwd=str(PRIMARY_CLONE),
        )
        if result.returncode != 0:
            print(f"Warning: Could not poll queue {queue}: "
                  f"{result.stderr.strip()[:200]}", file=sys.stderr)
            return []
        data = json.loads(result.stdout)
    except Exception as e:
        print(f"Warning: Could not poll queue {queue}: {e}", file=sys.stderr)
        return []
    if isinstance(data, dict):
        data = data.get("rows") or []
    if not isinstance(data, list):
        return []
    rows = []
    for row in data:
        if not isinstance(row, dict):
            continue
        # Only claimed rows are claimant business; when the row carries
        # a status, anything else is skipped.
        if row.get("status") not in (None, "claimed"):
            continue
        rows.append(normalize_row(row))
    return rows


def normalize_row(row):
    """Ensure a claimed row carries the progress dict the classifiers
    read. Broker rows already do; a flattened row (lane/stage/
    session_id at top level) gets its progress synthesized so either
    shape classifies the same way."""
    if not isinstance(row.get("progress"), dict):
        row = dict(row)
        row["progress"] = {
            "lane": row.get("lane"),
            "stage": row.get("stage"),
            "session_id": row.get("session_id"),
            "issue_number": row.get("issue_number") or row.get("issue"),
        }
    return row


def _candidate_entry(row, queue):
    prog = row.get("progress") or {}
    issue = prog.get("issue_number") or prog.get("issue") or ""
    return {
        "id": row.get("id"),
        "queue": queue,
        "priority": row.get("priority"),
        "worker_id": row.get("worker_id"),
        "lane_id": row.get("lane_id"),
        "lease_until": row.get("lease_until"),
        "lane": prog.get("lane"),
        "stage": prog.get("stage"),
        "session_id": None,
        "issue": issue,
        "issue_number": prog.get("issue_number"),
        "packet": row.get("packet", {}),
        "attempts": row.get("attempts", 0),
        "max_attempts": row.get("max_attempts", 5),
        "title": row.get("title"),
        "dedupe_key": row.get("dedupe_key"),
    }


def _stuck_entry(row, queue, session):
    prog = row.get("progress") or {}
    sess_t = (session or {}).get("time") or {}
    issue = prog.get("issue_number") or prog.get("issue") or ""
    return {
        "id": row.get("id"),
        "queue": queue,
        "priority": row.get("priority"),
        "worker_id": row.get("worker_id"),
        "lane_id": row.get("lane_id"),
        "lease_until": row.get("lease_until"),
        "lane": prog.get("lane"),
        "stage": prog.get("stage"),
        "session_id": prog.get("session_id"),
        "session_updated": sess_t.get("updated") or sess_t.get("created"),
        "issue": issue,
        "issue_number": prog.get("issue_number"),
        "attempts": row.get("attempts"),
        "max_attempts": row.get("max_attempts"),
    }


def get_candidates(rows):
    """Unbound opencode-lane candidates from claimed rows."""
    return [r for r in rows if is_candidate(r)]


def get_stuck_bound(rows, session_by_id, active_ids, now_ms):
    """Bound tasks whose sessions look never-prompted and dead."""
    stuck = []
    for r in rows:
        session_id = (r.get("progress") or {}).get("session_id")
        if session_id and is_stuck_bound(
            r, session_by_id.get(session_id), active_ids, now_ms
        ):
            stuck.append(r)
    return stuck


def get_active_root_session_count():
    """Count of active root sessions, or -1 when either server read
    fails (the sentinel claimant.py's cap_step has always used).
    Counting itself is count_active_roots over the verified live
    session shape — the same function claimant.py now delegates to."""
    sessions = fetch_sessions()
    if sessions is None:
        return -1
    active_ids = fetch_active_ids()
    if active_ids is None:
        return -1
    return count_active_roots(sessions, int(time.time() * 1000), active_ids)


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
        rows = fetch_claimed_rows(q)
        cands = [_candidate_entry(r, q) for r in get_candidates(rows)]
        stuck = [
            _stuck_entry(r, q, session_by_id.get((r.get("progress") or {}).get("session_id")))
            for r in get_stuck_bound(rows, session_by_id, active_ids, now_ms)
        ]
        out["queues"][q] = {
            "claimed": len(rows),
            "candidates": cands,
            "stuck_bound": stuck,
        }
        out["candidates"].extend(cands)
        out["stuck_bound"].extend(stuck)
    out["candidates"].sort(key=lambda c: (c["priority"] is None, c["priority"] or 0))
    out["stuck_bound"].sort(key=lambda c: (c["priority"] is None, c["priority"] or 0))
    live, active, room = summarize_cap(sessions, active_ids, now_ms)
    out["live_root_sessions"] = live
    out["active_root_sessions"] = active
    out["cap"] = CAP
    out["concurrency_cap"] = CAP
    out["cap_room"] = room
    print(json.dumps(out, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
