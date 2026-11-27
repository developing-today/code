# laptop-db-backup

The backup-mesh class-E legs (#13474, from the #13459 inventory;
design #9093, doc/2026-11-23T16-34-18Z_design_backup_mesh §6): a
qualifying laptop-local SQLite database gets a dedicated Turso home
(`laptop-backup-<name>`), itself class B, filled by a weekly
wholesale-replace import. A database qualifies when the data is
SQL-shaped, the size is modest (hundreds of MB, not tens of GB), it
has no other off-laptop copy, and wholesale-replace semantics are
acceptable — the home is a backup target, never a live read path.

The import discipline mirrors the class-D leg
(infra/fleet-queue-backup, #13458): the copy is loaded into staging
tables and verified before anything live moves; the swap is
witnessed by a marker whose import_id must equal the import
record's; counts in the record are the rows actually loaded; a run
that finds an interrupted swap reconciles it before doing anything
else. The one class-E difference: the source is read through a
consistent snapshot taken with the SQLite backup API, so the live
database moving mid-run cannot smear the copy.

## Pieces

- `bin/ldb_common.py` — the libsql HTTP pipeline client and schema
  helpers shared by the import and the drill.
- `bin/import_to_turso.py` — one import run: snapshot the source,
  stage, verify, swap, record. Prints the per-table count receipt
  as JSON.
- `bin/restore_drill.py` — the acceptance drill (mesh §9): rebuilds
  the home into a scratch local SQLite file and verifies the
  restored counts against the home's import record. Never writes to
  the source or the home.
- `systemd/` — one weekly user timer per leg. The wrapper
  (`run-<leg>-import.sh`) fetches the home's token from Bitwarden by
  name at runtime; the service first brings the service-owned
  checkout to origin/main (`update-checkout.sh`). Install with
  `systemd/install-units.sh` from a checkout on the laptop.
- `docs/RUNBOOK.md` — provisioning a new leg's home, the restore
  path, and the drill procedure.
- `tests/` — hermetic tests; sqlite3 is the oracle. Run:
  `python3 -I tests/test_import_to_turso.py`,
  `python3 -I tests/test_restore_drill.py`.

## Legs

| Source (laptop-local)                      | Home                           | Token (Bitwarden name)                        | Schedule                                                                               |
| ------------------------------------------ | ------------------------------ | --------------------------------------------- | -------------------------------------------------------------------------------------- |
| `~/.local/state/pm-projector/projector.db` | `laptop-backup-pm-projector`   | `turso.laptop-backup-pm-projector-rw-token`   | `laptop-db-backup-pm-projector.timer`, Sundays 05:40 + up to 30 min randomized delay   |
| `~/.local/state/fleet/metadata.db`         | `laptop-backup-fleet-metadata` | `turso.laptop-backup-fleet-metadata-rw-token` | `laptop-db-backup-fleet-metadata.timer`, Sundays 06:20 + up to 30 min randomized delay |

The remaining qualifying databases from the #13459 inventory
(user-prompts.db, t3code statev2.sqlite,
ready-for-agent.db) get their own legs under #13474 by the same
pattern: one home, one token, one wrapper, one timer each.

## Credentials

Environment only, fetched by name, never written down: each leg's
wrapper reads its home's token (`turso.laptop-backup-<name>-rw-token`,
minted when the home is provisioned; see the runbook) from Bitwarden
and exports it as TURSO_HOME_TOKEN to the import process tree only.
