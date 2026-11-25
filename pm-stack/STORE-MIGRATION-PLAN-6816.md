# Store migration plan — Kaneo / Grist / Pad cluster-native store targets

Issue: #6816 (step 1 — inventory and plan only). Parent: #3989. Rulings: #1258
(2026-11-03). Backups routine: #1514; empty-generation handling: #3869.

**This plan moves no data and reconfigures no service.** The inventory is
historical (2026-11-10), not a fresh live-host observation. The target decision
below replaces the deleted Supabase `muse-dev2` target as of 2026-11-24;
fencing, the cutover sequence, and rollback still precede any live move.

## 1. Inventory — verified live on the laptop (amd-6), 2026-11-10T00:32–00:36Z

Method: `docker ps`, `docker volume ls`, `du` over read-only volume mounts,
`psql` against the running Kaneo database. Every figure below was read from
the live stack at execution time, not copied from an earlier report.

| Store | Runtime (healthy, up 3 days)                                                                     | Volume                   | Volume size | Canonical content                                                                                                               |
| ----- | ------------------------------------------------------------------------------------------------ | ------------------------ | ----------- | ------------------------------------------------------------------------------------------------------------------------------- |
| Kaneo | `pm-stack-kaneo-1`, `ghcr.io/usekaneo/kaneo:2.47.0`; `pm-stack-postgres-1`, `postgres:16-alpine` | `pm-stack_postgres_data` | 197.9 MB    | Database `kaneo` (role `kaneo`), **80 MB** logical size, 46 user tables; the rest of the volume is WAL + base overhead          |
| Grist | `pm-grist`, `gristlabs/grist-oss` (digest-pinned, v1.7.21)                                       | `grist_grist_data`       | 2.4 MB      | `home.sqlite3` 380 KB (doc metadata/registry), `grist-sessions.db` 1.8 MB (sessions), `docs/` 228 KB (document storage)         |
| Pad   | `pm-pad`, `ghcr.io/perpetualsoftware/pad:0.18.1` (digest-pinned)                                 | `pad_pad_data`           | 1.8 MB      | `pad.db` 1.7 MB SQLite (WAL mode; `pad.db-wal` 0 B at read time), plus `encryption.key`, `config.toml`, `attachments/` (~empty) |

Deployment definitions: `pm-stack/compose.yml` (Kaneo + Postgres),
`pm-stack/grist/compose.yml`, `pm-stack/pad/compose.yml`. Public reach is
unchanged by this plan: Kaneo at `https://kaneo.security.cab` via the
Cloudflare Tunnel; Grist and Pad loopback + tailnet only. **The Cloudflare
Access hostname and policy do not change in any step.**

## 2. Replacement target decision — 2026-11-24

**Decided by lane `any-o`, not attributed to an owner ruling:** use native
stores on the basement cluster, with exactly one active application writer
per store. This supersedes this plan's original Supabase target, not its
recovery objectives. Rationale: `muse-dev2` was deleted, Turso cannot host
Kaneo's PostgreSQL database natively, and file copies in object storage do
not themselves provide an independent Grist/Pad runtime. See the
[current-state reconciliation](../doc/pm-stack/current-state-reconciliation-2026-11-24.md#dead-references-this-note-supersedes-do-not-follow-without-the-re-plan).
The [original researched native-store proposal](../doc/2026-11-01T21-12-00Z_pm-stack-durability-host-independence/2026-11-01T21-12-00Z_pm-stack-durability-host-independence.md#store-inventory-and-proposed-homes)
provides the format-preserving alternative; this is a **new post-deletion
decision**, not a claim that that older proposal was deployed.

- **Kaneo → cluster PostgreSQL**, dedicated database `kaneo` and dedicated
  application role, isolated from aggregator tables. Gate identity:
  `cluster-postgres`, with an explicit endpoint matching the target DSN.
  Do not reuse a retired Supabase secret or silently fall back to Turso.
  Retire the local `pm-stack-postgres-1` only after the rollback window.
- **Grist → cluster persistent storage**, mounted at the sole Grist
  instance's `/persist`: `home.sqlite3` and `docs/` remain native files.
  Session data is runtime state, not an independent canonical home.
- **Pad → separate cluster persistent storage**, mounted at the sole Pad
  instance's `/data`: preserve `pad.db`, attachments, `config.toml`, and
  `encryption.key`. The key must also be recoverable through the secret
  flow; a database-only restore is insufficient.

These are **intended homes, not provisioned or verified endpoints**. No
cluster storage class, volume, database endpoint, or running remote app is
asserted here. The repo still declares laptop canonical volumes (reconciliation
§ Current homes); the machine registry does not yet declare a basement node
ready. Provision and verify the selected storage and runtime before a
window; record their real identities in the ownership map, not an invented
hostname in this plan. The preflight gate only checks configuration.

Consume #1514's local + R2 + Drive backup path and retention, do not replace
it with a second sync schedule. The 4 h core RTO, 1 h Kaneo RPO, and 24 h
Grist/Pad RPO remain targets: a daily dump alone cannot prove 1 h Kaneo RPO.
Provision PITR or verified hourly-or-better recovery points and measure the
restore before claiming the RPO. Undoing this decision requires a new target
decision, gate changes/tests, and a revised ownership map before data moves.

## 3. Single-writer fencing (explicit, per store)

Rule from #6816 acceptance: **the old writer is fenced before any failover
promotion; no configuration permits two active writers.**

- **Kaneo:** fence = stop `pm-stack-kaneo-1` (the only writer; Postgres
  itself accepts writes only through Kaneo's role in practice, and the local
  Postgres is never host-exposed). Promotion = point the Kaneo DSN at
  cluster PostgreSQL and start Kaneo. The local Postgres stays read-only evidence —
  start it only with Kaneo stopped, or with its port/volume mounted
  read-only for inspection. Two Kaneo instances never run at once: the
  compose service is the single instance, and the cutover stops it before
  the new instance starts.
- **Grist:** fence = stop `pm-grist` before the final sync of
  `home.sqlite3`/`docs/` and before any restore/promotion. SQLite has no
  write arbitration across hosts — process-stopped is the fence. A restored
  Grist starts only after the laptop instance is confirmed stopped.
- **Pad:** fence = stop `pm-pad` and confirm a clean WAL checkpoint
  (`pad.db-wal` empty or checkpointed into `pad.db`) before the final copy.
  Same single-instance rule as Grist.

## 4. Cutover sequence (later windowed steps — not this task)

1. Provision the §2 homes and independent runtimes first. Run the offline
   Kaneo target preflight below, then pre-sync while live: Kaneo logical
   dump (`pg_dump` of `kaneo`) loaded into the cluster target and schema
   count verified; Grist/Pad file sync to their mounted persistent homes
   (everything except the final delta). A live SQLite copy is staging only,
   never a recovery point until a fenced final copy passes integrity checks.
2. Announce the window; stop the writers in order Pad, Grist, Kaneo
   (smallest blast radius first, Kaneo last because it is the public UI).
3. Final delta: Kaneo dump-diff or dump/reload (80 MB — minutes), Grist/Pad
   final file sync with checksums.
4. Verify: Kaneo table counts + a read-only UI smoke; Grist home document
   list plus SQLite `PRAGMA integrity_check` against the fenced final
   `home.sqlite3` copy; Pad DB integrity check (`PRAGMA integrity_check`) on
   a copy. A failed integrity check refuses promotion.
5. Promote: Kaneo DSN → cluster PostgreSQL, start Kaneo; Grist/Pad start
   against their cluster persistent homes. Cloudflare hostname and Access
   policy stay unchanged; validate the origin route and Access separately.
6. Rollback window: keep the laptop volumes untouched and the old config
   for 7 days (see §5).

### The gate: `pm-stack/scripts/kaneo-cutover.sh`

Steps 1 and 5 run through one driver, so the rule "no configuration permits
two active writers" is mechanical instead of advisory:

- All modes require `--target-home cluster-postgres --target-host HOST`,
  where `HOST` matches the PostgreSQL URI in `--target-dsn-file`. Start with
  `--mode preflight --source-dsn-file FILE --target-dsn-file FILE` and those
  identity flags. Preflight is offline: no database, fence, backup, dump,
  or receipt operation. A pass means **configuration accepted**, not target
  reachability, persistence, backup readiness, or permission to promote.
  The same validation runs before each operational mode. Only the driver's
  documented unambiguous URI subset is accepted; conninfo/service indirection
  is not supported. Supabase endpoints are refused wholesale because the
  deleted project's friendly name does not identify its endpoint reliably.
- `--mode presync` (step 1, while live): `pg_dump` the source `kaneo`
  database, `pg_restore` it into the target, and require **schema parity**
  (same public table count). Row drift is reported, not failed — the final
  delta has not landed yet.
- `--mode promote` (step 5, the gate): refuses unless the latest backup
  record (`--backup-record-file`) describes a complete generation with a
  valid archive identity and verified local, R2, and Drive copies, finished
  no more than 36 hours ago by default (`--max-backup-age-hours` can set a
  stricter window). That 36-hour archive bound is not sufficient to claim Kaneo's 1-hour RPO: promotion additionally requires a separately verified Kaneo recovery point no older than 1 hour, or the window record must report that RPO as unmet and must not describe the cutover as meeting it. `pm-stack/scripts/fence-check.sh` reports **FENCED** (fail
  closed on anything unreadable) **and** the target's exact table count and row
  count equal the source's. A refused promotion touches no store. Only a pass
  writes a cutover receipt (fence verdict, both counts, verdict line) with
  no DSN in it — the operator attaches it to the window record.

DSNs enter the driver through files (`--source-dsn-file`, `--target-dsn-file`)
and are excluded from its receipt. The existing PostgreSQL subprocess calls
still pass the DSN as `-d` argv; file input is **not** a promise that credentials
never enter a subprocess argument. Suites:
`pm-stack/scripts/kaneo-cutover.test.sh` (driver gates) and
`pm-stack/scripts/fence-check.test.sh` (fencing rule), both run by
`.github/workflows/pm-stack-tests.yml`.

## 5. Rollback

- **Kaneo:** stop Kaneo, restore the local DSN, start the local stack. Any
  writes made on cluster PostgreSQL during the window are exported (`pg_dump` of the
  target) and replayed into the local DB before reopening, so rollback
  restores the prior writer **without losing writes** (#6816 acceptance).
  Driver: `pm-stack/scripts/kaneo-cutover.sh --mode rollback` — refuses
  unless the promoted writer is fenced, replays the target delta into the
  source, demands exact table/row parity, and writes a receipt with no
  DSN. The DSN flip and restarts stay window operator steps; the first
  window records a run.
- **Grist / Pad:** stop and fence the promoted instance before touching the
  old volume. Do not bidirectionally sync SQLite files or treat checksum
  parity as a merge: divergent writes require a recorded, operator-reviewed
  reconciliation. For a straightforward rollback, preserve a copy of the
  promoted home, verify SQLite integrity (and Pad's WAL/checkpoint state),
  restore the selected authoritative generation to the old volume, verify
  checksums and application readability, then reopen only the single old
  writer. Where reconciliation is needed, keep both writers stopped until
  the selected data and recoverable writes are verified; neither volume is
  disposable during this process.
- Rollback is abandoned (volumes eligible for cleanup) only after 7 stable
  days and a recorded drill per the cold-restore runbook.

## 6. What remains after this step

- Provisioning/rehearsal slice: create the selected cluster homes and
  independent runtimes, verify actual endpoint/volume identities and backup
  cadence, then rehearse from off-machine copies with the source fenced.
  An offline preflight does not complete criterion 1 of #6816.
- Step 2 (windowed): the cutover itself, per §4 — run `preflight`, `presync`, then the
  fenced writers, then `promote` (the gate in §4 names the driver and what
  it refuses). Rollback (§5) is driven by `kaneo-cutover.sh --mode
rollback` (fence-gated replay with parity receipt, stub-tested); the
  DSN flip, restarts, and the first window run remain operator steps.
- Step 3 (windowed): the first isolated laptop-off drill, recorded per
  `doc/pm-stack/COLD-RESTORE-RUNBOOK.md` with measured RTO/RPO against the
  #1258 targets (4 h core RTO; 1 h Kaneo RPO; 24 h Grist/Pad RPO).
