#!/usr/bin/env python3
"""Provider usage / OpenCode fleet snapshot for the Jules keeper job.

Reads only local state and Drew's own provider endpoints. Never prints
credential values. Writes a JSON snapshot the keeper consults when picking
a failover model, and prints a compact human summary.

Snapshot: ~/workspace/goals/keep-the-four-repos-healthy/hidden_files/provider-usage-snapshot.json
Registry: ~/workspace/goals/keep-the-four-repos-healthy/hidden_files/provider-quota-registry.md
"""
import json
import os
import sys
import time
import urllib.request
import urllib.error

HOME = os.path.expanduser("~")
SNAPSHOT = os.path.join(
    HOME,
    "workspace/goals/keep-the-four-repos-healthy/hidden_files/provider-usage-snapshot.json",
)
SERVER = "http://127.0.0.1:4096"


def http(url, headers=None, timeout=20):
    req = urllib.request.Request(url, headers=headers or {})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.load(r)


def server_auth():
    pw_path = os.path.join(HOME, "tools/secrets.d/opencode-server-password")
    try:
        pw = open(pw_path).read().strip()
    except OSError:
        return {}
    import base64

    token = base64.b64encode(f"opencode:{pw}".encode()).decode()
    return {"Authorization": f"Basic {token}"}


def main():
    snap = {"generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}
    auth = server_auth()

    # --- OpenCode server: providers + sessions -------------------------------
    # /api/provider returns provider configs without model lists; the
    # resolved config at /api/config carries info.providers.<id>.models.
    config_models = {}
    try:
        cfg_resp = http(f"{SERVER}/api/config", auth)
        docs = cfg_resp if isinstance(cfg_resp, list) else [cfg_resp]
        for doc in docs:
            info = doc.get("info") if isinstance(doc, dict) else None
            if not isinstance(info, dict):
                continue
            for pid, p in (info.get("providers") or {}).items():
                models = (p or {}).get("models") or {}
                if isinstance(models, dict):
                    config_models[pid] = sorted(models.keys())
    except Exception as e:  # noqa: BLE001
        snap["config_models_error"] = f"{type(e).__name__}"
    try:
        prov = http(f"{SERVER}/api/provider", auth)
        providers = {}
        for p in prov.get("data", []):
            if not isinstance(p, dict):
                continue
            pid = p.get("id")
            providers[pid] = {
                "name": p.get("name"),
                "activation": p.get("activation"),
                "models": config_models.get(pid, []),
            }
        snap["providers"] = providers
        snap["server"] = "up"
    except Exception as e:  # noqa: BLE001
        snap["server"] = f"down: {type(e).__name__}"
        snap["providers"] = {}

    try:
        sess = http(f"{SERVER}/api/session?limit=100", auth)
        rows = sess if isinstance(sess, list) else sess.get("data", [])
        out = []
        for s in rows:
            if not isinstance(s, dict):
                continue
            out.append(
                {
                    "id": s.get("id"),
                    "title": s.get("title"),
                    "model": s.get("model"),
                    "time": s.get("time"),
                    "keys": sorted(s.keys()),
                }
            )
        snap["sessions"] = out
    except Exception as e:  # noqa: BLE001
        snap["sessions_error"] = f"{type(e).__name__}: {e}"

    try:
        active = http(f"{SERVER}/api/session/active", auth)
        snap["active"] = active.get("data", active)
    except Exception as e:  # noqa: BLE001
        snap["active_error"] = f"{type(e).__name__}"

    # --- ChatGPT / Codex plan usage (OAuth token from opencode auth store) ---
    codex = {"view": "chatgpt.com/backend-api/codex/usage"}
    try:
        auth_store = json.load(
            open(os.path.join(HOME, ".local/share/opencode/auth.json"))
        )
        entry = auth_store.get("openai-codex") or auth_store.get("openai") or {}
        access = entry.get("access")
        if not access:
            codex["status"] = "no oauth token in auth store"
        else:
            headers = {"Authorization": f"Bearer {access}"}
            acct = entry.get("accountId") or entry.get("account_id")
            if acct:
                headers["ChatGPT-Account-Id"] = acct
            usage = http(
                "https://chatgpt.com/backend-api/codex/usage", headers, timeout=25
            )
            codex["status"] = "ok"
            codex["usage"] = usage
    except urllib.error.HTTPError as e:
        codex["status"] = f"http {e.code}"
    except Exception as e:  # noqa: BLE001
        codex["status"] = f"{type(e).__name__}"
    snap["codex_usage"] = codex

    os.makedirs(os.path.dirname(SNAPSHOT), exist_ok=True)
    with open(SNAPSHOT, "w") as f:
        json.dump(snap, f, indent=1)

    # --- usage & billing hub snapshot records (design §4, writer (c)) -----
    # Additive only: the JSON snapshot above is unchanged for its existing
    # consumers. The run's observations are ALSO appended as §4 snapshot
    # records to the hub outbox beside the snapshot; the fleet-feed
    # collector drains the outbox into the feed, and the laptop store
    # dedupes on (service, metric, ts, value), so repeats append nothing.
    # Record shape mirrors src/snapshot-records.ts recordsFromProviderUsage
    # in the pm-stack aggregator (developing-today/code).
    try:
        recs = []
        ts = snap["generated_at"]
        if isinstance(snap.get("sessions"), list):
            recs.append(
                {
                    "ts": ts,
                    "serviceId": "opencode-server",
                    "metricId": "sessions-listed",
                    "value": len(snap["sessions"]),
                    "unit": "count",
                    "source": "api",
                    "observedBy": "check-usage",
                }
            )
        rate_limit = ((snap.get("codex_usage") or {}).get("usage") or {}).get(
            "rate_limit"
        ) or {}
        window_names = {604800: "7-day", 18000: "5-hour"}
        for window in (rate_limit.get("primary_window"), rate_limit.get("secondary_window")):
            if not isinstance(window, dict):
                continue
            metric = window_names.get(window.get("limit_window_seconds"))
            if metric and isinstance(window.get("used_percent"), (int, float)):
                recs.append(
                    {
                        "ts": ts,
                        "serviceId": "openai:codex",
                        "metricId": metric,
                        "value": window["used_percent"],
                        "unit": "pct",
                        "limit": 100,
                        "source": "api",
                        "observedBy": "check-usage",
                    }
                )
        if recs:
            outbox = os.path.join(
                HOME,
                "workspace/goals/keep-the-four-repos-healthy/hidden_files/usage-snapshot-outbox.jsonl",
            )
            with open(outbox, "a") as f:
                for r in recs:
                    f.write(json.dumps(r) + "\n")
            print(f"hub outbox: appended {len(recs)} snapshot record(s)")
    except Exception as e:  # noqa: BLE001
        print(f"hub outbox append failed (non-fatal): {type(e).__name__}")

    # --- compact summary ------------------------------------------------------
    print(f"server: {snap['server']}")
    for pid, p in sorted(snap["providers"].items()):
        print(f"provider {pid}: {len(p['models'])} models")
    print(f"sessions listed: {len(snap.get('sessions', []))}")
    print(f"codex usage view: {codex['status']}")
    if codex.get("usage"):
        print("codex usage:", json.dumps(codex["usage"])[:600])


if __name__ == "__main__":
    sys.exit(main())
