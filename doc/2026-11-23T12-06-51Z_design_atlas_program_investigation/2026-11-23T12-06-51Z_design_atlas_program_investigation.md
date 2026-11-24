# Atlas Program — Full Investigation, Next-Steps Design & Build-Out Prompt

**You are the coordinator of a program-scale investigation and build-out of Atlas** — Drew's
personal corpus search system in `developing-today/code` (`pkgs/atlas-search/`, `pkgs/atlas-core`
surfaces, the Cloudflare Worker, and the laptop corpus at `~/atlas/`). Dig into the **full
system**: what it holds, how it ingests, how it plans and ranks queries, who consumes it, and
what it should become. This prompt is a **starting point, not a conclusive or final stopping
point**. Work the design end to end, begin implementation wherever you are unblocked, and when a
pass finishes, review what remains, expand the plan, and repeat — until nothing more can be done
with the access and authority you have. Use subtasks and orchestration heavily: launch many
parallel agents (analyze, research, design, document, implement) and keep them pointed at the
frontier. A phase finishing is a signal to start the next one, not to stop and ask.

---

## 1. What Atlas is (ground truth — verify, don't re-derive)

Atlas is a unified search index over every corpus Drew produces, with provenance-first storage,
tiered sensitivity, and hybrid (FTS + vector) retrieval. Program epic:
[#8983](https://github.com/developing-today/code/issues/8983).

- **Canonical store**: laptop Turso/libSQL DB at `~/atlas/turso-atlas/data/atlas.db` plus 29
  topic shards. Last verified census (2026-10-31, on
  [#3125](https://github.com/developing-today/code/issues/3125)): **18,325 sessions / 122,124
  messages / 107,639 chunks**. Later records describe a ~7.3 GB monolith **without a verified
  recount** — the counts consumers see are frozen at the October publish while the store has
  grown. A fresh verified census is an early deliverable.
- **Provenance/CAS**: #3125 phase 1 landed 2026-11-22, phase 2 landed 2026-11-23 — backfill
  interned **12,092 content-addressed objects (~2.6 GB, 0 dedupe collisions)**: chatgpt-export +
  sandbox-agents; `ATLAS-EXTRACTS VERIFY OK`; atlas-backup covers the store. **Phases 3
  (occurrence backfill) and 4 (catalog DB + multi-shard resolution as MCP tools) remain open.**
  Staged tiers exist beside the canonical store: per-unit `atlas_stage` v1 staging DBs (promote
  gate built; the aggregator/session-close hook is not), and legacy topup/delta vector DBs
  resolved at query time by content_hash, never merged into canonical identity.
- **Code surfaces**: `pkgs/atlas-search/` (atlas-core engine, atlas-embed, atlas-proto,
  atlas-search binary; a Python reference implementation kept for parity); the **Cloudflare
  Worker** (`worker/src/index.ts` — holds no DB credentials, per-query Durable Object cache, one
  `executeSearch` path serving REST + MCP, D1 binding present but flag-off); the **Atlas MCP
  server lives in that same Worker** (`worker/src/mcp.ts`, #8625) — one `search` tool, live,
  wired into the laptop's mcpx as its remote `atlas` server; **`fleet search`**
  (`pkgs/fleet/crates/fleet-search/`) is a flag-for-flag port of the atlas-search client with a
  byte-parity guard, documented in the fleet book. atlas-core clippy cleanup #13056 is open with
  PR #13097 unmerged (flag-held shepherd — not yours to merge without Drew's word).
- **Embeddings**: production model verified in code: **`Snowflake/snowflake-arctic-embed-m-v2.0`**
  (the `Snowflake/` prefix is the stored model id), **768 dims**, table
  `vec_snowflake_m_v2_768_f32`, native Rust ONNX, CLS pooling, `"query: "` query prompt,
  bit-identical to the Python reference. Vectors live in the monolith/shards as plain BLOBs —
  **`vector_top_k` does not work on them; semantic search is an exact cosine scan (~4 s)**. A
  legacy BGE-small vector space coexists (mixed-embedder by accretion). A second embedder
  (BGE-M3 / Voyage / etc.) is an open evaluation in #9092. The D1 chain (#9134: #13100 closed,
  #13101/#13103 in review, #13105 gated, #13106 flag-held, #13107 behind) is a **result/content
  cache**, not a vector store; production does not touch D1 (master flag off).
- **Retrieval today**: hybrid FTS + vector with **RRF fusion only — no learned or cross-encoder
  reranker in the serving path** (Jaccard #5733 and SPLADE sit at eval-gate stage). Query-planner
  flags (spell, query-class, rewritten) are **accepted by the API but unimplemented**
  (#9092 / #5773 / #5774). #12900 thread 1 (parallel FTS+vector branches) landed via PR #13039;
  thread 2 is the still-open multi-client product question (Drew's); thread 3 substantially
  landed elsewhere.
- **Evaluation**: golden-311 v3 + a held-out 54, an ablation harness (#5741), and a standing
  eval programme #8627 (open). Feedback capture (`atlas-feedback/v1`) exists with **no ranking
  consumer**.
- **Recommender**: design only (#9140, #9109 Part C) plus one in-flight slice — snapshot paging
  #10158 / PR #12873 (flag-held shepherd). **No candidate generation exists.**
- **Sensitivity model** (two namings of one ladder): corpus tiers **public / internal / secret**
  (28 internal shards / 107,001 chunks; 1 secret shard / 638 chunks; secret material allowed only
  in the 2 secret DBs) vs client ceilings **shareable < personal < local-only < secret-bearing**
  (default personal), enforced server-side per client identity (sandbox-scripts → personal;
  drew-cli → secret-bearing; cf-worker → personal). Machinery: classification manifest #5410
  (can narrow, never widen), gitleaks + classifier + vector-neighbourhood revalidation #6021,
  Drew's own Layer-1 review #10412 (closed), Presidio NER #6048. **Absolute rules: candidate
  secret content is never reproduced in issues, chat, or logs; secret text is never sent to a
  hosted embedder** (held out of sweeps until the local embedder #6059 lands).
- **Open wound — #8479**: the 638 secret-bearing chunks also live in the production monolith +
  a local replica, so the tier boundary does not currently hold outside the shard set;
  purge-vs-reclassify is undecided (Drew's call — surface it with options, don't decide it).
- **Census**: **57 open `epic/atlas-program`** issues (123 all-state), verified 2026-11-23.

## 2. Drew's directions (the next-steps agenda — design all of it, build what's unblocked)

### A. Integrate all data sources with automatic ingest

Ingestion today is **manual/script-driven** — the only laptop schedule is the daily backup
timer; ingest scripts still live in hatch-personal (#5670 migration open); the recurring sweep
runner #7969 is unbuilt. The target list already exists as **#9097 Workstream E** (open,
blocked): (1) Turso queue/PM data; (2) **all** GitHub issues/PRs/commits/files for all orgs
(comments are in — #6023 closed, writer #10688 landed; objects + continuous delta are missing;
external repos gated on licensing #6024 + zone policy #6052, both open); (3) all Muse
conversations/memory/artifacts (no connector issue exists yet — design the mechanism); (4) all
OpenCode sessions, plus the explicitly undecided thinking-traces investigation; (5) all ID
objects (reverse direction: #6062 + projectors #11182–#11186, all open/unstarted); (6) a
sources-inventory pass naming zettel/notes, Fleet Reports, the ask-ledger, quota/burn records,
laptop state stores (connectors #8622 memory-lancedb, #8626 memory files/notes/repo docs already
filed). **Pipeline shape to design and build**: source-of-record CDC/hooks + periodic
reconciliation crawl; detection ladder metadata → SHA-256 → full process; per-document
parser/chunking/model version stamps; chunk-level hashing so a small edit re-embeds one chunk;
staging → classification → ingest-gate promotion (#6057) → per-batch attestation (#6045);
deletion as tombstones with purge propagating across CAS + FTS + vector + recommender tables as
one operation. **Every source enters under the tier machinery (§1) — classification before
promotion, never after.** Gmail: no ingest is filed and Drew has paused all Gmail background
checks (2026-11-22) — record it as a named future source; do not build Gmail ingest without his
word.

### B. Query planning

Design and build the planner the API already pretends to have: fast classifier (<10 ms) →
conditional transforms → routed retrieval → fuse → rerank. Components: conversational query
rewriting; expansion; multi-query / RAG-Fusion; HyDE (gated to underspecified queries);
decomposition into sub-queries; step-back; **routing by source / intent / sensitivity tier**;
structured filter extraction (kind/topic/project/client/date — the filters `fleet search`
already exposes). Disciplines: adaptive per-query strategy selection instead of stacking every
transform; evaluation sliced by query class so each strategy proves itself. Minimum viable
planner: routing + filter extraction + conversational rewriting; HyDE/decomposition second wave.

### C. More search characteristics on top of Snowflake

The base is Arctic Embed m-v2.0 (768d; MRL truncation to 256d retains ~98% quality — a free
two-resolution index before any new model). Layer, each golden-set-measurable: lexical (BM25 —
exists), **learned sparse (SPLADE)**, **late interaction (ColBERT) as a top-100 mid-stage**,
metadata pre-filtering (pre-filter/routing, never post-filter — the sensitivity tier is a
security boundary and belongs in the retrieval layer), behavioural signals, and the reranker
(§D). Fix the serving physics: replace the ~4 s exact scan with an ANN path (libSQL native
`F32_BLOB` + DiskANN `vector_top_k` where the store supports it, an embedded-replica strategy,
or a dedicated index) — measure recall loss against the exact scan on the golden set. Run the
second-embedder evaluation (#9092) to a decision: BGE-M3's one-pass dense+sparse+ColBERT is the
interesting challenger; any model change follows the migration discipline — model_id-tagged
vectors, shadow index, **cutover on eval result, not on completion**.

### D. Reranking

Add a cross-encoder stage over the fused top-50–200: fixes precision, not recall; typical
+5–15 nDCG@10 at 50–500 ms CPU. Default open candidate: **BGE-Reranker-v2-m3** (568M,
multilingual) via fastembed-rs `TextRerank` (ONNX) or candle (avoids the ONNX Runtime AVX/SIGILL
liability; v2-m3 ports as plain XLM-R). Hosted rerankers (Cohere/Voyage) are offline quality
ceilings for comparison only — and never see secret-tier text. Adoption is per-corpus and
**golden-set-gated**: a reranker that hurts on Atlas's own queries does not ship; pool size is
the latency knob. Wire the feedback loop (#6046/#8062 seam, `atlas-feedback/v1`) into ranking
experiments — it is the prerequisite for any learning-to-rank claims.

### E. Recommender

Single-user system: no cross-user collaborative filtering exists to copy. The signals are
**content similarity** (decaying weighted profile centroid over the existing embedding index),
**session co-occurrence** (item-to-item CF over Drew's own sessions — the strongest single-user
collaborative analogue), recency/frequency, and graph/provenance neighbours. Same three-stage
shape as search (candidate union → rank → MMR diversity/freshness/exploration re-rank), reusing
Atlas's stack — a trained two-tower model is overkill; the pattern collapses to the hand-built
profile vector, upgradeable in place. Surfaces: more-like-this, related results beside search,
a resurface feed, query suggestions. Build on the #9140 design and land the snapshot-paging
foundation (#10158) when its shepherd hold clears; design the DO-accumulated "next 10" fan-out
from #9109 Part C.

### F. Search integration for fleet and for ID

- **Fleet**: `fleet search` is live but searches a stale corpus (§1) — freshness (§A) is the
  fleet-facing fix. Then: result caching keyed by index version, batch search for lanes,
  offline fallback via libSQL embedded replicas (native vectors work identically local/remote —
  the route sqlite-vec can't take), and search-quality telemetry from real lane queries feeding
  the golden set (§H).
- **ID**: Atlas is ID's search/embeddings layer (the two programs meet here — coordinate with
  the ID program prompt). Deliverables: the #6062 publication format, the projectors
  #11182–#11186 built and running, #9109 Part C (storage economics, ID search over the graph,
  recommendations), automatic vectorization of ID content via Snowflake + the #9092 second
  embedder. ID has **no live query path today** — this workstream creates it.

### G. MCP and programmatic access

One retrieval core, three surfaces, tier enforcement **inside the core** so every surface
inherits it: **MCP** for agents (the copyable shape is `search → read` two-step tools returning
references + snippets, never resource enumeration at corpus scale; the current server exposes a
single `search` tool — grow it deliberately: search, read, related/recommend, corpus status),
**REST** for services (with signal-provenance `explain` on results), **CLI** for humans
(`fleet search` + atlas-search). Grow `atlas-proto`'s `query-parameters-v1.json` seed into a
versioned API contract all three surfaces honour. Close #8625's remainder: cross-client
acceptance for OpenClaw / OpenCode / OpenChamber / Muse lanes (identity-stamped, ceiling +
corpus scoping enforced in branch queries, audit-logged). #12900 thread 2 (is Atlas headed for
multi-client use?) is Drew's decision — prepare the options paper; don't pre-build for a
multi-tenant answer he hasn't given. Also fix the Worker cache: indefinite with no TTL/eviction
(#6805) is not a caching strategy.

### H. Evaluation & quality programme

Everything above is gated on measurement. Grow the golden set from real traces (gold contexts
harvested from real retrieval runs — beware the 0.91-offline / 0.40-production trap), with
no-answer probes and adversarial slices (distractors, semantic twins, alias-only, long-doc).
Metrics per stage: Recall@k gates retrieval, nDCG/MRR judge ranking, Precision@k controls noise.
CI regression gate on **every** model / chunker / fusion / reranker / planner change (#8627).
Online: lightweight failure mining from real fleet/ID queries folded back into the set.

### I. Book, issue map, and the living record

1. **An Atlas book in bookwright form** (survey before structure; coordinator-fixed skeleton;
   mechanical coverage proof; zero-warning build), joining the existing code-repo Pages deploy
   under **https://developing-today.github.io/code/atlas/** (one Pages site per repo — never a
   standalone deploy). The fleet book is the form reference. Include a DOC-01-style docs-sync
   contract in the atlas README surfaces: any engine/API/model change updates its chapter in
   the same change.
2. **An issue-by-issue map of ALL Atlas issues** — every open `epic/atlas-program` issue (and
   the atlas-adjacent #3125/#9097/#9134 families), prioritized in best working order, **with all
   blockers marked both on the issues themselves and in the priority list** (book chapter +
   tracking issue kept current). Blockers are worked, not just listed.
3. **Designs + PRs** — every workstream above produces landed designs (repo `doc/` archive) and,
   wherever unblocked, merged implementation PRs. CI runs on main only; merge when verified.

## 3. How to work (fleet rules — binding)

- GitHub via the **gh CLI / REST only** (the GitHub connector is banned). File a GitHub issue in
  the owning repo the same pass for any problem found.
- Execution on the **laptop** (primaries stay on main + clean; worktrees on branches, landed by
  PR). Never `git stash` in shared clones. Never force-push main.
- Before implementing any issue: premise check + singular-ownership check (a claim comment from
  another agent is a hard stop). Re-read an issue's newest comments immediately before merging.
- Builds/tests dispatch-first (`fleet queue dispatch`), laptop fallback; a container environment
  failure is not a test result — record which route verified. Tests name their oracle.
- Evidence standard: every factual claim carries its checked source or is labelled unverified;
  counts computed from a named source, never estimated in prose.
- **Privacy absolutes** (§1): secret candidate content is never reproduced in issues/chat/logs;
  secret text never goes to a hosted embedder or hosted reranker; tier enforcement lives in the
  retrieval layer, not in client goodwill.
- Respect standing holds: flag-held items (the #13106 slice, PRs #12873/#13097 shepherds) are
  not yours to start without Drew's item-specific word. Decisions that are Drew's (#8479
  purge-vs-reclassify, #12900 thread 2, the thinking-traces question in #9097) get surfaced with
  substance — options, ramifications, recommendation — never silently decided and never re-asked
  once recorded.
- Secrets by name only, never values. Never touch the `vertexinc` org. External communications
  need per-item pre-approval. Quota doctrine: plan quotas only, never funds; a 402 is a wall.

## 4. The iteration rule

This prompt is the **starting point**. When every workstream has reported, do not conclude:
re-survey (corpus, issues, consumers, eval results), reconcile what the passes learned, expand
the design set and the issue map, and run the next pass. The program ends only when a full pass
finds nothing more that can be designed, documented, or implemented with the authority
available — and even then, the book, the issue map, and the docs-sync contract remain as the
living record the next pass starts from.
