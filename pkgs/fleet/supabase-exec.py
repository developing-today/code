#!/usr/bin/env python3
"""Run SQL against Drew's primary Supabase project via the Management API.

Usage: supabase-exec.py <sql-file> [project-ref]

The project ref defaults to the Bitwarden secret
`supabase.muse-dev2.project-ref` (the canonical pointer for muse-dev2,
the fleet's Supabase project since the 2026-10-30 cutover), so a
future project move is a Bitwarden change, not a code change. Pass the
ref explicitly to target another project (its scoped token must then be
used too — the canonical token is scoped to the canonical project).

Auth: reads the Bitwarden Secrets Manager access token from
~/.config/bitwarden/access-token, fetches `supabase.muse-dev2.access-token-muse-cli`
(the sbp_ management token) from project "developing today", and uses it as
a Bearer token. No secret is ever printed. Output is the JSON result rows.
"""
import json
import os
import subprocess
import sys
import urllib.request

BWS = os.path.expanduser("~/tools/bin/bws")
TOKEN_FILE = os.path.expanduser("~/.config/bitwarden/access-token")
SECRET_KEY = "supabase.muse-dev2.access-token-muse-cli"
REF_KEY = "supabase.muse-dev2.project-ref"


def bws_env():
    env = dict(os.environ)
    with open(TOKEN_FILE) as f:
        env["BWS_ACCESS_TOKEN"] = f.read().strip()
    return env


def get_secret(env, key):
    out = subprocess.run([BWS, "secret", "list", "-o", "json"],
                         capture_output=True, text=True, env=env, check=True)
    for s in json.loads(out.stdout):
        if s.get("key") == key:
            got = subprocess.run([BWS, "secret", "get", s["id"], "-o", "json"],
                                 capture_output=True, text=True, env=env, check=True)
            return json.loads(got.stdout)["value"]
    raise SystemExit(f"secret {key} not found in Bitwarden")


def run_sql(token, ref, sql):
    # curl, not urllib: urllib chunked reads through the sandbox egress
    # proxy intermittently die with IncompleteRead (hit repeatedly
    # 2026-10-20); curl handles the same responses reliably.
    import tempfile
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
        json.dump({"query": sql}, f)
        fname = f.name
    try:
        out = subprocess.run(
            ["curl", "-sS", "-m", "120", "-A", "Mozilla/5.0", "-X", "POST",
             "-H", f"Authorization: Bearer {token}",
             "-H", "Content-Type: application/json",
             "--data", f"@{fname}",
             f"https://api.supabase.com/v1/projects/{ref}/database/query",
             "-w", "\n%{http_code}"],
            capture_output=True, text=True, check=True).stdout
        body, _, code = out.rpartition("\n")
        return int(code), body
    finally:
        os.unlink(fname)


def main():
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    sql_file = sys.argv[1]
    with open(sql_file) as f:
        sql = f.read()
    env = bws_env()
    ref = sys.argv[2] if len(sys.argv) > 2 else get_secret(env, REF_KEY)
    token = get_secret(env, SECRET_KEY)
    status, text = run_sql(token, ref, sql)
    print(f"HTTP {status}")
    print(text)
    if status >= 300:
        sys.exit(1)


if __name__ == "__main__":
    main()
