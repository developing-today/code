# Queue core on Turso — cutover runbook (#6741)

**Status:** CUTOVER EXECUTED 2026-11-07T05:10Z (2026-11-06 23:10
CST). The production queue broker runs the Turso backend against
`fleet-queue`; the backup load, live proofs, fallback guard,
failover switch, and outbox backfill are all verified below. The
Postgres gap backfill is a later manual follow-up (§ After
cutover), never a gate (Drew, 2026-11-06 21:52 CST).

**Why:** the queue core (`agent.tasks`, `leases`, `lanes`,
`workers`, `checkpoints`, `repo_task_lane_ledger`) was the one
piece of the #5689/#5690-series migration deliberately kept on
Postgres. When the muse-dev2 data plane went down on 2026-11-06
(#6619, Supabase platform fault), it had no live home; the GitHub
fallback mode (`queue-fallback-mode.md`) carried the work. Drew
ordered the dedicated Turso replacement the same evening.

## What exists

- **Database:** Turso `fleet-queue` (org dezren39, group
  great-star-lord, aws-us-east-1, id
  `01a1b43a-fb10-763c-ac90-5c1728b83906`,
  `turso://fleet-queue-dezren39.aws-us-east-1.turso.io`,
  **Type: Turso**). Created FRESH with
  `turso db create fleet-queue --group great-star-lord --tursodb`
  — the fleet standard is that new databases are born `--tursodb`
  (TursoDB/MVCC), per the atlas scaler revision design
  (doc/2026-11-04T21-27-34Z_design_atlas_scaler_revision, §6).
  History: the first instance was created 2026-11-06 as
  `muse-queue` WITHOUT the flag (Type: SQLite — libsql mode) and
  was destroyed on Drew's ruling (2026-11-06 22:13–22:16 CST);
  the replacement was created fresh under the name `fleet-queue`
  (it is the FLEET queue — OpenCode tasks that never touch Muse
  run through it too), never branched or converted in place. RW
  token vaulted in BWS as `turso.fleet-queue-rw-token` (the
  muse-mirror/muse-platform pattern; BWS only — those tokens have
  no 1Password copies either), verified byte-identical by live
  readback + a live pipeline `SELECT 1`. The retired
  `turso.muse-queue-rw-token` secret was deleted from BWS.
- **Backend:** `pkgs/fleet/crates/fleet-queue/src/turso_queue.rs`,
  behind the existing broker protocol — no client changes.
  Selection is the broker's config: `QUEUE_BROKER_BACKEND=turso`
  (default `postgres`), `QUEUE_BROKER_TURSO_URL`,
  `QUEUE_BROKER_TURSO_TOKEN_FILE` (else the secrets broker
  resolves `turso.fleet-queue-rw-token`). Schema is ensured
  idempotently at broker startup. Landed as PR #6755, merged
  `b88bb02f`.
- **Schema** (canonical copy = the `SCHEMA` const in
  turso_queue.rs): `tasks` (all PG columns the verbs touch, JSON
  columns stored as TEXT and parsed back on read), `leases`,
  `lanes`, `workers`, `checkpoints`, `repo_task_lane_ledger`,
  `events`; a NON-unique `(queue, dedupe_key)` index (deviation
  11); claim-scan index.
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

## Proof on the first instance (destroyed `muse-queue`, SQLite type)

2026-11-07T03:46–03:47Z, broker on 127.0.0.1:14801: full cycle
(enqueue → claim A → claim B refused exit 5 → lane → bind →
heartbeat B refused → heartbeat/checkpoint A → complete) plus a
lease-expiry reclaim (2 s lease; D refused, then claimed with
attempts 2). Every state read back from the DB directly. This
instance was then destroyed per the `--tursodb` ruling above; the
proof was re-run in full on `fleet-queue` (next section).

## Proof on `fleet-queue` (the live instance, 2026-11-07T05:08Z)

Against the real `fleet-queue` DB holding the loaded backup,
through the real client verbs, in a dedicated `fleet-queue-proof`
queue (so no backup-era task could be picked up by a proof claim):

1. Enqueue labeled migration-proof packet → task **10997**
   (ids continue from the backup's max 10996).
2. Claim by `turso-proof-a` → claimed, lease +45 min; claim by
   `turso-proof-b` → `{"task": null}`, exit 5.
3. Lane assign → ledger row, `assigned_lane=opencode`; bind →
   session `turso-proof-session-6741` stamped; heartbeat by B
   refused; heartbeat by A → lease extended; checkpoint → KV row
   `turso-proof-a / task:10997` (stage `proof-midpoint`);
   complete → `done`, result `{"proof":"fleet-queue-cycle-ok"}`.
4. Lease expiry: task **10998** claimed by `turso-proof-c` with a
   2 s lease; `turso-proof-d` refused immediately, then claimed
   after expiry (attempts 2, `lease_expired` event) and completed.
5. Refusal pair re-run (task **11000**): A claimed, B refused
   exit 5, B's heartbeat refused. A completed it.
6. **No-double-serve guard:** with `developing-today/code#6646`
   live-claimed in the GitHub fallback by `guard-holder-6741`, a
   Turso claim (task **10999**) for a packet naming that issue was
   auto-blocked: claim returned
   `{"task": null, "guard": "fallback_held", "held_by":
   "guard-holder-6741", ...}`, exit 5; the task row sits
   `blocked` with `reason_class: fallback_held` in its progress.
   The test task was then cancelled with the reason recorded and
   the fallback claim released.
7. **Permanent fallback switch (failover):** (a) the new client
   against the production broker while it still ran the dead PG
   backend emitted
   `{"ok": false, "fallback": true, "switch_to":
   "github-fallback", "verb": "/health", "broker_exit": 4}` and
   exited 6; (b) with the side broker killed by its recorded PID,
   a claim emitted the same switch JSON (transport error) and
   exited 6 immediately.
8. Every state above was read back from the DB itself
   (tasks/leases/checkpoints/ledger/workers/events).

## Cutover — as executed

1. **Binary:** PR #6755 merged (`b88bb02f`); the release build of
   the branch tip installed at `~/.local/bin/fleet` by atomic
   rename (the previous binary was preserved as
   `~/.local/bin/fleet.bak-20261106-fallback`; the Nov 4 binary
   remains as `fleet.bak-20261106-nov4`). The nix build path is
   still broken by #6725 (cargo vendor missing `sha2`); a durable
   nix pin waits on it (tracked on #6658). The sandbox
   `~/tools/fleet` is still the Nov 4 build (orchestration-only
   box).
2. **Backup load (executed, verified):** `fleet-queue` was
   populated from the fleet's last DIY R2 backup of muse-dev2
   (prefix `pg/muse-dev2`; restored export at
   `/home/user/workspace/scratch/muse-dev2-queue-core-export-2026-11-06/`,
   coverage **2026-11-06T10:47:11Z**, SHA-256 verified; record
   #6619 comment 6701907139). Loaded as-is, sanity readback only
   (Drew, 2026-11-06 21:51 CST): **tasks 8,170 · leases 107 ·
   workers 34 · checkpoints 2 · repo_task_lane_ledger 3,347 =
   11,660 rows, matching the export exactly**; status
   distribution done 7,039 · cancelled 432 · blocked 281 ·
   queued 234 · claimed 108 · parked 71 · failed 5; PG ids
   preserved (max 10,996). Loader note: the TursoDB instance
   rejects Hrana `batch` requests (HTTP 400) and per-statement
   pipeline latency is ~0.3 s, so the load ran as parallel
   execute-mode streams (~35 min wall). **In-flight claims in
   the restored data:** all 108 `claimed` rows carried expired
   leases; the first claims' `expire_leases` returned them to
   `queued` with bindings cleared — post-cutover health shows
   claimed = 0 in every queue, as designed.
3. **Flip (executed 2026-11-07T05:10Z):** systemd drop-in
   `~/.config/systemd/user/queue-broker.service.d/30-turso-queue.conf`
   (clears ExecStart, sets
   `ExecStart=/home/user/.local/bin/fleet serve queue-broker`,
   `Environment=QUEUE_BROKER_BACKEND=turso`), daemon-reload,
   `systemctl --user restart queue-broker.service`. Journal:
   `queue-broker: queue backend: turso
   (https://fleet-queue-dezren39.aws-us-east-1.turso.io)`,
   MainPID 925323, listening on 100.99.113.103:4801 + UDS. The
   broker resolves the token from the secrets broker by name —
   no token file in the unit.
4. **Verify (executed):** `fleet queue health` through the
   production broker names the Turso host and shows the restored
   queues (repo-health-code-pilot 5,711 · openchamber-pilot 935 ·
   opencode-pilot 861 · nix-pilot 580 · code-shadow 35, matching
   the backup); production proof task **11001** enqueued →
   claimed by `cutover-proof` → completed through the production
   path.
5. **Fallback stays armed PERMANENTLY** (Drew, 2026-11-06
   21:54 CST): this is not a soak bridge. Any primary-store
   failure on a work verb — health, claim, heartbeat, checkpoint,
   complete (transport error or error envelope) — makes the
   client emit the machine-readable fallback switch and exit 6,
   and the worker continues that work in the GitHub fallback
   (deviation 12 records the exact refusal semantics).
   `REPO_QUEUE_NO_FALLBACK_SWITCH=1` restores die-on-error for
   debugging. In-flight fallback claims stay in fallback until
   they release; the no-double-serve guard (proof 6) keeps the
   Turso queue from serving a fallback-held issue.

## Backfill from the fallback (executed)

Per Drew's master plan (2026-11-06 21:54 CST), the
backup→crash gap (10:47:11Z → 19:25:00Z) is reconciled from the
FALLBACK record, not from Postgres:

- **Outbox manifest**
  (`outbox-backfill-manifest-2026-11-06.md`, 33 packets): each
  target issue was re-verified open at cutover. **23 enqueued**
  into `fleet-queue` through the production broker — tasks
  **11002–11024**, all `queued` (code ×3 → 11002–11004 ·
  openchamber ×12 → 11005–11015 · opencode ×7 → 11016–11019,
  11022–11024 · nix ×1 → 11020; four of the 23 — openchamber#1737,
  opencode#1883/#1887/#1888 — carried packet-authoring defects
  (off-scale priorities 60/80; #1737 also kind `small-fix` and a
  `small-fix:` dedupe family) and were enqueued from minimally
  corrected copies: priority → 70, kind → `small_fix`, dedupe
  family → `fix:`; the prepared originals were NOT edited).
  **10 skipped** because their targets had closed: code#6492,
  #6588, #6593, #6594, #6608, #6630, #6631, #6679, #6680, #6734.
- **Fallback state deltas:** issues claimed in fallback stay in
  fallback until release (protocol unchanged); issues merely
  labeled `queue/queued` on GitHub are claimable in fallback at
  any time, and the guard keeps Turso from double-serving a
  fallback-held issue. No bulk import of fallback labels was
  needed beyond the outbox packets above.

## After cutover: the Postgres gap (manual footnote only)

The Postgres backfill is DROPPED as the plan of record. If the
muse-dev2 data plane ever becomes available again, anything
still missing gets ported manually: tasks created, completed, or
transitioned in the 10:47:11Z → 19:25:00Z gap, identified by
comparing PG `agent.tasks` (and its ledger) against the Turso
tables by dedupe key / task identity; if the two sources
conflict on a task, Postgres (the later state) wins. The
migration finalize scripts (`migrate_slice6_mirror.py`,
`finalize_pg.py`) remain PG-side hygiene for decommission time,
unrelated to the queue. End state for muse-dev2:
**decommissioned — left in place, unused — never deleted**
(Drew's data plan, 2026-11-06 21:49 CST).

## Rollback

Remove `30-turso-queue.conf`, daemon-reload, restart
`queue-broker.service` (returns to the #6450 Postgres build from
`20-fastlane-6450.conf`); or restore
`~/.local/bin/fleet.bak-20261106-fallback`. The Turso backend
never writes to Postgres and the Postgres backend never writes
to Turso; PG data is untouched. Tasks claimed in Turso stay in
Turso — on rollback, let them finish or expire there (visible in
the Turso `tasks`/`events` tables) before declaring the rollback
complete. Caveat: while the muse-dev2 data plane is down, a
rollback broker serves no queue at all and workers ride the
GitHub fallback — which stays armed permanently either way.

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
9. Task ids: the backup load PRESERVED PG ids (max 10,996);
   new tasks continue the same sequence (proof tasks are
   10997+). References to PG-era task ids therefore stay valid.
10. Priorities: NULL priorities sort last on claim/list (PG ASC
    default); packets are validated to always carry one, so this
    only affects hand-written rows.
11. The `(queue, dedupe_key)` index is NOT unique: the PG data
    itself carries duplicate pairs from before dedupe was
    enforced, and the backup load must accept them as-is.
    Enqueue dedupe is a read-then-insert check (single-writer
    serialization makes it race-free here).
12. Fallback-switch semantics (Drew: "encounter any issue →
    fallback"): work-verb REFUSALS that arrive as broker error
    envelopes (e.g. a heartbeat from a non-claimant, exit 5 at
    the broker) also emit the fallback switch (client exit 6) —
    the worker learns from the error text that the task is not
    theirs and continues in fallback. A claim that simply finds
    no claimable task stays a plain `{"task": null}` exit 5, and
    a guard-blocked claim stays exit 5 with the `guard` object
    (proof 6).

## What still touches Postgres after the queue cutover

The decommission checklist. Per Drew's data plan (2026-11-06),
muse-dev2 is **decommissioned — left in place, unused — never
deleted**; every item below is a move-or-retire decision that
stands between here and that decommission, and nothing below was
moved or dropped by #6741.

- **agent.events (PG stream, ~196k rows, FK → tasks)** — its
  queue-transition writers are replaced by the Turso `events`
  table at cutover; what remains is the historical stream plus
  its pg_cron consumers (below). Moving takes: port or retire
  prune/revalidate, confirm no direct readers remain, then
  export-or-abandon per the #5690 append-stream class.
- **agent.events Turso sink (PR #6762)** — the sink was verified
  live against the DESTROYED `muse-queue` instance; its
  `AGENT_EVENTS_TURSO_*` secrets still point at that name.
  Moving takes: re-point the secrets at `fleet-queue` and
  re-verify the sink live against the new instance.
  **Design resolved by the companion doc**
  [agent-events-turso.md](agent-events-turso.md) (#5690): full
  writer/reader inventory, the coupling verdict, the sink verbs
  (`/events-append`, `/events-seen`, `/events-count`,
  `/events-prune`), health terminal counters restored, the
  external-writer flips (github-ci Edge Function, fleet-fetch),
  and the C1–C7 procedure superseding deviations 1/4/5.
- **pg_cron jobs** (`agent-expire-leases` every minute,
  `agent-prune-events` daily, `park-revalidation-daily`) — they
  live inside the project and become irrelevant at decommission
  (expire is covered inside claim); prune (30-day retention) and
  park revalidation need small ports to a broker-side periodic
  task (deviations 4–5).
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
  full decommission waits on a deliberate re-home decision for
  that core (until then the project stays partially live for it).
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
  that function first; nothing else needs moving — these stay
  in the decommissioned project, unused, like everything else.


## Amendment 2026-11-10 — claim-model stamp columns (#8176)

Drew's order (2026-11-10 10:53 CST): a session picking up an item
provides its own model to the queue. `tasks` gains three additive
columns — `claim_provider`, `claim_model`, `claim_variant` — the
claiming session's serving leaf at claim time, read by the session
from its own `route_usage` lastServing surface (opencode#1891) and
passed on the claim (`repo-queue claim --model-provider/--model-id
[--model-variant]`; broker request fields `model_provider` /
`model_id` / `model_variant`). `ensure_schema` adds the columns to
pre-existing databases exactly like `platform` (#8097); existing
rows read NULL (the claim carried no model). The `claimed` event
detail carries the same three fields. Non-session claimants (Muse
lanes) stamp the distinct provider marker `muse-lane` rather than
a model. Claim-time semantics: a later failover changes the
serving leaf but never rewrites the stamp.
