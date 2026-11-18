# FDR-0011: Implementation target and usable work window

- Date: 2026-11-18
- Status: accepted
- Owner: Fleet queue maintainers
- Decision maker: Drew, explicit request to filter Fleet claims by actual implementation repository and suitable work window, 2026-11-18.

## Context and sources

Task 20592 tracks [code#10415](https://github.com/developing-today/code/issues/10415)
but explicitly implements a workflow in developing-today/kaneo. Its packet says
`repo=developing-today/code` and `risk.flags=[cross-repo-change]`; code-only workers
repeatedly claim and park it. Tasks 20725/#7587 and 20567/#10345 have 45-minute
timeboxes but no structured required budget, and their checkpoints also require
owner handoff. Those facts do not establish a numeric effort estimate or release.
Existing #3418 already implements required/remaining-budget matching.

## Options and decision

Keep the status quo, infer targets/durations from prose, or introduce explicit
implementation metadata and reuse the existing budget contract. Choose the last:
optional validated `payload.implementation_repo` (owner/name) takes precedence in
claim repo filters. Tracking repo, issue references, and work dedupe are unchanged.
Ordinary legacy packets retain their prior matching. Explicitly cross-repo packets
without a target fail closed at the claim payload guard with a repair diagnostic.

Bounded callers declare their current usable remaining window on each claim via
`--remaining-budget-minutes`; packets use `required_budget_minutes` for the minimum
uninterrupted checkpoint window. Retain #3418's gate, capacity timer bypass, and
manual/handoff semantics. Do not repurpose timebox or infer hours from effort/l.

## Consequences and verification

SQL and pure filter tests must agree for tracking-code/implementation-Kaneo, legacy
fallback, and negated filters. Validation rejects malformed targets. Synthetic SQL
checks exercise target selection together with the existing budget gate, including
exact boundary, too-short, absent budget, and requirement-free legacy work.

This code does not deploy the broker, change scheduled prompts, or repair stored
packets. Current broker has no supported payload edit verb; an enqueue dedupe hit
is not a repair. Explicit cross-repo packets may become quarantined until repaired.
Reconcile those diagnostics and provide a guarded packet-repair path; never edit
live task SQL or bypass ownership to make a demonstration pass.

## Revisit

Revisit if work requires multiple implementation repositories or measured metadata
shows a single target insufficient. Do not treat a tracking repo or prose guess as
proof. No prior accepted FDR is superseded; this refines #8097's repo dimension and
uses #3418's budget contract. FDR-0002 manual overrides remain unchanged.

## Open questions

Legacy capacity and ownership records need producer-verified metadata. Default:
leave their holds unchanged; no guessed duration, inferred release, or deployment.
