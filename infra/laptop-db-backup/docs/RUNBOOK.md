# laptop-db-backup runbook

## Provision a new leg (per database)

1. Confirm the database qualifies under the mesh design §6 and is
   not already covered by an existing leg (the #13459 inventory
   records the covering leg per database; check the covering script
   itself, not just the inventory — coverage claims go stale).
2. Create the home in the fleet's Turso group:
   `turso db create laptop-backup-<name> --group great-star-lord --tursodb -w`
3. Mint the home's token and file it in Bitwarden Secrets by name:
   `turso db tokens create laptop-backup-<name>`, stored as
   `turso.laptop-backup-<name>-rw-token` with a `[sync-metadata v1]`
   note. The token value is never printed, committed, or written to
   any file on disk.
4. Add the leg's wrapper (`systemd/run-<leg>-import.sh`, a copy of
   the pm-projector wrapper with the source path, home URL, and
   token name changed), its `.service`/`.timer` pair (weekly,
   staggered against the other mesh timers), and a row in the
   README's leg table and in `doc/fleet-schedule-registry.md` — all
   in the implementation PR.
5. After the PR merges: clone (or update) the service-owned
   checkout at `~/.local/share/laptop-db-backup/repo`, run
   `systemd/install-units.sh` from it, start the service once for
   the first import, and record the per-table count receipt on the
   leg's issue.
6. Run the restore drill (below) against the new home and record
   its receipt on the issue. The leg is not done until the drill
   verifies.

## Restore a database from its home

The home is a faithful SQL copy of the source at the last import.
To rebuild a local file from it:

```
TURSO_HOME_TOKEN=<the home's token, fetched by name> \
python3 bin/restore_drill.py \
  --home-url https://laptop-backup-<name>-dezren39.aws-us-east-1.turso.io \
  --home-name laptop-backup-<name> \
  --out /path/to/restored.db
```

The drill verifies the rebuilt file's per-table counts against the
home's import record as it goes; a receipt with `"verified": true`
means the file on disk is the recorded copy. Restore into a scratch
path first and swap it into place deliberately — never drill (or
restore) directly over the live source path.

For a point-in-time finer than the last weekly import, the home is
a regular Turso database: Turso PITR (30 days) applies to the home
itself.

## Drill cadence

The first drill is part of provisioning (above). The recurring
rotation — one leg × one store per week, every combination at least
quarterly — and the watchdog's mesh reporting are owned by #13463.

## Failure reading

- Wrapper exits naming a missing Bitwarden entry: the home was
  created but its token was never filed (step 3). File it; do not
  work around it with a token on disk.
- Import fails at "staging count mismatch" or "post-swap count
  mismatch": the run recorded nothing; the previous import is still
  the home's live copy. Re-run; a repeat failure is a real defect —
  file it with the receipt.
- Service fails in `update-checkout.sh` refusing a dirty checkout:
  someone worked in `~/.local/share/laptop-db-backup/repo`. Move
  the work to a lane worktree and clean the checkout; never commit
  from it.
- A run reports `recovered_interrupted_swap: true`: a previous run
  died mid-swap and this run reconciled it (rolled back to the
  retiring set, or finished a committed swap's cleanup). The
  receipt's counts are the new import's; no action needed unless it
  repeats.
