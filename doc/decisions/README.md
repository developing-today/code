# Fleet decision records (FDR) — convention, template, and index

This directory is the canonical home for **fleet decision records**: the durable
record of rulings about fleet queue semantics, supervision and stop authority,
cap/limit policy, and attribution/logging. A ruling that exists only in a chat
message, a job prompt, a queue row, or an issue comment is **unrecorded**, not an
accepted fleet decision ([#9180](https://github.com/developing-today/code/issues/9180)).

Maintained document: this file changes in place as records are added, accepted,
superseded, or deprecated. Individual record files keep their identity and are
never rewritten; a changed decision gets a new record that supersedes the old one.

## 1. Location and naming

- Records live in `doc/decisions/`. The index is this `README.md`.
- File name: `FDR-<NNNN>-<kebab-slug>.md` — for example `FDR-0001-resume-by-session.md`.
- An ID is allocated **once**, in the index table below, and is never reused,
  never renumbered, and never recycled after a record is superseded or deprecated.
- A superseded record keeps its file and ID, gets `Status: superseded`, and its
  `Superseded by:` field points at the replacement.
- Historical timestamped decision files elsewhere under `doc/` are **not** moved,
  renamed, or rewritten for tidiness. Where they still govern current fleet
  behavior they are linked from [§7 Related records](#7-related-records-and-historical-decisions).

### ID scheme — resolved scheme, evidence, and the alternative

**Resolved:** the unprefixed namespace `FDR-NNNN` is the **fleet-wide canonical**
namespace and is allocated only by the index table in this file. Package-scoped
record sets use a namespace prefix owned by that package (currently
`FDR-PMO-NNN`, the PM-stack / fleet-orchestration design package) and are linked
from the index, never renumbered into it.

Evidence for this resolution:

1. #9180's seed table allocates `FDR-0001` … `FDR-0010` row by row, as one
   unprefixed series with one index at a single canonical location. The issue's
   own acceptance criteria gate #9178's completion on those IDs.
2. The PM-stack package index
   ([`../2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/decisions/README.md`](../2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/decisions/README.md))
   states that `FDR-PMO-NNN` "is proposed for these package records, pending
   reconciliation with the canonical #9180 index. It does not allocate or reserve
   fleet-wide IDs. The existing `FDR-0001`–`FDR-0010` seeds in #9180 are a
   distinct namespace and are not reused here." The package therefore already
   treats the unprefixed range as #9180's, and prefixes its own records.
3. The #9178 triage caution forbids ratifying either scheme unilaterally. This
   record does not mint a competing namespace: it adopts the allocation #9180
   already made and leaves the remaining choice as a single documented decision
   point (open question **OQ-1** below).

**Alternative considered:** an all-prefixed scheme (`FDR-FLEET-*`), where no
unprefixed ID is ever allocated and every package keeps a prefix. Recorded as
OQ-1 rather than silently rejected.

**OQ-1 — open question, with default.** _Should fleet-wide records be prefixed
(`FDR-FLEET-NNNN`) instead of unprefixed (`FDR-NNNN`)?_
**Default chosen now:** unprefixed `FDR-NNNN`, as resolved above. The decision
point is cheap to flip only while no ID outside `FDR-0001…0010` has been
allocated; an owner flipping it must add a one-table mapping and keep
`FDR-0001…0010` as permanent aliases, because other documents already cite them.
Whoever settles OQ-1 records the answer here and in the record the change
supersedes. Nothing else in this directory depends on the answer.

## 2. Status lifecycle

| Status       | Meaning                                                                                                   |
| ------------ | --------------------------------------------------------------------------------------------------------- |
| `proposed`   | Written down with a named owner and a settling event; **not** yet in force as an accepted fleet decision. |
| `accepted`   | In force. The decision maker and the dated ruling are cited in the record.                                |
| `rejected`   | Considered and refused. The record stays so the option is not silently re-proposed; it cites why.         |
| `superseded` | Replaced by a later record. Points to its replacement; kept for history.                                  |
| `deprecated` | No longer applicable (the subsystem is gone or the condition vanished). Kept for history.                 |

A chat ruling not yet written as a record is **unrecorded**. A `proposed` record
is never cited as authority for behavior that contradicts it. Stale `proposed`
records get an explicit disposition at the fleet owners' periodic design-register
review ([§6](#6-maintenance-and-review)) — being left `proposed` indefinitely is
a defect, not a state.

## 3. Required contents of every record

Every record states, in this order:

- [ ] Decision ID, title, date, status, owner, and decision maker.
- [ ] Context and problem statement, including the incident or evidence that forced the decision.
- [ ] Source links: originating issue(s), PRs/commits, queue task IDs where relevant, and the dated ruling or other authority.
- [ ] Options considered, including doing nothing where that was a real option.
- [ ] Decision, stated operationally enough that an implementer or automation author can follow it without reading the whole history.
- [ ] Rationale and evidence, separating measured facts from inference.
- [ ] Consequences: behavior changes, compatibility/migration effects, new failure modes, and monitoring or tests required.
- [ ] Reversal or revisit triggers: what future evidence or condition should reopen the decision.
- [ ] `Supersedes` / `Superseded by` links and links to implementation/research issues.
- [ ] Open questions, if any, each with the default currently in effect.

A seed item in #9180 is not complete merely because it appears in that issue, and
a row in the index below is not complete merely because it exists: it must link to
a full record or to an **owned** `proposed` record naming the event that settles it.

## 4. Template

Copy [`TEMPLATE.md`](TEMPLATE.md) to `FDR-<NNNN>-<slug>.md`, fill it, add the index
row, all in one commit. The template reproduces the checklist in §3.

## 5. Same body of work

**A change to claim, park, resume, stop-authority, cap, or attribution semantics
requires a decision record in the same body of work** — the same PR, or the same
run for non-PR work.

Concretely, a PR, issue, migration, or automation change that alters any of:

- queue claim, park, resume, take-over, or manual-only semantics;
- who may stop, close, park, or complete fleet work (supervision/stop authority);
- an artificial count cap, page limit, batch limit, or attempt cap;
- attribution, run identity, transaction logging, or closure recording;

must add a new record or update an existing one (a status change, a consequence,
or a new open question) **in that same body of work**. If the change contradicts
an `accepted` record, it must supersede that record rather than quietly diverge
from it. A reviewer may block a semantics change whose record is missing.

The rule is also stated where fleet queue and runbook readers will meet it:
[`pkgs/fleet/README.md`](../../pkgs/fleet/README.md#operator-reference) and
[`doc/agents/one-queue-claiming-work.md`](../agents/one-queue-claiming-work.md).

## 6. Maintenance and review

- The fleet owners own this index; any contributor may open a record.
- The index is checked **when a related implementation issue is opened** and
  during the fleet owners' **periodic design-register review**.
- That review checklist includes a **decision-index pass**: every `proposed`
  record is accepted, rejected, superseded, or explicitly re-deferred with a new
  settling event. Nothing stays `proposed` by inattention.
- Records are dated in UTC; `Date` is the date the record was written, and the
  decision's own ruling date is cited in the record body.

## 7. Related records and historical decisions

These predate this convention and still govern current fleet behavior. They are
linked, not moved or renumbered; where one conflicts with a record here, the
record here wins only if it says so and supersedes it explicitly.

- [Queue core on Turso — cutover runbook (#6741)](../fleet/queue-turso-cutover.md) — the production queue backend cutover; governs the tasks table the claim/park semantics act on.
- [Queue fallback mode — GitHub-native claiming](../fleet/queue-fallback-mode.md) — the backend-down claim protocol, lease length, and marker shapes.
- [agent.events — Turso sink and cutover (#5690)](../fleet/agent-events-turso.md) — the append-only event stream behind actor provenance and audit.
- [Durable repo-queue task data model (#3424)](../2026-11-10_design_queue_task_data_model_3424.md) — blocker links, park revalidation, status reasons, event history, and actor provenance; the data model these records assume.
- [Wipe-test audit of the fleet-queue schema (#8579)](../2026-11-11_design_id8579_queue_schema_wipe_test_audit.md) — the schema of record for the tasks table.
- [PM-stack / fleet-orchestration package decision index (`FDR-PMO-001…008`)](../2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/decisions/README.md) — package-scoped design records; accepts statuses are local to that design package, and the package defers fleet-wide naming to this index (see [§1](#id-scheme--resolved-scheme-evidence-and-the-alternative)).
- [Design register](../../pm-stack/design-register-2026-11-11.md) and [decisions ledger system](../../pm-stack/decisions-ledger-system.md) — the pending-questions ledger that feeds rulings recorded here.
- [Fleet schedule registry](../fleet-schedule-registry.md) — where scheduled surfaces are registered; a schedule that changes stop or attribution behavior also needs a record here.

## 8. Index

Statuses are as of 2026-11-17. `Source` is the ruling or specification that forced
the decision; `Implementation` is where the decision lands in code, docs, or issues.

| ID                                                         | Record                                                                    | Status     | Owner                   | Source                                                                                                                                                                                         | Implementation                                                                                                                                                                                                                                                                          |
| ---------------------------------------------------------- | ------------------------------------------------------------------------- | ---------- | ----------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [FDR-0001](FDR-0001-resume-by-session.md)                  | Resume-by-session semantics                                               | `accepted` | Fleet queue maintainers | [#9178 §1](https://github.com/developing-today/code/issues/9178) (Drew directive, 2026-11-12 20:41 CST); incident [#9155](https://github.com/developing-today/code/issues/9155)                | #9178 §1 (schema + claim path); builds on [#9075](https://github.com/developing-today/code/issues/9075), [#9073](https://github.com/developing-today/code/issues/9073)                                                                                                                  |
| [FDR-0002](FDR-0002-manual-claim-override.md)              | Manual claim as owner override for parked and attempt-capped tasks        | `accepted` | Fleet queue maintainers | [#9178 §2](https://github.com/developing-today/code/issues/9178) (Drew directive, 2026-11-12); incident [#9155](https://github.com/developing-today/code/issues/9155)                          | #9178 §2 (`fleet queue claim --task <id>`, override record); [#9016](https://github.com/developing-today/code/issues/9016) verb conventions                                                                                                                                             |
| [FDR-0003](FDR-0003-multi-task-claim-take-over.md)         | Multi-task claim under one session, and `--take-over` semantics           | `accepted` | Fleet queue maintainers | [#9178 §3](https://github.com/developing-today/code/issues/9178) (Drew directive, 2026-11-12)                                                                                                  | #9178 §3 (batch claim, per-task results, take-over transfer + audit)                                                                                                                                                                                                                    |
| [FDR-0004](FDR-0004-manual-only-queue-items.md)            | Manual-only queue items: first-class flag, not park or low priority       | `accepted` | Fleet queue maintainers | [#9178 §4](https://github.com/developing-today/code/issues/9178) (recommendation, Drew directive 2026-11-12)                                                                                   | #9178 §4 (`manual_only` column, claim-query exclusion, audited updates); [#8097](https://github.com/developing-today/code/issues/8097) lane model                                                                                                                                       |
| [FDR-0005](FDR-0005-fleet-run-identity-transaction-log.md) | Fleet init/run identity, GitHub carriage, and a backed-up transaction log | `proposed` | Fleet queue maintainers | [#9178 §5](https://github.com/developing-today/code/issues/9178) (proposal to decide, not adopted by filing); closure dependency [#9221](https://github.com/developing-today/code/issues/9221) | #9178 §5 decision checklist; [#9016](https://github.com/developing-today/code/issues/9016), [#9075](https://github.com/developing-today/code/issues/9075), [#9073](https://github.com/developing-today/code/issues/9073), [#9221](https://github.com/developing-today/code/issues/9221) |
| [FDR-0006](FDR-0006-park-remediation-ladder.md)            | Park remediation ladder for bound work                                    | `accepted` | Fleet queue maintainers | Drew ruling from [#9155](https://github.com/developing-today/code/issues/9155), 2026-11-12 (as recorded in #9180)                                                                              | Supervision/park paths; root-cause input from [#9177](https://github.com/developing-today/code/issues/9177)                                                                                                                                                                             |
| [FDR-0007](FDR-0007-justified-count-caps.md)               | Artificial count caps must state the reason they exist                    | `accepted` | Fleet queue maintainers | Drew rulings, 2026-11-12 (minder action caps, lane bundle caps, reconcile write/scan caps, collector page cap)                                                                                 | Merged example: [#9059](https://github.com/developing-today/code/pull/9059); reviewer check on every new cap                                                                                                                                                                            |
| [FDR-0008](FDR-0008-purposeful-parks-owner-release.md)     | Every park has a purposeful term and an owner-release path                | `accepted` | Fleet queue maintainers | Exposed by [#9155](https://github.com/developing-today/code/issues/9155); manual-claim side in [#9178 §2](https://github.com/developing-today/code/issues/9178)                                | #9178 §2 (claim-by-ID release); owner-unpark verb is a **named gap** — [#9180](https://github.com/developing-today/code/issues/9180)                                                                                                                                                    |
| [FDR-0009](FDR-0009-park-reboot-test.md)                   | A park that leaves its trigger unchanged fails the reboot test            | `proposed` | Fleet queue maintainers | Drew's analysis of the #9155 stop, 2026-11-12                                                                                                                                                  | [#9177](https://github.com/developing-today/code/issues/9177) stopper investigation; #9178 §2 boot behavior                                                                                                                                                                             |
| [FDR-0010](FDR-0010-completion-requires-evidence.md)       | Issue completion requires deliverable evidence from the bound work        | `proposed` | Fleet queue maintainers | Policy direction from the [#9155](https://github.com/developing-today/code/issues/9155) closure, 2026-11-12; prior pattern [#7610](https://github.com/developing-today/code/issues/7610)       | [#9221](https://github.com/developing-today/code/issues/9221) closure records + reason-required closes; [#9177](https://github.com/developing-today/code/issues/9177)                                                                                                                   |

| [FDR-0011](FDR-0011-implementation-target-and-work-window.md) | Implementation target and usable work window | `accepted` | Fleet queue maintainers | Drew request, 2026-11-18; code#10415/task 20592 | Claim repo filter + existing #3418 budget gate |
| [FDR-0012](FDR-0012-no-ownership-handoff-park-gates.md) | No ownership-handoff park gates (supersedes FDR-0011 in part) | `accepted` | Fleet queue maintainers | Drew rulings, 2026-11-20 and 2026-11-21 | [#12061](https://github.com/developing-today/code/pull/12061), [#12074](https://github.com/developing-today/code/pull/12074), [#12155](https://github.com/developing-today/code/pull/12155); queue checkpoints reset 2026-11-21 |

### Package-scoped records (not in this namespace)

| Namespace     | Records                                                                                              | Status                 | Owner                   | Notes                                                                                            |
| ------------- | ---------------------------------------------------------------------------------------------------- | ---------------------- | ----------------------- | ------------------------------------------------------------------------------------------------ |
| `FDR-PMO-NNN` | [`FDR-PMO-001…008`](../2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/decisions/README.md) | accepted (design-only) | PM-stack design package | Package-local; do not reuse these IDs fleet-wide, and do not renumber them into the table above. |

## 9. Settling events for the `proposed` records

| Record   | Owner                   | Event that settles it                                                                                                                                                                       | Default in force meanwhile                                                                    |
| -------- | ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| FDR-0005 | Fleet queue maintainers | A decision issue answering #9178 §5's five questions (scope, init contract, GitHub carriage, transaction log, adoption boundary), recorded here                                             | The proposal is **not** adopted; no fleet-init implementation is authorized by #9178          |
| FDR-0009 | Fleet queue maintainers | [#9177](https://github.com/developing-today/code/issues/9177) stopper root-cause findings land                                                                                              | A park whose trigger is unchanged is delay, not remediation                                   |
| FDR-0010 | Fleet queue maintainers | [#9177](https://github.com/developing-today/code/issues/9177) identifies the closing automation; [#9221](https://github.com/developing-today/code/issues/9221) lands reason-required closes | No automation may mark done/close work that has an active bound task and no completion record |
