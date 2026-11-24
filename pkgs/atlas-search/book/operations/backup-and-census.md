# Backup, restore and the census

Source: `nixos/home/laptop-backups.nix` and `nixos/home/atlas-backup.sh`, the schedule registry row in `doc/fleet-schedule-registry.md`, and the census statement in the Atlas program investigation record `doc/2026-11-23T12-06-51Z_design_atlas_program_investigation/` under https://github.com/developing-today/code/blob/main/nixos/home/laptop-backups.nix — provenance only; this chapter is complete without following it.

Two different questions get confused under the word "backup": is the corpus recoverable if the laptop store is lost, and how big is the corpus right now? The first has a landed, scheduled answer. The second has a frozen answer from 2026-10-31 and an honest caveat. This chapter keeps them separate.

## The backup: `atlas-backup`

The wiring lives in **`nixos/home/laptop-backups.nix`**, with the script vendored beside it as `nixos/home/atlas-backup.sh` and packaged as `atlas-backup` (`pkgs.writeShellScriptBin`). It was migrated from a sandbox cron on 2026-10-30 under the laptop-only execution rule (#1515).

| Unit                   | Shape                                                                                                                                                                                                             |
| ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `atlas-backup.service` | systemd **user** service, `Type = oneshot`, `ExecStart` = the packaged `atlas-backup`, `TimeoutStartSec = 90m`, description `Daily atlas.db backup - local + R2 + Drive (code#1515)`. Gated on `cfg.atlas.enable` |
| `atlas-backup.timer`   | systemd **user** timer, `OnCalendar` from the option `services.laptop-backups.atlas.onCalendar`, **default `04:47`**, `Persistent = true`, `RandomizedDelaySec = 15m`, `WantedBy = [timers.target]`               |

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

The store that census describes is the canonical store on the laptop — the `atlas.db` above **plus 29 topic shards**. Its sensitivity split, from the same record, cross-checks the total exactly:

| Tier      | Shards | Chunks      |
| --------- | ------ | ----------- |
| Internal  | 28     | 107,001     |
| Secret    | 1      | 638         |
| Public    | 0      | 0           |
| **Total** | **29** | **107,639** |

Zero public-tier shards is a stated fact of the v1 record, not a gap in the table. Any chapter, dashboard, or answer that quotes 18,325 / 122,124 / 107,639 must date it **2026-10-31** — see [FAQ](faq.md).

### The unrecounted monolith

Later records describe a **~7.3 GB monolith without a verified recount**. Both halves of that sentence matter. The store has grown since October — ingest, however manual, did not stop — and no fresh verified census has been taken. The 7.3 GB is a size observation, not a count: it cannot be turned into sessions, messages, or chunks by arithmetic, and this book does not try.

### How a fresh census would be taken

A fresh census is an early deliverable of the Atlas programme (#13241), and its shape is fixed by what made the October one trustworthy:

1. Count the canonical store directly: sessions, messages, and chunks in `~/atlas/turso-atlas/data/atlas.db`.
2. Count every shard: each shard carries an `atlas_manifest` with an `expected_chunks` value, checked against an actual `COUNT(*)` of its chunks — a manifest that disagrees with its own store is a finding, not a number.
3. Split the result by tier (public / internal / secret) so the split cross-checks the total, as the October split does.
4. Date it, name the store it was taken from, and publish it in the same place the October census is quoted from — a design or programme record — so the next reader finds one canonical statement, not a scatter of counts.

Until that runs, there is no newer number to quote, from any source.

### Why consumers still see the October counts

Because nothing downstream recounts. Consumers — the service, the Worker, the CLIs — search the **published** store, and the publish consumers see is the October publish: ingestion today is manual and script-driven (the recurring sweep runner, #7969, is unbuilt; the ingest scripts still live outside the repo, with migration #5670 open), and no recount-and-republish pass has landed since. The counts are therefore not stale by accident or by caching — they are the counts of the corpus as published, frozen at the last verified census, while the underlying store has grown past them uncounted. The Worker cache's indefinite retention (#6805) is a separate staleness story, covered in [Troubleshooting](troubleshooting.md); even a perfectly fresh cache would still be answering over the October-published corpus.
