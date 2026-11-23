#!/usr/bin/env python3
"""Quota recorder (Drew's directive 2026-10-28; DB landing added
2026-10-28 at Drew's direction): every tick, append a timestamped record
of Muse's own subscription quota, read via the premade
`subscription-status` CLI (plan, weekly percent used, reset, additional
token pool). Value or error — whatever the source returns is recorded.

The fleet providers' quotas are already recorded 24/7 on the laptop by the
PM quota-gatherer (every 15 min: aggregator quota-current + the monthly
usage-snapshots JSONL), so this recorder covers the one pool that had no
recording: the assistant's own subscription.

Two sinks, file first:
1. FILE (primary, always): one JSON object per tick appended to
     ~/workspace/goals/stewardship-commission-recurring-review-and-dispatch/
       hidden_files/visibility/quota-records-YYYY-MM.jsonl
   The file is append-only and is the source of truth.
2. TURSO (secondary; was Supabase agent.quota_records until the
   2026-11-06 cutover, Drew's order — Supabase→Turso migration Wave 3,
   code#5690): the same records are inserted into `agent_quota_records`
   in the Turso DB `fleet-platform` via the libsql HTTP pipeline,
   PK (source, id, ts) — id is the record's pool id for per-pool sources
   ("" for muse-subscription) — INSERT OR IGNORE, so re-sends are
   harmless. Every record is born UNSENT; a record counts as sent only
   when an ack line in the same JSONL covers its ts. After appending its
   tick, the recorder flushes ALL unsent records in the current and
   previous month files (this tick's plus any backlog) in chunked
   inserts per file, then appends a receipt line to that file:
     success: {"kind": "turso-ack",   "ts": ..., "through": <max record
               ts landed>, "count": <records landed by this flush>}
     failure: {"kind": "turso-error", "ts": ..., "error": <what failed>,
               "pending": <records still unsent in that file>}
   Receipt lines written before the cutover carry kind "supabase-ack" /
   "supabase-error"; the parser accepts both (ack history stays valid).
   Unsent state is derivable from the file alone: records with a ts later
   than the last ack line's "through" (or all records, if no ack exists)
   are pending. Failures never lose data — the next tick retries
   everything un-acked. The flush gets its egress proxy from
   ~/tools/secrets.d/mcpx-daemon.env when the caller's env has none
   (the systemd unit's minimal env has no proxy; same pattern as the
   fleet-feed collector). The DB token is fetched by name only
   (Bitwarden `turso.fleet-platform-rw-token`, via the fleet secrets
   verb) at flush time and never stored.

No LLM involved. Always exits 0; failures are recorded as data.
"""
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime, timezone

OUT_DIR = os.path.expanduser(
    "~/workspace/goals/stewardship-commission-recurring-review-and-dispatch"
    "/hidden_files/visibility"
)
SUBSCRIPTION_STATUS = "/opt/hatch/bin/subscription-status"
TURSO_URL = "https://fleet-platform-dezren39.aws-us-east-1.turso.io"
TURSO_TOKEN_SECRET = "turso.fleet-platform-rw-token"
TABLE = "agent_quota_records"
def _resolve_fleet():
    """First working fleet binary on this host: the layout differs per
    host (sandbox: ~/tools/bin/fleet; laptop: ~/.local/bin/fleet), so a
    single hard-coded path silently breaks the other host (#6830)."""
    for cand in ("~/tools/bin/fleet", "~/.local/bin/fleet"):
        p = os.path.expanduser(cand)
        if os.path.isfile(p) and os.access(p, os.X_OK):
            return p
    return shutil.which("fleet") or os.path.expanduser("~/tools/bin/fleet")


FLEET = _resolve_fleet()
CHUNK = 200


def record_subscription(ts):
    rec = {"ts": ts, "source": "muse-subscription"}
    try:
        p = subprocess.run(
            [SUBSCRIPTION_STATUS, "status"],
            capture_output=True, text=True, timeout=60,
        )
        text = (p.stdout or "") + (p.stderr or "")
        m = re.search(r"on the (\w+) subscription", text)
        if m:
            rec["plan"] = m.group(1)
        m = re.search(r"Usage:\s*([\d.]+)% of weekly limit", text)
        if m:
            rec["weekly_percent"] = float(m.group(1))
        m = re.search(r"Weekly limit resets (.+)", text)
        if m:
            rec["resets"] = m.group(1).strip()
        m = re.search(
            r"Additional tokens:\s*([\d.]+)% used \(([^)]+)\)", text
        )
        if m:
            rec["additional_percent"] = float(m.group(1))
            rec["additional_left"] = m.group(2).strip()
        if "weekly_percent" not in rec:
            rec["error"] = "unparsed subscription-status output"
            rec["raw"] = text.strip()[:400]
    except Exception as e:  # failure is data, per Drew
        rec["error"] = f"{type(e).__name__}: {e}"
    return rec


# --- Turso landing --------------------------------------------------------

def _sql_str(v):
    if v is None:
        return "NULL"
    return "'" + str(v).replace("'", "''") + "'"


def _sql_num(v):
    if isinstance(v, (int, float)) and not isinstance(v, bool):
        return repr(float(v))
    return "NULL"


def build_insert(records, inserted_at):
    rows = []
    for r in records:
        rows.append(
            "("
            + ", ".join([
                _sql_str(r.get("source")),
                _sql_str(r.get("ts")),
                _sql_str(r.get("plan")),
                _sql_num(r.get("weekly_percent")),
                _sql_str(r.get("resets")),
                _sql_num(r.get("additional_percent")),
                _sql_str(r.get("additional_left")),
                _sql_str(r.get("error")),
                _sql_str(json.dumps(r)),
                _sql_str(inserted_at),
                _sql_str(r.get("id") or ""),
            ])
            + ")"
        )
    return (
        f"INSERT OR IGNORE INTO {TABLE} (source, ts, plan, weekly_percent,"
        " resets, additional_percent, additional_left, error, payload,"
        " inserted_at, id) VALUES\n"
        + ",\n".join(rows)
        + ";"
    )


def load_file_state(path):
    """Return (records, acked_through) parsed from one JSONL file.

    Records are lines without a "kind" field; receipt lines carry one.
    acked_through is the "through" of the file's last ack line
    ("turso-ack", or the pre-cutover "supabase-ack"), or None when no
    flush has ever landed for this file.
    """
    records = []
    acked_through = None
    with open(path) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
            except json.JSONDecodeError:
                continue
            if obj.get("kind") in ("turso-ack", "supabase-ack"):
                acked_through = obj.get("through", acked_through)
            elif "kind" not in obj and "ts" in obj:
                records.append(obj)
    return records, acked_through


PROXY_ENV_FILE = os.path.expanduser("~/tools/secrets.d/mcpx-daemon.env")
PROXY_KEYS = (
    "HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy",
    "ALL_PROXY", "all_proxy", "NO_PROXY", "no_proxy",
)


def flush_env():
    """Env for the flush subprocesses. Under the systemd unit's minimal
    env there is no egress proxy, and both hops of the flush (fleet
    secrets broker -> Bitwarden, curl -> libsql endpoint) need one;
    mirror the fleet-feed collector's pattern: when no proxy is set,
    load the proxy values from ~/tools/secrets.d/mcpx-daemon.env into
    the child's env only (memory only — never argv, never a file)."""
    env = dict(os.environ)
    if not (env.get("HTTPS_PROXY") or env.get("https_proxy")):
        try:
            with open(PROXY_ENV_FILE) as f:
                for line in f:
                    k, sep, v = line.strip().partition("=")
                    if sep and k in PROXY_KEYS and k not in env:
                        env[k] = v.strip().strip('"').strip("'")
        except OSError:
            pass
    return env


def get_turso_token(env):
    p = subprocess.run(
        [FLEET, "secrets", "get", TURSO_TOKEN_SECRET],
        capture_output=True, text=True, timeout=60, env=env,
    )
    val = (p.stdout or "").strip()
    if not val:
        raise RuntimeError(f"empty token from fleet secrets ({TURSO_TOKEN_SECRET})")
    return val


def run_flush(records, now_ts):
    """Insert one chunk of records into Turso via the libsql HTTP
    pipeline. Returns (ok, error_text)."""
    env = flush_env()
    try:
        token = get_turso_token(env)
    except Exception as e:
        return False, f"token: {type(e).__name__}: {e}"[:300]
    body = {"requests": [
        {"type": "execute",
         "stmt": {"sql": build_insert(records, now_ts)}},
        {"type": "close"},
    ]}
    fd, fname = tempfile.mkstemp(suffix=".json", prefix="quota-flush-")
    try:
        with os.fdopen(fd, "w") as f:
            json.dump(body, f)
        p = subprocess.run(
            ["curl", "-sS", "-m", "120", "-X", "POST",
             f"{TURSO_URL}/v2/pipeline",
             "-H", f"Authorization: Bearer {token}",
             "-H", "Content-Type: application/json",
             "--data-binary", f"@{fname}"],
            capture_output=True, text=True, timeout=180, env=env,
        )
        if p.returncode != 0:
            err = (p.stderr or p.stdout or "").strip().splitlines()
            return False, f"curl exit {p.returncode}: {err[-1] if err else ''}"[:300]
        try:
            out = json.loads(p.stdout)
        except json.JSONDecodeError:
            return False, f"unparseable pipeline response: {p.stdout[:120]}"
        for res in out.get("results", []):
            if res.get("type") == "error":
                return False, f"turso: {res.get('error', {}).get('message', res)}"[:300]
        return True, ""
    except Exception as e:
        return False, f"{type(e).__name__}: {e}"[:300]
    finally:
        try:
            os.unlink(fname)
        except OSError:
            pass


def flush_file(path, now_ts):
    """Flush every unsent record in one JSONL file; append the receipt.

    Returns the number of records still pending after this attempt."""
    if not os.path.exists(path):
        return 0
    records, acked_through = load_file_state(path)
    pending = [
        r for r in records
        if acked_through is None or r["ts"] > acked_through
    ]
    if not pending:
        return 0
    landed_through = acked_through
    landed = 0
    for i in range(0, len(pending), CHUNK):
        chunk = pending[i:i + CHUNK]
        ok, err = run_flush(chunk, now_ts)
        if not ok:
            with open(path, "a") as f:
                f.write(json.dumps({
                    "kind": "turso-error",
                    "ts": now_ts,
                    "error": err,
                    "pending": len(pending) - landed,
                }) + "\n")
            return len(pending) - landed
        landed += len(chunk)
        landed_through = chunk[-1]["ts"]
    with open(path, "a") as f:
        f.write(json.dumps({
            "kind": "turso-ack",
            "ts": now_ts,
            "through": landed_through,
            "count": landed,
        }) + "\n")
    return 0


def month_paths(now):
    """Current month's JSONL plus last month's (a month boundary must not
    strand un-acked records in the old file)."""
    paths = []
    y, m = now.year, now.month
    for _ in range(2):
        paths.append(os.path.join(
            OUT_DIR, f"quota-records-{y:04d}-{m:02d}.jsonl"))
        m -= 1
        if m == 0:
            y, m = y - 1, 12
    return paths


def main():
    ts = datetime.now(timezone.utc).isoformat(timespec="seconds")
    os.makedirs(OUT_DIR, exist_ok=True)
    path = os.path.join(
        OUT_DIR, datetime.now().strftime("quota-records-%Y-%m.jsonl")
    )
    with open(path, "a") as f:
        f.write(json.dumps(record_subscription(ts)) + "\n")
    still_pending = 0
    for p in month_paths(datetime.now()):
        still_pending += flush_file(p, ts)
    msg = f"{ts} recorded muse-subscription tick -> {path}"
    if still_pending:
        msg += f" (turso: {still_pending} record(s) still pending)"
    else:
        msg += " (turso: all records landed)"
    print(msg)
    return 0


if __name__ == "__main__":
    sys.exit(main())
