# Backup, restore and the census

Source: `nixos/home/laptop-backups.nix` and `nixos/home/atlas-backup.sh`, the schedule registry row in `doc/fleet-schedule-registry.md`, and the census statement in the Atlas program investigation record `doc/2026-11-23T12-06-51Z_design_atlas_program_investigation/` under https://github.com/developing-today/code/blob/main/nixos/home/laptop-backups.nix — provenance only; this chapter is complete without following it.

Atlas tracks backup completion, corpus row counts and publication freshness separately. Each observation below identifies the store or service and the date.

## The backup: `atlas-backup`

The wiring lives in **`nixos/home/laptop-backups.nix`**, with the script vendored beside it as `nixos/home/atlas-backup.sh` and packaged as `atlas-backup` (`pkgs.writeShellScriptBin`). It was migrated from a sandbox cron on 2026-10-30 under the laptop-only execution rule (#1515).

| Unit | Shape |
|---|---|
| `atlas-backup.service` | systemd **user** service, `Type = oneshot`, `ExecStart` = the packaged `atlas-backup`, `TimeoutStartSec = 90m`, description `Daily atlas.db backup - local + R2 + Drive (code#1515)`. Gated on `cfg.atlas.enable` |
| `atlas-backup.timer` | systemd **user** timer, `OnCalendar` from the option `services.laptop-backups.atlas.onCalendar`, **default `04:47`**, `Persistent = true`, `RandomizedDelaySec = 15m`, `WantedBy = [timers.target]` |

The fleet schedule registry corroborates the live shape: `atlas-backup.timer` at `OnCalendar=*-*-* 04:47:00` driving `atlas-backup.service` on the laptop (user systemd), dated 2026-11-04. Because the timer is persistent, a night the laptop is asleep is caught up on wake rather than skipped. Note what this timer is not: it is the **only** laptop schedule in the Atlas story — ingest has no timer; see the census section below.

### What is covered

The backup target is the canonical local store, **`~/atlas/turso-atlas/data/atlas.db`** — the durable asset of the collect → normalize → index pipeline, from which the published copies are derived. One run:

1. Snapshots `atlas.db` with `sqlite3 .backup` (a consistent copy of a live database, not a file copy).
2. Gzips the snapshot deterministically.
3. Files it to three places: the local backup directory on the laptop, R2 (bucket `supabase-backups`, prefix `atlas/&lt;date&gt;/`), and Google Drive (`Cloud/cold-store/db-backups/local-atlas/`).
4. Verifies **every copy by sha256**, and dedupes by content hash against the backup log — an unchanged database does not produce duplicate archives.
5. Applies retention of **7 / 7 / 4** (laptop dates / R2 dates / Drive archives).

Credentials are fetched from Bitwarden **by name at runtime**, never written into the Nix store — the module's shared pattern. The Python `atlas-backup.py` in the laptop script tree is the script-side ancestor of this unit, not the live shape; the Nix unit above is.

Also covered, through the same store: the content-addressed extracts of the provenance programme (#3125). Phase 1 landed 2026-11-22 and phase 2 landed 2026-11-23, interning **12,092 content-addressed objects (about 2.6 GB, 0 dedupe collisions)** for the chatgpt-export and sandbox-agents sources, with the verification marker **`ATLAS-EXTRACTS VERIFY OK`** recorded — and the programme record states plainly that atlas-backup covers that store. Phases 3 (occurrence backfill) and 4 (catalog database and multi-shard resolution as MCP tools) remain open.

What the backup is **not**: it is not a backup of the Turso-hosted publish (Turso is distribution — it never holds the only copy of anything, so it is the copy, not the source), not a backup of the service temp store or the Worker cache (both are caches with their own lifetimes; see [Running the service and the Worker](services.md)), and not a `fleet search` verb — a backup verb or maintenance role inside the fleet binary was proposed in the scaler design and has not landed as such. The Nix unit is the live shape.

### Restore shape

There is no restore verb. A restore is the backup read backwards, by hand, and its shape follows from what a backup is — a sha256-verified, gzipped `sqlite3 .backup` snapshot:

1. Stop the reader: `systemctl --user stop atlas-search.service`, so nothing serves a half-replaced store.
2. Pick the copy: the local backup directory first, then the R2 copy under `atlas/&lt;date&gt;/`, then the Drive archive under `Cloud/cold-store/db-backups/local-atlas/`.
3. Verify before trusting: check the copy's sha256 against the backup log. A copy that does not verify is not a restore source.
4. Decompress and place the result at the canonical path, `~/atlas/turso-atlas/data/atlas.db`. Keep the file it replaces until the restored store has been checked — the replaced file is the only thing a failed restore can fall back to.
5. Restart the service and check it: `GET /health` on port 18760 answers, and a known query returns sane results. The hosted publish, if it also needs rebuilding, is republished from this restored canonical store — never the other way round.

Two boundaries to keep straight. First, restoring `atlas.db` restores the canonical store; the shard set and the hosted publish are derived or distributed state with their own provenance (their manifests are described in [Data stores and schemas](../reference/data-stores.md)). Second, the cold-tier shard restore path (#6042) is a design record, not this procedure — do not present the manual restore above as the cold-tier mechanism.

## The census: where the numbers live

### The frozen snapshot: 2026-10-31

Issue **#3125** is the provenance and occurrence-registry issue. Its census does not live in a standalone census file in `doc/` — it is **quoted inside later design records**, and the canonical statement is section 1 of the program investigation record (2026-11-23):

> Last verified census (2026-10-31, on #3125): **18,325 sessions / 122,124 messages / 107,639 chunks.**

The October record also describes 29 topic shards of that corpus. Shard copies must be reconciled separately from the canonical count. Its sensitivity split, from the same record, cross-checks the total exactly:

| Tier | Shards | Chunks |
|---|---|---|
| Internal | 28 | 107,001 |
| Secret | 1 | 638 |
| Public | 0 | 0 |
| **Total** | **29** | **107,639** |

The split above belongs to the October snapshot.

### November 23 AMD observation

Separate SQLite read transactions on November 23, 2026 counted physical table rows in these local stores. A repeat read of the canonical database at 17:57:15 UTC reproduced the 17:56:17 UTC result. Sizes are observed database-file bytes.

| Local store | Sessions | Messages | Chunks | File bytes | SQLite schema version |
|---|---:|---:|---:|---:|---:|
| Canonical `~/atlas/turso-atlas/data/atlas.db` | 18,325 | 122,115 | 107,439 | 1,492,127,744 | 14 |
| Dated publish-source file `atlas-publish-v2-2026-10-31.db` in the same directory | 18,325 | 122,124 | 107,639 | 1,704,194,048 | 12 |

The canonical table counts differ from the October record by 0 sessions, −9 messages and −200 chunks. The census cannot determine which changes produced those differences. At 17:58:34 UTC, the canonical CAS registry contained 24,184 rows and 12,092 distinct hashes. The named Snowflake vector table was absent from both files. The dated file contained 107,639 rows in its legacy BGE table. These observations describe only the two selected files. The 7.3 GB database in PR #12028 was rebuilt from a Turso export.

On the same day the laptop service returned healthy with its embedder ready. Its health response supplied no publication generation or source watermark. Proving Fleet or Worker freshness requires tracing an authorized source revision through promotion and a scoped publication to the consumer.

The backup timer was active and waiting. Its November 23 backup service run timed out after 90 minutes (04:58:04–06:28:04 Central). The local dated gzip (491,721,304 bytes) and its manifest were present. The manifest recorded 18,325 sessions, 122,115 messages and 107,439 chunks, matching the live count. The November 15 local manifest recorded 18,325, 122,118 and 107,505 respectively. The ledger contained no November 23 completion and offsite copies were not independently verified. Verify a matching digest and destination receipt before relying on a particular copy. No backup or restore was triggered by the census. See #10304 for the existing backup investigation.

### Repeat the measurement

From the repository root, run `python3 pkgs/atlas-search/tools/atlas_census.py` on the laptop. Pass `--canonical PATH` to select a different canonical-shaped database. For each approved scan shard, add `--shard PATH`; for each per-tier hydration store, add `--content-store PATH`. Use `--seconds-per-store N` to bound each read (default 20 seconds).

The JSON report identifies inputs by role and ordinal. It measures each store in its own read-only transaction with WAL visibility, counts recognized tables, and distinguishes an empty table from a missing required table or an inapplicable one. A scan shard needs the expected manifest markers and a matching expected chunk count. Exit 1 means at least one requested store could not be counted and validated. Keep the path-to-ordinal mapping on the laptop. When reconciling shards against a generation, use its authorized inventory and count every required store before reporting coverage.

The runtime may need a compatible native read-only libSQL reader for schemas the local SQLite implementation cannot open. A failed read supplies no row count. Keep revisions, policy and the consumer's published generation with each freshness receipt.
