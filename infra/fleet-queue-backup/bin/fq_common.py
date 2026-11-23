#!/usr/bin/env python3
"""fq_common.py — shared helpers for the fleet-queue backup system.

Canonical home: developing-today/code, infra/fleet-queue-backup/.
Implements slice 6 of the fleet unification design (#8575) for the
queue store: a scheduled Turso -> D1 export of the fleet-queue
database (#8584), plus the restore drill that proves the D1 copy can
be brought back into a live Turso store.

Credential contract: this module never reads credential files and
never prints a credential. Callers pass tokens in memory (the systemd
wrapper fetches them from the vault by name at runtime and exports
them only to its own process tree; see systemd/run-export.sh).

Transport contracts:
- The Turso/libsql side speaks the libsql HTTP pipeline protocol
  (POST {url}/v2/pipeline) with typed statement arguments, so row
  values never pass through SQL text on the restore path.
- The export path generates SQL text for the D1 import API. Every
  literal in that text is produced by SQLite itself (SELECT quote(...)),
  never by Python string formatting, so escaping is the database's own.
- Paged reads follow the repo pagination contract (#6622): a paged
  read is complete, bounded, or says so. Every page loop carries a
  bound whose breach is an error naming the listing.
"""

import base64
import hashlib
import json
import os
import re
import time
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
FQ_DIR = os.path.dirname(HERE)

# The fleet-queue store (pkgs/fleet turso_queue::DEFAULT_TURSO_URL and
# TURSO_TOKEN_SECRET_NAME name the same pair for the broker).
DEFAULT_TURSO_URL = "https://fleet-queue-dezren39.aws-us-east-1.turso.io"
TURSO_TOKEN_SECRET_NAME = "turso.fleet-queue-rw-token"
CLOUDFLARE_TOKEN_SECRET_NAME = "cloudflare.api-token-account-wide"
DEFAULT_D1_NAME = "fleet-queue-backup"
META_TABLE = "_fleet_queue_backup_meta"

# Page size / bound for row reads (#6622): at 500 rows per page and a
# 200,000-page bound this admits 100M rows per table — far above the
# queue store's real size — while guaranteeing every loop terminates.
PAGE_ROWS = 500
MAX_PAGES = 200_000

HTTP_TIMEOUT_SECONDS = 120

# The D1 import upload PUTs the whole dump in one request (~1 GB for
# the live queue store); it gets a far longer socket timeout than the
# JSON API calls above it.
UPLOAD_TIMEOUT_SECONDS = 3600

# D1 caps one SQL statement at 100,000 bytes, and the import API
# enforces it: a larger statement fails the whole import with
# SQLITE_TOOBIG (seen live on the first #8584 export; the D1 import
# docs' "Statement too long" section names the same limit). A row
# whose quote()-rendered INSERT exceeds the cap is split instead:
# its largest TEXT/BLOB cells are chunked, chunk 1 riding the INSERT
# and the rest appended by UPDATE ... SET col = col || '<chunk>'
# statements keyed on the table's primary key, each statement under
# the cap. build_dump records every split so the exporter can verify
# the reassembled lengths in D1 after the import.
MAX_IMPORT_STATEMENT_BYTES = 90_000
SPLIT_CHUNK_BYTES = 40_000


def _shape(value):
    return type(value).__name__


# ------------------------------------------------------------ HTTP core
def http_json(method, url, token, payload=None, timeout=HTTP_TIMEOUT_SECONDS):
    """One JSON HTTP call with a bearer token; returns the parsed body.

    Raises RuntimeError naming the endpoint (never the token) on any
    transport or HTTP failure.
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
    except (urllib.error.URLError, TimeoutError, OSError) as exc:
        raise RuntimeError(f"{method} {url} failed: {exc}") from exc


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


def _stmt(sql, args):
    stmt = {"sql": sql}
    if args:
        stmt["args"] = [hrana_value(a) for a in args]
    return stmt


# ------------------------------------------------------- schema reading
def read_schema(client):
    """All user schema objects as (type, name, tbl_name, sql), in
    creation order. Internal sqlite_% objects and this system's own
    meta table are excluded: the meta table is written by the export,
    not copied from the source."""
    _cols, rows = client.execute(
        "SELECT type, name, tbl_name, sql FROM sqlite_master "
        "WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' "
        "AND name <> ? ORDER BY rowid",
        (META_TABLE,),
    )
    return rows


def table_names(schema):
    return [name for kind, name, _tbl, _sql in schema if kind == "table"]


def count_rows(client, table):
    _cols, rows = client.execute(f'SELECT count(*) FROM "{table}"')
    return rows[0][0]


def table_counts(client, schema):
    return {name: count_rows(client, name) for name in table_names(schema)}


def read_table_quoted(client, table, ncols):
    """Yield rows of `table` with every cell pre-rendered by SQLite's
    quote() as SQL literal text — the export's escaping oracle.

    Column names come from the schema itself, via a zero-row probe, so
    quote() wraps the real columns in order. Paged by OFFSET with the
    #6622 bound: a table whose pages never shorten past the bound
    raises instead of looping forever.
    """
    colnames, _probe = client.execute(f'SELECT * FROM "{table}" LIMIT 0')
    if len(colnames) != ncols:
        raise RuntimeError(f"{table}: expected {ncols} columns, found {len(colnames)}")
    quoted = ", ".join(f'quote("{c}")' for c in colnames)
    offset = 0
    for _page in range(MAX_PAGES):
        _cols, rows = client.execute(
            f'SELECT {quoted} FROM "{table}" LIMIT {PAGE_ROWS} OFFSET {offset}'
        )
        for row in rows:
            yield row
        if len(rows) < PAGE_ROWS:
            return
        offset += PAGE_ROWS
    raise RuntimeError(
        f"{table}: pagination exceeded {MAX_PAGES} pages of {PAGE_ROWS}; "
        "refusing to page unboundedly"
    )


def _decode_literal(literal):
    """Inverse of SQLite quote() for TEXT and BLOB literals.

    Returns ("text", str) or ("blob", bytes); (None, None) for anything
    else (numbers, NULL) -- those are never split candidates.
    """
    if len(literal) >= 2 and literal.startswith("'") and literal.endswith("'"):
        return "text", literal[1:-1].replace("''", "'")
    if len(literal) >= 3 and literal.startswith("X'") and literal.endswith("'"):
        return "blob", bytes.fromhex(literal[2:-1])
    return None, None


def _render_literal(kind, value):
    if kind == "text":
        return "'" + value.replace("'", "''") + "'"
    return "X'" + value.hex().upper() + "'"


def _value_chunks(kind, value, first_budget=None):
    """Chunk a decoded TEXT/BLOB value: the first chunk is at most
    `first_budget` bytes (default SPLIT_CHUNK_BYTES), later chunks at
    most SPLIT_CHUNK_BYTES; sizes are UTF-8 bytes for text and a UTF-8
    sequence is never split."""
    limit = first_budget or SPLIT_CHUNK_BYTES
    if kind == "text":
        chunks, current, size = [], "", 0
        for ch in value:
            n = len(ch.encode("utf-8"))
            if current and size + n > limit:
                chunks.append(current)
                current, size = "", 0
                limit = SPLIT_CHUNK_BYTES
            current += ch
            size += n
        if current or not chunks:
            chunks.append(current)
        return chunks
    chunks = [value[:limit]]
    rest = value[limit:]
    chunks.extend(
        rest[i : i + SPLIT_CHUNK_BYTES]
        for i in range(0, len(rest), SPLIT_CHUNK_BYTES)
    )
    return [c for c in chunks if c] or [b""]


def _pk_columns(create_sql):
    """Primary-key column names from a CREATE TABLE statement, or None.

    Handles the two shapes the queue schema uses: a table-level
    PRIMARY KEY (a, b) definition and an inline column PRIMARY KEY.
    The parenthesised body is split into top-level definitions first,
    so a single-line CREATE cannot confuse the match.
    """
    start = create_sql.find("(")
    if start < 0:
        return None
    depth, end = 0, None
    for i in range(start, len(create_sql)):
        if create_sql[i] == "(":
            depth += 1
        elif create_sql[i] == ")":
            depth -= 1
            if depth == 0:
                end = i
                break
    if end is None:
        return None
    defs, depth, cur = [], 0, ""
    for ch in create_sql[start + 1 : end]:
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
        if ch == "," and depth == 0:
            defs.append(cur)
            cur = ""
        else:
            cur += ch
    if cur.strip():
        defs.append(cur)
    for d in defs:
        m = re.match(r"\s*PRIMARY\s+KEY\s*\(([^)]*)\)", d, re.IGNORECASE)
        if m:
            return [c.strip().strip('"`[]') for c in m.group(1).split(",")]
    for d in defs:
        d = d.strip()
        if d.upper().startswith(
            ("PRIMARY", "FOREIGN", "UNIQUE", "CHECK", "CONSTRAINT")
        ):
            continue
        m = re.match(r'"?([A-Za-z_][A-Za-z0-9_]*)"?\s+', d)
        if m and re.search(r"\bPRIMARY\s+KEY\b", d, re.IGNORECASE):
            return [m.group(1)]
    return None


def _split_row_insert(table, colnames, row, pk_cols, splits):
    """Render one oversized row as an INSERT plus UPDATE appends.

    The largest splittable (TEXT/BLOB) cells are chunked until the
    INSERT fits under MAX_IMPORT_STATEMENT_BYTES; the remaining chunks
    are appended by UPDATEs keyed on the primary key (or, when the
    table declares none, on every unsplit column) with a running-length
    guard, so a mis-keyed UPDATE matches nothing instead of corrupting
    a row. Every split is recorded in `splits` as (table, column,
    where_sql, expected_length) for post-import verification. A row
    that cannot be split safely raises: a loud failure beats a silently
    wrong backup copy.
    """
    cells = list(row)
    key_idx = [colnames.index(c) for c in pk_cols] if pk_cols else None
    split_cells = {}
    while True:
        stmt = f'INSERT INTO "{table}" VALUES (' + ", ".join(cells) + ");"
        if len(stmt.encode("utf-8")) <= MAX_IMPORT_STATEMENT_BYTES:
            break
        candidates = [
            i
            for i in range(len(cells))
            if i not in split_cells
            and i not in (key_idx or [])
            and _decode_literal(cells[i])[0] is not None
        ]
        if not candidates:
            raise RuntimeError(
                f"{table}: row exceeds the D1 statement limit and has no "
                "splittable cell left; refusing to emit it"
            )
        target = max(candidates, key=lambda i: len(cells[i].encode("utf-8")))
        kind, value = _decode_literal(cells[target])
        byte_len = len(value.encode("utf-8")) if kind == "text" else len(value)
        overflow = len(stmt.encode("utf-8")) - MAX_IMPORT_STATEMENT_BYTES
        first_budget = max(1024, byte_len - overflow)
        chunks = _value_chunks(kind, value, first_budget)
        if len(chunks) < 2:
            raise RuntimeError(
                f"{table}: oversized cell did not split into chunks"
            )
        split_cells[target] = (kind, value, chunks)
        cells[target] = _render_literal(kind, chunks[0])
    if key_idx is None:
        key_idx = [i for i in range(len(cells)) if i not in split_cells]
        if not key_idx:
            raise RuntimeError(
                f"{table}: oversized row has no column left to key its "
                "UPDATE appends on; refusing to emit it"
            )
    where = " AND ".join(f'"{colnames[i]}" IS {row[i]}' for i in key_idx)
    out = [stmt]
    for target, (kind, value, chunks) in split_cells.items():
        col = colnames[target]
        done = len(chunks[0])
        for chunk in chunks[1:]:
            # SQLite's || always yields TEXT, so a BLOB cell's appends
            # CAST back to BLOB to keep the rebuilt value's storage
            # class (and bytes) identical to the source.
            expr = f'"{col}" || {_render_literal(kind, chunk)}'
            if kind == "blob":
                expr = f"CAST({expr} AS BLOB)"
            update = (
                f'UPDATE "{table}" SET "{col}" = {expr} '
                f'WHERE {where} AND length("{col}") = {done};'
            )
            if len(update.encode("utf-8")) > MAX_IMPORT_STATEMENT_BYTES:
                raise RuntimeError(
                    f"{table}: UPDATE append for column {col} exceeds the "
                    "D1 statement limit; refusing to emit it"
                )
            out.append(update)
            done += len(chunk)
        splits.append((table, col, where, len(value)))
    return out


def build_dump(client, exported_at):
    """A complete, re-importable SQL dump of the store as D1-safe text.

    Layout: DROP + CREATE for every table, then INSERTs (literals by
    quote()), then indexes/views/triggers, then the meta table holding
    the export timestamp and the per-table row counts the restore
    drill and the freshness readout verify against. Returns
    (dump_text, counts, splits), where splits records each row that
    had to be chunked for the D1 statement limit (see
    _split_row_insert), for post-import verification.
    """
    schema = read_schema(client)
    tables = [(name, sql) for kind, name, _t, sql in schema if kind == "table"]
    # Counts are taken from the rows actually emitted below, never from
    # a separate COUNT(*) pass: on the live store the two reads smear
    # (rows land between them), which made the exporter's read-back
    # check fail on a faithful copy and left the meta describing no
    # copy that ever existed (live run 4, #8584). Emitted counts make
    # the meta, the read-back check and the restore drill all describe
    # the same artifact.
    counts = {name: 0 for name, _sql in tables}
    lines = [
        "-- fleet-queue-backup export (#8584)",
        f"-- exported_at: {exported_at}",
        f"-- source tables: {len(tables)}",
    ]
    # Drop every object the dump recreates, dependents first, so the
    # dump is idempotent against a target already holding a copy
    # (re-import is the scheduled refresh path, not an error).
    for kind, name, _t, _sql in schema:
        if kind in ("trigger", "view", "index"):
            lines.append(f'DROP {kind.upper()} IF EXISTS "{name}";')
    for name, _sql in tables:
        lines.append(f'DROP TABLE IF EXISTS "{name}";')
    for _name, sql in tables:
        lines.append(sql.rstrip().rstrip(";") + ";")
    splits = []
    for name, sql in tables:
        colnames, _probe = client.execute(f'SELECT * FROM "{name}" LIMIT 0')
        ncols = len(colnames)
        pk_cols = _pk_columns(sql)
        for row in read_table_quoted(client, name, ncols):
            counts[name] += 1
            stmt = f'INSERT INTO "{name}" VALUES (' + ", ".join(row) + ");"
            if len(stmt.encode("utf-8")) <= MAX_IMPORT_STATEMENT_BYTES:
                lines.append(stmt)
            else:
                lines.extend(
                    _split_row_insert(name, colnames, row, pk_cols, splits)
                )
    for kind, _name, _t, sql in schema:
        if kind in ("index", "view", "trigger"):
            lines.append(sql.rstrip().rstrip(";") + ";")
    lines.append(f'DROP TABLE IF EXISTS "{META_TABLE}";')
    lines.append(
        f'CREATE TABLE "{META_TABLE}" (key TEXT PRIMARY KEY, value TEXT NOT NULL);'
    )
    meta = {"exported_at": exported_at, "counts": counts}
    lines.append(
        f'INSERT INTO "{META_TABLE}" VALUES ('
        + ", ".join(
            "'" + s.replace("'", "''") + "'"
            for s in ("export", json.dumps(meta, sort_keys=True))
        )
        + ");"
    )
    return "\n".join(lines) + "\n", counts, splits


# ------------------------------------------------------------- Cloudflare
class D1Client:
    """The slice of the Cloudflare D1 API this system uses."""

    API = "https://api.cloudflare.com/client/v4"

    def __init__(self, token, account_id):
        self.token = token
        self.account = account_id

    def _call(self, method, path, payload=None):
        data = http_json(method, f"{self.API}{path}", self.token, payload)
        if not data.get("success"):
            raise RuntimeError(f"D1 API {method} {path} failed: {str(data)[:300]}")
        return data.get("result")

    def find_database(self, name):
        result = self._call("GET", f"/accounts/{self.account}/d1/database?name={name}")
        for db in result or []:
            if db.get("name") == name:
                return db
        return None

    def create_database(self, name):
        return self._call(
            "POST", f"/accounts/{self.account}/d1/database", {"name": name}
        )

    def query(self, db_id, sql, params=()):
        result = self._call(
            "POST",
            f"/accounts/{self.account}/d1/database/{db_id}/query",
            {"sql": sql, "params": list(params)},
        )
        if not result:
            return []
        return result[0].get("results") or []

    def import_sql(self, db_id, sql_text):
        """Import SQL text via the D1 import API (init -> upload -> ingest -> poll).

        The poll step identifies the import by the current_bookmark
        the ingest step returns (its at_bookmark); polling with the
        filename instead is rejected by the live API with
        "current_bookmark => Required" (seen on the first live run,
        2026-11-22, #8584). Completion is the result status reaching
        "complete"; a result error aborts.

        The upload is a single PUT of the whole dump (~1 GB for the
        live queue store), so it gets its own long timeout, and a
        transport failure re-runs init + upload (the dump is already
        built; an upload retry never re-reads the source). API-level
        errors are not retried: they do not heal.
        """
        digest = hashlib.md5(sql_text.encode()).hexdigest()
        filename = None
        for attempt in range(3):
            init = self._call(
                "POST",
                f"/accounts/{self.account}/d1/database/{db_id}/import",
                {"action": "init", "etag": digest},
            )
            upload_url, filename = init["upload_url"], init["filename"]
            req = urllib.request.Request(
                upload_url, data=sql_text.encode(), method="PUT"
            )
            try:
                with urllib.request.urlopen(
                    req, timeout=UPLOAD_TIMEOUT_SECONDS
                ) as resp:
                    etag = resp.headers.get("etag", "").strip('"')
            except (urllib.error.URLError, TimeoutError, OSError) as exc:
                if attempt == 2:
                    raise RuntimeError(
                        f"D1 import upload failed after 3 attempts: {exc}"
                    ) from exc
                time.sleep(5)
                continue
            if etag == digest:
                break
            if attempt == 2:
                raise RuntimeError(
                    f"D1 import upload etag mismatch: sent {digest}, got {etag}"
                )
            time.sleep(5)
        ingest = self._call(
            "POST",
            f"/accounts/{self.account}/d1/database/{db_id}/import",
            {"action": "ingest", "etag": digest, "filename": filename},
        )
        bookmark = ingest.get("at_bookmark")
        if not bookmark:
            raise RuntimeError(f"D1 import ingest returned no at_bookmark: {ingest}")
        # A full-store import executes ~600k statements server-side;
        # the poll bound (360 x 5s = 30 min) is sized to that, not to
        # the hermetic fixtures.
        for _attempt in range(360):
            poll = self._call(
                "POST",
                f"/accounts/{self.account}/d1/database/{db_id}/import",
                {"action": "poll", "current_bookmark": bookmark},
            )
            if poll.get("error"):
                raise RuntimeError(f"D1 import failed: {poll["error"]}")
            if poll.get("status") == "complete":
                return poll
            time.sleep(5)
        raise RuntimeError("D1 import did not complete within the poll bound")

    def read_meta(self, db_id):
        rows = self.query(
            db_id, f'SELECT value FROM "{META_TABLE}" WHERE key = ?', ("export",)
        )
        if not rows:
            return None
        return json.loads(rows[0]["value"])

    def counts(self, db_id, names):
        out = {}
        for name in names:
            rows = self.query(db_id, f'SELECT count(*) AS n FROM "{name}"')
            out[name] = rows[0]["n"]
        return out
