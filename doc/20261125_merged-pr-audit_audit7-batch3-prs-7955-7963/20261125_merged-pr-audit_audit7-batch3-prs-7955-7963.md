# AUDIT-7 merged-PR audit — batch 3 continuation (#7955–#7963)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. The five payload entries at this boundary are #7955, #7958, #7961, #7962 and #7963.

## Method and evidence

- Verified all five payload entries resolve to merged PRs. Read full PR bodies, changed-file responses, PR comments, inline review comments and reviews; counts are #7955 (4 files/4 comments), #7958 (1/4), #7961 (1/6), #7962 (1/3), and #7963 (1/3). No inline review threads were returned for this slice.
- Read source issue bodies and full comment sets: #6181 (18), #5908 (29), #5801 (23), #5876 (30), and #4071 (71). Read archived PRs #7370 and #7550 and later fix PRs #9981 and #5181.
- #7955 re-applies the SSH/web logging portion of credential cleanup for #6181. Later PR #9981 removes remaining private-key and password-derived values from the shared auth helpers and closes the issue.
- #7958 records the decompose-versus-attach procedure, payload reference invariants, grey cases and promotion rule requested by #5908; the design issue closes with the record.
- #7961 records encrypted-share modes, key holders and delivery, rotation, revocation limits, recipient proofs and composition for #5801; the issue closes with the decision record.
- #7962 records residency declarations, assurance levels, provenance, lifetime and visibility for #5876; the issue closes on the decision record.
- #7963 re-applies a currency validation probe from archived PR #7550. The malformed-body gap is tracked by #4071; later PR #5181 adds the validation and fold behavior, then closes #4071.
- Applied and read back `audit:completed-later` on #7955 and #7963, and `audit:complete` on #7958, #7961 and #7962. Posted the later-completion links on the two salvage PRs.

## Per-entry ledger

| Payload number |           Spawning issue | Verdict                 | Finding                                                                                                                                                                                                                                        |
| -------------: | -----------------------: | ----------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
|          #7955 | #6181 (salvage of #7370) | `audit:completed-later` | Removes credential material from SSH and web auth logs. The shared auth-helper logs remained until later PR #9981, which completed the issue. PR note: #7160698928.                                                                            |
|          #7958 |                    #5908 | `audit:complete`        | Settles the ordered decompose-versus-attach procedure, ten grey cases, payload references through IDs, one-way lossless promotion, alternatives and open questions; the design issue closes with the record.                                   |
|          #7961 |                    #5801 | `audit:complete`        | Settles readable/sealed share modes, key ownership and delivery, event-driven rotation, forward-only revocation limits, recipient proofs and narrowing composition; the design issue closes with the record.                                   |
|          #7962 |                    #5876 | `audit:complete`        | Defines residency/citizenship declarations, assurance levels, provenance, expiry, visibility and rejected alternatives; the design issue closes with the record.                                                                               |
|          #7963 | #4071 (salvage of #7550) | `audit:completed-later` | Adds a probe for invalid currency authorisations folding live. Later PR #5181 validates malformed bounds, windows, fees, ledgers, rates, caps and allowlists and prevents invalid higher versions shadowing a valid one. PR note: #7160699527. |

## Slice state

Five payload entries were processed: five actual merged PRs audited and labelled, with no invalid references. Three are complete and two were completed by later merged work. Across all slices to this point, **171 valid PRs are audited from 175 processed payload identifiers**; #7667, #7668, #7693 and #7694 are issues rather than PRs. There are 75 payload identifiers remaining; next is #7964. Batch 3 remains partial.

Tracking progress comment: [#13879 comment 7160717320](https://github.com/developing-today/code/issues/13879#issuecomment-7160717320).

## Raw supplements

`raw/` contains all five PR bodies, changed-file responses, PR comments, inline review comments and review receipts; source issue bodies/comment pages; archived PRs #7370 and #7550; later fix PRs #9981 and #5181; merged-PR and issue searches; label readbacks; and the two posted disposition comments.
