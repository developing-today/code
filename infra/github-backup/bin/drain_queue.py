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

Ack discipline (#3219): an event is acked — i.e. deleted from the Worker
queue — only once the drainer has made the mirror reflect it:

- planned repo, fetch attempted: ack iff fetch_repo returned "ok".
  On "fail"/"missing" the event STAYS QUEUED for the next drain tick,
  the failure is appended to fetch.log (same leading format as the
  daily pass), and the coalesce stamp is NOT advanced.
- planned repo inside its coalesce window: the event stays queued
  (not acked); a drain after the window passes fetches and acks it.
- unplanned repo: acked + a fetch.log note (mirror creation is the
  weekly reconcile's decision).
- consecutive drain failures are counted per repo
  (state key drain_fail::<repo>); at CHRONIC_DRAIN_FAILURES the failure
  is logged loudly (DRAIN-CHRONIC) and the run exits non-zero so
  systemd surfaces it — the event is still kept, never silently dropped.
"""
import json, os, sys, time, urllib.request
from datetime import datetime, timezone

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ghb_common import load_config, backup_root
import fetch_mirrors

COALESCE_S = 15 * 60
CHRONIC_DRAIN_FAILURES = 5


def worker_req(path, payload=None):
    url = os.environ["GHB_WORKER_URL"].rstrip("/") + path
    data = json.dumps(payload).encode() if payload is not None else None
    req = urllib.request.Request(url, data=data, method="POST" if data else "GET")
    req.add_header("Authorization", "Bearer " + os.environ["GHB_WORKER_BEARER"])
    # Cloudflare's edge bot-filters the default Python-urllib UA (403);
    # a plain browser UA is required for this poller to reach the Worker.
    req.add_header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
    if data:
        req.add_header("Content-Type", "application/json")
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.loads(r.read() or b"{}")


def event_age_s(ev, now):
    """Age of a queued event in seconds, or None if its ts is unusable.

    The Worker stamps ts with `new Date().toISOString()`; accept epoch
    numbers too so a format change degrades to 'unknown', not a crash.
    """
    ts = ev.get("ts")
    if isinstance(ts, (int, float)):
        return max(0, int(now - ts))
    if isinstance(ts, str):
        try:
            t = datetime.strptime(ts, "%Y-%m-%dT%H:%M:%S.%fZ") \
                .replace(tzinfo=timezone.utc).timestamp()
            return max(0, int(now - t))
        except ValueError:
            try:
                t = datetime.fromisoformat(ts.replace("Z", "+00:00")).timestamp()
                return max(0, int(now - t))
            except ValueError:
                return None
    return None


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
    stamp = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    log_path = os.path.join(root, "fetch.log")
    plan_path = os.path.join(root, "plan.json")
    planned = {p["repo"] for p in json.load(open(plan_path))} \
        if os.path.exists(plan_path) else set()

    def log(line):
        with open(log_path, "a") as f:
            f.write(line + "\n")

    # Group events by repo; events without a repo carry nothing the
    # drainer can act on, so they are acked immediately.
    by_repo, ack = {}, []
    for ev in events:
        repo = ev.get("repo")
        if not repo:
            ack.append(ev["id"])
            continue
        by_repo.setdefault(repo, []).append(ev)

    fetched, failed, unplanned, pending = [], [], [], []
    for repo in sorted(by_repo):
        evs = by_repo[repo]
        ids = [e["id"] for e in evs]
        ages = [a for a in (event_age_s(e, now) for e in evs) if a is not None]
        age_note = f"oldest event age {max(ages)}s" if ages else "event age unknown"
        if repo not in planned:
            log(f"{stamp} {repo} webhook event for unplanned repo — "
                f"left for weekly reconcile")
            ack.extend(ids)
            unplanned.append(repo)
            continue
        key = f"drain_last::{repo}"
        if now - state.get(key, 0) < COALESCE_S:
            # Inside the coalesce window: keep the event queued. A drain
            # after the window passes will fetch (and then ack) it.
            pending.append(repo)
            continue
        res = fetch_mirrors.fetch_repo(root, repo, state)
        if res == "ok":
            state[key] = now
            state.pop(f"drain_fail::{repo}", None)
            ack.extend(ids)
            fetched.append(repo)
            continue
        fkey = f"drain_fail::{repo}"
        fs = state.setdefault(fkey, {"count": 0, "first": now})
        fs["count"] += 1
        fs["last"] = now
        err = (state.get(repo) or {}).get("last_error", "")
        log(f"{stamp} {repo} {res} {err} (drain: {age_note}, "
            f"consecutive drain failures {fs['count']})")
        if fs["count"] >= CHRONIC_DRAIN_FAILURES:
            log(f"{stamp} {repo} DRAIN-CHRONIC {fs['count']} consecutive "
                f"drain failures — event(s) kept queued, needs attention")
        failed.append({"repo": repo, "result": res,
                       "consecutive_failures": fs["count"]})
    with open(state_path, "w") as f:
        json.dump(state, f, indent=0)
    if ack:
        worker_req(cfg["worker"]["queue_poll_path"] + "/ack", {"ids": ack})
    print(json.dumps({"events": len(events), "fetched": fetched,
                      "failed": failed, "unplanned": unplanned,
                      "coalesced_pending": pending, "acked": len(ack)}))
    chronic = [f["repo"] for f in failed
               if f["consecutive_failures"] >= CHRONIC_DRAIN_FAILURES]
    if chronic:
        print(f"drain: chronic failures (events kept queued): {chronic}",
              file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
