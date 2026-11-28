# H4 — VFS offline tier: Parquet snapshot export + Basin audit path

Date: 2026-11-24 · Lane code-g (queue task 22924) · Status: proposed
Source issue: developing-today/code#14180 (child of #9100, workstream H slice H4)
Parent design: `doc/2026-11-24_design_atlas_vfs_workstream_h_9100.md` (PR #14176, merged 2026-11-24) — consumes §§4 (Decision 2, aggregated tables + sidecar manifest), 5 (Decision 3, Basin/Parquet hybrid verdict), 8 (query set), 10 (rollout + trigger), 11 (open questions), 12 (slice H4)

**H2 dependency (flagged, binding):** the H2 spec (#14178 — aggregated tables + sidecar manifest format) has NOT merged at the time of writing. This spec drafts against the parent design §§4–5 and consumes the H2 manifest format and bucket-generation definition without redefining them. If the H2 spec refines the manifest encoding, field registry, or generation boundary, this spec's §3–§4 must be reconciled to it before implementation; the reconciliation is editorial (field mapping), not a redesign.

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

**Input:** the generation's validated sidecar entries + tombstones (H2 manifest format), i.e. the exact stream that produces `vfs_locations` rows with states `fresh | stale | deleted | unverified`.

**Output:** one Parquet snapshot object per bucket generation, written beside the sidecar manifest:

```text
<bucket>/vfs-snapshots/<generation>/
  manifest.json          # the H2 sidecar manifest (authority for the generation)
  vfs_locations-<generation>.parquet
```

- **Partitioning:** by `tenant_scope`, then date (`tenant_scope=…/date=YYYY-MM-DD/` prefixes inside the snapshot object layout where the store supports it; single object otherwise — partitioning is a layout rule, not a second authority).
- **Schema:** the `vfs_locations` projection columns from parent §4 (`content_id, namespace, tenant_scope, locator_type, locator, display_path, filename, host, observed_at, access_class, state, tombstone_ref, sidecar_ref`) plus `generation` (the bucket generation id). Column order and encoding follow the H2 manifest's deterministic field order; the H2 spec owns any refinement.
- **Snapshot linkage:** the H2 manifest for the generation records the snapshot object name, row count, and content hash. A snapshot whose hash does not match its manifest entry is rejected by every consumer — same fail-closed posture as the front-door rule elsewhere in the program.
- **Idempotency:** re-export of a closed generation is byte-identical or rejected. Generations are append-only; a new location record belongs to a new generation, never to a rewrite.

**Failure handling:** export failure blocks nothing serving-side (serving reads never touch snapshots). A failed or partial export is retried from the validated sidecar stream; the manifest entry is written only on a complete, hash-verified snapshot.

## 4. Basin audit path + the orphan-scan query (G2)

**Path:** Parquet snapshots → Cloudflare Basin Pipeline → Iceberg table → Basin SQL. One export path, two consumers: DuckDB/Spark can read the snapshots directly today; Basin earns its place only for the corpus-wide questions. No Pipeline-per-location ingest — Pipelines consume per-generation snapshots (batch), never the location write path (parent §5 cost argument).

**The one proving query — orphan scan:**

```sql
-- Orphan candidates: content with NO fresh location in this generation.
-- Audit only: output is a report for reconciliation against #9084's
-- collector verdict. This query never deletes, never marks, never decides.
SELECT content_id, tenant_scope, MAX(observed_at) AS last_seen,
       COUNT(*) AS location_rows
FROM vfs_locations_snapshot
WHERE generation = '<generation>'
GROUP BY content_id, tenant_scope
HAVING SUM(CASE WHEN state = 'fresh' THEN 1 ELSE 0 END) = 0;
```

- **Semantics:** a row in the output is a _candidate_ — bytes whose every known location in this generation is `stale`, `deleted`, or `unverified`. Liveness is #9084's verdict, not this query's; candidates are reconciled against the collector, and disagreement (collector says live, scan says orphan, or vice versa) is itself an audited finding.
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

- **Golden vectors:** the parent §9 five-location `geordi.png` example is the fixture. Expected Parquet rows: five records with states `fresh × 5` (record 4 carrying its differing filename); after the record-3 delete tombstone, six rows with row 3 `deleted` + `tombstone_ref`. Expected orphan-scan output on that fixture: empty (four live locations) before the delete; after deleting all five (fixture extension), exactly one candidate row with `location_rows = 5`.
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
| H2 refines manifest encoding / generation boundary   | reconcile §§3–4 editorially to the H2 spec before implementation                                     | H2 spec owner (#14178); before export implementation    |
| Snapshot retention (how many generations kept on R2) | keep all until measured cost forces a policy; purge is a retention-policy action with lineage owners | joint with #9084/#3125 lineage owners; never unilateral |
| Second offline question before trigger               | allowed as one-off proving runs; standing schedule only post-trigger                                 | Atlas program owner (#8983)                             |

## 10. Record and tracking

Source: #14180; parent: #9100 / program #8983; parent design §§5 and 10 consumed; H2 manifest dependency (#14178) flagged in the header. Queue task 22924 tracks this spec slice. No implementation code in this record.
