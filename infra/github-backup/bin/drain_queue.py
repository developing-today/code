#!/usr/bin/env python3
"""drain_queue.py — pull queued webhook events from the Worker and fetch.

The Cloudflare Worker (../worker) receives GitHub App webhooks, validates
the HMAC, and stores events in KV. This drainer (timer: every 5 min):
  GET  <GHB_WORKER_URL>/queue   (Authorization: Bearer <GHB_WORKER_BEARER>)
  ->   {"events": [{"id", "repo", "event", "ts"}]}
  POST <GHB_WORKER_URL>/queue/ack {"ids": [...]}
Per-repo coalescing: a repo is fetched at most once per 15 min
(state.json key drain_last::<repo>) — hatch-personal pushes every ~2 min.
`repository` events with action=created are noted for the weekly sweep
(which owns mirror creation decisions) and also trigger an immediate
clone attempt through the normal plan if the repo is already planned.
Secrets come from the environment; the systemd unit sources them from
Bitwarden by name (see config.json app.bitwarden_secret_names).
"""
import json, os, sys, time, urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ghb_common import load_config, backup_root
import fetch_mirrors

COALESCE_S = 15 * 60


def worker_req(path, payload=None):
    url = os.environ["GHB_WORKER_URL"].rstrip("/") + path
    data = json.dumps(payload).encode() if payload is not None else None
    req = urllib.request.Request(url, data=data, method="POST" if data else "GET")
    req.add_header("Authorization", "Bearer " + os.environ["GHB_WORKER_BEARER"])
    if data:
        req.add_header("Content-Type", "application/json")
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.loads(r.read() or b"{}")


def main():
    cfg = load_config()
    root = backup_root(cfg)
    state_path = os.path.join(root, "state.json")
    state = json.load(open(state_path)) if os.path.exists(state_path) else {}
    q = worker_req(cfg["worker"]["queue_poll_path"])
    events = q.get("events", [])
    if not events:
        print("queue empty")
        return 0
    now = time.time()
    to_fetch, ack = [], []
    for ev in events:
        repo = ev.get("repo")
        ack.append(ev["id"])
        if not repo:
            continue
        key = f"drain_last::{repo}"
        if now - state.get(key, 0) < COALESCE_S:
            continue
        state[key] = now
        if repo not in to_fetch:
            to_fetch.append(repo)
    fetched = []
    for repo in to_fetch:
        plan_path = os.path.join(root, "plan.json")
        planned = {p["repo"] for p in json.load(open(plan_path))} if os.path.exists(plan_path) else set()
        if repo in planned:
            fetch_mirrors.fetch_repo(root, repo, state)
            fetched.append(repo)
        else:
            with open(os.path.join(root, "fetch.log"), "a") as f:
                f.write(f"{time.strftime('%Y-%m-%dT%H:%M:%S%z')} {repo} "
                        f"webhook event for unplanned repo — left for weekly reconcile\n")
    with open(state_path, "w") as f:
        json.dump(state, f, indent=0)
    worker_req(cfg["worker"]["queue_poll_path"] + "/ack", {"ids": ack})
    print(json.dumps({"events": len(events), "fetched": fetched}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
