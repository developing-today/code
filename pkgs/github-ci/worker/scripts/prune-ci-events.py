#!/usr/bin/env python3
"""Prune fleet-platform `ci_events` past the 90-day hot retention (#5693).

The ci events ledger lives in Turso `fleet-platform.ci_events`. The
design's retention rule is 90 days hot; this script is the prune that
enforces it:

    DELETE FROM ci_events WHERE received_at < datetime('now', '-90 days')

Usage: prune-ci-events.py [--days N] [--dry-run]

The Turso token is resolved by NAME only (`turso.fleet-platform-rw-token`
via `fleet secrets get`) and is never printed or written to disk.
Run it from any host that can reach the Turso pipeline endpoint and
the fleet secrets broker (the laptop, or an agent sandbox).
"""

import argparse
import json
import subprocess
import sys
import urllib.request

TURSO_URL = "https://d1-gateway.developing-today.workers.dev/db/fleet-platform/v2/pipeline"
TOKEN_NAME = "turso.fleet-platform-rw-token"


def get_token():
    out = subprocess.run(
        ["fleet", "secrets", "get", TOKEN_NAME],
        capture_output=True,
        text=True,
        timeout=60,
    )
    if out.returncode != 0:
        raise SystemExit(f"token fetch failed for {TOKEN_NAME} (rc={out.returncode})")
    return out.stdout.strip()


def execute(token, sql):
    body = json.dumps(
        {
            "requests": [
                {"type": "execute", "stmt": {"sql": sql}},
                {"type": "close"},
            ]
        }
    ).encode()
    req = urllib.request.Request(
        TURSO_URL,
        data=body,
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
            # Cloudflare (d1-gateway) refuses the default Python-urllib agent.
            "User-Agent": "fleet-d1-client/1",
        },
    )
    with urllib.request.urlopen(req, timeout=120) as resp:
        payload = json.load(resp)
    result = payload["results"][0]["response"]["result"]
    if result is None:
        raise SystemExit(f"pipeline error for: {sql[:60]}")
    return result


def scalar(result):
    return result["rows"][0][0]["value"]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--days",
        type=int,
        default=90,
        help="hot retention window in days (default 90, per #5693 design)",
    )
    ap.add_argument(
        "--dry-run",
        action="store_true",
        help="count the rows past the window without deleting",
    )
    args = ap.parse_args()
    if args.days < 1:
        raise SystemExit("--days must be positive")

    token = get_token()
    where = f"received_at < datetime('now', '-{args.days} days')"
    total = scalar(execute(token, "SELECT count(*) FROM ci_events"))
    stale = scalar(execute(token, f"SELECT count(*) FROM ci_events WHERE {where}"))
    print(f"ci_events total={total} past-{args.days}d={stale}")
    if args.dry_run:
        print("dry-run: nothing deleted")
        return 0
    result = execute(token, f"DELETE FROM ci_events WHERE {where}")
    print(f"deleted={result['affected_row_count']}")
    total_after = scalar(execute(token, "SELECT count(*) FROM ci_events"))
    print(f"ci_events total after={total_after}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
