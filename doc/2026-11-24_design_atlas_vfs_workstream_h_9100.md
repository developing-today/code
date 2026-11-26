# Workstream H — Virtual filesystem over the content-addressed store + aggregated file index

Date: 2026-11-24
Status: full design for [#9100](https://github.com/developing-today/code/issues/9100) (Atlas program #8983, proto design §H).
Design only — no implementation in this record's PR.

This record decides the five points the packet requires: (1) the location-record
schema and its Metadata versioning under workstream C doctrine; (2) the
aggregated index placement; (3) the Basin/Parquet evaluation with a verdict;
(4) consistency — how index updates ride upload and location add/edit against
the #9115 backend; (5) GC interplay with #9084. It references, and does not
restate, #9084 (content plane), #3125 (provenance/occurrence registry, closest
in-domain ancestor), #9095 (Metadata doctrine), and #9115 (location index
backend).

## 1. Problem and context

Atlas stores bytes content-addressed and immutable, uploaded once and never
changed, but answers questions about _places_: "everything referencing
`geordi.png`" must return every path/location naming those bytes, across
hosts and stores. The same bytes in five places are one upload plus five
location records, each carrying its own metadata (path, host, date,
producing process). Locations are added and edited without touching bytes.

Two prerequisites that blocked this design are now settled:

- **#9115 is closed as completed (2026-11-23).** The location-index backend
  exists as merged design (`doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md`,
  §A-9115.1/A-9115.2) and as landed code: typed locators in
  `pkgs/id/crates/id-core/src/location_ref.rs` (PR #12855),
  batch fold plus incremental ingest in
  `pkgs/id/crates/id-store/src/location_index.rs` (PR #12925), authorization
  filtering in `pkgs/id/crates/id-store/src/location_policy.rs` (PR #12958). The one open tail is A9115-existence-receipt
  (#10256), which this design does not need.
- **#9095 Metadata doctrine is merged** (`doc/2026-11-21T17-56-24Z_design_atlas_metadata_schema_v1/note.md`,
  PRs #12259 and #13527): versioned envelope (`schema_version`), additive-only
  evolution, `extras` preservation, closed `sensitivity`, exact-byte identity.
  #9095 is in-review, not parked, and its contract is stable enough to build on.

What remains undecided — and is decided here — is the VFS layer itself: the
location-record presentation schema, where the aggregated serving index lives,
whether Basin or Parquet earns a place, how VFS updates ride the #9115
harvester, and what a location deletion means for GC. #9084's reference-vs-copy
evaluation is explicitly _not_ pre-settled here (§6).

Corpus scale grounding: ~108K chunks / ~1 GB Turso at 2026-10-31 (#3125), laptop
~/atlas ~12 GB of distinct versions, plan envelope $24.92/mo with storage and
database-count quotas binding before embedding spend (growth model,
`doc/2026-11-08T11-18-02Z_design_atlas_storage_growth_model/`). The serving
index must therefore stay inside the existing Turso footprint — no new database
without a measured reason.

## 2. Goals and non-goals

### Goals

- One upload, N location records: bytes addressed by digest; every location is
  a versioned Metadata record pointing at the digest, never a copy.
- Point and prefix lookups (by hash, filename, path prefix, host) answered from
  an aggregated index in milliseconds at current scale, without scanning the
  corpus.
- The bucket works standalone: everything needed to rebuild the VFS projection
  travels beside the content-addressed data.
- Location add/edit/delete never touches bytes; delete of one location never
  affects bytes or sibling locations.
- Every decision states what carries over from #3125 and where the seam with
  #9115 and #9084 lies.

### Non-goals

- No re-decision of #9115's harvester/index internals or #9095's envelope and
  evolution rules; this design consumes both.
- No pre-settling of #9084's reference-vs-copy evaluation or its sensitivity
  tier table.
- No vector/FTS ranking — that is workstream A (#9092); the VFS answers
  "where", not "what is similar".
- No public surface: Atlas is internal-use only (#9091 constraint); the privacy
  rule from A-9115.2 (private by default, authorization filters before candidate
  counts) applies to every VFS query path.

## 3. Decision 1 — Location-record schema under #9095 doctrine

**Decided:** the canonical stored location is the A-9115.2 assertion, wrapped
as a #9095-v1 Metadata artifact. The assertion is the storage shape; the
Metadata envelope is the versioning and evolution shape.

```text
LocationRecord (stored) =
  { "schema_version": 1,                       # #9095 Atlas envelope
    "fields": {
      "content_id":   Digest,                  # what bytes (immutable)
      "occurrence_ref?": ID,                   # #3125 carry-over: chunk occurrence
      "source_version_ref?": ID,               # which version of which source
      "locator":      TypedLocator,            # R2{account,bucket,key} | Laptop{hostname,path} | Drive{account,file_id,url} | … (open registry, A-9115.2)
      "display_path": String,                  # VFS presentation: human path
      "filename":     String,                  # derived, indexed; not identity
      "host":         String,                  # originating host/store
      "namespace":    String,
      "tenant_scope": String,
      "observed_at":  Timestamp,
      "produced_by":  String,                  # originating process (Drew's model)
      "access_class": Enum(closed),            # sensitivity-tier input to #9084
      "signer":       Identity                 # sidecar authority (A-9115.2 gate)
    },
    "extras": {} }                             # #9095 §3.2 preservation
```

Rules:

- `content_id` + (`namespace`, `tenant_scope`, `locator`) is the record's
  identity; `filename`/`display_path` are derived projections, indexed but
  never identity. A rename changes the projection, not the record lineage.
- `access_class` is a closed enum under #9095 §3.1 with fail-closed fallback;
  unknown tokens are preserved in `extras` and resolve to most restrictive.
- Locator variants are an explicitly open registry (new stores add variants
  additively; readers preserve unknown variants in `extras`, never coerce a
  local path into a cloud URL).
- All evolution follows #9095 §5: additive-only; breaking candidates go to the
  #9095 parking lot and batch gate, never into a VFS-side fork of the doctrine.

**Rejected alternative:** a VFS-private record format with its own version
field. It would duplicate the envelope, the evolution rules, and the
sensitivity fallback for zero gain — every problem it solves is already solved
by #9095, and a second versioning scheme is a second source of truth.

## 4. Decision 2 — Aggregated index placement

**Decided:** dedicated VFS tables inside the existing Atlas serving Turso
database, plus a sidecar manifest/index object uploaded beside the bucket. No
separate VFS database.

```text
vfs_locations(content_id, namespace, tenant_scope, locator_type, locator,
              display_path, filename, host, observed_at, access_class,
              state CHECK IN ('fresh','stale','deleted','unverified'),
              tombstone_ref?, sidecar_ref)
  INDEX BY content_id          # "everything referencing these bytes"
  INDEX BY filename            # "everything named geordi.png"
  INDEX BY display_path prefix # prefix scans: LIKE 'prefix%' on indexed column
  INDEX BY host                # per-host inventory
```

- **Serving reads come from Turso.** At ~1 GB corpus scale this is one indexed
  table family; point lookups stay in the millisecond range and prefix scans
  are bounded by `LIMIT` + cursor. No serving query scans the corpus.
- **The sidecar manifest is the standalone and rebuild story.** Each bucket
  (or bucket prefix generation) carries a signed manifest of its location
  records in the same deterministic encoding as the tables. Bucket-works-
  standalone is satisfied by manifest + bytes, not by a second serving system.
  The tables are a rebuildable projection of validated sidecars + tombstones
  (A-9115.2 rule, inherited, not redefined).
- **No new database** because database-count quotas bind before any other
  resource (growth model §1) and a separate VFS DB buys no isolation the
  tenant_scope column does not already provide. Revisit only on a measured
  trigger (§9).

**Rejected alternatives:**

- _Its own Turso database:_ extra quota, backup, and connection surface for no
  query-shape benefit; cross-DB joins (locations × corpus metadata) get harder,
  not easier.
- _Sidecar-only with no serving tables (scan manifests per query):_ violates
  the aggregated-index rule — every lookup becomes a corpus scan.
- _Manifest scan cached in Workers KV/memory:_ an unversioned third copy with
  no authority story; tombstone reconciliation has no home.

## 5. Decision 3 — Basin/Parquet evaluation (required)

Evidence checked 2026-11-24: Basin GA 2026-10-01 (Pipelines / Catalog / SQL over
Iceberg on R2; Cloudflare blog + changelog); Basin SQL pricing $2.50/TB
compressed scanned with a 10 MB minimum per query; serverless, no clusters,
open Iceberg REST (DuckDB/Spark/Snowflake readable), zero egress; analytics
fit, not transactional; real-time processing and finer sovereignty controls
still planned, not shipped.

| Candidate                    | VFS serving fit                                                                                                                                                                   | Verdict                                                                                                                                                                             |
| ---------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Turso aggregated tables (§4) | ms indexed point/prefix lookups; transactional writes; already in-footprint                                                                                                       | **Serving path. Decided.**                                                                                                                                                          |
| Parquet snapshots on R2      | Cheap immutable interchange; DuckDB-queryable offline; no serving latency SLA                                                                                                     | **Cold interchange + offline export. Adopt as format, not as serving index.**                                                                                                       |
| Basin (Iceberg + SQL)        | Analytics engine: per-query 10 MB scan minimum, batch latency profile, no point-lookup SLA; strengths are corpus-wide scans, maintenance-free tables, open access from any engine | **Opt-in analytics tier for corpus-wide questions only** (orphan audit, growth rollups, lineage rollups that _would_ scan the corpus). Not the serving path. Revisit trigger in §9. |

Judged on the three required axes:

- **Lookup latency for VFS query shapes (hash, filename, prefix):** Turso
  B-tree indexes answer in ms; Basin SQL bills a 10 MB minimum scan per query
  and is built for distributed scans, not key lookups. Serving VFS point
  queries from Basin would be slower and pay-per-scan for no reason.
- **Update cost under constant ingestion (§E):** locations arrive one by one
  and in bulk; Turso incremental writes + the #9115 harvester's batch fold
  absorb both. A Pipeline-per-location ingest into Iceberg turns every location
  add into an analytics-ingest event — wrong cost profile for a serving write.
- **Operational weight:** Turso tables ride the existing backup/mesh story
  (§B); Parquet export is a scheduled job; Basin adds a warehouse, pipelines,
  and a second query engine to operate. Justified only for the offline
  questions that need it.

**Hybrid verdict:** Turso serves; Parquet snapshots (one per bucket generation,
beside the sidecar manifest) are the cold interchange any engine can read;
Basin earns its place when an offline question needs Iceberg (or when the §9
trigger fires), fed by the same Parquet snapshots through Pipelines — one
export path, two consumers. No VFS serving query may depend on Basin.

## 6. Decision 4 — Consistency: index updates ride upload and location add/edit

**Decided:** exactly one write path, layered on the #9115 backend. The VFS
never writes its serving tables directly.

```text
upload(bytes)            -> content_id; sidecar entry (complete scope stated)
location add/edit        -> signed sidecar entry (A-9115.2 assertion shape)
                           -> #9115 harvester: immediate for interactive adds,
                              batch fold for bulk ingest (both modes already landed in location_index.rs)
                           -> insert-all into per-scope location index (no opt-in omission)
                           -> VFS projection: upsert vfs_locations row (state=fresh)
location delete          -> signed stale/delete tombstone (never silent erase)
                           -> harvester fold marks state=deleted, keeps row + tombstone_ref
                           -> reference-release signal to #9084 collector (§7)
reconciliation cursor    -> replays sidecars + tombstones; tables rebuildable from scratch
```

Seams with #9115 (consume, do not redesign):

- The harvester, insert-all rule, tombstone protocol, reconciliation cursor,
  and `fresh | stale | deleted | unverified` states are #9115's; this design
  adds only the VFS presentation projection (`display_path`, `filename`,
  `host` indexes) and the four query shapes (§8).
- `ExistsAck`/B19 receipts (A-9115.1, tail in #10256) are out of scope for the
  VFS read path; existence checks reuse them when #10256 lands, and until then
  VFS reads assert nothing about liveness beyond index state.
- Stale/deleted rows are retained with their tombstone reference (audit +
  rebuild); hard purge is a retention-policy action owned with #9084/#3125
  lineage owners, never a VFS write-path side effect.

**Rejected alternative:** dual-write (VFS tables written alongside the
harvester by each producer). Two writers, two authority stories, guaranteed
divergence. The single projection path costs one fold and buys one truth.

## 7. Decision 5 — GC interplay with #9084

**Decided:** a location record is exactly one reference on
(`content_id`, scope). The VFS side of the contract:

- Location delete writes a tombstone and flips the VFS row to `deleted`; it
  emits a **reference-release signal** `{content_id, scope, tombstone_ref}` to
  #9084's collector. It never deletes bytes, never touches sibling locations,
  never decides liveness.
- Bytes die if and only if #9084's collector confirms zero live references
  across all scopes. What keeps bytes alive is the collector's verdict, not
  any VFS row.
- If #9084 lands reference counting, the release signal decrements; if it
  lands mark-and-sweep, the tombstone is a mark input; if it lands
  copy-into-DB, VFS rows address per-DB copies and the same tombstone protocol
  runs per copy. The VFS design works under any of the three #9084 models —
  this record states the interface (release out, liveness verdict in) and
  **does not pre-settle #9084's open evaluation** (per the #9091 binding
  constraint).

**Rejected alternative:** VFS-side reference counting with local delete of
last-copy bytes. It forks GC authority into the VFS, races with every other
reference holder (#9084's whole problem statement), and cannot see cross-scope
references. One collector; VFS is a signal source, not a collector.

## 8. VFS query set and latency expectations

| #   | Query                                       | Served by                              | Expectation (current scale)                                  |
| --- | ------------------------------------------- | -------------------------------------- | ------------------------------------------------------------ |
| 1   | By content hash: all locations naming bytes | `vfs_locations BY content_id`          | ms point lookup; never a corpus scan                         |
| 2   | By filename: all locations named X          | `vfs_locations BY filename`            | ms; filename is projection, collisions expected and returned |
| 3   | By path prefix: browse subtree              | `display_path LIKE 'prefix%'` + cursor | bounded pages; no unbounded subtree materialization          |
| 4   | By host: per-host inventory                 | `vfs_locations BY host`                | paged; the standby/restore path                              |

Authorization filters (tenant_scope, access_class) apply **before** ranking
and before candidate counts (A-9115.2 privacy rule, inherited). Public
enumeration stays prohibited unless explicitly configured. Corpus-wide
questions (orphans, growth, rollups) are Basin-tier batch queries (§5), never
serving queries, and carry no latency SLA here.

## 9. Worked example — one image, five locations

`geordi.png`, bytes digest `sha256:9f2c…`. One upload; five location records:

1. `Laptop{atlas-01, ~/photos/geordi.png}`, observed 2026-11-20, process `ingest-scan`.
2. `R2{atlas, corpus/blobs/9f/2c…}`, the canonical upload.
3. `Drive{atlas, file_id 1A2b…, url …}`, shared copy.
4. `Laptop{atlas-02, ~/incoming/geordi (1).png}`, different filename, same bytes.
5. Tar member `R2{atlas, extracts/2026-11.tar}` + span (sidecar index, range fetch — #3125 pattern).

- Query 1 (by hash) returns all five with per-location metadata and states
  (`fresh × 5`).
- Query 2 (by filename `geordi.png`) returns 1, 2, 3, 5 — not 4 (its filename
  differs); the record shows why filename is projection, not identity.
- **Rename** of 4 to `geordi-final.png`: new `display_path` projection on the
  same record lineage; queries 1 and 2 update accordingly; no bytes move.
- **Move** of 1 across directories: same as rename (locator + display_path
  update via signed sidecar entry).
- **Deletion** of 3 (Drive copy): signed delete tombstone; row 3 →
  `deleted`, kept with `tombstone_ref`; reference-release emitted; queries 1
  and 2 drop row 3; bytes and rows 1, 2, 4, 5 are untouched. Bytes survive
  until the #9084 collector sees zero live references.

## 10. Rollout and verification

Rollout: land VFS tables + sidecar writer first (reads still served by existing
paths); backfill projections from validated sidecars via batch fold; enable the
four query shapes behind the existing search surface flags; start Parquet
snapshot export; Basin tier only when an offline question or the §9 trigger
demands it. Rollback is projection deletion — sidecars and bytes are never
rewritten by this design, so disabling the VFS projection loses no data.

Verification:

- Golden vectors: five-location example above as fixtures (records, tombstone,
  rename) with expected query results per shape.
- Property: every `fresh` VFS row traces to a validated sidecar entry; every
  `deleted` row traces to a tombstone; rebuild-from-sidecars reproduces the
  tables exactly.
- Adversarial: unknown locator variant preserved in `extras`, never coerced;
  unauthorized scope returns zero rows _and zero counts_ (privacy rule probe).
- Conformance: #9095 fixtures (PR #13527 set) extended with the location
  profile; cross-check `location_index` fold vs VFS projection on the same
  sidecar stream.
- Scale gate: re-measure §8 expectations at 10× current corpus; fire the §9
  trigger on numbers, not vibes.

Open revisit trigger: serving tables exceed 10 GB or p95 point lookup exceeds
100 ms for a week, or an offline question runs weekly — then route the Basin
tier decision (dedicated warehouse vs. staying Parquet-only) with fresh
measurements.

## 11. Open questions (named deciders)

| Question                                                                 | Default in this design                                                  | Decider / revisit                                                            |
| ------------------------------------------------------------------------ | ----------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| Exact `display_path` normalization (case, separators, tar-member syntax) | byte-exact as observed; no normalization beyond documented prefix rules | Atlas VFS owner with #9095 registry review; before cross-host dedup of paths |
| Parquet snapshot cadence and partitioning                                | per bucket generation, partitioned by `tenant_scope`/date               | owning namespace operator; after first export measurements                   |
| Basin warehouse stand-up timing                                          | only on §10 trigger or a concrete offline question                      | Atlas program owner (#8983); not before trigger fires                        |
| Hard-purge policy for `deleted` rows                                     | retain with tombstone indefinitely; purge is retention-policy action    | joint with #9084/#3125 lineage owners; never unilateral VFS action           |
| New locator variants (new stores)                                        | additive registry; unknown variants preserved, not coerced              | ID/location owners (A-9115.2 registry); each addition reviewed               |

## 12. Implementation split (granular issues to file under #9100)

1. **H1 — Location-record profile:** freeze the §3 field registry (paths,
   types, required/optional, index policy, closed `access_class` values),
   `extras` policy, golden vectors. Consumes #9095, touches no other code.
2. **H2 — Aggregated tables + sidecar manifest:** `vfs_locations` schema +
   indexes, sidecar manifest writer/reader, rebuild-from-sidecars procedure.
   Depends on H1.
3. **H3 — Harvester seam + consistency:** wire H2 as a projection over
   `location_index.rs` fold (immediate + batch), tombstone flow,
   reconciliation cursor, reference-release signal shape for #9084. Depends on
   H1–H2 and consumes #9115 as backend.
4. **H4 — Offline tier:** Parquet snapshot export job + one Basin-backed audit
   query (orphan scan) proving the path; documents the §10 trigger
   measurements. Depends on H2; Basin warehouse itself only on trigger.

No implementation in this record. Slices are ordered; each is one lane/session
sized with its own acceptance in its issue.

## 13. Record and tracking

Source: #9100; parent program #8983; proto §H
(`doc/2026-11-12T23-59-00Z_design_atlas_v1_proto/note.md`).
Consumes: #9115 backend (design §A-9115.1/A-9115.2 + `location_ref.rs`,
`location_index.rs`, `location_policy.rs`), #9095 doctrine
(`doc/2026-11-21T17-56-24Z_design_atlas_metadata_schema_v1/note.md`).
References (not restated): #9084 content plane (reference-vs-copy open),
#3125 occurrence registry (`chunk_occurrences`, `cas_registry`, shard +
catalog resolution, tar sidecar + range fetch).
Queue task 20299 tracks this design slice.

### Evidence checked

- #9100, #9091, #9095, #9115, #9179, #9084, #3125 issue states + labels, read
  2026-11-24 (fresh; #9115 closed-completed 2026-11-23, #9095 in-review with
  merged PRs #12259/#13527).
- #9115 reconciliation note (Lane LU, 2026-11-23) and merged design section.
- Basin GA 2026-10-01 announcement, Basin SQL docs + pricing ($2.50/TB, 10 MB
  minimum), InfoQ 2026-11-10 — checked 2026-11-24.
- Growth model (`doc/2026-11-08T11-18-02Z_design_atlas_storage_growth_model/`),
  #3125 scale figures (2026-10-31: 107,639 chunks, Turso 1.0 GB).
- In-tree backend: `pkgs/id/crates/id-core/src/location_ref.rs`,
  `pkgs/id/crates/id-store/src/location_index.rs`,
  `pkgs/id/crates/id-store/src/location_policy.rs` (+ tests).
