# REL-01 — Claim and lease contract compatibility against #9178

Date: 2026-11-25
Status: design of record for [#9512](https://github.com/developing-today/code/issues/9512)
(REL-01 under [#9209](https://github.com/developing-today/code/issues/9209), design
set `doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`).
Pure model and tests only. No schema change, no new verb, no runtime change.

## 1. Problem

[#9178](https://github.com/developing-today/code/issues/9178) asks the queue for four
claim operations: resume a task in a named session, claim any task by number
(including parked and attempt-capped ones) under an owner override, claim several
tasks in one session with an explicit take-over, and mark work that automatic lanes
never claim. Its semantics are accepted as FDR-0001 to FDR-0004
(`doc/decisions/`). The rest of the #9209 lease, outbox and completion domain
(#9487, #9514, #9515, #9517) builds on whatever the claim doors do. Before any of
that is implemented, the current doors and the accepted target have to be written
down side by side, so that each later slice knows which behaviour it is changing.

## 2. Goals and non-goals

Goals:

- Pin the current claim behaviour on `main` for every door and every stored row
  shape, from the code itself (`turso_queue::claim` and
  `turso_queue::lifecycle`).
- Pin the #9178 target for the same doors and shapes.
- Produce the transition compatibility table (§5) and a pure state model with
  named tests (§6) in `pkgs/fleet/crates/fleet-queue/src/work/lifecycle.rs`.

Non-goals:

- No `manual_only` or `resume_session_id` column, no `--take-over` flag, no new
  claim verb. Those are the #9178 implementation slices.
- No change to the budget gate, the platform limitation (#8217), lane binding or
  the bundle cap. They are claim filters that sit beside eligibility and keep
  their current owners.
- No decision on a by-id batch cap. `BATCH_CLAIM_MAX` (5) stays as it is (§7).

## 3. The doors

| Door                | Code today                                                     | #9178 record |
| ------------------- | -------------------------------------------------------------- | ------------ |
| Automatic pick      | `default_pick_predicate` + `TASK_REQUIREMENTS_CLAUSE` + filter | FDR-0004     |
| Claim by id         | `claim_by_id` + `by_id_claimable`                              | FDR-0002     |
| Transfer, take-over | `claim_transfer` + `transfer_refusal` (#8210)                  | FDR-0003     |
| Batch               | count batch, `BATCH_CLAIM_MAX` (automatic only)                | FDR-0003     |
| Resume by session   | none (by-id claim may bind a named session, #9046)             | FDR-0001     |

Three facts from the code fix how the doors compose:

1. The task's own requirements (payload `requires`) bind every door: the pick, the
   by-id claim and the transfer all call `task_requirements_unmet`. A claimer that
   does not declare a required attribute does not meet it.
2. The claimer's filter (`--filter`) is a pick-time predicate only. `claim_by_id`
   and `claim_transfer` take no filter. With no requirements and an empty filter,
   both sides are true and the pick is a plain priority pick (#8097).
3. A `claimed` row whose lease lapsed is taken by no door until the sweep
   (`expire_leases`) returns it to `queued`, to the handoff sink, or fails it at
   the cap. The pick predicate never names `claimed`, `by_id_claimable` refuses it,
   and `transfer_refusal` refuses a lease that is not live.

## 4. Row shapes

The model reads a stored row as: status (`QueueState`), the claim hold (none, live
by the requester, live by someone else, expired but not reclaimed), attempts
against `max_attempts`, whether its wait has ended, whether a `handed_off` row is
an unbound sink row (`HANDOFF_SQL`), `manual_only`, and a Do Now directive. "Wait
has ended" means a queued row's `not_before` has passed, or a parked row's release
is due under the pick predicate: a timer park whose time has come, or the capacity
bypass. Condition-only parks (`task_terminal`, `task_status`, `event_seen`,
`person_answer`) are never due through the pick (FDR-0008).

## 5. Transition compatibility table

Each cell is the answer for a claimer whose requirements are met and whose filter
matches. "Claim" means the row moves to `claimed` under the claimer. Rows in bold
change under #9178.

| Row shape                       | Door                      | Today                                | #9178 target                                   | Record   |
| ------------------------------- | ------------------------- | ------------------------------------ | ---------------------------------------------- | -------- |
| queued, under cap, due          | pick / by id              | claim                                | claim                                          | —        |
| queued, not yet due             | pick                      | refuse (not due)                     | refuse (not due)                               | —        |
| **queued, not yet due**         | **by id + override**      | refuse (not due)                     | **claim, override recorded**                   | FDR-0002 |
| queued, at `max_attempts`       | pick, plain by id         | refuse (attempt cap)                 | refuse (attempt cap)                           | —        |
| **queued, at `max_attempts`**   | **by id + override**      | refuse (attempt cap)                 | **claim, override recorded, history kept**     | FDR-0002 |
| parked, release due             | pick                      | claim (timer release)                | claim (timer release)                          | FDR-0008 |
| parked, release not due         | pick                      | refuse (not due)                     | refuse (not due)                               | —        |
| parked, any                     | plain by id               | refuse                               | refuse (override required)                     | FDR-0002 |
| **parked, any**                 | **by id + override**      | refuse                               | **claim, override recorded**                   | FDR-0002 |
| handed_off, unbound sink        | pick / by id              | claim at +0 attempts                 | claim at +0 attempts                           | #8234    |
| handed_off, bound               | pick / by id              | refuse                               | refuse                                         | —        |
| **manual-only, otherwise due**  | **pick**                  | **claim (no such field)**            | **refuse, whatever the priority or Do Now**    | FDR-0004 |
| manual-only, otherwise due      | by id                     | claim                                | claim (the only door)                          | FDR-0004 |
| claimed, live, someone else     | pick / by id              | refuse                               | refuse; batch reports `skipped-claimed`        | FDR-0003 |
| **claimed, live, requester**    | **by id**                 | refuse                               | **idempotent: the existing claim is returned** | FDR-0002 |
| claimed, live, someone else     | transfer, holder consents | claim (holder gave it away)          | claim                                          | #8210    |
| **claimed, live, someone else** | **take-over, no consent** | refuse (only the holder can give it) | **claim; prior worker and hand-back recorded** | FDR-0003 |
| claimed, lease expired          | every door                | refuse until the sweep reclaims it   | refuse until the sweep reclaims it             | —        |
| blocked                         | every door, override too  | refuse                               | refuse                                         | —        |
| done, cancelled, failed         | every door                | refuse                               | refuse                                         | —        |
| any, requirements unmet         | every door                | refuse                               | refuse                                         | #8097    |
| any, filter excludes            | pick                      | refuse                               | refuse                                         | #8097    |
| any, filter excludes            | by id, transfer           | filter not applied                   | filter not applied                             | #8097    |
| **any task, by id**             | **batch of ids**          | no by-id batch                       | **one independent outcome per task**           | FDR-0003 |

A Do Now directive changes no cell. It is urgency, not eligibility: a manual-only
Do Now item is still refused by the pick, and a Do Now item that is not manual-only
is an ordinary pick. The ratified `expedite=15` and `next-up=25` priorities stay
distinct from `do_now` and are not modelled here.

Resume by session (FDR-0001) adds no row to this table. A stored
`resume_session_id` is a binding hint on an eligible claim, not an eligibility
rule: it never overrides ownership, and the missing-session, wrong-worktree and
busy-session cases are launcher outcomes recorded on the attempt. Its tests belong
to the FDR-0001 implementation slice.

## 6. State model and tests

`src/work/lifecycle.rs` gains a pure model beside the #9501 crosswalk:

- `ClaimContract::{Current, Fdr9178}`: both contracts in one function, so every
  table row is a pair of assertions.
- `ClaimPath::{Automatic, ById { owner_override }, TakeOver { holder_consents }}`.
- `ClaimCandidate` (§4), `TaskRequirements`, `ClaimerAttrs`, and `ClaimFilter`,
  the equality-conjunction subset of the filter grammar.
- `claim_eligibility(&ClaimRequest, &ClaimCandidate) -> ClaimEligibility`:
  `Eligible(ClaimGrant { override_recorded, transfer })`, `AlreadyOwned`, or
  `Refused(ClaimRefusal)` with one named reason per refusal.
- `batch_claim_outcomes` with FDR-0003's spellings: `claimed`,
  `already-owned-by-this-session`, `skipped-claimed`, `not-found`, `failed`.

Tests (filter `pmo_rel_claim_` plus the manual-only test):

| Test                                                                  | Proves                                                                                        |
| --------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| `pmo_rel_claim_requires_requirements_and_filter`                      | requirements AND filter, both default true; requirements bind every door, filter is pick-only |
| `pmo_rel_manual_only_is_eligibility_not_directive`                    | manual-only excludes the pick under any Do Now; by id reaches it; Do Now changes no answer    |
| `pmo_rel_claim_expired_but_unreclaimed_is_taken_by_no_door`           | the lapsed-lease row waits for the sweep on every door and contract                           |
| `pmo_rel_claim_parked_capped_and_blocked_rows`                        | the override rows of §5; blocked refuses on every door                                        |
| `pmo_rel_claim_live_claims_skip_take_over_and_idempotence`            | skip by default, consent-free take-over, idempotent own claim                                 |
| `pmo_rel_claim_batch_outcomes_are_per_task_and_keep_the_existing_cap` | per-task spellings, outcome independent of the batch, `BATCH_CLAIM_MAX` reused                |
| `pmo_rel_claim_empty_filter_and_handoff_sink`                         | empty filter matches a bare task; sink rows claimable, bound handoffs not                     |

Done check, as #9512 states it: list the tests by exact name with
`cargo test -p fleet-queue -- --list`, then run the filters `pmo_rel_claim_` and
`pmo_rel_manual_only_is_eligibility_not_directive`. A filter that runs zero tests
is not acceptance.

## 7. Alternatives considered

- **Model only the #9178 target.** Rejected: the slice's job is compatibility. With
  only the target, the five bold rows in §5 would be invisible and the
  implementation slices could not tell which assertion they flip.
- **Drive the model from the real SQL predicate.** Rejected for this slice: it needs
  a store, and the slice owns pure fixtures only. The model cites the functions it
  mirrors, and the implementation slice adds a store-backed test that compares
  them.
- **Pick a by-id batch cap now.** Rejected: #9178 does not settle one. The model
  reuses `BATCH_CLAIM_MAX` (5) and records the question in §8.

## 8. Open questions

- **By-id batch cap.** Whether a by-id batch shares the automatic count batch's
  `BATCH_CLAIM_MAX` or the per-launch `BUNDLE_CLAIM_CAP` (10). Default in force:
  `BATCH_CLAIM_MAX`, unchanged.
- **FDR-0004 wording.** FDR-0004's consequences say a manual-only item at the top
  of the queue makes an automatic lane claim nothing, while its decision says the
  pick excludes the row unconditionally. The model follows the decision: the row
  is invisible to the pick, which then takes the next eligible row. If the
  consequence sentence is meant literally, it is a head-of-line block and needs its
  own record.
- **Attempt accounting under override.** FDR-0002 keeps attempt history and
  bypasses the cap for the overriding claim. Whether that claim adds an attempt is
  left to the implementation slice; the model only records that the override was
  load-bearing.

## 9. Compatibility and rollback

No stored value, column, verb or spelling changes. The model is additive code
with tests, so rollback is reverting the PR. Source, deployment and live claim
evidence stay distinct, as #9512 requires: this record is source evidence only.
