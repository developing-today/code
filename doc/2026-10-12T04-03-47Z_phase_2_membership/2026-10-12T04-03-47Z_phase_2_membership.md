# Phase 2: membership, auth frontier, and delegation

- Plan: [phased plan](../../2026-10-11T11-08-42Z_design_remaining_work/plan-phases.md), section 7.
- Start commit: `8bea0731` (phase-1 head at the start of Phase 2; Phase 1 was still in its final fix, see its record).
- Base branch: `phase-2`. Track branches: `p2-auth` (P2.1, P2.2), `p2-friend` (P2.4), `p2-caps` (P2.6); `p2-voiding` (P2.3) and `p2-delegation` (P2.5) after the auth track; P2.7 and P2.8 at the end.
- Phase 1 fixes still in flight (p1-liveness) are merged into this branch when they land.

## Steps

| Step                              | Track      | Status | Commits  | Tests added                      | Notes                                                                  |
| --------------------------------- | ---------- | ------ | -------- | -------------------------------- | ---------------------------------------------------------------------- |
| P2.1 Auth-frontier mapping        | auth       | done   | 862401c9 | 13 unit, 2 property, 16 scenario | `membership::frontier`; S2 cases and failure modes as regressions      |
| P2.2 Admission gate               | auth       | done   | 236dcbc7 | 14 unit, 1 property              | `membership::{gate, layer}`; placeholders; regression for group absent |
| P2.3 Authority-based voiding      | voiding    | todo   |          |                                  | replaces the Phase 1 rule                                              |
| P2.4 Friendship cap and dissolve  | friend     | todo   |          |                                  |                                                                        |
| P2.5 Delegation modes             | delegation | todo   |          |                                  |                                                                        |
| P2.6 World capabilities           | caps       | todo   |          |                                  | use-count bounds dropped (decision 3)                                  |
| P2.7 Cross-world reconciliation   | later      | todo   |          |                                  |                                                                        |
| P2.8 Property and mutation checks | later      | todo   |          |                                  |                                                                        |

## Decisions

- Voiding is authority-based: a dependent is voided only when its author lost authority at the dependency's point (decision 1).
- Transfer on removal is derived from the signed removal (decision 2).
- Use-count bounds on world capabilities are dropped; enforcement stays local (decision 3). Content on removal is kept (decision 4).

## Open questions

- Group identity. A group's ID is the BLAKE3 hash of its create event (design §5.5). p2panda-auth needs one `Author` type for members and groups, so the membership code uses `AuthId`: a key or a group hash. Confirm before P2.3 depends on it.
- Nested groups. The frontier covers the target group only. A `GroupMember::Group` in an action names another group's state, and its frontier is not included.
- A refusal rebuilds the auth graph from the accepted operations (`process` consumes its state on error). That is O(accepted) per refusal. Measure before scaling.
- The gate computes `state_at(dependencies)` per operation, so it is O(state) each time. Not measured.
- The layer is not wired into the event store yet, and it has no expiry for held events. Both belong to the store integration.
- Refused events with no manager at their dependencies are walked through, not given a placeholder. Descendants depend on whatever sits below them.

## Changes to the plan

- None yet.

## Review notes and regression tests

- P2.1: the raw `after` stalls with `MissingDependencies` (`raw_after_as_dependencies_stalls_with_missing_dependencies`). The literal mapping panics (`literal_mapping_with_no_auth_dependency_does_not_apply`). A root dependency voids a valid add (`root_dependency_voids_a_valid_add`). An unknown dependency is an error (`unknown_dependency_is_an_error_not_a_panic`, and `unknown_after_is_an_error_not_a_root` in the index).
- P2.2: an event naming an undecided hash is held, not treated as a root (`an_event_naming_an_undecided_hash_is_held_until_it_arrives`). A group absent at its dependencies is refused without the crate's panic (`an_operation_for_a_group_absent_at_its_dependencies_is_refused_without_a_panic`). A refused event leaves a placeholder that its descendants apply (`a_refused_event_leaves_a_placeholder_so_descendants_apply`).
- Found while writing P2.2, in the test rather than the code: the frontier of an event is taken over its `after`, so a group operation named directly is its own candidate. The minimality test was corrected to query X2's and R's `after`.

## End

- End commit and full test result: pending.
