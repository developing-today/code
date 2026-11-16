# agent.events — Turso sink, coupling verdict, and cutover (#5690)

Companion to [queue-turso-cutover.md](queue-turso-cutover.md) (the
queue core, #6741). That runbook's scope was `agent.tasks` and its
satellites; this one resolves the piece it explicitly left behind:
the `agent.events` append-only stream in muse-dev2 (Supabase
Postgres), which the 2026-11-04..06 migration
([#5689](https://github.com/developing-today/code/issues/5689) /
[#5690](https://github.com/developing-today/code/issues/5690))
classified `S:streaming -> Turso` but left in place, blocked by
design on transactional coupling with the queue core. The outage
record is [#6619](https://github.com/developing-today/code/issues/6619).

Full Gate-1 inventory with per-row evidence:
`supabase/turso-migration/consumer-audit-5690.md` in hatch-personal
(commit `a771206ca`). This doc is the verdict + the build + the
procedure; the audit stays the evidence of record.

## The stream (as of the 2026-11-06 audit; PG has been down since)

- `agent.events(id bigint identity, ts timestamptz, worker_id text,
task_id bigint FK -> agent.tasks(id), kind text NOT NULL,
detail jsonb NOT NULL DEFAULT '{}')`; 88.8 MB, 213,066 rows,
  ~33k rows/day (~23/min), insert-only workload. RLS on, no
  triggers, no inbound FKs except its own FK to tasks.
- Kind census (top): checkpoint 70,091 · claimed 39,129 · issues
  37,990 · parked 19,086 · lease_expired 12,349 · enqueued 8,617 ·
  completed 7,164 · pull_request 5,410 · push 5,275 · lane_assigned
  3,535 · snapshot_refresh 1,533 · … · events_pruned 10.
- Retention: `prune_events(30)` deletes rows older than 30 days, so
  the table is a rolling ~30-day window, not an archive.

## Writers

| #   | Writer                                                                                                                                                                                                           | Mechanism                                                           | Transaction scope                                                                                            |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| W1  | 17 queue RPCs (`claim_repo_task`, `checkpoint_task`, `complete_task`, `fail_repo_task`, `park_task`, `block_task`, `cancel_repo_task`, `enqueue_repo_task(_fleet)`, lane assign/bind/record, `expire_leases`, …) | `INSERT INTO agent.events` inside the function body                 | **Same PG transaction as the `agent.tasks` state change** — atomic                                           |
| W2  | `expire_leases()` via pg_cron `agent-expire-leases` (every minute)                                                                                                                                               | part of W1's family                                                 | same                                                                                                         |
| W3  | `github-ci` Edge Function (webhooks: push / issues / pull_request)                                                                                                                                               | `supabase.schema("agent").from("events").insert(...)`, task_id NULL | standalone insert; **the function returns HTTP 500 if it fails** (GitHub retries, then disables the webhook) |
| W4  | `fleet-fetch.py` (sandbox systemd timer, every 10 min)                                                                                                                                                           | one `snapshot_refresh` insert via supabase-exec                     | standalone; rides in the same SQL batch as an `agent.watermarks` upsert                                      |
| W5  | `prune_events(30)` via pg_cron `agent-prune-events` (daily 04:17)                                                                                                                                                | self-logging `events_pruned` insert after its DELETE                | same transaction as the delete                                                                               |

## Readers

| #   | Reader                                                                                             | What it needs                                                                                                      | Cross-database problem                                           |
| --- | -------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------- |
| R1  | `revalidate_parked_tasks()` (pg_cron `park-revalidation-daily` 05:17)                              | `EXISTS (events WHERE task_id = parked.id AND kind = <the kind named in a {"kind":"event_seen"} ready_condition>)` | **Yes — the blocker.** PG cannot query Turso                     |
| R2  | `agent.repo_queue_health` view → broker `/health` (fleet-watchdog 15 min, claimant, worker shifts) | `terminal_events_24h` / `terminal_events_7d` per queue = events ⋈ tasks on terminal kinds                          | Yes, as a PG view; **no** once events and tasks share a database |
| R3  | Ad-hoc observability SQL via the Management API                                                    | `count(*) WHERE kind = ?`, rows `WHERE kind = ? AND ts > ?`                                                        | Needs an equivalent query route                                  |
| R4  | pg_dump backup (`supabase/muse-dev/backup/config.json`, 12 h cycle, 90 d retention)                | events is inside the `agent` schema dump                                                                           | Backup-of-record must re-point consciously                       |

## Coupling verdict

1. **The transactional coupling is real but narrow.** Only W1/W2
   (the queue RPC family) write events atomically with task state.
   That coupling is preserved by construction in the Turso queue
   backend: `turso_queue::event()` runs in the same broker dispatch
   and the same database as the task write. What libSQL does _not_
   give is PG's single-transaction rollback: the task UPDATE and
   the event INSERT are separate pipeline statements, so a crash
   mid-verb can land one without the other (the queue runbook's
   atomicity section). The PG semantics worth keeping — "a state
   change and its event are never _silently_ separated" — is
   approximated by single-writer serialization + same-dispatch
   ordering; strict atomicity would need a batched pipeline
   transaction and is recorded as a known deviation, not a blocker,
   because every consumer reads events as a log, never as a lock.
2. **The FK `events.task_id -> tasks.id` does not carry.** The
   Turso `events` table (queue runbook deviation) has no FK; PG
   task ids and Turso task ids are different id spaces anyway.
   Historical import must therefore land in a separate archive
   shape (step C4), never merged into the live `events` table.
3. **R1 (`event_seen`) is the only semantic blocker**, and it is a
   consumer-design problem, not a data problem: the check has to
   move to wherever the events live. The sink ships the primitive
   (`/events-seen`); the revalidator becomes a broker-side sweeper
   (step C3). Until the sweeper exists, `event_seen` parked tasks
   are the one readiness class that does not survive cutover —
   same status as queue-runbook deviation 4, now with its
   replacement designed and its primitive built.
4. **R2 dissolves in Turso**: events ⋈ tasks is a local JOIN, and
   `/health` on the Turso backend once again returns
   `terminal_events_24h` / `terminal_events_7d` (this PR).
5. W3/W4 (23% + ε of the stream by volume) have **no coupling at
   all** — they were blocked only by being easy to forget. Both get
   config flips (below); neither flip changes a default.

## What is built (this PR, stacked on #6755)

In `fleet-queue`'s Turso backend (`turso_queue.rs`), all behind the
existing `QUEUE_BROKER_BACKEND=turso` selection — the Postgres
dispatch and the production default are untouched:

- `events` indexes `(kind, created_at)` and `(created_at)` added to
  the idempotent schema ensure (the query shapes R1/R3/prune need).
- `POST /events-append {worker_id?, task_id?, kind, detail?}` —
  the sink endpoint for non-queue writers once the broker is
  Turso-backed; returns the inserted row id.
- `POST /events-seen {task_id, kind}` → `{seen, count,
latest_created_at}` — the R1 primitive.
- `POST /events-count {kind?, since?}` → `{count}` — R3 parity.
- `POST /events-prune {keep_days? = 30}` — the `prune_events`
  port: deletes older rows and self-logs `events_pruned`.
- `/health` restores `terminal_events_24h` / `terminal_events_7d`
  per queue via the local events ⋈ tasks JOIN (R2).

Writer flips prepared outside this repo, defaults unchanged:

- `github-ci` Edge Function (hatch-personal
  `supabase/muse-dev/functions/github-ci/index.ts`):
  `AGENT_EVENTS_SINK=pg|dual|turso`, default `pg`. `dual` = PG
  insert (fatal, as today) + Turso insert (non-fatal, the existing
  ci_events mirror pattern). `turso` = Turso insert (fatal —
  preserves today's 500-on-loss semantics) + PG skipped. Target is
  `fleet-queue` via new secrets `AGENT_EVENTS_TURSO_URL` /
  `AGENT_EVENTS_TURSO_AUTH_TOKEN` (the ci mirror's `TURSO_*`
  secrets point at muse-platform and are _not_ reused).
- `fleet-fetch.py`: `AGENT_EVENTS_SINK=pg|dual|turso`, default
  `pg`; the `snapshot_refresh` row goes to the PG batch, the Turso
  pipeline (token by name `turso.fleet-queue-rw-token` via the
  secrets CLI), or both. The `agent.watermarks` and
  `agent.repo_snapshots` writes are a different bullet and stay PG.

  Rename note (2026-11-07): the sink was first verified live
  against the interim muse-queue instance; that instance was
  destroyed at the #6741 cutover and recreated fresh as
  fleet-queue (--tursodb), and the secrets + writer targets
  above were re-pointed and the sink re-verified against
  fleet-queue in PR #6762 follow-through.

## Cutover procedure

Preconditions: queue core cut over per queue-turso-cutover.md
(broker on the Turso backend in production), PG still the
historical source. Order matters; each step is independently
reversible until C5.

- **C1 — dual-write the external writers.** Deploy the Edge
  Function with `AGENT_EVENTS_SINK=dual` + the two new secrets;
  set `AGENT_EVENTS_SINK=dual` for the fleet-fetch timer. Verify
  for 24 h: fleet-queue `events` shows `push`/`issues`/
  `pull_request` and `snapshot_refresh` kinds arriving
  (`/events-count` per kind), counts ≈ PG counts for the same
  window ( Management API SQL while PG lives).
- **C2 — health parity.** With the production broker on Turso,
  confirm `/health` rows carry `terminal_events_24h/7d` and the
  watchdog's picture matches the last PG-view reading within
  stream lag.
- **C3 — revalidator sweeper.** Stand up the broker-side
  replacement for `revalidate_parked_tasks()`: a timer-driven job
  that lists parked tasks with `{"kind":"event_seen"}`
  ready_conditions from the Turso backend, calls `/events-seen`
  per task, and releases (un-parks to queued, `park_released`
  event) the satisfied ones — the PG function's exact outcome.
  Run it in observe-only mode (log would-releases) for one daily
  cycle against the PG function's output before enabling.
  **Until C3 is enabled, disable the PG cron
  `park-revalidation-daily` at queue cutover** (it would otherwise
  release against a stream that stopped; safer to pause the class
  than corrupt it) — or keep it if PG events are still dual-fed
  by C1 and the queue's own writers; decide at cutover time and
  record the choice on #5690.
- **C4 — historical window (PG-gated).** On a verified-green PG
  window: capture `max(id)` watermark, export
  `agent.events WHERE id <= watermark` (the DIY pg_dump backup is
  a valid source), and import into a new `agent_events_pg_history`
  table in fleet-queue (columns mirror PG: `pg_id, ts, worker_id,
task_id, kind, detail`) — an archive beside the live stream,
  never merged (id spaces differ, §2). Anything past the watermark
  is covered by C1 dual-write overlap.
- **C5 — flip the external writers.** `AGENT_EVENTS_SINK=turso`
  for the Edge Function and fleet-fetch. Watch one full webhook
  day for 500s (GitHub's webhook deliveries page) and one fetch
  cycle.
- **C6 — prune + backup.** Daily timer calling `/events-prune`
  (keep_days 30) replaces pg_cron `agent-prune-events`. Re-point
  the backup-of-record for the stream: fleet-queue joins the Turso
  backup routine; the PG `agent` schema dump keeps covering the
  pre-cutover history until C4 is verified, then events is
  excluded from the PG expectation (the table itself stays in PG,
  frozen, as the archive of record for its window).
- **C7 — close out.** Record the flip timestamps + final counts on
  #5690; queue-runbook deviations 1/4/5 are superseded by this
  doc's C2/C3/C6.

## What remains gated on PG recovery

- C4 (historical export at a watermark) — needs one green PG
  window or a verified DIY-backup restore.
- The PG-side decommission decisions (cron disables, the frozen
  table's eventual drop) — Drew's call, after C4 verifies. This
  task's limits: no drops, no default flips; both hold.
- Nothing else: the sink, the primitives, the writer flips, and
  the health fields are built and default-safe today.
