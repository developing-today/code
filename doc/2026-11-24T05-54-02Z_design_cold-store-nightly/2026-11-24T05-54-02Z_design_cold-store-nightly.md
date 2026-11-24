# Cold-store nightly backup: as-built design and operating contract

- **Date:** 2026-11-24
- **Status:** As-built design record; the nightly job is shipped. This records the implementation and its contracts, not a proposal to deploy or migrate data.
- **Tracking:** developing-today/code#8833; queue task 21665; implementation lineage includes #1723 and the changes cited inline in the source.
- **Scope:** `pkgs/cold-store-reaper`'s nightly D1/Turso backup path and its laptop systemd schedule. The broader Drive/R2 layout and bundling policy in [`doc/living/cold-store-r2-layout-design-2026-10-25.md`](../living/cold-store-r2-layout-design-2026-10-25.md) is a separate design; it is not evidence that this nightly path bundles backups or copies them to R2.

## 1. Problem and context

The nightly backup was vulnerable to a single failed export starving later retention and manifest steps, and transient or truncated Turso `/dump` responses could produce incomplete or unfiled exports. Operational evidence recorded in the scripts includes a 2026-10-27 Turso failure that skipped later steps, a 2026-11-20 timeout that left other databases unfiled, and a 2026-11-22 systemd-journal gap that made the failed stage unprovable. These are source comments documenting incidents, not a claim that a live run was checked for this record.

The shipped job creates dated, unbundled Google Drive backup files for the cold-store-manifest D1 database and discovered Turso databases, then applies retention and syncs its local manifest to D1. Supabase is a retired/legacy readable backup family, not a nightly source. The active scope is declared by `BACKUP_DBS = ("turso", "d1")` and `LEGACY_BACKUP_DBS = ("supabase",)` in `pkgs/cold-store-reaper/cold_store.py`.

## 2. Goals and non-goals

### Goals

- Run the nightly backup on the laptop on its configured daily 03:12 schedule.
- Produce separately dated Drive files for D1 and each discovered Turso database; verify an uploaded file's size and MD5 against the local artifact before adding it to the transfer ledger.
- Keep the run diagnosable and failure-loud while attempting later steps after a failure.
- Bound and verify Turso dump reconstruction; retain backups under explicit daily/weekly/monthly policy and sync manifest state to D1.
- Keep credentials out of the repository and out of routine log output.

### Non-goals

- Back up Supabase in the nightly job, copy nightly backups to R2, or bundle the database files into 4–16 GB cold-store bundles.
- Implement the future multi-class R2/Drive migration, manifest database, inventory, or lifecycle policy described in the separate R2 layout design.
- Claim a restore service-level objective or a recent successful live backup/restore. This record captures source and test evidence; it does not perform live operations.

## 3. Alternatives and trade-offs

| Option | Trade-off | Disposition |
| --- | --- | --- |
| Do nothing / keep the former coupled run | Smallest change, but one export failure can skip pruning, manifest sync, and status; a missing journal record weakens diagnosis. | Rejected; the current orchestrator isolates steps and writes a durable run log. |
| Stop after the first failed step | Makes the overall failure obvious but leaves independent maintenance and status work undone. | Rejected; attempt every step and return non-zero at the end if any failed. |
| Bundle nightly DB backups using the R2-layout design | Aligns with the proposed archive model, but adds a different packaging/index/restore pipeline and is not what the shipped job does. | Not selected for this shipped path; future change requires its own design and migration/restore proof. |
| Keep direct, unbundled dated Drive backups | Simple per-database restore points and existing `put-backup` verification/retention; Drive listing and upload limits remain relevant. | Selected and implemented. |

## 4. Chosen design and contracts

### Flow and interfaces

`cold-store-nightly.timer` invokes the laptop wrapper daily at `03:12`; the wrapper calls the repository's `pkgs/cold-store-reaper/scripts/nightly-backup.sh`. The job runs, in order: `cold-store backup-d1`; optional `scripts/export-turso.sh`; `cold-store prune-backups --apply`; `cold-store sync-manifest`; and `cold-store status` (displaying at most 20 lines). Every step is attempted. A guarded Turso-token absence is a visible skip, not a failure; any actual failed step makes the overall exit non-zero after the remaining work is attempted.

The Turso exporter discovers databases from the Turso Platform API, obtains per-database tokens, fetches each database's libsql `/dump`, applies the repair/dedupe/vector-index filters, requires a terminal `COMMIT;`, rebuilds with the `sqlite3` CLI, checks `PRAGMA integrity_check = ok` and at least one user table, gzips, then calls `cold-store put-backup turso`. Download failures can be retried within the run deadline; deterministic later-stage failures are reported per database. The exporter requires `sqlite3`; it has no in-memory rebuild fallback.

### Data and storage model

- Active backup families: `d1` and `turso`; legacy `supabase` files remain listable/retrievable and subject to retention if that old folder exists, but the folder is not created for new backups.
- Drive files are unbundled under `db-backups/<family>/`, named `<family>-<UTC timestamp>-<source filename>`.
- The local manifest is SQLite (`manifest.sqlite`); its schema includes `objects`, `bundles`, `members`, `transfer_ledger`, and `meta`. Nightly sync sends the manifest tables to the configured Cloudflare D1 database.
- Backup uploads are size-limited before transfer; Drive-reported size and MD5 are checked before a `backup-upload` ledger entry is recorded. Retention trashes files rather than permanently deleting them and records prune activity in the ledger.
- The retention target stated by the implementation is 30 daily, 12 weekly, and 12 monthly restore points. For Turso, retention is per source database, not across the family.

### Security and failure boundaries

- Drive OAuth material is read from `~/.config/cold-store/credentials.json`; the file is created with private permissions. Cloudflare API and optional Turso Platform tokens are supplied through the environment by the operator/cron. The scripts refer to Bitwarden item names, never embed live credentials.
- The Turso scratch directory is mode 0700 and serialized with `flock`; minted database-token response files are removed after each database. Tokens are not intentionally printed.
- A failed step does not suppress later steps. A Turso database failure does not suppress other database exports. A failed/invalid rebuilt dump is not filed as a backup. An upload whose Drive metadata verification fails is trashed where possible and is not ledgered as a verified backup.
- Failure logs can identify the failed step/database and may expose operational metadata. The durable log is under the cold-store home; if log-directory creation fails, the backup still runs but emits a warning and has no durable run log.

## 5. Dependency-ordered shipped slices and acceptance

These slices reconstruct the shipped dependency order; they are not new work assignments.

1. **Local manifest and Drive backup primitive.** `cold_store.py` owns credentials, local SQLite manifest, Drive folders, dated `put-backup`, upload-size checks, read-back metadata verification, and ledger writes. **Accept when:** focused cold-store tests prove successful upload verification is ledgered and size/hash mismatch is rejected without a ledger success (`python3 pkgs/cold-store-reaper/tests/test_cold_store.py`).
2. **D1 backup and manifest sync.** Export the cold-store D1 database and sync manifest rows to D1. **Accept when:** the D1 backup/sync tests pass, and sync handles bounded batches and configured transient 429/503 retries (same focused test module).
3. **Turso discovery and verified export.** Discover databases, rebuild complete dumps with the repair filters, verify SQLite integrity, and file each result. **Accept when:** `python3 -m unittest discover -s pkgs/cold-store-reaper/tests -v` passes the exporter/filter tests; incomplete dumps and invalid rebuilt databases fail closed.
4. **Nightly orchestration, retention, and durable evidence.** Run all stages independently, prune according to policy, sync, and log. **Accept when:** `python3 pkgs/cold-store-reaper/tests/test_nightly_backup.py` passes both all-success and injected-failure cases, including later-step execution and durable-log assertions.
5. **Laptop schedule and freshness monitoring.** Configure the timer and watchdog checks. **Accept when:** the Nix unit configuration and registry agree on daily `03:12`, and watchdog tests detect stale timer, failed result, or missing/unreadable manifest evidence. The registry records the configured schedule; this design record does not assert a live host's current timer state.

## 6. Rollout and rollback

The shipped rollout is the laptop systemd user timer invoking the wrapper; the schedule registry records `cold-store-nightly.timer` with `OnCalendar=*-*-* 03:12:00`. No new rollout is authorized by this record. Keep the configured schedule and credentials as-is unless a separate change owns them.

If the job regresses, disable or pause the timer through the owning NixOS configuration, preserve its durable logs and the last verified Drive backups, and restore the timer only after the failing stage's targeted tests and an observed manual run pass. Do not delete backup files to roll back code. If Turso export is unavailable, its unset-token behavior permits the other stages to run, but the visible skip means Turso coverage is absent and must not be represented as a complete backup set.

## 7. Verification plan and evidence

- **Automated:** `python3 pkgs/cold-store-reaper/tests/test_nightly_backup.py` covers ordering, non-starvation after failure, failure exit status, output truncation, durable run log, and the no-log-directory fallback. `python3 -m unittest discover -s pkgs/cold-store-reaper/tests -v` covers the package's cold-store and Turso export/filter behavior.
- **Configured schedule:** `doc/fleet-schedule-registry.md` lists `cold-store-nightly.timer` at `*-*-* 03:12:00`; `nixos/home/laptop-backups.nix` defines the unit and wrapper. These are repository configuration evidence, not a live timer read.
- **Per-run evidence:** inspect `${COLD_STORE_HOME}/logs/nightly-<timestamp>.log` (or `nightly-latest.log`) for the per-step outcomes and final verdict. A successful script exit alone does not prove a database restore.
- **Restore verification still needed as operational evidence:** choose a dated Drive backup, verify its bytes/hash, restore it to an isolated target, check schema/application-level invariants, and record destination, elapsed time, and results. Do not use a production target for the first restore test.

## 8. Open questions (named decider)

These are follow-on policy choices, not blockers to documenting the shipped behavior. **Decider for each: Drew.** Defaults below preserve the current safe behavior until a separate design or issue changes it.

1. **Do Turso backups need independent R2 copies?** Default: no; retain the shipped Drive path until a class-specific design proves a need and restore path.
2. **Should any part of the nightly DB path adopt bundles or the broader R2 layout?** Default: no; the current simple unbundled backups remain distinct from the proposed 4–16 GB bundle architecture.
3. **What are the restore-time targets and drill cadence for D1/Turso?** Default: no RTO claim until a measured scratch restore and an explicit target are recorded; use the existing cold-store drill as the operational mechanism where applicable.
4. **What is the operational response to a visible Turso-token skip?** Default: treat it as incomplete Turso coverage, alert through the existing job result/watchdog path, and do not call the run a complete database backup.

## 9. Record and tracking

This dated record is filed in `doc/` and indexed from [`doc/README.md`](../README.md). Tracking home: [developing-today/code#8833](https://github.com/developing-today/code/issues/8833); queue item 21665 carries the delivery. The scheduled job is registered in [`doc/fleet-schedule-registry.md`](../fleet-schedule-registry.md).

### Source map

- `pkgs/cold-store-reaper/scripts/nightly-backup.sh` — step order, isolation, required/optional environment, durable logging, final exit semantics.
- `pkgs/cold-store-reaper/scripts/export-turso.sh` — discovery, token/scratch handling, bounded retries, dump repair/validation/rebuild, filing behavior.
- `pkgs/cold-store-reaper/cold_store.py` — active and legacy backup families, Drive storage, manifest, limits, read-back verification, retention, D1 backup/sync.
- `pkgs/cold-store-reaper/tests/test_nightly_backup.py` and `pkgs/cold-store-reaper/tests/test_cold_store.py` — automated oracles.
- `nixos/home/laptop-backups.nix` and `doc/fleet-schedule-registry.md` — configured timer and registry row.
- `doc/living/cold-store-r2-layout-design-2026-10-25.md` — separate broader R2/Drive layout proposal, used here only to make the implementation boundary explicit.
