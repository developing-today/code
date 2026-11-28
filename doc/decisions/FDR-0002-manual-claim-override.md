# FDR-0002 — Manual claim as an owner override for parked and attempt-capped tasks

- **Date:** 2026-11-17
- **Status:** accepted
- **Owner:** Fleet queue maintainers
- **Decision maker:** Drew — directive of 2026-11-12, specified in the companion specification
- **Scope:** claiming a specific queued, parked, or attempt-capped task by ID under a recorded manual-claim override, and the boot behavior that follows
- **Supersedes:** none
- **Superseded by:** none
- **Related records:** FDR-0001, FDR-0003, FDR-0008, FDR-0009
- **Implementation:** [#9178 §2](https://github.com/developing-today/code/issues/9178); verb conventions from [#9016](https://github.com/developing-today/code/issues/9016); attempt accounting per the #3424 data model

## Context and problem

The queue had no owner verb to claim a specific task, and in particular no way to
pick up a **parked** or **attempt-capped** task deliberately. Two failures drove
this:

1. During [#9155](https://github.com/developing-today/code/issues/9155) the bound
   task was stopped and parked, and the queue offered no way to say "I know it is
   parked; bring it back under my session".
2. The only routes around a park were informal: edit the row, wait for the
   condition, or create a duplicate task. None recorded who overrode what or why.

There is also a trap to design against: if a manual claim is just "set status to
claimed", the supervision that parked the task on boot can immediately park it
again for the same unchanged condition — the override would be defeated in
seconds.

Sources: [#9178 §2](https://github.com/developing-today/code/issues/9178); incident
[#9155](https://github.com/developing-today/code/issues/9155) (queue task 15345).

## Source links

- Originating issue: [#9178 §2](https://github.com/developing-today/code/issues/9178)
- Directive: Drew, 2026-11-12 ("claim any specific task by number, including a parked or attempt-capped task, under an explicit owner override")
- Incident: [#9155](https://github.com/developing-today/code/issues/9155), queue task 15345
- Related: FDR-0008 (purposeful parks and owner release), FDR-0009 (reboot test)

## Options considered

| Option                                 | What it means                                                                              | Assessment                                                                                                      |
| -------------------------------------- | ------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------- |
| A. Explicit manual-claim override verb | Claim by numeric ID; record an override; supervision honors it for the unchanged condition | Chosen. Auditable, does not fake state, and preserves attempt history                                           |
| B. Direct row edit / SQL fix-up        | Operator edits status by hand                                                              | Unaudited, invisible to supervision, and exactly the class of action that made the #9155 closure unattributable |
| C. Auto-claim by priority boost        | Raise priority so the normal claim picks it up                                             | Not a contract: another lane, a drain, or a boost can take it first; and it cannot express "this specific task" |
| D. Do nothing                          | Parked/attempt-capped tasks stay unreachable except by hand                                | Leaves the #9155 recovery improvised, which is the problem being fixed                                          |

## Decision

- Provide a fleet CLI verb to claim one task by numeric ID (working name
  `fleet queue claim --task <id>`), following the existing verb conventions
  ([#9016](https://github.com/developing-today/code/issues/9016)).
- The claim path may claim a task that is **queued, parked, or at `max_attempts`**.
  It never pretends those states do not exist; it records an explicit
  **manual-claim override**.
- The override record includes: claimant identity, session id when known,
  timestamp, reason, prior task status, prior `not_before`/park reason, and prior
  attempt count.
- A manual claim does **not** reset attempt history. The max-attempts gate is
  bypassed for this claim by the recorded override; the historical attempts stay
  visible.
- The resulting claim carries an **override marker/epoch** that startup and
  supervision logic can see. A worker booting under a manual claim must not
  immediately re-stop or re-park the task for **the same unchanged condition** that
  caused the earlier park. A **materially new** condition may still stop or park
  it, and that new condition is recorded.
- Repeating the same claim while the same worker/session owns it is **idempotent**
  and returns the existing claim. A claim by a different worker follows FDR-0003's
  skip/take-over rules.

## Rationale and evidence

**Measured / sourced facts:** the #9155 recovery has no queue-native verb in its
record; the parked task was resumed outside the queue's claim semantics. The
tasks table already records attempt counts and park reasons, so both the
before-state and the override can be attached to the same row without a new
table.

**Inference:** an override marker is the minimum mechanism that satisfies both
requirements at once — the claim must survive supervision (which is automated and
fires on boot) and the decision must be attributable (which is what #9155 lacked).
Option B fails the second property by construction; option C fails the first.

## Consequences

- **Behavior changes:** a new verb can claim tasks that were previously
  unreachable; supervision gains an override-aware gate. Claim results for parked
  tasks gain the override/prior-state block.
- **Compatibility / migration:** additive — new optional fields and a new verb.
  Existing claims carry no override marker and behave as today.
- **New failure modes:** an override that is honored after its reason has expired
  (the task becomes stuck in a "protected" state). Mitigation: the override is
  scoped to the recorded condition, not blanket immunity, and it is visible in
  listings.
- **Monitoring and tests required:** a specific queued task claims by ID without
  lane selection; a parked task claims by ID and the record shows override, prior
  park state, actor, session, time, reason; a `max_attempts` task claims by ID
  without erasing history; a test reproduces the original park condition and
  proves the first boot does not re-park solely for it; a second claim by the same
  owner is idempotent; a claim by another owner does not disturb the live claim
  without take-over.

## Revisit triggers

- Overrides that outlive their stated reason and block real park conditions —
  argues for an override expiry or a re-validation check.
- Evidence that supervision re-parks anyway (the #9155 shape recurring under a
  manual claim) — that would reopen FDR-0009 as well.
- The verb surface settling differently under #9016's conventions.

## Open questions

None. The sub-choices (verb spelling, override field names) are implementation
detail fixed by #9178 §2's acceptance criteria and #9016's conventions; the
semantics above are not open.
