# Restore / recreate procedure for selective exports

**Scope:** any restore that rebuilds a store from a _selective_
(per-table) export — a hand-built manifest, a partial dump, a
table-by-table copy — into a fresh store. Whole-database moves
(file copy, full dump/restore, provider-level replication) are out
of scope: they carry every table by construction and cannot fail
the way this procedure guards against.

**Why this exists:** the 2026-11-07 fleet-queue restore-completeness
audit (issue #6853, worked example below). A selective export listed
five tables; the lanes registry table — a reference table in the
same queue core — was never exported, so the recreated store ran a
full day with an empty lane registry and every lane claim rejected
(unknown lane_id). The load matched the export exactly because the
export itself was incomplete, and nothing in the procedure caught
the omission. The four steps below are the fix, ordered so the
omission becomes impossible to miss: inventory first, reference
tables named, a runnable diff gate after load, additive-only
backfill for whatever the gate finds.

Tooling: scripts/fleet/restore-row-count-diff.sh (the gate) and
scripts/fleet/restore-row-count-queries.sql (the inventory/count
query set for SQLite/Turso and PostgreSQL sources).

## Step 1 — Source-inventory-first checklist

Before any export is built:

1. Enumerate **every** table in the source scope.
   - PostgreSQL: the information_schema.tables query in the query
     set (or the COPY headers of a full dump).
   - SQLite/Turso: the sqlite_schema query in the query set.
2. Record each table in the export manifest with its exact source
   row count (count(*), never a planner estimate).
3. Give **every** source table an explicit disposition:
   - load — copied into the new store;
   - archive — not loaded; the note names the destination it
     survives in (the by-design history archive class);
   - retire — not loaded; the note names the reason (superseded
     by, duplicate of, by-design empty).
4. A source table missing from the manifest **fails the export
   step**. Silence is not a disposition. The gate enforces this
   mechanically when given the inventory file (step 3).

Manifest format (TSV, header required) — the same file feeds the
export, the second-reader check, and the gate. Columns: table,
source_count, disposition, allowed_delta, note.

## Step 2 — Reference/config tables are named explicitly

Registries and trackers are the highest-risk omission class: data
tables load fine without them, and the failure surfaces later,
somewhere else (here: a day of rejected claims for one missing
registry). The manifest review names this class explicitly. A table
is reference/config class if losing it breaks other tables' or
other systems' behaviour while its own absence throws no error at
load time. The known members, by family:

- **Lane / worker registries** — the lane registry (the #6853
  omission), worker registrations.
- **Token / credential trackers** — the token-expirations tracker
  (names and dates only, never secret values), credential lifecycle
  tables.
- **Watermarks and cursors** — sync watermarks, last-seen markers,
  ingestion checkpoints.
- **Feature / config tables** — provider registries, quota-record
  tables, policy and settings tables.

**Second-reader check (required):** a second person or a second
lane reads the manifest against the source inventory — every
inventory row has a disposition, and every reference/config-class
table is either load or carries an archive/retire note the reader
accepts. The reader records their name/lane on the manifest PR or
the cutover issue. One pair of eyes built the export; the failure
mode is exactly what that pair of eyes did not see.

## Step 3 — Post-restore row-count diff gate

After the load, before the cutover is declared complete:

1. Take live per-table counts from the new store in one pass (query
   set, one counts leg per manifest table).
2. Run the gate:

   scripts/fleet/restore-row-count-diff.sh --manifest MANIFEST.tsv --live LIVE.tsv --inventory INVENTORY.txt

3. The gate fails (exit 1) on: an inventory table missing from the
   manifest; an archive/retire row with no note; a load table
   missing from the live counts; a live count short of source by
   more than the row's allowed_delta; a table non-empty in the
   source and empty live. Live counts **above** source are reported
   as info — post-cutover writes are expected, never a failure.

**Explained-delta allowlist.** allowed_delta is per-table, set
before the gate runs, and the note carries the explanation. Known
legitimate classes:

- **Post-cutover writes** — normally make live at least source; a
  delta is only needed when the snapshot predates a known purge.
- **By-design archives** — tables whose rows live elsewhere by
  disposition (the history-archive class named in the issue).
- **Pre-rename duplicate keys** — rows the load deduplicated under
  a rename, counted once live against twice in the source.

Any table non-empty in the source and empty or unexplained-short in
the target **blocks the declaration**: no cutover-complete until
the gate passes or the delta joins the allowlist with a written
explanation a second reader accepts (step 2's reader, same pass).

## Step 4 — Backfill is additive-only

Gaps the gate finds are backfilled from the snapshot with
INSERT OR IGNORE semantics only:

- no deletes, no overwrites, no sync that can shrink the target;
- record before/after counts for every backfilled table in the
  cutover record (precedent: the lanes backfill, 32 to 47 rows,
  zero deletions);
- re-run the gate after backfill; it must pass before the cutover
  is declared complete.

## Worked example — the 2026-11-07 audit (#6853)

The hand-built export manifest listed five tables (tasks, leases,
workers, checkpoints, and the lane-ledger table). The source
inventory it was never checked against also contained the lane
registry (47 rows after backfill; the registry every claim
validates against) and the token-expirations tracker (10 rows, no
Turso home anywhere at audit time). Under this procedure: step 1
puts both in the manifest before the export exists; step 2 names
both as reference/config class; step 3's gate fails on the registry
empty live — the exact production symptom, caught at the gate
instead of a day later. The token-expirations tracker was since
loaded into Turso fleet-platform as agent_token_expirations (10 of
10 rows from the 2026-11-06 snapshot, readback-verified; disposition
recorded on issue #6994, 2026-11-08) — the disposition step 1 would
have recorded up front.

## Related docs

- doc/fleet/queue-turso-cutover.md — the cutover this restore
  belonged to; its whole-database moves are the out-of-scope class
  above.
- doc/fleet/turso-databases.md — which store holds what; the
  manifest's archive destinations name stores from this list.
- hatch-personal backup side (noted on issue #6994, task 11467):
  the sandbox-state-backup and sandbox-home-backup script sets are
  snapshot jobs with no selective-restore procedure of their own.
  If a selective restore is ever run from those snapshots, it
  follows this procedure; a pointer belongs in those scripts'
  headers when they are next touched.
