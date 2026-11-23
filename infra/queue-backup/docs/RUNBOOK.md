# queue-backup runbook

The fleet-queue store is one Turso database (`fleet-queue`, org
dezren39, group great-star-lord) behind the queue broker. Its backup
copy lives in Cloudflare D1 — same SQLite family — where it also serves
as the analytics copy. This runbook covers the export, the restore
drill, and what production wiring remains.

## The path

1. **Source.** A SQLite file of the queue store: the broker host's local
   replica file, copied (never moved) to the export host. `export_d1.py`
   opens it read-only.
2. **Export.** `export_d1.py` writes a SQL dump + manifest. The dump is
   verified standalone (fresh load, identical schema fingerprint and
   per-table counts) before it is written anywhere.
3. **D1 copy.** The dump is loaded into the D1 database. D1 is SQLite;
   a dump that loads into vanilla SQLite with matching counts is the
   fidelity bar, and the manifest is the record of what was loaded.
4. **Restore.** The D1 copy is dumped and loaded into a scratch Turso
   database; the drill verifies counts and schema against the manifest
   and proves a live claim against the restored store.

## The local drill (landed, runs anywhere with python3)

Against any queue-shaped database file — the test fixture is built
from the DDL of record in `turso_queue/core.rs`:

```sh
python3 infra/queue-backup/bin/export_d1.py \
  --source queue-replica.db --out queue-d1.sql
python3 infra/queue-backup/bin/restore_drill.py \
  --dump queue-d1.sql --manifest queue-d1.sql.manifest.json \
  --out restored-scratch.db
```

The drill fails non-zero, naming the check, when: the dump's SHA-256
differs from the manifest; the D1 copy's counts or schema differ from
the manifest; the restored store's counts or schema differ; or no
claimable task can be claimed exactly once (pass `--allow-empty` only
for a store that legitimately holds no claimable task — the report then
records that the claim step was vacuous).

What the local drill does not prove: a broker-mediated claim. Its claim
check runs the claim's conditional-UPDATE core (queued branch) directly
against the restored file. The broker form is step 4 of the production
drill below.

## Production wiring (remaining — operator session)

Account scope is already decided (Drew, 2026-11-16, recorded on
#8584): the fleet's existing Cloudflare account for D1, the existing
Turso org, mirrors under developing-today. In order:

1. Create the D1 database in the existing Cloudflare account with the
   sanctioned `cf` tool; record its name here.
2. Wire the scheduled export on the broker host: copy the queue's local
   replica file, run `export_d1.py`, load the dump into D1. Add the
   timer beside `infra/github-backup`'s systemd units and register it
   in `doc/fleet-schedule-registry.md` in the same change.
3. Production restore drill: create a scratch database in the existing
   Turso org, load the D1 copy's dump into it, start a broker with
   `QUEUE_BROKER_TURSO_URL` pointed at the scratch database, and run
   `fleet queue claim` — the claim must return a task from the restored
   store. Delete the scratch database afterwards.
4. Only after the drill passes: the D1 copy joins the five-copy
   freshness readout (#8584 leg 5) with its lag target.

No step here reads or prints a credential value; tokens move through
the fleet's existing named-secret paths only.
