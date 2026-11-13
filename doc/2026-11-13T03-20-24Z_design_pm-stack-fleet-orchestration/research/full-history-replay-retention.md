# JRN-04 — Full-history replay and bounded retention feasibility research

<!-- pmo-9196:JRN-04 -->

- **Issue:** #9466 (`design(pm-orchestration): Full-history replay and bounded retention feasibility research`)
- **Queue:** task 15991 (repo-health-code-pilot), lane code-d, 2026-11-13
- **Tier:** research only. This document is a feasibility record. It authorizes no code,
  schema, migration, or production change, creates no second event database, and claims
  no full event sourcing.
- **Gate state at research time:** PR #9225 MERGED 2026-11-13T13:36:36Z (verified fresh
  via `gh pr view 9225`: state MERGED, `mergedAt` set); this doc tree is on `main` at
  merge commit `84131d4b75`. Slice-internal blockers as written on #9466: none for
  read-only research. CEN-04 (#9434, the DEP-01 alias) is a coordination input, cited
  below where its open status matters; it is not treated as a gate for this research.
- **Oracle:** `pmo_history_replay_evidence_matrix_distinguishes_observation_from_authority` —
  every record source is classified as authoritative mutation, observation, telemetry,
  projection, or unknown (§3), and every replay claim carries primary-source support (§4).
- **Requirement trace:** H-R023 and P-R006 (allocated on #9466; design-expansion support
  only — explicitly not a literal full-event-store mandate and not a parallel DB). The
  source occurrences P-110…P-122 (`sources/03_full_orchestration_proposal.md` §2.5–2.6)
  require durable progress, recoverable ownership, idempotent mutations, a transactional
  outbox, optimistic concurrency, expiring leases, checkpoints and handoffs, incremental
  activity retrieval, and explicit repair — and §2.6 requires *not conflating different
  kinds of state*. Neither the P-occurrences nor H-083…H-087
  (`sources/01_handoff_pm_stack_fleet_orchestration.md`) mandate reconstructing all past
  state from an event stream; the full-history wording enters through #3424 piece 5,
  consumed here as a feasibility question, not as a mandate.

## 1. The question

#3424 piece 5 states the strongest form: "Full event history — resurrect any moment.
An append-only events table records every change to a task (status, priority, park,
claim, links, notes) with before/after values and the actor. Any past state is
reconstructable from the event stream." (`doc/2026-11-10_design_queue_task_data_model_3424.md:43-46`)

This research tests that statement against the records that actually exist today:
the live fleet-queue Turso backend (`pkgs/fleet/crates/fleet-queue/src/turso_queue.rs`,
the schema of record), its `events`, `result`, and `blocked_deps` surfaces named by the
slice, the #8579 wipe-test schema audit (PR #8602, merged 2026-11-11), the agent-events
coupling record (`doc/fleet/agent-events-turso.md`, #5690), the #9208 component design
and FDR-PMO-005, and the Track A / Track B prior-art receipts in this tree.

**Verdict, up front:** full-history replay in the #3424 piece-5 literal sense is *not
feasible from current records and is not what the allocated requirements ask for*.
What exists is a bounded, rolling audit log beside authoritative current-state rows.
Bounded replay of the *transition timeline* inside the retention window is feasible
today; reconstruction of arbitrary past *state* is not, because most mutations do not
record before/after values, several current-state fields are overwritten or cleared
in place, and the log itself is pruned on a 30-day horizon. §7 evaluates the
alternatives, including the no-full-replay alternative, which this research recommends
for now, paired with the bounded journal extension already designed in #9208/JRN-01
if and when per-field reconstruction is ever genuinely required.

## 2. Method and primary sources

Every classification in §3 was read from a primary source, not inferred from a name.
In particular, no semantics were inferred from the word `events` in a table name —
the slice's explicit prohibition, and the same warning #9208 records
("The existing `events` table may be operational telemetry, not a domain event
stream"; `components/9208-typed-dependencies-notes-journal-field-authority.md` §3).

| # | Primary source | What it establishes |
|---|---|---|
| S1 | `pkgs/fleet/crates/fleet-queue/src/turso_queue.rs` — `SCHEMA` const, `event()`, `audit_marks()`, `prune_events()`, `/events-*` verbs (read at `main` 84131d4b75) | The live table shapes, the event record's actual fields, the append-only surface, and the prune-based retention that bounds it |
| S2 | `doc/fleet/agent-events-turso.md` (#5690) | The predecessor PG stream's census (213,066 rows, ~33k rows/day at the 2026-11-06 audit), its writers/readers, the coupling verdict, and the rule that historical import lands in a separate archive shape, never merged into the live `events` table |
| S3 | `doc/2026-11-11_design_id8579_queue_schema_wipe_test_audit.md` (#8579, PR #8602) | Field-by-field durability classes for all 9 fleet-queue tables; where the durable half of each split field must live (GitHub agent notes and labels) |
| S4 | `doc/2026-11-10_design_queue_task_data_model_3424.md` (#3424) | The six-piece durable task model; piece 5's before/after wording; the intake observation that live rows carry only *adjacent* fields (`ready_condition`, `last_checked_at`, `claim_*` stamps), not the pieces themselves |
| S5 | `components/9208-typed-dependencies-notes-journal-field-authority.md` §2–§3, FDR-PMO-005 | The census verdict that the current `events` table "does not establish before/after for every task mutation or full reconstruction. Not a proven domain journal."; the future journal envelope (per-work sequence, schema version, before/after or immutable payload reference, visibility, supersedes refs) that a real journal would require |
| S6 | `research/track-a-state-of-art.md` §3B, §4; `research/track-b-state-of-art.md` "Dependencies, journal, notes" and decision matrix row D | prior art: event-sourced aggregates and pure event sourcing both assessed and *not* recommended as the starting point; the recommended shape is append-only journal facts alongside relational current state |
| S7 | Live reads by this lane, 2026-11-13: `gh pr view 9225`, `gh issue view 9466`, `fleet queue health`, `fleet queue timeline 15991` | Gate state; the timeline a consumer actually sees for one task (see §4.3) |

Accessible surfaces for a future investigator: the broker verbs `/events-list`,
`/events-count`, `/events-seen`, `/events-append`, `/events-prune`
(`turso_queue.rs` dispatch table), the client `fleet queue timeline <task>` view,
`fleet queue health` (whose `terminal_events_24h` / `terminal_events_7d` counters are
an events⋈tasks join, per S2 reader R2), and the GitHub issue/PR record. There is no
broker verb that reconstructs a task's state as of a past instant; no code path in
`fleet-queue` performs state reconstruction from `events` (the `replay` executor in
`client.rs` is the unrelated PR-stack rebase replay of #8582 — a different meaning of
the word, recorded here so the two are never conflated).

## 3. Evidence matrix — every record source classified

Classification vocabulary (the oracle's): **authoritative mutation** = the record
whose write *is* the state change; **observation** = a durable record that a change
or external fact was seen, without being the change itself; **telemetry** =
operational measurement exhaust, valuable for operations, not for state;
**projection** = derived, rebuildable state; **unknown** = semantics or contents
not verified by a primary source in this research.

| Record source | Class | Evidence and limits |
|---|---|---|
| `tasks` row — `status`, `priority`, `payload`, `title`, `body` | authoritative mutation (current state only) | The claim/transition UPDATEs write this row (`turso_queue.rs`); it holds the *latest* values. Prior values survive only if an event detail happened to carry them (§4.1). #8579 classes: packet fields durable truth, `status` split (in-flight = coordination, terminal = durable truth whose home is the GitHub outcome note). |
| `tasks.progress` (checkpoints, stage, `session_id`, audit-adjacent fields) | authoritative mutation (current state only) | Overwritten by every checkpoint/heartbeat UPDATE (e.g. the `progress = ?` update in the block path). It is the live resumption record — the fleet's anti-evaporation discipline reads it — but it is a *current* value, not a history: yesterday's checkpoint content is gone once today's lands. |
| `tasks.result` | authoritative mutation (latest outcome only) | One JSON value, overwritten per transition (park result, then block result, then terminal result). On task 15991 itself, claiming this shift already replaced the recorded park result's context: the row's `result` at claim time still described the earlier dep-block park while `progress` described the new claim. Only the latest outcome is on the row; earlier outcomes are recoverable only from the GitHub agent notes the lane discipline posts per transition (#8579: the agent-note stream is the home for the durable half). |
| `tasks.blocked_deps` + `dep_next_recheck_at`, `dep_blocked_until`, `dep_recheck_seconds` | authoritative mutation (current dependency set only) | Written by the dep-block path; *cleared to NULL* on unblock, requeue, and dep clearance (three separate `blocked_deps = NULL` updates in `turso_queue.rs`). The row therefore proves the *current* gate set and nothing about past gates; past gates live in `blocked` event details and issue notes, inside the retention window only. It is a JSON list on a row — #9208 census: explicitly *not* a typed dependency graph. |
| `events` — broker queue-transition kinds (`claimed`, `parked`, `blocked`, `completed`, `checkpoint`, `lease_expired`, `enqueued`, `lane_assigned`, `model_stamp`, …) | observation (append-only audit of mutations) | `event()` inserts `(worker_id, task_id, kind, detail, created_at)` in the same broker dispatch as the task write (S1, S2 verdict 1). The detail carries the #8187/#8176 audit marks (task, queue/pool, platform, lane, model stamp, session) plus verb-specific fields — *not* a guaranteed before/after pair. The module exposes no update/delete path for the table, but see the prune row below: append-only is a property of the verb surface, not of the data's lifetime. PG-era kind census (S2): checkpoint and claimed dominate volume — most of the stream is coordination noise, a fact #8579's split class for `events` already records. |
| `events` — non-queue writers via `/events-append` (GitHub CI webhooks: `push`, `issues`, `pull_request`; `fleet-fetch` `snapshot_refresh`) | telemetry / observation of external systems | Standalone inserts with `task_id` NULL (S2 writers W3/W4, ~23% of PG-era volume). They record that an external thing was seen; they mutate no queue state and can never contribute to reconstructing it. Their presence in the same table is the concrete reason the table's name proves nothing about journal semantics. |
| `events` — `events_pruned` self-log rows | telemetry (about retention itself) | `prune_events()` deletes rows older than the cutoff and inserts one `events_pruned` row carrying `{deleted, cutoff, keep_days}`. The count of what was destroyed survives; the destroyed content does not. |
| `leases` | authoritative mutation (coordination) | Current lease per task (`task_id` PK, `lease_until`, `heartbeat_at`). #8579: the wipe test's definition case — wiping loses only in-flight claims. No history. |
| `lanes`, `workers` | authoritative mutation (registry, current) | Current lane enablement and worker heartbeat state. No history; lane disablement events (e.g. the 2026-11-08 lane disables) are queue events/GitHub record, not registry history. |
| `checkpoints` table (`worker_id`, `key`, `value`) | projection (current-value store) | #8579: coordination — lane cursors and watermarks, overwritten in place; wiping costs a re-scan, never a decision. Not the same thing as task `progress` checkpoints, and not a history of either. |
| `repo_task_lane_ledger` | authoritative mutation (assignment record) | One row per task (`task_id` UNIQUE), updated in place (`assigned_lane`, reasons, `session_id`, timestamps). #8579 classes it durable truth (the assignment record), but as a current row it shows the *latest* assignment; reassignment history is in `lane_assigned` events, window-bounded. |
| `creation_outbox` + `creation_outbox_circuit` | authoritative mutation (durable intent) | A real state machine for GitHub-creation intents (pending → leased → completed, attempt counts, dedupe unique index, circuit breaker). This is the one place the current system already keeps a replayable *intent* log in the event-sourcing-adjacent sense: a stuck intent can be re-driven from its row. Scope is creation only — S5/#9208: "a bounded producer path, not a generic note or GitHub comment store", and Track A §4: not evidence of a general lifecycle outbox. |
| GitHub issues, PRs, labels, and the per-transition agent-note stream | authoritative mutation (durable-truth home, external) | Per #8579's homes: terminal status, priority decisions, and durable-transition outcomes live here, keyed by task in the notes. This is currently the *only* record whose retention is not bounded by the queue's prune — and it is prose-shaped, human/agent-authored, and incomplete for coordination transitions by design (checkpoints and heartbeats are deliberately not all mirrored). |
| PM Stack projector store (`pmp_*`: projections, `pmp_hot_events`, `pmp_deliveries`, consumer cursors) | projection (disposable, rebuildable) | Its README (via #9208 §1): the `pmp_` store "is disposable, rebuildable projection glue that never writes back"; reconciliation from GitHub can rebuild current resources, but "deleted comments and overwritten historical records no longer returned by GitHub cannot be recovered". An observation adapter — explicitly not Fleet operation truth, and itself a demonstration that projection rebuild ≠ history recovery. |
| `merge_intent` / stack-replay state (`merge_intent.rs`, `client.rs` stack replay) | authoritative mutation (separate typed aggregate) | A typed state machine for PR-stack rebase replay (#8582). #9208 census: it "does not prove generic work-item completion/outbox semantics". Its `replay` is re-executing git operations, not reconstructing queue state from events. |
| Predecessor PG `agent.events` (muse-dev2, pre-2026-11-06) | unknown (inaccessible archive) | The PG data plane has been down since 2026-11-06 (#6619). S2 verdict 2: the FK and id spaces do not carry to Turso; historical import "must land in a separate archive shape, never merged into the live `events` table". Whether a restorable dump exists is a backup question (S2 reader R4: the backup-of-record must be re-pointed consciously) that this research did not verify against a live backup — so the pre-cutover stream's current recoverability is honestly unknown, and no replay claim may rest on it. |
| Broker/application logs (`log.rs`, daemon logs) | telemetry | #9208 census: "`log.rs` is a logging facility, not the journal." Operational exhaust; no schema, no retention contract, no reconstruction semantics. |

## 4. What replay is actually possible today

### 4.1 Transition-timeline replay — feasible, inside the window

Within the retention window, `/events-list` (and `fleet queue timeline <task>`)
replays the *sequence of transitions* for a task: kinds in order, with worker,
timestamps, and audit marks. That answers "what happened, in what order, by whom"
for coordination and audit — the use the health view's terminal counters and the
`event_seen` ready-condition primitive (`/events-seen`) also make of the stream.
This is the replay the system genuinely supports, and S2's coupling verdict is
written for exactly these consumers: "every consumer reads events as a log, never
as a lock."

### 4.2 State reconstruction — not feasible in general

Reconstructing a task's *state* at an arbitrary past instant fails on at least
five independent grounds, each primary-sourced:

1. **No before/after guarantee.** #3424 piece 5 requires before/after values; the
   live event record (S1) has no before/after fields, and #9208's census states the
   schema "does not establish before/after for every task mutation or full
   reconstruction". Some verb details carry a new value; a reducer would have to
   guess the rest — guessing is not reconstruction.
2. **Current-state fields are destroyed in place.** `progress` and `result` are
   overwritten; `blocked_deps` is NULLed on clearance (§3). The row cannot
   corroborate a reconstruction, and the events that might cover those exact
   writes do not carry the overwritten contents in any schema-guaranteed form.
3. **The window is 30 days.** `prune_events` deletes `events` rows older than
   `keep_days` (default 30, clamped 1..=3650), run by the sweep and the
   `/events-prune` verb (S1). The PG predecessor had the identical shape: "a
   rolling ~30-day window, not an archive" (S2). Any "full history" claim silently
   expires a month after the fact.
4. **Atomicity is approximated, not guaranteed.** In Turso the task UPDATE and the
   event INSERT are separate pipeline statements in one dispatch; "a crash
   mid-verb can land one without the other" (S2 verdict 1). A stream that can
   silently miss a mutation cannot be the sole basis for resurrecting state.
5. **Not every authoritative write passes through the broker.** GitHub-side facts
   (labels, comments, merges) and the durable agent-note record live outside the
   stream entirely; a queue-only replay reconstructs at most the queue's view,
   and §2.6 of the source proposal explicitly forbids conflating that view with
   the work item's whole state.

### 4.3 A worked observation (task 15991, this research's own task)

`fleet queue timeline 15991` on 2026-11-13 lists `claimed`, `parked`,
`model_stamp` entries across its three claims — the transition skeleton is there.
But the checkpoints this lane wrote (`claimed`, `verified`, `branch_created`
stages, each carrying the next-step text) do not appear as timeline entries with
their contents; the resumption information a successor lane actually needs lives
in the *current* `progress` JSON on the task row, exactly as the anti-evaporation
discipline assumes. The system's real recovery mechanism in production is
current-state-plus-notes, not stream reconstruction. (Bounded claim: this is what
the timeline view surfaced for this task on this date; checkpoint *kinds* existed
in the PG-era census, so the Turso timeline view's kind coverage is itself a
surface to crosswalk before relying on it — see §8.)

### 4.4 The bounded reconstruction that *is* sound

For *terminal and durable* facts, reconstruction today is: task row (current) +
GitHub agent-note stream (per-transition durable notes) + PR/issue record. That
triangulation — not event replay — is what #8579's homes formalize, what lane
handoffs already practice, and what any honest "resurrect" story must currently
rest on. It covers decisions and outcomes; it does not cover coordination noise,
and it depends on lane note discipline, which is a process guarantee, not a
schema guarantee.

## 5. Retention, redaction, and compaction

**Current retention (verified, S1/S2):** `events` rows older than 30 days are
deleted by `prune_events(30)` (the PG cron's argument, carried into the Turso
`/sweep` and `/events-prune` verb; `keep_days` clamp 1..=3650). Task rows are not
pruned by this path; the `tasks` table accumulates terminal rows (the
repo-health-code-pilot health view alone reported 8,335 done and 481 cancelled
of 9,293 total on 2026-11-13). No compaction, snapshotting, or archival export of
`events` exists in the broker today; the self-logged `events_pruned` count is the
only trace of a prune.

**Redaction:** there is no selective-redaction path. The verb surface exposes no
per-row delete; the prune is age-based and kind-blind. A secret or private text
that landed in an event `detail` (or in `progress`/`result`, which are mirrored
into checkpoints and notes) can today be removed only by age-prune or by direct
database surgery outside the broker contract. #9208's journal design answers this
for the future journal — deletion as a tombstone/redaction *event*, original
retained under the approved retention policy — but that is a design, not a
capability. Any retention proposal must state this gap plainly: bounded retention
by age is currently the *only* redaction mechanism, and it redacts everything
else too.

**Compaction options** (feasibility notes, not proposals to implement here):
rolling daily counts per (queue, kind) before prune would preserve the health
view's long-range shape at trivial cost; snapshotting a terminal task's final
row into its GitHub outcome note already happens by discipline and could be made
mechanical; neither compacts the *transition* history, which is the part with
audit value inside the window.

## 6. Migration evidence required before any future history claim

If a later, separately authorized implementation slice (e.g. JRN-01) wants to
claim reconstruction for *covered* fields, #9208 and #3424 together require, at
minimum: a pinned-commit crosswalk of every event producer and payload against
every authoritative field (before/after coverage per field, not per table);
the journal envelope of #9208 §3 (per-work sequence/version, schema version,
authority-time timestamp, authenticated-principal provenance, operation/source
refs, visibility, supersedes refs for corrections); a transaction story that
closes or detects the S2 crash-gap (batched pipeline transaction, or a
gap-detection/repair verb); a prune/archive interplay design in which historical
material lands in a *separate archive shape*, never merged into the live table
(S2 verdict 2) and the backup-of-record is consciously re-pointed (S2 R4);
migration of existing rows as unknown/unverified provenance, never fabricated
(#9208 migration section); and the named oracles — #9208's atomicity and MBT
oracles, JRN-01's `pmo_journal_correction_appends_superseding_event_without_rewriting_prior`,
and this slice's oracle run against the new matrix. CEN-04 (#9434, open) owns the
cross-model schema/migration compatibility matrix that any such migration must
fit; QTI (Queue Table Improvements) is recorded in #9466's runtime gates as
unlocated/unverified with #9241 as its provenance-recovery issue — both are
genuine prerequisites for a migration, and neither blocks this research.

## 7. alternatives

| Alternative | What it buys | What it costs / why not (or not yet) |
|---|---|---|
| **A. No full replay (recommended now).** Keep current-state rows + 30-day transition log + GitHub durable notes; describe the system honestly in those terms. | Zero migration; matches what consumers actually do (§4.1, §4.4); satisfies P-R006/H-R023 as written — recoverability via durable progress, leases, outbox, checkpoints, and repair, with no event-store mandate. | No arbitrary-instant resurrection; durable record depends on note discipline; audit beyond 30 days is GitHub-shaped only. Accepted knowingly: no current consumer has presented a requirement that §4.4's triangulation fails. |
| **B. Bounded journal extension (the #9208/JRN-01 path).** Add the journal envelope to the *existing* store for covered fields; corrections append superseding events; reconstruction claimed only for covered fields. | Provable per-field history where it matters; keeps one store, one identity; FDR-PMO-005's proposed decision. | Real migration + producer crosswalk + transaction work (§6); CEN-04/QTI gates; permanent schema-versioning duty. This is the only upgrade path this research endorses — when a consumer requirement, not a table name, demands it. |
| **C. Full event sourcing.** Task state becomes a fold over an event stream; projections rebuild from scratch. | Theoretical completeness of replay. | Rejected by both prior-art tracks: Track A shape B ("much more architecture than the user asks for… Do not adopt a standalone event store merely to get reason codes") and Track B matrix row D ("Use append-only journal facts plus relational current state, not convert whole Fleet DB to event store"). Adds event-versioning, compaction, and projection-consistency burdens permanently; the mutable-external-facts problem (GitHub state) survives untouched. |
| **D. Archive-before-prune export.** Export expiring `events` rows to a separate archive shape before the 30-day prune. | Extends the audit horizon cheaply; S2-compliant shape separation. | Buys *log* retention, not reconstruction (§4.2 grounds 1, 2, 4, 5 still apply); creates a second store to secure, back up, and eventually explain; worth doing only if an audit consumer names a horizon > 30 days. |
| **E. GitHub-as-history.** Treat the issue/PR/note record as the history of record and drop queue-side history ambitions. | Retention and access control already solved by the platform; #8579's homes already point here for durable truth. | Coordination transitions are deliberately not all mirrored; GitHub cannot recover what it never saw (projector README, §3); queue-internal audit (claims, leases, dep rechecks) would go dark. A complement to A, not a replacement for the window log. |

## 8. unknown — open questions and evidence gaps

- **Turso-era event volume and kind census:** not measured in this research. The
  broker exposes `/events-count`; the PG-era census (~33k rows/day, checkpoint-
  dominated) is *not* assumed to carry over. Any retention sizing needs a fresh
  count first.
- **Timeline kind coverage on Turso:** §4.3's observation (checkpoint contents not
  surfaced for task 15991) is a single-task observation, not a census; whether
  checkpoint events are written, filtered by the view, or both is unverified.
- **Pre-cutover stream recoverability:** whether a restorable muse-dev2 dump of
  `agent.events` exists is unverified (§3, PG row); treat all pre-2026-11-06
  queue history as unknown until the backup record is checked.
- **CEN-04 / QTI status:** #9434 open; QTI unlocated (#9241). Both bound §6, and
  the field-authority half of any journal design is unknown until CEN-04 lands.
- **Global registration:** #9180 open; per #9466 it is not a blocker to this
  bounded design work, and no claim here depends on it.

## 9. Recommendation

Adopt alternative **A** as the stated position — no full replay; the honest
description is *current-state authority + bounded append-only audit log (30-day
rolling retention) + GitHub durable-transition record* — and keep alternative
**B** (the #9208/JRN-01 bounded journal, FDR-PMO-005) as the designed, gated
upgrade if a named consumer ever needs per-field reconstruction. Do not create a
second event database, do not describe the current `events` table as an event
store or a domain journal, and do not let the word replay in the stack-replay
feature be cited as evidence for history replay. Rollback for this slice is
research-only: deleting this document restores the prior state exactly.

## 10. prior art receipts

- Track A (`research/track-a-state-of-art.md`, receipts `research/track-a-sources.json`):
  shape B assesses an event-sourced aggregate with rebuildable projections —
  provenance and deterministic replay in exchange for permanent retention,
  compaction, and replay-compatibility costs; §4 pins replay to accepted
  normalized commands and pinned inputs ("otherwise replay today can differ from
  yesterday"), and warns the creation outbox is not evidence of a general outbox.
- Track B (`research/track-b-state-of-art.md`, receipts `research/track-b-sources.json`):
  decision matrix row D rejects pure event sourcing for Fleet in favor of
  append-only journal facts plus relational current state; row B records Temporal
  as a *reference model* for durable execution and event-history replay — concepts
  adopted, infrastructure deferred; the journal section specifies append-only
  observations/notes/receipts with superseding corrections, and mutable current
  state as a projection, not a substitute for history.
- Internal precedents: the creation outbox (bounded durable intent, §3), the
  #5690 coupling verdict ( consumers read events as a log, never as a lock), and
  the #8579 wipe test (durability decided field-by-field, with GitHub as the home
  for durable truth) are the fleet's own prior art for deciding history questions
  by evidence class rather than by aspiration.

---

*Deliverable for #9466 / task 15991. Sources are cited inline by file and section;
no deployed or live behavior beyond the §2 S7 reads is claimed.*
