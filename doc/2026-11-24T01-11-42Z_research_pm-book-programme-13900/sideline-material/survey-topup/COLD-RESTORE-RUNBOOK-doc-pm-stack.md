# Cold-restore runbook — pm-stack

Status: **operative for the pre-cutover shape; rehearsal-proven 2026-11-22**
(see `restore-drills/2026-11.md`). Prepared by task 11415 for issue #6816
step 1; moved from `pm-stack/COLD-RESTORE-RUNBOOK.md` to this
`doc/pm-stack/` home by the 2026-11-22 rehearsal slice, which is where
#6816's acceptance criteria name it. The post-cutover source order (step
"Restore sources" item 1) takes over when the #6816 cutover lands; the
first isolated **laptop-off** drill ran 2026-11-23 as a simulation
(off-machine sources only, isolated namespace on the laptop's Docker —
no second Docker host exists on this seat; see the drill record).
Design record:
`doc/2026-11-01T21-12-00Z_pm-stack-durability-host-independence/` (#1258).
A repeatable rehearsal driver lives at
`pm-stack/scripts/restore-rehearsal.sh`. The cutover that precedes a
post-cutover restore is gated by `pm-stack/scripts/kaneo-cutover.sh`
(plan §4): promotion is refused unless `pm-stack/scripts/fence-check.sh`
reports FENCED and the target's table/row counts equal the source's.

Failover posture (Drew's #1258 rulings, 2026-11-03): all PM-stack state to
Supabase `muse-dev2`, nothing laptop-only; **tested cold restore, no warm
standby**; backups local + R2 + Drive, daily 30 d / monthly 12 mo (routine
owned by #1514, empty-generation handling by #3869 — this runbook consumes
those backups, it does not redefine them).

## Targets (#1258)

| Scope                         | RTO (restore)   | RPO (data loss) |
| ----------------------------- | --------------- | --------------- |
| Core PM stack available again | 4 h             | —               |
| Kaneo (Postgres)              | within core 4 h | 1 h             |
| Grist                         | within core 4 h | 24 h            |
| Pad                           | within core 4 h | 24 h            |

A drill that misses a target is recorded as a miss with the gap named —
targets are never claimed until verified by a drill.

## Restore sources, in preference order

1. Supabase `muse-dev2` itself (post-cutover canonical home): Kaneo data in
   Postgres; Grist/Pad canonical copies in Storage.
2. Off-machine backups from the #1514 routine: R2, then Google Drive.
3. The laptop volumes (`pm-stack_postgres_data`, `grist_grist_data`,
   `pad_pad_data`) — **only if the laptop is alive**; a laptop-off restore
   never depends on them.

Secrets are restored **by name** from Bitwarden via the standing secrets
route; no backup archive contains plaintext secrets. A new computer first
bootstraps its roots of trust (BWS token, Google Drive OAuth) per
[`BOOTSTRAP-CREDENTIALS.md`](BOOTSTRAP-CREDENTIALS.md).

## Drill / restore steps

Times are UTC. One operator drives; a second person (or a later operator)
verifies. Record start time at step 1 — RTO is measured from the declared
loss, not from when work got convenient.

1. **Declare.** Note the loss time and scope (whole host / single store).
   Fence the old writer if the old host is reachable at all: stop
   `pm-stack-kaneo-1`, `pm-grist`, `pm-pad`. If unreachable, state that —
   an unreachable host is fenced by definition, and it must be fenced
   (services disabled) before it is allowed to rejoin later.
2. **Provision the restore host.** Any clean host with Docker + this repo:
   clone `developing-today/code`, check out the commit the plan names.
3. **Kaneo.** Restore the `kaneo` database into the target Postgres
   (post-cutover: verify the `muse-dev2` schema is the live one and skip
   to step 6; pre-cutover or Supabase-loss: `pg_restore` the newest #1514
   dump whose timestamp is inside the 1 h RPO, or the newest available —
   record which). Note: `globals.sql` installs the production `kaneo`
   role's password hash (one `ALTER ROLE … PASSWORD` statement), so
   after restoring globals the app can only authenticate with that
   password — set the restored role's password from Bitwarden by name
   on a real restore, or reset it inside the drill database on a drill
   (found by the 2026-11-23 drill, run 1: app boot failed 28P01 until
   the reset). Verify: user tables and row counts spot-checked against
   the backup manifest (2026-11-22 rehearsal measured 45 public tables,
   ~90 k live rows; the 46-table figure in earlier notes dates from the
   2026-11-10 inventory). Note the daily #1514 cadence cannot meet the
   1 h Kaneo RPO pre-cutover — worst case is ~24 h; the 1 h target is
   only reachable once Kaneo's canonical home is Supabase (#6816
   criterion 1). Record the gap, never paper over it.
4. **Grist.** Restore `home.sqlite3`, `grist-sessions.db`, and `docs/` into
   the Grist data directory from the canonical Storage copy or the newest
   backup inside the 24 h RPO. Verify: `PRAGMA integrity_check` on
   `home.sqlite3`; doc count matches the manifest.
5. **Pad.** Restore `pad.db`, `config.toml`, and `encryption.key` (key by
   name from Bitwarden if the archive copy is unavailable). Verify:
   `PRAGMA integrity_check` on a copy of `pad.db` before first start.
6. **Start and smoke.** Deploy per `pm-stack/README.md` (compose for
   Kaneo/Postgres pre-cutover shape, `grist/compose.yml`, `pad/compose.yml`).
   Smoke: Kaneo UI loads and shows current boards; Grist opens the roadmap
   doc; Pad serves its UI. Kaneo's app container needs `DATABASE_URL`
   **and** `AUTH_SECRET` (from the rendered stack env, by name from
   Bitwarden), and its `DATABASE_URL` password must be the role
   password as restored by `globals.sql` (see step 3) — with
   `DATABASE_URL` alone the API process exits
   (found by the 2026-11-22 rehearsal; with both set, `/api/health`
   returns 200 against the restored database). The Cloudflare Access hostname and policy are
   re-pointed only at the tunnel step of a real host move — a drill uses an
   **isolated namespace / non-public route** and never touches the live
   hostname.
7. **Measure.** Record end time; compute measured RTO (declare → smoke
   pass) and RPO (loss time → newest restored record per store) against
   the table above.
8. **Record.** Write the drill report (template below) to
   `doc/pm-stack/restore-drills/YYYY-MM.md`, link it from #6816 and the backup
   tracker (#1514), and file follow-up issues for every gap. A drill with
   no record did not happen.

## Drill record template (`doc/pm-stack/restore-drills/YYYY-MM.md`)

```markdown
# Cold-restore drill — YYYY-MM-DD

- Loss declared: <UTC> · Restore host: <host, isolated route>
- Sources used: <muse-dev2 / R2 / Drive + archive digests + timestamps>
- Kaneo: restored to <timestamp> — RPO measured <…> (target 1 h)
- Grist: restored to <timestamp> — RPO measured <…> (target 24 h)
- Pad: restored to <timestamp> — RPO measured <…> (target 24 h)
- Smoke passed: <UTC> — RTO measured <…> (target 4 h core)
- Integrity checks: <table counts / integrity_check results>
- Gaps and follow-up issues: <#…>
```

Never include credentials, raw private records, or copied archives in the
report.
