# Independent final package / coverage review

## Verdict: agree with the bounded design scope; disagree that the issue package is ready to publish

The underlying design scope is coherent and preserves the two first-class tracks (PM Stack priority/urgency and Fleet work-item orchestration), the bounded existing substrate, and the explicit QTI discovery gap. The current artifacts correctly distinguish the 389-record source inventory from issue count, identify PRI-08/JRN-04 as design-expansion support rather than fabricated intake requirements, retain the #7030 protected expedite policy, and avoid claiming production/deployment proof. Source crosswalk IDs join exactly (389), source statements and disposition IDs reconcile (3,571), actual design heading fragments resolve, the command inventory and map reconcile (215), and local body/requirement joins do not produce errors in this validator.

The publication package is not green from this read-only pass. The live issue index has no published issue IDs for the 91 canonical entries. Offline graph review reports 255 unsequenced exact writable-file overlaps across 21 paths (notably `work/operations.rs` 63 pairs, `work/priority.rs` 30, `work/authority.rs` and `work/contracts.rs` 26 each, `turso_queue.rs` 25, `work/selection.rs` 22). Example: AUTH-01/AUTH-02 both claim `pkgs/fleet/crates/fleet-queue/src/work/authority.rs`, with no hard sequencing path or explicit reviewed read-only-consumer declaration. These are ownership collisions, not merely shared path mentions. The publisher's own read-only `--check` currently exits 2 on `ADAPTER-01: missing fields ['manual_support', 'external_requirement_refs']`. No live issue IDs, native parent IDs/blocked-by edges, body readbacks, or queue coverage were independently verified. Therefore do not call the package published or graph-complete based on `complete_edges_pending` or local intended mappings. Keep #9196 open and do not infer QTI equivalence to #3424.

## Checks performed

- Read the current semantic inventory receipt (`reviews/semantic-inventory.md`, especially lines 47–73): the source-semantic review is explicitly complete for sources 01–05; it is not a design/issue coverage verdict.
- Confirmed the package inventory reports 389 requirement rows, 3,571 dispositions, 215 command inventory keys, and 215 slice-map rows; issue manifest currently has 115 source-key rows.
- Confirmed the README states the batch remains draft pending parent approval and that native edges are deferred until all live IDs exist (`issues/README.md:1–4, 25–35`).
- Added `coverage/check_package.py`, a stdlib-only checker with mutually-exclusive modes and explicit-only report writes. `--self-test` passes; offline mode validates the trace, body and graph joins and surfaces the path collisions above. The structured result is `coverage/offline-report.json` (255 actionable collision rows; 91 canonical issues still pending publication). `--live` fails closed because parent approval/notification has not arrived; no live adapter was called.

## Blocking publication gates

1. **Live materialization/readback:** after parent-approved publication, verify every actual created issue number/URL, exact body marker and requirement IDs, body readback hash, parent DB ID, and native blocked-by edges in correct direction. Paginate fully and assert `hasNextPage` exhausted. Local intents and a completed index flag are insufficient.
2. **Queue coverage:** use the existing sanctioned read-only queue-coverage helper after every actual issue create; each open issue needs a live task in its own pilot queue (`queued`, `claimed`, `parked`, or `blocked`), never `done` or shadow-queue-only. Do not create or repair queue items in this review.
3. **Issue/body and graph validation:** resolve the 255 ownership pairs by minimal path-level sequencing or mark the genuine read-only consumer with reviewer-backed evidence; don't paper over shared ownership. Fix the publisher schema mismatch (`ADAPTER-01` missing two required fields), then rerun `publish_one.py --check`. Offline trace checks currently pass, but this graph finding blocks the complete offline verdict.
4. **Report honesty:** until those live checks complete, record the result as partial/unverified, not green. Existing #9241 remains research-only and its current bounded task does not prove the original Queue Table Improvements source was found.

## Current implementation boundaries to preserve

Do not duplicate #6533 live-launch proof, #9028 producer-bypass work, #9073–#9076/#9079/#9081 substrate, or expand the current bounded outbox into a general producer. Reuse existing crates and the single CLI glue path. Keep future tests described as future tests until run; distinguish pinned V1 source evidence at `a038f084` from deployed proof. Preserve full command/option semantics and both complete design tracks; unsupported functionality must be represented, not silently omitted.
