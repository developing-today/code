# Glossary

Source: `pkgs/atlas-search/schema/`, `pkgs/atlas-search/crates/atlas-core/src/` (`stage.rs`, `gate.rs`, `search.rs`, `fusion.rs`, `shards/`, `eval.rs`), `pkgs/atlas-search/crates/atlas-proto/query-parameters-v1.json`, `pkgs/atlas-search/worker/src/` (`search.ts`, `cache_do.ts`, `hot_cache.ts`), and `.github/workflows/fleet-search-parity-guard.yml` under https://github.com/developing-today/code/blob/main/pkgs/atlas-search/ — provenance only; this chapter is complete without following it.

Atlas terms, defined once. Where a term names two things that must not be confused — there are two caches, two sensitivity vocabularies, and two RRF constants in the older records — the definition says so and names both.

### atlas_stage

The staging system: the versioned schema (`atlas_stage_v1.sql`) that every staging database carries, and the discipline around it. A staging database has exactly five tables — `_schema_version`, `items`, `chunks_raw`, `notes`, `kv` — holds raw, un-embedded chunks, and **never stores vectors**, which is what makes staging databases cheap to write and cheap to branch. The schema is versioned by adding a new vN artifact, never by editing an applied version in place: a staging database is promoted by being read, never by being migrated. See also stage unit, promote.

### attestation

The per-batch ingest record (#6045): an in-toto-shaped statement binding a batch's chunks to the sources they came from, built on the #3125 provenance substrate. Attestation is the step after promotion in the target pipeline — staging → classification → ingest-gate promotion → per-batch attestation — so that any chunk in the corpus can be traced to the batch statement that admitted it. A design-and-substrate term today: the record shape is designed and its substrate has landed in phases; it is not yet emitted for every batch.

### ceiling

Short for sensitivity ceiling: the highest sensitivity a caller may be shown, enforced **before ranking**, never after. The ladder's canonical names are `public &lt; internal &lt; private &lt; sensitive`; in code the rungs are still spelled `shareable &lt; personal &lt; local-only &lt; secret-bearing` (rename: #13506). A search's effective ceiling is a minimum of independent caps — the caller's grant, the client token's cap on the service, and the disclosure context's ceiling — and neither side can widen the other. Content above the ceiling is absent from results, not down-ranked. The full model is in [Sensitivity tiers and client ceilings](../concepts/sensitivity.md).

### chunk

The unit of search: one slice of one message (the corpus is sessions → messages → chunks; chunks run to roughly 3,500 characters in the canonical pipeline), deduplicated by content_hash and embedded as one vector. A chunk carries its provenance — session, client, sensitivity — and is what search returns, what `get-chunk` fetches by id, and what every count in the census ultimately counts. See [Backup, restore and the census](backup-and-census.md) for the frozen count: 107,639 chunks at 2026-10-31.

### content_hash

The chunk identity every store shares: a hash of the chunk's content, `UNIQUE NOT NULL` in the publish and shard schemas, and the join key wherever id spaces differ — vector side stores are mapped onto the corpus by content_hash, never by chunk id, because an id join would silently attach wrong vectors. The golden-set corpus fingerprint is likewise built from sampled content_hashes. One honest caveat, recorded in `test-vectors/content-hash-v1.json`: the hash's producer is not yet pinned there with an algorithm and known-answer vectors, and interop claims are forbidden until it is — the code treats as canonical an identity whose derivation no single artifact yet specifies.

### corpus tier

The **physical** sensitivity vocabulary: `public` / `internal` / `secret` — the tiers shards, staging registries, and gate records are labelled with. It is a different vocabulary from the ceiling ladder (canonically `public` / `internal` / `private` / `sensitive`, spelled in code `shareable` / `personal` / `local-only` / `secret-bearing`), and the two are two namings of one ladder by prose, not by any schema-encoded mapping. The working correspondence, in canonical names: public → public; internal and private → internal; sensitive → secret (in code spellings: `shareable` → public; `personal` and `local-only` → internal; `secret-bearing` → secret). At the frozen census the corpus held 28 internal shards, 1 secret shard, and zero public shards.

### delta DB

A vector side store holding chunks beyond the main corpus file: in the local backend's known pairs, `~/atlas/atlas-delta-chunks2.db`, where unusually the vector file and the corpus file whose `chunks` map its ids are the same file. Like every side store, it joins to the corpus by content_hash. See also topup DB.

### Durable Object cache

The Worker's live cache: one Cloudflare **Durable Object** (the `QueryCache` class) per distinct cache key, each holding exactly one SQLite row — the normalized query, its embedding, the verbatim result, and bookkeeping (timestamps, embed/search milliseconds, hit count). The whole check-and-fill runs inside the object, so concurrent misses for one key serialize and the laptop fan-out executes once. Entries move absent → embedded → complete, with the embedding persisted before the search runs. The cache **retains entries indefinitely** — no TTL, no eviction in v1 (#6805); the only deletion is an explicit per-key purge. This is not the service temp store, which is in memory and lives 10 minutes.

### executeSearch

The Worker's one search execution path: the function in `worker/src/search.ts` that both `POST /v1/search` and the MCP `search` tool call, after their own gate and validation. It derives the cache key, opens that key's Durable Object, and returns the object's hit or fill result as the search envelope. Because there is exactly one such function, ceiling pinning and cache keying cannot drift between the REST and MCP surfaces — an MCP search and a REST search for the same normalized query at the same `k` share one cache entry.

### Gate A

Embedder parity: the Rust embedder must reproduce the production Python corpus pipeline **bit-identically** — token ids equal, output vectors at delta 0.0, single and batched — on the fixed probe set in `fixtures/embed-probes.json`. The gate is version-matched: it reads the ONNX Runtime version from the runtime itself and fails as NOT APPLICABLE, rather than reporting a verdict, against a different version than the fixtures were recorded with (1.31.1).

### Gate B

Search parity: the Rust search must tie the Python production path on the **golden-311** set, against the production corpus, within the tolerances the design states. Its expectations are recorded in `fixtures/core-expectations.json`, generated from the production Python — envelopes and SQL strings the Rust parity tests must reproduce.

### Gate C

The adoption rule that closes the parity programme: whichever vector backend strategy wins the A/B comparison — exact scan, fixed-depth index, or escalating fill — is the one implemented as the default. Gates A and B prove the port is faithful; Gate C decides what the faithful port should default to, on evidence rather than preference.

### golden set

A judged query set: queries with known relevant chunks (qrels), against which rankings are scored offline. Atlas's set of record is **golden-311** — 311 queries, frozen 2026-10-26 against the merged corpus of 101,471 chunks, identified by a content_hash fingerprint — plus the held-out near-exact set of 54 queries used by the Jaccard gate. Discipline: the dev split is for iteration and parameter selection; the test split is held out and scored once, at decision time. Not to be confused with the CLI text-render goldens in the test tree, which are expected-output files, not query sets. See [Evaluation: golden sets and gates](../concepts/evaluation.md).

### hot cache

The D1 tenant hot cache: a Cloudflare D1 database (`atlas-hot-cache`) designed as a cheaper cache tier in front of the Durable Objects — one shared database, rows keyed by (tenant, cache key), each holding an opaque payload, a class (`A` / `B` / `C`), a source watermark, fill and last-hit timestamps, and a hit count, with an index for least-recently-hit eviction order. Its storage primitives (get / put / touch / evict) are complete and tested, and it has no TTL as implemented. It is **not wired into the request path** (#13105): today it caches nothing, and the Durable Object cache is the only live Worker cache.

### job protocol

The service's asynchronous search protocol over HTTP and JSON, defined by the shared `atlas-proto` crate: submit with `POST /v1/search/jobs` and receive `202` with a job id and a per-job capability token; poll `GET /v1/search/jobs/&lt;id&gt;`; page the finished, immutable ranking with `GET …/results` (cursor, limit — default 100); fetch a chunk in the job's context, or `DELETE` the job early. Fusion runs once, inside the job, over complete branch pools — which is why depth escalation lives in the vector branch and never in client paging. A one-shot `POST /v1/search` is the same machinery with the waiting done for the caller. The Worker does not use this protocol; its service calls are two synchronous posts (embed, then search).

### MRL

Matryoshka Representation Learning, as Atlas uses it: the property that a vector's first N dimensions are themselves a usable embedding. The stored Snowflake vectors are always the full 768 dimensions; a query may truncate both sides to N (for example `--trunc 384`) and renormalize, trading accuracy for speed without re-embedding the corpus. `trunc = 0` means full width, and is the production default. Evaluation numbers recorded at trunc-384 are 384-dimension numbers — one of the parity breaks catalogued in [Troubleshooting](troubleshooting.md).

### parity guard

The byte-parity drift guard for the fleet copies of the Atlas writers (#5920, contract origin #5433): a pair of scripts — the guard's own contract tests, then the comparison itself — checking that `pkgs/fleet/crates/fleet-search/src/pyjson.rs` and `render.rs` stay byte-identical to the `atlas-search` originals outside their pinned parity headers, so a fleet-rendered service envelope cannot silently drift from an atlas-search-rendered one. It runs as the `fleet-search-parity-guard` workflow — on manual dispatch only, a fact the book records because a guard nothing runs automatically is a guard that can be skipped.

### profile centroid

A **design** term, from the recommender programme: the weighted average (centroid) of the embedding vectors of documents a user has opened, saved, or searched for, with weights decaying with age — the user's profile in the same 768-dimension space as the corpus, so "more like what I have been working on" is a nearest-neighbour query over the existing index with no new infrastructure. It names a designed mechanism in [Design: the single-user recommender](../design/recommender.md), not a shipped store: no centroid is computed or kept by Atlas today.

### promote

The merge-on-close transition that admits a staging unit into the corpus (`Registry::promote`, driven by `atlas-stage promote`): the unit's staged chunks are embedded with the mandated model, written as a new core scan shard, the unit's provenance is stamped, a merge manifest is written (rows in, rows promoted, content hash, embedding model, timestamp), the staging database is branched as the unit's archive, and the unit leaves every active state as `merged`. Promotion is gated — the M6 model gate refuses a unit whose recorded embedding model is not the mandated one, whose provenance is missing, or which is empty — and the batch gate's five stages (write → classify → dedup → provenance-check → publish) must each have passed, in order, before it. See [Ingest today: staging, classification, promotion](../concepts/ingest.md).

### RRF

Reciprocal rank fusion: the fusion that merges the FTS and vector branch pools into one ranking. Each branch contributes `weight / (k + rank)` per chunk, rank 1-based, and the fused list sorts by descending score, then best rank, then chunk id. **Production k = 10**, pool 100 per branch, both weights 1.0 — the contract in `query-parameters-v1.json`, giving a chunk ranked first in both branches a score of 1/11 + 1/11 = 0.181818. The Python eval lineage uses k = 60 at different depths; that is a different configuration, not a second production value.

### shard

One database of the core tier's fan-out set, named `atlas-core-&lt;tier&gt;-&lt;topic&gt;-&lt;nn&gt;`. A scan shard holds `chunks`, its vector table, and an `atlas_manifest` self-description (tier, topic, part, expected chunk count, metric `cosine`, embedding model). Shards join the fan-out by existing — discovery reads each candidate's manifest and cross-checks it — never by code change; a shard above the caller's tier ceiling, or of a tier with no credential, is never even connected. Fan-out runs an exact cosine scan per shard in parallel and merges by distance, which stays exact because the global top-k is contained in the union of per-shard top-ks. A failing shard fails the query: a partial answer that silently omits a shard is worse than an error.

### stage unit

One unit of work's staging database, registered in the staging registry: a queue task, a session, an epic, a notes batch — "many small DBs, free writes". The registry row carries the unit id, its database URI, owner, sensitivity, schema version, and a state of `open` (producers are writing), `closing` (an aggregator may pick it up), or `merged` (promoted; terminal). Producers write items and raw chunks into the unit's own atlas_stage database without mediation — sensitivity is recorded at write time, and enforced at classification and promotion, not at the write.

### sweep

Two related uses, kept apart by context. The **source sweep** is the periodic ingest pass over candidate sources — candidate filtering, guardrails, headroom checks, locking, and run reporting — whose machinery is the `atlas-sweep` binary; the standing _recurring runner_ for it (#7969) is unbuilt, which is why ingest today is still manual. The **note promotion sweep** (`tools/note_promotion_sweep.py`) is a different, smaller tool: it moves agent notes to their new homes on next touch, leaving pointer stubs behind. When an Atlas chapter says "the sweep" about ingest, it means the first.

### tombstone

The marker left where content was deleted or retired, so the absence itself is representable. In the span/join machinery, a retired version renders a `Tombstone` — built from the span's stored snippet — instead of its text, and a span whose every referencing version is retired behaves as retired. In the target ingest pipeline, deletion propagates **as tombstones**: one operation that purges across the content-addressed store, the FTS index, the vector tables, and the recommender tables together, with the tombstone as the trace-safe record that something was there and was removed. Quarantine in the ingest gate is not a tombstone — a quarantined batch still exists, unpromoted.

### topup DB

A vector side store of additional chunks staged on top of the main corpus file: in the local backend's known pairs, `~/atlas/v3/topup-chunks.db`, with its vectors in `~/atlas/v3/vec-bge-small-topup.db`. Like the delta DB, it joins to the corpus by content_hash, because its chunk ids live in their own id space.

### vec table

The per-model table holding one vector per chunk, shaped `(chunk_id INTEGER PRIMARY KEY, vec BLOB NOT NULL)` in SQLite-shaped stores, the blob being little-endian float32 in dimension order. Atlas has exactly two named vec tables: the production **`vec_snowflake_m_v2_768_f32`** (Snowflake Arctic m-v2.0, 768 dimensions; in the Turso publish it is typed `F32_BLOB(768)` with the vector index named `vec_snowflake_m_v2_768_f32_idx`) and the dormant fallback `vec_bge_small_384_f32` (bge-small, 384 dimensions). The table name is the embedding space's address: a query vector from one table's space is meaningless against the other, which is why the Worker pins the table name into its cache key and refuses any embedding that does not name the production space.
