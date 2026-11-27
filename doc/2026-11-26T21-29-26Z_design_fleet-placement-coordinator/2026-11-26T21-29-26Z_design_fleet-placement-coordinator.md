# Fleet placement coordinator — design record

**Recorded:** 2026-11-26T21:29:26Z  
**Tracking:** [developing-today/code#13765](https://github.com/developing-today/code/issues/13765)  
**Implementation:** [PR #14585](https://github.com/developing-today/code/pull/14585), merged 2026-11-24 (merge commit `20a7cf1a1aa4115bca49ed0252ca4fb9d0ae17fd`)  
**Follow-up proofs:** [#15594](https://github.com/developing-today/code/issues/15594)

This record preserves the design of record from #13765, comment
[7114228830](https://github.com/developing-today/code/issues/13765#issuecomment-7114228830), and its proof bar. It records both the intended architecture and its implementation disposition as known on the date above. It supersedes the design-status statements in `pkgs/fleet/book/concepts/compile-coordinator.md` that say the build has not landed; that maintained chapter is not rewritten by this immutable record.

## Decision and scope

Put fleet-wide placement admission in the queue broker, the existing fleet coordination point. Keep the current per-seat dispatch path as a singular fallback whenever the coordinator is unavailable or times out, and make that path visible as `placement_path: "singular-fallback"`. Scope is compile/dispatch placement only; the agent task queue is not changed.

The coordinator addresses four defects in per-seat placement:

1. **Atomic slots:** concurrent seats must not both consume the last owner slot.
2. **One true load view:** admission must use cross-owner GitHub Actions counts, not zero counts caused by a seat lacking token coverage.
3. **Global budgets:** Blacksmith minutes must be reserved and settled fleet-wide, not tracked as a per-seat honour system.
4. **Cross-seat visibility and queueing:** saturation must put work in a shared FIFO-per-owner waitlist rather than dispatching beyond capacity.

## State and admission contract

The broker store owns three records:

| Table                   | Role                                                                                                                                          |
| ----------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `placement_owner_state` | Per-owner cap, GitHub running/queued counts, load provenance/time, and active claim count.                                                    |
| `placement_claims`      | Claim identity, surface, owner, seat, estimated minutes, lifecycle state, run id, and expiry. `queued` rows form the FIFO-per-owner waitlist. |
| `placement_budgets`     | Per-month, per-surface budget, reserved minutes, and settled use.                                                                             |

The embedded surface registry is authoritative for surface owner, kind, caps and budget; clients cannot provide those values. A Blacksmith budget reservation is attempted first with a conditional update. The slot gate then uses one conditional update of the owner's active-claim counter, constrained by `gh_running + gh_queued + claims_active < cap`. The affected-row count decides the outcome: one row admits, zero rows queues. LibSQL's single-writer semantics serialize competing updates.

The broker exposes `/placement-claim`, `/placement-confirm`, `/placement-release`, `/placement-claim-status`, and `/placement-load`. Confirm associates an admitted claim with a dispatched run. Release frees a pre-confirm hold or settles Blacksmith actual minutes. Claim status and the true load view are readable by any seat.

## Coordinator sweep and client behavior

A background worker in the queue broker periodically reads GitHub Actions counts and live run ids for runner-host repos using broker-side credentials. It expires stale claimed/granted holds, settles terminal Blacksmith runs, recomputes active claims from persisted claims plus observed live runs, and grants queued work FIFO as capacity opens. Refresh failures are recorded; they do not overwrite the last known counts with zero.

For a real dispatch, the client asks the coordinator to claim after choosing a surface and before dispatching. A grant proceeds to dispatch and confirm; dispatch failure releases the claim. A saturated claim persists the full job as a queued session; session status later observes a grant and completes dispatch. A budget refusal causes one re-decision without Blacksmith. If coordinator calls fail, the existing singular path proceeds and output/session state names the fallback. Dry-run behavior and command surface remain unchanged.

## Proof bar

The design requires named evidence for each contract:

1. Two simultaneous claims for the last slot: exactly one is admitted.
2. Seeded GitHub-observed running/queued counts: a zero-coverage seat is queued when full and admitted when there is room; `/placement-load` returns those counts to any seat.
3. Fleet-wide Blacksmith budget: reservations beyond the configured limit refuse; settlement converts reservation to actual use.
4. Saturation: waiters drain FIFO as slots open, and claims resolve cross-seat.
5. Coordinator failure: dispatch still succeeds through singular fallback and reports that path.

## Implementation order and status

The dependency order is backend verbs, schema and proof tests; sweep and broker worker; client integration; then runbook, verification and PR. PR #14585 landed those implementation slices and records 654 passing `fleet-queue` library tests in its verification evidence. Independent review found the budget, FIFO, and fallback proofs present, but noted that the concurrency test was sequential in form and the load-view test did not seed nonzero GitHub counts. It also noted that production `BrokerCoordGh` reads still need live evidence. Those proof-strength follow-ups are tracked in #15594; they do not retroactively change this design.

## Sources

- #13765 issue body and design-of-record comment 7114228830.
- Independent PR verification comment 7143762737 on #13765.
- PR #14585 merged implementation and its recorded test evidence.
- `pkgs/fleet/crates/fleet-queue/src/turso_queue/placement_coord.rs` and `pkgs/fleet/crates/fleet-queue/src/placement/coordinator.rs` on main.
