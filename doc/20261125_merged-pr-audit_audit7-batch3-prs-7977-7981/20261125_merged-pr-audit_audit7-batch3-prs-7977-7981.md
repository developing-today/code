# AUDIT-7 merged-PR audit — batch 3 continuation (#7977–#7981)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. The five payload entries at this boundary are #7977, #7978, #7979, #7980 and #7981.

## Method and evidence

- Verified all five payload entries resolve to merged PRs. Read full PR bodies, changed-file responses, PR comments, inline review comments and reviews; counts are #7977 (3 files/3 comments), #7978 (1/1), #7979 (1/3), #7980 (2/3), and #7981 (2/11). No inline review threads were returned for this slice.
- Read source issue bodies and full comment sets: #5962 (38 comments), #5810 (19), #6512 (26), #6582, #6424 (28), #10575 (6), and #6125 (3). Read later fix PRs #8069 and #6559.
- #7977 delivers the server-side budget, observability and exploit-resistance decision record requested by #5962; the design issue closes with the record.
- #7978 salvages an overflow probe for #6512. The test pins current unsafe behavior; later PR #8069 adds checked arithmetic to production `dechunked_len` and closes #6512.
- #7979 salvages a non-player signer probe for #6424. Later PR #6559 adds the player-membership checks and closes #6424.
- #7980 records the framing and fragmentation decisions across stream, datagram and store-and-forward carriers for #5810; the design issue closes with the record.
- #7981 registers Mistral in quota attribution. The PR has no spawning issue reference, and the issue search found no matching Mistral-specific tracker. Later #10575 tracks Groq/OpenRouter registration and is a separate change.
- Applied and read back `audit:complete` on #7977 and #7980, `audit:completed-later` on #7978 and #7979, and `audit:no-issue` on #7981. Posted each disposition on the PRs.

## Per-entry ledger

| Payload number |           Spawning issue | Verdict                 | Finding                                                                                                                                                                                                      |
| -------------: | -----------------------: | ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
|          #7977 |                    #5962 | `audit:complete`        | Defines server/world budgets, accounting windows and enforcement classes, observable trip records, exploit-resistance boundaries and co-resident-world containment; the design issue closes with the record. |
|          #7978 | #6512 (salvage of #7524) | `audit:completed-later` | Adds the characterization probe for peer-declared chunk-size overflow. Later PR #8069 replaces unchecked arithmetic with checked size and total calculations and closes #6512. PR note: #7161058036.         |
|          #7979 | #6424 (salvage of #7520) | `audit:completed-later` | Adds the non-player signer probe. Later PR #6559 makes `sign_genesis` and `sign_claim` refuse non-player signers and closes #6424. PR note: #7161058498.                                                     |
|          #7980 |                    #5810 | `audit:complete`        | Chooses one canonical serialized frame at the adapter boundary, with carrier-specific stream framing or checked datagram/store-and-forward chunks; records invariants and conformance requirements.          |
|          #7981 |                     none | `audit:no-issue`        | Registers the measured Mistral pool and fixes its attribution test. No matching source issue was found; #10575 covers Groq/OpenRouter separately. PR note: #7161059021.                                      |

## Slice state

Five payload entries were processed: five actual merged PRs audited and labelled, with no invalid references. Two are complete, two were completed by later merged work, and one has no separate spawning issue. Across all slices to this point, **186 valid PRs are audited from 190 processed payload identifiers**; #7667, #7668, #7693 and #7694 are issues rather than PRs. There are 60 payload identifiers remaining; next is #7984. Batch 3 remains partial.

Tracking progress comment: [#13879 comment 7161084999](https://github.com/developing-today/code/issues/13879#issuecomment-7161084999).

## Raw supplements

`raw/` contains all five PR bodies, complete changed-file responses, PR comments, inline review comments and review receipts; source issue bodies/comment pages; later PRs #8069 and #6559; issue and merged-PR searches; label readbacks; and the posted disposition comments.
