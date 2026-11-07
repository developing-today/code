# Queue core on Turso — cutover runbook (#6741)

**Status:** backend BUILT and PROVEN live 2026-11-06; production
broker still on Postgres (default unchanged). Cutover = the env
flip in step 2 below, executed as its own change.

**Why:** the queue core (`agent.tasks`, `leases`, `lanes`,
`workers`, `checkpoints`, `repo_task_lane_ledger`) was the one
piece of the #5689/#5690-series migration deliberately kept on
Postgres. When the muse-dev2 data plane went down on 2026-11-06
(#6619, Supabase platform fault), it had no live home; the GitHub
fallback mode (`queue-fallback-mode.md`) carried the work. Drew
ordered the dedicated Turso replacement the same evening.

## What exists

- **Database:** Turso `muse-queue` (org dezren39, group
  great-star-lord, aws-us-east-1, id
  `01a1b414-2d01-7c27-8a43-783bb5b09296`,
  `libsql://muse-queue-dezren39.aws-us-east-1.turso.io`). RW token
  vaulted in BWS as `turso.muse-queue-rw-token` (the
  muse-mirror/muse-platform pattern; BWS only — those tokens have
  no 1Password copies either), verified by live readback.
- **Backend:** `pkgs/fleet/crates/fleet-queue/src/turso_queue.rs`,
  behind the existing broker protocol — no client changes.
  Selection is the broker's config: `QUEUE_BROKER_BACKEND=turso`
  (default `postgres`), `QUEUE_BROKER_TURSO_URL`,
  `QUEUE_BROKER_TURSO_TOKEN_FILE` (else the secrets broker resolves
  `turso.muse-queue-rw-token`). Schema is ensured idempotently at
  broker startup.
- **Schema** (canonical copy = the `SCHEMA` const in
  turso_queue.rs): `tasks` (all PG columns the verbs touch, JSON
  columns stored as TEXT and parsed back on read), `leases`,
  `lanes`, `workers`, `checkpoints`, `repo_task_lane_ledger`,
  `events`; unique `(queue, dedupe_key)` index; claim-scan index.
- **Semantics** are reconstructed from the recovered live PG
  function bodies (`pm-stack/opencode-claimant/migration-0021.sql`
  ..`migration-0024.sql`): claim (incl. parked resume without an
  attempts increment, lane validation, binding clearing),
  expire_leases, fail (900 s / 2700 s requeue backoff), park
  (ready-condition + wait_kind mapping), lane assignment
  (opencode/direct balancing), bind (live-lease guard). A claim is
  one conditional UPDATE — libSQL's single-writer serialization is
  the atomicity; a competing claim matches no row and returns
  `{row: null}` (client exit 5), the PG envelope.

## Proof (2026-11-07T03:46–03:47Z, broker on 127.0.0.1:14801)

Against the real `muse-queue` DB, through the real client verbs:

1. Health (empty): `project: muse-queue-dezren39.aws-us-east-1.turso.io (resolved via turso)`.
2. Enqueue of the labeled migration-proof packet → task 1, queued.
3. Claim by `turso-proof-a` → claimed, lease +45 min, attempts 1.
4. Claim by `turso-proof-b` → refused: `{"task": null}`, exit 5.
5. Lane assign → ledger row, `assigned_lane=opencode`;
   bind → session stamped; heartbeat by B → refused
   (`task 1 is not claimed by turso-proof-b`, exit 5);
   heartbeat by A → lease extended; checkpoint → row in
   `checkpoints` (stage `proof-midpoint`); complete → `done`.
6. Lease expiry: task 2 claimed by `turso-proof-c` with a 2 s
   lease; `turso-proof-d` refused immediately, then claimed the
   task after expiry (attempts 2) and completed it.
7. Final health: queue `repo-health-code-shadow` — 2 done,
   0 queued/claimed. Every state above was also read back from
   the DB directly (tasks/leases/checkpoints/ledger/workers/events).

## Cutover steps

1. **Binary:** land a fleet build carrying this backend through
   the normal build/pin path. (The nix build is currently broken
   by #6725 — cargo vendor missing `sha2`; until it is fixed, the
   fastlane drop-in pattern from #6658 is the interim route.)
   Verify the installed binary answers `fleet queue health`
   against a Turso-backed test broker on a side port (as in the
   proof) before touching the production broker.
2. **Flip:** set `QUEUE_BROKER_BACKEND=turso` in the queue-broker
   unit env on the laptop and restart the unit. Nothing else
   changes: clients, claimant, and worker runbooks already speak
   only to the broker. **Drain decision first:** the Turso queue
   starts empty. Either (a) flip while the GitHub fallback is
   carrying work (PG is down anyway — there is nothing to drain
   until PG returns), or (b) if PG has returned, let PG claims
   drain to zero, export any open queued/parked/blocked PG tasks,
   and re-enqueue them into Turso preserving dedupe keys. Option
   (a) is the expected path while #6619 is open.
3. **Verify:** `fleet queue health` names the Turso host; run one
   labeled proof cycle (the #6741 packet shape) on the production
   broker; watch the first real lane claims land in the Turso
   `events` table.
4. **Soak:** GitHub fallback mode stays ACTIVE as the bridge
   until the Turso broker has carried a full day of real claims;
   workers keep routing to fallback whenever `fleet queue health`
   fails, per runbook §0a.

## Interaction with the PG staged-recovery sequence

**Unaffected.** If Postgres returns before cutover, the staged
sequence still runs in its order — outbox reconciliation (17) →
monitor requeue (16) → 142-at-cap requeue → migration finalize
scripts (`migrate_slice6_mirror.py`, then `finalize_pg.py`). The
requeues target the PG queue; if the flip (step 2) has already
happened when PG returns, run the export half of the finalize
scripts and the mirror drops, but skip the requeues — the work
they would revive is re-enqueued into Turso per step 2(b) or has
already been carried by the fallback.

## Rollback

Set `QUEUE_BROKER_BACKEND=postgres` (or remove it) and restart
the broker unit. The Turso backend never writes to Postgres and
the Postgres backend never writes to Turso; PG data is untouched
by a Turso soak. Tasks claimed in Turso during a soak stay in
Turso — on rollback, let them finish or expire there (claims are
visible in the Turso `tasks`/`events` tables) before declaring
the rollback complete.

## Deviations from the PG behavior (complete list)

1. `agent.events`: PG writes it in the same transaction as each
   queue transition, with an FK to `tasks(id)`, and the
   `repo_queue_health` view joins it for terminal counters. The
   Turso backend writes its own `events` table (no FK — libSQL
   enforces none here) and health counts task statuses directly;
   there are no `terminal_events_24h/7d` fields.
2. `repo_queue_health`'s exact PG column set was not recovered;
   Turso health returns per-queue status counts + total +
   `oldest_queued_at`. The client renders whatever columns the
   first row carries, so this is wire-compatible.
3. `expire_leases` runs inside claim (as in PG's claim body).
   PG additionally had a pg_cron job calling it every minute;
   there is no standalone Turso sweeper yet — leases also clear
   lazily on the next claim. Follow-up.
4. `revalidate_parked_tasks` (PG pg_cron, daily) is NOT ported:
   parked tasks with non-time ready conditions do not auto-release
   in Turso mode yet. Time parks release through claim at
   `not_before`, as in PG. Follow-up.
5. `prune_events` (30-day retention) is not ported; the Turso
   `events` table grows unbounded until it is. Follow-up.
6. Enqueue dedupe returns the existing row as-is; PG's
   revive-on-dedupe nuance (inferred from the client's outcome
   wording) was not fully recovered.
7. `heartbeat_task`, `checkpoint_task`, `block_task`,
   `complete_task`, `cancel_repo_task`, and
   `record_repo_task_lane_execution` bodies were not recovered;
   they are implemented to the broker/client contract (owner-only
   transitions, progress/result merges, ledger execution upsert)
   and are marked as such in the code.
8. Timestamps are second-resolution ISO-8601 UTC strings (PG
   `timestamptz` carries sub-second precision); ordering is
   lexicographic = chronological.
9. Task ids are per-database autoincrements starting at 1. PG
   task ids do not carry over; references to them remain PG
   history.
10. Priorities: NULL priorities sort last on claim/list (PG ASC
    default); packets are validated to always carry one, so this
    only affects hand-written rows.

## What still touches Postgres after the queue cutover

The delete-the-project checklist. Nothing below was moved or
dropped by #6741.

- **agent.events (PG stream, ~196k rows, FK → tasks)** — its
  queue-transition writers are replaced by the Turso `events`
  table at cutover; what remains is the historical stream plus
  its pg_cron consumers (below). Moving takes: port or retire
  prune/revalidate, confirm no direct readers remain, then
  export-or-abandon per the #5690 append-stream class.
- **pg_cron jobs** (`agent-expire-leases` every minute,
  `agent-prune-events` daily, `park-revalidation-daily`) — they
  die with the project. Expire is covered inside claim; prune
  (30-day retention) and park revalidation need small ports to a
  broker-side periodic task (deviations 4–5).
- **ci.events + the `github-ci` Edge Function** — the writer runs
  ON Supabase (Edge Functions), upserts PG, and dual-writes Turso
  `ci_events` (live since 2026-11-06). The morning-readout reader
  still reads PG (`ci.recent_failures`). Moving takes: re-home
  the webhook receiver off Edge Functions and flip the reader to
  Turso (flip criteria already staged: one verified real delivery
  landing in `ci_events`).
- **agent.mirror_\* PG originals (28 tables)** — verified shadows
  live in Turso `muse-mirror`; the PG copies remain the target of
  the **paused runtime-db-export cron**, whose loader still points
  at PG. Retiring takes: repoint that loader to `muse-mirror`
  BEFORE any unpause (standing hazard, recorded on #5691), then
  the staged `finalize_pg.py` drops (28 tables).
- **Slice-2 sheet staging + curated copies (#5692)** — Turso
  shadows landed; consumer cutovers pending. Separately, the
  ainventory relational core (`models`, `measurement_events`,
  `price_events`, `provider_policies`, `benchmark_results` + the
  views over them) is KEEP-on-Postgres BY DESIGN — not a leftover;
  deleting the project needs a deliberate re-home decision for
  that core.
- **Slice-1 `sources` remainder (#5695)** — 5 rows exist only in
  PG (`automation`/`finance`/`home`/`research` sources; D1
  `muse-config` holds the other 49 of 54). Moving takes one PG
  read at recovery + the D1 insert; then the 4 slice-1 Turso
  config shadows drop per the finalize script.
- **automation.events** — orchestration passes append via
  `automation_append.py`; appends have failed since the outage.
  Moving takes repointing that helper to a Turso events home
  (the #5690 stream class) — not yet done.
- **agent.external_comms** — the external-comms send-time ledger
  (`pm-stack/external-comms/record-external-send`) writes PG.
  Moving takes a Turso table + a script repoint.
- **pm-stack aggregator** (`src/supabase.ts`,
  `scripts/collect-fleet-feed.ts`) — a live reader of muse-dev2
  behind the fleet feed. Moving takes an audit of its exact
  tables and a repoint to the Turso shadows.
- **PG-bound tooling** — `supabase-exec` helper, the
  `pm-stack/supabase-export` quota script, and every consumer of
  the `supabase.muse-dev2.*` secret names: retire or repoint with
  their consumers, last.
- **Supabase platform schemas (auth 27, storage 10, realtime 3,
  vault 1 — the 41-table RETIRE class)** — no fleet consumer was
  found in the audits; the only platform dependency is Edge
  Functions hosting (the `github-ci` receiver above). Re-home
  that function first; nothing else needs moving, and deleting
  the project is what retires these.
