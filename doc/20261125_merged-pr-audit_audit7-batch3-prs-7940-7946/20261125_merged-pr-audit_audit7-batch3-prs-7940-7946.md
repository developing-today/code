# AUDIT-7 merged-PR audit — batch 3 continuation (#7940–#7946)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. The five payload entries at this boundary are #7940, #7941, #7943, #7945 and #7946.

## Method and evidence

- Verified all five payload entries resolve to merged PRs. Read full PR bodies, changed-file responses, PR comments, inline review comments and reviews; counts are #7940 (1 file/3 comments), #7941 (3/4), #7943 (1/3), #7945 (4/4), and #7946 (1/3). No inline review threads were returned for this slice.
- Read source issue bodies and comments for #1252 (16), #2338 (42), #5833 (47), #7089 (31) and #5905 (28). For #7940, also read #7297's archive/salvage disposition trail around PR #7382, task 14094 and this PR (#7297 comments #6728547159, #6731754729 and #6731827138); the parent has a large unrelated archive-batch history.
- #7940 restores the sole unique document remainder from bulk-closed PR #7382. The restored document is the #1252 event-driven freshness design. Later PRs #3976–#3978 implement the design; #1252's final closure record confirms its requirements and gate were resolved or routed (comment 6935125520).
- #7941 implements device.info per #2338's typed manifest contract, including version negotiation, refusal cases and tests; the source issue closes with the implementation.
- #7943 records the edit/fork graph decision requested by #5833; the source design issue closes with the record.
- #7945 is Phase 1 of #7089 and explicitly leaves Phases 2–4 open. Later PRs and child issues complete re-arming, enablement and the closing audit; #7089 comment 6905371441 records the final complete disposition.
- #7946 records the provider security model requested by #5905; the issue closes with the decision record.
- Applied and read back `audit:completed-later` on #7940 and #7945, and `audit:complete` on #7941, #7943 and #7946. Posted both completed-later relationships on the corresponding PRs.

## Per-entry ledger

| Payload number | Spawning issue | Verdict | Finding |
|---:|---:|---|---|
| #7940 | #1252 (salvage task #7297) | `audit:completed-later` | Restores the event-driven freshness design note; the implementation followed in PRs #3976–#3978, and the remaining hierarchy clause was answered and routed before #1252 closed. PR note: #7160410393. |
| #7941 | #2338 | `audit:complete` | Implements the typed device.info manifest, adapter-side cached capability description, version refusals and unknown-capability refusal, with tests covering the declared v1 contract. |
| #7943 | #5833 | `audit:complete` | Settles lineage and divergence views, mark-most-correct semantics and merge proposals for the edit/fork graph; the design issue closes with the record. |
| #7945 | #7089 | `audit:completed-later` | Delivers the agreement checker and Phase 1 contract while leaving Phases 2–4 open. Later PRs/child issues complete the remaining re-arm and audit work; final closure is recorded in #7089 comment 6905371441. PR note: #7160411110. |
| #7946 | #5905 | `audit:complete` | Settles provider identity, signed admission, key-bound addressing, rogue-provider containment and bridge compromise boundaries; the design issue closes with this record. |

## Slice state

Five payload entries were processed: five actual merged PRs audited and labelled, with no invalid references. Three are complete and two were completed by later merged work. Across all slices to this point, **161 valid PRs are audited from 165 processed payload identifiers**; #7667, #7668, #7693 and #7694 are issues rather than PRs. There are 85 payload identifiers remaining; next is #7947. Batch 3 remains partial.

Tracking progress comment: [#13879 comment 7160429145](https://github.com/developing-today/code/issues/13879#issuecomment-7160429145).

## Raw supplements

`raw/` contains all five PR bodies, full changed-file responses, PR comments, inline review comments and review receipts; source issue bodies/comment pages; relevant pages of #7297's archive and salvage record; merged-PR searches; label readbacks; and the two posted disposition comments.
