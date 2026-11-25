# Backup-path audit — Cloudflare, Turso, laptop, seats

Audit for #14864, performed 2026-11-25 by a Felix lane on the muse
seat. Every claim below is tied to evidence read during the audit
(timer state, restic snapshot listings, D1 meta rows, Turso/R2 API
listings, live `git ls-remote` probes); where a path could only be
assessed from its documentation, that is said explicitly.

## Inventory

### A. Seat (sandbox) backups — R2 bucket `supabase-backups`, restic/rclone

| # | Path | Captures | Lands | Schedule | Retention | Last verified restore |
|---|------|----------|-------|----------|-----------|-----------------------|
| A1 | sandbox-state-backup | `~/.config` | R2 `supabase-backups`, prefix `restic-sandbox-config` | daily 03:37 CT, Persistent | 14 daily + 8 weekly, prune in-run | Real restore 2026-11-21 (seat rebuild from this repo). See finding F1: the two scheduled snapshots before today held 44 files |
| A2 | sandbox-home-backup | `/home/hatch`, per top-level entry | R2 `supabase-backups`, prefix `restic-sandbox-home` | daily 04:37 CT, Persistent | per-entry anchors (36 monthly + 10 yearly + keep-tag), prune on graceful cycle end | Real restore 2026-11-21 (same rebuild). See finding F2: cycle bookkeeping failed/stalled since 2026-11-22 |
| A3 | sandbox-home-mirror | `/home/hatch` (same scope as A2) | R2 `supabase-backups`, prefix `sandbox-home-staging` (rclone crypt) → laptop pulls → restic repo `restic-sandbox-home-laptop` | seat push daily 03:10 CT; laptop pull daily 05:30 CT | staging is a mirror (no archive); laptop repo 14d/8w/12m, Sunday check | Never as a chain. See finding F3: no completed seat-side run since 2026-11-14 |
| A4 | sandbox-backup-watchdog | — (verifies A1–A3 timers are installed/active) | — | every 15 min | — | n/a. Verifies timer *presence*, not backup *effect* — the root pattern behind F1–F3 |
| A5 | hatch-autocommit | seat state in `dezren39/hatch` + `~/hatch-personal` working trees | GitHub | every 45 min | git history | Continuous; timer firing at audit time (last 2026-11-25 08:26 UTC). Not a data backup, but it is the only off-machine copy of seat-local docs/logs state |

### B. GitHub backup / rehost program (#8584, closed as implemented)

Units live in `infra/github-backup/systemd/` (fetch, mirror, r2,
freshness, rehost, reconcile, drain), installed together by
`install-units.sh` **on the laptop**. Config covers 18 owners,
`backup_root ~/backups/github` on the laptop.

| # | Path | Captures | Lands | Schedule | Retention | Last verified restore |
|---|------|----------|-------|----------|-----------|-----------------------|
| B1 | plain git mirrors | all repos of 18 owners (config.json tier rules) | laptop `~/backups/github` | via github-backup-mirror timer (laptop) | mirror (current state) | Drill HEADs verified 2026-11-22; all four fleet mirrors had drifted at last check, and the timer is not installed (F4) |
| B2 | R2 bundles / cold bundles | mirror repos as bundles | R2 | via github-backup-r2 timer (laptop) | per #12269 | 2026-11-22 drill set; timer not installed (F4), so no new bundles since |
| B3 | git rehost | published repos served as git smart-HTTP | R2 bucket `fleet-git-rehost` + Worker `fleet-git-rehost`, `git.security.cab` | publisher via github-backup-rehost timer (laptop) | current packs | Live clone proof 2026-11-24 — for **one** repo only. Audit probe 2026-11-25: `developing-today/fleet` answers (HEAD b3289202, unchanged since the proof); `developing-today/code` and `dezren39/darwin-nix` return "not found" — never published |
| B4 | five-copy freshness probe | — (compares GitHub / mirror / R2 / rehost / D1 copies, auto-files alert issues) | — | daily 04:10 (laptop timer) | — | Not running: timer not installed (F4). No freshness alert issues exist because the prober itself is off |
| B5 | config-declared gaps | `vertexinc` org: **nothing mirrored** (SAML SSO blocks enumeration). `dezren39/edsebooks`: HTTP 451, cannot be fetched. `NixOS` org: mirrored by sweep, no webhook coverage | — | — | — | Documented in `infra/github-backup/config.json` itself; recorded here so the audit trail carries them |

### C. Fleet queue / Turso

| # | Path | Captures | Lands | Schedule | Retention | Last verified restore |
|---|------|----------|-------|----------|-----------|-----------------------|
| C1 | fleet-queue-backup export | Turso `fleet-queue` (+ mirror tables) | D1 `fleet-queue-backup` (a13bddb9) | daily timer (laptop) | export snapshot rows | Restore drill passed 2026-11-23. **Stale**: `_fleet_queue_backup_meta.exported_at` read live during this audit = **2026-11-23T15:27:30Z** (~41 h old); the 2026-11-24 02:40 CT fire and every fire since produced no export. Tracked as silent failure on #10296 |
| C2 | Turso platform PITR | all 42 Turso DBs in org `dezren39` (33 `atlas-*`, `fleet-queue`, `fleet-platform`, `fleet-mirror`, `fleet-dispatch-jobs`, `muse-meta`, `bedside-and-away`, `notes-submissions`, `voice-line-ledger`, `great-star-lord`) | Turso-internal | continuous (platform) | plan-dependent window, **not verified** | Never drilled. All 42 DBs have `delete_protection = true` (verified live). Fleet-side export exists only for C1; the backup-mesh program (#8988 epic, #9093, #13455, #13457, #13458, #14247) is designed/sliced but **not live** — its first-light slice #14250 was still at intake at audit time |
| C3 | Turso → D1 copies (new) | `fleet-queue`, `fleet-platform`, `fleet-mirror`, `fleet-dispatch-jobs`, `bedside-and-away`, `voice-line-ledger` | same-named D1 databases | — | — | Observed, not documented anywhere this audit found: six D1 databases created **2026-11-25T03:14–03:15Z** (during the #14866 Cloudflare-first work). One-time copy or continuous replication could not be determined from this seat — flagged in F5 |

### D. Other Cloudflare data

| # | Path | Notes |
|---|------|-------|
| D1 | R2 bucket `atlas-ledger-snapshots` (created 2026-11-05) | Atlas ledger snapshot target; cadence/retention not documented in either repo this audit searched — unverified path |
| D2 | D1 databases with no fleet backup evidence: `decision-events`, `fleet-config`, `cold-store-manifest`, `webhook-gateway`, `labops-db`, `nix-cache-db` (+ the six in C3) | Recovery would rely on Cloudflare D1 point-in-time recovery (plan-dependent, never drilled) — same class as C2 |
| D3 | R2 buckets `agent-inbox-mail`, `webhook-gateway-payloads`, `labops-files`, `fleet-exec`, `nix-cache-nars`, `poc-2285-eu2` | Payload/file stores, not backups themselves; no backup-of-bucket path found. R2 has no built-in PITR; bucket loss = data loss for anything whose only copy lives there (notably the A1–A3 restic repos and B3 packs — see F6) |

### E. Laptop (amd-6) local state

The laptop is the scheduled home of B1–B4, C1, and A3's pull side.
At audit time **none of the `infra/github-backup` timers are
installed there**: `install-units.sh` has never been run from a
clean main checkout because the laptop's `~/git/code` checkout is
parked on another lane's branch (#13833). Recorded, not chased —
per the audit's scope. Consequences are finding F4.

## Findings

- **F1 — Config backup silently captured a near-empty tree
  (fixed exposure; root cause filed).** The scheduled
  sandbox-state-backup snapshots of 2026-11-23 (20.5 KiB) and
  2026-11-24 (36.6 KiB) contain **44 files**; the healthy
  2026-11-17 snapshot contains 4,255 files and the live tree has
  3,834 files / 94 MB. The job reported success both days; the
  sparse snapshots were taken while the seat's `.config` was
  transiently sparse across recycles. The script has **no
  minimum-content guard** (the mirror script has an empty-source
  guard; this one does not). Exposure closed during this audit:
  a manual run on 2026-11-25 produced snapshot `f1c40e1e`,
  83.884 MiB, full tree. Guard + watchdog-effect check filed on
  hatch-personal (dezren39/hatch-personal#100).
- **F2 — Home backup cycle has not completed since 2026-11-22.**
  Seat state shows `cycle-failed` (2026-11-22 11:20), no
  `last-run.done` (only `.tmp`), and entry-state untouched since
  then — yet the restic repo received per-entry snapshots as late
  as 2026-11-24 11:52 UTC (2,225 snapshots total, 2026-10-31 →
  2026-11-24). State and repo disagree; whether the retention
  forget/prune (which runs on graceful cycle end) has executed
  since 2026-11-22 cannot be confirmed from seat state. Filed
  with F1/F3 as one hatch-personal issue: the watchdog checks
  timers, not effects.
- **F3 — Mirror chain (the backup-of-record direction,
  hatch-personal#50) is stalled at the first hop.** Last completed
  seat-side mirror run: 2026-11-14 18:01; no state activity since
  2026-11-15. The laptop-side restic repo of record
  (`restic-sandbox-home-laptop`) therefore also stopped advancing
  ~2026-11-14. Cause not determinable from this seat (no journal
  survives from the firing incarnations). Filed with F1/F2.
- **F4 — The entire laptop leg is one uninstalled unit set away
  from existing, and it shows.** Because `install-units.sh` never
  ran (E above): mirrors drift (B1), no new R2 bundles (B2), the
  rehost publisher never ran so 3 of 4 fleet repos were never
  published (B3), the freshness probe that would have alerted on
  all of this is itself off (B4), and the queue export is 41 h
  stale (C1). Every one of these paths has exactly one machine
  and one checkout state between "designed" and "dead". Filed as
  a priority/critical design item in this repo: give the
  export/publish/probe legs a home that does not depend on the
  laptop being up and on a clean checkout (seat-runnable or
  hosted scheduling), or explicitly accept laptop-dependence and
  wire the freshness alert to something that *is* running.
- **F5 — Managed-database recovery rests on unverified platform
  PITR.** 42 Turso DBs (C2) and 12 D1 databases (D2/C3) have no
  completed fleet-side backup; the mesh program that would cover
  the Turso side is not live (#14250 at intake). PITR retention
  windows on either platform have never been verified by a
  drill. Not filed as new work — already owned by #8988/#9093/
  #13455/#13463 (restore-verification rotation) — but recorded
  here as the audit's largest uncovered surface, with the C3
  observation attached for the #14866 slices to confirm or
  correct.
- **F6 — Duplication and single-copy notes.**
  - *Managed duplication:* A2 (direct restic) and A3
    (mirror → laptop restic) cover the same tree into the same
    bucket; A3 was directed (2026-11-14) to *replace* A2 and
    never did — both still run (A3 stalled, F3), so the fleet
    pays for two paths and currently gets one.
  - *Deliberate separation:* A1's config repo is separate from
    A2's home repo by design (2026-10-28 decision, documented in
    the script) — not waste.
  - *Only copy on the machine it protects:* B1 mirrors live on
    the laptop; their off-machine leg (B2) is uninstalled, so
    today the mirrors' only copies sit on amd-6 (GitHub itself
    remains the reconstruct-source, which is precisely the
    dependency this program exists to remove).
  - *Only copy, platform side:* the A1–A3 restic repos and B3
    packs live solely in R2 with no second-target replication;
    the mesh replication leg that would copy R2 → laptop/Drive
    (#13457) is not live.

## What was fixed in this pass

1. **F1 exposure:** manual sandbox-state-backup run produced a
   full snapshot (`f1c40e1e`, 83.884 MiB, 2026-11-25 08:48 UTC);
   the config repo's newest restore point is healthy again.
2. **Seat timers verified live** on the current incarnation
   (state/home/mirror/watchdog timers installed and scheduled;
   hatch-autocommit firing) — the keeper had restored them after
   this morning's recycle; no dead timer found on this seat.

## What was filed

- dezren39/hatch-personal#100: backup jobs must verify effect,
  not just run — sparse-snapshot guard for state-backup, stalled
  home-backup cycle bookkeeping, stalled mirror chain, watchdog
  effect checks (F1–F3).
- This repo, #14964 (priority/critical design): de-single-point
  the backup plane's laptop leg (F4).
- F5/F6 recorded here and on #14864; owned by the existing mesh
  program (#8988 and children), no duplicate filed.
