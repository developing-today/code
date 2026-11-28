# FDR-0004 — Manual-only queue items: a first-class flag, not a park or a low priority

- **Date:** 2026-11-17
- **Status:** accepted
- **Owner:** Fleet queue maintainers
- **Decision maker:** Drew — directive of 2026-11-12; recommendation recorded in the companion specification
- **Scope:** how work that automatic lanes must never claim is represented in the queue
- **Supersedes:** none
- **Superseded by:** none
- **Related records:** FDR-0002, FDR-0003, FDR-0008
- **Implementation:** [#9178 §4](https://github.com/developing-today/code/issues/9178); claim-query model in [#8097](https://github.com/developing-today/code/issues/8097); attempt/park accounting per the #3424 data model

## Context and problem

Some work must not be claimed automatically: direct-dispatch design-mill items
bound to a named session are the immediate case. The queue had no first-class way
to say so. Teams made such items effectively manual by **parking** them or by
relying on dispatch discipline. Neither says what it means:

- a park means "waiting for a stated condition or date", so a manual item parked
  for human pickup is misdescribed, and park-aware watchers and reports treat it
  as stalled or blocked;
- very low priority is a race, not a contract: a priority boost, a queue drain, a
  lane-scope change, or an empty lane can all make the item claimable by an
  automatic lane.

The failure is silent when it happens: a design-mill item claimed by the wrong
lane looks like a normal claim in every listing.

Sources: [#9178 §4](https://github.com/developing-today/code/issues/9178);
[#9155](https://github.com/developing-today/code/issues/9155) (direct-dispatch
design-mill work); [#8097](https://github.com/developing-today/code/issues/8097)
(one-queue claim matching).

## Source links

- Originating issue: [#9178 §4](https://github.com/developing-today/code/issues/9178)
- Directive: Drew, 2026-11-12 ("create work that automatic lanes never claim")
- Related: [#8097](https://github.com/developing-today/code/issues/8097), FDR-0002 (the only path that may claim such an item)

## Options considered

| Option                                                 | What it means                                                                  | Assessment                                                                                                                                    |
| ------------------------------------------------------ | ------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------- |
| A. First-class `manual_only` / `no_auto_claim` boolean | Automatic claim query excludes it unconditionally; only by-id claim reaches it | Chosen. States the actual contract, is auditable, and is distinguishable from parked/blocked in listings                                      |
| B. Create as parked, then claim by hand                | Stopgap, no schema change                                                      | Conflates "waiting for a condition" with "waiting for a human"; inherits park watchers and stall reporting; requires unparking to work at all |
| C. Very low priority                                   | Stopgap, no schema change                                                      | Not a guarantee — explicitly a race; priority changes and drains defeat it                                                                    |
| D. Do nothing                                          | Keep advertising items that automatic lanes may take                           | The silent-wrong-claim failure stays, and it is not detectable after the fact                                                                 |

## Decision

- Adopt a first-class boolean on `tasks` (working name `manual_only`) meaning
  "automatic claim must never select this".
- The **automatic claim query excludes `manual_only = true` unconditionally** —
  priority, age, lane, and queue depth do not change that.
- The item remains visible in queue listings and dashboards, with its manual-only
  state shown as an **intentional** state, not as parked, stalled, or blocked.
- Only the explicit claim-by-ID path (FDR-0002) may claim it. Batch claims follow
  the same rule per task.
- The flag may be set at enqueue time or by an authorized later update; setting
  and clearing it is **audit-logged**. Clearing it makes the item eligible for
  normal automatic claiming.
- **Migration:** audit current parked-for-manual/direct-dispatch items
  individually and convert only those whose record shows they are waiting for a
  human or named session. **No blanket conversion** — most parks mean something
  else (FDR-0008).

## Rationale and evidence

**Measured / sourced facts:** the #9178 §4 assessment table records that parked
items inherit watcher and reporting semantics that describe them as stalled, and
that priority is mutable by boosts, drains, and lane-scope changes. The one-queue
claim model ([#8097](https://github.com/developing-today/code/issues/8097)) is a
single unfiltered query by default, so an unconditional exclusion is expressible
there without a second queue.

**Inference:** only an explicit exclusion in the claim query is a contract.
Park and low priority are both _influences_ on the query, not exclusions, so both
can be defeated by an ordinary operation. The "intentional state" requirement
follows from the same evidence: if the item looks stalled, operators will
"remediate" it, so the representation has to be distinct from park.

## Consequences

- **Behavior changes:** a new claim-query predicate; new audited updates; listings
  gain a manual-only marker. A manual-only item at the top of the queue causes an
  automatic lane to claim **nothing** rather than the next item — that is the
  intended contract, and it is a change lanes' shift logic must expect.
- **Compatibility / migration:** additive nullable boolean, default false; the
  migration report is required because existing parked items are ambiguous.
- **New failure modes:** work parked in `manual_only` that nobody ever claims is
  now invisible to automatic remediation signals — it is deliberate stalling, and
  it needs a listing someone actually reads. Blanket-converting parks would hide
  real blocked work behind a manual-only label; the no-blanket rule is the guard.
- **Monitoring and tests required:** with a manual-only item as the only
  otherwise-claimable item, an automatic lane run claims nothing; the same item is
  claimable through by-id claim, including under a named session; listings
  distinguish manual-only from parked and blocked; a migration report lists
  current candidates and each disposition.

## Revisit triggers

- Manual-only items aging without pickup in numbers that indicate the flag is
  being used as a parking lot — argues for an age surface or a review requirement.
- Lane shift logic being repeatedly starved by manual-only head-of-queue items —
  argues for surfacing them in the lane's "nothing claimed" reason (already
  required by the listings rule; revisit if insufficient).
- A future directive allowing a named session to be pre-bound instead of using
  by-id claim.

## Open questions

- **OQ-1 — flag spelling: `manual_only` or `no_auto_claim`?** **Default in force:**
  `manual_only`, matching #9178 §4's working name. Settled by: the schema
  implementation for #9178 §4; the other spelling becomes an alias only if an
  implementation has already shipped with it.
