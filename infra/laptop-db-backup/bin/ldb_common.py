#!/usr/bin/env python3
"""ldb_common.py — shared helpers for the laptop-db backup mesh legs.

Canonical home: developing-today/code, infra/laptop-db-backup/.
Implements mesh class E of the backup mesh design (#9093, design
doc/2026-11-23T16-34-18Z_design_backup_mesh): qualifying laptop-local
SQLite databases get a dedicated Turso home (`laptop-backup-<name>`)
filled by a weekly wholesale-replace import (#13459 inventory,
#13474 steps 2-5). The import discipline mirrors the class-D leg
(infra/fleet-queue-backup/bin/export_to_turso.py, #13458): staged
load, verified counts, marker-witnessed swap, import record in the
home's own meta table.

Credential contract: this module never reads credential files and
never prints a credential. Callers pass tokens in memory (the systemd
wrapper fetches them from the vault by name at runtime and exports
them only to its own process tree; see systemd/run-*.sh).

Transport contract: the Turso/libsql side speaks the libsql HTTP
pipeline protocol (POST {url}/v2/pipeline) with typed statement
arguments, so row values never pass through SQL text.

Paged reads follow the repo pagination contract (#6622): a paged
read is complete, bounded, or says so. Every page loop carries a
bound whose breach is an error naming the listing.
"""

import base64
import json
import time
import urllib.error
import urllib.request

# Page size / bound for row reads (#6622).
PAGE_ROWS = 500
MAX_PAGES = 200_000

HTTP_TIMEOUT_SECONDS = 120

HOME_META_TABLE = "_laptop_backup_home_meta"


def _shape(value):
    return type(value).__name__


# ------------------------------------------------------------ HTTP core
TRANSPORT_ATTEMPTS = 5


def http_json(method, url, token, payload=None, timeout=HTTP_TIMEOUT_SECONDS):
    """One JSON HTTP call; returns the parsed body.

    HTTP failures raise RuntimeError (see _http_json_once). Transport
    failures (connection reset, timeout, DNS) are retried with
    backoff first: over the fleet's paths a single reset deep into a
    long import must not be fatal. Retrying a write whose first
    attempt may have landed is safe here — the staged load drops and
    recreates its staging tables, and the swap verifies final counts
    and fails loudly rather than passing a wrong copy.
    """
    last_exc = None
    for attempt in range(TRANSPORT_ATTEMPTS):
        try:
            return _http_json_once(method, url, token, payload, timeout)
        except (urllib.error.URLError, TimeoutError, OSError) as exc:
            last_exc = exc
            if attempt + 1 < TRANSPORT_ATTEMPTS:
                time.sleep(min(30, 2 ** (attempt + 1)))
    raise RuntimeError(f"{method} {url} failed: {last_exc}") from last_exc


def _http_json_once(method, url, token, payload=None, timeout=HTTP_TIMEOUT_SECONDS):
    """One JSON HTTP call with a bearer token; returns the parsed body.

    One attempt. HTTP failures raise RuntimeError naming the
    endpoint (never the token); transport failures propagate raw so
    http_json can retry them.
    """
    data = None if payload is None else json.dumps(payload).encode()
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Authorization", f"Bearer {token}")
    if data is not None:
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return json.loads(resp.read().decode())
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode(errors="replace")[:300]
        raise RuntimeError(f"{method} {url} -> HTTP {exc.code}: {detail}") from exc


# ------------------------------------------------------- libsql (Turso)
def hrana_value(value):
    """Encode one Python value as a libsql/Hrana JSON value."""
    if value is None:
        return {"type": "null"}
    if isinstance(value, bool):
        return {"type": "integer", "value": str(1 if value else 0)}
    if isinstance(value, int):
        return {"type": "integer", "value": str(value)}
    if isinstance(value, float):
        return {"type": "float", "value": value}
    if isinstance(value, (bytes, bytearray)):
        return {"type": "blob", "value": base64.b64encode(bytes(value)).decode()}
    if isinstance(value, str):
        return {"type": "text", "value": value}
    raise ValueError(f"cannot encode {_shape(value)} as a libsql value")


def python_value(cell):
    """Decode one libsql/Hrana JSON value cell to a Python value."""
    kind = cell.get("type")
    if kind == "null":
        return None
    if kind == "integer":
        return int(cell["value"])
    if kind == "float":
        return float(cell["value"])
    if kind == "text":
        return cell["value"]
    if kind == "blob":
        return base64.b64decode(cell["value"])
    raise ValueError(f"unknown libsql value type {kind!r}")


def _stmt(sql, args):
    stmt = {"sql": sql}
    if args:
        stmt["args"] = [hrana_value(a) for a in args]
    return stmt


class LibsqlClient:
    """Minimal libsql HTTP pipeline client (one statement per request)."""

    def __init__(self, url, token):
        self.base = url.rstrip("/")
        self.token = token

    def execute(self, sql, args=()):
        stmt = {"sql": sql}
        if args:
            stmt["args"] = [hrana_value(a) for a in args]
        body = {"requests": [{"type": "execute", "stmt": stmt}, {"type": "close"}]}
        data = http_json("POST", f"{self.base}/v2/pipeline", self.token, body)
        results = data.get("results") or []
        if not results or results[0].get("type") != "ok":
            raise RuntimeError(
                f"libsql execute failed for {sql[:80]!r}: {str(data)[:300]}"
            )
        result = results[0]["response"]["result"]
        cols = [c["name"] for c in result.get("cols", [])]
        rows = [
            tuple(python_value(cell) for cell in row) for row in result.get("rows", [])
        ]
        return cols, rows

    def execute_many(self, statements):
        """Run (sql, args) pairs as one pipeline batch; raises on any error."""
        requests = [
            {"type": "execute", "stmt": _stmt(sql, args)} for sql, args in statements
        ]
        requests.append({"type": "close"})
        data = http_json(
            "POST", f"{self.base}/v2/pipeline", self.token, {"requests": requests}
        )
        for item in data.get("results") or []:
            if item.get("type") != "ok":
                raise RuntimeError(f"libsql batch failed: {str(item)[:300]}")


# ------------------------------------------------------- schema reading
def read_schema(client, meta_table=HOME_META_TABLE):
    """All user schema objects as (type, name, tbl_name, sql), in
    creation order. Internal sqlite_% objects and the home's own
    meta table are excluded: the meta table is written by the
    import, not copied from the source."""
    _cols, rows = client.execute(
        "SELECT type, name, tbl_name, sql FROM sqlite_master "
        "WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' "
        "AND name <> ? ORDER BY rowid",
        (meta_table,),
    )
    return rows


def table_names(schema):
    return [name for kind, name, _tbl, _sql in schema if kind == "table"]


def count_rows(client, table):
    _cols, rows = client.execute(f'SELECT count(*) FROM "{table}"')
    return rows[0][0]


def table_counts(client, schema):
    return {name: count_rows(client, name) for name in table_names(schema)}


def read_table_paged(client, table):
    """Yield every row of `table` in OFFSET pages (#6622 bound)."""
    offset = 0
    for _page in range(MAX_PAGES):
        _cols, rows = client.execute(
            f'SELECT * FROM "{table}" LIMIT {PAGE_ROWS} OFFSET {offset}'
        )
        if not rows:
            return
        yield from rows
        if len(rows) < PAGE_ROWS:
            return
        offset += PAGE_ROWS
    raise RuntimeError(f"{table}: page bound {MAX_PAGES} exceeded")
