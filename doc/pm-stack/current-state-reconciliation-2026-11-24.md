# PM-stack ownership-map reconciliation — 2026-11-24

Additive to [`current-state-ownership-map-2026-11-05.md`](current-state-ownership-map-2026-11-05.md)
and [`current-state-reconciliation-2026-11-23.md`](current-state-reconciliation-2026-11-23.md):
it does not replace either note and makes no fresh live-host claim (read-only
repo + issue inspection from this run; no laptop transport was used, so no
claim is made about running processes or data contents).

## Why this note exists

The 2026-11-05 map's target column names Supabase `muse-dev2` as the
canonical home for every store. That project has since been **deleted**:
commit `be0d2b9f34c` ("The Supabase project (muse-dev2) is deleted and live
services already run on Turso"). Any worker inheriting the 11-05 target
column — or the cutover driver and plan built on it — would aim at a home
that no longer exists. This note re-pins each service's home and writer
against current `origin/main` and names the re-plan owner.

## Current homes and writers (repo-declared, with evidence)

| Service                | Canonical data home today (repo-declared)                                                                               | Single active writer (repo-declared)                                                                                                                          | Evidence                                                                                                                                        |
| ---------------------- | ----------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| **Aggregator**         | Turso (`PM_AGG_STORE_BACKEND=turso` deployed; `file` is the dev/test + rollback path, any other value refuses to start) | `pm-aggregator.service` remains the only writer of the aggregator stores; the collector writes only its one `aa-last-good` document as `fleet-feed-collector` | `pm-stack/aggregator/src/store-backend.ts:19`, `pm-stack/aggregator/install.sh:190`, `pm-stack/aggregator/src/store-backend.ts` writer identity |
| **Kaneo / PostgreSQL** | Still the laptop compose stack: `postgres_data` volume at `/var/lib/postgresql/data`, database `kaneo`                  | One `kaneo` app service backed by one `postgres` service                                                                                                      | `pm-stack/compose.yml` services `postgres:`/`kaneo:`, volume `postgres_data:/var/lib/postgresql/data`                                           |
| **Grist**              | Still the laptop volume `grist_data` at `/persist`                                                                      | One `grist` service                                                                                                                                           | `pm-stack/grist/compose.yml` service `grist:`, volume `grist_data:/persist`                                                                     |
| **Pad**                | Still the laptop volume `pad_data` at `/data`                                                                           | One `pad` service                                                                                                                                             | `pm-stack/pad/compose.yml` service `pad:`, volume `pad_data:/data`                                                                              |

No verification was performed that production currently runs these
declarations (no live-host inspection on this run); the claim is that the
checked-in deployment shape is as above, and nothing checked in contradicts
single-writer operation.

## What changed since the 2026-11-23 note

- **#6815 is CLOSED** (aggregator's first four stores + dashboard comments
  moved off laptop-local files; Turso is the canonical backend, `file` is
  the rollback target). The sibling stores (`freshness.json`,
  `notifications.jsonl`, `needs-drew.json`, `decisions.json`,
  `decision-events*.jsonl`) remain file-backed and are follow-on scope.
- **#1514 is CLOSED** (application-database backup routine landed; daily
  `#1514` dumps feed the runbook's pre-cutover restore source).
- **#3869 is CLOSED** (silent empty-day/container-name backup failures now
  handled, not silently green).
- **muse-dev2 is deleted** (`be0d2b9f34c`, 2026-11-21). Turso is the only
  remote store; the retired `supabase` backend value fails startup loudly
  (`store-backend.ts:18`).
- **Fencing gate landed** (`bda1f6662d3`: `kaneo-cutover.sh` promote refused
  unless `fence-check.sh` reports FENCED plus exact table/row parity;
  `7da5dfb132f` added the fence check for #3989).
- **Runbook + drills recorded**: `COLD-RESTORE-RUNBOOK.md` (rehearsal-proven
  2026-11-22) and `restore-drills/2026-11.md` (isolated same-host rehearsal
  plus the 2026-11-23 laptop-off simulation from off-machine copies: RTO
  145 s measured; Kaneo/Aggregator RPO misses recorded against the #1258
  targets, Grist/Pad met).
- **#1258 (design) is CLOSED.** Its Supabase-target rulings are superseded
  by the deletion; the surviving posture (tested cold restore, no warm
  standby; RTO/RPO numbers) stands until explicitly revised.

## Dead references this note supersedes (do not follow without the re-plan)

- `pm-stack/STORE-MIGRATION-PLAN-6816.md` §§2–5 and
  `pm-stack/scripts/kaneo-cutover.sh` still drive toward Supabase
  `muse-dev2`. The fencing rule and the gate mechanics in them remain
  sound; the **target** does not. Kaneo speaks Postgres and Turso is
  SQLite-shaped, so "Kaneo → Turso" is not a drop-in either — the new
  target home for Kaneo/PostgreSQL (and the file-shaped Grist/Pad
  canonical copies) is a decision owed under **#6816**, which remains
  OPEN. Its criterion 1 ("Kaneo runs against Supabase Postgres
  (muse-dev2)…") cannot pass as written.
- The 2026-11-05 map's "Target data home" column (all rows: Supabase
  `muse-dev2`) is stale in the same way; read it as history, not as the
  plan.

## #3989 criterion states after this note

1. **Ownership map — CHECKED (current again).** This note plus the two
   priors name one canonical home and one active writer per service with
   current-main evidence above. Guard: `pm-stack/scripts/ownership-map.test.sh`
   pins every machine-checkable citation, so the map cannot silently rot.
2. **Laptop-off usability — UNCHECKED.** Kaneo/Grist/Pad canonical stores
   are still laptop Docker volumes; no cutover has run.
3. **Route move + fencing — PARTIAL.** Fencing gate landed; the public
   route is unchanged (no move attempted).
4. **Cold restore per store + full-stack laptop-off drill — PARTIAL.**
   Same-host rehearsal + off-machine-copy simulation recorded with
   measured RTO/RPO; a real second-host drill and the Kaneo/Aggregator
   RPO misses remain open.
5. **Measured RTO/RPO in `restore-drills/` — PARTIAL.** Recorded for
   2026-11 with misses named, not met-or-revised.
6. **Health/watchdog visibility — CHECKED.** Restore point, backup age,
   active host, and failed/empty generations are surfaced in the
   host-health path (`watchdog.ts` backup-restore-point derivation; no
   empty generation reads as success).
7. **Dev/test path + rollback — PARTIAL.** Aggregator: `file` backend is
   the documented dev/test + rollback target; Kaneo/Grist/Pad rollback
   remains operator steps (migration plan §6).

## Next slice (for the worker that picks this up)

Re-plan #6816's criterion 1 target: name the post-`muse-dev2` canonical
home for Kaneo/PostgreSQL and the file-shaped Grist/Pad copies (the
all-in-Turso ruling cannot host Kaneo natively), then cut the first
mechanical slice of that plan the way `fence-check.sh` cut the fencing
rule — a gated script with fixture tests, not a windowed move. Until that
decision lands, do not run `kaneo-cutover.sh` presync/promote against any
target: its `muse-dev2` target is gone.
