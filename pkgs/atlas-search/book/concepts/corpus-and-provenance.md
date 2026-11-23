# The corpus: provenance and content addressing

Source: https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/local.rs, https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/stage.rs, https://github.com/developing-today/code/blob/main/pkgs/atlas-search/test-vectors/content-hash-v1.json, https://github.com/developing-today/code/blob/main/pkgs/atlas-search/fixtures/fixture_corpus.sql

Atlas exists to answer one question reliably: *where have I seen this
before, and where did it come from?* The second half of that question is the
provenance half, and it shapes the corpus's data model. This chapter covers
that model — sessions, messages, and chunks — the hash that identifies a
chunk everywhere it appears, the content-addressed extracts programme that
preserves the sources themselves, and the extra databases that still live
beside the canonical store. The system the corpus sits in is drawn in
[Architecture](architecture.md); who may read which parts of the corpus is
[Sensitivity tiers and client ceilings](sensitivity.md).

## Sessions, messages, chunks

Every conversation, whatever client produced it, is normalized into the same
three-level shape in the canonical store:

- A **session** is one conversation. Its row carries an id, a title, the
  client that produced it (the `client` column — `muse`, `chatgpt`,
  `opencode`, and so on), and its creation time.
- A **message** belongs to a session and carries an id, a role, and its text.
- A **chunk** belongs to a message and is the unit of search: an id, the
  message id, its position within the message (`chunk_idx`), its text, a
  `content_hash`, and a sensitivity. Messages are cut into chunks of roughly
  3,500 characters in the original pipeline; chunking in the newer staging
  path is boundary-driven instead, and may change only at declared source
  boundaries.

Classification hangs off the session. A `session_labels` row per session
records the scheme-1.0 classification: `kind`, `topics` and `projects` (each
a JSON list), `sensitivity`, a confidence, and the scheme name. Sensitivity
is also denormalized onto each chunk in the published store; a chunk's
*effective* sensitivity is its own value, falling back to its session label's
when the chunk carries none. Two derived structures make the chunks
searchable: a `chunks_fts` FTS5 table over chunk text (its rowid is the chunk
id), and one vector table per embedding space, keyed by chunk id — the
production table is `vec_snowflake_m_v2_768_f32`.

A toy example, invented for this chapter: a session titled "porch roof
quotes" from one client might hold four messages and nine chunks. Search
never returns the session or the message as its unit — it returns chunks,
each carrying enough provenance (session, client, time, labels) for the
caller to judge and to fetch the surrounding message.

## `content_hash`: the identity every store shares

Chunk ids are local to the database that assigned them. The same chunk text
appearing in the canonical store, a staging database, a vector side store,
and a shard will in general have four different ids — and the id spaces are
not merely different, they are unrelated: joining two stores on chunk id
would silently attach the wrong vectors to roughly 14,000 chunks (verified
2026-10-31). What every store shares is the chunk's `content_hash`.

So `content_hash` is the join key, everywhere:

- In the canonical and shard schemas it is a column of `chunks`, declared
  `UNIQUE NOT NULL`.
- In a staging database it is a column of `chunks_raw`, with its own index.
- The local backend resolves legacy vector side stores by it: it reads
  `id, content_hash` from the side store's mapping corpus and from the main
  database, builds a hash-to-main-id map, and re-keys each stored vector to
  the main chunk id whose chunk has the same hash. The first store to supply
  a vector for an id wins, and coverage is reported as a note ("vector
  coverage: &lt;have&gt; of &lt;total&gt; chunks have vectors via
  content-hash mapping"). The one exception is an explicitly supplied vector
  database, which is assumed to be keyed by the corpus's own chunk ids.
- The golden-set corpus fingerprint is built from it: SHA-256 over the
  concatenation of the `content_hash` values of chunks with
  `id % 997 = 0` — a cheap way to prove two corpora are the same corpus.
- A promotion's merge manifest records one, as the durable audit of what was
  promoted.

One caveat a careful reader should know: the hash's *producer* is not yet
pinned. The shared test-vector file (`test-vectors/content-hash-v1.json`,
#10147) records `chunk_content_hash` with status `producer_unavailable` —
algorithm, producer, and profile all null, no known-answer vectors, and
interop claims explicitly forbidden until a producer is pinned there. The
same file does pin a different hash — the classification manifest's content
hash, SHA-256 lowercase hex over the manifest's canonical form — and says in
terms that it is *not* the chunk `content_hash`, recorded precisely to keep
the two apart. In practice the value is produced by the ingest pipelines and
treated as opaque identity by everything downstream; treat any claim about
how it is computed as unverified until the vector file says otherwise.

Deduplication follows from the same identity. Re-running an ingest adds only
what is new, because a chunk whose hash is already present is the same chunk.
But dedup has a known historical cost, and it is the reason the next section
exists: the first ingest pipelines deduplicated by dropping repeated
occurrences, silently losing roughly 127,000 records of *where* a chunk had
been found — and 6,344 canonical sessions own zero chunks for exactly this
reason. The bytes survived; the provenance did not.

## The extracts programme: content-addressed sources (#3125)

Issue #3125 — the provenance registry, content-addressed extracts, and
multi-database vector resolution programme — is the corrective. Its rules,
set by Drew on 2026-10-31, are: keep all data (only byte-identical duplicates
are cleaned; backups are kept); keep backups *and* plain extracts for every
corpus; and record, for every chunk, *every* place it was found. That last
rule is the `chunk_occurrences` table: one row per occurrence, mapping a
chunk to a source locator with its byte span and the time it was observed
(`observed_at`). Sources that are not files are pinned as
`repo@commit:path` — a toy form would be `some-repo@0123abc:docs/notes.md`.

The programme has four phases. Phases 1 and 2 have landed; 3 and 4 are open.

**Phase 1 — the occurrence and registry tables (landed 2026-11-22).** The
canonical database gained `chunk_occurrences` and a `cas_registry` (the
registry of content-addressed objects), and ingest writes occurrence rows for
new imports from then on. A further in-repo slice landed 2026-11-23: each
chunk promoted through the staging path now gets a `chunk_occurrences` row
recording its canonical text hash, source locator, byte span, and staged
observation time. That slice is explicitly not the full criterion —
historical occurrence recovery, canonical dedup identity, and the
CAS/source-root registry remain open work.
A further slice landed the same day: the in-repo source registry
('s  module) — stable source-root and
locator registration, where a root is a named origin (a pinned
repository, an extracts corpus) and locators hang from it.
Registration is idempotent and additive only: a repeat returns the
first registration unchanged, re-pointing a root or registering a
locator under an unknown root is refused, and registration never
alters existing chunk or occurrence data — each property pinned by
oracle tests.

**Phase 2 — the extracts store and its backfill (landed 2026-11-23).** The
extracts store lives on the laptop under `~/atlas/extracts/`. Objects are
filed content-addressed, by SHA-256, under
`cas/&lt;ab&gt;/&lt;cd&gt;/&lt;sha256&gt;` (the first two path components are
the hash's first two byte pairs); beside the objects sit manifests named
`manifests/<source>-<date>.json`, each entry carrying the source, the
extraction time, the hash, the byte count, and the destination. The tooling
(`atlas-extracts.py`, in hatch-personal) can register objects into the
canonical `cas_registry`, and its `verify` subcommand re-hashes every
manifest-referenced object — a full-store verification prints
`ATLAS-EXTRACTS VERIFY OK`. The daily `atlas-backup` now covers the store
too, and each backup run proves recoverability by verifying against a
restored copy.

The 2026-11-23 backfill run interned the sources still on disk, starting
that morning from zero objects and no `cas_registry` table:

| Corpus | Files interned | Bytes |
|---|---:|---:|
| chatgpt-export (the export archive, one file) | 1 | 103,945,101 |
| sandbox-agents | 12,091 | 2,492,957,497 |
| **Total** | **12,092** | **~2.6 GB** |

The run recorded **zero dedupe collisions** — every interned file was unique
content — and the full-store verification passed: `ATLAS-EXTRACTS VERIFY OK`.

**Phase 3 — occurrence backfill (open).** Recovering the historical
occurrences from the now-interned extracts, and documenting the publish
predicate, has not started as a completed slice.

**Phase 4 — catalog and resolution as tools (open).** A catalog database and
a shard layout (collection × month) at the next publish growth, with
resolution exposed as MCP tools — search to ids, resolve, fetch span — is
designed direction, not built machinery.

## Staged tiers beside the canonical store

The canonical store is not the only database a search may touch. Two kinds
of side store sit beside it, and they must not be conflated.

**Per-unit staging databases** are the write side, covered as a pipeline
stage in [Architecture](architecture.md) and in
[Ingest today: staging, classification, promotion](ingest.md). Each is a
small `atlas_stage` v1 database holding one unit's `items` and un-embedded
`chunks_raw` — staging databases never store vectors — located through the
staging registry, and read by the aggregator at promotion. Promotion is the
terminal write: the promoted unit's database is stamped with a manifest
table, which closes it to further staging writes by design, and is branched
as the unit's archive.

**Legacy topup and delta databases** are the read side's history. They were
never merged into canonical identity; instead the search backends resolve
them at query time, by `content_hash`, as described above. The local backend
probes its known side-store pairs in a fixed order — a vector file plus the
corpus file whose chunks map its ids:

| Vector store | Mapping corpus |
|---|---|
| `~/atlas/v3/vec-bge-small.db` | `~/atlas/v3/corpus.db` |
| `~/atlas/v3/vec-bge-small-topup.db` (the topup DB) | `~/atlas/v3/topup-chunks.db` |
| `~/atlas/atlas-delta-chunks2.db` (the delta DB) | itself — vector file and mapping corpus are the same file |
| `~/atlas/v3/vec-snowflake.db` | `~/atlas/v3/corpus.db` |

The topup store alone accounts for roughly 14,000 chunks. This query-time,
hash-keyed resolution is the working-today form of multi-database vector
resolution; the general catalog-and-shard form is phase 4 of #3125, above.

Beside both sits the shard set — 29 topic shards next to the Turso monolith
at the last verified layout — which is a serving layout of the same corpus,
not a separate corpus. Schemas for every store named here are collected in
[Data stores and schemas](../reference/data-stores.md).

## The frozen census, and the growth since

Every corpus number in this book traces to one verified count, and it is a
snapshot, not a live figure. **The census of 2026-10-31** (recorded on
#3125) counted the canonical store at:

| Level | Count |
|---|---:|
| Sessions | 18,325 |
| Messages | 122,124 |
| Chunks | 107,639 |

The tier split of the same census agrees with the total: 107,001 chunks in
28 internal shards and 638 chunks in 1 secret shard; there were no
public-tier shards. The session labels were complete for all 18,325
sessions: `personal` 16,009, `local-only` 2,226, `secret-bearing` 90. The
Turso publish at that date was 1.0 GB holding 100,698 chunks — 6,941 chunks
of personal-client text were at that point held back from Turso by policy.
That same day the hold-back was lifted: everything in canonical publishes,
and privacy moved to query-time classification filters instead.

Since then the store has grown without a recount. Later records (2026-11-12)
describe the Turso monolith as about **7.3 GB** over the same 107,639 chunks
plus the 29 topic shards, and state in terms that the canonical counts are
frozen at the 2026-10-31 publish while the monolith grew *without a verified
recount*. Read every number in the table above as "true on 2026-10-31", not
as today's count; a fresh verified census is an open deliverable of the
Atlas programme. For scale, the laptop's `~/atlas/` directory overall was
12 GB at the census date, hash-verified to contain no consequential
byte-identical duplicates — it is distinct versions and working sets, so no
cleanup windfall exists there.
