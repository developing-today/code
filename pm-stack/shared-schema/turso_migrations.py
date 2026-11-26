"""Turso (SQLite-dialect) application of the shared-schema migrations.

The migration files in ``migrations/`` are the schema contract, written
in PostgreSQL dialect for the hosted-Postgres convention they were
designed under. The live copies of these tables are in Turso (see
README.md: the Supabase project was deleted and the schema was created
in Turso during the 2026-11-18 recovery), and until now there was no
migration runner at all — "migrations are applied by hand in version
order" (README.md). This module is that runner for the Turso stores:

- ``translate`` converts one migration's PostgreSQL DDL to the SQLite
  dialect, following the same conventions the aggregator's live bridge
  already uses (``aggregator/src/turso-stores.ts``): ``public.``
  prefixes dropped, ``jsonb``/``timestamptz`` become TEXT, the ``text[]``
  dedupe column is stored as JSON text (``'{}'`` becomes ``'[]'``), and
  ``now()`` becomes ``strftime('%Y-%m-%dT%H:%M:%fZ', 'now')``.
  PostgreSQL-only machinery with no Turso counterpart — row-level
  security, grants to ``service_role``, table comments — is dropped,
  exactly as the README records ("RLS is enabled on both tables
  (PostgreSQL DDL only; Turso has no RLS)").
- The translator is deliberately a closed set of mechanical rules, not
  a SQL parser. Any construct it does not recognise makes it raise
  ``TranslationError`` rather than guess, and the test suite pins the
  exact translated output of every migration in ``golden/`` so a new
  construct in a future migration fails loudly until its translation
  is reviewed and the golden regenerated.
- ``apply_migrations`` records each applied migration in the target
  store's ``pm_schema_migrations`` ledger and applies each migration
  as one transaction: a migration either lands whole, ledger row
  included, or not at all. Re-running reports already-applied
  migrations as no-ops, as #14256 requires.

JSON type checks need care: PostgreSQL ``jsonb_typeof`` reports one
``number`` type, while SQLite ``json_type`` distinguishes ``integer``
from ``real``, and reports booleans as ``true``/``false``. The rules
below map between the two vocabularies exactly.

Regex checks (``~``) have no SQLite operator, so the three patterns
the migrations use are rewritten to equivalent GLOB/length forms; the
functional tests exercise each rewrite with accepting and rejecting
values, including the segment-shape cases of the github_ref pattern.
"""

from __future__ import annotations

import json
import re
import sqlite3
import urllib.error
import urllib.request
from pathlib import Path
from urllib.parse import urlsplit

MIGRATIONS_DIR = Path(__file__).resolve().parent / "migrations"
LEDGER_TABLE = "pm_schema_migrations"
NOW_SQL = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')"


class TranslationError(ValueError):
    """A migration uses a construct this translator does not cover."""


class StoreError(RuntimeError):
    """A store round trip failed or its outcome could not be verified."""


# ---------------------------------------------------------------------
# Statement splitting
# ---------------------------------------------------------------------


def split_statements(sql: str) -> list[str]:
    """Split a migration file into statements.

    ``--`` comments are stripped and statements are split on ``;``
    outside string literals (``''`` is an escaped quote). The migration
    files use no other quoting, and the leftover checks in
    ``translate_statement`` refuse anything unexpected downstream.
    """
    statements: list[str] = []
    buf: list[str] = []
    in_string = False
    i = 0
    while i < len(sql):
        ch = sql[i]
        nxt = sql[i + 1] if i + 1 < len(sql) else ""
        if in_string:
            buf.append(ch)
            if ch == "'":
                if nxt == "'":
                    buf.append(nxt)
                    i += 1
                else:
                    in_string = False
            i += 1
            continue
        if ch == "'":
            in_string = True
            buf.append(ch)
            i += 1
            continue
        if ch == "-" and nxt == "-":
            while i < len(sql) and sql[i] != "\n":
                i += 1
            continue
        if ch == ";":
            stmt = "".join(buf).strip()
            if stmt:
                statements.append(stmt)
            buf = []
            i += 1
            continue
        buf.append(ch)
        i += 1
    if in_string:
        raise TranslationError("unterminated string literal in migration SQL")
    tail = "".join(buf).strip()
    if tail:
        statements.append(tail)
    return statements


# ---------------------------------------------------------------------
# Translation
# ---------------------------------------------------------------------

_DROP_PATTERNS = (
    re.compile(r"^ALTER\s+TABLE\s+\S+\s+ENABLE\s+ROW\s+LEVEL\s+SECURITY$", re.I),
    re.compile(r"^GRANT\s+", re.I),
    re.compile(r"^COMMENT\s+ON\s+", re.I),
)

_JSONB_TYPEOF = re.compile(
    r"jsonb_typeof\(\s*([A-Za-z_][A-Za-z0-9_.]*)\s*"
    r"(?:->\s*'([^']+)')?\s*(?:::jsonb)?\s*\)\s*=\s*'(object|number|boolean)'"
)
_JSON_ARROW_BOOL = re.compile(
    r"([A-Za-z_][A-Za-z0-9_.]*)\s*->\s*'([^']+)'\s*=\s*'(true|false)'::jsonb"
)
_JSON_ARROW_TEXT = re.compile(r"([A-Za-z_][A-Za-z0-9_.]*)\s*->>\s*'([^']+)'")
_LEFTOVER = re.compile(
    r"::|->|jsonb|timestamptz|btrim|\bpublic\.|~ '|ROW LEVEL|service_role",
    re.I,
)


def _translate_jsonb_typeof(stmt: str) -> str:
    def repl(match: re.Match) -> str:
        column, key, pg_type = match.group(1), match.group(2), match.group(3)
        if key is not None:
            probe = f"json_type({column}, '$.{key}')"
        else:
            probe = f"json_type({column})"
        if pg_type == "object":
            return f"{probe} = 'object'"
        if pg_type == "number":
            # SQLite splits PostgreSQL's one number type in two.
            return f"{probe} IN ('integer', 'real')"
        return f"{probe} IN ('true', 'false')"  # boolean

    return _JSONB_TYPEOF.sub(repl, stmt)


def _translate_regex_checks(stmt: str) -> str:
    # Anchored hex digest: ^[0-9a-f]{64}$ — length plus charset is the
    # same predicate, and GLOB has no {n} repetition.
    for column in ("prev_hash", "hash"):
        stmt = stmt.replace(
            f"{column} ~ '^[0-9a-f]{{64}}$'",
            f"(length({column}) = 64 AND {column} NOT GLOB '*[^0-9a-f]*')",
        )
    # ^[0-9]+$ over the extracted version text.
    version_probe = "json_extract(document, '$.version')"
    stmt = stmt.replace(
        f"{version_probe} ~ '^[0-9]+$'",
        f"(length(CAST({version_probe} AS TEXT)) >= 1"
        f" AND CAST({version_probe} AS TEXT) NOT GLOB '*[^0-9]*')",
    )
    # ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+#[0-9]+$ — owner/repo#number with
    # every segment non-empty, exactly one '/' before exactly one '#',
    # and a digits-only tail. GLOB cannot count or repeat, so the
    # shape is spelled out with instr/substr; the functional tests pin
    # each rejecting case (missing/extra separators, empty segments,
    # bad characters, non-digit tail).
    tail = (
        "substr(github_ref, instr(github_ref, '/')"
        " + instr(substr(github_ref, instr(github_ref, '/') + 1), '#') + 1)"
    )
    github_ref_sql = (
        "(github_ref NOT GLOB '*[^A-Za-z0-9._/#-]*'"
        " AND github_ref NOT GLOB '*/*/*'"
        " AND github_ref NOT GLOB '*#*#*'"
        " AND instr(github_ref, '/') > 1"
        " AND instr(substr(github_ref, instr(github_ref, '/') + 1), '#') > 1"
        f" AND length({tail}) >= 1"
        f" AND {tail} NOT GLOB '*[^0-9]*')"
    )
    stmt = stmt.replace(
        "github_ref ~ '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+#[0-9]+$'",
        github_ref_sql,
    )
    return stmt


def translate_statement(stmt: str) -> str | None:
    """Translate one PostgreSQL statement, or None if it is dropped."""
    flat = " ".join(stmt.split())
    if any(pattern.search(flat) for pattern in _DROP_PATTERNS):
        return None
    out = flat
    out = _translate_jsonb_typeof(out)
    out = _JSON_ARROW_BOOL.sub(
        lambda m: f"json_extract({m.group(1)}, '$.{m.group(2)}')"
        f" = {1 if m.group(3) == 'true' else 0}",
        out,
    )
    out = _JSON_ARROW_TEXT.sub(
        lambda m: f"json_extract({m.group(1)}, '$.{m.group(2)}')", out
    )
    out = _translate_regex_checks(out)
    out = out.replace(
        "(json_extract(document, '$.version'))::integer",
        "CAST(json_extract(document, '$.version') AS INTEGER)",
    )
    out = re.sub(r"::jsonb\b", "", out)
    out = re.sub(r"::timestamptz\b", "", out)
    out = re.sub(r"\bdouble\s+precision\b", "REAL", out, flags=re.I)
    out = re.sub(r"\btimestamptz\b", "TEXT", out, flags=re.I)
    out = re.sub(r"\bjsonb\b", "TEXT", out, flags=re.I)
    out = re.sub(r"\bsmallint\b", "INTEGER", out, flags=re.I)
    out = re.sub(r"\bboolean\b", "INTEGER", out, flags=re.I)
    out = re.sub(r"\btext\[\]", "TEXT", out, flags=re.I)
    out = re.sub(
        r"\bDEFAULT\s+now\(\)", f"DEFAULT ({NOW_SQL})", out, flags=re.I
    )
    out = re.sub(r"\bnow\(\)", NOW_SQL, out, flags=re.I)
    out = re.sub(r"\bDEFAULT\s+false\b", "DEFAULT 0", out, flags=re.I)
    out = re.sub(r"\bDEFAULT\s+true\b", "DEFAULT 1", out, flags=re.I)
    # The one text[] column (agg_activity_ingestion.seen_ids) holds a
    # JSON array in the Turso bridge, whose empty literal is '[]'.
    out = out.replace("DEFAULT '{}'", "DEFAULT '[]'")
    out = re.sub(r",\s*false\)", ", 0)", out)
    out = re.sub(r"\bpublic\.", "", out)
    out = re.sub(r"\bbtrim\(", "trim(", out)
    leftover = _LEFTOVER.search(out)
    if leftover:
        raise TranslationError(
            f"untranslated PostgreSQL construct {leftover.group(0)!r} "
            f"in statement: {flat[:120]}"
        )
    return out


def translate(sql: str) -> list[str]:
    """Translate a whole migration file to SQLite statements."""
    translated: list[str] = []
    for stmt in split_statements(sql):
        result = translate_statement(stmt)
        if result is not None:
            translated.append(result)
    return translated


def load_migrations(migrations_dir: Path = MIGRATIONS_DIR) -> list[tuple[str, str]]:
    """The migration set in version order: (migration id, SQL text)."""
    files = sorted(migrations_dir.glob("*.up.sql"))
    if not files:
        raise StoreError(f"no *.up.sql migrations in {migrations_dir}")
    return [(path.name[: -len(".up.sql")], path.read_text()) for path in files]


# ---------------------------------------------------------------------
# Store clients
# ---------------------------------------------------------------------


class LocalSqliteClient:
    """A store that is a local SQLite file (stand-in and tests)."""

    def __init__(self, path: str):
        self.db = sqlite3.connect(path)
        self.db.execute("PRAGMA foreign_keys = ON")

    def query(self, sql: str) -> list[dict]:
        cursor = self.db.execute(sql)
        columns = [d[0] for d in cursor.description or []]
        return [dict(zip(columns, row)) for row in cursor.fetchall()]

    def batch(self, statements: list[str], transaction: bool = False) -> None:
        try:
            if transaction:
                self.db.execute("BEGIN IMMEDIATE")
            for sql in statements:
                self.db.execute(sql)
            self.db.commit()
        except sqlite3.Error as exc:
            self.db.rollback()
            raise StoreError(f"sqlite statement failed: {exc}") from None


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def _hrana_argument(value):
    if value is None:
        return {"type": "null"}
    if isinstance(value, bool):
        return {"type": "integer", "value": str(int(value))}
    if isinstance(value, int):
        return {"type": "integer", "value": str(value)}
    if isinstance(value, float):
        return {"type": "float", "value": value}
    return {"type": "text", "value": str(value)}


class HttpTursoClient:
    """A Turso store over the Hrana HTTP pipeline.

    Same discipline as scripts/fleet_turso.py: HTTPS only, the origin
    pinned to the store's inventoried address, no redirects (a redirect
    must never carry the bearer token elsewhere), and a batch counts
    as landed only when every step's result is evidenced in the reply.
    """

    def __init__(self, url: str, token: str, expected_host: str):
        cleaned = url.replace("libsql://", "https://").rstrip("/")
        parsed = urlsplit(cleaned)
        if (
            parsed.scheme != "https"
            or parsed.hostname != expected_host
            or parsed.username
            or parsed.password
            or parsed.port not in (None, 443)
            or parsed.path
            or parsed.query
            or parsed.fragment
        ):
            raise StoreError(
                f"store URL must be the inventoried HTTPS origin "
                f"https://{expected_host}"
            )
        if not token:
            raise StoreError("empty store credential")
        self.url = cleaned
        self.token = token

    def _pipeline(self, statements: list[str], transaction: bool) -> list:
        stmts = list(statements)
        if transaction:
            stmts = ["BEGIN IMMEDIATE"] + stmts
        steps = []
        for i, sql in enumerate(stmts):
            step = {"stmt": {"sql": sql, "args": [], "want_rows": True}}
            if i:
                step["condition"] = {"type": "ok", "step": i - 1}
            steps.append(step)
        if transaction:
            last = len(steps) - 1
            steps.append(
                {"condition": {"type": "ok", "step": last}, "stmt": {"sql": "COMMIT"}}
            )
            steps.append(
                {
                    "condition": {
                        "type": "not",
                        "cond": {"type": "ok", "step": last + 1},
                    },
                    "stmt": {"sql": "ROLLBACK"},
                }
            )
        request = urllib.request.Request(
            self.url + "/v2/pipeline",
            data=json.dumps(
                {
                    "requests": [
                        {"type": "batch", "batch": {"steps": steps}},
                        {"type": "close"},
                    ]
                }
            ).encode(),
            headers={
                "Authorization": "Bearer " + self.token,
                "Content-Type": "application/json",
            },
        )
        uncertain = (
            "Turso response incomplete or rejected; outcome unverified, "
            "read back before retrying"
        )
        try:
            opener = urllib.request.build_opener(_NoRedirect())
            with opener.open(request, timeout=120) as response:
                result = json.load(response)["results"][0]
        except urllib.error.HTTPError as exc:
            raise StoreError(
                f"Turso HTTP {exc.code}; outcome unverified, "
                "read back before retrying"
            ) from None
        except (OSError, ValueError, KeyError, IndexError, TypeError):
            raise StoreError(
                "Turso transport/response failure; outcome unverified, "
                "read back before retrying"
            ) from None
        if not isinstance(result, dict) or result.get("type") != "ok":
            raise StoreError(uncertain)
        response = result.get("response")
        if not isinstance(response, dict) or response.get("type") != "batch":
            raise StoreError(uncertain)
        batch = response.get("result")
        if not isinstance(batch, dict):
            raise StoreError(uncertain)
        results, errors = batch.get("step_results"), batch.get("step_errors")
        if (
            not isinstance(results, list)
            or not isinstance(errors, list)
            or len(results) != len(steps)
            or len(errors) != len(steps)
        ):
            raise StoreError(uncertain)
        if any(error is not None for error in errors):
            rolled_back = transaction and errors[-1] is None and results[-1] is not None
            detail = next(
                (e.get("message") for e in errors if isinstance(e, dict)), None
            )
            if rolled_back:
                raise StoreError(
                    "Turso statement failed; rollback confirmed"
                    + (f" (first error: {detail})" if detail else "")
                )
            raise StoreError(uncertain)
        return results

    @staticmethod
    def _rows(result) -> list[dict]:
        if not isinstance(result, dict):
            raise StoreError("Turso response incomplete; outcome unverified")
        cols = [c["name"] for c in result.get("cols", [])]
        rows = []
        for row in result.get("rows", []):
            rows.append(
                {
                    col: (value.get("value") if value.get("type") != "null" else None)
                    for col, value in zip(cols, row)
                }
            )
        return rows

    def query(self, sql: str) -> list[dict]:
        results = self._pipeline([sql], transaction=False)
        return self._rows(results[0])

    def batch(self, statements: list[str], transaction: bool = False) -> None:
        self._pipeline(statements, transaction=transaction)


# ---------------------------------------------------------------------
# Application
# ---------------------------------------------------------------------


def apply_migrations(client, migrations: list[tuple[str, str]]) -> dict:
    """Apply every pending migration, in order, one transaction each.

    Returns the verdict: which migrations were applied by this run,
    which were already applied (the no-op re-run case), and the first
    failure, if any. A failed migration stops the run — later
    migrations may depend on it — and is not recorded in the ledger.
    """
    client.batch(
        [
            f"CREATE TABLE IF NOT EXISTS {LEDGER_TABLE} ("
            "migration_id TEXT PRIMARY KEY, "
            "applied_at TEXT NOT NULL)"
        ]
    )
    applied_rows = client.query(f"SELECT migration_id FROM {LEDGER_TABLE}")
    already = {row["migration_id"] for row in applied_rows}
    verdict = {"applied": [], "already_applied": [], "failed": None}
    for migration_id, sql in migrations:
        if migration_id in already:
            verdict["already_applied"].append(migration_id)
            continue
        try:
            statements = translate(sql)
        except TranslationError as exc:
            verdict["failed"] = {"migration": migration_id, "error": str(exc)}
            break
        ledger_insert = (
            f"INSERT INTO {LEDGER_TABLE} (migration_id, applied_at) "
            f"VALUES ('{migration_id}', {NOW_SQL})"
        )
        try:
            client.batch(statements + [ledger_insert], transaction=True)
        except StoreError as exc:
            verdict["failed"] = {"migration": migration_id, "error": str(exc)}
            break
        verdict["applied"].append(migration_id)
    return verdict
