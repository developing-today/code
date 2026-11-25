# Backup inventory

The cross-system inventory for issue #14864 (audit: all backup paths
across Cloudflare, Turso, laptop, and seats). Scope of this slice: every
backup path that is **defined in this repo**, plus the paths recorded in
the backup-mesh design record
(`doc/2026-11-23T16-34-18Z_design_backup_mesh/note.md`, #9093). Anything
whose timer or job lives only on a machine is listed as such — that is a
finding, not a detail.

`backup-inventory.test.sh` (next to this file) checks that every repo
path cited below still exists, so the inventory cannot silently drift
from the tree.

## Inventory

| # | Backup | Captures | Lands in | Schedule | Retention | Last verified restore |
|---|--------|----------|----------|----------|-----------|-----------------------|
| 1 | GitHub mirrors (`infra/github-backup/`) | Bare `--mirror` clones of all non-fork repos + forks < 500 MB across 17 owners (config in `infra/github-backup/config.json`) | Laptop `~/backups/github`, plus R2 cold bundles and the Cloudflare rehost (git.security.cab) | Webhook drain every 5 min; fetch daily; mirror 02:15; R2 01:30; rehost 03:20; freshness probe 04:10; reconcile weekly (units in `infra/github-backup/systemd/`) | Mirror tree is the unit; cold copies are `git bundle`s | Freshness probe daily; no full restore drill recorded in repo |
| 2 | Cold-store nightly (`pkgs/cold-store-reaper/scripts/nightly-backup.sh`) | D1 `cold-store-manifest` export + a portable SQLite export of every Turso database (`pkgs/cold-store-reaper/scripts/export-turso.sh`) | Google Drive `db-backups/d1/` and `db-backups/turso/` (Drive is the only target) | Nightly (timer lives on the laptop, not in this repo) | 30 daily / 12 weekly / 12 monthly, enforced by `prune-backups` (trash, never delete) | Quarterly restore drill (`pkgs/cold-store-reaper/scripts/restore-drill.sh`): 3 cold objects hash-verified through the production rehydrator path + newest D1 backup compared row-by-row against live |
| 3 | Cold-store archive (`pkgs/cold-store-reaper/`) | Operator files packed as immutable zstd-frame bundles | Google Drive (app-owned `Cold Store Reaper` tree); manifest in local SQLite + D1 `cold-store-manifest` | On demand (`pack`/`upload`) | Bundles are immutable; objects marked `cold` only after Drive md5+size verify | Same quarterly drill as #2; `fetch` sha256-verifies every object on read |
| 4 | Agent credential snapshots (`pkgs/agent-backup/backup.sh`) | The small irreplaceable credential/config set (auth files, API keys, settings) of one seat | **The same machine**: `~/.local/share/agent-backups/<timestamp>/`, plaintext, mode 0700/0600 | On change (no timer in repo; invoked per seat) | Grows only when content changes; no pruning recorded | None recorded — copies are compared for change, never restore-tested |
| 5 | Runtime DB export (`pkgs/agent-backup/README.md`) | Muse runtime records DB (scheduler, agents, goals, sessions, usage rollups — 28 landing tables) | Turso `muse-mirror` (`agent_mirror_*`) | Agent cron `runtime-db-export`, every 6 h (America/Chicago); the job body is `pkgs/agent-backup/JOB-BODY.md` | Watermark-based history tables; no retention limit recorded | None recorded — row counts reconciled per run, no restore drill |
| 6 | Laptop legs (recorded in the #9093 design record, defined in `nixos/home/laptop-backups.nix`) | atlas DB; repo archive (restic); laptop state (restic); Supabase; dashboard | Laptop + R2 + Drive (Supabase: R2 only) | Daily (Supabase twice daily) | Per-leg restic policy (see the design record) | Freshness watchdog `laptop-backups-watchdog` (#1841) checks completion, incl. cold-store (PR #10988) |
| 7 | fleet-queue export (named starting point of #14864) | fleet-queue Turso database | D1 database `fleet-queue-backup` (#8584) | Daily timer — **the timer is not defined in this repo** | Not recorded | None recorded; per the #9093 record, the D1 side itself has no backup home yet |
| 8 | Turso platform protection | All 42 Turso databases (~9.4 GB at the 2026-11-12 inventory) | Turso PITR + Delete Protection | Continuous | PITR window 30 days; Delete Protection ON for all 42 (verified per DB 2026-11-17, per the #9093 record) | PITR is a platform feature; no restore exercise recorded in repo |
| 9 | id node state (`pkgs/id/book/operations/backup-and-restore.md`) | An id node's data directory (`.iroh-store/`, `.iroh-key`, `groups.jsonl`, `events/`, worlds, keysets) | **Wherever the operator copies it** — there is no backup or restore verb for a whole node in the `id` CLI (only `id world journal backup` for one world's journal) | Manual | Operator's choice | None — the chapter marks unverifiable steps as unverified |

## Findings

- **Only copy lives on the machine it protects:** #4 (agent credential
  snapshots are plaintext on the same seat's disk — the script's own
  header says this protects against deletion and bad edits, not disk
  loss) and #9 (id node state has no automated off-machine leg at all).
- **Seat/machine-only timers:** #2's nightly timer, #5's agent cron, and
  #7's daily export timer are not defined in this repo. A rebuilt or
  replaced machine does not recreate them from the tree; #1 is the
  counterexample — its units live in `infra/github-backup/systemd/` with
  argument tests beside them.
- **Unverified restores:** #1, #4, #5, #7, #8, #9 have no restore drill
  recorded in the repo. #2/#3 are the exception (quarterly drill with
  hash and row-level checks). The #9093 design record says the same in
  general form: "no scheduled restore verification for most stores."
- **Single-target legs:** #2 lands only in Drive; #7 lands only in D1,
  and D1 itself has no backup home yet (#9093 §6). The backup mesh
  (#9093, legs #13455–#13463) is the filed large finding that closes
  this class; it is a design, not yet a running leg.
- **Known gap, already filed:** the OpenCode session DB has no
  off-laptop backup (21.37 GB, measured 2026-11-12); economics in #9094,
  mesh leg #13462 gated on it.
- **Duplication:** Turso data is exported by #2 (all databases, to
  Drive), #5 (one runtime DB's records, into another Turso DB), #7 (one
  database, to D1), and protected in place by #8 (PITR). These are
  different shapes (cold export vs mirror vs point-in-time), but #2 and
  #8 overlap for any restore inside the 30-day PITR window — accepted
  while restores from #2 remain the only exercised path.
- **Named starting points, disposition:** #8584 program = rows #1 + #7;
  fleet-queue-backup D1 export timer = row #7 (not in repo, see above);
  sandbox state/home timers = row #4's class (seat-local, not in repo);
  mirror timers = row #1 (in repo, with tests).
