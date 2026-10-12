# Phase 6: invites, pull, and the CLI

- Plan: [phased plan](../2026-10-11T11-08-42Z_design_remaining_work/plan-phases.md), section 11.
- Start commit: 479cc76c (origin/main, after Phase 5 tags were started elsewhere; Phase 4 merged as #191 and #192).
- Base branch `phase-6`. Tracks run in parallel:
  - `p6-seitan`: P6.1 Seitan V2, P6.2 freshness window.
  - `p6-invites`: P6.3 tickets (option E), P6.4 invite events and links.
  - `p6-recovery`: P6.6 fork recovery, P6.8 rejection reasons.
  - Later: P6.5 pull policy, P6.7a and P6.7b CLI (groups, delegation).
  - Blocked: P6.7c CLI documents and tags needs Phase 5 merged.
- Decisions in force: freshness window default 5 minutes (N23); ticket option E (N24); address exposure documented (N30); pull open with an authorized option (decision 11); fork recovery by remove and re-enrol under a new device key (decision 17).

## Steps

| Step  | Track    | Status  | Commits  | Tests                  | Notes                                                                                                                                        |
| ----- | -------- | ------- | -------- | ---------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| P6.1  | seitan   | todo    |          |                        | verify accept-message bytes and the Keybase admin handler before shipping                                                                    |
| P6.2  | seitan   | todo    |          |                        |                                                                                                                                              |
| P6.3  | invites  | todo    |          |                        |                                                                                                                                              |
| P6.4  | invites  | todo    |          |                        |                                                                                                                                              |
| P6.5  | pull     | todo    |          |                        |                                                                                                                                              |
| P6.6  | recovery | done    | 02e7dd8b | 1 unit, 2 updated      | Recovery takes a removed key and an added key (N8 passes one key twice). Quarantine with approval is a later option (decision 17), not built |
| P6.7a | cli      | todo    |          |                        |                                                                                                                                              |
| P6.7b | cli      | todo    |          |                        |                                                                                                                                              |
| P6.7c | cli      | blocked |          |                        | needs Phase 5                                                                                                                                |
| P6.8  | recovery | done    | 7ce0f1df | 11 unit, 1 integration | One `Reason` enum (`src/rejection.rs`), mapped by `reason()` on the event store, the fold and the adapter. See the notes below               |

## P6.6 and P6.8 notes (recovery track)

- **Red, then green.** P6.6: the red test calls `Session::recover` with a removed key and an added key. It did not compile until the signature took both (three E0061 errors), and it passes after the change. P6.8: the red tests call `reason()`, which did not exist (12 errors: the missing `rejection` module and the missing methods). They pass after the change.
- **Fork recovery reuses `Session::recover`.** The removal takes the old key out, and `add_from_join` takes the new key in. The forked copy cannot read messages sent after the removal, and the new key reads only from its re-enrol point. Quarantine with approval is recorded as a later option (task 10, decision 17) and is not built.
- **One reason enum.** `Reason::{Voided, Waiting, Frozen}` (`src/rejection.rs`), with the design's words as its display. The mapping for each surface:
  - Event store (`Ingest::reason`): `Refused(Frozen)` is frozen. `Refused(WaitingFull)` and `Waiting` are waiting. `Voided` is voided. `Malformed`, `Accepted`, `Duplicate` and `Fork` give none.
  - Groups fold (`Refusal::reason`): `Dissolved` is frozen, by the removal or policy that dissolved the group. `NoGroup` and `NotActive` are waiting. Every other rule refusal is voided.
  - Adapter (`SchemeError::reason`): `Poisoned` and `RosterMismatch` are frozen. `NotWelcomed` and `History` are waiting. `NotInModel`, `StillInModel`, `UnaskedSecret` and `SchemeMismatch` are voided. `NoScheme`, `UnsupportedScheme`, `Crate`, `InvalidKey` and `TooManyParents` give none.
- **Refusals with no reason** are errors about bytes, keys or a local precondition, not a judgement on an event. The owner should confirm this reading.
- **Test layer.** The plan says "e2e: each reason appears". These are unit and integration tests, one per reason per surface. The CLI e2e comes with P6.7a.
