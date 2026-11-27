# AUDIT-7 merged-PR audit — batch 3 continuation (#7649–#7653)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and its comments, check later merged work for any remainder, label the outcome, and record the ledger on #13879. This slice covers PRs #7649, #7650, #7651, #7652 and #7653.

## Method and evidence

- Read the complete PR bodies and every changed-file response. File counts match each PR's `changed_files` value: #7649 (1), #7650 (2), #7651 (2), #7652 (2), #7653 (4). All changed-file pages fit within one `per_page=100` response.
- Read each PR conversation comment set; fetched counts match the PR metadata: #7649 (1), #7650 (5), #7651 (1), #7652 (4), #7653 (4).
- Read every source issue body and comment. The fetched counts match live metadata: #6060 (11), #2319 (28), #5789 (43), #5835 (74), #5813 (94).
- For the declared remainder on #2319, read the merged PR #12960 body and its complete changed-file response (7 files, 2 conversation comments), and read the #2321 completion record and its linked PR reference. The record verifies the later F1/F2/F3 composition, ledger submission, signed receipt and issue→transfer flow. The Lane 4 completed-later relationship is also linked in a comment on PR #7650.
- Applied and read back one outcome label per audited PR. The raw API responses, issue comment/event snapshots and final labels are kept in `raw/`.

## Per-PR ledger

|    PR | Merged     | Spawning issue | Verdict                 | Finding                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| ----: | ---------- | -------------: | ----------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| #7649 | 2026-11-08 |          #6060 | `audit:complete`        | The decision record chooses one embedding model across Atlas tiers, states the raw-score merge/rescore contract and model provenance requirements, preserves the pre-merge tier boundary, and records the cost consequences. Implementation follow-ons #7646–#7648 are separate, explicitly filed issues.                                                                                                                                                                              |
| #7650 | 2026-11-14 |          #2319 | `audit:completed-later` | The PR implements the signed, canonical-CBOR gateway envelope, strict verification, replay protection, device→account resolver and authorization seam, while explicitly leaving registry/factory/ledger integration on #2319. PR #12960 later completed that integration as the F1/F2/F3 gateway PoC, with ledger submission, signed receipt and the issue→transfer flow; #2319's later completion check records the envelope plus these follow-ons. The link is recorded on PR #7650. |
| #7651 | 2026-11-08 |          #5789 | `audit:complete`        | The design record covers the requested view-as-object, set algebra, federation, discovery v2, abuse shape, open questions and implementation boundaries. The issue's design criteria are complete; implementation slices are separate.                                                                                                                                                                                                                                                 |
| #7652 | 2026-11-10 |          #5835 | `audit:complete`        | The decision record settles approval queues, outcomes, rejected-draft ownership and resubmission, while keeping parent-owned policy provisional as requested. The issue closed on the merged record.                                                                                                                                                                                                                                                                                   |
| #7653 | 2026-11-10 |          #5813 | `audit:complete`        | The canonical design record covers relay roles, multi-relay leases, delivery and receipts, retention, backpressure, scaling, alternatives and invariants. The issue history confirms the overlapping PR #7972 was closed as a duplicate with no unique content to salvage.                                                                                                                                                                                                             |

## Later-work and filing check

No unfiled gap or bug remains in this slice. For #7650, the remainder originally named in its PR body was later implemented in merged PR #12960; issue #2321's completion record verifies the PoC acceptance. Issue #2320's separate report-only reconciliation scope was completed later by PRs #13139 and #13166; it is outside #7650's issue acceptance. PR #7653's duplicate #7972 offered one distinct custody alternative, which the canonical #7653 record rejected explicitly; no omitted acceptance item remains.

## Slice state

This slice adds five audited PRs to the prior 10: **15/250 audited; 235 remain; batch 3 remains partial**. The next PR in the fixed payload list is #7654. All five outcome labels were read back. The 250-PR batch ledger is not complete.

## Raw supplements

`raw/` contains the five audited PR bodies, full changed-file responses, PR comments and label readbacks; their source issue bodies and complete comment sets; the later #12960 PR body, complete file list and comments; and #2320/#2321 issue comments and event snapshots supporting the completed-later check.
