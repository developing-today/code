# FDR-0009 — A park that leaves its trigger unchanged fails the reboot test

- **Date:** 2026-11-17
- **Status:** proposed
- **Owner:** Fleet queue maintainers / supervision owners
- **Decision maker:** Drew — analysis of the #9155 stop, 2026-11-12; recorded as proposed until the stopper investigation completes
- **Scope:** the test a park must pass before it is recorded as remediation rather than delay
- **Supersedes:** none
- **Superseded by:** none
- **Related records:** FDR-0002 (manual claim boot behavior), FDR-0006 (remediation ladder), FDR-0008 (park terms)
- **Implementation:** [#9177](https://github.com/developing-today/code/issues/9177) (stopper root cause and completion guard); [#9178 §2](https://github.com/developing-today/code/issues/9178) (boot behavior under override); supervision park paths

## Context and problem

In the #9155 stop, the watcher that could park the task had **already established
that the work was bound and known** — it knew which session owned which task. The
park it performed left the triggering condition exactly as it was: the same
session, the same location, the same task state. Nothing about the work's
situation improved; the work simply stopped.

Drew's analysis names the failure: a park that leaves its trigger unchanged fails
the **reboot test**. Restart the worker, re-read the state, and the same
supervision would park the task again for the same reason. That is a loop, not a
remedy — the park bought delay and presented it as resolution.

Sources: Drew's analysis of the #9155 stop, 2026-11-12 (recorded as a seed in
[#9180](https://github.com/developing-today/code/issues/9180));
[#9177](https://github.com/developing-today/code/issues/9177) is the open
investigation of the stopper.

## Source links

- Ruling: Drew, 2026-11-12, analysis of the [#9155](https://github.com/developing-today/code/issues/9155) stop
- Incident: [#9155](https://github.com/developing-today/code/issues/9155), queue task 15345
- Investigation: [#9177](https://github.com/developing-today/code/issues/9177) — the unidentified App-identity writer, completion guard, detective control
- Related: FDR-0002 (an override must survive the boot), FDR-0006 (the ladder), FDR-0008 (parks need terms)

## Options considered

| Option                                           | What it means                                                                              | Assessment                                                                                                     |
| ------------------------------------------------ | ------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------- |
| A. Apply the reboot test before recording a park | A park is valid only if re-reading state would not immediately re-park for the same reason | Chosen. Cheap test, catches the exact #9155 failure                                                            |
| B. Count parks and alert on repeats              | Detect loops after the fact                                                                | Useful as a control, but does not prevent the first bad park and depends on someone reading the count          |
| C. Trust supervision to know when a park helps   | Leave it to the watcher's judgement                                                        | The #9155 watcher _had_ the knowledge and still parked into no change; judgement without a test is what failed |
| D. Do nothing                                    | Parks keep being recorded as remediation                                                   | The loop stays invisible: each park looks like a handled event                                                 |

## Decision

- A park performed by a watcher that can see the task is bound/known must pass the
  **reboot test** before it is recorded as remediation:

  > If the worker restarted now and re-read the same state, would the same
  > condition cause the same park again?

  If **yes**, the park is **delay, not remediation**. It must not be recorded as a
  resolution; the actor must either apply FDR-0006's next rung (steer, stop with
  preservation, restart elsewhere) or record the park as an explicit
  **repeating-condition hold** with a term (FDR-0008), acknowledging it will recur.

- A park that leaves the triggering condition unchanged must record that fact —
  the condition, and the fact that the park did not alter it — so the next reader
  and the next boot can see that the hold is not a fix.
- A **materially new** condition may justify a new park; a repeat of the recorded
  one does not (this is the same distinction FDR-0002 draws for manual claims).
- Where the stopper cannot tell whether a condition changed, it must record
  "unknown" rather than assert resolution.

## Rationale and evidence

**Measured / sourced facts:** the #9155 watcher had bound-task knowledge and
parked anyway; the park left session, location, and task state unchanged. The
unidentified automation that later closed the epic ([#9177](https://github.com/developing-today/code/issues/9177))
compounded the same incident.

**Inference:** the reboot test is a **cheap decision procedure** for a property
that is otherwise a judgement call. Because the same state would produce the same
park, "resolved" is false by construction — no measurement of the condition is
required to see it.

**Why this is `proposed`, not `accepted`:** #9180 directs that this be recorded as
proposed until the stopper investigation is complete, because the root cause may
change which component is responsible for applying the test (supervisor, watcher,
or the completion guard in #9177). The **principle** is settled; the **placement**
is not.

## Consequences (if accepted in this form)

- **Behavior changes:** park records gain the reboot-test outcome and the
  condition's before/after state; a repeating-condition hold is a distinct state
  from a remediating park.
- **Compatibility / migration:** additive fields on the existing park/status-reason
  record; historical parks are not rewritten, but a review can classify them.
- **New failure modes:** a watcher that cannot determine the condition's state may
  label everything "unknown" and never remediate — the ladder (FDR-0006) is what
  forces a next rung rather than an endless hold.
- **Monitoring and tests required:** a test that re-running supervision against an
  unchanged state produces a repeating-condition hold and not a fresh "resolved"
  park; a test that a materially new condition parks normally; a count of
  repeating-condition holds as the detective control.

## Revisit triggers

- [#9177](https://github.com/developing-today/code/issues/9177)'s root-cause
  findings landing — **this is the settling event** and should either accept this
  record in place or amend it with the responsible component.
- Evidence that the test is being satisfied cosmetically (a park records
  "condition changed" with no evidence of change).
- The completion guard from #9177 making parks by unidentified writers impossible —
  that changes the population of actors this record governs.

## Open questions

- **OQ-1 — which component applies the test: the watcher, the supervisor, or a
  boot-time check?** **Default in force:** the component that performs the park
  evaluates it and records the outcome, because it has the state in hand; a
  boot-time check is the backstop. Settled by: [#9177](https://github.com/developing-today/code/issues/9177).
