# AUDIT-7 merged-PR audit — batch 3 continuation (#7905–#7912)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. The five payload entries at this boundary are #7905, #7906, #7910, #7911 and #7912.

## Method and evidence

- Verified all five payload entries resolve to merged PRs. Read full PR bodies, changed-file responses, issue comments, inline review comments and reviews; counts are #7905 (16 files/3 comments), #7906 (1/2), #7910 (1/2), #7911 (1/1), and #7912 (1/2). #7906 has two inline review comments, including the P2 finding and accepted fix-forward disposition.
- Read source issue bodies and full comment sets: #5345 (58 comments), #5844 (31), #6300 (51), and #5852 (57).
- #7905 implements the transport and CLI/REPL slice of #5345, including a two-server integration test. The issue explicitly scopes out web UI as a separate follow-on (#5346), so that separate surface does not remain in this issue's scope.
- #7906 contains an accepted Codex P2 finding: authored annotations were incorrectly treated as discardable derived state. PR #7910 fixes that invariant; the two PRs share the #5844 source issue, which is complete after the fix-forward.
- #7911 provides #6300's v1 design and spike breakdown. The source issue stayed open until all five child steps #7913–#7917 were implemented; PR #13236 later landed the end-to-end spike and the issue was then verified complete.
- #7912 records the full native-rendering decision for #5852, including explicit parent assumptions, and the source issue closes on the record.
- Applied and read back `audit:complete` on #7905 and #7912, and `audit:completed-later` on #7906, #7910 and #7911. Posted the follow-up relationships on each completed-later PR.

## Per-entry ledger

| Payload number | Spawning issue | Verdict | Finding |
|---:|---:|---|---|
| #7905 | #5345 | `audit:complete` | Implements server-to-server file transfer over the existing blob and metadata paths, with source authorization, destination write policy, safe retry, conflict handling, archive-on-replacement and a two-server integration test. Web UI is explicitly a separate issue. |
| #7906 | #5844 | `audit:completed-later` | Delivers the attachments/media decision record, but its invariant incorrectly groups user-authored annotations with regenerable derivatives. The accepted P2 was corrected by later PR #7910. PR note: #7160181884. |
| #7910 | #5844 | `audit:completed-later` | Fixes #7906's annotation durability invariant: only computed derivatives may be discarded or regenerated; authored annotations remain durable. The shared design issue is complete after this fix-forward. PR note: #7160182277. |
| #7911 | #6300 | `audit:completed-later` | Delivers the v1 MCP bridge design and five-step implementation breakdown. Later PRs complete the spike; #13236 provides the end-to-end run and the issue's completion check. PR note: #7160182858. |
| #7912 | #5852 | `audit:complete` | Settles OS-standard controls, assertion mapping, platform conflict behavior, and explicit assumptions against unsettled parent issues. The design issue closes with the record. |

## Slice state

Five payload entries were processed: five actual merged PRs audited and labelled, with no invalid references. Two are complete and three were completed by later merged work. Across all slices to this point, **151 valid PRs are audited from 155 processed payload identifiers**; #7667, #7668, #7693 and #7694 are issues rather than PRs. There are 95 payload identifiers remaining; next is #7921. Batch 3 remains partial.

Tracking progress comment: [#13879 comment 7160201690](https://github.com/developing-today/code/issues/13879#issuecomment-7160201690).

## Raw supplements

`raw/` contains all five PR bodies, complete changed-file responses, issue comments, inline review comments and review receipts; four source issue bodies and full comment pages; merged-PR searches for the source issues; label readbacks; and the posted disposition comments.
