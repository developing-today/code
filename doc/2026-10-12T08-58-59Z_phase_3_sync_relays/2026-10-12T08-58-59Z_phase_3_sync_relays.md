# Phase 3: sync, blobs, and relays

- Plan: [phased plan](../../2026-10-11T11-08-42Z_design_remaining_work/plan-phases.md), section 8.
- Start commit: `02569c9e` (origin/main, after PR #156, Phase 1). Phase 2 is still in review and is not a blocker for these steps; P3.4 waits for its membership interface.
- Base branch: `phase-3`. Track branches: `p3-range` (P3.1, P3.1a to c), `p3-logsync` (P3.2, P3.3), `p3-relays` (P3.5, P3.6, P3.8).
- Owner decisions used: K and T are local (N6); range above the 3.5 percent crossover, LogSync below (N5); n0 relay now (N19); relay ops per the S11 runbook (N25); QAD-capable relays first (N43); startup waits only for the home relay (N32).

## Steps

| Step                                      | Track   | Status | Commits | Tests added | Notes                             |
| ----------------------------------------- | ------- | ------ | ------- | ----------- | --------------------------------- |
| P3.1 Range sync on the redb store         | range   | todo   |         |             | K=16, T=16 defaults               |
| P3.1a Fork in a batch isolated            | range   | todo   |         |             | red first                         |
| P3.1b T=0 refused                         | range   | todo   |         |             |                                   |
| P3.1c Chunked flights under the frame cap | range   | todo   |         |             |                                   |
| P3.2 LogSync through p2panda-sync         | logsync | todo   |         |             | fork or vendor the sqlite default |
| P3.3 Method selection                     | logsync | todo   |         |             | after P3.1 and P3.2               |
| P3.5 Relay defaults and startup policy    | relays  | todo   |         |             |                                   |
| P3.6 Relay map order (QAD first)          | relays  | todo   |         |             |                                   |
| P3.8 Relay operations and unit file       | relays  | todo   |         |             |                                   |

## Decisions

- K and T are local settings and never on the wire. Changing them needs no version bump. A change to the wire format needs /id-sync/2.
- LogSync dependency: p2panda-sync pulls p2panda-store's sqlite default. Resolve in-repo (vendored copy with the one dependency line changed) rather than an external fork, so no account action is needed.

## Open questions

- None yet.

## Changes to the plan

- None yet.

## Review notes and regression tests

- To be filled.

## End

- End commit and full test result: pending.
