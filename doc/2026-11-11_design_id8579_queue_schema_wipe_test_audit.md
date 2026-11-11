# Wipe-test audit of the fleet-queue database schema

Issue: #8579 (fleet unification slice 1 of 7) · Parent design: #8575
(one door, one home per fact) · Date: 2026-11-11 · Status: audit only —
no schema changes in this slice.

## Scope, sources, method

The store under audit is the fleet-queue Turso database
(`fleet-queue-dezren39.aws-us-east-1.turso.io`), the live backend behind
every `fleet queue` verb. The schema of record is the `SCHEMA` constant
plus the additive `ALTER TABLE` migrations in
`pkgs/fleet/crates/fleet-queue/src/turso_queue.rs` (canonical broker
source since #5430); field semantics were checked against the writers
and readers in the same crate (`turso_queue.rs`, `validate.rs`,
`creation_outbox.rs`, `fallback.rs`). The store has **9 tables**:
`tasks`, `leases`, `lanes`, `workers`, `checkpoints`,
`repo_task_lane_ledger`, `events`, `creation_outbox`,
`creation_outbox_circuit`.

Classes (per #8575 and #8579):

- **Coordination** — ephemeral by design; wiping it loses only in-flight
  state. Stays in the database.
- **Durable truth** — losing it loses what was decided or done. Must move
  to GitHub or become a derivation of GitHub facts.
- **GitHub mirror cache** — a copy of a GitHub fact; rebuildable from
  GitHub at any time; never authoritative.

Every field below is classified, with the evidence for the class. Where
a field is split (coordination while live, truth at a terminal
transition), the split is stated — that split is exactly what the slice
owners below act on.

## tasks

The work-order table. 35 columns. This is where the two-store problem
lives: the table mixes the packet (truth), the claim machinery
(coordination), and the outcome record (truth) in one row.

| Field                                                                                                                                  | Class                    | Evidence / home                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| -------------------------------------------------------------------------------------------------------------------------------------- | ------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| id                                                                                                                                     | Coordination             | Internal surrogate. Caveat: agent notes on GitHub cite task ids, so after a wipe those citations dangle. Accepted: work is recoverable by issue/PR (below), and slice 7 archives a final id→issue map before any wipe is allowed.                                                                                                                                                                                                                                 |
| queue                                                                                                                                  | Coordination             | Stream label inside the one queue (#8097); routing only.                                                                                                                                                                                                                                                                                                                                                                                                          |
| title, body                                                                                                                            | Durable truth            | The human-readable work order, authored by the packet producer — not copied from GitHub, so not a mirror. **New home: the GitHub issue named in `payload.issues`** (packet posted on the issue at enqueue); for pr_shepherd tasks, the PR plus its batch issue.                                                                                                                                                                                                   |
| payload (JSON: schema, kind, repo, issues, prs, problem, priority_reason, evidence, acceptance, tests, context_refs, requires, dedupe) | Durable truth            | The machine-readable work order (`repo-task/v1`, `validate.rs`). Same home as title/body: the packet on the issue. The DB copy becomes a cache of it once slice 3 lands.                                                                                                                                                                                                                                                                                          |
| dedupe_key                                                                                                                             | Durable truth, derivable | Idempotence key against duplicate work. It is already a deterministic function of durable facts (`repo-health:v1:<kind>:<repo>:issue:<n>` etc.), so the **derivation rebuilds it** from the issue/PR set; no separate home needed.                                                                                                                                                                                                                                |
| priority                                                                                                                               | Durable truth            | A triage decision. **New home: the `priority/*` label** on the issue (already the durable-transition label set in the #8575 design).                                                                                                                                                                                                                                                                                                                              |
| status                                                                                                                                 | Split                    | `queued` / `claimed` / in-flight `parked` = coordination. Terminal `done` / `blocked` / `failed` = durable truth. **Home for the terminal half: the issue close + outcome agent note + durable-transition labels** (`status/done`, blocked labels) — labels change only at durable moments per #8575.                                                                                                                                                             |
| not_before                                                                                                                             | Coordination             | Scheduling gate for the claim scan.                                                                                                                                                                                                                                                                                                                                                                                                                               |
| worker_id, claimed_at, lease_until, lane_id, platform                                                                                  | Coordination             | The live claim. `platform` is the task's coarse platform selector (#8097 Rule 2); while live it is claim input, and its durable form already lives in the packet (`payload.requires`) / the recorded limitation on the issue.                                                                                                                                                                                                                                     |
| claim_provider, claim_model, claim_variant                                                                                             | Coordination             | #8176 claim-time model stamp; an audit mark recorded at claim, also carried into `events`. Losing it loses nothing decided.                                                                                                                                                                                                                                                                                                                                       |
| attempts                                                                                                                               | Split                    | Live retry counter = coordination. The final count is attempt history = record, and it is NOT derivable from claim events: reclaiming a parked/handed-off task adds 0 (`claim_update_sql`), transfers emit `claimed` without incrementing, and revival resets the counter to 0. **Home: the completing verb persists the final count inside the outcome note's result record** (slice 3 requirement); without that write the count is unrecoverable after a wipe. |
| max_attempts                                                                                                                           | Coordination             | Policy constant (default 4); config, not a fact about the work.                                                                                                                                                                                                                                                                                                                                                                                                   |
| progress (JSON: stage, next, artifact, session_id)                                                                                     | Coordination             | Lane checkpoints. Loss = the next lane re-derives state from the issue notes, which lanes already post at every checkpoint.                                                                                                                                                                                                                                                                                                                                       |
| result (JSON: outcome, pr, merge_sha, tests, ci, evidence, follow-ups)                                                                 | **Durable truth**        | THE outcome record — what was decided and done. **New home: the outcome agent note on the issue/PR + the issue close**, already the lane discipline at every completion; slice 3 makes the verb write the note as the primary write and `result` a cache of it. This is the audit's headline item.                                                                                                                                                                |
| last_error                                                                                                                             | Coordination             | Live failure context for the current attempt; terminal failure reasons that matter are in `result` / the block note.                                                                                                                                                                                                                                                                                                                                              |
| park_kind, wait_kind, ready_condition (JSON)                                                                                           | Coordination             | The park/re-check machinery (a parked task's wake-up condition). Machine state, rebuilt by re-evaluating the condition.                                                                                                                                                                                                                                                                                                                                           |
| (the park/block REASON, where one exists in progress/result)                                                                           | Durable truth            | A block that needs a person or a dependency is a decision. **Home: the issue's blocked labels + the agent note** recorded when the block is set.                                                                                                                                                                                                                                                                                                                  |
| last_checked_at, check_outcome                                                                                                         | Coordination             | Condition re-checker bookkeeping.                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| created_at, updated_at                                                                                                                 | Coordination             | Row bookkeeping.                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| completed_at                                                                                                                           | Durable truth, derivable | When the work finished. **Derivation: the timestamp of the outcome agent note** on the issue.                                                                                                                                                                                                                                                                                                                                                                     |
| blocked_deps (JSON), dep_next_recheck_at, dep_blocked_until, dep_recheck_seconds                                                       | Coordination             | #8276 dependency re-checker state. The underlying fact "blocked on issue #X" is durable and lives on the issue (note + blocked label); the re-check schedule is machine state.                                                                                                                                                                                                                                                                                    |

## leases

`task_id` (PK), `worker_id`, `lease_until`, `heartbeat_at` — **all
coordination**. This table is the wipe test's definition case: wiping
it loses only in-flight leases; lanes re-claim.

## lanes

| Field                     | Class                   | Evidence / home                                                                                                                                                                                                                                                                                                                             |
| ------------------------- | ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| lane_id, worker_id, queue | Coordination, derivable | The lane registry. Registrations dropping mid-shift (unknown lane_id verdicts) are the fleet's top infra pain, but the registry is configuration, not truth about work. **Derivation: the repo-declared lane roster** (the schedules/lane configuration that creates lanes); slice 3 or 5 makes registration self-healing from that roster. |
| enabled                   | Durable truth           | An operator decision (e.g. Drew disabled 12 lanes on 2026-11-08). **New home: the same roster configuration in the repo**, so the flag is a committed fact, not a row.                                                                                                                                                                      |

## workers

`worker_id`, `role`, `display_name`, `status`, `last_heartbeat_at`,
`created_at` — **coordination** (presence and heartbeat). `role` /
`display_name` are worker configuration, derivable from the worker
definitions; nothing here records what was decided or done.

## checkpoints

`worker_id`, `key`, `value`, `updated_at` (PK worker_id+key) —
**coordination**. Lane cursors and watermarks (the GitHub-marker
fallback coordinates through the same cadence, `fallback.rs`). Wiping
costs a re-scan, never a decision.

## repo_task_lane_ledger

One row per task: `queue`, `task_id`, `assigned_lane`, `executed_lane`,
`exclusion_reason`, `opencode_reason`, `platform_limit`,
`platform_limit_reason`, `assigned_by`, `session_id`, `fallback_reason`,
`assigned_at`, `executed_at`, `updated_at`.

**Class: durable truth (the assignment record), with one coordination
input.** The crate's own audit-marks comment (#8187) treats these marks
as the durable audit log: which lane was assigned and why, why an
assignment was excluded or limited, which session executed. Attempt
history is record per #8579.

- **New home: the per-issue agent-note stream.** Claim and completion
  notes already carry the lane stamp (`agent-note:v1 agent=… task=…`);
  slice 3's verb contract makes the assignment/exclusion/limit reasons
  part of the note the verb writes when they are set, instead of a
  DB-only row.
- `platform_limit` while the task is live is also a claim-eligibility
  input (coordination use), but its durable form — the recorded
  limitation with its reason — already belongs on the issue per #8210,
  so the DB copy is a cache of that record.

## events

Append-only: `worker_id`, `task_id`, `kind`, `detail` (JSON, carries
the #8187/#8176 audit marks), `created_at`. The module exposes no update
or delete path — it is the broker's audit log.

**Class: split.** Durable-transition kinds (completed, blocked,
parked-with-reason, terminal failure) are **durable truth**; purely
coordinative kinds (claims, heartbeats, checkpoints) are coordination
noise that may be wiped.

- **Home for the durable half: the issue's agent-note stream** — one
  note per durable transition is already the lane discipline; slice 3
  moves the write into the verbs so the mirror cannot be skipped.
- **History:** the pre-unification event history has no GitHub copy.
  Slice 7 owns a one-time export of the historical `events` +
  ledger tail to an archive (issue/doc) before any wipe is permitted;
  after the verb contract lands, new durable events are born on GitHub
  and the DB tail stops growing.

## creation_outbox

The GitHub-creation outbox (`creation_outbox.rs`): `id`, `repo`, `kind`,
`payload`, `dedup_key`, `payload_hash`, `producer`, `state`,
`created_at`, `available_at`, `attempt_count`, `max_attempts`,
`last_error_class`, `lease_owner`, `lease_until`, `github_number`,
`github_url`, `completed_at`.

| Field group                                                                            | Class                    | Evidence / home                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| -------------------------------------------------------------------------------------- | ------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Undelivered `payload` (state pending/leased, no `github_number` yet)                   | Durable truth            | An undelivered row's authored payload exists nowhere else: the pilot producer enqueues INSTEAD of creating (`doc/fleet/creation-outbox.md`), and a GitHub dedupe check cannot recover a payload that was never delivered. Wiping it silently discards accepted filings. Migration: wipe precondition — the outbox drains to empty, or undelivered rows are exported with the slice-7 archive; slice 3 owns giving producers a persisted, replayable source where one exists. |
| Pending-work machinery (state, available_at, attempt_count, lease_*, last_error_class) | Coordination             | Delivery bookkeeping for the rows above; carries no content of its own.                                                                                                                                                                                                                                                                                                                                                                                                      |
| dedup_key, payload_hash                                                                | Durable truth, derivable | Same deterministic dedupe function as `tasks.dedupe_key`; rebuildable from the producer's inputs.                                                                                                                                                                                                                                                                                                                                                                            |
| github_number, github_url, completed_at (on completed rows)                            | GitHub mirror cache      | The truth is the created GitHub issue itself; these fields only point at it. Rebuildable by listing the repo's issues.                                                                                                                                                                                                                                                                                                                                                       |

## creation_outbox_circuit

Singleton: `consecutive_rate_limits`, `open_until`, `last_error_class`,
`updated_at` — **pure coordination** (the rate-limit circuit breaker for
the outbox). Wipe = the breaker starts closed; nothing decided is lost.

## What is NOT in this database (checked, for the record)

- No GitHub mirror tables exist yet — issues/labels/PR state are read
  live from GitHub by lanes today. Slice 2 (#8580) ADDs mirror tables;
  they are born mirror-cache and must be rebuild-tested as such.
- Merge-order / stack tables do not exist yet — slice 4 (#8582) adds
  them; they are coordination by design (#8575 piece 4) and must pass
  the wipe test at birth.
- The secrets broker is a separate store and out of scope for #8579.

## Durable-truth inventory (everything that must move or derive)

1. `tasks.title` / `body` / `payload` → packet posted on the issue
   (owner: slice 3, verb contract — enqueue writes GitHub first).
2. `tasks.result` + terminal `status` + `completed_at` + final
   `attempts` → outcome agent note + issue close + durable-transition
   labels (owner: slice 3; `completed_at` derives from the note
   timestamp; the final attempt count is written INTO the note's
   result record — it is not derivable from claim events, see the
   `attempts` row above).
3. Park/block reasons → blocked labels + note at block time
   (owner: slice 3).
4. `tasks.priority` → `priority/*` label (owner: slice 3; label
   de-mirroring rules in slice 7 keep priority labels as durable).
5. `tasks.dedupe_key`, `creation_outbox.dedup_key` → derivation
   (deterministic function of repo/kind/issue; owner: slice 3 documents
   and tests the function).
6. `repo_task_lane_ledger` reasons + assignment record → agent-note
   stream at assignment time (owner: slice 3); historical rows →
   one-time archive export (owner: slice 7).
7. `events` durable kinds → born on GitHub via the verbs (owner:
   slice 3); historical tail → archive export (owner: slice 7).
8. `lanes.enabled` → repo-declared lane roster config (owner: slice 3
   or 5; must land before the DB-down replica in slice 5 trusts the
   registry).
9. `creation_outbox` undelivered payloads → drain-or-export before
   any wipe (precondition owner: slice 7; replayable producer source
   owner: slice 3).
10. PRE-slice-3 history: existing `tasks.payload` / `tasks.result`
    rows have no GitHub home, and the completion `events` rows carry
    only audit marks + `via`, not the result — so the events/ledger
    archive alone cannot preserve them. A one-time backfill (export
    every live and completed task's packet + result to its issue or
    to the archive, verified field by field) is required before the
    wipe (owner: slice 7).

## Migration order for the later slices

1. **Slice 1 — this audit (#8579).** No schema changes.
2. **Slice 2 — GitHub mirror (#8580).** Adds only mirror-cache tables,
   each rebuildable; lands early because slices 3–5 read through it.
   Exit check: drop the mirror tables, rebuild from GitHub, diff empty.
3. **Slice 3 — verb contract v2 (#8581).** The truth moves: enqueue,
   complete, block, and assignment verbs write the GitHub home FIRST
   (packet on issue, outcome note, blocked labels/notes, assignment
   reasons) and treat the DB rows above as cache. Also owns the dedupe
   derivation and the lane-roster-as-config change. Exit check: for a sample of tasks completed after this slice, every
   durable field above is recoverable from GitHub alone — AND the
   slice enumerates every pre-slice-3 task row whose packet/result
   still has no GitHub home, as slice 7's backfill input.
4. **Slice 4 — merge-intent queue + stack replay (#8582).** New
   coordination tables only; wipe test at birth: wiping loses merge
   intents, which lanes re-declare from their local stacks.
5. **Slice 5 — local coordination replica (#8583).** Safe only after
   slice 3: what replicates is coordination + cache, so a stale or
   lost replica loses nothing durable. Owns the DB-down drill.
6. **Slice 6 — backups (#8584).** Turso→D1 export becomes a backup of
   coordination state (nice to have, no longer the only copy of any
   truth); restore drill verifies a rebuild-from-GitHub, not just a
   restore-from-export.
7. **Slice 7 — label de-mirroring + final reconciliation (#8585).**
   Stops queue-state label writes, archives the historical
   events/ledger tail, archives the final task-id→issue map, performs
   the pre-slice-3 backfill (inventory item 10) and verifies it,
   confirms the creation outbox is drained or exported, runs the
   reconciliation backlog one last time, and performs the closing
   wipe-test drill: wipe a copy of the database, rebuild from GitHub,
   and diff — the audit above is the checklist for that diff.
