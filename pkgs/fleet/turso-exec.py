#!/usr/bin/env python3
"""Run SQL against a Turso DB via Platform API token -> DB token -> libSQL pipeline.
Usage: turso-exec.py <db-name> <sql-file>
Reads platform token from ~/.config/turso/settings.json. Prints pipeline JSON results.
Never prints tokens."""
import json
import os, sys, urllib.request, urllib.error

def req(method, url, tok, body=None):
    data = json.dumps(body).encode() if body is not None else None
    r = urllib.request.Request(url, data=data, method=method)
    r.add_header("Authorization", f"Bearer {tok}")
    r.add_header("Content-Type", "application/json")
    r.add_header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/131.0.0.0 Safari/537.36")
    try:
        with urllib.request.urlopen(r) as resp:
            return resp.status, resp.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()

def main():
    db, sqlfile = sys.argv[1], sys.argv[2]
    org = "dezren39"
    platform = json.load(open(os.path.expanduser("~/.config/turso/settings.json")))["token"]
    s, b = req("POST", f"https://api.turso.tech/v1/organizations/{org}/databases/{db}/auth/tokens", platform, {})
    if s != 200:
        print("DB token mint failed", s, b[:300]); sys.exit(1)
    dbtok = json.loads(b)["jwt"]
    stmts = [x.strip() for x in open(sqlfile).read().split(";\n") if x.strip()]
    requests = [{"type": "execute", "stmt": {"sql": st}} for st in stmts] + [{"type": "close"}]
    s2, b2 = req("POST", f"https://{db}-{org}.aws-us-east-1.turso.io/v2/pipeline", dbtok, {"requests": requests})
    print("HTTP", s2)
    out = json.loads(b2)
    for i, res in enumerate(out.get("results", [])):
        if res.get("type") == "error":
            print(f"[{i}] ERROR", res["error"])
        else:
            r = res.get("response", {}).get("result", {})
            rows = r.get("rows", [])
            print(f"[{i}] ok affected={r.get('affected_row_count')} rowid={r.get('last_insert_rowid')} rows={rows}")

if __name__ == "__main__":
    main()
