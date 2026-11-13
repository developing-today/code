# Decisions ledger system — pending-questions ledger / Outstanding Questions

Living document · Status: shipped (2026-11-04) · Register entry P2.1 · Tracking epic: developing-today/code#8805

## Purpose

- Hold, in one ledger, everything waiting on Drew, with evidence.
- Ensure nothing Drew is asked gets lost.
- First asked: 2026-11-04 23:47 CST.
- Drew requested the Outstanding Questions side chat as the
  mirror / working surface for the hourly pending-questions job.

## What landed (features, and how each works)

- A single pending-questions ledger is the central artifact.
- Ledger location:
  `~/workspace/goals/pending-decisions-hourly-questions-for-drew/hidden_files/pending-questions.json`
- Items accumulate in the ledger instead of living only in chat.
- The Outstanding Questions side chat mirrors the ledger as a
  working surface.
- The hourly pending-questions job is the associated job named
  in the register entry.
- Ledger state at the records-pass read on 2026-11-11:
  - 150 items total
  - 84 answered
  - 34 asked
  - 20 pending
  - 12 dropped
- The ledger is the substrate for downstream surfaces:
  - the decisions page (register entry P2.2)
  - the answerable pushes in developing-today/code#8568

## Rough architecture

- Store: the JSON ledger file at the path above.
- Owning goal: pending-decisions-hourly-questions-for-drew.
- Working surface: the Outstanding Questions side chat.
- Producer / upkeep: the hourly pending-questions job.
- Consumers recorded in the register: the decisions page (P2.2)
  and #8568's answerable pushes, both building on this ledger.
- Tracking: no single GitHub issue; the artifact is the ledger
  itself.

## Status

- design_status in the register: built-into-running-system.
- implementation_status in the register: shipped + running.
- Counts above are a snapshot from 2026-11-11, not a live count.

## Next steps — immediate / near-term / long-term

- Immediate: work through the pending items the ledger carries;
  the pending design rulings are listed in register entry P2.3.
- Near-term: not recorded as a separate step in this entry.
- Long-term: not recorded.

## Open follow-ups

- The pending design rulings the ledger carries are listed in
  register entry P2.3.
- No other follow-ups are recorded for this entry itself.

## Future ideas not yet specced

- None recorded in the register entry.
- Related built work (decisions page, P2.2) is documented under
  its own register entry, not here.

## Links (register entry, related issues/PRs named in the register entry)

- Register entry: P2.1, in pm-stack/design-register-2026-11-11.md.
- Ledger: `~/workspace/goals/pending-decisions-hourly-questions-for-drew/hidden_files/pending-questions.json`
- Owning goal: pending-decisions-hourly-questions-for-drew.
- Related: register entry P2.2 (decisions page).
- Related: developing-today/code#8568 (answerable pushes).
- Tracking epic: developing-today/code#8805.
- Note: this document is filed in the repo at
  pm-stack/decisions-ledger-system.md.
