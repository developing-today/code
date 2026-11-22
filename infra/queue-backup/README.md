# queue-backup

Backup path for the fleet-queue store — the Turso (libSQL) database the
queue broker serves (#6741) — into Cloudflare D1, with a drilled restore
path back. This is leg 3 of the fleet unification backups slice
(developing-today/code#8584); the repo-side legs (R2 bundles, plain
mirrors) live in `infra/github-backup`.

Layout:

- `bin/export_d1.py` — export a queue-store SQLite file to a D1-loadable
  SQL dump plus a manifest (per-table counts, schema fingerprint, dump
  SHA-256). The export self-verifies: the dump must load into a fresh
  vanilla SQLite database with identical counts and schema before
  anything is written.
- `bin/restore_drill.py` — the restore drill: manifest/dump integrity,
  D1-copy load, restore into a scratch store, count + schema checks, and
  a claim check that runs the queue's claim shape (the conditional
  UPDATE of `turso_queue/claim.rs`) against the restored store.
- `bin/queue_backup_common.py` — shared counts / schema / dump helpers.
- `tests/` — self-executing suites:
  `python3 -I infra/queue-backup/tests/test_export_d1.py` and
  `python3 -I infra/queue-backup/tests/test_restore_drill.py`.
- `docs/RUNBOOK.md` — the operating path, the local drill, and the
  production wiring steps.

Status: the tooling and the local drill are landed and green. The
production wiring (creating the D1 database in the fleet's existing
Cloudflare account, the scheduled export, and the broker-mediated claim
against a real scratch Turso database) is an operator session, itemized
in the runbook — no live infrastructure is changed by anything here.
