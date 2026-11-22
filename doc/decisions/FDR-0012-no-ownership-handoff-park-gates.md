# FDR-0012 — No ownership-handoff park gates

- **Date:** 2026-11-21
- **Status:** accepted
- **Owner:** Fleet queue maintainers
- **Decision maker:** Drew, 2026-11-20 ("my being assigned means nothing yall
  share my username") and 2026-11-21 ("yes handle it all"), in the thread that
  tracked agent lanes parking issues assigned to him.
- **Scope:** which ownership conditions may hold, park, or gate queue work, and
  what happens to checkpoints that carry an ownership-handoff gate.
- **Supersedes:** FDR-0011, in part: only its retained "handoff semantics" and its
  open-question default to leave legacy ownership holds unchanged. FDR-0011's
  implementation-target and work-window decision stands.
- **Superseded by:** none
- **Related records:** FDR-0002, FDR-0006, FDR-0008, FDR-0011
- **Implementation:** [#12061](https://github.com/developing-today/code/pull/12061)
  (lane guard, sweeper removed), [#12074](https://github.com/developing-today/code/pull/12074)
  (wayfinder claims by label), [#12155](https://github.com/developing-today/code/pull/12155)
  (this rule in `AGENTS.md` and
  [`doc/agents/one-queue-claiming-work.md`](../agents/one-queue-claiming-work.md));
  lane prompts and seven queue checkpoints corrected on the fleet laptop, 2026-11-21.

## Context and problem

Every fleet agent acts through one shared GitHub account, `dezren39`, which is
also Drew's username. Lanes read "assigned to dezren39", "opened by dezren39",
or an earlier run's branch or draft PR as someone else owning the work, then
parked the task "awaiting ownership handoff", "owner release" or
"reassignment". Nobody ever gives that handoff, so such a park never ends.
By 2026-11-20 1,068 open issues and 13 open PRs carried the assignment, and
lanes kept parking on it. On 2026-11-21, after every assignment was cleared,
seven parked queue tasks still carried the gate in their checkpoint `next` and
`resume_condition`, so a resumed lane would have parked them again.

FDR-0011 records this same pattern as fact (tasks 20725/#7587 and 20567/#10345
"require owner handoff") and, as its open-question default, leaves legacy
ownership holds unchanged. That default keeps the dead gate alive.

## Source links

- Originating issues/PRs: #12061, #12074, #12155.
- Queue task IDs: the seven tasks requeued on 2026-11-21 (their checkpoints
  record `stage: ownership_gate_cleared`).
- Dated ruling: Drew, 2026-11-20 and 2026-11-21, quoted above.
- Evidence: zero open issues or PRs assigned to `dezren39` after the 2026-11-21
  sweep; laptop prompt and queue-store backups suffixed `bak-20261121-handoff`.

## Options considered

| Option                             | What it means                                                                     | Assessment                                                                  |
| ---------------------------------- | --------------------------------------------------------------------------------- | --------------------------------------------------------------------------- |
| A. Keep FDR-0011's default         | Leave ownership-gated checkpoints as they are until a producer verifies them      | Rejected: no producer can verify a handoff that nobody gives; parks forever |
| B. Clear assignments only          | Unassign the account, leave checkpoints                                           | Rejected: checkpoints re-park resumed work on the same gate                 |
| C. No ownership-handoff park gates | Only a live claim or a hold label holds work; handoff-gated checkpoints are reset | Chosen                                                                      |

## Decision

1. Only these hold work: a live queue claim, or the labels `held`, `user/owned`
   or `do-not-auto-edit` (`do-not-merge` blocks merging only). Assignment,
   authorship, commits or a `work/assigned` marker by the shared account are
   not holds.
2. There is no ownership handoff. A lane never parks "awaiting ownership
   handoff", "owner release", "owner coordination" or "reassignment", and never
   writes such a gate into a resume brief, `next`, or `resume_condition`.
3. A checkpoint whose only remaining gate is one of those is invalid. Any lane
   or maintainer may reset its `next` and `resume_condition` to "workable now"
   and requeue the task, recording `stage: ownership_gate_cleared` in progress.
   A checkpoint that also names a real gate (a hold label, a live claim, a
   design or capacity gate) keeps that gate.
4. A queue task's own earlier branch, worktree or draft PR is its own work to
   inspect and continue, not another owner's.
5. FDR-0008 still applies: a park needs a purposeful term and a release path,
   and an ownership handoff is neither.

## Rationale and evidence

**Measured / sourced facts:** the shared account is the only GitHub identity
agents use; assignment and authorship by it carry no information about who holds
the work. The 2026-11-20 sweep and 2026-11-21 checkpoint audit are recorded in
#12155.

**Inference:** a gate whose release depends on an act nobody performs fails
FDR-0008's release-path requirement and FDR-0009's reboot test by construction.

## Consequences

- Behavior changes: lanes stop parking on assignment or authorship; seven
  previously gated tasks are workable.
- Compatibility / migration: done once on 2026-11-21 for the live queue; any
  later checkpoint carrying the gate is reset under rule 3.
- New failure modes: two lanes could pick up the same unclaimed task's earlier
  branch; the queue claim, not ownership, is what prevents that.
- Monitoring and tests required: `pm-stack/lane-guard` tests (from #12061)
  cover assignment not holding work.

## Revisit triggers

Agents get distinct GitHub identities, or a real person other than Drew starts
taking fleet work through the shared account.

## Open questions

- **OQ-1 — Should the broker reject a park reason naming an ownership handoff?**
  Default: prompts and this record forbid it; no broker validation yet. Settled
  by: a park-reason validation change in `pkgs/fleet`.
