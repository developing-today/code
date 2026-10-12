# Phase 2: membership, auth frontier, and delegation

- Plan: [phased plan](../../2026-10-11T11-08-42Z_design_remaining_work/plan-phases.md), section 7.
- Start commit: `8bea0731` (phase-1 head at the start of Phase 2; Phase 1 was still in its final fix, see its record).
- Base branch: `phase-2`. Track branches: `p2-auth` (P2.1, P2.2), `p2-friend` (P2.4), `p2-caps` (P2.6); `p2-voiding` (P2.3) and `p2-delegation` (P2.5) after the auth track; P2.7 and P2.8 at the end.
- Phase 1 fixes still in flight (p1-liveness) are merged into this branch when they land.

## Steps

| Step                              | Track      | Status | Commits | Tests added | Notes                                 |
| --------------------------------- | ---------- | ------ | ------- | ----------- | ------------------------------------- |
| P2.1 Auth-frontier mapping        | auth       | todo   |         |             |                                       |
| P2.2 Admission gate               | auth       | todo   |         |             |                                       |
| P2.3 Authority-based voiding      | voiding    | todo   |         |             | replaces the Phase 1 rule             |
| P2.4 Friendship cap and dissolve  | friend     | todo   |         |             |                                       |
| P2.5 Delegation modes             | delegation | todo   |         |             |                                       |
| P2.6 World capabilities           | caps       | todo   |         |             | use-count bounds dropped (decision 3) |
| P2.7 Cross-world reconciliation   | later      | todo   |         |             |                                       |
| P2.8 Property and mutation checks | later      | todo   |         |             |                                       |

## Decisions

- Voiding is authority-based: a dependent is voided only when its author lost authority at the dependency's point (decision 1).
- Transfer on removal is derived from the signed removal (decision 2).
- Use-count bounds on world capabilities are dropped; enforcement stays local (decision 3). Content on removal is kept (decision 4).

## Open questions

- None yet.

## Changes to the plan

- None yet.

## Review notes and regression tests

- To be filled.

## End

- End commit and full test result: pending.
