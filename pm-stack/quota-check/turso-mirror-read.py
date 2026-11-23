#!/usr/bin/env python3
"""quota-check Turso mirror reads — per-DB libsql HTTP, per-DB tokens.

Implements developing-today/code#6828: quota-check's two Turso mirror
reads must NOT go through the Turso CLI (which authenticates with the
org-wide platform token in ~/.turso/config.json). Each DB is read over
the libsql HTTP pipeline (/v2/pipeline) with its own per-DB token,
fetched BY NAME via `fleet secrets get` (broker first):

  fleet-platform -> turso.fleet-platform-rw-token
  fleet-mirror   -> turso.fleet-mirror-rw-token

If the DB rejects the broker-served value (stale cache after a
rotation — the #6767 pattern from runtime-db-export's turso_client.py),
fall back to a direct BWS read of the same secret and retry once.
Token values are never printed, logged, or written to a file.

Usage:
  turso-mirror-read.py --db fleet-platform --sql "SELECT ..."
  turso-mirror-read.py --db fleet-mirror --sql-file query.sql

Output: a JSON array of row objects (column name -> plain value) on
stdout. Exit codes: 0 ok; 2 usage; 3 token resolution failed;
4 DB/auth failure (after the BWS fallback retry, where applicable).

Test hook: --token-name <bws-name> overrides which token is fetched
for the chosen DB — used by the #6828 scope check (one DB's token
must not read the other DB).
"""
import json
import os
import subprocess
import sys
import urllib.error
import urllib.request

DBS = {
    "fleet-platform": (
        "https://fleet-platform-dezren39.aws-us-east-1.turso.io",
        "turso.fleet-platform-rw-token",
    ),
    "fleet-mirror": (
        "https://fleet-mirror-dezren39.aws-us-east-1.turso.io",
        "turso.fleet-mirror-rw-token",
    ),
}
FLEET = os.path.expanduser("~/tools/fleet")


class AuthError(RuntimeError):
    pass


def _last_line(text):
    lines = [l for l in text.strip().splitlines() if l.strip()]
    return lines[-1].strip() if lines else ""


def token_from_fleet(name):
    out = subprocess.run([FLEET, "secrets", "get", name],
                         capture_output=True, text=True, timeout=60)
    if out.returncode != 0:
        raise RuntimeError(f"fleet secrets get failed rc={out.returncode}")
    value = _last_line(out.stdout)
    if not value:
        raise RuntimeError("fleet secrets get returned an empty value")
    return value


def token_from_bws(name):
    helper = os.path.expanduser("~/tools/bws-env.sh")
    script = (
        f"source {helper} >/dev/null 2>&1; "
        "SID=$(bws secret list 2>/dev/null | jq -r "
        f"'.[] | select(.key==\"{name}\") | .id'); "
        "bws secret get \"$SID\" | jq -r .value"
    )
    out = subprocess.run(["bash", "-c", script], capture_output=True,
                         text=True, timeout=60)
    if out.returncode != 0:
        raise RuntimeError(f"bws fallback failed rc={out.returncode}")
    value = _last_line(out.stdout)
    if not value:
        raise RuntimeError("bws fallback returned an empty value")
    return value


def _plain(v):
    t = v.get("type")
    raw = v.get("value")
    if t == "null" or raw is None:
        return None
    if t == "integer":
        return int(raw)
    if t == "float":
        return float(raw)
    return raw  # text; blob arrives base64 and is passed through as-is


def pipeline(base_url, token, sql):
    body = json.dumps({"requests": [
        {"type": "execute", "stmt": {"sql": sql, "args": []}},
        {"type": "close"},
    ]}).encode()
    req = urllib.request.Request(
        f"{base_url}/v2/pipeline", data=body,
        headers={"Authorization": f"Bearer {token}",
                 "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=120) as resp:
            data = json.load(resp)
    except urllib.error.HTTPError as e:
        detail = e.read().decode()[:300]
        if e.code in (401, 403, 404) and "auth" in detail.lower():
            raise AuthError(f"HTTP {e.code}: {detail}")
        raise RuntimeError(f"pipeline HTTP {e.code}: {detail}")
    rows = []
    for item in data.get("results", []):
        if item.get("type") == "error":
            msg = item["error"].get("message", "")
            if "auth" in msg.lower():
                raise AuthError(f"pipeline error: {msg}")
            raise RuntimeError(f"pipeline error: {msg}")
        res = item.get("response", {}).get("result")
        if res is None:
            continue
        cols = [c["name"] for c in res.get("cols", [])]
        for raw_row in res.get("rows", []):
            rows.append({c: _plain(v) for c, v in zip(cols, raw_row)})
    return rows


def main(argv):
    db = None
    sql = None
    sql_file = None
    token_name = None
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == "--db":
            i += 1
            db = argv[i]
        elif a == "--sql":
            i += 1
            sql = argv[i]
        elif a == "--sql-file":
            i += 1
            sql_file = argv[i]
        elif a == "--token-name":
            i += 1
            token_name = argv[i]
        else:
            print(f"unrecognized argument: {a}", file=sys.stderr)
            return 2
        i += 1
    if db not in DBS or (not sql and not sql_file):
        print(__doc__, file=sys.stderr)
        return 2
    if sql_file:
        with open(sql_file) as f:
            sql = f.read()
    base_url, default_token_name = DBS[db]
    name = token_name or default_token_name
    # Token resolution: the token VALUE stays in this process's memory
    # and the Authorization header only — never printed or persisted.
    try:
        token = token_from_fleet(name)
        source = "fleet"
    except RuntimeError as e:
        print(f"token resolution failed for {name}: {e}", file=sys.stderr)
        return 3
    try:
        rows = pipeline(base_url, token, sql)
    except AuthError:
        if source == "bws":
            print(f"auth rejected by {db} (token source: bws)", file=sys.stderr)
            return 4
        # #6767 fallback: broker may have served a stale cached value.
        try:
            token = token_from_bws(name)
        except RuntimeError as e:
            print(f"bws fallback failed for {name}: {e}", file=sys.stderr)
            return 3
        source = "bws"
        try:
            rows = pipeline(base_url, token, sql)
        except (AuthError, RuntimeError) as e:
            print(f"read failed on {db} after bws fallback: {e}", file=sys.stderr)
            return 4
    except RuntimeError as e:
        print(f"read failed on {db}: {e}", file=sys.stderr)
        return 4
    json.dump(rows, sys.stdout, indent=1)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
