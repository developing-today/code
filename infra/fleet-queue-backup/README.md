# fleet-queue-backup

The backup leg of the fleet-queue store, built for issue #8584
(slice 6 of the fleet unification design, #8575): a scheduled export
of the fleet-queue Turso database into a Cloudflare D1 database —
same SQLite family, restorable — with a drilled restore path back
into Turso. The D1 copy also serves as the analytics copy.

Per #8575's wipe test, the queue database holds coordination only —
claims, leases, heartbeats — so this backup is a convenience and an
analytics copy, never a second durable truth: durable truth lives on
GitHub. Losing the queue store entirely costs in-flight leases, not
decisions.

## Pieces

- `bin/export_to_d1.py` — full logical export (schema + rows) from
  Turso over the libsql HTTP API into D1 via the D1 import API, then
  a read-back verification of per-table row counts. Literals in the
  generated SQL are rendered by SQLite's `quote()` at the source.
- `bin/restore_drill.py` — the acceptance drill: reads the D1 copy
  back out through the D1 query API, loads it into a scratch Turso
  store with parameterized statements, and verifies the restored
  counts against the counts the export recorded in the copy's
  `_fleet_queue_backup_meta` table.
- `bin/fq_common.py` — the shared clients and the dump builder.
- `systemd/` — a daily user timer (`fleet-queue-backup-export.timer`,
  02:15 + up to 30 min randomized delay) whose wrapper fetches the
  credentials from Bitwarden by name at runtime. Install with
  `systemd/install-units.sh` from a checkout on the laptop.
- `docs/RUNBOOK.md` — the restore path, the drill procedure, and the
  freshness target.
- `tests/` — hermetic tests; sqlite3 is the oracle. Run:
  `python3 -I tests/test_fq_common.py`,
  `python3 -I tests/test_restore_drill.py`,
  `bash systemd/test-export-arguments.sh`.

## Credentials

Environment only, fetched by name, never written down:
`turso.fleet-queue-rw-token` (export source) and
`cloudflare.api-token-account-wide` (D1). The restore drill takes a
token for its scratch target via `TURSO_TARGET_TOKEN` — never the
production queue token.

## Status against #8584

This directory covers the Turso -> D1 leg. The repo legs live in
`infra/github-backup` (R2 bundles, plain org mirrors and their
schedules). Still outstanding for #8584: the Cloudflare rehost leg
and the unified five-copy freshness/lag readout with automatic issue
filing; both are recorded on the issue.
