# FDR-0008 — Every park has a purposeful term and an owner-release path

- **Date:** 2026-11-17
- **Status:** accepted
- **Owner:** Fleet queue maintainers
- **Decision maker:** Drew — exposed by the #9155 stop, 2026-11-12; the manual-claim side is specified in the companion specification
- **Scope:** what a park must record, and the requirement that an owner can always release one
- **Supersedes:** none
- **Superseded by:** none
- **Related records:** FDR-0002 (manual claim is the release mechanism that exists) , FDR-0006 (park as last resort), FDR-0009 (reboot test)
- **Implementation:** [#9178 §2](https://github.com/developing-today/code/issues/9178) (claim-by-ID release path); **named gap:** the owner-unpark verb does not exist — [#9180](https://github.com/developing-today/code/issues/9180) tracks it

## Context and problem

During [the #9155 incident](https://github.com/developing-today/code/issues/9155)
the bound task was parked, and the queue offered **no owner-unpark verb**. The
park had no stated term and no release path: the task could be brought back only
by improvised means outside the queue's semantics. A park in that shape is not a
hold — it is a stop that nobody can undo through the tool that made it.

The same gap appeared twice over: the absence of release meant the park could not
be answered by the person responsible for the work, and the absence of a stated
term meant nobody could tell whether the park was still valid a day later.

Sources: [#9155](https://github.com/developing-today/code/issues/9155) (queue task
15345); the ruling is a seed in
[#9180](https://github.com/developing-today/code/issues/9180); the manual-claim
side is [#9178 §2](https://github.com/developing-today/code/issues/9178).

## Source links

- Ruling: Drew, 2026-11-12, exposed by the [#9155](https://github.com/developing-today/code/issues/9155) stop
- Incident: [#9155](https://github.com/developing-today/code/issues/9155), queue task 15345
- Manual-claim side: [#9178 §2](https://github.com/developing-today/code/issues/9178)
- Gap record: [#9180](https://github.com/developing-today/code/issues/9180) (this record set's issue)
- Data model: [`doc/2026-11-10_design_queue_task_data_model_3424.md`](../2026-11-10_design_queue_task_data_model_3424.md) (park revalidation, `not_before`, status reasons)

## Options considered

| Option                                                                 | What it means                                                                     | Assessment                                                                                                               |
| ---------------------------------------------------------------------- | --------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| A. Park requires a term and an owner-release path; absence is a defect | Every park states why, until when or under what condition, and who can release it | Chosen. Makes park a reversible hold by construction                                                                     |
| B. Park freely; rely on manual claim as the release                    | What #9178 §2 partially supplies                                                  | Incomplete alone: a release path exists only once #9178 §2 ships, and it still does not require the park to state a term |
| C. Rely on revalidation to notice stale parks                          | The #3424 long-park revalidation cadence                                          | Necessary but not sufficient: revalidation flags a stale park, it does not give the owner a verb to end it               |
| D. Do nothing                                                          | Parks stay as they are                                                            | This is the measured state that produced an unreleasable task                                                            |

## Decision

- Every park records, at the moment it is created:
  - **why** — the condition that forces the hold (not "supervision decided");
  - **a term** — a `not_before` datetime, an external condition with a check, or
    an explicit "until a human decides", each of which is a real term;
  - **who can release it** — the owner or role, recorded on the park.
- Every park has an **owner-release path through the queue's own verbs**. Today
  that path is the manual claim by task ID ([#9178 §2](https://github.com/developing-today/code/issues/9178)).
  The **absence of a dedicated owner-unpark verb is a named gap, not an acceptable
  permanent state** — the gap is tracked on [#9180](https://github.com/developing-today/code/issues/9180)
  and closes when the queue can release a park without claiming it.
- A park without a term, or without a release path, is **invalid**: it must be
  rejected at creation or flagged for correction, not left in the table.
- Refusing to park is always permitted; parking badly is not. Where
  FDR-0006's ladder still has a rung left, park is not the answer.

## Rationale and evidence

**Measured / sourced facts:** in the #9155 incident the parked task had no owner
unpark verb in the queue surface; the recovery happened outside the queue's
semantics. The park also carried no term that would let anyone decide whether it
was still valid.

**Inference:** a park with a term and a release path is a **hold**; a park with
neither is a **cancel that nobody recorded as one**. The distinction is not
cosmetic: the queue's watchers, reports, and claim query all treat parked items as
waiting, so an unreleasable park makes the reporting lie about the work's state.

## Consequences

- **Behavior changes:** park creation validates its inputs; the manual claim by ID
  is the release path until a dedicated verb exists; parks gain recorded
  owner/term fields.
- **Compatibility / migration:** existing parks may lack a term — they need an
  audit pass that either adds a term or converts them (see FDR-0004's no-blanket
  rule: most of these mean something else and must be classified individually).
- **New failure modes:** requiring a term can tempt authors to write a fake date
  to satisfy validation. The guard is FDR-0009's reboot test: a term that does not
  change the condition is not a term.
- **Monitoring and tests required:** park without term is rejected (or flagged);
  park with a term is claimable/releasable by the owner through the queue verb;
  the release records actor, time, and reason; a listing distinguishes
  term-bearing from termless parks.

## Revisit triggers

- A dedicated owner-unpark verb landing — this record's "named gap" clause is then
  satisfied and should be updated to cite the verb.
- Termless parks accumulating despite validation, indicating the validation is
  bypassable.
- The #3424 revalidation work changing what "a term" means mechanically.

## Open questions

- **OQ-1 — is the release path claim-by-ID, or a separate unpark verb?**
  **Default in force:** claim-by-ID is the release path now, because it ships as
  part of #9178 §2 and needs no new surface; a dedicated verb is preferred once it
  exists. Settled by: the issue that adds the owner-unpark verb, which updates this
  record rather than minting a new ID.
