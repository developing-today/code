# Atlas v1 — proto design

**Status:** PROTO DESIGN (design-mill intake, 2026-11-12). This note is the
intake artifact for the Atlas v1 program (Drew directive, 2026-11-12 17:38
CST): the current-state inventory written against reality, the workstream
specs A–I, and the statement→issue map (§J). The summit session owning
starter issue #9091 elaborates this proto design, in the design-mill
pattern, into the full design set — many smaller designs — under program
epic #8983. Where this note conflicts with a merged design it names, the
named design wins until this program amends it.

**Program issues:** starter #9091 · A #9092 · B #9093 (+ research #9094) ·
C #9095 · D #9096 · E #9097 · F #9098 · G #9099 · H #9100 · I #9101 — all
native sub-issues of #8983.

## 0. Current state — what Atlas actually is today (verified 2026-11-12)

There is **no `developing-today/atlas` repository**. Atlas lives in
`developing-today/code` (`pkgs/atlas-search/`, `pkgs/fleet/crates/
fleet-search/`, 17 design dirs under `doc/`), in hatch-personal's
`workspace/turso-atlas/` (the original pipeline + the only real README),
and in the laptop's `~/atlas/` working tree. This note follows the 17
precedents: Atlas designs land in `code/doc/`.

- **Binaries/service.** `atlas-search` (Rust): subcommands `search`,
  `get-chunk`, `serve`, `init`, `fetch-model`. Production runs the fleet
  rehome (`fleet serve search`, laptop unit `atlas-search.service`,
  loopback :18760, read-only by construction). The Python predecessor is
  the parity reference, enforced by golden tests.
- **Search today.** One-stage hybrid: FTS5/BM25 branch + Snowflake cosine
  branch, reciprocal-rank fusion (k=10), SQL pre-filters on both branches
  (kind/topic/project/client/since/until, `--max-sensitivity` ladder
  `shareable < personal < local-only < secret-bearing`). Contract flags
  for planner, spell-correction, query-class hint, and `rewritten|both`
  variants are **accepted but unimplemented**. The hosted surface is a
  Cloudflare Worker (`POST /v1/search`) with one Durable Object per query
  key, caching results **indefinitely — no TTL, eviction, or
  invalidation** (#6805, #8312). No result caching in the standalone CLI
  (#5668).
- **Embedder.** `Snowflake/snowflake-arctic-embed-m-v2.0` pinned by
  revision, 768 dims full width, table `vec_snowflake_m_v2_768_f32`
  (typed F32_BLOB + DiskANN index). Documents embedded prompt-free;
  queries with the `query:` prompt. The Rust embedder reproduces the
  Python ONNX pipeline bit-identically (Gate A). A bge-small table sits
  dormant as fallback. Turso/libSQL has native vector support (typed
  columns, `vector_top_k` DiskANN) — already the mechanism in use.
- **Corpus.** Canonical publish 2026-10-31: 18,325 sessions / 122,124
  messages / **107,639 chunks**, sources = Muse, ChatGPT/Claude
  exports, OpenCode/OpenChamber DBs, others template-grade. Turso
  monolith `atlas` has since grown to **7.3 GB** without a verified
  recount. **29 topic shards** (`atlas-core-internal-*`, self-describing
  via `atlas_manifest`) partition exactly the same 107,639 chunks by
  topic; the shard fan-out path is a hidden experimental CLI flag
  (#5755), not the production default. Secret tier: two small DBs —
  **plus 638 secret-bearing chunks reported inside the production
  monolith (#8479, remediation undecided)**. Zero public-tier shards.
  ask-ledger (Cloudflare AI Search over 10 allowlisted operational
  snapshots) is a separate thin edge surface, not the corpus.
- **Ingestion.** Manual/script-driven; the only Atlas schedule on the
  laptop is the daily backup. The staging tier exists in code with a
  promote gate (embedding-model match required); the aggregator/embedder
  and session-close hook are not built (#7969, #6018).
- **Backups today.** `atlas-backup` daily (local atlas → laptop + R2 +
  Drive, #1515). `cold-store-nightly` exports **every** Turso DB to
  Drive — **its 2026-11-12 run failed** (30-min timeout, killed; step
  unconfirmed). Fleet Turso: 40 DBs, Scaler, 30-day PITR, **Delete
  Protection OFF everywhere checked**, overages OFF (24 GB storage hard
  wall; 11 GB used). OpenCode DB: **21.37 GB, no scheduled/off-laptop
  backup**; measured zstd ratio 23.8% (≈5.1 GB compressed); net growth
  Nov 2→12 = 0 bytes.
- **Platform facts the design uses.** Workers Paid: 1,000
  subrequests/invocation (internal services incl. DOs always 1,000);
  Durable Objects SQLite-backed: 10 GB/object, unlimited per account.
  Cloudflare Basin is real (GA 2026-10-01): Apache Iceberg over R2 —
  Pipelines (ingest), Catalog (maintenance), Basin SQL (serverless
  queries) — an analytics lakehouse, not a transactional store.

## 1. Principles

1. **Internal only.** The corpus is never exposed directly to a public
   surface; public disclosure happens only through workstream D's
   context filters.
2. **Classification and metadata are the substrate.** Every item,
   embed, and query carries versioned Metadata (§C) and per-embed
   classification (§D) from the moment it lands.
3. **Measure, don't argue.** Shard sizes, fan-out widths, storage
   layouts, and backup schemes are settled by named-oracle benchmarks
   (§A, §F, §B), not by preference.
4. **Reference, don't fork.** #9060's genealogy model, #9084's content
   plane, and #9077's identity are consumed as-is; this program amends
   them only through their own issues.

## A. Search build-out + Workers fan-out — #9092

Build `search` into the strong v1 surface:

- **Kill the dead contract.** Planner, spell, query-class hint,
  rewritten/both variants, Jaccard rescoring: each flag is implemented
  or formally retired from the contract; an advertised flag that no-ops
  is a defect. Query understanding builds on #5773/#5774.
- **Fan-out as the default serving shape.** Hosted search fans out from
  Workers across shards in parallel; Durable Objects take coordination,
  hot-shard, and cache roles. The design explores Workers connection
  fan-out empirically — fan-out width is increased only while it
  measurably improves latency (1,000-subrequest ceiling; chaining via
  queues/invocations beyond it). The experimental `--shards` path
  graduates or is replaced.
- **Shard-size benchmarks (named oracle).** p50/p95 end-to-end latency
  at fixed recall@10 against the standing eval sets (#8627), over a
  shard-size × shard-count grid, on **both Turso and D1**, using the
  production corpus shape. A configuration wins only at equal-or-better
  recall. Deliverable: the latency curves + the chosen configuration.
- **Cache semantics.** Results cache until the corpus changes (ingest
  watermark, §E). After a change, cached results may serve for a short
  specified staleness window — **unless** the caller sets the real-time
  requirement or the cache-disable flag; both are first-class at every
  surface (CLI, service protocol, Worker). This supersedes the Worker's
  current indefinite cache (#6805 policy, #8312 purge route).
- **Second embedder.** Evaluate a second embedding approach beside
  Snowflake (candidate set and criteria in #9092; prior art: the 2026-10
  –11 bake-offs, #5733, #6069's embedding-compute decision). The corpus
  must support **mixed embedders** — per-corpus/per-item model identity
  (#8623) under the §C schema — with no flag-day re-embed, ever.

## B. Backup mesh — #9093, research #9094

- **Cadence:** every Turso DB backed up on a **~14-day** cycle (inside
  the 10–15 day band). Turso's 30-day PITR covers any loss inside 30
  days; the mesh is the protection beyond 30 days and against deletion.
- **Topology:** Turso → restic deltas (stream-zstd) → R2; R2 → laptop
  and → Google Drive; the mesh **R2↔laptop↔Drive↔R2 plus Turso into all
  three** — every store can rebuild any other. D1 exports and
  laptop-local DBs land into Turso as their backup home where
  appropriate (the appropriateness line is drawn in the design).
- **Embeds are data:** vector/shard stores ride the same mesh; a
  restore rebuilds search without a re-embed pass.
- **Findings folded in as requirements:** enable Turso Delete
  Protection (or per-DB recorded exception); root-cause or replace the
  failing cold-store-nightly export leg; identify or retire the orphan
  `laptop-state` restic repo; every leg restore-verified on schedule
  (freshness watchdog #1841 is the surface).
- **OpenCode DB (research #9094):** Drew keeps it large on purpose.
  Measured: 21.37 GB, ~5.1 GB zstd, R2 cost $0.08–0.32/mo by scheme —
  storage price is not the constraint; churn rate, restore time, and
  upload bandwidth are. Targets: Google Drive + the laptop archive
  location. Deliverable: measured churn, per-table growth model, and a
  costed scheme comparison (full copy / zstd copy / restic deltas).

## C. Metadata schema + evolution doctrine — #9095

**This doctrine is shared verbatim by the Atlas corpus metadata and
the issue/PR Metadata (#9060 §12).** Amendments binding #9073 and #9074
to it are filed (comments 6839361945, 6839362599); #9060's §12 note that
`pm_meta` was only a bare `1` is superseded by this section.

1. **Version always.** Every Metadata object carries its schema version
   (`pm_meta` for issue/PR; Atlas metadata carries its own version
   field). Writers write current; **readers accept every prior version
   indefinitely. Old metadata is as valid as new metadata.**
2. **Additive-only.** A version bump may add fields and (where allowed)
   enum values. It may **never remove a field** and **never make an
   optional field required**. Embeds embedded under an older version
   run under a newer system — supporting that is the reader's job, not
   a second schema's.
3. **Breaking changes: park, batch, reclassify.** Required-ification
   and deprecation candidates go to the **breaking-changes parking
   lot** (seeded on #9095, currently empty by record). When the parked
   set justifies it: one version event applies the whole set, followed
   by a **reclassify-everything pass**. Rare, prudent, no churn.
4. **Extras.** Unlisted data lives in an **`extras`** dictionary on the
   object (root-level extras are the evaluated alternative; the design
   records the choice). **Type-directed handling:** a value whose type
   mismatches its declared field is treated as an extra (relocated to
   `extras`), never an error. **Coercion:** lossless conversions may
   coerce (int-array → string-array allowed; string-array → int-array
   only when every value actually converts); non-converting values
   relocate unchanged.
5. **Strict core enums.** `kind`, `area`, `group`, `sensitivity` are
   documented strict value sets. Each enum declares closed (no extra
   values) or open (extras accepted and preserved). **An invalid value
   in a safety enum defaults to the most locked-down value** — a typo'd
   `sensitivity` becomes the schema's most restrictive value.
6. **Optionality is the compatibility mechanism.** Optional attributes
   are hard to filter/sort — accepted, deliberately: classifying
   agents later will hold no schema-rewrite permission, and extras +
   optionality are how they still enrich the corpus. Index what's
   extracted (cf. #9074); the rest rides along.

## D. Classification model — #9096

- **Per-embed, never per-file.** Chunking (#6037 boundary policy) and
  classification cooperate at content boundaries: a transcript that
  turns from trip planning to deployment yields separately-classified
  embeds, so a permitted consumer can receive the deployment embeds
  with no boundary-crossing leakage — and the default is **not**
  "everything secret."
- **Granularity axes:** project content is shareable; direct secrets in
  content are hidden and remediated; Drew's personal-life content
  (non-coding/design) is off limits. The existing ladder
  (`shareable < personal < local-only < secret-bearing`) is the
  starting point, refined by the design; sensitivity is a §C strict
  enum.
- **Disclosure contexts.** Access is a function of *surface*, not only
  identity: fleet task agents see nearly everything; a result posted
  to a public surface (e.g. a GitHub comment) is filtered to
  public-allowed content even when the searcher could see more. §A's
  search surfaces and §G's query records both carry the context.
- **Standing items decided here:** the #8479 remediation (638
  secret-bearing chunks in the monolith: purge / reclassify /
  move-to-secret-tier) is decided inside this design, with #6021
  revalidation and #6052's zones as inputs.

## E. Corpus ingestion — constantly updating — #9097

Sources, all internal-only: **(1)** Turso queue + PM stack data;
**(2)** all GitHub issues/PRs/commits/files for all orgs/repos in
scope (extending #6023); **(3)** all Muse conversations and data;
**(4)** all OpenCode sessions and data — with an explicit investigation
of ingesting **thinking traces** (volume/value/classification) and a
recorded decision; **(5)** all ID objects across their stores;
**(6)** a **sources inventory pass** for anything missed (notes/zettel,
Fleet Reports Space, ask-ledger logs, quota records, laptop state) —
the inventory is a deliverable and each missed source becomes a
connector issue.

Mechanics: event hooks where supported, scheduled sweeps otherwise
(#7969); delta-only where possible (#6028); every item lands with §C
Metadata and as much §D classification as can be found or applied.
The ingest watermark is the corpus-change signal §A's cache consumes.
A canonical recount procedure is part of the design — counts never
freeze silently at a publish again.

## F. Embed-storage economics — #9098

**Question (to be settled by measurement):** Turso stores embeddings +
a content-addressed hash of the value + per-embed metadata; raw content
lives in the R2/RustFS/Drive content-addressed system. Does the Turso
saving beat the R2 cost, at acceptable hydration latency?

- Composition audit first (vectors ≈331 MB of the monolith's 7.3 GB —
  the audit establishes what the rest is before savings are claimed).
- Both-ways prototype on a representative slice: storage $, R2 $,
  latency on hydration-heavy paths, plus the hot-cache (Turso/D1)
  variant. Offload path: R2 → laptop fallback → lab RustFS (#9087).
- **Boundary:** the general reference-vs-copy question and the GC
  design belong to #9084 (cross-ref comment 6839363279); this
  workstream is its Atlas measurement input and adopts its GC outcome.
  Atlas prior art: #3125.

## G. Query-as-data + result archive + versioning — #9099

- **Every query joins the corpus.** The query becomes an embed stored
  in Atlas; that embed attaches to its results. The query record keeps
  verbatim: query text, **every flag and parameter**, the §D disclosure
  context, and the **hash + version of every tool involved** (search
  build, embedder model revision, corpus watermark, §C schema version).
  Built on #8245's atlas-query-trace/v1 — extended, not replaced.
- **Result archive.** All results cached (§A serving semantics); the
  archive investigates **keeping cache/result data forever** so a
  query's answers can be diffed across time (corpus, embedder, planner
  changes). Growth is modeled; the archive is itself classified corpus
  inside §B's mesh.
- **Versioning.** Every Workers deploy is versioned and recorded;
  Turso and D1 schemas are versioned in a registry, with migration
  discipline aligned to §C where stored metadata is concerned.

## H. Virtual filesystem over the content-addressed store — #9100

- Bytes land **content-addressed, immutable, uploaded once**. Paths
  are a first-class virtual filesystem over them: "everything
  referencing `geordi.png`" returns every location naming those bytes.
  Same bytes in five places = **one upload + five location records**;
  each record carries its own metadata (path, host, date, originating
  process) attached to that location's records/embeds. Locations can
  be added/edited without touching bytes.
- **Aggregated index rule:** hash/filename/path lookups never scan the
  corpus. The VFS index is aggregated — its own Turso DB or dedicated
  tables maintained beside the bucket so the bucket works standalone.
- **Big-data evaluation (required):** Parquet vs **Cloudflare Basin**
  (Iceberg over R2: Pipelines/Catalog/Basin SQL — analytics fit, not
  transactional) vs the Turso index, judged on the VFS query shapes,
  update cost under §E's constant ingestion, and operational weight.
- **Boundary:** the store, GC, and sensitivity tiers are #9084's; a
  location record *is* a reference in #9084's sense. Prior art: #3125.

## I. Worker instructions rollout — #9101

After §A's surface is stable: every fleet worker's instructions gain
a **search section** beside queue / `fleet gh` / connect — the four
runbook siblings (repo-copy-first workflow) and the lane job bodies,
plus OpenChamber's saved scheduled-task prompts (a known stale-copy
bypass needing its own pass). The section teaches: when to search
before re-deriving; the v1 flags (ceilings, real-time, cache-disable);
classification-aware use (never paste internal-only results onto a
public surface); query provenance under §G; and ask-ledger vs Atlas
search — which tool for which question.

## J. Statement → issue map

| Drew's statement (2026-11-12 17:38 CST) | Home |
|---|---|
| Build the search subcommand out substantially | #9092 (§A) |
| Workers fan-out; explore other techniques | #9092 (§A) |
| All workers instructed on search + queue/gh/connect | #9101 (§I) |
| Benchmark shard sizes for lowest latency | #9092 (§A, named oracle) |
| Workers + Durable Objects best use | #9092 (§A) |
| Cache until corpus change; short staleness; real-time / no-cache flags | #9092 (§A) |
| Query planner + other tooling investigation | #9092 (§A) |
| Second embedding approach beside Snowflake | #9092 (§A) |
| Backup all Turso DBs every ~14 days; Turso 30d protection inside 30d | #9093 (§B) |
| Turso → restic zstd deltas → R2; R2 → laptop + Drive; full mesh; D1/laptop DBs → Turso | #9093 (§B) |
| OpenCode DB: keep large; backup cost/growth investigation (Drive + laptop archive) | #9094 (§B) |
| Corpus constantly updating; ingest queue/PM/GitHub/Muse/OpenCode(+thinking?)/ID + missed sources | #9097 (§E) |
| Versioned Metadata schema on all items; many labels/categories applied | #9095 (§C) + #9097 (§E) |
| Issue/PR Metadata also versioned; schema version always included | #9095 (§C) → #9073/#9074 amendments |
| Additive-only evolution; old metadata valid indefinitely; old embeds on new systems | #9095 (§C) |
| Breaking changes parked, applied as one set, reclassify everything | #9095 (§C) |
| Extras / type-mismatch-as-extra / coercion rules | #9095 (§C) |
| Strict enums (kind/area/group/sensitivity); invalid sensitivity → most locked-down | #9095 (§C) |
| Optional attrs hard to filter — accepted; classifiers without schema permission | #9095 (§C) |
| Per-embed classification; boundary safety without all-secret default | #9096 (§D) |
| Granular sensitivity: project shareable / secrets hidden / personal-life off limits | #9096 (§D) |
| Disclosure by surface (public posting filtered even for privileged searcher) | #9096 (§D) + #9092 (§A) |
| Embeds backed up for restore | #9093 (§B) |
| Fan-out + Turso/D1 sharding for speed, cost-bounded | #9092 (§A) |
| Content offload to R2, fallback laptop → lab RustFS; hot cache in Turso/D1 | #9098 (§F) |
| Store result cache forever; compare queries over time | #9099 (§G) |
| Every query ingested as an embed attached to its results; all flags/params; tool hashes+versions | #9099 (§G) |
| Version every Workers deploy; version Turso + D1 schemas | #9099 (§G) |
| Turso stores embeddings + content hash + metadata; raw content in R2/RustFS/Drive — test the economics | #9098 (§F) |
| Content-addressed files + virtual path abstraction (`geordi.png` lookups) | #9100 (§H) |
| One upload, N location metadata records (host/date/process) | #9100 (§H) |
| Aggregated VFS index (own Turso DB / sidecar tables), no corpus scans | #9100 (§H) |
| Parquet / Cloudflare Basin / big-data fit for the metadata | #9100 (§H) |
| Internal use only; never exposed directly to public | #9091 + §D everywhere |

## K. Open questions for the summit (defaults recorded; Drew settles only the starred)

- Second-embedder candidate shortlist and the compute home (local ONNX
  vs Modal credits, #6069) — summit evaluates, no default assumed.
- Staleness window value after corpus change (§A) — default proposed:
  bounded in minutes, named in the §A design with measurements.
- ★ Whether the query/result archive (§G) is truly forever or
  forever-with-compaction — growth math may force the question.
- ★ Delete Protection exceptions, if any DB must stay deletable
  (§B) — default: none; protection on everywhere.
- Thinking-trace ingestion (§E) — investigation decides on evidence;
  escalates to Drew only if classification risk (§D) can't bound it.
