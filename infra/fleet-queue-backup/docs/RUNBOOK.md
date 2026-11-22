# fleet-queue-backup runbook

## What is backed up, and how fresh it must be

The fleet-queue Turso database (`fleet-queue-dezren39`, the queue
broker's coordination store) is exported in full to the D1 database
`fleet-queue-backup` in the fleet's Cloudflare account, daily at
02:15 (plus up to 30 min randomized delay) by
`fleet-queue-backup-export.timer` on the laptop.

Freshness target: the D1 copy's `_fleet_queue_backup_meta` row
(key `export`) is never more than 24 hours old. The unified
five-copy freshness readout that files an issue automatically when
a copy misses its target is still outstanding on #8584; until it
lands, a stale copy is spotted by the export service's own failure
state (`systemctl --user status fleet-queue-backup-export.service`)
and by reading the meta row:

```sql
SELECT value FROM _fleet_queue_backup_meta WHERE key = 'export';
```

## The export, by hand

From a checkout on the laptop (credentials are fetched by name at
runtime; nothing else to configure):

```bash
systemctl --user start fleet-queue-backup-export.service
```

or, without systemd, `systemd/run-export.sh`. A successful run ends
with `read-back counts match`; anything else is a failed export and
the previous D1 copy is left as it was (the import replaces table
contents only as a whole verified dump — see below for what a
partial failure means).

Provisioning the D1 database the first time is an explicit act:
`bin/export_to_d1.py --create` (with the three credential variables
exported — see the module header). The scheduled wrapper never
passes `--create`.

## Restore: D1 copy back into a live Turso store

This is the disaster path for a lost or corrupted queue database.
Per the #8575 wipe test, the restored store only needs to serve
coordination again — durable truth is on GitHub, and lanes re-claim
in-flight work — but the restore is still verified like it matters,
because a silently wrong coordination store double-claims work.

1. Create the target database in the existing Turso org (a fresh
   name for a disaster; `fleet-queue-restore-drill` for a drill):

   ```bash
   turso db create fleet-queue-restored
   turso db show fleet-queue-restored   # note the URL
   turso db tokens create fleet-queue-restored  # a write token
   ```

2. Run the drill tool against it — despite the name it is the
   restore: it rebuilds every table from the D1 copy and verifies
   counts against the export's recorded counts.

   ```bash
   TURSO_TARGET_TOKEN=<target token> \
   CLOUDFLARE_API_TOKEN=<account token> \
   CLOUDFLARE_ACCOUNT_ID=<account id> \
     python3 bin/restore_drill.py \
       --target-url https://fleet-queue-restored-dezren39.aws-us-east-1.turso.io
   ```

   It must end with `target counts match the export`. If it does
   not, the restored store is not trustworthy: destroy it and
   investigate the D1 copy before retrying.

3. Prove the restored store serves the real code path — a live
   claim through a broker pointed at it (scratch port, scratch
   token file; never the production broker's):

   ```bash
   printf '%s' '<target token>' > ~/restore-drill/turso-token
   printf '%s' '<any drill token>' > ~/restore-drill/broker-token
   QUEUE_BROKER_BIND=127.0.0.1:4899 \
   QUEUE_BROKER_SOCKET=~/restore-drill/broker.sock \
   QUEUE_BROKER_TOKEN_FILE=~/restore-drill/broker-token \
   QUEUE_BROKER_TURSO_URL=https://fleet-queue-restored-dezren39.aws-us-east-1.turso.io \
   QUEUE_BROKER_TURSO_TOKEN_FILE=~/restore-drill/turso-token \
     ~/tools/fleet serve queue-broker &
   REPO_QUEUE_BROKER_URL=http://127.0.0.1:4899 \
   QUEUE_BROKER_TOKEN_FILE=~/restore-drill/broker-token \
     ~/tools/fleet queue claim --worker restore-drill
   REPO_QUEUE_BROKER_URL=http://127.0.0.1:4899 \
   QUEUE_BROKER_TOKEN_FILE=~/restore-drill/broker-token \
     ~/tools/fleet queue list --status claimed
   ```

   The claim must succeed and the claimed task must be visible in
   the listing. Then stop the scratch broker and release or leave
   the drill claim to expire — it exists only in the restored
   store.

4. Disaster only: repoint the production broker at the restored
   database (its `QUEUE_BROKER_TURSO_URL` configuration) and restart
   it. For a drill, destroy the scratch database instead:
   `turso db destroy fleet-queue-restore-drill`.

## Failure notes

- Export fails at the D1 import step: D1 imports are whole-file;
  a failed import leaves the previous copy in place. Fix the cause
  (usually credentials or a schema object D1 rejects) and rerun —
  the dump drops and recreates its objects, so reruns are safe.
- A table's row count differs between source and read-back: the
  export reports the mismatching tables by name and exits non-zero.
  Do not treat the copy as current until a run verifies clean.
- The queue schema changes (a fleet-queue migration): the export is
  schema-agnostic — it copies whatever `sqlite_master` holds — so
  no change is needed here; the first export after a migration
  carries the new schema.
