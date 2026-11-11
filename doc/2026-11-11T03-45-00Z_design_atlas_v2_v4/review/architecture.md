# Independent architecture and source review

**Verdict: APPROVE** (2026-11-11 UTC; final re-review).

## Resolved finding

**Selected-shard failure semantics:** The prior draft's partial-result language did not explicitly preserve v1's whole-query failure for a selected shard failure. Existing [`fanout_scan`](../../../pkgs/atlas-search/crates/atlas-core/src/shards/scan.rs#L107-L109) requires query failure and propagates shard errors at [lines 143–159](../../../pkgs/atlas-search/crates/atlas-core/src/shards/scan.rs#L143-L159); the [cross-tier decision](../../2026-11-08T10-44-00Z_decision_atlas_cross_tier_vector_comparability/2026-11-08T10-44-00Z_decision_atlas_cross_tier_vector_comparability.md#L126-L132) adopts that posture. The revised main design §§6–7 now explicitly fails the whole selected fan-out on failure, timeout or budget cancellation, prohibits merging surviving shards, and restricts partial fallback to optional components. Any change to this contract requires a separate amendment. Finding resolved.

## What was reviewed

Read the main design top to bottom and inspected the baseline and provenance appendices and cited local source contracts, including the actual shard implementation. Re-reviewed the revised failure provisions, final section 9, all four scoped fixture drafts and the condensed follow-on audit ledger. Checked the single embedding model and pipeline rule, read-plane/write-plane separation, manifest versus authorization, explicit cold access, purge versus routine retention, independent rights for shared bytes, replay under current access, historical writer-set projection limitation, learned-model rollback following rights withdrawal, graph scale/rebuild and artifact deletion. The design identifies the principal uncertainties and avoids representing design records as proof of deployment. Its staged options and verification gates do not require premature implementation choices. No further material defect found in this review's scope.

## Limitations

No tests, builds, deployment inspection or independent external-source retrieval were performed. Not every external survey citation was independently re-fetched. This review's initial Git preflight could not run because its shell PATH did not expose `git` or `bash`; the parent reports its own preflight passed. No Git refs were modified by this review.
