# FDR-0005 — Fleet init/run identity, GitHub carriage, and a backed-up transaction log

- **Date:** 2026-11-17
- **Status:** proposed
- **Owner:** Fleet queue maintainers (index owner of record until an implementation owner is named)
- **Decision maker:** none yet — this is Drew's recorded suggestion, explicitly **not adopted by filing** [#9178 §5](https://github.com/developing-today/code/issues/9178)
- **Scope:** whether fleet work carries a minted run identity into GitHub operations, and whether a complete transaction log is required, backed up, and ingestible by Atlas
- **Supersedes:** none
- **Superseded by:** none
- **Related records:** FDR-0010 (completion evidence), FDR-0006 (park/stop attribution), FDR-0002 (override audit)
- **Implementation:** [#9178 §5](https://github.com/developing-today/code/issues/9178) (decision checklist); consumers [#9016](https://github.com/developing-today/code/issues/9016), [#9075](https://github.com/developing-today/code/issues/9075), [#9073](https://github.com/developing-today/code/issues/9073), [#9221](https://github.com/developing-today/code/issues/9221)

## Context and problem

The automation that labeled [#9155](https://github.com/developing-today/code/issues/9155)
`status:done` and closed it **during an active bound Summit run** has not been
identified from the GitHub surface alone. Every fleet worker posts as the same
account, so authorship keys nothing; the issue timeline records the event but not
the run, task, or session behind it. The same gap makes "who stopped, merged,
closed, or edited this?" an investigation rather than a lookup.

Drew's proposal ([#9178 §5](https://github.com/developing-today/code/issues/9178),
verbatim in that issue): require a fleet init step that mints a unique run/task ID
and stamps everything discoverable — or ties that ID to a task or cron job —
carry the ID into GitHub operations so comments, edits, closes, and merges are
attributable to the run that performed them, and log every fleet transaction to a
durable, backed-up log that can be injected into Atlas.

**This record does not adopt the proposal.** #9178 §5 states that appearing there
authorizes nothing, and requires the decision to end in an explicit record. This
record exists to make the open decision durable, owned, and falsifiable — not to
pre-empt it.

## Source links

- Originating issue: [#9178 §5](https://github.com/developing-today/code/issues/9178)
- Suggestion: Drew, recorded 2026-11-12 (not a ruling; explicitly "proposal to decide")
- Motivating incident: [#9155](https://github.com/developing-today/code/issues/9155) closure by an unidentified automation
- Prior defect pattern: [#7610](https://github.com/developing-today/code/issues/7610) (bot closes Refs-only issues on PR merge)
- Companion: [#9221](https://github.com/developing-today/code/issues/9221) (closure records + reason-required closes), which builds on this record's transaction log
- Existing pieces to compose with: [#9075](https://github.com/developing-today/code/issues/9075) (session id in env + worktree pwd-file), [#9073](https://github.com/developing-today/code/issues/9073) (create-path provenance stamping), [#9016](https://github.com/developing-today/code/issues/9016) (fleet gh verbs)

## Options considered

| Option                             | What it means                                                                                                                                                                            | Assessment                                                                                                                                                                              |
| ---------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A. Full adoption as proposed       | Init mints a run ID; every fleet-mediated mutation carries it; all transactions to a durable backed-up log                                                                               | Strongest attribution. Cost: an init contract every caller must satisfy, a carriage encoding on every GitHub write, and a log pipeline with redaction, rotation, and backup obligations |
| B. Narrowed adoption               | Adopt run identity + carriage for **mutations only** (comments, edits, closes, merges), leave reads out; transaction log as an append-only local file that joins the existing backup set | Meets the #9155 need (mutations) at lower cost; the read path is not implicated in the incident                                                                                         |
| C. Adopt logging only              | Record transactions as they already occur; no init contract or carriage                                                                                                                  | Gets the audit trail but not attribution: the log would still need to attribute to _something_, and without a run ID it attributes to a session only when one exists                    |
| D. Reject                          | Keep the current state; attribute by investigation                                                                                                                                       | Cheapest. Leaves the #9155 question unanswerable from the fleet side and leaves #9221 without a record to append to                                                                     |
| E. Do nothing (as distinct from D) | Leave the decision open indefinitely                                                                                                                                                     | Fails #9178 §5's own acceptance criterion ("ends in an explicit decision record") — a proposal that never resolves is a silent abandonment                                              |

**Tradeoff comparison:** A and B differ mainly in scope of carriage and in how
much of the read path must satisfy an init contract; B is the smaller change that
still answers the incident. C is not independently sufficient for the motivating
question. D and E are recorded because they were real: E is what has effectively
been happening since 2026-11-12.

## What a settled decision must specify

The record that replaces this one must answer all five of #9178 §5's questions,
each with a chosen default and the evidence for it:

1. **Scope.** All fleet-mediated queue/GitHub mutations; all GitHub operations
   including reads; interactive human use as well as agents/cron — or a phased
   scope. Which bypasses remain allowed, and how they are detected.
2. **Init contract.** What `fleet init` mints and stamps (run id, task id,
   cron/job id, session id, host, worktree, repo, actor/route, start time); what is
   inherited from env, the #9075 pwd-file, flags, and task claims; and what happens
   when init is missing — fail closed for mutations, warn-and-log, or read-only
   mode. Whether an audited emergency bypass exists.
3. **GitHub carriage.** How the run id appears on issue/PR comments, edits,
   closes, and merges — pm-metadata, a hidden marker, a database mapping, or a
   combination — without noisy duplication on every operation.
4. **Transaction log.** Append-only local file, queue/DB table, or both; required
   fields; secret-redaction rules; rotation/retention; backup target and frequency;
   failure behavior when logging fails; the Atlas ingestion path and schema.
5. **Adoption boundary.** If adopted, separate implementation issues for
   init/identity, GitHub carriage, and logging/backup/Atlas ingestion. If rejected
   or narrowed, the rationale and the alternative attribution mechanism.

## Rationale and evidence

**Measured / sourced facts:** the #9155 close is not attributable from the GitHub
surface — every fleet writer shares one account, and the timeline records the
event without a run identity. #7610 is a recorded prior instance of an automated
close of work that should have stayed open. Existing provenance work already
stamps session identity (#9075) and create-path metadata (#9073), so part of the
stamping substrate exists.

**Inference:** a run identity carried into mutations is the only mechanism that
turns the #9155 question into a lookup; a log without identity still cannot answer
"which run closed this". This is inference, not a measurement: no prototype of the
carriage has been run, and the cost of the init contract is not estimated here.

## Consequences (if adopted)

- **Behavior changes:** fleet mutations would require an initialized run (subject
  to the chosen missing-init behavior); GitHub writes gain a carriage marker.
- **Compatibility / migration:** existing callers without init need the chosen
  fallback path; `fleet gh` verbs gain a required-or-inherited identity input.
- **New failure modes:** a missing or stale run id blocking legitimate work;
  carriage noise on issues; log growth and a redaction failure leaking secrets.
  Each is the reason #9178 §5 requires explicit answers, not defaults.
- **Monitoring and tests required:** init presence/absence behavior; carriage
  round-trip (write then read back the run id from the artifact); log append
  durability and redaction; backup inclusion; Atlas ingestion of a sample.
- **If rejected or narrowed:** the record must name the alternative attribution
  mechanism (for example a queue-side only mapping from GitHub artifact to task),
  or #9221 has nothing to append closure records to and must be rescoped.

## Revisit triggers

- Every closure or merge that cannot be attributed after the fact — the #9155
  shape recurring. Each occurrence strengthens adoption and should be cited here.
- #9221 needing a transaction record to exist before it can ship: that makes this
  decision a blocker rather than an open item.
- #9075 / #9073 landing a run or session identity that makes carriage nearly free.
- A measured cost estimate showing the init contract would break existing callers
  at a scale that argues for narrowing rather than adopting.

## Open questions

- **OQ-1 — scope: mutations only, or all operations including reads?**
  **Default in force:** mutations only (option B's scope) — reads are not
  implicated in the #9155 incident and are the expensive half to instrument.
  Settled by: the decision issue listed below.
- **OQ-2 — missing-init behavior: fail closed, warn-and-log, or read-only?**
  **Default in force:** warn-and-log for the first release, because a fail-closed
  default would break every existing unattended caller on the day it lands.
  Settled by: the same decision issue, with the measured caller inventory.
- **OQ-3 — who settles this and by when?** **Default in force:** the fleet queue
  maintainers, in a decision issue answering §5's five questions; until then the
  proposal is not adopted and no fleet-init implementation is authorized by
  [#9178](https://github.com/developing-today/code/issues/9178). Settled by: that
  issue being filed and ruled on.
