#!/usr/bin/env python3
"""drill_compare.py — the restore drill's backup-vs-live check.

Usage: drill_compare.py <backup.sql> <manifest.sqlite>

The backup is a point-in-time export of the D1 manifest, and the
manifest is append-mostly: rows keep arriving after a backup is taken
(the nightly run itself appends transfer_ledger rows for its own
backup upload and any prune trashing). Comparing row *counts* therefore
false-fails every healthy backup sooner or later. The invariant that
actually means "this backup is good" is a subset check:

    every row in the backup exists in the live manifest with identical
    values in every column (backup ⊆ live, content-equal).

Extra live rows are expected — they are reported as a count note, never
a failure. A backup row that is missing from live, or present with
different content, is a failure naming the table and the row's key.

Live rows come from the D1 REST query API (same approach as
restore-drill.sh and backup-d1: token from CLOUDFLARE_API_TOKEN in the
environment, account/database from COLD_STORE_CF_ACCOUNT_ID /
COLD_STORE_CF_D1_DATABASE_ID or the cold-store defaults, base URL from
COLD_STORE_D1_BASE). If D1 is unreachable (transport failure only), the
fallback oracle is the local manifest SQLite file. A definitive D1
answer — the API rejecting the query (success=false) or a payload that
is not a query result — aborts the drill non-zero instead: comparing
against a stale local oracle while D1 itself is refusing would let a
restore drill pass on evidence D1 never gave (#6858). Primary keys
come from the schema itself
(PRAGMA table_info): objects.hash, bundles.bundle_id,
members.(bundle_id, object_hash), transfer_ledger.id, meta.key.

Prints one line per table and exits 0 when the invariant holds, 1 on
any mismatch.
"""

import json
import os
import sqlite3
import sys
import urllib.error
import urllib.request

ACCOUNT = os.environ.get("COLD_STORE_CF_ACCOUNT_ID", "234b866b99ce2b142d7963d8d139cbfa")
DATABASE = os.environ.get(
    "COLD_STORE_CF_D1_DATABASE_ID", "15d5f460-835c-4f24-b229-222908a5b30c"
)
D1_BASE = os.environ.get("COLD_STORE_D1_BASE", "https://api.cloudflare.com/client/v4")
TABLES_SQL = (
    "SELECT name, sql FROM sqlite_master WHERE type='table'"
    " AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_cf_%'"
)


class D1Error(Exception):
    """D1 answered, and the answer is definitive: the API rejected the
    query (success=false) or the payload is not a query result at all.
    The drill must fail loudly on this — never downgrade its oracle to
    the local manifest on the say-so of a D1 that is actively refusing
    or garbling its answers (#6858)."""


class D1Unreachable(Exception):
    """Transport failure: no D1 answer arrived (connection error,
    timeout). The documented local-manifest fallback applies."""


def _d1_first_error(payload):
    """The API's own (code, message) from a D1 error payload, or
    (None, None) when the payload does not carry them in the expected
    shape: a dict whose "errors" is a list whose first entry is a dict
    with a message. Never raises, whatever shape arrived (mirrors
    cold_store.py's _d1_first_error_message discipline, #2618)."""
    if not isinstance(payload, dict):
        return None, None
    errors = payload.get("errors")
    if isinstance(errors, list) and errors and isinstance(errors[0], dict):
        first = errors[0]
        message = first.get("message")
        if message:
            return first.get("code"), str(message)
    return None, None


def _d1_error_from_body(raw, what):
    """Build a D1Error from a response body, carrying the API's own
    error code/message when the body is a D1 error payload and the raw
    text otherwise."""
    try:
        payload = json.loads(raw) if raw else {}
    except (ValueError, TypeError):
        payload = {}
    code, message = _d1_first_error(payload)
    if message:
        detail = f"error {code}: {message}" if code is not None else message
        return D1Error(f"{what}: {detail}")
    text = (raw or b"").decode("utf-8", "replace").strip()
    return D1Error(f"{what}: {text[:200] if text else 'no error detail in response body'}")


def d1(sql):
    req = urllib.request.Request(
        f"{D1_BASE}/accounts/{ACCOUNT}/d1/database/{DATABASE}/query",
        data=json.dumps({"sql": sql}).encode(),
        method="POST",
        headers={
            "Authorization": "Bearer " + os.environ["CLOUDFLARE_API_TOKEN"],
            "Content-Type": "application/json",
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            raw = resp.read()
    except urllib.error.HTTPError as exc:
        # The server answered with an error status: that is a
        # definitive response, not unreachability — HTTPError is a
        # URLError subclass, so this arm must be caught first.
        try:
            body = exc.read()
        except Exception:
            body = b""
        raise _d1_error_from_body(body, f"D1 query failed (HTTP {exc.code})") from exc
    except (urllib.error.URLError, OSError) as exc:
        raise D1Unreachable(f"D1 unreachable: {exc}") from exc
    try:
        payload = json.loads(raw)
    except (ValueError, TypeError):
        raise D1Error("D1 query returned a non-JSON response body") from None
    if not isinstance(payload, dict):
        raise D1Error(
            f"D1 query returned a non-object payload ({type(payload).__name__})"
        )
    if not payload.get("success"):
        raise _d1_error_from_body(raw, "D1 query rejected")
    result = payload.get("result")
    if (
        not isinstance(result, list)
        or not result
        or not isinstance(result[0], dict)
        or not isinstance(result[0].get("results"), list)
        or not all(isinstance(r, dict) for r in result[0]["results"])
    ):
        raise D1Error("D1 query returned an unexpected shape: result rows missing")
    return result[0]["results"]


def fetch_source(manifest_path):
    """Return (creates, rows_by_table, source_label).

    creates is [(table_name, CREATE TABLE sql)]; rows_by_table maps a
    table name to its live rows as dicts. Tries live D1 first, falls
    back to the local manifest if D1 is unreachable.
    """
    try:
        meta = d1(TABLES_SQL)
        creates = [(r["name"], r["sql"]) for r in meta]
        rows = {r["name"]: d1(f'SELECT * FROM "{r["name"]}"') for r in meta}
        return creates, rows, "live D1"
    except D1Unreachable:
        conn = sqlite3.connect(f"file:{manifest_path}?mode=ro", uri=True)
        conn.row_factory = sqlite3.Row
        creates = [(r["name"], r["sql"]) for r in conn.execute(TABLES_SQL)]
        rows = {
            name: [dict(r) for r in conn.execute(f'SELECT * FROM "{name}"')]
            for name, _sql in creates
        }
        conn.close()
        return creates, rows, "local manifest (D1 unreachable)"


def table_shape(conn, table):
    info = conn.execute(f"PRAGMA table_info({table})").fetchall()
    cols = [r[1] for r in info]
    pk = [r[1] for r in sorted((r for r in info if r[5]), key=lambda r: r[5])]
    return cols, pk


def row_key(row, cols, pk):
    fields = pk if pk else cols
    return tuple(row[c] for c in fields)


def key_label(row, cols, pk):
    fields = pk if pk else cols
    return "(" + ", ".join(f"{c}={row[c]!r}" for c in fields) + ")"


def main(argv):
    export_path, manifest_path = argv[1], argv[2]
    try:
        creates, live_rows, source = fetch_source(manifest_path)
    except D1Error as exc:
        print(f"  drill aborted: {exc}")
        return 1

    mem = sqlite3.connect(":memory:")
    mem.row_factory = sqlite3.Row
    for _name, sql in creates:
        if sql:
            mem.execute(sql)
    try:
        with open(export_path) as f:
            mem.executescript(f.read())
    except sqlite3.Error as exc:
        print(f"  backup does not load: {exc}")
        return 1

    ok = True
    for name, _sql in sorted(creates):
        cols, pk = table_shape(mem, name)
        backup = [dict(r) for r in mem.execute(f'SELECT * FROM "{name}"')]
        live = live_rows.get(name, [])
        live_by_key = {row_key(r, cols, pk): r for r in live}
        line = f"  {name}: backup has {len(backup)} row(s), {source} has {len(live)}"
        extra = len(set(live_by_key) - {row_key(r, cols, pk) for r in backup})
        if extra:
            line += f" ({extra} additional row(s) appended since this backup)"
        print(line)
        for row in backup:
            key = row_key(row, cols, pk)
            label = key_label(row, cols, pk)
            if key not in live_by_key:
                print(f"  {name}: FAIL backup row {label} is missing in {source}")
                ok = False
                continue
            other = live_by_key[key]
            differing = [c for c in cols if row[c] != other.get(c)]
            if differing:
                print(
                    f"  {name}: FAIL backup row {label} differs in {source}"
                    f" (columns: {', '.join(differing)})"
                )
                ok = False
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
