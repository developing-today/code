# Atlas / Turso data architecture — Scaler-era revision

Design doc — 2026-11-04. Companion to the Atlas binary+service design
(`doc/2026-11-04T08-07-13Z_design_atlas_binary_service/`, epic #5350) and to
dezren39/hatch-personal#20 (writer stall + vector index) and #21 (search
improvements; Drew's (b)+(c) ruling). This note revises the _data_ side of
Atlas for the Turso **Scaler** plan Drew signed up for on 2026-11-04. The
binary/service design stands unchanged; this note changes where data lives,
how it is written, and how it is cached.

Scaler allowances used throughout (turso.tech/pricing, verified
2026-11-04): unlimited databases; 24 GB storage included (+$0.50/GB);
100 B rows read/mo; 100 M rows written/mo; 24 GB syncs/mo; 30-day
point-in-time restore; 14-day audit logs. Our October actuals on the old
plan: 257.2 M rows read (0.26 % of the Scaler read allowance); atlas was
3.8 GB of the old 5 GB org cap by web counters, 4.8 GB by `turso db show`
(the planning number, §2 C3). **Storage and syncs are the only allowances
that can bind; reads and writes are effectively unbounded at our scale.
Drew's directive: optimize for speed, not cost.**

Corpus constants: 107,639 chunks; Snowflake `arctic-embed-m-v2.0` vectors,
F32_BLOB(768) = 3,072 B/vector raw; production atlas 4.8 GB total, of
which `messages` (the hydration source both search implementations JOIN)
is 1.15 GB.

---

## 1. What a "write" is on production atlas, and why one took 1,345 s

### 1.1 Anatomy of a write

On hosted Turso (libSQL server), a write transaction is shipped to the
database's **primary**, applied by a **single writer** (writes to one
database serialize on the libSQL engine atlas runs; the concurrent-writer
engine is a separate opt-in database type — see §6), and appended to the
WAL. Production atlas has exactly **one location** (`aws-us-east-1`,
group `great-star-lord`, verified by `turso db show` 2026-11-04), so no
hosted-replica fan-out is in the write path today; post-AWS-migration,
Turso's multi-region story is embedded replicas (§5.5), not hosted edge
replicas (deprecated 2025-05). Every other write to the same database —
DDL included — queues behind whatever the writer is currently doing. A
client-side timeout does not cancel the server-side statement: the
Nov 3 manifest insert eventually landed (HTTP 200, affected_row_count 1)
after 1,345.5 s of server-side time, and the six earlier attempts that
"timed out" were the same insert queuing, failing harmlessly on the
primary key when they finally ran.

So "a write" for us is never just our row. It is a position in a
single-file queue shared with every DDL statement, index operation, and
bulk load touching the same database.

### 1.2 The 1,345 s insert — proven facts vs hypotheses

Proven (recorded on hatch-personal#20):

- **F1.** The insert landed after 1,345.5 s server-side; reads and search
  were never affected during the stall.
- **F2.** Seven hours later, with no intervention, a scratch-table probe
  (create / insert / read-back / drop) ran every step in 0.2–0.9 s
  (2026-11-04 01:24 CST). The stall cleared on its own.
- **F3.** The stall is reproducible on demand by index DDL: `REINDEX`
  (2026-11-04 01:29–~02:44) ran ~75 min server-side, held the write path
  the whole time (a probe DROP blocked >110 s), and still produced a
  0-row shadow. On this database, `CREATE INDEX` (01:25) and `REINDEX`
  both left the shadow empty, while post-creation row writes populate
  it incrementally (the §1.3 rewrite's shadow count climbs as rows are
  rewritten — that much is empirically certain). **Correction to the
  earlier framing:** Turso's own documentation states the opposite of
  "no backfill" — "an index created for a table with existing data will
  be automatically populated with this data"
  (docs.turso.tech/features/ai-and-embeddings). So the empty shadow is
  _not_ documented behavior; it is an observed hosted defect or limit
  on this database (a public migration note cited on #20 reports the
  same observation in the wild). **Distinguishing test:** create a
  scratch table with ~100 pre-existing F32_BLOB(768) rows, `CREATE
INDEX`, and count the shadow — if a small table backfills and ours
  didn't, the limit is scale/shape (107k rows × DiskANN shadow layout);
  if it doesn't, the deployed server build contradicts its own docs and
  the evidence package (our two attempts + timings) goes to Turso
  support. Either way the row-rewrite remains the working population
  path, and the test is cheap enough to run in the same maintenance
  window as the C2 drop.
- **F4.** The load's own log stops _right before_ its index step:
  `load.log` ends and the first swap agent ended silently at exactly the
  point where `CREATE INDEX` would have run against the just-populated
  107,639-row table.
- **F5.** An independent hosted measurement (cited on #20) records
  `CREATE INDEX` on a populated hosted table wedging the write path for
  20+ minutes — the same shape as F3.

Hypotheses:

- **H1 (best fit).** The load's `CREATE INDEX` wedged the single writer
  (F4 + F5); every later write — including the manifest insert — queued
  behind a dead index build until it was reaped server-side. Explains
  F1, F2, F3 with one mechanism.
- **H2.** Replica catch-up / WAL replay after the 107k-row bulk load.
  Weaker: catch-up does not block the writer for 22 minutes, and F3
  reproduced the wedge with no bulk load at all.
- **H3.** A stuck lock from the killed load client. Possible but
  indistinguishable from H1 in effect, and F3 shows no dead client is
  needed to produce the wedge.

**Distinguishing test (next time, and as a standing rule):** Scaler's
14-day audit log now records DDL server-side. If a write stall recurs,
correlate its window against audit-log DDL/index activity _before_
probing; then run the scratch probe twice — once while a known index
operation is running on a scratch table (expect the wedge: writes queue)
and once idle (expect <1 s). H1 predicts the wedge tracks index DDL
exactly. Operational rule that follows regardless: **never run index
DDL or REINDEX against a populated production table outside a declared
maintenance window; index population happens by row rewrite (§2), which
is throttleable, resumable, and visible.**

### 1.3 The rewrite in flight is the same lesson at low speed

The sanctioned fix for the index — recreate it empty, then rewrite all
107,639 rows (`INSERT OR REPLACE`, batches of 100) so incremental
indexing populates the shadow — has been running since 2026-11-04
01:23 CST in segments (laptop unit `atlas-index-rewrite.service`,
relaunched 11:48 CST from offset 10,500 after the morning's stop/go).
Live state at 2026-11-04 ~15:02 CST: **main segment 11,700 / 97,139
rows, rate decayed 3.15 → 2.11 → 1.01 rows/s** (batches of 100 now take
92–157 s). At 1.01 rows/s the main segment needs ≈ 23.5 more hours
(≈ 2026-11-05 14:30 CST), plus mop-up of the ~6,600 rows in
previously-failed batches. Each rewritten row costs ~1 s of writer time
— incremental index maintenance on a 160,576 B/row shadow structure
(§2) through the single writer — while unindexed scratch writes run
0.5 s per transaction (F2). The write path is not broken; it is
_serialized and expensive per indexed row_. Nothing about the current
run should be disturbed; §4/§5 design so that no future index build
ever again has to be a 26-hour single-file queue.

---

## 2. Config inefficiency audit — current atlas setup

| #   | Finding                                                                                                                                                                                                                                                                                                                                                                                                                                                             | Evidence                                                          | Disposition                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| C1  | **Vector-index shadow costs 160,576 B/row** — 52× the 3,072 B vector it indexes. Full index ≈ 17.3 GB.                                                                                                                                                                                                                                                                                                                                                              | Measured uniform per-row shadow size, 2026-11-04 inventory        | **Accept for this build** (Scaler: 4.8 GB + 17.3 GB ≈ 22.1 GB ≤ 24 GB included, ~1.9 GB headroom). **Spike the cause** before indexing any second corpus: a 52× blow-up is a representation bug (likely per-row page slack in the shadow's vector+graph layout), not a law of nature. Never index a staging DB.                                                                                                                                                                                                                       |
| C2  | Legacy **BGE vector table (~221 MB)** still stored beside Snowflake vectors; dormant, flag-reachable fallback only.                                                                                                                                                                                                                                                                                                                                                 | Swap record 2026-11-03; drop was probe-gated during the DDL wedge | **Fix after the rewrite completes:** drop it in the same maintenance pattern (probe first, then DROP), freeing ~221 MB and removing a second vector column from every schema copy. Until then it is also the only rollback if the Snowflake gate ever re-opens — keep until the index verifies.                                                                                                                                                                                                                                       |
| C3  | **Location topology — audited, already minimal.** An earlier read ("org at 4/3 locations") suggested hosted replica fan-out in the write path. Direct check (`turso db show atlas`, 2026-11-04): **one location, `aws-us-east-1` primary, group `great-star-lord`** — there is nothing to trim, and no hosted fan-out cost exists to remove. Turso deprecated hosted edge replicas in the 2025 AWS migration; locality is now our job via embedded replicas (§5.5). | `turso db show` 2026-11-04                                        | **Accept + note:** keep the primary in us-east (closest hosted region to the laptop). The old "locations over limit" figure was org-level accounting on the retired plan model, not atlas replicas. Also: the CLI reports atlas at **4.8 GB** vs the 3.8 GB the web counters showed on 2026-11-04 — C8's counter-lag rule applies to our own notes too; 4.8 GB is the planning number (post-rewrite total ≈ 22.1 GB, still inside 24 GB included, headroom ~1.9 GB — thinner than earlier estimates, which strengthens the C1 spike). |
| C4  | `messages` (1.15 GB, hydration) co-resident with chunks+vectors+FTS in one DB: every scan-store operation drags the hydration store's pages, and the single writer is shared by publish, search-adjacent writes, and DDL.                                                                                                                                                                                                                                           | DB inventory                                                      | **Fix in rebuild (§4):** core splits into a scan store (chunks, vectors, FTS) and a content store (messages/full text) per sensitivity level. Interim: accept.                                                                                                                                                                                                                                                                                                                                                                        |
| C5  | **Scan-based search** while the index is down: 6.18 s wall / 5.94 s server for the pool-100 branch (Nov 3); Gate B golden queries 18.5–21.9 s end-to-end for the Rust scan arm (Nov 4, rewrite idle).                                                                                                                                                                                                                                                               | #20 measurements; gateb-results 2026-11-04                        | **Accept as the designed default.** The sensitivity ceiling must apply _before_ ranking — a pre-filtered scan is the correctness-preserving shape; the index (when verified) is an accelerator for unfiltered hot paths only, arbitrated by the golden-311 A/B (Gate B: parity confirmed, Δhits 0, ΔMRR +0.0016). See Appendix A for today's degraded in-flight numbers and why they strengthen §4/§5.                                                                                                                                |
| C6  | **Cold-start model load** (~28 s sentence-transformers per process, ~35 s cold Python run).                                                                                                                                                                                                                                                                                                                                                                         | #21 thread 3                                                      | **Retired by the Step-6 cutover:** the Rust binary (embedded ONNX embedder) is the production CLI as of 2026-11-04. The remaining cold cost is per-query, not per-process — solved by §3 caching + §5.5 replicas, not by process lifetime. **Gap found:** the binary's `serve` role exists but **no search service is currently running on the laptop** (port 18760 refused; the wrapper falls back to standalone with a warning). Standing the serve role up is an operational follow-up, recorded in Appendix A.                    |
| C7  | One DB is simultaneously the publish target, the search store, and the DDL playground — every maintenance operation queues behind (or blocks) serving traffic on one writer.                                                                                                                                                                                                                                                                                        | §1, all of #20                                                    | **Fix in rebuild (§4):** staging tier takes all routine writes; core receives only aggregator promotions in bounded batches; DDL happens on staging/branch copies first.                                                                                                                                                                                                                                                                                                                                                              |
| C8  | Turso size counters lagged direct reads all day on 2026-11-04 (org showed headroom that direct table reads contradicted).                                                                                                                                                                                                                                                                                                                                           | Rewrite watch logs                                                | **Accept + rule:** treat _write failures_, not the size counter, as the storage wall signal; the rewrite already records and retries them.                                                                                                                                                                                                                                                                                                                                                                                            |

---

## 3. Caching design — query embeddings, results, and the new flags

For the search service (#5354 shape; this section adds the cache layers
and the client flags Drew asked for).

### 3.1 Query-embedding cache — cacheable forever

A query vector is a pure function of (normalized query text, model id,
model revision). Identical inputs give bit-identical outputs (proven for
the Snowflake embedder in the Gate A spike). Therefore:

- **Key:** `sha256(model_id || model_rev || normalized_text)` → F32 vector.
- **Stores:** (a) service memory (LRU, ~10k entries ≈ 30 MB); (b) a small
  Turso **cache DB in the internal tier** (§4) so every client of the
  service shares it — writes are best-effort and asynchronous (a cache
  write must never sit in the request path; §1 is why).
- **Invalidation:** none by time. A model change changes the key. No TTL.
- Expected hit shape: agent fleets repeat queries constantly (lane
  prompts, watchers, retries) — this removes the embedder from the hot
  path for all of them, even before `serve` mode's resident embedder.

### 3.2 Result cache — keyed by corpus snapshot

A fused ranking is a function of (query, corpus state, ceiling, flags).
Corpus state is named by the **snapshot id**: the `vec_manifests` /
publish-manifest hash of the core stores (§4 aggregator writes a new
manifest row per promotion batch).

- **Key:** `sha256(query_text || snapshot_id || ceiling || flags)`.
- **Reuse rule:** while the snapshot id is unchanged, a cached result is
  _correct_, not merely fresh — return it as-is.
- **TTL backstop:** 24 h, purely as a garbage-collection bound; expiry
  never changes correctness, only memory.
- **Store:** service memory (per #5354: immutable fused ranking, paged by
  the job protocol). Cross-client sharing via the internal-tier cache DB
  is a phase-2 option, off by default — cached results carry
  ceiling-gated snippets and must not outlive the process enforcing the
  ceiling (the #5354 privacy rule stands).
- **Invalidation:** a promotion batch bumps the snapshot id →
  all result-cache keys rotate at once. No per-key surgery.

### 3.3 Flags (CLI + service protocol)

- `--no-cache` — bypass both cache _reads_; results still land in the
  result cache (refresh semantics). For eval runs and debugging.
- `--wait` — block until the full fused result is ready. Without it,
  service mode returns a job handle + first page immediately and the
  client pulls until satisfied (fetch-to-fill, #5354). CLI one-shot mode
  implies `--wait`.
- `--expand N` — after ranking, hydrate the top N hits to full chunk
  text (and the source file/document where the chunk maps to one)
  instead of snippets. Hydration reads the content store (§4), never the
  scan store.
- `--raw` — return ungrouped rows: no dedupe, no per-source grouping, no
  snippet shaping. For tooling that wants the ranking exactly as the
  branches produced it (pre-fusion rows available as `--raw=branches`).

All four flags are part of the result-cache key, so a `--raw` answer is
never served to a grouped request or vice versa.

---

## 4. Two-tier data architecture

### 4.1 Staging tier — many small DBs, free writes

- **Partition function: the work unit.** One staging DB per unit of work
  that produces data — a queue task, an OpenCode session, an epic, a
  brain-dump explosion, a notes-incorporation batch. Not per repo (a
  repo never closes; it is a rollup, not a writer target) and not per
  message/chunk (absurd cardinality). A **registry table** (in the
  internal core tier) maps unit id → staging DB, owner, sensitivity
  level, state (open / closing / merged), schema version.
- **Standard versioned schema** across every staging DB (`atlas_stage`
  v1: `items`, `chunks_raw`, `notes`, `kv`, `_schema_version`). Same DDL
  file, applied by one tool; a staging DB can be promoted by _reading
  it_, never by migrating it.
- **Writes are free and unmediated:** the unit's owner writes directly
  with its own token. No cross-writer contention exists because no two
  units share a writer queue. Detail is unlimited — stage everything,
  decide at promotion.
- **Merge-on-close:** when a unit closes (session ends, task completes,
  epic closes), the **aggregator/embedder** (§4.3) reads the staging DB,
  embeds new chunks, and promotes survivors into core. It then writes a
  merge manifest (unit id, rows in, rows promoted, content hash) and
  **branches** the staging DB (Scaler branching costs no usage) as its
  archive before dropping it from the active registry. An epic/repo
  staging DB is shared working notes for the life of the epic — merge
  happens at close, or in periodic partial promotions for long-lived
  units, never by making core a shared writer.
- **Async extraction after sessions close** is the canonical producer:
  session-end hook → staging DB per session → aggregator promotes.
  Sessions never wait on core, and core never sees session-rate writes.

### 4.2 Core tier — partitioned first by sensitivity

Exactly **three physical sensitivity levels** — separate databases,
separate credentials, no exceptions:

| Level      | Contents                                                           | Default? |
| ---------- | ------------------------------------------------------------------ | -------- |
| `public`   | publishable corpora (public zettel, public docs)                   |          |
| `internal` | **everything by default** — sessions, notes, code docs, fleet data | ✔        |
| `secret`   | secret-bearing chunks (the 90 known), credential-adjacent material |          |

- **Within a level**, partition by corpus/topic: e.g.
  `internal-sessions`, `internal-notes`, `internal-codedocs`. Each core
  corpus DB is a **scan store** (chunks, vectors, FTS); one **content
  store per level** (`internal-content`, …) holds messages/full text for
  `--expand` hydration (fixes C4).
- **Reclassification is a mechanical row move:** identical schema across
  levels → copy rows to the target level's store, verify counts + hash,
  delete from source, log the move in the manifest. No transforms.
- **Default is `internal`.** A row with no explicit level lands internal;
  promotion to `public` and demotion to `secret` are both deliberate,
  recorded moves.
- **Ceilings become physical:** a client's token opens only the levels
  (and corpora) it is allowed. The pre-ranking sensitivity filter of the
  one-big-DB era disappears from the hot path — the ceiling is which
  databases the query fans out to at all (strictly stronger, and faster:
  no per-row filter cost in the scan).

**What the performance-first ordering changes — and what it doesn't.**
Sensitivity is the outermost partition only because isolation must be
physical (credentials, replicas, backups differ per level); every
performance decision _inside_ a level (sharding, indexing, replicas,
caching) is made as if sensitivity didn't exist. It does **not** make
sensitivity optional: three levels, default internal, is the whole
taxonomy — no per-row labels in the hot path, no fourth level, no
"basically internal" grey zone. Misclassification remains possible;
physical separation contains a wrong label to one level instead of
leaking it through a filter bug — and the row-move mechanism makes
fixing a label a five-minute operation instead of a re-embed.

### 4.3 Aggregator / embedder

One fleet service role (`fleet serve aggregate` lineage; runs on the
laptop): polls the registry for closing units, embeds with the resident
Snowflake model, promotes in **bounded batches** (≤ 500 rows/txn, so a
promotion never holds a core writer for more than seconds), bumps the
snapshot id per batch (§3.2), and records every promotion in the core
manifest. Core writers = exactly one (the aggregator). Core DDL =
maintenance windows only (§1.2 rule).

---

## 5. Sharding math — spending a fixed pool on speed

Premises: reads/writes allowances are non-binding (§0); storage and
syncs bind. Sharding is bought with storage/sync, spent on latency.

### 5.1 Why fan-out multiplies throughput for scan-class queries

A pre-filtered vector scan is CPU-bound per database: today's full scan
is 5.94 s server-side for 107,639 rows ≈ **18,100 rows/s per DB stream**.
The work is embarrassingly parallel — each row's distance is
independent. Split the corpus into N shards and the same query scans
all N concurrently: aggregate throughput ≈ N × 18.1k rows/s, wall time
≈ (rows/shard) / 18.1k + merge. **Rows read do not multiply** — each row
lives in exactly one shard, so a full-corpus query reads the same
~108k rows total; what multiplies is the CPU and disk IO pointed at
them. (This is the honest version of "sharding gives more CPU/disk":
it does, per query, _across databases_ — a single database's scan is
unchanged.)

### 5.2 Shard sizing

- **Scan-time budget per shard: ≤ 250 ms server-side** → at 18.1k rows/s,
  **≤ ~4,500 chunks/shard**. Today's corpus: 107,639 / 4,500 ≈ **24
  shards** for a fully-sharded corpus; wall for a full scan ≈ 0.25 s +
  fan-out/merge (~0.1–0.3 s) vs 5.94 s today — a ~10–20× latency cut.
- Fan-out has a fixed per-shard cost (connection, query setup, tail
  latency of the slowest shard). Below ~1,000 chunks/shard the fixed
  cost dominates; the useful band is **2,000–8,000 chunks/shard**. As
  the corpus grows, add shards; never let a shard exceed ~8k chunks
  without re-splitting.
- With hundreds of parallel _workers_ (the staging tier's writers, §4.1)
  the same math inverts: each worker owns its DB, so write throughput
  scales linearly with workers and no sizing is needed beyond "one unit,
  one DB" — a staging DB is typically 10–2,000 rows, far below any
  scan budget.

### 5.3 Topic-first, hash inside a hot partition

- **Partition by meaning first** (level → corpus, §4.2): scoped queries
  ("search my notes") touch only their corpus's shards; ceilings stay
  physical; merge-on-close stays human-legible.
- **Hash-shard only inside a hot partition:** when one corpus outgrows
  the per-shard budget at a shard count where fan-out overhead is still
  sane (≳ 32 shards), split that corpus by `hash(chunk_id) mod N`.
  Cost: every query to that corpus fans out to all N — acceptable
  exactly when the corpus is hot enough to need it.
- Recommendation: sessions corpus will be the first hash candidate
  (it grows fastest); notes/codedocs stay topic-only for the foreseeable
  corpus sizes.

### 5.4 Do split indexes cost or save latency?

Index _storage_ is per-row (C1: 160,576 B/row), so splitting changes
nothing about total index size — 24 shards still total ≈ 17.3 GB for
today's corpus. What splitting buys is **build parallelism**: the
current index population is one 26-hour serial rewrite because one
database has one writer; 24 shards are 24 independent rewrites that run
concurrently (each ~1/24th the rows), and a failed shard rebuilds alone.
What it costs: `vector_top_k` per shard returns per-shard top-k that
must be merged (cheap, exact), and the no-backfill rule (§1 F3) applies
per shard — population still happens by row rewrite, just in parallel.
**Decision: scan remains the default everywhere; indexes are built per
shard only for corpora that prove scan latency insufficient in the
golden A/B, after the C1 shadow-cost spike explains the 52×.**

### 5.5 Embedded replicas and the sync budget

Embedded replicas (libSQL's local-copy mode) turn reads into local
disk reads and sync deltas in the background — the real latency weapon
for the laptop, where the search service and most clients live.

- **Laptop holds replicas of:** all `internal` core scan stores +
  `public` stores (working set ≈ 4–6 GB today, post-BGE-drop), the
  cache DB, and the registry. **`secret` stores are never replicated
  off their home node** — a replica is a copy, and copies of the secret
  tier defeat its physical separation.
- **Sync budget check:** initial sync ≈ replica size (one-time; a
  ~5 GB initial sync is 21 % of one month's 24 GB sync allowance —
  schedule it alone in its month, or accept overage at $0.25/GB ≈ $1.25).
  Steady-state delta sync ≈ promoted bytes ≈ our write volume
  (≤ a few hundred MB/mo) — noise. Replicas of the _staging_ tier are
  pointless (single-writer, short-lived) — none.
- Turso's delta sync is page/WAL-based, not row-based: replicas receive
  changed pages, so a one-row promotion costs kilobytes of sync, not a
  re-download. (Atlas currently syncs nothing — `Bytes Synced: 0 B` —
  because nothing holds an embedded replica of it yet; §4's core split
  is what makes laptop replicas worth creating.)

---

## 6. Engine verification — what production atlas actually runs

**Verified 2026-11-04** (`turso db show atlas` on the laptop, turso CLI
v1.0.33; Turso docs/blog as cited):

- **Atlas runs the libSQL engine** — the CLI reports `Type: SQLite`,
  group `great-star-lord`, one instance (`aws-us-east-1`, primary),
  4.8 GB, lifetime 820,083,493 rows read / 724,875 rows written. The
  `libsql://` URL form is shared by both engines and proves nothing;
  the `Type` field is the discriminator (the HTTPS endpoint discloses
  no engine/version header — checked).
- **The Rust rewrite is a separate database type, not a migration.**
  Turso Cloud now hosts two engines side by side: libSQL (default,
  battle-tested) and **TursoDB** — "a ground-up rewrite of SQLite"
  with concurrent writes by default (docs.turso.tech/turso-cloud).
  Existing databases were **not** converted; a TursoDB database is
  created new (`turso db create --tursodb`). What migrated fleet-wide
  in 2025 was infrastructure (Fly.io → S3-based AWS), which is also
  what deprecated hosted edge replicas.
- **Concurrent writes are early preview, opt-in, open to everybody**
  (public early preview since 2026-08-03, turso.tech/blog/concurrent-
  writes-on-turso-cloud). Mechanism: MVCC with `BEGIN CONCURRENT`;
  conflicts detected at commit at **row granularity** (retry logic
  still required for hot rows). Getting it requires: (1) enable the
  feature in the account dashboard (Settings → General), (2) create a
  **new** `--tursodb` database — **no in-place conversion of atlas is
  documented**, (3) write with `BEGIN CONCURRENT` / SDK equivalents.
  No plan gate is stated; Scaler is not a blocker.

**Design consequences for this revision:**

- §1's single-writer analysis stands for atlas and for every libSQL
  core/staging DB. The two-tier design (§4) is deliberately built so
  that single-writer-per-DB is a non-issue: one work unit = one writer.
- TursoDB's natural pilot targets are the two stores where §4 still
  concentrates writers: the **registry** and the **shared
  query-embedding cache DB** (§3.1). Both are small, new, and
  disposable-by-design — a preview engine can fail there without
  touching the corpus. Pilot = create them as `--tursodb` from day one
  (after Drew enables the dashboard toggle), measure, and only then
  consider a core migration.
- A core migration to TursoDB, if ever justified, is a _rebuild_
  (create new DBs per level/corpus, aggregator replays promotions,
  repoint, verify) — the §4 machinery already makes corpora rebuildable
  from staging + manifests, so the option stays open without designing
  for it now. **Not recommended in this revision:** preview engine,
  26-hour-rewrite lessons fresh, and the scan path doesn't need
  concurrent writers.

## 7. Wider database strategy — Supabase, central Turso, lab Postgres

Today: **Supabase (Postgres) is the system of record** for agent/fleet
ops (queue, AA domains, alerts, tracking) and Turso holds the corpus.
Drew's direction: keep a central store, consider a central Turso, plan
for the large self-hosted Postgres in the lab Kubernetes cluster, keep a
small managed fallback, and keep an Oracle-runnable equivalent possible.

- **Phase A (now, Scaler era).** Supabase stays system of record for
  ops data — it works, PostgREST + RLS are load-bearing for the queue.
  Turso takes everything corpus-shaped and staging-shaped (§4), plus
  high-write append ledgers that are currently awkward in Supabase
  (e.g. the route-attempt ledger: append-only, latency-sensitive,
  read-mostly-by-scan — a staging-tier native). Rule: new data goes to
  the tier whose shape it matches; nothing migrates for its own sake.
- **Phase B (lab k8s Postgres).** The large central store is self-hosted
  Postgres in the lab cluster: system of record for ops data migrates
  there when the cluster is real, with streaming replication to a second
  node + WAL archiving (PITR) to R2. Supabase is _not_ replicated into
  it wholesale; schemas move with the versioned-schema rule below.
- **Managed fallback subset (always).** A deliberately _small_ subset —
  fleet config, queue state, provider registry, alert state — is kept
  continuously replicated to managed Postgres (Supabase + Neon as the
  two independent copies), sized so the fleet's core functions run
  degraded-but-alive if the lab is down. This is a subset by design:
  if it grows to "everything", it becomes a second system of record
  and the phases collapse.
- **Oracle variant.** An equivalent deployment on Oracle's stack is
  kept possible by two portability rules, adopted now because they are
  cheap now and expensive later: (1) **the portable core uses standard
  Postgres + a REST contract we specify** (the PostgREST shape is fine —
  it is open source and runs against any Postgres, including on Oracle
  Cloud) with **no Supabase-only features** (no auth.* helpers, no
  storage APIs, RLS expressed in plain SQL policies) inside the portable
  core; (2) **every schema is a versioned migration set** (§4.1's rule,
  generalized) so any target — Supabase, Neon, lab k8s, Oracle — is
  "apply migrations, point the REST layer at it". What stays
  Supabase-specific lives outside the portable core and is listed as
  such.
- **Backups under the new shape:** Turso PITR (30 d) is first-line
  restore for every Turso tier; branching makes restore drills free —
  drill one per quarter, starting this month (never done before).
  Nightly Drive cold-store exports stay as the cross-provider disaster
  copy for core stores. Lab Postgres: WAL PITR + the managed fallback
  subset doubles as its off-site copy of the critical slice.

## 8. Code migration backlog — into developing-today/code + the fleet binary

Drew's order: every Atlas/search/embedder script living outside the code
repo migrates in. Verified inventory (2026-11-04; laptop-only items
marked †):

**Live — migrate:**

| Item                                                                                        | Role today                                                                                    | Destination                                                                                                                            |
| ------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| `~/hatch-personal/workspace/turso-atlas/scripts/atlas_search.py`                            | Python reference search CLI (eval reference of record; production until the Step-6 cutover)   | code repo `pkgs/atlas-search/` as the permanently-kept eval/reference implementation                                                   |
| `~/hatch-personal/workspace/turso-atlas/scripts/eval3.py` + golden-311 corpus               | Eval harness (all parity gates run through it)                                                | code repo eval tooling beside pkgs/atlas-search                                                                                        |
| `~/hatch-personal/workspace/turso-atlas/scripts/embed_pass3.py`, `embed_onnx.py`            | Embedder passes (ST + ONNX) — the publish-side embedding pipeline                             | Absorbed by the §4.3 aggregator (`fleet serve` role) embedding stage                                                                   |
| `~/hatch-personal/workspace/turso-atlas/scripts/atlas-ingest.py`                            | Publish/ingest into production atlas                                                          | Absorbed by the §4.3 aggregator (promotion path replaces direct-to-core ingest)                                                        |
| `~/tools/bin/atlas-search` (sh wrapper)                                                     | Production entry: tries the laptop service on :18760, falls back to the nix-store Rust binary | Replaced by the fleet `search` domain entry once the serve role is stood up; wrapper retires                                           |
| `atlas-backup` (laptop systemd unit, from ~/tmp/atlas-backup.sh, nix-declared 2026-10-30) † | Nightly atlas.db snapshot → laptop backups + R2 + Drive, sha256-verified                      | `fleet search` domain (backup verb) or a fleet serve maintenance role; verify the flake version matches the surviving shell copy first |
| Snowflake model provisioning (`atlas-model-rev.tar`, model state dir)                       | Provisions the pinned model rev to clients                                                    | The #5355 nix provisioning path (already designed, Q1 in the binary/service note)                                                      |

**Historical — retire/archive after the migration lands:**

| Item                                                                                                                                                     | Note                                                                                                                                                                                     |
| -------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `~/atlas/v3/swap-2026-11-03/` † (~40 scripts: load_prod, gate/fix_gate, probe__, rw_rewrite_, reindex, verify_*)                                         | One-off swap/gate/remediation tooling. The rewrite scripts stay until the in-flight rewrite verifies, then archive as the #20 evidence record. staging.db (462 MB) † archives with them. |
| `~/hatch-personal/workspace/turso-atlas/docs/` (bakeoff + design notes, Oct 26–31)                                                                       | Design history; the living designs are the code-repo doc/ notes. Archive in place (hatch-personal is the record), no code destination.                                                   |
| `/home/hatch/atlas-model-rev.tar` (1.2 GB), `atlas-step6-closure.nar` (300 MB), `~/tmp/atlas-migrate/`, `~/tmp/atlas-parts/`, assorted proof/backup logs | Deployment-transfer artifacts of the Nov 4 cutover. Delete once the provisioned model + nix closure are confirmed reproducible from code (~1.5 GB reclaimed).                            |

## Appendix A — measurements, 2026-11-04

Read latencies for `atlas-search search "fleet binary consolidation
status"` (k=10, pool=100, hybrid, scan backend), production corpus:

| When         | Path                                           | Wall                                               | Note                                                |
| ------------ | ---------------------------------------------- | -------------------------------------------------- | --------------------------------------------------- |
| Nov 3        | Python CLI (laptop), scan branch only          | 6.18 s (5.94 s server)                             | pool-100 branch, #20                                |
| Nov 3        | Python CLI cold, end-to-end                    | ~35 s                                              | ~28 s of it model load                              |
| Nov 4 ~11:13 | Gate B golden set (laptop, **rewrite idle**)   | py-scan 14.9–26.2 s; rs-scan 18.5–21.9 s per query | parity: Δhits 0, ΔMRR +0.0016                       |
| Nov 4 ~15:2x | Rust binary standalone (sandbox), cold         | **582.9 s** (7.2 s CPU)                            | service on :18760 not running → standalone fallback |
| Nov 4 ~15:3x | Rust binary standalone (sandbox), warm attempt | **did not finish** (>479 s, killed)                | no output produced                                  |

Today's two runs are **~25–30× worse than this morning's Gate B** on
comparable work, and wait-dominated (no local CPU). Two candidate
causes, not yet distinguished: **(H-A)** the primary is shared with the
in-flight index rewrite, whose incremental index maintenance is
consuming the server (batches of 100 rows take 92–157 s — the box is
busy doing exactly the wrong thing to share with scans); **(H-B)** the
sandbox standalone path moves scan data differently from the laptop
eval path. **Distinguishing test (post-rewrite, two single runs):**
repeat this exact query (1) from the laptop with the rewrite complete,
(2) from the sandbox. If (1) ≈ Gate B and (2) ≈ (1), H-A was the cause
and §4's separation (serving stores never share a writer with
maintenance) is the structural fix; if (2) ≫ (1), the standalone scan
path needs the serve role/replica treatment first. Do not run these
during the rewrite — the degraded number is already decision-useful,
and the rewrite's footprint discipline (§1.3) wins.

**Operational follow-ups from this appendix:** (i) stand up the binary's
`serve` role on the laptop (it exists; nothing is listening on :18760);
(ii) production search should be considered degraded until the rewrite
completes (~2026-11-05 14:30 CST at the current rate).

Write-latency evidence (no new probe — **the scratch write probe was
skipped by rule because the index rewrite is in flight**): the
2026-11-04 01:24 CST scratch probe (all steps 0.2–0.9 s, §1 F2) plus the
rewrite's own batch timings (92–157 s per 100 _indexed_ rows) are the
current state of knowledge: unindexed writes are fast; indexed-row
writes cost ~1 s/row through the single writer.

## Appendix B — open questions for Drew

1. C1 spike priority: explain the 160,576 B/row shadow before any second
   corpus is indexed — recommended yes, sequenced after the in-flight
   rewrite verifies.
2. TursoDB pilot (§6): enabling the account-level concurrent-writes
   preview toggle is Drew's dashboard click; the pilot itself (registry
   - embedding-cache DBs born as `--tursodb`) is recommended.
3. Cache DB in the internal tier (§3.1): shared query-embedding cache is
   the one new always-on write path into a core-adjacent DB — async and
   best-effort by design; confirm acceptable.

## 9. Addendum (2026-11-04, later) — Supabase placement audit: what moves out of Postgres

Added the same day, answering Drew's follow-up question (17:00 CST):
_is there anything in Supabase that would be appropriate to move out,
to get a cleaner definition of what belongs in Postgres vs what
belongs in Turso/D1?_ Answer: yes — most of it, by volume. This section
extends §7 without changing any of its placements (Phase A Supabase as
system of record; Phase B lab Kubernetes Postgres; the small managed
fallback subset on Supabase + Neon; Turso for corpus and staging).

**Method.** Bounded, read-only inventory of the `muse-dev2` Supabase
project on 2026-11-04 via the Management API (`information_schema`,
`pg_tables`, `pg_stat_user_tables`, `pg_class`): no writes, no locks,
no row scans of live tables. Counters are `n_live_tup` estimates;
`pg_stat` counters (inserts/updates/scans) cover the window since the
stats reset on 2026-10-29 (the 2026-10-30 cutover era), i.e. roughly
six days of fleet operation. Totals were verified with a second,
independent read: **120 tables, ~473k live rows, ~345 MB of relation
size (358 MB database)**. Two structural facts drive the placements:
foreign keys exist only in `agent` (8), `ainventory` (8), `alerts`
(2), `engineering` (2) and `ideas` (1) — the sheet-derived domain
schemas have none — and RLS is enabled broadly but carries only 2
actual policies (both in `cron`); real access is the service-role /
PostgREST contract the queue-broker already uses, which is exactly
§7's portable core.

### 9.1 The placement rule

Data belongs in Postgres only if it is original, multi-writer
transactional state whose correctness depends on relational
integrity, atomic cross-row updates, or the PostgREST/service-role
contract — queue and alert state and small relational cores — and
that Postgres is, per §7, Supabase in Phase A and the lab Kubernetes
Postgres in Phase B, with only the small named fallback subset (fleet
config, queue state, provider registry, alert state) continuously
replicated to managed Postgres (Supabase + Neon). Everything else
follows its shape out of Postgres: append-only streams, mirrored or
derived analytics, raw staging rows, and search/query corpora belong
in Turso, partitioned per §4's staging/core tiers, because they are
rebuildable or single-writer appends that never needed cross-row
transactions and only pay Postgres's costs; tiny, read-mostly,
edge-shaped configuration and KV-style state (a handful of rows, no
joins, no transactions) belongs in D1; and anything that is a
duplicate snapshot, an empty shell, or a Supabase platform schema
belongs nowhere — it is retired or left behind, never migrated. New
data is placed by this rule at creation, and no table migrates for
its own sake.

### 9.2 Placement table (every schema/table group)

Counts are live-row estimates and total relation size at audit time.
Classes: **KEEP** = stays in Postgres; **→ Turso** / **→ D1** =
migration candidate (design-level only in this pass — no data moves);
**RETIRE** = dead/duplicate/platform, no migration.

| Group (schema.tables)                                                                                                                                                                             | Tables | Live rows | Size     | Class   | Reason                                                                                                                                                                                                                                                                                                                       |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | --------- | -------- | ------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `agent` queue core: `tasks`, `leases`, `workers`, `lanes`, `checkpoints`, `watermarks`, `repo_task_lane_ledger`, `repo_snapshots`, `token_expirations`                                            | 9      | 9,652     | 39.8 MB  | KEEP    | The fleet queue itself: `tasks` alone shows 89,846 updates against 7,162 inserts and `leases` churns 21k inserts/deletes — multi-writer transactional state with 8 FKs, PostgREST/service-role consumers (queue-broker), and §7 names queue state in the managed fallback subset.                                            |
| `ainventory` relational core: `models`, `provider_policies`, `notes`, `price_events`, `benchmark_results`, `measurement_events`                                                                   | 6      | 1,994     | 2.7 MB   | KEEP    | Small original relational core holding ainventory's 8 FKs and feeding its views (`v_current_measurements`, `v_current_prices`, `v_rankings`, `v_threshold_metrics`); provider registry is also named in §7's fallback subset.                                                                                                |
| `alerts`: `incidents`, `incident_events`, `notification_sends`, `push_subscriptions`                                                                                                              | 4      | 18        | 0.2 MB   | KEEP    | Alert state is explicitly named in §7's managed fallback subset; tiny, transactional (notification sends, incident lifecycle, 2 FKs).                                                                                                                                                                                        |
| `cron`: `job`, `job_run_details`                                                                                                                                                                  | 2      | 12,559    | 2.3 MB   | KEEP    | pg_cron platform tables (the only 2 RLS policies in the project are here); they live and die with the Postgres instance, they are not fleet data to place.                                                                                                                                                                   |
| `agent` append streams: `events`, `quota_records`                                                                                                                                                 | 2      | 166,965   | ~69 MB   | → Turso | Append-only ledgers: `events` is 159k inserts / 0 updates in the window, `quota_records` is periodic plan/quota snapshots (jsonb payload, insert-only) — the §7 Phase A "high-write append ledgers" shape, staging-tier natives.                                                                                             |
| `agent` mirror + stats: 20 `mirror_*` tables (`mirror_sessions`, `mirror_agent_agents`, `mirror_scheduler_job_runs`, token/session/run `*_daily`/`*_hourly` aggregates, `mirror_export_state`, …) | 20     | 171,236   | 136.9 MB | → Turso | Mirrored, derived analytics copied from Muse's local stores — rebuildable from source, aggregate-upsert shaped (e.g. token-usage daily: 200k updates re-aggregating 27k rows), no FK or transaction need; the single largest block in the project.                                                                           |
| `ainventory` staging: `sheet_rows`, `ingest_batches`                                                                                                                                              | 2      | 84,599    | 43.3 MB  | → Turso | Raw sheet-ingest staging: 84.6k jsonb row blobs keyed by import batch — §4 staging-tier data, partitioned by ingest batch/work unit, merge-on-close.                                                                                                                                                                         |
| Sheet-domain staging: `sheet_rows` + `ingest_batches` in `automation`, `engineering`, `finance`, `home`, `research`                                                                               | 10     | 3,962     | 3.5 MB   | → Turso | Same raw-staging shape as ainventory's, one copy per domain; staging belongs with the staging tier, not the system of record.                                                                                                                                                                                                |
| Sheet-domain curated rows: `events` in `automation`, `engineering`, `finance`, `home`, `research`, plus `engineering.records` and `ideas.ideas`                                                   | 7      | 8,385     | 9.6 MB   | → Turso | Sheet-derived, dedupe-keyed query/search corpus: zero FKs in the domain schemas, insert-mostly, read by scan — corpus-shaped under §7, not transactional state. (`ideas.ideas` counted here: 554 rows, sheet-mirrored with `mirrored_at`.)                                                                                   |
| `ci.events`                                                                                                                                                                                       | 1      | 10,952    | 31.8 MB  | → Turso | Append-only CI webhook ledger (10,957 inserts, 0 updates, ~2.9 KB jsonb payload/row); the fastest-growing table by bytes and a pure stream. Its `recent_failures` view is a scan query, portable to the Turso side.                                                                                                          |
| Ingest source registries: `sources` in `ainventory`, `automation`, `engineering`, `finance`, `home`, `research`, `ideas`                                                                          | 7      | 52        | 0.3 MB   | → D1    | Tiny read-mostly ingest configuration (1–42 rows per domain, no transactions, no joins that matter) — KV/config-shaped state that does not need a relational engine. Moves only together with its domain's ingest pipeline.                                                                                                  |
| `public.muse_notes`                                                                                                                                                                               | 1      | 2         | 0.03 MB  | → D1    | Two-row standalone notes table with no FKs, no views, and no evidenced relational consumer — KV-style state, the cleanest D1 shape in the project. Low priority: move opportunistically, not as its own project.                                                                                                             |
| `agent` mirror history + empty shells: `mirror_history_*` (5) + `mirror_agent_sessions`, `mirror_scheduler_job_definitions`, `mirror_tool_stats_daily` (all 0 live rows)                          | 8      | 2,440     | 3.4 MB   | RETIRE  | `mirror_history_*` are duplicate snapshots of the live `mirror_*` tables (2,440 rows, 0 index scans — nothing reads them); the other three are empty shells that never accumulated a row. Delete after a final export; do not migrate.                                                                                       |
| Supabase platform schemas: `auth` (27), `storage` (10), `realtime` (3), `vault` (1)                                                                                                               | 41     | 253       | 1.6 MB   | RETIRE  | Platform-owned, and empty of fleet data — `auth.users`, `storage.objects`, `storage.buckets`, `vault.secrets` all hold 0 rows; the 253 rows are the platform's own `schema_migrations`. §7 already places Supabase-specific machinery outside the portable core: on any Postgres move these are left behind, never migrated. |

**Counts per class:** KEEP in Postgres **21 tables / 24,223 rows**
(11,664 rows excluding the pg_cron platform log) · CANDIDATE → Turso
**42 tables / 446,099 rows (~295 MB — 94% of all live rows)** ·
CANDIDATE → D1 **8 tables / 54 rows** · RETIRE **49 tables / 2,693
rows**. The cleaner definition the question asked for: the
Postgres-proper fleet core is ~11.7k rows of queue, alert, and
registry state; everything else in the project is streams, mirrors,
staging, corpus, config, or platform.

### 9.3 What this does _not_ change

Nothing in §§1–8 moves: Supabase remains the Phase A system of
record for the KEEP set, the lab Kubernetes Postgres remains the
Phase B home for it, the managed fallback subset stays deliberately
small (this audit confirms it is — the whole KEEP set fits in
~45 MB), and Turso's corpus/staging tiers are unchanged. The
→ Turso / → D1 rows above are design-level recommendations only;
each gets its own design issue in this repo — #5690 (`agent`
append streams), #5691 (`agent` mirror analytics), #5692
(sheet-derived domains), #5693 (`ci.events`), #5695 (D1 config
candidates, gated on §9.4 Q2) — and no data moves in this pass. The `agent.events` move in particular must be sequenced
behind a consumer audit — at 54M cumulative index scans since
2026-10-29 it is the heaviest-read table in the project — and
not treated as a lift-and-shift.

### 9.4 Open questions for Drew

1. `agent` mirror tables: their source of truth is Muse's local
   stores — confirm the mirror's purpose is analytics/reporting
   (safe for Turso) and not a backup of record; if it is a backup,
   the Turso copy needs the §7 nightly cold-store export kept.
2. D1 candidates are deliberately small (54 rows total): confirm
   D1 is wanted at all for config of this size, versus folding the
   `sources` registries into the Turso staging tier with their
   domains and keeping D1 for genuinely edge-consumed state only.
