# Turso → D1 migration audit — table-by-table classification and programme design

**Date:** 2026-11-23. **Tracking issue:** #13844. **Kind:** research + design only. No migrations, schema changes, or
data moves were made. **Method:** live inventory from the Turso platform API
(organization usage + per-database usage, read 2026-11-23 ~23:05Z, current billing
period), live table enumeration + row counts over the libSQL HTTP API for every
production database (representative shard enumerated for the 30 Atlas shards; the
frozen 2026-11-17/18 staging copies were classified from the recovery record, not
enumerated), and code ownership read from this repo. Write/read attribution below
the database level is not exposed by the Turso API; where a table group's profile
is inferred from its owning code, the text says so.

## Answers up front

- **The compile coordinator's three tables** (`placement_owner_state`,
  `placement_claims`, `placement_budgets`, #13765) are designed to be added to the
  SCHEMA list in `pkgs/fleet/crates/fleet-queue/src/turso_queue/core.rs`, beside
  `placement_sessions` — i.e. they live in the **same single Turso database as the
  rest of the queue broker store, `fleet-queue`**. They are **not yet present** in
  the live database (verified by enumeration 2026-11-23); the build is queue task 21517. Verdict for this audit: STAY-TURSO with the rest of the broker core.
- **The queue broker store is ONE Turso database** (`fleet-queue`), not several.
  The `replica_*` / `cloud_*` DDL variants in the same crate are the same schema
  aimed at embedded-replica deployments, not separate cloud databases. The
  actions-exec tables (`actions_exec_sessions`, `actions_exec_commands`,
  `actions_exec_io`) also live in `fleet-queue`.
- **Estate split (50 databases, one group):** the Atlas knowledge store (33 DBs:
  `atlas` + 30 `atlas-core-internal-*` content shards + 2 `atlas-core-secret-*`
  shards, plus `atlas-core-internal-content` and `atlas-ask-ledger`), the fleet
  plane (4 live DBs: `fleet-queue`, `fleet-platform`, `fleet-mirror`,
  `fleet-dispatch-jobs`), the coordination store (`muse-meta`), small product
  stores (`notes-submissions`, `bedside-and-away`, `voice-line-ledger`,
  `great-star-lord`), and 8 frozen staging/recovery copies from the 2026-11-17/18
  Supabase recovery.

## Measured usage (current billing period, per Turso API)

| Database                              |         Rows read |   Rows written |      Storage |
| ------------------------------------- | ----------------: | -------------: | -----------: |
| fleet-platform                        |       529,260,299 | **57,436,319** |       465 MB |
| fleet-queue                           | **6,080,653,109** |      5,766,063 |       312 MB |
| atlas                                 |       847,324,868 |      1,118,143 | **7,278 MB** |
| muse-meta                             |           569,689 |        638,371 |        97 MB |
| atlas-core-internal-content           |         5,429,548 |        178,480 |       509 MB |
| 30 atlas-core-internal-* shards (sum) |             ~6.1M |          ~360K |      ~750 MB |
| fleet-mirror                          |         1,298,613 |          5,952 |       175 MB |
| 8 staging/recovery copies (sum)       |             ~2.2M |          ~440K |      ~480 MB |
| everything else (sum)                 |               <1M |            <3K |        <2 MB |
| **Estate total (per-database sum)**   |        **~7.47B** |     **~65.8M** | **~10.0 GB** |

Three facts drive everything below:

1. **`fleet-platform` is 87% of all writes** (57.4M of 65.8M) on only 465 MB of
   data — a churn workload, not a growth workload. Its owners are Cloudflare
   Workers in `pm-stack/` (aggregator, dashboard, decision-events) writing
   through the Turso HTTP API.
2. **`fleet-queue` is 81% of all reads** (6.08B) — the broker and its pollers
   scan. That is a query-shape/indexing problem, not a store-location problem;
   moving the store would move the scans, not remove them.
3. **`atlas` is 73% of all storage** (7.28 GB of 10.0 GB) and is a vector store:
   107,639 chunks with a native libSQL vector table
   (`vec_snowflake_m_v2_768_f32`, 107,639 rows) + FTS5 shadows.

## Pricing and limits grounding

- **Turso (current plan: Scaler)** — turso.tech/pricing, read 2026-11-23:
  $29/mo monthly billing; includes 100M rows written, 100B rows read, 24 GB
  storage; overage $0.80 per million rows written, $0.80 per billion rows read,
  $0.50/GB storage. Developer ($5.99) includes 25M writes / 2.5B reads / 9 GB.
  Rows written count index touches as well as row touches (vendor FAQ).
- **Cloudflare D1 (Workers Paid)** — developers.cloudflare.com/d1/platform/pricing
  and /d1/platform/limits, read 2026-11-23: $5/mo account minimum; first 50M rows
  written/month included, then $1.00/million; first 25B rows read included, then
  $0.001/million; first 5 GB storage included, then $0.75/GB-month. Hard limits:
  **10 GB per database** (cannot be raised), 1 TB per account, each database
  processes queries **sequentially (single-writer)** — throughput per database is
  bounded by query latency, and D1 is reachable **only** from Workers bindings or
  the Cloudflare HTTP API: there is no embedded replica, no local file, no
  connection string a laptop or Rust process can hold.
- The estate already runs D1 where it fits: `atlas-hot-cache` (the #9134 design —
  a D1 tenant hot cache in front of Turso, Turso remaining the source of truth),
  `decision-events`, and `cold-store-manifest`. The bridge pattern proposed below
  is therefore an in-estate precedent, not a new invention.

## Classification

### G1 — fleet-platform telemetry mirror & aggregates — MOVE-CLEAN (first slice)

Tables: `agent_mirror_*` (21 tables, ~230K rows: sessions 45,177, token usage
hourly 46,229 + daily 34,931, scheduler job runs 17,719, histograms 15,868,
goals updates 10,870, …), `agent_quota_records` (75,024), `agg_*`
(`agg_usage_snapshots` 18,093, `agg_activity_gaps`, `agg_actions`,
`agg_store_documents`, `agg_activity_ingestion`), `agent_token_expirations`,
`agent_external_comms` (70).

- **Why it is a fit, with a writer-path correction:** the data is _derived_ —
  a mirror of Muse session/usage state plus quota
  snapshots — so it is rebuildable from source and a cutover mistake loses
  nothing canonical. No joins against Turso-resident tables. The writer paths
  are not all Workers: `agent_quota_records` is also written by the laptop
  quota gatherer and subscription recorder, and `agg_*` is written by the
  laptop aggregator through `remote-stores.ts`. These processes need the D1
  HTTP API; they cannot use a Worker binding. The `agent_mirror_*` writer still
  needs to be identified before that family can be cut over. A scheduled
  mirror sync can dual-write only after its concrete writer is located.
- **Win:** this family is the presumed majority of fleet-platform's 57.4M
  rows written/month (inference: the mirror re-upserts its row set every sync
  cycle; per-table write attribution is not exposed by the Turso API). Even at
  half the database's writes it is ~29M rows/month off the Turso meter — the
  single biggest write win in the estate.
- **D1 fit:** ~600 KB–sized working set per family table, total well under the
  10 GB cap; D1's 50M included writes absorb the churn. One caution: D1's
  single-writer serialization means the sync writer must batch (it already
  writes in sync cycles, not per-event transactions).
- **Effort/risk:** a D1 HTTP transport for laptop writers, Worker bindings for
  any Worker writers, and per-writer dual-write/parity windows. Medium until
  the `agent_mirror_*` writer and every reader are inventoried.
- **Side effect:** `fleet-mirror` (175 MB) holds a second, lagging copy of this
  same `agent_mirror_*` dataset (5,952 writes/month — a periodic copy). Once G1
  lands in D1, `fleet-mirror` is redundant and can be retired after a parity
  check (a deletion decision, outside this audit's authority).

### G2 — fleet-platform ingest families (Sheets-shaped) — MOVE-CLEAN

Tables: `ainventory_*` (sheet_rows 85,450, measurement_events 1,444, models,
price_events, findings, sources, ingest_batches), `automation_*` (events 5,908,
sheet_rows 1,853), `research_*` (events 1,714, sheet_rows 945), `engineering_*`,
`finance_*`, `home_*` — each family is `*_sources` + `*_ingest_batches` +
`*_sheet_rows` + `*_events`, written by Worker collectors and read by the
dashboard Worker.

- **Why clean:** Worker-owned end to end, append-mostly, family-local joins
  only. Move family by family into one D1 database (or one per family — D1
  allows 100K databases per account; per-family keeps each under cap and
  isolates the single-writer).
- **Win:** the remainder of fleet-platform's write churn after G1 (share is
  inference, same basis as G1), plus the `ainventory_sheet_rows` storage.
- **Effort/risk:** repetitive, mechanical — the family shape repeats six times.
  Low risk, medium tedium.

### G3 — fleet-platform operational ledgers — MOVE-CLEAN

Tables: `alerts_*` (incidents 277, incident_events 541, notification_sends 271,
push_subscriptions 0 — this is also where page-health/outage state lives),
`decision_set_events` (538), `ci_events` (35,263), `ideas_*`, `orch_*`,
`public_muse_notes`, `shared_comments`.

- **Why clean:** Worker intake/delivery/ack paths only; small row counts;
  no cross-store joins. `decision_set_events` already has a sibling D1 database
  (`decision-events`) in the same worker family.
- **Win:** small in rows, but completes the fleet-platform evacuation: G1+G2+G3
  together remove **all 57.4M rows written/month (87% of the estate)** and
  529M rows read/month from Turso, and retire a 465 MB database.
- **Effort/risk:** low; `alerts_*` is the only group with a live delivery path —
  move it last within the group, behind the bridge pattern (§Bridge) for one
  cycle.

### G4 — fleet-queue broker core — STAY-TURSO

Tables: `tasks` (18,489), `events` (376,616), `checkpoints` (9,816),
`repo_task_lane_ledger` (9,724), `lanes` (235), `workers` (110), `leases` (35),
`placement_sessions` (307), `creation_outbox*`, `merge_intents` (3), and the
incoming coordinator tables `placement_owner_state` / `placement_claims` /
`placement_budgets` (#13765); plus `actions_exec_sessions` (5),
`actions_exec_commands` (4), `actions_exec_io` (0).

- **Why stay:** the consumer is the Rust queue broker on the laptop, not a
  Worker. Its claim path is transactional (the coordinator's slot gate is a
  conditional UPDATE with the budget reservation in the same motion, #13765);
  D1 offers no embedded replica and no local access path for that process —
  every claim would become an HTTPS round trip to the Cloudflare API under
  D1's per-database sequential processing, at the exact point the design needs
  atomicity and local latency. This is the "few core components" residue in
  Drew's sketch, and it is genuinely core.
- **The real problem here is reads, not location:** 6.08B rows read/month
  against ~600K live rows means pollers are scanning. File separately:
  index/cursor the poll paths (events by id watermark, tasks by state+lane
  index). Estimated win if scans become index probes: an order of magnitude
  off the estate's read total — larger than any D1 move in this document.
- **Effort/risk of staying:** zero. Risk of moving: high, for no billing win
  (5.77M writes is 9% of the estate).

### G5 — fleet-queue read-side mirrors — MOVE-WITH-BRIDGE

Tables: `github_mirror_issues` (8,635), `github_mirror_pull_requests` (8,072),
`github_mirror_meta`, `github_mirror_tombstones`, `github_mirror_deleted_notes`;
and `agent_events_pg_history` (203,004, append-only history).

- **Why bridge, not clean:** the mirror writer runs with the broker (Turso
  side), while several readers are Workers/dashboards. Move the _read side_ to
  D1: writer dual-writes (or a periodic reconcile copies deltas by updated-at
  watermark); readers use D1 first and fall back to Turso on a miss or a stale
  watermark. Consistency window: one sync cycle (minutes) — acceptable for a
  mirror whose Turso copy is itself a mirror of GitHub.
- `agent_events_pg_history` is append-only and cold: dual-write new rows for
  one cycle, backfill once, then D1-only. (An R2/SQLite-file archive would
  also serve; D1 keeps it queryable from Workers.)
- **Win:** modest writes; the value is read locality for dashboards and
  shrinking the queue database's blast radius. **Effort/risk:** medium-low.

### G6 — muse-meta coordination store — MOVE-WITH-BRIDGE (low priority)

Tables: `coord_events` (215,450, append-only), `coord_tasks` (9,005),
`coord_workers` (34), `coord_checkpoints` (2), `coord_leases` (0),
`meta_notes` (1), `mail_messages` (10). 638K rows written/month (1% of estate).

- **Why bridge:** consumers are agent tooling scripts (the `turso-exec`
  pattern), which can reach D1 only through the Cloudflare HTTP API — workable
  for an append-only event log, awkward for lease/task claim semantics.
  Sensible split: `coord_events` + `meta_notes` + `mail_messages` move behind
  the bridge (dual-write, D1-first reads); `coord_tasks` / `coord_leases` /
  `coord_workers` stay until a consumer is Worker-shaped.
- **Win:** ~1% of writes. Do it for uniformity, not for money. **Effort/risk:**
  low-medium; defer until G1–G3 land.

### G7 — Atlas — STAY-TURSO (the embedding store)

Databases: `atlas` (sessions 18,325, messages 71,551, chunks 107,639, FTS5
shadows ~343K rows, `vec_snowflake_m_v2_768_f32` 107,639 + index shadow),
30 `atlas-core-internal-*` shards (each: `atlas_manifest`, `chunks`,
`vec_snowflake_m_v2_768_f32` — representative shard 4,435 chunks),
2 `atlas-core-secret-*` shards, `atlas-core-internal-content`
(sessions 18,235 + messages 70,978, no vectors in this DB).

- **Why stay:** the workload _is_ native libSQL vector search (`vector_top_k`
  over `vec_*` tables) plus FTS5, run by the aggregator across shards. D1 has
  no vector index; Cloudflare's vector product is Vectorize, a separate store —
  moving Atlas to "D1" really means re-platforming search onto Vectorize +
  D1 metadata, a different programme with its own bake-off, not a table move.
  Drew's sketch ("Turso remains primarily a sharded embedding store") is
  exactly right for this group: 33 databases, ~8.5 GB, already sharded.
- **One bridge candidate, long-horizon:** `atlas-core-internal-content`
  (sessions/messages text, 509 MB, 178K writes/month) is the shards' content
  store and is read by the atlas-search Worker — which already fronts Turso
  with the #9134 D1 hot cache. Extending that cache is cheaper than moving
  the source of truth, and moving it would split the aggregator's
  chunk↔message join across two stores. Recommendation: keep, revisit only
  if the content store's read profile grows past the hot cache's hit rate.
- `atlas-ask-ledger` (exports/queries/usage_counters, 20 rows): MOVE-CLEAN
  whenever convenient; negligible either way.

### G8 — Small product stores — MOVE-CLEAN (negligible win, do opportunistically)

`notes-submissions` (8 tables, <250 rows), `bedside-and-away` (9 tables,
<15 rows), `voice-line-ledger` (2 tables, 4 rows), `fleet-dispatch-jobs`
(`jobs`, 6 rows). All are Worker- or service-shaped, tiny, and join-free;
each fits D1 trivially. Exception: **`great-star-lord` STAYS** — it is a
third-party app's database (Charm/Wish tables) and a standing keep-by-name
directive; it also has zero measured usage.

### Not a D1 question — frozen staging/recovery copies

`fleet-supabase-recovery-20261117` (422 MB, 0 writes this period),
`fleet-supabase-runtime-stage-20261118`, `fleet-supabase-runtime-legacy-stage-20261118`,
`fleet-supabase-runtime-tx-stage-20261118`, `fleet-platform-reconcile-stage-20261118`,
`fleet-platform-batched-stage-20261118`, `fleet-platform-short-tx-stage-20261118`,
`fleet-mirror-recovery-stage-20261118` — point-in-time copies from the
2026-11-17/18 Supabase→Turso recovery. They should never be _migrated_; the
programme should export each to an SQLite file in object storage and delete
the Turso copies (frees ~480 MB storage and 8 database slots). Deletion needs
an explicit decision per database; this audit only recommends.

## The bridge pattern (for G5, G6, and the G3 alerts tail)

For any group marked MOVE-WITH-BRIDGE:

1. **Sync:** the existing writer dual-writes (Turso primary, D1 secondary,
   failures logged not fatal) for at least one full sync cycle; a reconcile
   job compares row counts + max(updated_at) per table and repairs deltas.
   Append-only tables (G5 history, G6 events) may skip dual-write and use
   watermark copy instead — strictly simpler and safe.
2. **Reads:** the Worker reads D1 first; on a miss, or when the D1 copy's
   watermark is older than the group's stated window, it falls back to Turso
   and backfills D1 from the result (the #9134 hot-cache shape).
3. **Consistency window, honestly:** readers may see data one sync cycle stale
   (minutes for G5, one ingest cycle for G6). Any group whose reader acts on
   the data operationally (alerts delivery in G3) keeps Turso as primary
   until its dual-write has run clean for a full cycle, then flips.
4. **Cutover:** flip readers, keep dual-write one more cycle, then stop the
   Turso write. Rollback is a config flip at every step before the final one.

## Testing Drew's end-state sketch

"Turso as a sharded embedding store plus a few core components; most things
run in D1." Against the measurements:

- **By writes: confirmed.** 87% of rows written live in Worker-owned
  fleet-platform tables that move cleanly. After G1–G3, Turso's write meter
  drops from ~65.8M to ~8.4M rows/month.
- **By databases: confirmed.** 33 of 50 stay for Atlas alone; D1 absorbs the
  PM/dashboard plane (3 groups, ~15 small stores counting G8).
- **By reads: refuted, and it matters.** The estate's dominant read load
  (fleet-queue, 6.08B/month) stays on Turso _and_ is a scanning problem that
  no migration fixes. "Most things run in D1" is true of the dashboard plane,
  not of the estate's traffic.
- **Plan math, honestly:** moving G1–G3 alone does **not** permit a downgrade
  from Scaler — remaining reads (~6.9B) exceed Developer's 2.5B included, and
  remaining storage (~9.5 GB, atlas-dominated) sits at Developer's 9 GB line.
  The downgrade becomes real only if the G4 read-shape fix lands too
  (queue reads → index probes) **and** Atlas storage is trimmed or the plan
  stays Scaler for storage headroom. Near-term value of the programme is
  therefore: write headroom (the meter that was growing fastest), isolation
  of the noisy mirror workload from the broker's database group, and D1's
  included 50M writes absorbing churn the Turso meter currently counts —
  not an immediate $23/month.
- **D1-side walls, honestly:** one D1 database per moved family, never one
  giant D1 database (10 GB cap, single-writer serialization); the
  fleet-platform sync writers must batch under D1's sequential processing;
  and anything whose consumer is not a Worker (G4, G6 claims) cannot follow
  the data to D1 without an HTTP-API rewrite — which is why the end state
  keeps a real Turso core rather than a vestige.

## Recommended programme order

1. **Slice 1 — G1** (agent_mirror + quota + agg → one D1 database, dual-write
   cutover). Biggest write win, derived/rebuildable data, Worker-only access.
2. **Slice 2 — G2 + G3** (ingest families + operational ledgers → per-family
   D1). Completes the fleet-platform evacuation; retire `fleet-platform`
   and, after parity, `fleet-mirror`.
3. **Slice 3 — G4 read-shape fix** (index/cursor the queue pollers). Not a
   migration; the largest read win in the estate and the downgrade enabler.
4. **Slice 4 — G5 bridge** (github_mirror read side + pg_history archive).
5. **Slice 5 — G8 small stores + G6 events bridge**, opportunistically.
6. **Housekeeping (decision needed):** export + delete the 8 frozen
   staging/recovery copies (§G8 note).

## Caveats

- Usage figures are the Turso API's current-billing-period totals as read on
  2026-11-23; the organization endpoint's aggregate (69.7M rows written)
  differs slightly from the per-database sum (65.8M) — the per-database sum
  is used throughout, and the difference does not change any ranking.
- Per-table write attribution does not exist in the Turso API; G1/G2 shares
  of fleet-platform's writes are inference from writer behaviour (sync-cycle
  re-upserts), labelled as such wherever used.
- Atlas shard table shapes are from one representative shard plus the
  per-shard usage symmetry; the content shard was enumerated in full.
- This audit made no changes to any database. Tokens were fetched by name
  from the secrets broker at use time and are not recorded here.
