# FDR-0003 — Multi-task claim under one session, and `--take-over` semantics

- **Date:** 2026-11-17
- **Status:** accepted
- **Owner:** Fleet queue maintainers
- **Decision maker:** Drew — directive of 2026-11-12, specified in the companion specification
- **Scope:** batch claim of several tasks by one session; per-task outcomes; explicit transfer of an existing claim
- **Supersedes:** none
- **Superseded by:** none
- **Related records:** FDR-0001, FDR-0002, FDR-0004
- **Implementation:** [#9178 §3](https://github.com/developing-today/code/issues/9178); verb conventions from [#9016](https://github.com/developing-today/code/issues/9016); lease/claim rules in [`doc/fleet/queue-fallback-mode.md`](../fleet/queue-fallback-mode.md) and the #3424 data model

## Context and problem

One session doing related work often needs several queue items — a fix plus its
follow-up, or a batch of small fixes. The queue exposed one claim per invocation,
so a session that needed three items either claimed three times, racing other
lanes each time, or claimed one and hand-edited the rest. Separately, there was no
sanctioned way to take an already-picked-up task off a worker that had gone
silent or was working in the wrong place; the only options were to wait or to
improvise.

Sources: [#9178 §3](https://github.com/developing-today/code/issues/9178);
motivating case [#9155](https://github.com/developing-today/code/issues/9155) —
a bound task whose supervision needed to change hands without inventing a
mechanism.

## Source links

- Originating issue: [#9178 §3](https://github.com/developing-today/code/issues/9178)
- Directive: Drew, 2026-11-12 ("claim several tasks under one session")
- Related: [#8097](https://github.com/developing-today/code/issues/8097) (one-queue matching model; by-id and transfer claims are the explicit special cases it names)
- Related: [`doc/agents/one-queue-claiming-work.md`](../agents/one-queue-claiming-work.md) (lanes do not claim by task ID by default)

## Options considered

| Option                                                       | What it means                                     | Assessment                                                                                                                                      |
| ------------------------------------------------------------ | ------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| A. Batch claim with per-task results, explicit `--take-over` | One invocation, many IDs, honest per-task outcome | Chosen. Composes with FDR-0001/0002/0004 and keeps the skip default                                                                             |
| B. Loop the existing single claim verb                       | Client-side repetition                            | Each iteration races other lanes and can half-succeed with no single record of the batch; ambiguity about whether a partial result is a success |
| C. Take-over by taking the lease implicitly                  | Reuse the lease-expiry path                       | Silent transfer: the prior worker is not recorded, and a live worker can be robbed without a decision trail                                     |
| D. Do nothing                                                | Sessions claim one item at a time                 | Leaves the one-session-many-items case improvised, which is why the section exists                                                              |

## Decision

- Extend the claim verb to accept multiple task IDs for one session (for example
  `fleet queue claim --task <id1> --task <id2> --session <session-id>`).
- Report the result **per task**: `claimed`, `already-owned-by-this-session`,
  `skipped-claimed`, `not-found`, or `failed`, with the reason for every
  non-claim.
- Tasks already picked up by another worker are **skipped by default**; their
  existing claims are left untouched.
- An explicit `--take-over` flag may claim an already-picked-up task. A take-over
  **atomically transfers** the lease/claim, ends or supersedes the prior claim per
  the queue's lease rules, and records prior worker/session, new worker/session,
  actor, time, and reason. Where the prior worker can be signalled it receives a
  hand-back/stop signal; where it cannot, **that limitation is recorded**.
- Resume-by-session (FDR-0001), parked and max-attempts (FDR-0002), and
  manual-only (FDR-0004) semantics **compose** with batch claims: each task's
  override, take-over, or manual authorization is independently auditable.

## Rationale and evidence

**Measured / sourced facts:** the queue's claim is a single-task operation today,
and the one-queue model records lanes' default claims as unfiltered, with by-id
and transfer claims named as explicit special cases ([#8097](https://github.com/developing-today/code/issues/8097)).
The lease rules and their 30-minute fallback window are recorded in
[`doc/fleet/queue-fallback-mode.md`](../fleet/queue-fallback-mode.md), so "ends or
supersedes the prior claim" has a defined meaning rather than an invented one.

**Inference:** a per-task result set is what makes a batch auditable — a single
aggregate exit status cannot say which task was skipped and why, which is the
failure mode of option B. Recording the prior worker on take-over is what
distinguishes a transfer from a silent steal; the explicit fallback ("where the
prior worker cannot be signalled, record that") prevents the mechanism from
claiming a guarantee it cannot keep.

## Consequences

- **Behavior changes:** one invocation may claim several tasks; output becomes a
  per-task result list; take-over can move a live claim.
- **Compatibility / migration:** additive. A single-ID call behaves as before and
  returns the same per-task shape it always implied.
- **New failure modes:** partial batch failure is now a normal outcome and must
  not be treated as total failure or total success by callers; a take-over against
  a worker that is in fact still writing can produce two writers on one task if
  the stop signal is not honored — the recorded limitation is the only honest
  answer where signalling is impossible.
- **Monitoring and tests required:** claiming two unclaimed tasks for one session
  returns a per-task result; already-claimed tasks are skipped with their claims
  unchanged; `--take-over` transfers an already-claimed task with both sides
  audit-logged; a mixed batch (unclaimed, owned by requester, owned by another,
  missing, parked) yields the specified per-task outcomes and disturbs nothing
  that was skipped.

## Revisit triggers

- Measured double-writers after take-over — argues for a fencing token on the
  claim rather than lease-supersession alone.
- The lease model changing in FDR-0001/FDR-0009 work or the queue cutover.
- A need to claim across repos in one call by more than incidental coincidence
  (the one-queue model already allows it; this trigger is about batch ergonomics).

## Open questions

- **OQ-1 — stop signal when the prior worker has no reachable session.**
  #9178 §3 requires recording the limitation where the prior worker cannot be
  signalled. **Default in force:** perform the take-over, record
  `handback_signal=unavailable` with the reason, and rely on the prior worker's
  own claim-check to fail closed. Reason: refusing the take-over would leave a
  stuck task stuck, which is the condition take-over exists to fix. Settled by:
  the #9178 §3 implementation issue when it defines the signal inventory.
