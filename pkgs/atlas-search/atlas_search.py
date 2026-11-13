#!/usr/bin/env python3
"""atlas-search — the Atlas access layer: hybrid search over the corpus.

Atlas (see README.md in this directory) normalizes every AI-client
conversation into one canonical store (sessions -> messages -> chunks),
classifies every session (scheme-1.0: kind / topics / projects /
sensitivity), embeds every chunk
(Snowflake/snowflake-arctic-embed-m-v2.0 at full 768 dimensions), and
publishes a hot copy to Turso. This CLI is how agents and scripts QUERY that:

  atlas-search search "how do we back up the database" [--filters...]
  atlas-search get-chunk 43001 [--neighbors 2]

Search is the production shape decided 2026-10-31 from the v3
bake-off (docs/embedding-bakeoff-v3-results-2026-10-26.md): one-stage
hybrid — FTS5 (BM25) and vector (Snowflake cosine, full 768) branches
fused with reciprocal rank fusion, k=10, pool 100 per branch (best
measured row: 189 hits@10 / MRR 0.3894). Classification filters are
SQL pre-filters applied to BOTH branches before ranking, so they also
cut the scan. The bge-small set remains in the publish as a dormant
fallback (--model BAAI/bge-small-en-v1.5 --vec-table
vec_bge_small_384_f32 --embedder fastembed).

Privacy model (docs/atlas-provenance-and-scale-design-2026-10-31.md):
sensitivity is a caller ceiling, not a display hint. The default ceiling
is `personal` — local-only and secret-bearing chunks are excluded
unless --max-sensitivity explicitly raises the ceiling, and reaching
secret-bearing requires spelling --max-sensitivity secret-bearing.

Backends:
  --backend turso  libSQL HTTPS pipeline against the published DB
                   (atlas-dezren39.aws-us-east-1.turso.io). Read-only:
                   the token is resolved (in order) from
                   ATLAS_TURSO_TOKEN, a Bitwarden secret named
                   turso.atlas-ro-token / turso.atlas-read-only-token /
                   atlas-ro, or minted read-only via the logged-in
                   `turso` CLI (`turso db tokens create atlas
                   --read-only`, cached 20h in ~/.cache/atlas-search/).
                   NOTE: the Turso Platform API mint endpoint ignores
                   read-only request bodies (verified 2026-10-31: it
                   returns claim a=rw regardless), so minting goes
                   through the CLI, whose tokens carry a=ro.
  --backend local  a local SQLite file: the publish copy (labels +
                   sensitivity + vectors in one file) or canonical
                   atlas.db. Against canonical, labels/sensitivity are
                   attached from a sibling atlas-publish-*.db when one
                   sits in the same directory, and vectors fall back to
                   the known side stores (vec-bge-small.db,
                   vec-bge-small-topup.db, atlas-delta-chunks2.db,
                   vec-snowflake.db; or --vecdb / ATLAS_VEC_DB). Side
                   stores are mapped by CONTENT HASH through their
                   corpus files, never by raw chunk_id: the id spaces
                   differ between stores (verified 2026-10-31 — an
                   id-join would silently attach wrong vectors to
                   ~14k chunks), while content_hash is the identity
                   every store shares. A --vecdb file given explicitly
                   is the one exception: it is assumed keyed by the
                   corpus DB's own chunk ids.
  default          turso when a token resolves, else local.

Output is JSON (an agent consumes it without parsing prose); --text
renders a human-readable form. FTS-only mode and get-chunk need nothing
beyond the stdlib.

Vector model / table are configurable (--model / --vec-table /
--embedder / --trunc, or ATLAS_SEARCH_MODEL / ATLAS_VEC_TABLE).
Defaults are the Snowflake set in the live publish
(vec_snowflake_m_v2_768_f32, typed F32_BLOB(768) with a
libsql_vector_idx index). Snowflake query embedding uses the
sentence-transformers path (prompt_name="query", per eval3.py); bge
uses fastembed with the bge query prefix. Every result carries its
per-branch `rank_sources` and fused `score`.

Turso perf note: the publish schema has no secondary indexes, so a
session-filtered vector query costs ~15 s server-side (the chunk->
message->session join chain over remote storage); unfiltered and
FTS queries are ~0.5-4 s. The branch query scans with
vector_distance_cos so the sensitivity pre-filter applies before
ranking; vector_top_k against the Snowflake index is available for
unfiltered nearest-neighbor lookups.
--vector-backend index-fixed / index-fill instead route the Turso
vector branch through that index and apply the same filters AFTER
retrieval (fixed: a single pool-depth lookup, survivors kept; fill:
escalating depth until the pool fills or a 3200 cap is hit). Opt-in
A/B modes; scan remains the default.
"""

import argparse
import base64
import json
import os
import re
import sqlite3
import subprocess
import sys
import time
import urllib.error
import urllib.request
from datetime import date, timedelta

# ------------------------------------------------------------------ config
TURSO_HOST = "atlas-dezren39.aws-us-east-1.turso.io"
TURSO_ORG = "dezren39"
TURSO_DB = "atlas"
MODEL = "Snowflake/snowflake-arctic-embed-m-v2.0"
QUERY_PREFIX = "Represent this sentence for searching relevant passages: "
VEC_TABLE = "vec_snowflake_m_v2_768_f32"
RRF_K = 10
SENS_ORDER = ["shareable", "personal", "local-only", "secret-bearing"]
BWS_TOKEN_NAMES = ["turso.atlas-ro-token", "turso.atlas-read-only-token", "atlas-ro"]
TOKEN_CACHE = os.path.expanduser("~/.cache/atlas-search/turso-token.json")
MESSAGE_TEXT_CAP = 50000

for _k in ("NO_PROXY", "no_proxy"):
    if "[" in os.environ.get(_k, ""):
        os.environ[_k] = "localhost,127.0.0.1"


def die(msg, code=2):
    print(f"atlas-search: {msg}", file=sys.stderr)
    sys.exit(code)


# ------------------------------------------------------- secrets (bws) ----
# Pattern copied from atlas-backup.py: secrets are fetched from Bitwarden
# Secrets Manager BY NAME at runtime; values are never printed or logged.
_bws_cache = {}


def bws_secret(name):
    if name in _bws_cache:
        return _bws_cache[name]
    out = subprocess.run(
        [
            "bash",
            "-c",
            'source ~/tools/bws-env.sh >/dev/null 2>&1; printf %s "$BWS_ACCESS_TOKEN"',
        ],
        capture_output=True,
        text=True,
        timeout=30,
    )
    env_token = out.stdout.strip()
    if env_token:
        os.environ["BWS_ACCESS_TOKEN"] = env_token
    bws = os.path.expanduser("~/tools/bin/bws")
    if not os.path.exists(bws):
        raise RuntimeError("bws CLI not found")
    last = None
    for _ in range(3):
        try:
            lst = subprocess.run(
                [bws, "secret", "list", "-o", "json"],
                capture_output=True,
                text=True,
                timeout=60,
            )
            sid = None
            for item in json.loads(lst.stdout):
                if item.get("key") == name:
                    sid = item["id"]
                    break
            if not sid:
                raise RuntimeError(f"bws secret named {name} not found")
            got = subprocess.run(
                [bws, "secret", "get", sid, "-o", "json"],
                capture_output=True,
                text=True,
                timeout=60,
            )
            val = json.loads(got.stdout)
            _bws_cache[name] = (
                val[0]["value"] if isinstance(val, list) else val["value"]
            )
            return _bws_cache[name]
        except Exception as e:  # noqa: BLE001 - report the last failure
            last = e
    raise RuntimeError(f"bws fetch failed for {name}: {last}")


def resolve_turso_token():
    """Return (token, source) or (None, reason). Never prints the token."""
    env = os.environ.get("ATLAS_TURSO_TOKEN")
    if env:
        return env, "env ATLAS_TURSO_TOKEN"
    for name in BWS_TOKEN_NAMES:
        try:
            return bws_secret(name), f"bitwarden:{name}"
        except Exception:
            continue
    # Cached read-only token minted via the turso CLI.
    try:
        with open(TOKEN_CACHE) as f:
            cache = json.load(f)
        if (
            cache.get("db") == TURSO_DB
            and time.time() - cache.get("fetched_at", 0) < 20 * 3600
        ):
            return cache["token"], "cache (turso CLI read-only mint)"
    except Exception:
        pass
    # Mint read-only via the turso CLI (its tokens carry claim a=ro; the
    # Platform API mint endpoint ignores read-only bodies — see docstring).
    try:
        out = subprocess.run(
            ["turso", "db", "tokens", "create", TURSO_DB, "--read-only", "-e", "7d"],
            capture_output=True,
            text=True,
            timeout=60,
        )
        token = out.stdout.strip()
        if out.returncode == 0 and token.count(".") == 2:
            try:
                os.makedirs(os.path.dirname(TOKEN_CACHE), exist_ok=True)
                fd = os.open(TOKEN_CACHE, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
                with os.fdopen(fd, "w") as f:
                    json.dump(
                        {"db": TURSO_DB, "token": token, "fetched_at": time.time()}, f
                    )
            except Exception:
                pass
            return token, "turso CLI read-only mint"
    except Exception:
        pass
    return (
        None,
        "no ATLAS_TURSO_TOKEN, no Bitwarden atlas-ro secret, turso CLI mint unavailable",
    )


def turso_cli_logged_in():
    try:
        out = subprocess.run(
            ["turso", "auth", "whoami"], capture_output=True, text=True, timeout=30
        )
        return out.returncode == 0 and bool(out.stdout.strip())
    except Exception:
        return False


# ------------------------------------------------------------- turso ------
def _hrana_arg(x):
    if x is None:
        return {"type": "null"}
    if isinstance(x, bool):
        return {"type": "integer", "value": str(int(x))}
    if isinstance(x, int):
        return {"type": "integer", "value": str(x)}
    if isinstance(x, float):
        return {"type": "float", "value": x}
    if isinstance(x, (bytes, bytearray)):
        return {"type": "blob", "value": base64.b64encode(bytes(x)).decode()}
    return {"type": "text", "value": str(x)}


def _hrana_val(v):
    if v is None:
        return None
    t = v.get("type")
    if t == "integer":
        return int(v["value"])
    if t == "float":
        return float(v["value"])
    if t == "text":
        return v["value"]
    if t == "blob":
        return base64.b64decode(v["value"])
    return None


class TursoBackend:
    name = "turso"
    sens_expr = "c.sensitivity"
    labels_table = "session_labels"
    sens_join = ""

    def __init__(self, vec_table=VEC_TABLE):
        token, source = resolve_turso_token()
        if not token:
            die(
                f"turso backend: token did not resolve ({source}). "
                "Set ATLAS_TURSO_TOKEN or log the turso CLI in."
            )
        self.token = token
        self.token_source = source
        self.vec_table = vec_table

    def pipeline(self, stmts):
        """stmts: list of (sql, [args]). Returns list of list-of-dict rows."""
        requests = [
            {
                "type": "execute",
                "stmt": {"sql": sql, "args": [_hrana_arg(a) for a in args]},
            }
            for sql, args in stmts
        ]
        requests.append({"type": "close"})
        body = json.dumps({"requests": requests}).encode()
        r = urllib.request.Request(
            f"https://{TURSO_HOST}/v2/pipeline", data=body, method="POST"
        )
        r.add_header("Authorization", f"Bearer {self.token}")
        r.add_header("Content-Type", "application/json")
        try:
            with urllib.request.urlopen(r, timeout=300) as resp:
                out = json.loads(resp.read().decode())
        except urllib.error.HTTPError as e:
            die(f"turso backend: HTTP {e.code}: {e.read().decode()[:200]}")
        results = []
        for res in out.get("results", []):
            if res.get("type") == "error":
                die(f"turso backend: SQL error: {res['error'].get('message')}")
            result = res.get("response", {}).get("result")
            if result is None:
                continue
            cols = [c["name"] for c in result.get("cols", [])]
            rows = [
                {cols[i]: _hrana_val(v) for i, v in enumerate(row)}
                for row in result.get("rows", [])
            ]
            results.append(rows)
        return results

    def execute(self, sql, args=()):
        return self.pipeline([(sql, list(args))])[0]


# ------------------------------------------------------------- local ------
class LocalBackend:
    name = "local"
    sens_expr = "cs.sensitivity"
    labels_table = "v_labels"
    sens_join = "LEFT JOIN v_chunk_sens cs ON cs.id = c.id"

    def __init__(self, db_path, vecdb_arg=None, vec_table=VEC_TABLE, trunc=0):
        if not db_path or not os.path.exists(db_path):
            die(f"local backend: DB not found: {db_path}")
        self.db_path = db_path
        self.vec_table = vec_table
        self.trunc = trunc
        self.con = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
        self.con.row_factory = sqlite3.Row
        self.notes = []
        self._setup_views()
        self._vec_sources = self._find_vec_sources(vecdb_arg)
        self._matrix = None  # (ids list, numpy matrix, id->row index)

    # -- schema normalization: temp views give the queries one shape
    #    whether the file is a publish copy (labels+sensitivity+vecs) or
    #    canonical atlas.db (none of those; attached from siblings).
    def _tables(self, schema="main"):
        return {
            r[0]
            for r in self.con.execute(
                f"SELECT name FROM {schema}.sqlite_master WHERE type IN ('table','view')"
            )
        }

    def _cols(self, table, schema="main"):
        return {r[1] for r in self.con.execute(f"PRAGMA {schema}.table_info({table})")}

    def _setup_views(self):
        main_tables = self._tables()
        aux = None
        if "session_labels" not in main_tables or "sensitivity" not in self._cols(
            "chunks"
        ):
            # Look for a publish copy beside the DB to attach for
            # labels + sensitivity (same session/chunk id space).
            import glob

            sibs = sorted(
                glob.glob(
                    os.path.join(
                        os.path.dirname(os.path.abspath(self.db_path)),
                        "atlas-publish-*.db",
                    )
                ),
                key=lambda p: os.path.getmtime(p),
                reverse=True,
            )
            for sib in sibs:
                if os.path.abspath(sib) == os.path.abspath(self.db_path):
                    continue
                try:
                    self.con.execute(
                        "ATTACH DATABASE ? AS aux", (f"file:{sib}?mode=ro",)
                    )
                    aux = sib
                    break
                except sqlite3.Error:
                    continue
            if aux:
                self.notes.append(
                    f"labels/sensitivity attached from sibling publish copy {os.path.basename(aux)}"
                )
        aux_tables = self._tables("aux") if aux else set()
        if "session_labels" in main_tables:
            self.con.execute(
                "CREATE TEMP VIEW v_labels AS SELECT * FROM main.session_labels"
            )
            self.sens_source = "session_labels (this DB)"
        elif "session_labels" in aux_tables:
            self.con.execute(
                "CREATE TEMP VIEW v_labels AS SELECT * FROM aux.session_labels"
            )
            self.sens_source = "session_labels (attached publish copy)"
        else:
            self.con.execute(
                "CREATE TEMP TABLE v_labels (session_id TEXT PRIMARY KEY, client TEXT,"
                " kind TEXT, topics TEXT, projects TEXT, sensitivity TEXT,"
                " confidence TEXT, scheme TEXT)"
            )
            self.sens_source = "none — this DB carries no labels"
            self.notes.append(
                "no session_labels available: kind/topic/project filters match nothing"
            )
        if "sensitivity" in self._cols("chunks"):
            self.con.execute(
                "CREATE TEMP VIEW v_chunk_sens AS SELECT id, sensitivity FROM main.chunks"
            )
            self.chunk_sens_source = "chunks.sensitivity (this DB)"
        elif aux and "sensitivity" in self._cols("chunks", "aux"):
            self.con.execute(
                "CREATE TEMP VIEW v_chunk_sens AS SELECT id, sensitivity FROM aux.chunks"
            )
            self.chunk_sens_source = "chunks.sensitivity (attached publish copy)"
        else:
            self.con.execute(
                "CREATE TEMP TABLE v_chunk_sens (id INTEGER PRIMARY KEY, sensitivity TEXT)"
            )
            self.chunk_sens_source = "none — no sensitivity data available"
            self.notes.append(
                "WARNING: no sensitivity data in this DB and no publish copy beside it; "
                "the sensitivity ceiling excludes every chunk (fail-closed). "
                "Point --db at a publish copy to search this corpus."
            )

    # (vec file, corpus file whose chunks(id, content_hash) maps its ids)
    KNOWN_VEC_PAIRS = (
        ("~/atlas/v3/vec-bge-small.db", "~/atlas/v3/corpus.db"),
        ("~/atlas/v3/vec-bge-small-topup.db", "~/atlas/v3/topup-chunks.db"),
        ("~/atlas/atlas-delta-chunks2.db", "~/atlas/atlas-delta-chunks2.db"),
        ("~/atlas/v3/vec-snowflake.db", "~/atlas/v3/corpus.db"),
    )

    def _find_vec_sources(self, vecdb_arg):
        """Vector sources as descriptors:
        ("main",)                 — vec table inside the corpus DB itself
        ("idkey", path)           — explicit --vecdb, keyed by corpus ids
        ("mapped", vecpath, mappath) — known side store, content-hash mapped
        """
        if self.vec_table in self._tables():
            return [("main",)]
        env = vecdb_arg or os.environ.get("ATLAS_VEC_DB")
        if env:
            return [("idkey", p.strip()) for p in env.split(",") if p.strip()]
        out = []
        seen_vec = set()
        for vec_rel, map_rel in self.KNOWN_VEC_PAIRS:
            vec_p = os.path.expanduser(vec_rel)
            map_p = os.path.expanduser(map_rel)
            if (
                vec_p in seen_vec
                or not os.path.exists(vec_p)
                or not os.path.exists(map_p)
            ):
                continue
            probe = sqlite3.connect(f"file:{vec_p}?mode=ro", uri=True)
            try:
                tabs = {
                    r[0]
                    for r in probe.execute(
                        "SELECT name FROM sqlite_master WHERE type IN ('table','view')"
                    )
                }
            finally:
                probe.close()
            if self.vec_table not in tabs:
                continue
            seen_vec.add(vec_p)
            out.append(("mapped", vec_p, map_p))
        if out:
            self.notes.append(
                f"vectors: {len(out)} side store(s) mapped by content_hash "
                "(id spaces differ between stores; content_hash is the identity)"
            )
        return out

    def execute(self, sql, args=()):
        return [dict(r) for r in self.con.execute(sql, list(args)).fetchall()]

    # -- vector branch: numpy scan over the filter-allowed id set
    #    (stdlib sqlite3 has no vector_distance_cos; eval3.py pattern).
    def load_matrix(self):
        if self._matrix is not None:
            return self._matrix
        import numpy as np

        vec_by_id = {}
        total_chunks = self.con.execute("SELECT count(*) FROM chunks").fetchone()[0]
        mapped_used = False
        for i, src in enumerate(self._vec_sources):
            kind = src[0]
            if kind == "main":
                rows = self.con.execute(
                    f"SELECT chunk_id, vec FROM main.{self.vec_table}"
                )
                for cid, blob in rows:
                    vec_by_id.setdefault(cid, blob)
            elif kind == "idkey":
                schema = f"vx{i}"
                self.con.execute(
                    f"ATTACH DATABASE ? AS {schema}", (f"file:{src[1]}?mode=ro",)
                )
                if self.vec_table not in self._tables(schema):
                    continue
                for cid, blob in self.con.execute(
                    f"SELECT chunk_id, vec FROM {schema}.{self.vec_table}"
                ):
                    vec_by_id.setdefault(cid, blob)
            elif kind == "mapped":
                mapped_used = True
                _, vec_p, map_p = src
                vschema, mschema = f"vm{i}", f"mm{i}"
                self.con.execute(
                    f"ATTACH DATABASE ? AS {vschema}", (f"file:{vec_p}?mode=ro",)
                )
                if map_p != vec_p:
                    self.con.execute(
                        f"ATTACH DATABASE ? AS {mschema}", (f"file:{map_p}?mode=ro",)
                    )
                else:
                    mschema = vschema
                if self.vec_table not in self._tables(vschema):
                    continue
                id2hash = {
                    r[0]: r[1]
                    for r in self.con.execute(
                        f"SELECT id, content_hash FROM {mschema}.chunks"
                    )
                }
                hash2main = {
                    r[1]: r[0]
                    for r in self.con.execute(
                        "SELECT id, content_hash FROM main.chunks"
                    )
                }
                for cid, blob in self.con.execute(
                    f"SELECT chunk_id, vec FROM {vschema}.{self.vec_table}"
                ):
                    mid = hash2main.get(id2hash.get(cid))
                    if mid is not None:
                        vec_by_id.setdefault(mid, blob)
        if mapped_used:
            self.notes.append(
                f"vector coverage: {len(vec_by_id)} of {total_chunks} chunks "
                "have vectors via content-hash mapping"
            )
        ids = sorted(vec_by_id)
        if not ids:
            self._matrix = ([], None, {})
            return self._matrix
        vecs = []
        for i in ids:
            v = np.frombuffer(vec_by_id[i], dtype=np.float32)
            vecs.append(v[: self.trunc] if self.trunc else v)
        M = np.stack(vecs)
        M = M / np.linalg.norm(M, axis=1, keepdims=True)
        self._matrix = (ids, M, {cid: i for i, cid in enumerate(ids)})
        return self._matrix

    def vector_search(self, qvec, allowed_ids, pool):
        import numpy as np

        ids, M, index = self.load_matrix()
        if M is None:
            return []
        if allowed_ids is not None:
            rows = [index[i] for i in ids if i in allowed_ids]
            if not rows:
                return []
            sub = M[rows]
            sims = sub @ qvec
            order = np.argsort(-sims)[:pool]
            return [(ids[rows[j]], float(sims[j])) for j in order]
        sims = M @ qvec
        order = np.argsort(-sims)[:pool]
        return [(ids[j], float(sims[j])) for j in order]


# ------------------------------------------------------------ filters -----
class Filters:
    def __init__(self, args):
        self.kinds = split_csv(args.kind)
        self.topics = split_csv(args.topic)
        self.projects = split_csv(args.project)
        self.clients = split_csv(args.client)
        if args.max_sensitivity not in SENS_ORDER:
            die(f"--max-sensitivity must be one of {SENS_ORDER}")
        self.max_sensitivity = args.max_sensitivity
        self.allowed_sens = SENS_ORDER[: SENS_ORDER.index(args.max_sensitivity) + 1]
        self.since = args.since
        self.until = None
        if args.until:
            try:
                d = date.fromisoformat(args.until)
            except ValueError:
                die(f"--until must be YYYY-MM-DD, got {args.until}")
            self.until_exclusive = (d + timedelta(days=1)).isoformat()
            self.until = args.until
        else:
            self.until_exclusive = None
        if args.since:
            try:
                date.fromisoformat(args.since)
            except ValueError:
                die(f"--since must be YYYY-MM-DD, got {args.since}")

    def where(self, be):
        """SQL fragment + params, assuming aliases c/m/s/l and the
        backend's sensitivity expression. Sensitivity is ALWAYS applied
        (the ceiling); NULL sensitivity never passes (fail-closed)."""
        clauses, params = [], []
        sens_ph = ",".join("?" * len(self.allowed_sens))
        clauses.append(f"{be.sens_expr} IN ({sens_ph})")
        params.extend(self.allowed_sens)
        for col, vals in (("l.kind", self.kinds), ("s.client", self.clients)):
            if vals:
                clauses.append(f"{col} IN ({','.join('?' * len(vals))})")
                params.extend(vals)
        for col, vals in (("l.topics", self.topics), ("l.projects", self.projects)):
            for v in vals:
                clauses.append(f"{col} LIKE ?")
                params.append(f'%"{v}"%')
        if self.since:
            clauses.append("s.created_at >= ?")
            params.append(self.since)
        if self.until_exclusive:
            clauses.append("s.created_at < ?")
            params.append(self.until_exclusive)
        return " AND ".join(clauses), params

    def session_clauses(self, s_alias="s", l_alias="l"):
        """Session-level filters only (kind/topic/project/client/dates) —
        everything except the sensitivity ceiling — with caller-chosen
        table aliases, for the Turso vector branch's IN-subquery."""
        clauses, params = [], []
        for col, vals in (
            (f"{l_alias}.kind", self.kinds),
            (f"{s_alias}.client", self.clients),
        ):
            if vals:
                clauses.append(f"{col} IN ({','.join('?' * len(vals))})")
                params.extend(vals)
        for col, vals in (
            (f"{l_alias}.topics", self.topics),
            (f"{l_alias}.projects", self.projects),
        ):
            for v in vals:
                clauses.append(f"{col} LIKE ?")
                params.append(f'%"{v}"%')
        if self.since:
            clauses.append(f"{s_alias}.created_at >= ?")
            params.append(self.since)
        if self.until_exclusive:
            clauses.append(f"{s_alias}.created_at < ?")
            params.append(self.until_exclusive)
        return clauses, params

    def joins(self, be):
        return (
            f"JOIN messages m ON m.id = c.message_id "
            f"JOIN sessions s ON s.id = m.session_id "
            f"LEFT JOIN {be.labels_table} l ON l.session_id = s.id "
            f"{be.sens_join}"
        )

    def describe(self):
        d = {"max_sensitivity": self.max_sensitivity}
        if self.kinds:
            d["kind"] = self.kinds
        if self.topics:
            d["topic"] = self.topics
        if self.projects:
            d["project"] = self.projects
        if self.clients:
            d["client"] = self.clients
        if self.since:
            d["since"] = self.since
        if self.until:
            d["until"] = self.until
        return d


def split_csv(v):
    if not v:
        return []
    return [x.strip() for x in v.split(",") if x.strip()]


# ------------------------------------------------------------ embedding ---
_model = None


def embed_query(text, model=MODEL, embedder="auto", trunc=0):
    """Query embedding for the configured model.

    fastembed path (bge): the bge query prefix from eval3.py.
    sentence-transformers path (Snowflake et al.): prompt_name="query"
    so the model's own shipped query prompt is applied (eval3.py).
    Truncation (Matryoshka) slices first, then renormalizes — the same
    order the bake-off's trunc patch applies to queries."""
    global _model
    import numpy as np

    framework = embedder
    if framework == "auto":
        framework = "fastembed" if "bge" in model.lower() else "st"
    key = (framework, model)
    if _model is None or _model[0] != key:
        if framework == "fastembed":
            try:
                from fastembed import TextEmbedding
            except ImportError:
                die(
                    "vector/hybrid modes with a fastembed model need the fastembed "
                    "package. Use --mode fts, or run with the atlas venv python "
                    "(~/atlas/.venv on the laptop)."
                )
            _model = (key, TextEmbedding(model_name=model))
        elif framework == "st":
            try:
                from sentence_transformers import SentenceTransformer
            except ImportError:
                die(
                    "vector/hybrid modes with a sentence-transformers model need the "
                    "sentence_transformers package (~/atlas/v3-venv on the laptop)."
                )
            _model = (key, SentenceTransformer(model))
        else:
            die(f"unknown embedder: {embedder}")
    m = _model[1]
    if framework == "fastembed":
        qs = [(QUERY_PREFIX + text) if "bge" in model.lower() else text]
        vec = next(iter(m.embed(qs)))
        v = np.asarray(vec, dtype=np.float32)
    else:
        v = m.encode(
            [text], prompt_name="query", convert_to_numpy=True, show_progress_bar=False
        )[0].astype(np.float32)
    if trunc:
        v = v[:trunc]
    n = float(np.linalg.norm(v))
    if n == 0:
        die("query embedding produced a zero vector")
    return v / n


def fts_match_expr(query):
    """eval3.py hybrid-branch construction: content words OR-joined as
    quoted phrases. Words >3 chars, plus short tokens containing a digit
    (r2, s3, b2) which the >3 rule would otherwise silently drop."""
    words = []
    for w in re.findall(r"[a-z0-9]+", query.lower()):
        if len(w) > 3 or (len(w) >= 2 and any(ch.isdigit() for ch in w)):
            words.append(w)
    if not words:
        return None
    return " OR ".join(f'"{w}"' for w in words)


# -------------------------------------------------------------- search ----
def branch_queries(be, filters, match_expr, qvec_json, pool):
    """Turso branch statements. The FTS branch joins the full chain
    (cheap: the FTS index anchors it to a few hundred rows first). The
    vector branch must NOT join messages/sessions/labels per row — on
    Turso's remote storage that chain costs 20-60 s (no secondary
    indexes; measured 2026-10-31). Instead: the sensitivity ceiling
    rides the chunks join (chunks.sensitivity is denormalized in the
    publish schema), and session-level filters ride an IN-subquery,
    which materializes the allowed id set once (~15 s worst case,
    ~3 s with no session filters)."""
    where, params = filters.where(be)
    joins = filters.joins(be)
    stmts = []
    if match_expr is not None:
        stmts.append(
            (
                "SELECT c.id AS cid FROM chunks c "
                "JOIN chunks_fts f ON f.rowid = c.id "
                + joins
                + f" WHERE chunks_fts MATCH ? AND {where} "
                "ORDER BY bm25(chunks_fts) LIMIT ?",
                [match_expr, *params, pool],
            )
        )
    if qvec_json is not None:
        sens_ph = ",".join("?" * len(filters.allowed_sens))
        sql = (
            f"SELECT v.chunk_id AS cid, vector_distance_cos(v.vec, vector32(?)) AS dist "
            f"FROM {be.vec_table} v JOIN chunks c ON c.id = v.chunk_id "
            f"WHERE c.sensitivity IN ({sens_ph})"
        )
        vparams = [qvec_json, *filters.allowed_sens]
        sess_clauses, sess_params = filters.session_clauses("s2", "l2")
        if sess_clauses:
            sql += (
                " AND v.chunk_id IN (SELECT c2.id FROM chunks c2 "
                "JOIN messages m2 ON m2.id = c2.message_id "
                "JOIN sessions s2 ON s2.id = m2.session_id "
                "LEFT JOIN session_labels l2 ON l2.session_id = s2.id "
                f"WHERE {' AND '.join(sess_clauses)})"
            )
            vparams.extend(sess_params)
        sql += " ORDER BY dist ASC LIMIT ?"
        vparams.append(pool)
        stmts.append((sql, vparams))
    return stmts


def turso_allowed_ids(be, filters):
    """The exact chunk-id set the scan branch would rank: the
    sensitivity ceiling (denormalized on chunks in the publish
    schema) intersected with the session-level filter set when any
    session filter is set - the same decomposition branch_queries
    uses, materialized as ids so the index arms can post-filter."""
    sens_ph = ",".join("?" * len(filters.allowed_sens))
    rows = be.execute(
        f"SELECT c.id AS cid FROM chunks c WHERE c.sensitivity IN ({sens_ph})",
        list(filters.allowed_sens),
    )
    allowed = {r["cid"] for r in rows}
    sess_clauses, sess_params = filters.session_clauses("s2", "l2")
    if sess_clauses:
        rows = be.execute(
            "SELECT c2.id AS cid FROM chunks c2 "
            "JOIN messages m2 ON m2.id = c2.message_id "
            "JOIN sessions s2 ON s2.id = m2.session_id "
            "LEFT JOIN session_labels l2 ON l2.session_id = s2.id "
            f"WHERE {' AND '.join(sess_clauses)}",
            sess_params,
        )
        allowed &= {r["cid"] for r in rows}
    return allowed


def turso_index_branch(be, filters, qvec_json, pool, vbackend):
    """vector_top_k-backed vector branch (turso only): the ANN index
    retrieves candidates WITHOUT the filters; the standard filter set
    is applied afterwards against the materialized allowed set.
    index-fixed takes a single pool-depth lookup and keeps whatever
    survives (the naive post-filter shape); index-fill escalates the
    retrieval depth (pool, 2x, 4x, ... to a 3200 cap) until the pool
    fills with survivors. Survivors keep index-returned order;
    distances are computed exactly afterwards for the final list only
    (vector_top_k returns ids, no distances)."""
    allowed = turso_allowed_ids(be, filters)
    if vbackend == "index-fixed":
        depths = [pool]
    else:  # index-fill
        depths = []
        d = pool
        while d < 3200:
            depths.append(d)
            d *= 2
        depths.append(3200)
    survivors = []
    for d in depths:
        rows = be.execute(
            "SELECT id FROM vector_top_k(?, vector32(?), ?)",
            [f"{be.vec_table}_idx", qvec_json, d],
        )
        survivors = [r["id"] for r in rows if r["id"] in allowed][:pool]
        if len(survivors) >= pool:
            break
    if not survivors:
        return []
    ph = ",".join("?" * len(survivors))
    drows = be.execute(
        f"SELECT chunk_id AS cid, vector_distance_cos(vec, vector32(?)) AS dist "
        f"FROM {be.vec_table} WHERE chunk_id IN ({ph})",
        [qvec_json, *survivors],
    )
    dist = {r["cid"]: r["dist"] for r in drows}
    return [(cid, dist.get(cid)) for cid in survivors]


def run_search(be, args):
    vbackend = getattr(args, "vector_backend", "scan")
    if (
        args.mode in ("vector", "hybrid")
        and vbackend != "scan"
        and not isinstance(be, TursoBackend)
    ):
        die(
            "--vector-backend index modes need the turso backend "
            "(the vector index lives in the Turso publish)"
        )
    filters = Filters(args)
    match_expr = fts_match_expr(args.query) if args.mode in ("fts", "hybrid") else None
    if args.mode == "fts" and match_expr is None:
        die(
            "query has no FTS-searchable terms (all words too short); try --mode vector"
        )
    qvec = None
    qvec_json = None
    if args.mode in ("vector", "hybrid"):
        qvec = embed_query(
            args.query, model=args.model, embedder=args.embedder, trunc=args.trunc
        )
        qvec_json = json.dumps([float(x) for x in qvec])

    fts_ranked, vec_ranked = [], []  # [(chunk_id, raw)] in rank order
    notes = list(getattr(be, "notes", []))
    if (
        isinstance(be, LocalBackend)
        and args.mode in ("vector", "hybrid")
        and not be._vec_sources
    ):
        notes.append(
            "no vector store found for the local backend; vector branch returned nothing"
        )
    if isinstance(be, TursoBackend):
        if vbackend == "scan" or qvec_json is None:
            stmts = branch_queries(be, filters, match_expr, qvec_json, args.pool)
            results = be.pipeline(stmts)
            i = 0
            if match_expr is not None:
                fts_ranked = [(r["cid"], None) for r in results[i]]
                i += 1
            if qvec_json is not None:
                vec_ranked = [(r["cid"], r["dist"]) for r in results[i]]
        else:
            fts_stmts = branch_queries(be, filters, match_expr, None, args.pool)
            if fts_stmts:
                fts_ranked = [(r["cid"], None) for r in be.pipeline(fts_stmts)[0]]
            vec_ranked = turso_index_branch(be, filters, qvec_json, args.pool, vbackend)
    else:
        where, params = filters.where(be)
        joins = filters.joins(be)
        if match_expr is not None:
            rows = be.execute(
                "SELECT c.id AS cid FROM chunks c "
                "JOIN chunks_fts f ON f.rowid = c.id "
                + joins
                + f" WHERE chunks_fts MATCH ? AND {where} "
                "ORDER BY bm25(chunks_fts) LIMIT ?",
                [match_expr, *params, args.pool],
            )
            fts_ranked = [(r["cid"], None) for r in rows]
        if qvec is not None:
            allowed = {
                r["cid"]
                for r in be.execute(
                    f"SELECT c.id AS cid FROM chunks c {joins} WHERE {where}", params
                )
            }
            vec_ranked = [
                (cid, 1.0 - sim)
                for cid, sim in be.vector_search(qvec, allowed, args.pool)
            ]

    # Reciprocal rank fusion, k=10 (production shape, decided 2026-10-31).
    fused = {}
    for rank, (cid, _raw) in enumerate(fts_ranked, start=1):
        e = fused.setdefault(cid, {"chunk_id": cid, "score": 0.0, "rank_sources": {}})
        e["score"] += 1.0 / (RRF_K + rank)
        e["rank_sources"]["fts"] = rank
    vec_dist = {}
    for rank, (cid, dist) in enumerate(vec_ranked, start=1):
        e = fused.setdefault(cid, {"chunk_id": cid, "score": 0.0, "rank_sources": {}})
        e["score"] += 1.0 / (RRF_K + rank)
        e["rank_sources"]["vector"] = rank
        vec_dist[cid] = dist
    ranked = sorted(
        fused.values(),
        key=lambda e: (-e["score"], min(e["rank_sources"].values()), e["chunk_id"]),
    )

    # the local backend learns things mid-search (vector coverage);
    # fold any late notes in before rendering
    for n in getattr(be, "notes", []):
        if n not in notes:
            notes.append(n)
    if args.group_by_session:
        out = grouped_output(be, args, filters, ranked, vec_dist, notes)
    else:
        keep = ranked[: args.k]
        details = fetch_details(be, [e["chunk_id"] for e in keep])
        results = []
        for e in keep:
            d = details.get(e["chunk_id"])
            if d is None:
                continue
            results.append(result_entry(be, args, e, d, vec_dist))
        out = envelope(args, be, filters, results, notes)
    if args.mode in ("vector", "hybrid"):
        # Provenance, kept explicit so callers can see exactly which
        # model/table produced the pool.
        out["vector"] = {
            "model": args.model,
            "table": be.vec_table,
            "embedder": args.embedder,
            "trunc": args.trunc or None,
            "pool": args.pool,
            "vector_backend": getattr(args, "vector_backend", "scan"),
        }
    return out


def result_entry(be, args, fused_entry, d, vec_dist):
    cid = fused_entry["chunk_id"]
    entry = {
        "chunk_id": cid,
        "session_id": d["session_id"],
        "score": round(fused_entry["score"], 6),
        "rank_sources": dict(fused_entry["rank_sources"]),
        "text": d["text"],
        "session": session_obj(d),
    }
    if cid in vec_dist and vec_dist[cid] is not None:
        entry["vector_distance"] = round(vec_dist[cid], 6)
    if args.expand:
        entry["context"] = neighbor_map(be, d, args.expand)
    if args.with_message:
        entry["message"] = message_obj(be, d)
    return entry


def session_obj(d):
    return {
        "title": d["s_title"],
        "client": d["s_client"],
        "created_at": d["s_created_at"],
        "kind": d["l_kind"],
        "topics": parse_json_list(d["l_topics"]),
        "projects": parse_json_list(d["l_projects"]),
        "sensitivity": d["eff_sensitivity"],
    }


def parse_json_list(v):
    if not v:
        return []
    try:
        out = json.loads(v)
        return out if isinstance(out, list) else []
    except (ValueError, TypeError):
        return []


def grouped_output(be, args, filters, ranked, vec_dist, notes):
    # Roll chunk hits up to sessions: session score = best chunk score;
    # sessions ordered by (best score, hit count). Consider a generous
    # slice of the fused list so hit counts mean something.
    consideration = ranked[: max(args.k * 10, args.pool)]
    by_session = {}
    details = fetch_details(be, [e["chunk_id"] for e in consideration])
    for e in consideration:
        d = details.get(e["chunk_id"])
        if d is None:
            continue
        g = by_session.setdefault(
            d["session_id"],
            {
                "session_id": d["session_id"],
                "score": e["score"],
                "hit_count": 0,
                "chunks": [],
                "_d": d,
            },
        )
        g["hit_count"] += 1
        g["score"] = max(g["score"], e["score"])
        g["chunks"].append((e, d))
    groups = sorted(by_session.values(), key=lambda g: (-g["score"], -g["hit_count"]))[
        : args.k
    ]
    results = []
    for g in groups:
        chunks = []
        for e, d in g["chunks"][:5]:
            ce = {
                "chunk_id": e["chunk_id"],
                "score": round(e["score"], 6),
                "rank_sources": dict(e["rank_sources"]),
                "text": d["text"],
            }
            if e["chunk_id"] in vec_dist and vec_dist[e["chunk_id"]] is not None:
                ce["vector_distance"] = round(vec_dist[e["chunk_id"]], 6)
            if args.expand:
                ce["context"] = neighbor_map(be, d, args.expand)
            if args.with_message:
                ce["message"] = message_obj(be, d)
            chunks.append(ce)
        results.append(
            {
                "session_id": g["session_id"],
                "score": round(g["score"], 6),
                "hit_count": g["hit_count"],
                "session": session_obj(g["_d"]),
                "chunks": chunks,
            }
        )
    return envelope(args, be, filters, results, notes)


def envelope(args, be, filters, results, notes):
    out = {
        "query": getattr(args, "query", None),
        "mode": getattr(args, "mode", None),
        "backend": be.name,
        "filters_applied": filters.describe(),
        "results": results,
    }
    if notes:
        out["notes"] = notes
    return out


# -------------------------------------------------------------- details ---
def details_sql(be):
    return (
        "SELECT c.id AS chunk_id, c.text AS text, c.chunk_idx AS chunk_idx, "
        "c.message_id AS message_id, m.role AS m_role, m.session_id AS session_id, "
        "s.title AS s_title, s.client AS s_client, s.created_at AS s_created_at, "
        "l.kind AS l_kind, l.topics AS l_topics, l.projects AS l_projects, "
        f"l.sensitivity AS l_sensitivity, {be.sens_expr} AS chunk_sensitivity "
        "FROM chunks c "
        "JOIN messages m ON m.id = c.message_id "
        "JOIN sessions s ON s.id = m.session_id "
        f"LEFT JOIN {be.labels_table} l ON l.session_id = s.id "
        f"{be.sens_join} "
        "WHERE c.id IN ({ph})"
    )


def fetch_details(be, chunk_ids):
    if not chunk_ids:
        return {}
    sql = details_sql(be).replace("{ph}", ",".join("?" * len(chunk_ids)))
    rows = be.execute(sql, list(chunk_ids))
    out = {}
    for r in rows:
        r["eff_sensitivity"] = r.get("chunk_sensitivity") or r.get("l_sensitivity")
        out[r["chunk_id"]] = r
    return out


def neighbor_map(be, d, n):
    rows = be.execute(
        "SELECT id, chunk_idx, text FROM chunks WHERE message_id = ? "
        "AND chunk_idx BETWEEN ? AND ? ORDER BY chunk_idx",
        [d["message_id"], d["chunk_idx"] - n, d["chunk_idx"] + n],
    )
    before = [
        {"chunk_id": r["id"], "text": r["text"]}
        for r in rows
        if r["chunk_idx"] < d["chunk_idx"]
    ]
    after = [
        {"chunk_id": r["id"], "text": r["text"]}
        for r in rows
        if r["chunk_idx"] > d["chunk_idx"]
    ]
    return {"before": before, "after": after}


def message_obj(be, d):
    rows = be.execute(
        "SELECT id, role, text FROM messages WHERE id = ?", [d["message_id"]]
    )
    if not rows:
        return None
    r = rows[0]
    text = r["text"] or ""
    truncated = len(text) > MESSAGE_TEXT_CAP
    return {
        "id": r["id"],
        "role": r["role"],
        "text": text[:MESSAGE_TEXT_CAP],
        "truncated": truncated,
    }


# ------------------------------------------------------------ get-chunk ---
def run_get_chunk(be, args):
    details = fetch_details(be, [args.chunk_id])
    d = details.get(args.chunk_id)
    if d is None:
        die(f"chunk {args.chunk_id} not found", code=1)
    out = {
        "chunk_id": d["chunk_id"],
        "text": d["text"],
        "chunk_idx": d["chunk_idx"],
        "message": {
            "id": d["message_id"],
            "role": d["m_role"],
            "session_id": d["session_id"],
        },
        "session": session_obj(d),
        "session_id": d["session_id"],
    }
    if args.neighbors:
        out["context"] = neighbor_map(be, d, args.neighbors)
    notes = list(getattr(be, "notes", []))
    if notes:
        out["notes"] = notes
    return out


# ----------------------------------------------------------------- text ---
def render_text(out):
    lines = []
    if "query" in out:
        lines.append(
            f"query: {out.get('query')}  mode: {out.get('mode')}  backend: {out.get('backend')}"
        )
        lines.append(f"filters: {json.dumps(out.get('filters_applied', {}))}")
        for n in out.get("notes", []):
            lines.append(f"note: {n}")
        for i, r in enumerate(out["results"], 1):
            if "chunks" in r:  # grouped by session
                s = r["session"]
                lines.append(
                    f"\n[{i}] session {r['session_id']} score={r['score']} hits={r['hit_count']}"
                )
                lines.append(
                    f"    {s['title']} · {s['client']} · {s['created_at']} · "
                    f"{s['kind']} · {','.join(s['topics'])} · {s['sensitivity']}"
                )
                for ce in r["chunks"]:
                    lines.append(
                        f"    chunk {ce['chunk_id']} score={ce['score']} {ce['rank_sources']}"
                    )
                    lines.append("      " + snippet(ce["text"]))
            else:
                s = r["session"]
                lines.append(
                    f"\n[{i}] chunk {r['chunk_id']} score={r['score']} {r['rank_sources']}"
                )
                lines.append(
                    f"    {s['title']} · {s['client']} · {s['created_at']} · "
                    f"{s['kind']} · {','.join(s['topics'])} · {s['sensitivity']}"
                )
                lines.append("    " + snippet(r["text"]))
    else:  # get-chunk
        s = out["session"]
        lines.append(
            f"chunk {out['chunk_id']} (idx {out['chunk_idx']}) session {out['session_id']}"
        )
        lines.append(
            f"{s['title']} · {s['client']} · {s['created_at']} · "
            f"{s['kind']} · {','.join(s['topics'])} · {s['sensitivity']}"
        )
        lines.append(f"message {out['message']['id']} role={out['message']['role']}")
        lines.append(out["text"])
        ctx = out.get("context")
        if ctx:
            for side in ("before", "after"):
                for c in ctx[side]:
                    lines.append(
                        f"[{side} chunk {c['chunk_id']}] " + snippet(c["text"])
                    )
    return "\n".join(lines)


def snippet(text, n=300):
    t = " ".join((text or "").split())
    return t[:n] + ("…" if len(t) > n else "")


# ------------------------------------------------------------------ main --
def default_db_path():
    env = os.environ.get("ATLAS_DB")
    if env:
        return env
    laptop = os.path.expanduser("~/atlas/turso-atlas/data/atlas.db")
    if os.path.exists(laptop):
        return laptop
    repo_data = os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "..", "data", "atlas.db"
    )
    if os.path.exists(repo_data):
        return os.path.normpath(repo_data)
    return laptop  # for the error message


def pick_backend(args):
    vec_table = getattr(args, "vec_table", VEC_TABLE)
    trunc = getattr(args, "trunc", 0)
    if args.backend == "turso":
        return TursoBackend(vec_table)
    if args.backend == "local":
        return LocalBackend(args.db or default_db_path(), args.vecdb, vec_table, trunc)
    # auto: turso when a token resolves, else local
    if os.environ.get("ATLAS_TURSO_TOKEN"):
        return TursoBackend(vec_table)
    for name in BWS_TOKEN_NAMES:
        try:
            bws_secret(name)
            return TursoBackend(vec_table)
        except Exception:
            continue
    if turso_cli_logged_in():
        return TursoBackend(vec_table)
    return LocalBackend(args.db or default_db_path(), args.vecdb, vec_table, trunc)


def add_backend_args(p):
    p.add_argument("--backend", choices=["auto", "turso", "local"], default="auto")
    p.add_argument(
        "--db", help="local backend: SQLite DB path (default: canonical atlas.db)"
    )
    p.add_argument(
        "--vecdb",
        help="local backend: comma-separated vector DB path(s) "
        "if the corpus DB has no vec table",
    )


def add_vector_args(p):
    p.add_argument(
        "--model",
        default=os.environ.get("ATLAS_SEARCH_MODEL", MODEL),
        help=f"embedding model (default {MODEL}; env ATLAS_SEARCH_MODEL)",
    )
    p.add_argument(
        "--vec-table",
        default=os.environ.get("ATLAS_VEC_TABLE", VEC_TABLE),
        help=f"vector table name (default {VEC_TABLE}; env ATLAS_VEC_TABLE)",
    )
    p.add_argument(
        "--embedder",
        choices=["auto", "fastembed", "st"],
        default="auto",
        help="query embedder: fastembed (bge) or st = sentence-transformers "
        "with prompt_name='query' (Snowflake); auto picks by model name",
    )
    p.add_argument(
        "--trunc",
        type=int,
        default=0,
        help="truncate query+doc vectors to N dims (Matryoshka; 0 = full)",
    )
    p.add_argument(
        "--vector-backend",
        choices=["scan", "index-fill", "index-fixed"],
        default="scan",
        help="turso vector branch: scan = exact vector_distance_cos over "
        "the filtered set (default); index-fixed = one vector_top_k "
        "lookup at pool depth, filtered afterwards; index-fill = "
        "vector_top_k with escalating depth until the pool fills "
        "after filtering (cap 3200)",
    )


def main():
    ap = argparse.ArgumentParser(
        prog="atlas-search", description=__doc__.splitlines()[0]
    )
    sub = ap.add_subparsers(dest="cmd", required=True)

    ps = sub.add_parser(
        "search", help="hybrid FTS+vector search with classification filters"
    )
    ps.add_argument("query")
    ps.add_argument("--k", type=int, default=10, help="final results (default 10)")
    ps.add_argument(
        "--pool", type=int, default=100, help="per-branch candidates (default 100)"
    )
    ps.add_argument("--mode", choices=["hybrid", "fts", "vector"], default="hybrid")
    ps.add_argument("--kind")
    ps.add_argument("--topic")
    ps.add_argument("--project")
    ps.add_argument("--client")
    ps.add_argument("--since", help="YYYY-MM-DD, on sessions.created_at (inclusive)")
    ps.add_argument("--until", help="YYYY-MM-DD, on sessions.created_at (inclusive)")
    ps.add_argument(
        "--max-sensitivity",
        default="personal",
        help="ceiling in shareable<personal<local-only<secret-bearing "
        "(default personal; secret-bearing must be spelled out)",
    )
    ps.add_argument(
        "--group-by-session",
        action="store_true",
        help="roll chunk hits up to sessions (score = best chunk, plus hit count)",
    )
    ps.add_argument(
        "--expand",
        type=int,
        default=0,
        help="include N neighboring chunks (same message) around each hit",
    )
    ps.add_argument(
        "--with-message",
        action="store_true",
        help="include the parent message text (truncated at 50000 chars)",
    )
    ps.add_argument(
        "--text", action="store_true", help="human-readable output instead of JSON"
    )
    add_backend_args(ps)
    add_vector_args(ps)

    pg = sub.add_parser("get-chunk", help="one chunk with provenance")
    pg.add_argument("chunk_id", type=int)
    pg.add_argument("--neighbors", type=int, default=0)
    pg.add_argument("--text", action="store_true")
    add_backend_args(pg)

    args = ap.parse_args()
    be = pick_backend(args)
    if args.cmd == "search":
        out = run_search(be, args)
    else:
        out = run_get_chunk(be, args)
    if args.text:
        print(render_text(out))
    else:
        print(json.dumps(out, indent=1, ensure_ascii=False))


if __name__ == "__main__":
    main()
