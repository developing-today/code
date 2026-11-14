# Store migration plan — Kaneo / Grist / Pad canonical stores to Supabase-backed homes

Issue: #6816 (step 1 — inventory and plan only). Parent: #3989. Rulings: #1258
(2026-11-03). Backups routine: #1514; empty-generation handling: #3869.

**This step moves no data and reconfigures no service.** It inventories the
three laptop-only canonical stores, names the Supabase `muse-dev2` target
shape for each, and fixes the single-writer fencing, cutover sequence, and
rollback for the later, windowed steps.

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

## 2. Target shapes on Supabase `muse-dev2`

- **Kaneo → Supabase Postgres, native.** Kaneo already speaks Postgres; the
  target is a dedicated schema/database role on `muse-dev2` holding the same
  46 tables. Kaneo's connection moves from the local container DSN to the
  Supabase connection string (secret by name only:
  `supabase.muse-dev2.*` via the standing secrets route — never in this repo).
  The local `pm-stack-postgres-1` container is retired only after the
  rollback window (§4) expires.
- **Grist → Supabase-backed home, file-shaped.** Grist's canonical store is
  files (SQLite + `docs/`), not a Postgres schema. Target home: the Grist
  data directory on a Supabase-backed sync home — `home.sqlite3` and `docs/`
  replicated to `muse-dev2` Storage with a scheduled sync, and the location +
  sync shape recorded in the ownership map on #3989 (acceptance on #6816).
  Grist keeps its file runtime; the _canonical, restorable copy_ stops being
  laptop-only.
- **Pad → Supabase-backed home, file-shaped.** Same pattern as Grist:
  `pad.db` (SQLite, WAL) plus `encryption.key` and `config.toml` replicated
  to `muse-dev2` Storage on the #1514 backup cadence. The encryption key is
  additionally restorable by name from Bitwarden — a Storage copy alone is
  not a restore if the key is lost with the host.

## 3. Single-writer fencing (explicit, per store)

Rule from #6816 acceptance: **the old writer is fenced before any failover
promotion; no configuration permits two active writers.**

- **Kaneo:** fence = stop `pm-stack-kaneo-1` (the only writer; Postgres
  itself accepts writes only through Kaneo's role in practice, and the local
  Postgres is never host-exposed). Promotion = point the Kaneo DSN at
  Supabase and start Kaneo. The local Postgres stays read-only evidence —
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

1. Pre-sync while live: Kaneo logical dump (`pg_dump` of `kaneo`) loaded
   into the `muse-dev2` target and row-count verified; Grist/Pad file sync
   to Storage (everything except the final delta).
2. Announce the window; stop the writers in order Pad, Grist, Kaneo
   (smallest blast radius first, Kaneo last because it is the public UI).
3. Final delta: Kaneo dump-diff or dump/reload (80 MB — minutes), Grist/Pad
   final file sync with checksums.
4. Verify: Kaneo table counts + a read-only UI smoke; Grist home doc list;
   Pad DB integrity check (`PRAGMA integrity_check`) on a copy.
5. Promote: Kaneo DSN → Supabase, start Kaneo; Grist/Pad start against the
   Supabase-backed home. Cloudflare Tunnel/Access untouched.
6. Rollback window: keep the laptop volumes untouched and the old config
   for 7 days (see §5).

## 5. Rollback

- **Kaneo:** stop Kaneo, restore the local DSN, start the local stack. Any
  writes made on Supabase during the window are exported (`pg_dump` of the
  target) and replayed into the local DB before reopening, so rollback
  restores the prior writer **without losing writes** (#6816 acceptance).
- **Grist / Pad:** stop the promoted instance, point the data directory
  back at the untouched laptop volume, restart. Writes made in the
  Supabase-backed home sync back before the restart — the sync is
  bidirectional-checked by checksum at cutover and rollback alike.
- Rollback is abandoned (volumes eligible for cleanup) only after 7 stable
  days and a recorded drill per the cold-restore runbook.

## 6. What remains after this step

- Step 2 (windowed): the cutover itself, per §4.
- Step 3 (windowed): the first isolated laptop-off drill, recorded per
  `pm-stack/COLD-RESTORE-RUNBOOK.md` with measured RTO/RPO against the
  #1258 targets (4 h core RTO; 1 h Kaneo RPO; 24 h Grist/Pad RPO).
