# AUDIT-7 merged-PR audit — batch 3 continuation (#8049–#8054)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. The five payload entries at this boundary are #8049, #8050, #8051, #8053 and #8054.

## Method and evidence

- Verified all five payload entries resolve to merged PRs. Read full PR bodies, changed-file responses, PR comments, inline review comments and reviews; counts are #8049 (1 file/6 comments), #8050 (1/0), #8051 (5/4), #8053 (9/7, 26 inline comments and 19 review records), and #8054 (2/4).
- Read source issue bodies and full comment sets: #961 (118 comments across pages 1–2), #3984 (33), #5555 (71), and #6939 (7). Read later #8017 follow-on PR #11236 and the #8053 review dispositions.
- #8049 flips gopls to worktree scope but does not assert #961's three live workspace proofs. Issue #961 closed while those criteria were still outstanding; follow-up #15098 and live queue task #23811 now track the main/linked-worktree API proofs and distinct instances. Lane 2 closure pointer: #13871 comment 7162914420.
- #8050 is a workflow-generated flake input update with no spawning issue reference or matching source issue.
- #8051 delivers the shared Roadmap/Projects forecast contract for #3984. Unavailable quota-rate inputs are represented as explicit unknown states while #3983's measurement model remains open; #3984 closes with the contract.
- #8053 implements task/session read-status endpoints and token-vault documentation for #5555. All reviewed security and error-handling findings, including the durable session-to-repo authorization lookup, are fixed on the merged head; #5555 closes with the final PR.
- #8054 validates `--server` and unknown arguments before ACP traffic for #6939; the issue closes with the fix.
- Applied and read back `audit:partial` on #8049, `audit:no-issue` on #8050, and `audit:complete` on #8051, #8053 and #8054. Posted the partial closure pointer and no-issue disposition.

## Per-entry ledger

| Payload number | Spawning issue | Verdict          | Finding                                                                                                                                                                                                                                                                                                    |
| -------------: | -------------: | ---------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
|          #8049 |           #961 | `audit:partial`  | Changes the Gopls server to worktree scope, but the live prompt, linked-worktree rooting and distinct-instance proofs were not run. Issue #961 closed before those criteria were recorded. Follow-up #15098/task #23811 tracks the proof; Lane 2 pointer: #13871 comment 7162914420. PR note: #7162919013. |
|          #8050 |           none | `audit:no-issue` | Automated OpenChamber/OpenCode flake input update; no source issue reference or matching tracker was found. PR note: #7163080277.                                                                                                                                                                          |
|          #8051 |          #3984 | `audit:complete` | Defines one forecast contract for Roadmap and Projects with explicit uncertainty states and shared rendering; it consumes #3983's model without inventing missing quota rates.                                                                                                                             |
|          #8053 |          #5555 | `audit:complete` | Adds authenticated task/session status reads and documents the token source of truth. Review findings were resolved in the final head, including authorization through the durable cleared-session binding; #5555 closes with the merged fix.                                                              |
|          #8054 |          #6939 | `audit:complete` | Strictly validates `--server` and unknown arguments before starting ACP traffic; malformed flags fail with a usage error and exit 2.                                                                                                                                                                       |

## Slice state

Five payload entries were processed: five actual merged PRs audited and labelled, with no invalid references. Three are complete, one partial, and one has no separate spawning issue. Across all slices to this point, **226 valid PRs are audited from 230 processed payload identifiers**; #7667, #7668, #7693 and #7694 are issues rather than PRs. There are 20 payload identifiers remaining; next is #8055. Batch 3 remains partial.

Tracking progress comment: [#13879 comment 7163264171](https://github.com/developing-today/code/issues/13879#issuecomment-7163264171).

## Raw supplements

`raw/` contains all five PR bodies, complete changed-file responses, review comments and receipts; source issue bodies/comment pages; later PR #11236; follow-up issue #15098 and task #23811 records; merged-PR searches; label readbacks; the Lane 2 closure pointer; and posted disposition comments.
