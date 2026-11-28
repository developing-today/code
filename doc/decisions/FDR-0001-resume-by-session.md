# FDR-0001 — Resume a queue task in a named session

- **Date:** 2026-11-17
- **Status:** accepted
- **Owner:** Fleet queue maintainers
- **Decision maker:** Drew — directive of 2026-11-12 20:41 CST, specified in the companion specification
- **Scope:** `resume_session_id` on the tasks table; claim-time session continuation, its fallback, and the worktree-mismatch rule
- **Supersedes:** none
- **Superseded by:** none
- **Related records:** FDR-0002, FDR-0003, FDR-0004, FDR-0005
- **Implementation:** [#9178 §1](https://github.com/developing-today/code/issues/9178); builds on [#9075](https://github.com/developing-today/code/issues/9075) (session id in env + worktree pwd-file) and [#9073](https://github.com/developing-today/code/issues/9073) (create-path provenance stamping)

## Context and problem

A queue task could not name the session that should resume it. When epic #9155's
bound Summit session was stopped and its task parked, the eventual resume had to
create a fresh OpenCode session and rebuild the design context from artifacts;
roughly 600K tokens of shared context from the stopped run were unrecoverable
through the queue. The queue's only resume story was "claim it again and hope the
new session can reconstruct the state".

Sources: [#9178 §1](https://github.com/developing-today/code/issues/9178);
incident [#9155](https://github.com/developing-today/code/issues/9155) (queue task
15345); [#9075](https://github.com/developing-today/code/issues/9075).

## Source links

- Originating issue: [#9178 §1](https://github.com/developing-today/code/issues/9178)
- Directive: Drew, 2026-11-12 20:41 CST ("The queue needs first-class support for four related operations", item 1)
- Incident: [#9155](https://github.com/developing-today/code/issues/9155), queue task 15345
- Foundations: [#9075](https://github.com/developing-today/code/issues/9075), [#9073](https://github.com/developing-today/code/issues/9073)
- Data model: [`doc/2026-11-10_design_queue_task_data_model_3424.md`](../2026-11-10_design_queue_task_data_model_3424.md)

## Options considered

| Option                                                  | What it means                                                                    | Assessment                                                                                                   |
| ------------------------------------------------------- | -------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| A. `resume_session_id` column + claim-time continuation | Task names a session; claim continues that session by ID instead of creating one | Chosen. Durable in the queue, survives restarts, composes with ownership rules                               |
| B. Reconstruct from artifacts on each resume            | What was actually done during the #9155 incident                                 | Measured to cost ~600K tokens of context and produce a materially different session; not a resume            |
| C. Pin the session in the task payload text             | No schema change                                                                 | Free-text pinning is unqueryable and unvalidated; the launcher cannot enforce it and nothing is audit-logged |
| D. Do nothing                                           | Resume stays an ad-hoc operator action                                           | Leaves the measured failure in place; every stopped long task costs a re-derivation                          |

## Decision

- Add a nullable `resume_session_id` to the fleet-queue `tasks` table.
- When a worker claims a task that carries `resume_session_id`, the launcher
  **continues that OpenCode session by ID** instead of creating a new session.
- Record **both** the requested session and the session actually used on the
  claim/attempt record. A silent substitution is prohibited: every fallback is
  recorded with its reason and surfaced in the claim result.
- Only an authorized actor may set, replace, or clear the value: the task
  creator, the currently bound worker/session, or an explicit operator action.
  Every change is audit-logged with actor, prior value, new value, time, and
  reason.
- The resume hint does not override ownership. If the task is already claimed,
  FDR-0003's skip/take-over rules apply.
- **Missing session:** fall back to a fresh session in the task's worktree, record
  `resume_fallback=missing_session` plus the new session id, and surface the
  substitution. Never retry a missing session forever.
- **Wrong worktree:** never execute in the wrong tree (see OQ-1 for the chosen rule).
- **Busy session owned by another active task:** do not steal it. The claim is
  skipped unless take-over (FDR-0003) is explicitly requested.
- **Host restart:** the id survives in the queue; existence is checked against
  the target server on claim, and the missing-session fallback applies.
- **No `resume_session_id`:** behavior is unchanged from today.

## Rationale and evidence

**Measured / sourced facts:** the #9178 write-up records that the #9155 resume
required a fresh session and that ~600K tokens of shared context were not
recoverable through the queue; the claim could not name a session because no such
field existed. The tasks table already carries per-claim/attempt records and
audit fields (#9075, #9073), so the requested-vs-actual distinction has a place
to live.

**Inference:** a durable column is the only mechanism that survives a host
restart and is enforceable by the launcher without operator memory. Recording the
actual session on the attempt is what makes a fallback auditable rather than
silent; without it a substitution is indistinguishable from a successful resume.

## Consequences

- **Behavior changes:** a claim may continue an existing session; claim results
  grow a "requested vs used" distinction and fallback reasons.
- **Compatibility / migration:** additive nullable column; tasks without it follow
  the existing path unchanged. No backfill required.
- **New failure modes:** resuming a session whose worktree or context no longer
  matches; a session that exists but is wedged (covered by the busy-session rule
  and the attempt/claim records, not by this record).
- **Monitoring and tests required:** tests for the live-match case (evidence the
  same session id receives the task prompt), the missing-session fallback, the
  wrong-worktree case, and the busy-session case, each asserting the recorded
  outcome. A task without the column must follow the unchanged path.

## Revisit triggers

- Measured resume failures where the recorded session existed but was not
  resumable in practice — that argues for a pre-claim health probe.
- Cross-worktree sessions becoming common enough that rebind (OQ-1 option a)
  is the better default.
- #9075's session-state contract changing what "the same session" means.

## Open questions

- **OQ-1 — wrong-worktree rule: rebind or fall back?** #9178 §1 leaves two
  permissible rules: (a) an explicit rebind action recording old worktree, new
  worktree, actor, and reason; or (b) fall back to a fresh session in the task's
  worktree and record the mismatch. **Default in force: (b)** — fall back into the
  task's worktree and record `resume_fallback=worktree_mismatch` with both
  worktrees. Reason: it needs no new authorization surface, cannot execute in the
  wrong tree, and the mismatch stays visible in the claim result, which is the
  property the rule exists to protect. Settled by: the #9178 §1 implementation
  issue, which must pick one rule, implement it, and test it — amending this
  record rather than adding a new ID.
