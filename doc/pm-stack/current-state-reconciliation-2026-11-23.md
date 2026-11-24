# PM-stack ownership-map reconciliation — 2026-11-23

This is an additive reconciliation of [`current-state-ownership-map-2026-11-05.md`](current-state-ownership-map-2026-11-05.md), not a replacement inventory. The 2026-11-05 host observations remain historical; this note makes no fresh claim about production processes, host state, or data contents.

## Current repository and issue evidence

- **Aggregator:** the checked-in aggregator README documents Turso-backed stores (`pm-stack/aggregator/README.md` and its store implementation), not the older Supabase `muse-dev2` target in the November 5 map. Issue #6815, which tracked the older aggregator migration wording, is closed. This corrects the target description; it does not establish that every runtime file has migrated or that production has no laptop-local state.
- **Kaneo/PostgreSQL:** the checked-in deployment remains the compose service in `pm-stack/compose.yml`. Issue #1752 records this existing NixOS-managed compose service as satisfying its Kaneo/PostgreSQL service requirement. This is repository/configuration evidence, not a fresh check that the service is running or that its database is off the laptop.
- **Restore evidence:** `doc/pm-stack/restore-drills/2026-11.md` records a successful isolated **same-host** rehearsal using a local archive. It explicitly says R2/Drive were not exercised and the laptop-off drill still needs a second Docker host. Therefore it does not satisfy #3989's off-machine laptop-off acceptance criterion. The runbook also records that pre-cutover daily Kaneo dumps do not meet the one-hour RPO.
- **Remaining tracker:** #6816 remains open for the Kaneo/Grist/Pad cutover and laptop-off drill. #3989 remains open; this reconciliation does not claim the broader migration, fencing, route move, or measured objectives are complete.

## Verification boundary

On 2026-11-23, issue state and checked-in repository files were inspected from this run. Laptop transport was unavailable, so there was no fresh live-host inspection, no laptop-off operation, and no verification of off-machine restore sources. Do not treat the November 5 inventory as current live evidence or the November 22 same-host rehearsal as an off-machine drill.

## Evidence pointers

- [Historical ownership map](current-state-ownership-map-2026-11-05.md)
- [Aggregator README](../../pm-stack/aggregator/README.md)
- [Kaneo/PostgreSQL compose declaration](../../pm-stack/compose.yml)
- [November restore rehearsal and gaps](restore-drills/2026-11.md)
- [Cold-restore runbook](COLD-RESTORE-RUNBOOK.md)
- GitHub issues checked: [#3989](https://github.com/developing-today/code/issues/3989), [#6815](https://github.com/developing-today/code/issues/6815), [#6816](https://github.com/developing-today/code/issues/6816), [#1752](https://github.com/developing-today/code/issues/1752).
