# Design brief: durable repo-queue task data model

- Date: 2026-11-10
- Status: **design-mill intake brief** — seeded source material for
  the design build-out, not the design itself. The owning summit
  session builds the design out in the design-mill pattern
  (statement inventory → state-of-the-art expansion → versioned
  designs → granular implementation issues).
- Issue: [#3424](https://github.com/developing-today/code/issues/3424)
  (design(queue): durable task data model — blocker links, park
  revalidation, status reasons + notes, full event history, actor
  provenance)
- Direction (Drew, 2026-11-01): make the repo-queue data model
  durable — "make this system to last." Six pieces, one design.
  The #3424 issue body is the canonical statement set until the
  summit session's statement inventory supersedes it.
- Related: [#3418](https://github.com/developing-today/code/issues/3418)
  (budget-aware claiming — the claim-side companion; blocker links
  and budget requirements both belong in the claim query).

## The six pieces (from #3424)

1. **External waits take a real datetime.** `park` accepts only
   `--delay-seconds` today; a mid-work park should also accept an
   absolute `not_before` datetime (external events have known
   instants: a quota reset, a review window).
2. **Blockers are links, not text.** Blockers recorded as typed
   links — to other queue tasks (preferred: queryable, so claim can
   exclude a task whose blocker task is incomplete, and completing
   a blocker can surface its dependents), and to PRs and issues.
   Today `block` stores free text; blocked-on relationships are
   invisible to the claim query.
3. **Long parks get re-verified on a schedule.** A maintenance
   check re-verifies every long park per item on a cadence (default
   daily), writes `last_checked_at` and its outcome onto the item;
   a still-valid park stays untouched, a stale park is flagged with
   a note (piece 4) for pickup or release.
4. **Status reason column + a notes table, mirrored out.** Every
   status transition records a `status_reason`, which automatically
   lands as a row in a notes/comments table (task, actor, kind —
   status_reason / finding / handoff — body, timestamp). The system
   comments notes back automatically to the linked GitHub issue.
5. **Full event history — resurrect any moment.** An append-only
   events table records every change to a task (status, priority,
   park, claim, links, notes) with before/after values and the
   actor. Any past state is reconstructable from the event stream.
6. **Actor provenance on every write.** Every create/update records
   who made it: lane, runtime, session id, hostname, pid, ip
   address, task id and the task's start, and — at creation — which
   system found/filed the originating issue. Provenance rides on
   tasks, notes, and events alike.

## Migrations question (in scope)

Schema changes land as proper versioned migrations, in order, in a
repo: the design must first inventory how the queue backend schema
(tasks, claim/park RPCs) is currently changed, and establish the
migration discipline before adding pieces 1–6.

## V1 baseline — what already exists (intake observation, 2026-11-10)

The live task rows already carry fields in this design's territory —
the summit session's statement inventory must verify each piece
against the current schema before designing anything new:

- `ready_condition` (e.g. `{"kind": "time", "until": ...}`) rides on
  parked tasks — adjacent to piece 1, though the park verb's
  absolute-datetime surface is still the open question.
- `last_checked_at` and `check_outcome` ride on task rows — adjacent
  to piece 3's revalidation record.
- `lane_id`, `claim_provider`, `claim_model`, `claim_variant` ride
  on claims, and PR #8178 (merged 2026-11-10) stamps the claiming
  session's own model on the task at claim time — a partial,
  growing piece 6.
- Task rows carry `park_kind` / `wait_kind` and a structured
  `result.park_reason` — adjacent to piece 4's status reasons, but
  no notes table or mirror-out is established by these fields.

## Acceptance (from #3424)

- Design record covering pieces 1–6: schemas for blocker links,
  notes, events; the park-datetime change; the revalidation check's
  cadence/config and where it runs; provenance field set.
- Migration discipline decided and the current schema state
  inventoried as the baseline migration.
- Implementation packets split per piece and enqueued, each
  independently landable.
