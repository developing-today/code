# AUDIT-7 merged-PR audit — batch 3 continuation (#7921–#7933)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. The five payload entries at this boundary are #7921, #7923, #7925, #7932 and #7933.

## Method and evidence

- Verified all five payload entries resolve to merged PRs. Read full PR bodies, changed-file responses, issue comments, inline review comments and reviews; counts are #7921 (1 file/6 comments), #7923 (1/3), #7925 (1/3), #7932 (4/6), and #7933 (1/3). No inline review threads were returned for this slice.
- Read source issue bodies and full comment sets: #5855 (35 comments), #5849 (23), #2894 (59), #5759 (full comments captured), and #6005 (38).
- #7921 and #7923 each deliver their follow-on design record and close their respective issues #5855 and #5849. Later implementation PRs implement those designs under their own feature issues; they are not remaining scope on the decision-record issues.
- #7925 repairs a test fixture exposed after #2894's restore validation was completed in PR #4518. The fixture now journals participant issuance like production. This fallout had no separate spawning issue and leaves no open scope gap.
- #7932 implements the client half of #5759. The daemon routes later landed in PR #11323, but #5759 remains open pending a live laptop round-trip. That remaining check is already tracked by task 20716 and gated on darwin-nix#2003 and a deployment window.
- #7933 records the data-only moderation-pack format requested by #6005; the source design issue is closed with the record.
- Applied and read back `audit:complete` on #7921, #7923 and #7933, `audit:no-issue` on #7925, and `audit:partial` on #7932. Posted the no-issue and partial dispositions on those PRs.

## Per-entry ledger

| Payload number |            Spawning issue | Verdict          | Finding                                                                                                                                                                                                                        |
| -------------: | ------------------------: | ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
|          #7921 |                     #5855 | `audit:complete` | Settles the rendering pipeline's compile/runtime split, target-specific conformance corpus, and author tooling/linting; the design issue closes by this record.                                                                |
|          #7923 |                     #5849 | `audit:complete` | Defines the v1 closed semantic-markup registry and authoring syntax, raw HTML boundary, and target-neutral semantics; the design issue closes by this record.                                                                  |
|          #7925 | none (fallout from #2894) | `audit:no-issue` | Fixes the scheduling_actor test fixture to journal its issued participant after #2894's production validation was completed by PR #4518. No separate source issue or remaining gap was found. PR note: #7160277156.            |
|          #7932 |                     #5759 | `audit:partial`  | Delivers the client half of fleet proxy get/put. The daemon routes later landed in #11323, but a live laptop round-trip remains open under task 20716, gated on darwin-nix#2003 and a deployment window. PR note: #7160277946. |
|          #7933 |                     #6005 | `audit:complete` | Records a data-only, signed moderation-pack format with a static rule/feed core and separate per-world parameters; the design issue closes with the record.                                                                    |

## Slice state

Five payload entries were processed: five actual merged PRs audited and labelled, with no invalid references. Three are complete, one partial, and one has no separate spawning issue. Across all slices to this point, **156 valid PRs are audited from 160 processed payload identifiers**; #7667, #7668, #7693 and #7694 are issues rather than PRs. There are 90 payload identifiers remaining; next is #7940. Batch 3 remains partial.

Tracking progress comment: [#13879 comment 7160296832](https://github.com/developing-today/code/issues/13879#issuecomment-7160296832).

## Raw supplements

`raw/` contains all five PR bodies, complete changed-file responses, issue comments, inline review comments and review receipts; the source issue bodies/comment pages; merged-PR searches; tracking/umbrella claim checks; label readbacks; and the posted disposition comments.
