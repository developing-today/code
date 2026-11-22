# FDR-0006 — Park remediation ladder for bound work

- **Date:** 2026-11-17
- **Status:** accepted
- **Owner:** Fleet queue maintainers / supervision owners
- **Decision maker:** Drew — ruling arising from the #9155 stop, 2026-11-12
- **Scope:** what a supervisor does, in order, when bound work must move out of where it is running, before parking is considered
- **Supersedes:** none
- **Superseded by:** none
- **Related records:** FDR-0008 (purposeful parks), FDR-0009 (reboot test), FDR-0002 (manual claim)
- **Implementation:** supervision and park paths in the fleet queue; root-cause input from [#9177](https://github.com/developing-today/code/issues/9177); the #9155 incident record

## Context and problem

On 2026-11-12, epic [#9155](https://github.com/developing-today/code/issues/9155)'s
bound Summit session was working in a location that supervision considered
non-compliant. The response was to **stop the session and park the task**. The
work stopped, the parked task offered no owner-release path, and the resume cost
~600K tokens of unrecoverable context in a fresh session (see FDR-0001). Two
distinct harms resulted:

1. **Sole-copy artifacts were at risk.** A stopped session's in-flight work exists
   in that session and its worktree; stopping before preserving or relocating it
   can strand the only copy.
2. **Park was used as the first tool, not the last.** Parking is a hold state, not
   a remediation: it records that work is waiting, not that the condition was
   fixed.

Sources: [#9155](https://github.com/developing-today/code/issues/9155) (queue task
15345); the ruling is recorded as a seed in
[#9180](https://github.com/developing-today/code/issues/9180);
[#9177](https://github.com/developing-today/code/issues/9177) (the unidentified
App-identity writer and the completion guard).

## Source links

- Ruling: Drew, 2026-11-12, arising from the [#9155](https://github.com/developing-today/code/issues/9155) stop; recorded in [#9180](https://github.com/developing-today/code/issues/9180)
- Incident: [#9155](https://github.com/developing-today/code/issues/9155), queue task 15345
- Investigation: [#9177](https://github.com/developing-today/code/issues/9177)
- Related: [`doc/agents/parallel-writers-and-subsessions.md`](../agents/parallel-writers-and-subsessions.md) (worktrees, one writer per file), FDR-0001 (resume), FDR-0008 (parks need a term and a release)

## Options considered

| Option                                                       | What it means                                                               | Assessment                                                                                                                        |
| ------------------------------------------------------------ | --------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| A. Ordered ladder: steer → stop+preserve+restart → park last | Try the least destructive remedy first; park only when nothing else applies | Chosen. Preserves work, keeps the task live, and reserves park for a real hold                                                    |
| B. Stop first, then decide                                   | What happened on #9155                                                      | Measured to cost the session's context and risk sole-copy artifacts; the remedy arrived after the harm                            |
| C. Park and let a human sort it out                          | Park immediately on any non-compliance                                      | Makes the queue the parking lot and the human the remediation engine; FDR-0008 records why an unreleasable park is not acceptable |
| D. Do nothing                                                | Let the session continue in the wrong place                                 | Non-compliance is a real condition; ignoring it is not a policy                                                                   |

## Decision

When bound work must leave where it is running, apply this ladder **in order** and
record which rung was used:

1. **Steer.** Signal the bound session to relocate itself — its own worktree, its
   own files, its own context preserved. This is preferred because the session
   holds the only complete copy of its in-flight state.
2. **Stop, preserve, and restart in the compliant location.** If steering fails or
   the session cannot act, stop it only after preserving or moving the work
   (commits pushed, artifacts written and linked) and restart it in the compliant
   location. Record what was preserved and where.
3. **Park — last resort only.** Park only when the work cannot be relocated at
   all, and **never strand sole-copy artifacts**: if the only copy of the work
   lives in the session being stopped, preserving it is a precondition of the stop,
   not a follow-up.

A park used at any rung carries FDR-0008's requirements: a purposeful term and an
owner-release path. "Non-compliant location" alone is not a park reason while
rungs 1 and 2 remain available.

## Rationale and evidence

**Measured / sourced facts:** the #9155 stop produced a parked task with no
owner-release verb, and the eventual resume had to rebuild context in a fresh
session at a measured cost of roughly 600K tokens of shared context. The ruling
itself is Drew's, dated 2026-11-12, and is recorded as a seed in #9180.

**Inference:** steering is cheaper than restarting because the session's context
is not reproducible from artifacts — the #9155 measurement is the evidence. The
"never strand sole-copy artifacts" clause is a precondition rather than a
consequence because the harm is irreversible at the moment of the stop.

## Consequences

- **Behavior changes:** supervision gains an ordered remediation path and must
  record the rung used; a bare "stop and park" on a relocation condition becomes a
  policy violation.
- **Compatibility / migration:** no schema change; the rung and preserved-artifact
  references belong on the existing park/stop record (see the #3424 status-reason
  and event-history model).
- **New failure modes:** steering can silently fail (the session ignores the
  signal) and look like success — the rung record must state observed outcome. A
  stop that cannot preserve work must be escalated rather than performed.
- **Monitoring and tests required:** a test that a relocation condition produces a
  steer attempt before a stop; a test that a stop without a preservation record is
  refused or flagged; a test that a park whose artifacts were sole-copy is not
  recorded as clean.

## Revisit triggers

- [#9177](https://github.com/developing-today/code/issues/9177)'s root-cause
  findings landing — this record should incorporate them (the #9180 seed says so).
- A rung-1 steer failing repeatedly in a way that indicates the signal mechanism,
  not the session, is at fault.
- Bound sessions gaining durable, resumable state (#9075) such that a restart is
  materially cheaper — that would weaken, though not remove, the steer-first rule.

## Open questions

- **OQ-1 — what counts as "preserved"?** **Default in force:** the work is
  recoverable by someone other than the stopped session — pushed commits, or
  artifacts written to a path and linked from the task/issue. Settled by: the
  #9177 root-cause findings, which should define the concrete check.
