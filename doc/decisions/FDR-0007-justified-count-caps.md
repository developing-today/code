# FDR-0007 — An artificial count cap must state the reason it exists

- **Date:** 2026-11-17
- **Status:** accepted
- **Owner:** Fleet queue maintainers
- **Decision maker:** Drew — rulings of 2026-11-12 covering minder action caps, lane bundle caps, reconcile write/scan caps, and the collector page cap
- **Scope:** any artificial limit that bounds work intake, recovery, batching, or reporting
- **Supersedes:** none
- **Superseded by:** none
- **Related records:** FDR-0002 (attempt caps are a different kind of cap and are not abolished by this record), FDR-0005 (transaction-log retention is a justified cap), FDR-0009
- **Implementation:** merged example [#9059](https://github.com/developing-today/code/pull/9059) (collector paginates to exhaustion); reviewer check applied to new caps in `pkgs/fleet` and `scripts/fleet`

## Context and problem

The fleet kept acquiring small numeric limits that silently defined how much work
could happen. Each looked harmless in isolation; together they produced failures
where work was **quietly not done**:

- a minder action cap limited how many actions one pass could take;
- lane bundle caps limited how much a lane could claim or hand off;
- reconcile write/scan caps limited how many rows a reconciliation could touch or
  read;
- the collector stopped reading the Project pool at a fixed page count.

The collector case is the measured example: the cap made the pool appear complete
when it was not, and the fix ([#9059](https://github.com/developing-today/code/pull/9059))
was to paginate to exhaustion. In every case the cap's number had no recorded
reason — it was a default someone chose once.

Sources: Drew rulings, 2026-11-12 (recorded as the seed for
[#9180](https://github.com/developing-today/code/issues/9180));
[#9059](https://github.com/developing-today/code/pull/9059) as the merged
corrective example.

## Source links

- Ruling: Drew, 2026-11-12 (four named cap classes)
- Merged example: [#9059](https://github.com/developing-today/code/pull/9059) — paginate the Project pool to exhaustion
- Incident family: [#6619](https://github.com/developing-today/code/issues/6619) / [#6728](https://github.com/developing-today/code/issues/6728) shape (a silent limit reported as success)
- Related: [#3424](https://github.com/developing-today/code/issues/3424) (attempt accounting, a _justified_ cap family)

## Options considered

| Option                                              | What it means                                                                       | Assessment                                                                                                                     |
| --------------------------------------------------- | ----------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| A. Every cap states a reason, or is removed         | A cap is allowed only with a named performance, safety, or provider limit behind it | Chosen. Keeps genuinely necessary caps, removes accidental ones, and makes each one reviewable                                 |
| B. Remove all caps                                  | Paginate and process to exhaustion everywhere                                       | Ignores real provider page limits and real protection needs; some caps are load-bearing (GitHub API pagination, memory bounds) |
| C. Leave caps as they are, document as time permits | Status quo                                                                          | This is the state that produced the failures; undocumented caps keep being added by copy-paste                                 |
| D. Make caps configurable instead of fixed          | Turn limits into settings                                                           | Moves the problem: a configurable cap with no stated reason still silently bounds work, and the default is what runs           |

## Decision

- A cap that **binds work intake, recovery, batching, or reporting** must state, in
  code beside the constant and in the doc that governs it, the **performance,
  safety, or provider-limit reason** it exists. "It seemed enough" is not a reason.
- A cap without such a reason is a **defect**: the fix is to remove it (process to
  exhaustion, or drive the loop off a completion condition) or to justify it in
  place. Either is acceptable; leaving it unstated is not.
- Where a cap is kept, record what happens **at** the cap: does the caller know it
  was truncated? A cap whose truncation is reported as success is worse than the
  cap.
- New caps inherit the rule: a PR adding one states the reason in the same body of
  work, and a reviewer may block a cap whose reason is missing.

**This record does not abolish attempt caps or quota-based admission controls.**
Those are capped by design for named reasons (bounded retry, provider quota); they
are in scope only in that their reason must be stated and their truncation
visible.

## Rationale and evidence

**Measured / sourced facts:** the collector cap is a merged, fixed instance: the
Project pool was read to a page limit and the result treated as complete, and
[#9059](https://github.com/developing-today/code/pull/9059) changed it to paginate
to exhaustion. The other three cap classes are named in the 2026-11-12 rulings.

**Inference:** the common failure mode is not the number but the **silence** — a
cap that reports success while dropping work. Requiring a stated reason forces the
author to say what the cap protects, which in practice either produces a real
reason or exposes that there is none. Option D fails because the default value is
what runs in unattended lanes.

## Consequences

- **Behavior changes:** some caps are removed (loops run to a completion
  condition); kept caps gain a comment and, where missing, a truncation signal.
- **Compatibility / migration:** no schema change. Callers that depended on
  bounded output volume may see larger results — this is the intended change and
  should be noted per call site.
- **New failure modes:** removing a cap can expose unbounded loops against a slow
  or paginated provider; that is why "process to exhaustion" means "until the
  provider's own completion condition", not "until nothing happens". Justified caps
  must keep a stated bound.
- **Monitoring and tests required:** for each removed cap, a test that a
  previously-truncated input is now fully processed (the #9059 shape); for each
  kept cap, a test that hitting it is observable rather than silent; a review
  checklist item for new caps.

## Revisit triggers

- A removed cap causing measured load or provider throttling — the cap returns with
  a stated reason and a visible truncation.
- A kept cap being hit routinely in production — the number, not only the reason,
  is then wrong.
- Adoption of FDR-0005's transaction log, which is itself a retention cap and must
  comply with this rule.

## Open questions

None. Where an individual cap's reason or number is unresolved, that is a defect to
file — the rule itself is not in question.
