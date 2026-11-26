# H4 — VFS offline tier: Parquet snapshot export + Basin audit path

Date: 2026-11-24 · Lane code-g (queue task 22924) · Status: proposed
Source issue: developing-today/code#14180 (child of #9100, workstream H slice H4)
Parent design: `doc/2026-11-24_design_atlas_vfs_workstream_h_9100.md` (PR #14176, merged 2026-11-24) — consumes §§4 (Decision 2, aggregated tables + sidecar manifest), 5 (Decision 3, Basin/Parquet hybrid verdict), 8 (query set), 10 (rollout + trigger), 11 (open questions), 12 (slice H4)

**H2 dependency (resolved 2026-11-25):** this spec was drafted before the H2 spec (#14178) merged. H1 (`doc/2026-11-25T01-14-10Z_design_atlas_vfs_h1_location_profile_14177/note.md`), H2 (`doc/2026-11-25T01-16-52Z_design_atlas_vfs_h2_tables_manifest_14178/note.md`) and H3 (`doc/2026-11-25T01-18-12Z_design_atlas_vfs_h3_harvester_seam_14179/note.md`) have since merged, and §§3, 4, 7 and 9 are reconciled to them (see §11). The reconciliation is editorial: layout, column list, linkage field and fixture numbers. The export, the audit path and the trigger are unchanged.

## 1. Problem + context

The serving path is settled: Turso `vfs_locations` tables answer the four §8 query shapes in milliseconds, and corpus-wide questions (orphans, growth, rollups) are explicitly NOT serving queries — they would scan the corpus. Today there is no offline path at all: no export exists, so the first orphan audit or growth rollup would either scan serving tables (unacceptable load on the serving DB) or re-walk the corpus (a full bucket scan per question). The H4 slice builds the cold interchange the parent design mandates — one export path, two consumers (any Parquet reader today, Basin Pipelines when the trigger fires) — and proves it with exactly one Basin-backed audit query. No serving query may depend on anything in this spec.

## 2. Goals and non-goals

Goals:

- G1. A Parquet snapshot export job: one snapshot per bucket generation, written beside the sidecar manifest, carrying the full `vfs_locations` projection for that generation.
- G2. One Basin-backed audit query (orphan scan) proving the offline path end to end, run against snapshots fed through Pipelines — without standing up a permanent warehouse ahead of the trigger.
- G3. Recorded trigger measurements/thresholds that would justify standing up the Basin warehouse (inheriting parent §10, adding export-side numbers).

Non-goals:

- Serving reads from Parquet or Basin (parent §5 verdict stands: Turso serves).
- The Basin warehouse itself — only on trigger, per #14180.
- H1/H2/H3 work: field registry freeze (H1, #14177), table schema + manifest writer (H2, #14178), harvester seam (H3, #14179). This spec consumes their outputs.
- #9084's liveness verdict: the orphan scan is an audit report, never a collection decision (parent §7 interface holds — release out, verdict in).

## 3. Export job spec (G1)

**Trigger:** once per bucket generation, after the generation's sidecars + tombstones are validated and the H2 rebuild stream for the generation is complete. The export reads the same validated input the H2 rebuild consumes — never the serving tables directly — so export output and serving projection cannot diverge at the source.

**Input:** the scope's state at the previous manifest (H2 §5 rebuild) plus the records this generation lists, i.e. the exact stream that produces `vfs_locations` rows with states `fresh | stale | deleted | unverified`. A bucket generation is one H2 manifest: the records of one scope folded after the previous manifest's epoch, up to this manifest's epoch (H2 §4.1).

**Output:** one Parquet snapshot object per bucket generation, written beside the sidecar manifest:

```text
<bucket>/vfs-snapshots/<namespace>/<tenant_scope>/<epoch-key>/
  manifest.json                      # the H2 manifest (authority for the generation)
  vfs_locations-<epoch-key>.parquet
```

- **Layout:** H2 §4.2. Namespace and tenant scope are percent-encoded path segments, and `<epoch-key>` is the H2 §4.1 fold-order key, so generations sort in fold order. The generation id is the triple (namespace, tenant scope, epoch key).
- **Content:** the whole `vfs_locations` projection of the scope at the manifest's epoch, deleted rows included. That is exactly the row set whose count and digest the manifest's `projection` records, so a snapshot is checkable against its manifest before anyone reads it.
- **Partitioning:** the path is already per scope, so a snapshot is one object, row-grouped by `observed_at` date where the writer supports it. Partitioning is a layout rule, not a second authority.
- **Schema:** the H2 §3 `vfs_locations` columns in table order (`namespace, tenant_scope, content_id, locator, locator_type, display_path, filename, host, produced_by, archive_member, observed_at, access_class, state, occurrence_ref, source_version_ref, signer, metadata_artifact_id, sidecar_ref, asserted_generation, tombstone_ref`) plus `generation` (the epoch key). Rows are sorted by the H2 primary key.
- **Snapshot linkage:** the manifest's `offline` field (H2 §4.3) holds `{"object", "rows", "digest"}`: the object name, its row count, and `sha256:` of the Parquet object's exact bytes. When export is on for a scope, the writer exports first and writes the manifest only after the snapshot verifies, so manifests stay immutable. A snapshot whose bytes do not match `offline.digest`, or whose row count differs from `projection.rows`, is rejected by every consumer.
- **Idempotency:** re-export of a closed generation is byte-identical or rejected. The writer pins its Parquet library version, compression, row-group size and metadata (no created-by timestamp) so the same rows give the same bytes. Generations are append-only; a new location record belongs to a new generation, never to a rewrite.

**Failure handling:** export failure blocks nothing serving-side (serving reads never touch snapshots). A failed or partial export is retried from the validated sidecar stream; the manifest entry is written only on a complete, hash-verified snapshot.

## 4. Basin audit path + the orphan-scan query (G2)

**Path:** Parquet snapshots → Cloudflare Basin Pipeline → Iceberg table → Basin SQL. One export path, two consumers: DuckDB/Spark can read the snapshots directly today; Basin earns its place only for the corpus-wide questions. No Pipeline-per-location ingest — Pipelines consume per-generation snapshots (batch), never the location write path (parent §5 cost argument).

**The one proving query — orphan scan:**

```sql
-- Orphan candidates: content with NO fresh location in this generation.
-- Audit only: output is a report for reconciliation against #9084's
-- collector verdict. This query never deletes, never marks, never decides.
SELECT content_id, tenant_scope, MAX(observed_at) AS last_seen,
       COUNT(*) AS location_rows,
       CASE WHEN SUM(CASE WHEN state = 'deleted' THEN 0 ELSE 1 END) = 0
            THEN 'all-deleted' ELSE 'no-fresh' END AS class
FROM vfs_locations_snapshot
WHERE namespace = '<namespace>' AND tenant_scope = '<tenant_scope>'
  AND generation = '<epoch-key>'
GROUP BY content_id, tenant_scope
HAVING SUM(CASE WHEN state = 'fresh' THEN 1 ELSE 0 END) = 0;
```

- **Semantics:** a row in the output is a _candidate_ — bytes whose every known location in this generation is `stale`, `deleted`, or `unverified`. Liveness is #9084's verdict, not this query's; candidates are reconciled against the collector, and disagreement (collector says live, scan says orphan, or vice versa) is itself an audited finding.
- **Classes (H3 §5.3):** stale and unverified locations still hold a reference, so only `all-deleted` candidates are expected to have zero references at the collector. `no-fresh` candidates are expected to be live there. A `no-fresh` candidate the collector calls dead, or an `all-deleted` one it calls live, is the finding.
- **Cost envelope:** Basin SQL bills $2.50/TB compressed scanned with a 10 MB minimum per query (parent §5 evidence). At current corpus scale (~1 GB, #3125 figures) each run bills the 10 MB minimum — the proving run is near-free and its measured scan bytes become the baseline for the growth trigger in §5.
- **Cadence:** on demand for the proving run; scheduled (weekly at most) only after the §5 trigger fires. No standing schedule before then.

## 5. Trigger measurements and thresholds (G3)

Inherited from parent §10 (unchanged, restated for locality): route the Basin-tier decision when serving tables exceed 10 GB, p95 point lookup exceeds 100 ms for a week, or an offline question runs weekly. Export-side additions recorded here:

- **Snapshot growth:** record per-generation snapshot bytes alongside generation id from the first export. The warehouse decision uses measured bytes-per-generation, not estimates.
- **First-scan baseline:** the proving orphan-scan run records scanned bytes, billed bytes, and wall-clock latency. These three numbers are the denominator for every future "should Basin stand up" argument.
- **Frequency rule:** a second distinct offline question (beyond orphan scan) running more than once, or the orphan scan itself running weekly, fires the trigger regardless of table size.
- **Decider:** Atlas program owner (#8983), per parent §11 — not before the trigger fires.

## 6. Alternatives considered

- **Dump serving tables instead of exporting from the sidecar stream.** Rejected: couples the offline path to serving-DB internals and load; export-from-stream keeps one authority (sidecars) with two projections (Turso serving, Parquet cold).
- **Basin-first (Pipelines ingest locations directly, snapshots derived from Iceberg).** Rejected: per-location analytics ingest for a serving write path (parent §5 cost argument); inverts the decided authority (Iceberg would become primary, Parquet derived). Parquet-first keeps one export path, two consumers.
- **Per-table full dumps on a timer instead of per-generation snapshots.** Rejected: timer dumps have no generation boundary, so a dump cannot be tied to a manifest hash or to the H2 rebuild stream — auditability of "which generation did this answer come from" is lost.

## 7. Verification

- **Golden vectors:** the H1 vectors and the H2 §6 records are the fixture. Expected Parquet rows at manifest M0 (epoch `0.0.0`): six rows, the five `geordi.png` locations plus `laptop-other`, with Drive `unverified` and the rest `fresh` (`laptop-incoming` carrying its differing filename). At M1 (epoch `2.1.2`): seven rows, with `laptop-incoming` and Drive `deleted` (each with its `tombstone_ref`) and `tar-member` `stale`. Each snapshot's row count and digest equal its manifest's `projection`. Expected orphan-scan output: empty at both M0 and M1 (the `geordi.png` bytes keep fresh locations, as does `laptop-other`). After delete tombstones on every remaining `geordi.png` location (fixture extension), exactly one candidate row: the `geordi.png` content, with `location_rows = 6` (its six rows, all `deleted`) and class `all-deleted`. `laptop-other` is other content and stays fresh. A variant that stales instead of deleting one of those locations yields the same row with class `no-fresh`.
- **Export–serving equivalence:** for any closed generation, the Parquet snapshot content equals the Turso projection for that generation row-for-row (modulo the added `generation` column). Property test on the fixture stream.
- **Orphan-scan precision:** the known-deleted fixture content is flagged; every known-`fresh` content is not. False positive or false negative fails the proof.
- **Manifest linkage:** tampered snapshot (hash mismatch vs manifest entry) is rejected by the reader harness.
- **No-serving-dependency:** CI asserts no serving query path imports the snapshot reader or Basin client (the serving/cold boundary is a dependency rule, not a convention).

## 8. Rollout

1. Land the export job writing snapshots beside manifests (reads: nothing consumes them yet).
2. Run the proving orphan scan once via Pipelines → Iceberg → SQL; record the §5 baseline numbers in the tracking issue.
3. Schedule nothing further until the §5 trigger fires; on trigger, the warehouse stand-up is a new issue under #9100, not an extension of this spec.

Rollback is snapshot deletion — sidecars, manifests, tables, and bytes are untouched by this slice.

## 9. Open questions

| Question                                             | Default in this spec                                                                                 | Decider                                                 |
| ---------------------------------------------------- | ---------------------------------------------------------------------------------------------------- | ------------------------------------------------------- |
| Snapshot retention (how many generations kept on R2) | keep all until measured cost forces a policy; purge is a retention-policy action with lineage owners | joint with #9084/#3125 lineage owners; never unilateral |
| Second offline question before trigger               | allowed as one-off proving runs; standing schedule only post-trigger                                 | Atlas program owner (#8983)                             |

## 10. Record and tracking

Source: #14180; parent: #9100 / program #8983; parent design §§5 and 10 consumed; H2 manifest dependency (#14178) resolved in §11. Queue task 22924 tracked the original spec slice. No implementation code in this record.

## 11. 2026-11-25 Update: reconciled to H1–H3

Editorial reconciliation, as the header required once H2 merged:

- **Layout and generation (§3):** `vfs-snapshots/<generation>/` became H2 §4.2's `vfs-snapshots/<namespace>/<tenant_scope>/<epoch-key>/`; a bucket generation is one H2 manifest (H2 §4.1).
- **Content and schema (§3):** a snapshot holds the whole scope projection at the manifest's epoch, so it is checkable against `projection`; the column list follows the H2 §3 table.
- **Linkage (§3):** the manifest's `offline` field holds the snapshot linkage; the writer exports before it writes the manifest.
- **Orphan scan (§4):** scoped to one namespace and tenant scope, with a `class` column that separates all-deleted candidates from stale or unverified ones, matching H3 §5.3's reference rule.
- **Fixture (§7):** numbers now follow the H1 vectors and H2 §6 (six and seven rows; Drive is `unverified`).
- **Open questions (§9):** the H2 reconciliation row is closed.
