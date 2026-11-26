# AUDIT-7 merged-PR audit — batch 3 continuation (#7745–#7753)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. The five payload entries at this boundary are #7745, #7747, #7749, #7752 and #7753.

## Method and evidence

- Verified all five payload entries resolve to merged PRs. Read each PR body, complete file response and PR comments. Changed-file and comment counts match metadata: #7745 (2/1), #7747 (1/3), #7749 (3/4), #7752 (2/2), #7753 (7/4).
- Read source issue bodies and full comment sets, confirming counts: #7743 (4), #6439 (23), #2329 (111 across two pages), #7751 (4), #5082 (154 across two pages).
- For #7747, issue comment #7072088764 records the final decision that the design is complete; live application round-trips remain a pre-implementation gate and the five product questions remain outside #6439.
- For #7753, the full #5082 comment history says its board JSON payload was brought within bounds, but #5082 remains partially satisfied. Its current residual cold `/program` stall is tracked on open issue #10383; checked that issue and its five comments. Cross-linked this outcome on PR #7753 comment 7157445700; no duplicate issue was filed.
- Applied and read back `audit:complete` on #7745, #7747, #7749 and #7752, and `audit:partial` on #7753.

## Per-entry ledger

| Payload number | Spawning issue | Verdict | Finding |
|---:|---:|---|---|
| #7745 | #7743 | `audit:complete` | Makes all six SaaS extraction caps surrogate-safe, retains exact-fit behavior, and adds boundary tests for each field. The issue completion comment confirms the fix on merged main. |
| #7747 | #6439 | `audit:complete` | Provides the two export-mode design, format comparison, mappings/loss manifest, read-authority handling, target profiles and round-trip corpus. Live GUI runs are explicitly a pre-implementation gate; the issue's later decision comment confirms that is the intended boundary. |
| #7749 | #2329 | `audit:complete` | Adds stable-ID summary/full projections and signed canonical-body round trips, covering the issue's projection and worked-example scope. Existing evaluator tests retain the investigation's ordering and DST behavior. |
| #7752 | #7751 | `audit:complete` | Rejects extra arguments before reading the config, while preserving zero/one-path behavior; subprocess tests cover the failing and control paths requested. |
| #7753 | #5082 | `audit:partial` | Bounds `/api/board.json` to the visible board and its linked items. The broader watched surface family remains partially satisfied; current cold `/program` failure is tracked by #10383, with the evidence cross-linked on the PR. |

## Slice state

Five payload entries were processed: five actual merged PRs audited and labelled, with no invalid references. Four PRs are complete; #7753 is partial with the remaining surface defect already tracked on #10383. Across all slices to this point, **66 valid PRs are audited from 70 processed payload identifiers**; #7667, #7668, #7693 and #7694 are issues rather than PRs. There are 180 payload identifiers remaining; next is #7757. Batch 3 remains partial.

Tracking progress comment: [#13879 comment 7157463299](https://github.com/developing-today/code/issues/13879#issuecomment-7157463299).

## Raw supplements

`raw/` contains the five PR bodies, complete changed-file responses, comments and label readbacks; the five source issue bodies/comments; full paginated comment history for #5082; merged PR search results; and the live #10383 issue/comment evidence.
