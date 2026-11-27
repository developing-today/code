# VAL-01 — Graph and journal migration, rollback, quarantine and privacy validation

Issue: [#9498](https://github.com/developing-today/code/issues/9498)
(slice `VAL-01` of [#9208](https://github.com/developing-today/code/issues/9208),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the VAL-01 row of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225): "validate additive graph/journal migration/backfill,
rollback, cycle quarantine, event preservation, privacy and Project
ownership", first oracle
`pmo_val_legacy_blockers_and_events_migrate_or_quarantine_with_rollback`
("ambiguous links/events, upgrade, write, disable/rollback and
prior-data read"), negatives "ambiguous ref, cycle, missing principal,
telemetry mislabeled journal, private leak, rollback after remote
effect". It decides what the migration reads, what it writes, in what
phases, what parity must hold before each phase, and what a rollback
keeps. It also reports one conflict between the dependency records
and the store that has since landed on main (§3), because a
validation plan that ignored it would validate the wrong migration.
It authorizes no code, schema, migration or deployment (§10).

Governing decisions consumed, not re-opened:

- **CEN-04** (`doc/2026-11-23T13-51-45Z_design_cen04_schema_owner_migration_compatibility/`):
  §3.5 free-text block reasons are never parsed into edges; §3.6
  `events` is **not** the journal and no journal table exists; §3.7 a
  checkpoint is not a note; §3.10 GitHub is the system of record for
  Project fields, `pm_metadata_items` is a rebuildable projection; §5
  the field-admission gate (a second home for one fact is a conflict
  row, and nothing is written until it resolves); §7 expand → backfill
  → dual-read with the old reader authoritative → cutover → contract,
  and rollback disables only.
- **DEP-03** (`doc/2026-11-25T01-14-01Z_design_dep03_hard_dependency_dag/`):
  cycle-safe insert; imports go through the same check in a stable
  order and a refused edge is written `quarantined` with
  `reason_code = imported` and the cycle path, never dropped; the
  broker never chooses which edge of a cycle is wrong; converted
  legacy rows start `certainty = asserted`.
- **DEP-04** (`doc/2026-11-25T01-16-35Z_design_dep04_relation_types_native_links/`):
  only `blocks` gates and resolves; no relation writes state; native
  import runs `off` → `shadow` → `enforce` per repository.
- **PRI-04** (`doc/2026-11-25T01-11-48Z_design_pri04_dependency_pressure/`):
  owns `work/relations.rs` first (shared-file edge); a quarantined
  edge contributes no pressure.
- **JRN-02** (`doc/2026-11-25T21-59-00Z_design_jrn02_private_note_status_reason/`):
  `task_notes.visibility` defaults `private`; reads are scoped; a
  request actor field never widens them.
- **AUTH-02** (`doc/2026-11-25T22-05-00Z_design_auth02_public_mirror_consent/`):
  nothing private leaves Fleet without a single-use consent bound to
  one note revision.
- **JRN-03** (`work/journal.rs` on main): the journal is a pure model;
  checkpoint and handoff entries carry `HydrationRef`/`CursorRef`.

## Verified state on main

Read at `origin/main` `bc3303580` on 2026-11-25:

- **Q6 normalized stores landed** (#14170, commit `98786e972`,
  `turso_queue/core.rs` and `turso_queue/links.rs`), after the DEP-03
  and DEP-04 records were written:
  - `task_deps` (`task_id`, `block_id`, `dep_kind` =
    `blocked-by-issue|pr|task|ci`, `dep_key`, `ref_repo`,
    `ref_number`, `ref_task_id`, `state` ∈ `open|cleared|malformed`,
    `created_at`, `cleared_at`, `cleared_by`, `detail`;
    `UNIQUE(task_id, block_id, dep_key)`).
  - `queue_links` (kinds `implements`, `pr_for`, `parent`, `child`,
    `split_from`, `supersedes`, `duplicate_of`, `related`; tombstoned
    by `removed_at`/`removed_by`; unique active `(task_id, link_kind,
link_key)`).
  - `queue_schema_migrations(name, applied_at, detail)` with the
    marker `q6-links-deps-v1`.
- **The Q6 backfill is the house migration pattern**
  (`links.rs` `ensure_q6_backfill`): skip if the marker exists; scan
  `tasks` by id in batches of 200, each batch one `BEGIN IMMEDIATE`
  transaction; insert with `WHERE NOT EXISTS` so a rerun adds nothing;
  for `blocked` tasks parse `blocked_deps`, and keep anything
  unparseable or untyped as a `malformed` row whose `detail` holds the
  raw value, never dropping it; write the marker last with counts.
  Its tests prove a removed link stays removed across a rerun and that
  a SQL census reproduces the recorded blocker counts.
- `tasks.blocked_deps` and the recheck fields remain, now as the cache
  beside `task_deps`.
- `events` (`/events-append|list|count|seen|prune`) is the broker
  timeline. No journal table exists. `task_notes` exists without a
  `visibility` column.
- `work/relations.rs` does not exist.

## 1. Intent

Moving Fleet from its current blocker and link records to the typed
graph and journal must not unblock a single task by accident, lose a
single event, leak a single private note, or touch a single GitHub
item. If the move is turned off halfway, everything that was true
before is still readable, and anything that already happened on
GitHub stays recorded.

## 2. Ownership

| Concern                                 | Owner                                | This slice                        |
| --------------------------------------- | ------------------------------------ | --------------------------------- |
| Migration and rollback model tests      | `work/relations.rs` (after PRI-04)   | **owns** (tests and fixtures)     |
| Actual versioned migration              | #9198 store owner, Q6 marker table   | consumer; command not guessed     |
| Edge semantics, cycle check, quarantine | DEP-03, DEP-04                       | consumer                          |
| Journal entry model                     | JRN-03                               | consumer                          |
| Note visibility, mirror consent         | JRN-02, AUTH-02                      | consumer                          |
| Project fields                          | GitHub via the single Project writer | consumer; written by nothing here |

## 3. The conflict this validation has to report

DEP-03 §3 proposes a new `dep_edges` table and DEP-04 a `relation`
column on it. Since then Q6 landed `task_deps` for blocking edges and
`queue_links` for the non-gating relations. Building `dep_edges` as
well would give "this task is blocked by that" two homes, which
CEN-04 §5 treats as a conflict row: nothing is written until one home
is chosen.

This record does not choose, but validates against the choice and
recommends one:

- **Recommended:** extend the existing stores. `task_deps` gains
  DEP-03's missing columns additively (`hardness`, `certainty`,
  `reason_code`, `provenance`, `revision`, and the `quarantined`
  state), and a per-authority graph revision; `queue_links` already
  carries DEP-04's non-gating relations (`parent`/`child` →
  `hierarchy`, `duplicate_of` → `duplicates`, `supersedes`,
  `related` → `related_to`). No data moves; the backfill is a column
  fill.
- **Alternative:** `dep_edges` as DEP-03 wrote it, with `task_deps`
  becoming the legacy side of a CEN-04 §7 move and `queue_links`
  staying the home of non-gating links.

Either way the validation below holds; §4 names where the two differ.
The owner of #9443/#9444 and the #9198 store owner decide, and the
DEP-03 record is amended to match before any implementation issue is
filed.

## 4. Phases and what each must prove

The move follows CEN-04 §7 and the Q6 pattern. Each phase is gated by
the parity of the one before.

| Phase        | Writes                                                                 | Must prove before the next phase                                                                                                     |
| ------------ | ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| 1. Expand    | new columns or table, all nullable or defaulted; old writers unchanged | old readers and writers unchanged; every existing test passes                                                                        |
| 2. Backfill  | batched, marker-guarded, `WHERE NOT EXISTS`; one transaction per batch | rerun adds nothing; counts match a SQL census of the source; every source row is accounted for as migrated, quarantined or malformed |
| 3. Dual-read | both readers run; the **old** reader decides claim eligibility         | per task, old and new eligibility and derived priority label agree; every disagreement listed with its cause; none unexplained       |
| 4. Cutover   | the new reader decides; old writes continue                            | owner sign-off on the phase 3 report                                                                                                 |
| 5. Contract  | old shape retired                                                      | separate owner decision; not part of this validation                                                                                 |

Backfill rules:

1. **Blocking edges.** Each `open` `task_deps` row (and, under the
   alternative, each row converted to `dep_edges`) becomes a hard
   `blocks` fact with `reason_code = migrated`, `certainty =
asserted`, inserted through DEP-03's cycle check in the stable
   order (source key, then target id). A refused insert is written
   `quarantined` with the cycle path. A `cleared` row becomes
   `resolved` with its `cleared_at`/`cleared_by` as evidence.
2. **Ambiguous refs.** A `malformed` row, or any reference that names
   a number without a repository, stays `malformed` with its raw value
   and is **not** turned into an edge. Its task keeps whatever legacy
   gate it had, so nothing becomes claimable because its blocker could
   not be read. Free-text reasons are never parsed (CEN-04 §3.5).
3. **Non-gating links.** `queue_links` rows are read in place; any
   copy (alternative only) is a soft, non-gating relation and keeps
   tombstones as tombstones.
4. **Missing principal.** Legacy rows have an actor string (`backfill`,
   `fleet queue link`, a worker id) but no principal. Provenance
   records `principal_ref: null` and `actor_ref: "legacy:<string>"`.
   No principal is invented. Removing or editing such a row needs the
   owner role rather than the original author's.
5. **Events.** `events` rows are read only, never copied into the
   journal, and never pruned by the migration; `/events-prune` is not
   called during phases 2–4. The journal starts empty at cutover with
   one boundary entry that records the migration marker; history
   before it is read from `events` and labelled as the broker
   timeline, never as journal entries.
6. **Notes.** Adding `task_notes.visibility` fills every existing row
   with `private`. No row is set `public`, and no AUTH-02 consent is
   created by the migration.
7. **Project ownership.** The migration makes no GitHub call and no
   Project-field write. `pm_metadata_items` is rebuilt, not migrated.

## 5. Rollback

- **Before cutover:** disable the new reader. The old reader was
  authoritative throughout, so claim behaviour is unchanged. New
  columns or rows stay, unused.
- **After cutover:** switch the reader back. Edges added after cutover
  through new commands are also written to the old shape during
  phases 3–4 (dual write), so the old reader sees them; a
  `quarantined` edge maps to `open` in the old shape, because holding a
  task is the safe direction.
- **After a remote effect** (a completion closed an issue that
  resolved an edge, or a native import recorded a GitHub link): the
  record of it stays. Rollback never reopens, unresolves or deletes a
  row whose state reflects something that happened on GitHub; the old
  reader sees the resolution through its own `cleared` state.
- **Never:** dropping the new table or columns as part of rollback,
  deleting `events`, rewriting `task_deps` history, or resetting the
  migration marker on a live store.

## 6. Tests

In `work/relations.rs` (after PRI-04 creates it), as pure fixtures
over an in-memory store with the Q6 schema:

| Test                                                                     | Asserts                                                                                                               |
| ------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------- |
| `pmo_val_legacy_blockers_and_events_migrate_or_quarantine_with_rollback` | fixture of open, cleared, malformed and cyclic deps plus events: upgrade, write, roll back, read prior data unchanged |
| `pmo_val_backfill_rerun_adds_nothing`                                    | second run after deleting the marker in a test store inserts zero rows                                                |
| `pmo_val_backfill_counts_match_sql_census`                               | migrated + quarantined + malformed equals the source count                                                            |
| `pmo_val_ambiguous_ref_not_converted_and_task_stays_held`                | number without repo stays malformed; task eligibility unchanged                                                       |
| `pmo_val_imported_cycle_quarantined_with_path`                           | cycle among migrated deps → one edge quarantined, path recorded, every member unready                                 |
| `pmo_val_missing_principal_kept_null`                                    | legacy actor kept as `legacy:<string>`; no principal assigned                                                         |
| `pmo_val_events_not_copied_or_pruned`                                    | events count and content identical before and after; journal holds only the boundary entry                            |
| `pmo_val_existing_notes_backfilled_private`                              | every pre-existing note reads `private`; no consent rows                                                              |
| `pmo_val_dual_read_disagreement_reported_not_fixed`                      | a seeded disagreement appears in the report; neither store is edited                                                  |
| `pmo_val_rollback_after_remote_effect_keeps_resolution`                  | resolved-by-close edge stays resolved after rollback; old reader shows `cleared`                                      |
| `pmo_val_migration_makes_no_remote_call`                                 | adapter call count zero across all phases                                                                             |
| `pmo_val_removed_link_stays_removed`                                     | tombstoned `queue_links` row stays tombstoned through every phase                                                     |

Done command (remote runner):

```sh
cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_val_legacy_blockers_and_events_migrate_or_quarantine_with_rollback -- --list
cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_val_
```

The actual DB upgrade and rollback command is the store owner's to
name and run before rollout; it is not guessed here.

## 7. Evidence recorded separately

Per the slice: source tests, the DB migration run, staging, a deployed
canary and a live read are each their own evidence. The phase 3
parity report is the canary's main artifact.

## 8. Persisted fields

None proposed by this slice. The recommended §3 option adds DEP-03's
columns to `task_deps` and a `quarantined` state; that admission is
DEP-03's row under CEN-04 §5, amended, not VAL-01's. The migration
marker name follows Q6 (`<slice>-<what>-v<n>`, e.g. `dep03-typed-deps-v1`).

## 9. Requirement trace (design-level)

| IDs    | Outcome                                                                                                                                                                  | Where      |
| ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------- |
| C-R128 | priority labels derive from the canonical score; the migration changes no score input, so no label moves (checked by phase 3 parity, which also compares derived labels) | §4 phase 3 |
| P-R008 | existing stores reused; journal, events and notes kept as distinct facts                                                                                                 | §3, §4     |

## 10. Gates

- **Blocking owner decision:** §3, the home of typed blocking edges.
- Implementation waits for PRI-04's `relations.rs`, the DEP-03
  amendment, JRN-02's visibility row and the #9198 owner's migration
  path. #9073/#9074 apply only if a metadata schema is touched; this
  plan touches none.
- **Rollback:** §5.

## 11. Non-goals

- No migration file, SQL or marker written.
- No change to Q6's backfill, `task_deps`, `queue_links` or `events`.
- No GitHub or Project write.
- No choice between the two §3 options.
