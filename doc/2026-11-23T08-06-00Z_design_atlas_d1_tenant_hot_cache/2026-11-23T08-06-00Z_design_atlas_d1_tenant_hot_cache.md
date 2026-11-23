# Design: Atlas D1 tenant hot cache — admission, invalidation, Turso-hop skip

- **Issue:** #9134 (parent epic #9109, program #9106 §2 items 4–6, 17)
- **Status:** design record. No cache code, D1 binding, or schema is
  written or enabled by this document; implementation is filed as
  granular follow-on issues under #9134 (listed in §8).
- **Inputs consumed, not re-defined:** #9106's clarification of record
  (extensibility is the architecture goal; everything behavioral is
  configuration, §7 of the program); #9083 tenancy (tenant is the
  isolation and tiering unit); the v1 proto note
  (`doc/2026-11-12T23-59-00Z_design_atlas_v1_proto/note.md` §F, which
  names the hot-cache (Turso/D1) variant as a measurement input to
  #9098's embed-storage economics); #9098 (the both-ways prototype
  that consumes this design's counters, §7 below).
- **Current-state note (verified 2026-11-23 on main):** the
  atlas-search Worker (`pkgs/atlas-search/worker`) has **no D1
  binding today**. It fronts the fleet search service
  (`SERVICE_BASE`), which reads Turso, and keeps a per-query Durable
  Object cache (`QUERY_CACHE`). This design adds D1 as a tenant-scoped
  hot tier in front of the Turso read; it does not replace the DO
  query cache (which is keyed by query, not by tenant data) and does
  not change Turso's role as source of truth.
- **Ordering note:** per the 2026-11-17 triage record on #9134, the
  freshness bound is settled first (§1). Every other rule in this
  document — admission (§3), skip eligibility (§2), invalidation
  (§4) — is defined relative to it.

## 1. The freshness bound (settled first)

A cached entry may be served **only inside its freshness bound**.
Outside the bound the read revalidates against Turso; there is no
serve-stale mode in v1.

Every entry carries two facts from fill time:

- `source_watermark` — the tenant's corpus watermark at the Turso
  source when the entry was filled (the same watermark the query
  record already keeps verbatim, proto note §G).
- `filled_at` — wall-clock fill time.

Freshness is defined **per data class**, because the classes differ
in mutability, and one bound for all of them would be either unsafe
or useless:

| Class | Contents | Freshness rule |
| ----- | -------- | -------------- |
| A — content-addressed payloads | Chunk/document bodies keyed by content hash | Fresh indefinitely. The hash is the version: a body whose hash matches the request cannot be stale. No TTL, no watermark check. Eviction is capacity-only (§5). |
| B — query result sets | Ordered result id lists + scores for a (tenant, query, flags) key | Fresh iff `source_watermark` equals the tenant's current watermark **or** age ≤ TTL_B, whichever fails first. Default TTL_B = **60 s**. A watermark advance makes the entry stale by rule even inside the TTL. |
| C — tenant aggregates | Counts, facets, and other derived per-tenant summaries | Fresh iff age ≤ TTL_C **and** no invalidation event for the tenant post-dates the entry. Default TTL_C = **15 min**. |

Two cross-class rules:

- **Fail-safe degradation.** If the tenant's current watermark cannot
  be read (source unreachable), Class B degrades to TTL-only with
  TTL_B halved (30 s), never extended; Class C stops being served
  (aggregates are the class most likely to be quoted as exact); Class
  A is unaffected. Unavailability of the truth source narrows the
  bound; it never widens it.
- **No silent stale serves.** A serve that violates the rule above is
  a defect, not a mode. §7 counts watermark-mismatch serves
  separately; the count's expected value is zero and any non-zero
  reading is a bug report, not a tuning signal.

All bounds are configuration (program §7): TTL_B, TTL_C, and the
degraded-mode divisor are named config values with the defaults
above, changeable per deployment without code changes.

## 2. Turso-hop skip eligibility

A read **skips the Turso hop** iff all of the following hold:

1. An entry exists for the (tenant, key) pair.
2. The entry is fresh for its class under §1.
3. The tenant is not in forced-revalidation state (§4: set by an
   invalidation event or an operator purge until the next successful
   revalidation).

Skip is decided **per read path independently**: a result-set read
(Class B) and the hydration reads for its result bodies (Class A)
can each skip or revalidate on their own facts. A Class B skip with
Class A misses still saves the search hop and pays only body fills.

Every skip is logged with (tenant, key, class, entry watermark, entry
age). The log is the measurement surface §7 aggregates; an unlogged
skip is indistinguishable from a bug, so logging is part of the
eligibility path, not an afterthought.

## 3. Admission

Admission decides what earns a D1 entry. The default shape is
**evidence-based**: data is admitted after it has demonstrated heat,
plus one cost-based exception.

- **Heat admission (default).** A key becomes eligible after **2
  requests within a 10-minute window** (defaults; config). The first
  request is served from Turso and counted; the second triggers a
  fill. This keeps one-hit traffic — the bulk of a long-tail corpus —
  from evicting useful entries.
- **Heavy-query immediate admission.** A first request whose source
  cost exceeds a threshold is filled immediately: recompute is the
  expensive case the cache exists for. Default threshold (estimate,
  to be tuned from the first measurement round): source latency
  > 250 ms **or** rows-read > 10,000 for the single query.
- **Frequent-query admission** is the heat rule read over a longer
  window: keys with sustained request rates (the "recent heavy or
  frequent queries" of the issue body) stay admitted because their
  hit counters keep them inside the eviction horizon (§5); no
  separate mechanism is needed.
- **Tenant-tier weighting.** Cache footprint is a per-tenant quota of
  the D1 storage budget. Default weights by tenant tier
  (estimate): **1 : 4 : 16** across the #9083 tiers, with a **5%
  floor** of the total budget reserved per active tenant so a small
  tenant's hot set is never fully crowded out by a large tenant's.
- **Entry size cap.** Entries larger than **256 KB** serialized
  (default; config) are not admitted; oversized results stream from
  the source every time. The cap keeps one pathological result set
  from consuming a tenant's whole quota.
- **Negative results.** Empty result sets are admitted with a short
  TTL (default **30 s**, config): repeated misses against the same
  key are a real traffic shape (clients polling for not-yet-ingested
  content) and absorbing them is cheap, but a negative entry must
  expire quickly once ingestion lands.

## 4. Invalidation

Turso is the source of truth; the cache never argues with it.
Invalidation has three mechanisms, in the order they normally fire:

1. **Watermark advance (lazy, primary for Class B).** When a tenant's
   corpus watermark moves past an entry's `source_watermark`, the
   entry is stale by the §1 rule — no deletion is required for
   correctness. The next read revalidates and refills. This makes
   the common case (continuous ingestion) correct without any
   invalidation traffic.
2. **Tenant invalidation events (eager, for Class C).** The ingest /
   write path emits a per-tenant invalidation event (tenant id + new
   watermark) on completion of a write batch. The Worker consumes
   events to drop the tenant's Class C entries and set the tenant's
   forced-revalidation flag (§2 rule 3) until the next successful
   revalidation. **Default transport (v1): piggyback on the existing
   ingest completion records** — no new transport is built for v1;
   a dedicated stream is an open question (§9), not a prerequisite.
3. **Operator purge.** A per-tenant purge operation deletes all
   classes for one tenant and sets forced-revalidation. It exists
   for incident/support use (e.g. a mis-tiered ingest that must stop
   being served immediately), is audited (operator, tenant, reason),
   and is never invoked by automated paths.

Class A entries are never invalidated by any of the three: a
content-addressed body is either the bytes its hash names or it is
corrupt, and corruption is a storage-integrity matter, not a
freshness one.

## 5. Storage shape and eviction

One shared D1 database, tenant carried in the key — **not** one
database per tenant (default; §9 records the revisit condition).
Per-tenant databases would multiply deploy and migration surface by
tenant count; the isolation requirement is already met by
tenant-scoped keys and quotas.

```sql
CREATE TABLE hot_cache (
  tenant_id        TEXT    NOT NULL,
  cache_key        TEXT    NOT NULL,
  class            TEXT    NOT NULL,  -- 'A' | 'B' | 'C'
  payload          BLOB    NOT NULL,
  payload_bytes    INTEGER NOT NULL,
  source_watermark TEXT    NOT NULL,
  filled_at        INTEGER NOT NULL,  -- unix seconds
  last_hit_at      INTEGER NOT NULL,
  hit_count        INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (tenant_id, cache_key)
);
CREATE INDEX hot_cache_eviction ON hot_cache (tenant_id, last_hit_at);
```

Eviction is quota enforcement, run as a sweep (on-write sampling in
v1; a scheduled sweep is a follow-on if sampling proves uneven):

- Per tenant: while the tenant's total `payload_bytes` exceeds its
  §3 quota share, evict that tenant's least-recently-hit entries
  (Class B and C first — Class A bodies are re-derivable from the
  content store and are evicted last within the tenant).
- Global: unused quota is borrowable — a tenant may exceed its share
  while the database is under its total budget; the per-tenant rule
  binds only under pressure. This keeps the floor guarantee (§3)
  without stranding capacity.

## 6. Failure modes

- **D1 unavailable:** reads fall through to Turso (the pre-cache
  path). The cache is an optimization; its absence is a latency
  event, never an outage.
- **Watermark source unavailable:** §1 fail-safe degradation.
- **Fill races:** two concurrent fills for one key write the same
  (tenant, key) row; last writer wins, both payloads were filled
  from the same source of truth inside their own freshness reads,
  so a lost race costs a duplicate fill, never correctness.
- **Clock skew:** TTL checks use the Worker's clock for both
  `filled_at` and now; a single clock domain per entry keeps skew
  out of the comparison. Watermark comparison is the primary guard
  for Class B precisely because it is clock-free.

## 7. Measurement

The design's value is a measured number, not an assertion (the
proto note §F question — does the saving beat the cost — is settled
by measurement, and #9098's both-ways prototype consumes these
counters):

- `hits`, `skips`, `revalidations`, per class and per tenant.
- `stale_serves` (watermark-mismatch serves for Class B/C): expected
  zero; any non-zero value files a defect (§1).
- Estimated Turso cost avoided: skips × the entry's recorded fill
  cost (latency and rows-read captured at fill time), reported as an
  estimate with its method, never as an exact figure.
- Fill cost: D1 writes and storage bytes per tenant vs quota.

First tuning round after two weeks of counters (estimate cadence):
TTL_B, the heavy-query threshold, and tier weights are re-set from
measured hit/skip ratios, and the re-tuned values are recorded on
#9134's follow-on measurement issue (§8 item 6).

## 8. Implementation follow-ons (filed under #9134)

1. D1 schema + binding + cache store module in the atlas-search
   Worker (get/put/evict primitives over the §5 schema), behind a
   config flag, default off.
2. Freshness evaluator: the §1 class rules, watermark read, TTL
   logic, and fail-safe degradation. Oracle: property tests over
   generated (watermark, age, class, event) sequences asserting the
   invariant *no serve outside the bound*.
3. Admission policy: heat counters, heavy-query immediate admission,
   per-tenant quota + LRU eviction sweep (§3, §5).
4. Read-path integration: Class B result-set skip and Class A
   hydration skip in the Worker, with the §2 skip logging.
5. Invalidation: ingest-side tenant invalidation event emission on
   the existing completion records, Worker consumption (Class C
   drop + forced-revalidation), and the audited operator purge.
6. Measurement: the §7 counters surfaced as a report for #9098's
   economics prototype, plus the first tuning round.

## 9. Open questions (with chosen defaults — work proceeds on the defaults)

| # | Question | Chosen default | Revisit when |
| - | -------- | -------------- | ------------ |
| Q1 | Class B bound: TTL-only, watermark-only, or both? | **Both** (§1): watermark equality or 60 s TTL, whichever fails first | Measured revalidation cost shows the watermark read dominates the saving |
| Q2 | Dedicated invalidation transport vs piggyback? | **Piggyback** on ingest completion records for v1 (§4) | Event lag measurably delays Class C correctness past TTL_C |
| Q3 | One D1 database per tenant? | **No** — shared DB, tenant in the key (§5) | A tenant tier gains a hard isolation requirement #9083 does not already meet by key scoping |
| Q4 | Serve-stale on source outage? | **No** — degrade the bound, never serve outside it (§1, §6) | An availability incident review explicitly accepts bounded staleness as a mode |
| Q5 | Does the DO query cache merge into this tier? | **No** for v1 — different key space (query vs tenant data); revisit after both have counters | §7 counters show the two caches double-storing the same payloads at scale |

## 10. Acceptance mapping (issue #9134)

- [x] Design section merged under `doc/` — this document.
- [x] Admission by tenant tier / compute usage specified — §3.
- [x] Turso-hop skip specified, valid only inside a defined
      freshness bound — §1 + §2.
- [x] Invalidation / staleness vs the Turso source of truth
      specified — §4 (+ §1 fail-safe).
- [x] Open questions recorded with chosen defaults — §9.
- [x] Granular implementation issues filed for anything
      implementable — §8 items, filed under #9134 on merge of this
      design.
