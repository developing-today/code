# Data stores and schemas

Source: `schema/atlas_stage_v1.sql`, `schema/stage_registry_v1.sql` and
`schema/gate_records_v1.sql`, `fixtures/fixture_corpus.sql`,
`crates/atlas-core/src/{stage,gate,manifest,embed_cache,aggregate,feedback,shards}.rs`
and `worker/migrations/0001_hot_cache.sql` and `worker/src/cache_do.ts` in
https://github.com/developing-today/code/blob/main/pkgs/atlas-search/ — provenance
only; this chapter is complete without following it.

Atlas has no single database. It has a canonical publish store, a set of
per-shard scan stores, a family of small write-side staging stores, two cache
stores, and two file stores. This chapter names every one of them, says where
it lives, and gives its schema as the code defines or uses it. Where no
`CREATE TABLE` file exists in the repository — the canonical publish store is
the main case — the schema below is the shape the code issues SQL against and
the fixture corpus mirrors, and the chapter says so.

Two vocabularies recur and must not be confused. The sensitivity *ladder* —
canonically `public`, `internal`, `private`, `sensitive`, and spelled in
the stored values today `shareable`, `personal`, `local-only`,
`secret-bearing` — is the value set of
the publish store's sensitivity columns. The physical *tier* vocabulary —
`public`, `internal`, `secret` — is the value set of the staging, gate and
shard stores. The aggregator translates tier to ladder at promotion; no schema
encodes that mapping.

## The stores at a glance

| Name | Where it lives | What it holds |
|---|---|---|
| Canonical Turso DB (`atlas`) | Hosted libSQL database `atlas` on the Atlas Turso host, read over the HTTPS pipeline API with a read-only token; a local publish file (`atlas.db`) has the same shape for the local backend | Sessions, messages, chunks, session labels, the FTS5 index, the classification manifest, generic labels, and the production vector table |
| Vector tables | Inside the canonical DB, inside each scan shard, and in local side-store files | One row per chunk: the chunk id and its embedding vector in one named model space |
| Scan shards (`atlas-core-&lt;tier&gt;-&lt;topic&gt;-&lt;nn&gt;`) | One Turso database (or one local `.db` file) per shard | That shard's chunks, its vector table, and its `atlas_manifest` self-description; no sessions, messages or labels |
| Content stores (`atlas-core-&lt;tier&gt;-content`) | One Turso database per tier | The messages and sessions half of the corpus for that tier, reused from the publish tables, for hydrating shard-served hits |
| Staging DB (v1) | One local SQLite/libSQL file per staging unit | Raw, un-embedded staged items and chunks for one unit of work; never any vectors |
| Registry DB (v1) | One local SQLite/libSQL file, owned by the staging registry | Which staging units exist, where their DBs are, their state, and the audit record of each promotion |
| Gate records (v1) | A table inside the registry DB | One audit row per ingest batch: stage verdicts, strikes and state — counts, codes and hashes only, never content |
| Query-embedding cache DB | A separate Turso database, configured only by environment variables | Raw query vectors keyed by a hash of model identity and normalised query text |
| Feedback store | JSONL files under `~/.local/state/atlas-search/feedback/` | Append-only feedback records, one file per day, plus derived normalised files |
| D1 hot cache (`atlas-hot-cache`) | A Cloudflare D1 database bound to the Atlas Worker | Opaque cached payloads keyed by tenant and cache key; storage layer only, not wired into the request path |
| Worker Durable Object entry | SQLite storage inside one `QueryCache` Durable Object instance per cache key | One row: the normalised query, its embedding, its search result and hit bookkeeping |

## The canonical Turso DB

There is no `CREATE TABLE` file for the publish DB in the repository. The
tables below are the publish shape as the branch, detail and neighbour SQL in
`atlas-core` uses them, and as `fixtures/fixture_corpus.sql` — a miniature
publish-shaped database — defines them.

| Table | Columns and shape | What it holds |
|---|---|---|
| `sessions` | `id TEXT PRIMARY KEY`, `title TEXT`, `client TEXT`, `created_at TEXT` | One row per conversation session; `client` and `created_at` are filter dimensions |
| `messages` | `id TEXT PRIMARY KEY`, `session_id TEXT`, `role TEXT`, `text TEXT` | One row per message in a session |
| `chunks` | `id INTEGER PRIMARY KEY`, `message_id TEXT NOT NULL`, `chunk_idx INTEGER`, `text TEXT NOT NULL`, `content_hash TEXT UNIQUE NOT NULL`, `sensitivity TEXT` | One row per searchable chunk. `content_hash` is the chunk identity every other store joins on. Sensitivity is denormalised onto the chunk; the effective sensitivity is the chunk's own value, falling back to its session label's |
| `session_labels` | `session_id TEXT PRIMARY KEY`, `client TEXT`, `kind TEXT`, `topics TEXT`, `projects TEXT`, `sensitivity TEXT`, `confidence TEXT`, `scheme TEXT` | The classification of one session. `topics` and `projects` hold JSON list text. The production classification scheme recorded by the Python header is scheme-1.0 |
| `chunks_fts` | Virtual table: `CREATE VIRTUAL TABLE chunks_fts USING fts5(text)`, with `rowid` equal to `chunks.id` | The FTS5 (BM25) branch's index over chunk text |
| `classification_manifest` | Rows read as `manifest_version`, `content_hash`, `name`, `shape`, `semantics`, `label_column` | The manifest declaring the filterable dimensions: each dimension's name, storage shape, match semantics and label column. Loaded when a backend opens; an absent or invalid manifest falls back to the built-in manifest, loudly. The manifest schema carries no sensitivity keys, by a safety invariant enforced at load |
| `generic_labels` | `session_id TEXT`, `dimension TEXT`, `values_json TEXT`, primary key `(session_id, dimension)` | Values for manifest dimensions declared with the `generic_list` shape. Not part of the base publish fixture; created from `atlas_core::manifest::GENERIC_LABELS_DDL` when that shape is in use |

Behaviour notes:

- The search backends only read this store. The Turso backend issues `SELECT`
  statements over the pipeline API and resolves a read-only token; it holds no
  write credential for the canonical DB.
- The ceiling and the filters are applied inside the branch SQL, before any
  ranking. A chunk whose sensitivity is `NULL` never passes the ceiling
  clause.
- Corpus size anchors are frozen snapshots, not live counts: the census
  recorded on 2026-10-31 is 18,325 sessions / 122,124 messages / 107,639
  chunks, and the frozen v3 corpus behind the golden-311 set is 101,471
  chunks / 18,116 sessions. See
  [Backup, restore and the census](../operations/backup-and-census.md).

## Vector tables

Every vector table has the same row shape in SQLite-shaped stores —
`chunk_id INTEGER PRIMARY KEY, vec BLOB NOT NULL`, the vector stored as
little-endian `f32` values. In the live Turso publish the vector column is
typed `F32_BLOB(768)` and carries a `libsql_vector_idx` index named after the
table (the table name plus `_idx`).

| Table | Model space | Dimensions | Status |
|---|---|---|---|
| `vec_snowflake_m_v2_768_f32` | Snowflake `snowflake-arctic-embed-m-v2.0`, pinned revision `95c2741480856aa9666782eb4afe11959938017f` | 768 | The production table and the default everywhere. A query may be truncated to fewer dimensions at query time and renormalised; the stored table stays at 768 |
| `vec_bge_small_384_f32` | `BAAI/bge-small-en-v1.5`, via the fastembed embedder in the Python implementation only | 384 | Dormant legacy fallback. It remains in the publish and in local side-store files; no Rust code names it, and the Worker refuses this space as not comparable with the corpus |

No other vector tables are named anywhere in the tree. The two spaces are
never comparable with each other: a vector from one must never be scored
against the other. See
[Embeddings: Snowflake Arctic and the parity contract](../concepts/embeddings.md).

Local side stores: when a local publish file has no vector table of its own,
the local backend probes known side-store files and joins them to the corpus
by `content_hash`, never by raw chunk id — the id spaces differ between
stores, and an id join would silently attach the wrong vectors. An explicitly
given vector DB file is the one exception: it is assumed keyed by the corpus
DB's own chunk ids.

## Scan shards and content stores

The core tier can also be served as a set of scan shards. A shard's name is
`atlas-core-&lt;tier&gt;-&lt;topic&gt;-&lt;nn&gt;`, where the tier is one of
`public`, `internal` or `secret`, the topic is non-empty, and the part number
has at least two digits. Local shard files are the shard name plus `.db`.

Each scan shard holds exactly three tables:

```sql
CREATE TABLE chunks (id INTEGER PRIMARY KEY, message_id TEXT NOT NULL,
  chunk_idx INTEGER, text TEXT NOT NULL, content_hash TEXT UNIQUE NOT NULL,
  sensitivity TEXT);
CREATE TABLE vec_snowflake_m_v2_768_f32 (chunk_id INTEGER PRIMARY KEY, vec BLOB NOT NULL);
CREATE TABLE atlas_manifest ("key" TEXT PRIMARY KEY, value TEXT);
```

- `chunks` — the shard's chunks. Chunk ids in a newly written shard are
  assigned sequentially from 1 in piece order. `message_id` is the staged
  item's source pointer, or `unit:&lt;unit_id&gt;` when the item has none.
  Chunk `sensitivity` is written in ladder vocabulary.
- The vector table — the shard's vectors, in the production 768-dimension
  space only.
- `atlas_manifest` — the shard's self-description as key/value pairs. The
  aggregator writes exactly nine keys: `shard`, `tier`, `topic`, `part`,
  `expected_chunks`, `metric` (the value `cosine`), `embedding_model`,
  `engine` (the value `tursodb`) and `source_unit`. Discovery reads this
  table and cross-checks it against the shard's name. A shard whose manifest
  names a different embedding model than the consumer's, or none in strict
  mode, is refused: vectors from different embedding models are not
  comparable.

Tier assignment: a shard's tier is the staging unit's recorded sensitivity
at promotion, and its topic is the unit's topic string. The sensitivity
ladder maps onto tiers as, in canonical names, public to public, internal
and private to internal, and sensitive to secret (in the stored spellings:
`shareable` to public, `personal` and `local-only` to internal, and
`secret-bearing` to secret) — so a caller at the default
`personal` ceiling can never fan out to a secret-tier shard, and a tier above
the caller's ceiling is never connected, probed or scanned.

Scan shards carry no `sessions`, `messages` or `session_labels` tables, and
TursoDB shards have no `chunks_fts`. That is why queries carrying
session-level filters stay on the canonical DB for both branches.

Content stores reuse the publish tables instead of defining new ones: one
database per tier, named `atlas-core-&lt;tier&gt;-content`, holding the
`messages` and `sessions` half of the corpus in publish shape. A shard-served
search hydrates its top hits from the content store for the chunk's tier,
running the canonical details SQL against it. A store above the discovery
ceiling, or without a credential, is rejected without any connection attempt;
a hydration failure names the store and never falls back silently to another
tier's store.

## Staging v1

Every staging DB carries the v1 schema from `schema/atlas_stage_v1.sql`,
applied exactly once to an empty database — there is no `IF NOT EXISTS`, and
re-provisioning over an existing set is an error. A staging DB is promoted by
being read, never by being migrated; a schema change means a new versioned
DDL artifact, never an edit to v1.

| Table | Columns and shape | What it holds |
|---|---|---|
| `_schema_version` | `version INTEGER NOT NULL`, seeded with the single row `1` | The schema version of this staging DB |
| `items` | `id INTEGER PRIMARY KEY`, `unit_id TEXT NOT NULL`, `kind TEXT NOT NULL`, `source TEXT`, `text TEXT NOT NULL`, `sensitivity TEXT NOT NULL DEFAULT 'internal'`, `created_at TEXT NOT NULL`, `meta TEXT`; index on `(unit_id, kind)` | One row per staged item (a message, note or document fragment). `meta` is an open JSON bag. Writes are unmediated: sensitivity is recorded, not enforced, at write time. The default value `internal` is a tier word |
| `chunks_raw` | `id INTEGER PRIMARY KEY`, `item_id INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE`, `idx INTEGER NOT NULL`, `text TEXT NOT NULL`, `content_hash TEXT NOT NULL`, `created_at TEXT NOT NULL`; indexes on `(item_id, idx)` and on `(content_hash)` | The item's raw, un-embedded chunks. A staging DB never stores vectors, which is what makes it cheap to write and cheap to branch |
| `notes` | `id INTEGER PRIMARY KEY`, `unit_id TEXT NOT NULL`, `body TEXT NOT NULL`, `created_at TEXT NOT NULL`, `meta TEXT`; index on `(unit_id)` | Free working notes against the unit |
| `kv` | `key TEXT PRIMARY KEY`, `value TEXT NOT NULL`, `updated_at TEXT NOT NULL` | Cursor and checkpoint scratch space for producers; last write wins |

The code-side check asserts the version row is 1 and the table set is exactly
these five. Promotion adds one more table — the `atlas_manifest` stamp — to
the unit's DB, which deliberately closes it to further staged writes.

## Registry v1

The registry's own store, from `schema/stage_registry_v1.sql`, created and
owned by the staging registry in `atlas-core`. Both tables are created with
`IF NOT EXISTS`.

| Table | Columns and shape | What it holds |
|---|---|---|
| `staging_units` | `unit_id TEXT PRIMARY KEY`, `db_uri TEXT NOT NULL`, `owner TEXT NOT NULL`, `sensitivity TEXT NOT NULL`, `state TEXT NOT NULL CHECK (state IN ('open','closing','merged'))`, `schema_version INTEGER NOT NULL`, `created_at TEXT NOT NULL`, `updated_at TEXT NOT NULL`; index on `(state)` | One row per staging unit: where its DB lives (`db_uri` is a local file path today), who owns it, its tier, and its lifecycle state |
| `merge_manifests` | `unit_id TEXT PRIMARY KEY`, `rows_in INTEGER NOT NULL`, `rows_promoted INTEGER NOT NULL`, `content_hash TEXT NOT NULL`, `embedding_model TEXT NOT NULL`, `promoted_at TEXT NOT NULL` | The durable audit record of each successful promotion, kept after the unit leaves the active states |

## Gate v1

One table, from `schema/gate_records_v1.sql`, kept in the registry DB beside
`staging_units`.

| Table | Columns and shape | What it holds |
|---|---|---|
| `gate_records` | `batch_id TEXT PRIMARY KEY`, `unit_id TEXT NOT NULL`, `tier TEXT NOT NULL CHECK (tier IN ('public','internal','secret'))`, `state TEXT NOT NULL CHECK (state IN ('open','closing','gated','published','quarantined'))`, `stage_reached TEXT`, `verdicts_json TEXT NOT NULL`, `strikes_json TEXT NOT NULL`, `needs_review INTEGER NOT NULL CHECK (needs_review IN (0,1))`, `publish_manifest_id TEXT`, `updated_at TEXT NOT NULL`; indexes on `(unit_id)` and on `(state)` | One row per ingest batch: the five gate stages' verdicts as JSON, the per-reason strike counts as JSON, and the batch state. Written at the sensitivity of its batch, it holds only counts, enumerated reason codes, row ids, content hashes and the publish manifest id — never content |

## The query-embedding cache

A query vector is a pure function of the normalised query text, the model id
and the model revision, so the cache needs no time-to-live and no
invalidation: a model change changes the key. There are two tiers — an
in-service in-memory cache of about 10,000 entries, always on, and a shared
Turso cache DB that exists only when both of its environment variables are
set. Its single table is created by `atlas-core`'s embed-cache module:

```sql
CREATE TABLE IF NOT EXISTS query_embedding_cache (
  key BLOB PRIMARY KEY, vector BLOB NOT NULL, created_unix INTEGER NOT NULL)
```

The key is a SHA-256 hash over the length-prefixed model id, model revision
and normalised text. The stored vector is the raw full-dimension vector,
before any per-request truncation; truncation is applied after retrieval, on
hit and on miss alike. Shared-tier writes are best-effort: an outage of the
cache DB never fails a search.

## The feedback JSONL store

Feedback is not a database. The raw store is a directory of append-only JSONL
files under `~/.local/state/atlas-search/feedback/`:

- One file per day, named `atlas-feedback-&lt;YYYY-MM-DD&gt;.jsonl`, the day
  taken from each record's own timestamp. One `atlas-feedback/v1` record per
  line: a feedback id, the query id and chunk id the vote attaches to (the
  chunk id is null for a per-query vote), the vote, an optional verbatim
  comment, the requester and the timestamp.
- Raw records are never mutated. A later vote from the same requester on the
  same result supersedes an earlier one only in the derived active view,
  never by rewriting the file.
- Normalised derivatives live in the `normalised` subdirectory by default, as
  `atlas-feedback-normalised-&lt;YYYY-MM-DD&gt;.jsonl` files in the
  `atlas-feedback-normalised/v1` schema. A derivative file is rewritten, not
  appended, when the normalisation pass is re-run, so derivatives can always
  be regenerated from the raw store.

The query-trace store beside it follows the same one-file-per-day JSONL
layout under `~/.local/state/atlas-search/traces/`. See
[atlas-search: serve, init, fetch-model, traces and feedback](atlas-search-admin.md).

## The D1 hot cache (Worker)

The migration `worker/migrations/0001_hot_cache.sql` creates one table in one
shared D1 database (binding name `HOT_CACHE`, database name
`atlas-hot-cache`); the tenant is carried in the key, never in a per-tenant
database. Summarised:

| Table | Columns and shape | What it holds |
|---|---|---|
| `hot_cache` | `tenant_id TEXT NOT NULL`, `cache_key TEXT NOT NULL`, `class TEXT NOT NULL` (one of `A`, `B` or `C`), `payload BLOB NOT NULL`, `payload_bytes INTEGER NOT NULL`, `source_watermark TEXT NOT NULL`, `filled_at INTEGER NOT NULL` (unix seconds), `last_hit_at INTEGER NOT NULL`, `hit_count INTEGER NOT NULL DEFAULT 0`; primary key `(tenant_id, cache_key)`; one index, `hot_cache_eviction` on `(tenant_id, last_hit_at)` | Opaque payloads supplied by a future filler, with their byte length, an opaque source watermark, and hit bookkeeping. The eviction index orders rows by least-recent hit per tenant |

There is no expiry column and no freshness check: an entry lives until it is
explicitly evicted or replaced, and no automatic eviction sweep exists. As
shipped, this is a storage layer only — no code on the Worker's request path
reads or writes it, whether or not its enabling flag is set. Do not confuse
it with the live Durable Object cache below, or with the service's in-memory
job store: three different stores, on different machines, with different
retention reasoning. See [The Worker HTTP API](worker-api.md).

## The Worker Durable Object entry table

The live Worker cache is one `QueryCache` Durable Object instance per cache
key — the instance name is the cache key — with SQLite-backed storage. Its
constructor creates a single table:

```sql
CREATE TABLE IF NOT EXISTS entry (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  query TEXT NOT NULL,
  k INTEGER NOT NULL,
  embedding_json TEXT,
  result_json TEXT,
  created_at TEXT NOT NULL,
  searched_at TEXT,
  embed_ms INTEGER,
  search_ms INTEGER,
  hit_count INTEGER NOT NULL DEFAULT 0
)
```

One object holds exactly one row (`id = 1`): the normalised query text and
its `k`, the query's embedding as a JSON array in the pinned Snowflake space,
the search result stored verbatim as JSON, the creation and search
timestamps, the embed and search timings in milliseconds, and the hit count.
The entry moves through the states absent, then embedded (the embedding is
persisted before the search runs, so a failed search is retried without
re-embedding), then complete. Entries are retained indefinitely — no
time-to-live and no eviction in v1 — and the only deletion is an explicit
operator purge of that one key, which deletes the row and returns the object
to the absent state.

## Failure shapes

- A staging DB whose version row is not 1, or whose table set is not exactly
  the five v1 tables, fails the schema check and is not read as a staging DB.
- A shard whose `atlas_manifest` disagrees with its name, or whose embedding
  model disagrees with the consumer's expected model, is rejected at
  discovery with the reason recorded — degradation is explicit, never a
  silent omission from the fan-out.
- A unit promotes only when its recorded embedding model equals the mandated
  model. A wrong model, no recorded model, or an empty unit are all fatal
  refusals that leave the unit unpromoted.
- The chunk `content_hash` is the identity every store joins on, but its
  producer is not pinned in the repository: the content-hash test vectors
  record it as producer-unavailable, with no algorithm and no known-answer
  vectors, and forbid interop claims until a producer is pinned. Treat the
  hash as an opaque identity, not as a recomputable value.
