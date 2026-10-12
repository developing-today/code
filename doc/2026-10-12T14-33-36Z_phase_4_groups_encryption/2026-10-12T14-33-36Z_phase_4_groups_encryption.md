# Phase 4: groups, encryption, and history

- Plan: [phased plan](../../2026-10-11T11-08-42Z_design_remaining_work/plan-phases.md), section 9.
- Start commit: `c12448c9` (origin/main after Phase 3).
- Base branch `phase-4`. Tracks: `p4-adapter` (P4.1, P4.2, P4.3; first, it unblocks the rest), then `p4-scheme` (P4.4), `p4-history` (P4.5, P4.6, P4.12), `p4-heads` (P4.7), `p4-warn` (P4.8 to P4.11).
- Decisions in force: full history by default; recent:N per world (32, 128 presets); scheme v1 (serde_bytes) as the first released scheme; from_join fork kept in the fork; no periodic rotation; PCS recovery by remove and re-add; warnings at 96, 128, 1000; BeeKEM not adopted.

## Steps

| Step                                  | Track   | Status | Commits | Tests | Notes           |
| ------------------------------------- | ------- | ------ | ------- | ----- | --------------- |
| P4.1 DataScheme adapter               | adapter | todo   |         |       | blocks the rest |
| P4.2 Our DGM and the from_join fork   | adapter | todo   |         |       |                 |
| P4.3 from_join withholding rule       | adapter | todo   |         |       |                 |
| P4.4 Scheme v1                        | scheme  | todo   |         |       |                 |
| P4.5 recent:N and full                | history | todo   |         |       |                 |
| P4.6 Lazy full history                | history | todo   |         |       |                 |
| P4.7 Heads rule                       | heads   | todo   |         |       |                 |
| P4.8 Rotation policy and PCS recovery | warn    | todo   |         |       |                 |
| P4.9 Warnings                         | warn    | todo   |         |       |                 |
| P4.10 scheme none notice              | warn    | todo   |         |       |                 |
| P4.11 DataScheme cost measurement     | warn    | todo   |         |       |                 |
| P4.12 Welcome chunking                | history | todo   |         |       |                 |

## Decisions

- See the decisions above and the index.

## Open questions

- None yet.

## Changes to the plan

- None yet.

## Review notes and regression tests

- To be filled.

## End

- Pending.
