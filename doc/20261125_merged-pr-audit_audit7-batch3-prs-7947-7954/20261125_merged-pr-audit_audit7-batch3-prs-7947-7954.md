# AUDIT-7 merged-PR audit — batch 3 continuation (#7947–#7954)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. The five payload entries at this boundary are #7947, #7951, #7952, #7953 and #7954.

## Method and evidence

- Verified all five payload entries resolve to merged PRs. Read full PR bodies, changed-file responses, PR comments, inline review comments and reviews; counts are #7947 (1 file/3 comments), #7951 (1/4), #7952 (1/5), #7953 (2/4), and #7954 (2/4). PR #7951 has eight CodeRabbit inline suggestions; the source task disposition classifies them as minor definition-expansion points deferred to later implementation, with no blocker to its design-record acceptance.
- Read source issue bodies and full comment sets: #1388 (15 comments), #5788 (31), #5225 (23), #7587 (67), and #6665 (39). Read the salvage record for #7528 and the later PRs #9007, #12206 and #13233.
- #7947 re-applies the unique archived remainder of PR #7442 and closes the hard-break corruption bug tracked by #1388.
- #7951 delivers the feature-leveled build-variants decision requested by #5788; the source design issue closes with the record.
- #7952 re-applies a probe exposing the Petnames::open replay-validation bug. The real bug is tracked by #5225 and was fixed later by PR #9007; the issue closed after that fix.
- #7953 delivers only slice 1 of #7587. Later PRs #12206 and #13233 implement the local rendering/anchoring and reference-edge slices, but #13233 explicitly says the cross-server slice remains. The issue was later closed while that acceptance item was still open; Lane 2's existing pointer is #13871 comment 7120005251, so Lane 4 did not reopen it.
- #7954 fixes the filtered shard-query path for #6665 by routing filters through the monolith; the source issue closes with the implementation.
- Applied and read back `audit:complete` on #7947, #7951 and #7954, `audit:completed-later` on #7952, and `audit:partial` on #7953. Posted the two follow-up relationships on the corresponding PRs.

## Per-entry ledger

| Payload number |           Spawning issue | Verdict                 | Finding                                                                                                                                                                                                                                                             |
| -------------: | -----------------------: | ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
|          #7947 | #1388 (salvage of #7442) | `audit:complete`        | Restores hard-break normalization for headings and the requested parser/serializer regression cases; #1388 closes with the fix.                                                                                                                                     |
|          #7951 |                    #5788 | `audit:complete`        | Settles the four build-variant levels, composition boundaries, capability matrix, budgets and conformance approach; the design issue closes with the record.                                                                                                        |
|          #7952 | #5225 (salvage of #7528) | `audit:completed-later` | Adds the replay probe for invalid Petnames records. Later PR #9007 validates journal replay and adds rejection tests; #5225 then closes. PR note: #7160549199.                                                                                                      |
|          #7953 |                    #7587 | `audit:partial`         | Implements reference-backed board posts and the embeds edge (slice 1). Later #12206 and #13233 cover local views/anchors and graph/backlinks, but the cross-server slice remains. Existing Lane 2 closure pointer: #13871 comment 7120005251. PR note: #7160550888. |
|          #7954 |                    #6665 | `audit:complete`        | Routes filtered sharded vector queries through the monolith while preserving the sensitivity ceiling and surfacing the fallback diagnostic; the issue closes by this PR.                                                                                            |

## Slice state

Five payload entries were processed: five actual merged PRs audited and labelled, with no invalid references. Three are complete, one partial, and one was completed by later merged work. Across all slices to this point, **166 valid PRs are audited from 170 processed payload identifiers**; #7667, #7668, #7693 and #7694 are issues rather than PRs. There are 80 payload identifiers remaining; next is #7955. Batch 3 remains partial.

Tracking progress comment: [#13879 comment 7160583774](https://github.com/developing-today/code/issues/13879#issuecomment-7160583774).

## Raw supplements

`raw/` contains all five PR bodies, complete changed-file responses, PR comments, inline review comments and review receipts; source issue bodies/comment pages; salvage PR #7528 and later PRs #9007, #12206 and #13233; issue searches; label readbacks; relevant Lane 2 pointer evidence; and the posted disposition comments.
