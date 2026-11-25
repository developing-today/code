# AUDIT-7 merged-PR audit — batch 3 continuation (#7857–#7864)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. The five payload entries at this boundary are #7857, #7860, #7861, #7863 and #7864.

## Method and evidence

- Verified all five payload entries resolve to merged PRs. Read each complete PR body, changed-file response and comments; counts match metadata: #7857 (1/2), #7860 (3/5), #7861 (1/5), #7863 (1/4), #7864 (1/6).
- Read all five source issue bodies and full comment sets, confirming #5861 (56), #5897 (16), #5797 (48), #5815 (18), and #5854 (24).
- The records meet their issue-level decision-record acceptance: board moderation, benchmark gates/results retention, fallback latency budgets and measurement, adapter/relay security binding, and per-target composition. #7860's later issue close-out confirms the record met #5897's close condition despite a sibling issue remaining under its own state.
- Applied and read back `audit:complete` on all five PRs.

## Per-entry ledger

| Payload number | Spawning issue | Verdict | Finding |
|---:|---:|---|---|
| #7857 | #5861 | `audit:complete` | Records board-scoped roles/actions, signed moderation hooks, private reporting, append-only audit and appeals; corrects the #5680 reference distinction between world-session capabilities and board authority. |
| #7860 | #5897 | `audit:complete` | Specifies regression budgets, paired-run noise handling, waiver/re-baselining process and durable JSONL result states/archive. The issue's later close-out confirms its design acceptance is met. |
| #7861 | #5797 | `audit:complete` | Defines per-tier latency budgets and a reproducible measurement plan while preserving the parent fallback order and keeping Workers out of bulk SLOs. |
| #7863 | #5815 | `audit:complete` | Binds session security to ID-layer signatures/transcripts/policy floor, defines replay/downgrade behavior and relay constraints. It preserves parent decisions and records sibling interface obligations. |
| #7864 | #5854 | `audit:complete` | Settles style/theme/override/CSS cascade, assets, conditional blocks and a three-target worked example, with sibling dependencies named and no runtime behavior changed. |

## Slice state

Five payload entries were processed: five actual merged PRs audited and labelled, with no invalid references. Across all slices to this point, **126 valid PRs are audited from 130 processed payload identifiers**; #7667, #7668, #7693 and #7694 are issues rather than PRs. There are 120 payload identifiers remaining; next is #7865. Batch 3 remains partial.

Tracking progress comment: [#13879 comment 7158858912](https://github.com/developing-today/code/issues/13879#issuecomment-7158858912).

## Raw supplements

`raw/` contains the five PR bodies, complete changed-file responses, comments and label readbacks, plus all five source issue bodies/comment pages.
