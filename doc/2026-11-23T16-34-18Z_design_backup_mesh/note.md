# Backup mesh — design (#9093)

- Design record · Issue: developing-today/code#9093 (Workstream B of the
  Atlas program, #8983) · Proto design:
  `doc/2026-11-12T23-59-00Z_design_atlas_v1_proto/note.md` §B · Status:
  design for implementation; the granular legs are filed as #13455,
  #13456, #13457, #13458, #13459, #13461, #13462, #13463 (§10).
- This record satisfies #9093's design acceptance item. It creates no
  schedule; every implementation PR lands its own
  `doc/fleet-schedule-registry.md` row in the same pass.

## 1. Current state (verified, with dates)

- **42 Turso databases** (~9.4 GB summed at the 2026-11-12 inventory;
  Scaler plan, overages OFF, 24 GB storage is a hard wall). Turso PITR
  reaches any point within 30 days.
- **Delete Protection is ON for all 42 databases** — enabled per DB and
  verified per DB on 2026-11-17 (#9093 step-1 record; this is the
  per-database disposition #9093's acceptance asks for: enabled,
  fleet-wide, no exceptions recorded).
- **cold-store-nightly is repaired.** Its 2026-11-12 failure was the
  `export-turso` step killed by the service's 30-minute start timeout
  (plus ~30 GB of accumulated export scratch in anonymous /tmp dirs).
  Fixes landed 2026-11-11 (`TimeoutStartSec` 120m, d3fa454ccc;
  per-database bounded scratch, b4278bcbc2) and full 42/42 successful
  runs are on record for 2026-11-15 and 2026-11-16. The interim risk
  (degraded Turso→Drive leg, PITR as sole protection, Delete
  Protection still off) is named on #1516 (comment 7107336806).
- **The orphan restic repo is not orphaned:** `~/backups/restic/
  laptop-state/` is written by `laptop-state-backup` (#1849), verified
  producing snapshots 2026-11-16.
- **Existing legs** (all in `nixos/home/laptop-backups.nix`): atlas-
  backup (canonical atlas DB → laptop + R2 + Drive, daily),
  repo-archive-backup (restic → local + R2 + Drive, daily),
  laptop-state-backup (restic → local + R2 + Drive, daily),
  supabase-backup (→ R2, twice daily), dashboard-backup (→ all three,
  daily), cold-store-nightly (every Turso DB + Supabase + D1 → Drive
  only, nightly, full exports). The freshness watchdog (#1841,
  `laptop-backups-watchdog`) covers these jobs, including a cold-store
  completion check added by PR #10988.
- **D1 exists in the fleet:** the `fleet-queue-backup` D1 database
  (#8584) receives the fleet-queue Turso export on a daily timer. That
  direction (Turso → D1) is a backup *target* choice for one database;
  the D1 side itself has no backup home yet — §6 covers it.
- **The OpenCode session DB has no off-laptop backup** (21.37 GB,
  ~5.1 GB zstd, measured 2026-11-12). Its economics research is #9094
  (open); the mesh leg is #13462, gated on it.
- **Gap this design closes:** the only every-database leg is a nightly
  *full* export to a *single* store (Drive). There is no delta-efficient
  leg, no R2/laptop copy of the general Turso fleet, and no scheduled
  restore verification for most stores.

## 2. Topology

One restic repository per store, one snapshot per (database, cycle)
whose payload is that database's export artifact set (the `.dump` from
the proven export path plus a manifest of per-table row counts and
sha256 hashes — the manifest is what restore verification checks
against). Snapshots carry tags `db=<name>` and `class=<A|B>` so
retention and verification select per database.

| Source | Leg | Store | Retention |
|---|---|---|---|
| Turso DBs, class A (platform) | restic backup of stream-zstd export, weekly staggered | R2 repo `restic-turso-mesh` | keep-daily 7 / keep-weekly 8 / keep-monthly 12, per-DB tags |
| Turso DBs, class B (general) | restic backup of stream-zstd export, 14-day rotation | R2 repo `restic-turso-mesh` | same |
| R2 mesh repo | `restic copy`, nightly after ingest | laptop-local mesh repo | mirrors the R2 snapshot set exactly |
| R2 mesh repo | `restic copy`, nightly after ingest (changed packs only) | Google Drive mesh repo | mirrors the R2 snapshot set exactly |
| D1 databases (in scope, §6) | weekly export → import, wholesale replace | dedicated Turso home DB (`d1-backup-<name>`), which is itself class B | Turso PITR 30d + the home's own mesh snapshots |
| Laptop-local DBs (qualifying, §6) | weekly export → import, wholesale replace | dedicated Turso home DB (`laptop-backup-<name>`), itself class B | same |
| Embedding / shard stores | ride their parent DB's leg (Turso-resident) or a named file-level restic leg for non-Turso stores | R2 mesh repo + copies | same as parent class |
| OpenCode session DB | scheme per #9094 (full / zstd / restic-delta) | Google Drive + laptop archive location | per #9094's costing |

Every store can rebuild any other: R2, laptop, and Drive hold the same
mesh snapshot set (§5's copy discipline), and the Turso homes are
themselves mesh members, so a Turso-side loss is rebuilt from any
store's snapshots and a store-side loss is re-copied from either of
the other two.

## 3. Repository and credential shape

- The mesh repos follow the repo-archive / laptop-state pattern
  exactly: restic with the shared backup password key, R2 access via
  the `supabase-backups` bucket (new prefix `restic-turso-mesh/`),
  Drive via the same rclone remote those jobs use. Credentials are
  fetched from Bitwarden **by name** at runtime; nothing secret is
  written to this repo, the nix store, or any manifest.
- The export step reuses `pkgs/cold-store-reaper/scripts/export-turso.sh`
  (including its repair filter for libsql's malformed `/dump` schema
  rows, #8811) and its bounded per-database scratch discipline — the
  two things the 2026-11-12 failure proved necessary.
- Database enumeration is always `turso db list` at run time. No leg
  hard-codes a database list; a database that appears joins the
  rotation, and one that disappears is named in the run record.

## 4. Cadence mechanics per database class

- **Class A — platform databases** (`fleet-queue`, `fleet-platform`,
  canonical `atlas`): weekly per database, staggered across the week.
  These carry the fleet's own coordination state, so they run at
  double the mesh cadence. Leg: #13455.
- **Class B — general Turso databases** (everything else, ~39 today):
  the set is divided into 14 daily cohorts by stable hash of database
  name; each database is snapshotted once per 14 days. A failed export
  retries the next night and is named in the run record and the
  watchdog — never silently skipped. Leg: #13456.
- **Class C — per-user databases** (#9083's tenancy design): not yet in
  existence. Because enumeration is by `turso db list`, they join
  class B's rotation automatically when they arrive; no mesh change
  is needed, which is the sequencing property §8 requires.
- **Class D — D1 databases:** weekly export → Turso home (§6). Leg:
  #13458.
- **Class E — laptop-local databases:** weekly export → Turso home
  (§6), staggered with class D. Leg: #13459.
- **Class F — OpenCode session DB:** cadence set by #9094's scheme;
  leg #13462.
- **Replication:** the R2 → laptop and R2 → Drive copies run nightly
  after the ingest cohorts, so a snapshot is in all three stores
  within a day of being taken. Leg: #13457.
- **Why 14 days:** the PITR window is 30 days. A 14-day cadence
  guarantees at least two mesh snapshots inside every PITR window,
  bounds the worst case (a database snapshotted the day before a
  loss that PITR cannot cover — deletion past the 5-day window,
  account loss) to 14 days of restic-only age, and keeps the nightly
  export load at ~3 databases instead of 42.

## 5. Retention

- Mesh repos: `keep-daily 7 / keep-weekly 8 / keep-monthly 12` applied
  per database via tags. At the 14-day cadence this yields roughly a
  year of reach-back at monthly granularity for class B, denser for
  class A.
- **Forget/prune runs in R2 only.** The laptop and Drive repos receive
  the thinned set by `restic copy`; no store prunes independently, so
  the three snapshot sets cannot drift apart.
- The pre-existing dated-artifact legs (atlas-backup 7/7/4,
  dashboard/openclaw 30-day, supabase-backup) keep their current
  retention; the mesh does not rewrite them. cold-store-nightly's
  Drive artifacts are superseded per class by §8, not deleted early.

## 6. The appropriateness line — what takes a Turso home

D1 exports and laptop-local databases land in Turso as their backup
home **when all four hold**: (1) the data is SQL-shaped and a SQL
export is a faithful copy; (2) the size is modest (order of hundreds
of MB, not tens of GB); (3) it has no other off-laptop/off-cloud copy;
(4) wholesale-replace semantics are acceptable (the home is a backup
target, never a live read path).

Named exclusions, with reasons:

- **OpenCode session DB** — fails (2) on size and churn; its home is
  Drive + the laptop archive under #9094's scheme (#13462).
- **A D1 database that is a pure replica of a Turso source** — the
  source's mesh coverage is its backup; a second home adds a copy of
  a copy. Each such exclusion is recorded by name in #13458.
- **Anything already covered by an existing leg** (repo-archive,
  laptop-state, dashboard, atlas backups) — no double coverage; the
  inventory in #13459 records the covering leg per database.

Embeds need no line of their own: embedding stores that live in Turso
databases ride those databases' legs, and #13461 inventories any that
live elsewhere and gives each a leg or a recorded exclusion.

## 7. PITR vs restic, per age band

- **Age 0–30 days — PITR is primary.** Any point in time, no snapshot
  granularity loss. The mesh's role in this band is insurance against
  the losses PITR cannot cover: deletion past the 5-day deleted-DB
  window (Delete Protection now makes deletion a deliberate act, but
  a deliberate act can still be a mistake), Turso account/plan loss,
  and corruption old enough that the operator wants a pre-corruption
  *snapshot* rather than a point-in-time roll.
- **Age > 30 days — the restic chain is the only protection.** PITR
  has expired; restore granularity is the retention ladder's (weekly
  to ~2 months, monthly to ~a year). That thinning is accepted
  deliberately: restores that old are archaeology and audit, not
  incident response, and the storage cost of daily granularity at a
  year is paid by every database for a case almost none will hit.
- **Either band — the mesh is the exit path.** All three mesh stores
  sit outside Turso, so the fleet's database state survives losing
  Turso entirely; that is the property the single-store Drive leg
  gestured at and the mesh makes symmetric.

## 8. Sequencing and the cold-store-nightly replacement path

- The mesh **does not depend on the hosted editions** (#9060 / #9078)
  existing. It protects today's laptop + Turso shape; §4's class C is
  the only forward reference, and it is satisfied by enumeration, not
  by building anything early.
- cold-store-nightly keeps running unchanged until, **per class**, the
  class's delta leg has passed restore verification twice (its own
  leg issue's acceptance, then one rotation cycle under #13463). At
  that point the class's full-export leg is retired in the same PR
  that records the second verification, with its schedule-registry row
  updated in that PR. This is the "leg replaced by the mesh design"
  path for #9093's acceptance item 3; the root-cause path already
  landed (§1).

## 9. Restore verification

Backup-verified is not restore-verified. The #1841 laptop-backups
watchdog is the verification surface: each leg issue's acceptance
includes a first restore verification (restore to an isolated scratch
target, per-table row counts against the snapshot manifest, never a
write to a production store — the discipline of the #10300 cold-
restore drill and the #8584 D1 restore drill), and #13463 owns the
recurring rotation (one leg × one store per week, every combination
at least quarterly) plus the watchdog's mesh reporting.

## 10. Granular implementation issues filed

| Leg / class | Issue |
|---|---|
| Class A — platform DBs → R2 restic deltas | #13455 |
| Class B — general DBs → R2, 14-day rotation | #13456 |
| Replication — R2 → laptop, R2 → Drive | #13457 |
| Class D — D1 exports → Turso homes | #13458 |
| Class E — laptop-local DBs → Turso homes | #13459 |
| Embeds — coverage + no-re-embed restore proof | #13461 |
| Class F — OpenCode DB (gated on #9094) | #13462 |
| Restore-verification rotation + watchdog | #13463 |

Each carries a restore-verification acceptance step, per #9093's
acceptance list.
