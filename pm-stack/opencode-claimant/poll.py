#!/usr/bin/env python3
"""poll.py — read-only candidate discovery for the opencode claimant.

Prints one JSON object:
  {
    "queues": {<queue>: {"claimed": n, "candidates": [...]}, ...},
    "candidates": [ {id, queue, priority, worker_id, lease_until,
                     issue_number} ... ],   # claimed, progress.lane ==
                                              # "opencode", no session_id
    "live_root_sessions": int | null,        # null = server unreadable
    "cap": 12,
    "cap_room": int | null                   # null when count unknown
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
import urllib.request
import base64

QUEUES = [
    "repo-health-code-pilot",
    "repo-health-opencode-pilot",
    "repo-health-openchamber-pilot",
    "repo-health-nix-pilot",
]
CAP = 12
HELPER = os.environ.get(
    "REPO_QUEUE_HELPER", os.path.expanduser("~/hatch/tools/repo-queue.py"))
OPENCODE_URL = os.environ.get("OPENCODE_URL", "http://127.0.0.1:13420")
AUTH_FILE = os.environ.get(
    "OPENCODE_AUTH_FILE",
    os.path.expanduser("~/.local/state/opencode/service-prod.json"))


def load_helper():
    spec = importlib.util.spec_from_file_location("repo_queue", HELPER)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def live_root_sessions():
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
    roots = [s for s in data
             if isinstance(s, dict) and not s.get("parentID")
             and not s.get("parent_id")]
    return len(roots)


def main():
    helper = load_helper()
    key = helper.get_service_key()
    out = {"queues": {}, "candidates": []}
    for q in QUEUES:
        rows = helper.http(
            "GET",
            f"tasks?queue=eq.{q}&status=eq.claimed"
            "&select=id,queue,status,worker_id,priority,progress,lease_until"
            "&limit=500",
            key) or []
        cands = []
        for r in rows:
            prog = r.get("progress") or {}
            if prog.get("lane") != "opencode" or prog.get("session_id"):
                continue
            cands.append({
                "id": r["id"], "queue": q,
                "priority": r.get("priority"),
                "worker_id": r.get("worker_id"),
                "lease_until": r.get("lease_until"),
                "issue_number": prog.get("issue_number"),
            })
        out["queues"][q] = {"claimed": len(rows), "candidates": cands}
        out["candidates"].extend(cands)
    out["candidates"].sort(key=lambda c: (c["priority"] is None,
                                          c["priority"] or 0))
    count = live_root_sessions()
    out["live_root_sessions"] = count
    out["cap"] = CAP
    out["cap_room"] = None if count is None else max(0, CAP - count)
    print(json.dumps(out, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
