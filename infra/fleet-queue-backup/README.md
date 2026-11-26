# fleet-queue-backup

The backup leg of the fleet-queue store, built for issue #8584
(slice 6 of the fleet unification design, #8575): a scheduled export
of the fleet-queue Turso database into a Cloudflare D1 database —
same SQLite family, restorable — with a drilled restore path back
into Turso. The D1 copy also serves as the analytics copy.

**Name contract:** the live queue database is `fleet-queue` (Turso
today; D1 after CF-2's verified cutover). The separate backup D1
database is `fleet-queue-backup`; it remains the backup copy after
that cutover.

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
- `bin/export_to_turso.py` — the reverse leg (backup mesh class D,
  #13458): a weekly export of the `fleet-queue-backup` D1 database
  into its dedicated Turso home `d1-backup-fleet-queue-backup`,
  staged and wholesale-replacing, with the import record kept in
  the home's `_d1_backup_home_meta` table. A failed import leaves
  the previous import intact.
- `bin/restore_drill_d1_home.py` — the class-D acceptance drill:
  rebuilds the D1 database from its Turso home into a local SQLite
  file and verifies counts against the home's import record.
- `systemd/` — a daily user timer (`fleet-queue-backup-export.timer`,
  02:15 + up to 30 min randomized delay) and a weekly one
  (`fleet-queue-backup-d1-home.timer`, Sundays 03:40) whose wrappers
  fetch the credentials from Bitwarden by name at runtime. Both
  services first run `systemd/update-checkout.sh` as ExecStartPre,
  which brings the service-owned checkout they were installed from
  to origin/main (refusing a dirty tree; warning and continuing on a
  fetch failure), so the units follow main and never depend on a
  working directory. Install with `systemd/install-units.sh` from a
  dedicated checkout on the laptop.
- `docs/RUNBOOK.md` — the restore path, the drill procedure, and the
  freshness target.
- `tests/` — hermetic tests; sqlite3 is the oracle. Run:
  `python3 -I tests/test_fq_common.py`,
  `python3 -I tests/test_restore_drill.py`,
  `python3 -I tests/test_export_to_turso.py`,
  `python3 -I tests/test_restore_drill_d1_home.py`,
  `bash systemd/test-export-arguments.sh`,
  `bash systemd/test-update-checkout.sh`.

## Credentials

Environment only, fetched by name, never written down:
`turso.fleet-queue-rw-token` (export source) and
`cloudflare.api-token-account-wide` (D1). The restore drill takes a
token for its scratch target via `TURSO_TARGET_TOKEN` — never the
production queue token. The class-D leg takes the home's token via
`TURSO_HOME_TOKEN` (`turso.d1-backup-fleet-queue-backup-rw-token`,
minted when the home is provisioned; see the runbook).

## Status against #8584

This directory covers the Turso -> D1 leg. The repo legs live in
`infra/github-backup` (R2 bundles, plain org mirrors and their
schedules). Still outstanding for #8584: the Cloudflare rehost leg
and the unified five-copy freshness/lag readout with automatic issue
filing; both are recorded on the issue.

## Status against #13458 (mesh class D)

The D1 -> Turso home leg's code, weekly units, and hermetic tests
are in this directory; the home database itself is provisioned in
that issue's live leg (create `d1-backup-fleet-queue-backup`, mint
its token into the vault, install the units, first verified import

- drill), which is recorded on #13458.
