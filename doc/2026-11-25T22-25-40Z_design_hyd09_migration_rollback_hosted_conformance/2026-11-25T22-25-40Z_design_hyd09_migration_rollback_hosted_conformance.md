# HYD-09 — Migration, performance, rollback, and hosted conformance

Design record for
[#9462](https://github.com/developing-today/code/issues/9462)
(`pmo-9196:HYD-09`), slice HYD-09 of
`components/9207-context-hydration-incremental-activity.md` in
`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`.

The slice's first failing oracle is
`pmo_hyd_migration_rollback_preserves_generation_and_unknown_cursor`,
which "models upgrade→write→disable/rollback→read prior generation".
This record says what that model is, which states it must survive,
what the four named negatives do, what "hosted parity" compares, and
how the six acceptance stages are recorded apart from each other.

This is a design record. It authorizes no code, schema, migration,
rollout, schedule or live run. The slice text says a DB migration
command "must be cited by the actual migration owner before rollout;
do not guess it". This record names no such command.

## Governing decisions consumed

- **Component 9207 V3 text**, quoted: persistent cursor or index
  additions "must be additive behind existing Fleet storage owner";
  migration "includes backfill as 'unknown cursor' (trigger full
  initial fetch), dual-read/compare if needed, explicit rollback
  retaining prior tables/bundles and ignoring new records"; "Do not
  delete legacy state until rollback window and current authority
  approve"; "Recovery from partial migration leaves old reader
  authoritative; write path must not create two independent truths."
- **Component 9207 V4 — hosted parity**, quoted: "Storage internals
  may vary; identity, manifest, cursor, completeness and attribution
  semantics may not. Report bytes, pages, peak memory, elapsed time,
  completeness/partial rate, and incremental bytes vs full baseline."
- **HYD-01** (record in open PR #15184, head `1841ee59`): manifest
  `fleet.context-manifest/v1`; typed top-level errors only for unknown
  `schema_version`, an invalid digest or path, and identity failure;
  old artifacts stay readable when the serializer is disabled.
- **HYD-02** (merged): repo-scoped source keys and traversal-safe
  artifact paths.
- **HYD-03** (merged): layout
  `<root>/<work-key>/context/{staging/, generations/<gid>/, current.json, publish.lock}`;
  pointer `fleet.context-pointer/v1` with `publication_seq` (CAS),
  `current` and `last_complete`; an append-only publication log;
  two candidate backends (local filesystem, broker pointer row);
  the orphan table (staging orphan, unpublished, rejected, published,
  pinned); "Nothing is deleted by HYD-03"; a pointer that fails
  validation is a typed read error and the reader falls back to
  `last_complete`; "Repair is a fresh publication."
- **HYD-04** (merged): one budget object per hydration, proposed
  defaults 200 requests, 120 s, 64 MiB total, 16 MiB per source,
  8 MiB diff; outcome mapping for each source.
- **HYD-05** (merged): cursor key (resource, endpoint, query shape,
  API version fingerprint, auth-scope class, generation); states
  absent / in-progress / complete / unknown; unknown forces a full
  snapshot; legacy `last_checked_at` "is never read as, converted
  into, or used to fabricate a cursor"; "Rollback is disable-only".
- **HYD-06, HYD-07, HYD-08** (merged): revisions and tombstones,
  attribution classes, bounded refresh/watch/reconcile with no
  scheduler and recurring work only through
  `doc/fleet-schedule-registry.md`.
- **CEN-04** (merged): §3.8 cursors have no home and are "absent —
  gated"; §3.14 PM stores; §5 field-admission gate; §7 migration
  shape "expand → backfill → dual-read with the old reader
  authoritative until parity is evidenced → cutover → contract", and
  "Rollback disables the new capability only".

## Verified state on `origin/main` `8bd1405a`

- `pkgs/fleet/crates/fleet-queue/src/work/context.rs` does not exist.
  `work/` holds `authority.rs`, `completion.rs`, `contracts.rs`,
  `degraded.rs`, `journal.rs`, `lifecycle.rs`, `operations.rs`,
  `priority.rs`, `routing.rs`, `selection.rs`, `sqlite_replica.rs`.
- No code on main writes or reads `fleet.context-pointer/v1`; the
  string appears only in the HYD-03 record.
- No cursor table, column or file exists (CEN-04 §3.8).
- `tasks.last_checked_at` exists and is a park-recheck field, not an
  ingestion cursor (CEN-04 §3.8).
- Hosted surfaces: #9081 (fleet tool, Workers edition) is closed with
  its design merged in PR #12593 and implementation sub-issues
  #12583–#12592; #9079 (PM service, Workers edition) is open. A
  search of `pm-stack/` and the worker packages on main finds no
  hydration command. So no hosted hydration surface exists to run
  parity against today.
- HYD-01's record is in open PR #15184. HYD-02 through HYD-08 records
  are merged. CEN-04 #9434's record is merged; the issue is open.

## 1. Intent

Every earlier HYD slice states its own compatibility rule. HYD-09
puts them on one timeline and tests that they hold together:

- a work with only legacy state is upgraded;
- generations are published and cursors committed;
- the capability is disabled (the only rollback);
- readers still get the generation they could see before;
- re-enabling never trusts what it cannot validate.

It also fixes what "the same contract on a hosted surface" means, and
how performance is measured so that hosted and local runs, and full
and incremental runs, can be compared.

## 2. Ownership

| Concern                                                               | Owner                                                         | HYD-09's part                                    |
| --------------------------------------------------------------------- | ------------------------------------------------------------- | ------------------------------------------------ |
| Migration and rollback model, its oracles, component docs             | **HYD-09 (this)**, in `work/context.rs`'s test module         | owns                                             |
| `assess_store` pure classifier (§4)                                   | **HYD-09**, in `work/context.rs`                              | owns                                             |
| Publisher, pointer, log, orphan table                                 | HYD-03                                                        | consumes; changes nothing                        |
| Cursor key, states, page-atomic commit                                | HYD-05                                                        | consumes; changes nothing                        |
| Cursor, pointer-row and log storage; any DB migration and its command | #9198 storage owner / fleet-queue, admitted through CEN-04 §5 | none; names no table and no command              |
| The on/off switch's home (config key, env, flag)                      | the implementation issue that lands the capability            | states only what "off" must do (§3.3)            |
| Hosted hydration surface                                              | #9079 / #9081 lines                                           | defines the parity comparison (§7), runs nothing |
| Any recurring schedule                                                | `doc/fleet-schedule-registry.md` row, its surface's owner     | none; HYD-09 adds no schedule                    |
| Reason-code names                                                     | the `ReasonCode` registry owner (as in HYD-08 §8)             | proposes names (§5)                              |

`work/context.rs` is shared: HYD-01 through HYD-08 add to it first.
The issue's shared-file edge makes HYD-09 land after HYD-08's
implementation, so HYD-09's tests are written against the publisher,
cursor store and reader those slices land, through their test fakes.

## 3. The migration model

### 3.1 States of one work's hydration store

| State      | What is stored                                                               | Who reads                                                                 |
| ---------- | ---------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| `legacy`   | no pointer, no log, no Fleet cursor; legacy fields such as `last_checked_at` | the prior reader only                                                     |
| `enabled`  | pointer, log, generations, cursors as HYD-03 and HYD-05 define them          | the new reader; the prior reader is untouched                             |
| `disabled` | exactly what `enabled` left; nothing removed, nothing rewritten              | the new reader for published generations only; the prior reader as before |
| `partial`  | some of the `enabled` objects exist and others do not (§4)                   | the prior reader is authoritative                                         |

`disabled` is not a different store. It is the `enabled` store with
the write side off. That is the whole of rollback.

### 3.2 Upgrade

- Upgrade writes nothing by itself. A work enters `enabled` at its
  first successful publication (HYD-03 sequence `"1"`), so an upgrade
  that is never used leaves the store byte-identical.
- Every cursor read under the new code at upgrade time is `absent` or
  `unknown`. A stored value of any pre-design shape is `unknown`
  (HYD-05 §8). There is no conversion path.
- `last_checked_at`, or any other legacy timestamp, is never read by
  the new write path. It is not a cursor, not a high-water mark and
  not a `since` default. The prior reader keeps using it as before.
- The first walk after upgrade is therefore a full snapshot for each
  resource and endpoint. This is the component's "backfill as unknown
  cursor".

### 3.3 Disable (rollback)

"Off" means, checked once at command start:

- no new generation is staged or published;
- no cursor is committed;
- no orphan, generation, pointer, log entry or cursor is moved,
  rewritten or deleted;
- reads of already published generations keep working, through
  `current`, `last_complete` and any checkpoint's `HydrationRef`
  (HYD-03 §5), because HYD-01 keeps old artifacts readable;
- callers that used the prior read path get exactly its prior
  behavior and outputs.

A command that would write while off returns a typed refusal before
any read of GitHub (proposed reason `hydration_disabled`, §5). It
never falls back to writing through another path, which would be the
"second independent truth" the component forbids.

A downgraded binary that does not know the pointer or cursor schema
must not read or write them. If it ships before the read side exists,
there is nothing for it to read; if it is older than the read side, it
ignores the new objects by construction. HYD-09 adds no code to old
binaries.

### 3.4 Re-enable

- The pointer, log and generations are used as they stand. The next
  publication CASes on the stored `publication_seq`.
- Each cursor is validated by HYD-05's rules before use. A cursor whose
  fingerprint differs from the one the re-enabled code would issue (the
  query shape or API version moved while the capability was off, or a
  different binary is running) is `unknown`, and that resource and
  endpoint restart as a full snapshot.
- A cursor that was `unknown` before the disable stays `unknown`. No
  step of disable or re-enable promotes a cursor to `in-progress` or
  `complete`.
- Time spent disabled is not "covered". A `complete` cursor already
  makes the next refresh start a new generation (HYD-05 §2.2), so
  activity during the gap is read by that refresh. HYD-09 adds nothing
  there.

### 3.5 The first oracle, step by step

`pmo_hyd_migration_rollback_preserves_generation_and_unknown_cursor`,
in `work/context.rs`, over the in-memory test backend of HYD-03 and
HYD-05 and a fake adapter:

1. **Legacy.** One work with `last_checked_at` set and a stored cursor
   blob of a pre-design shape. Assert `assess_store` gives `legacy`,
   and the cursor reads `unknown`.
2. **Upgrade.** Assert the store is byte-identical to step 1.
3. **Write 1.** Hydrate with every source answering. Assert a full
   snapshot (the adapter saw first-page requests only, never the
   legacy blob), generation `g1` published at sequence `"1"` with
   status `complete`, cursors `complete` under `g1`, and
   `last_checked_at` unchanged.
4. **Write 2.** Hydrate again with one source returning 502. Assert
   `g2` at `"2"`, `current = g2 / partial`, `last_complete = g1`.
5. **Disable.** Snapshot every stored byte. Attempt a hydrate. Assert
   it is refused with `hydration_disabled`, the adapter saw zero
   requests, and every stored byte equals the snapshot.
6. **Read prior generation.** While disabled, assert `current`
   resolves to `g2`, `last_complete` to `g1`, a checkpoint
   `HydrationRef { generation: "1", manifest_digest: g1's }` resolves
   to `g1` byte for byte, and the prior read path returns the same
   output it returned in step 1.
7. **Re-enable under a new fingerprint.** Change the fake API version.
   Assert every cursor now reads `unknown`, the next hydrate is a
   full snapshot, it publishes `g3` at `"3"`, and `g1` and `g2` are
   still readable and unchanged.

The test is listed first (`-- --list`) and must fail before
implementation; a zero-test filter passing is not acceptance.

## 4. Store assessment and the four negatives

`assess_store(snapshot) -> StoreAssessment` is a pure function over
what one work's store holds. It decides who is authoritative. It
never repairs.

| Observed                                                                               | Assessment                | Reader                                                                     |
| -------------------------------------------------------------------------------------- | ------------------------- | -------------------------------------------------------------------------- |
| no pointer, no log entries, no cursors                                                 | `legacy`                  | prior reader                                                               |
| pointer valid, every log entry up to its sequence present, named manifests verify      | `enabled`                 | new reader                                                                 |
| pointer present, log missing or short of the pointer's sequence                        | `partial: log`            | prior reader; the new write path refuses                                   |
| log entries present, pointer absent                                                    | `partial: pointer`        | prior reader; a log entry beyond the pointer is ignored (HYD-03 §6 rule 3) |
| cursor present with no committed events for it                                         | that cursor `unknown`     | full snapshot for that resource and endpoint (HYD-05 §5.3)                 |
| pointer names a generation whose manifest digest differs, or whose objects are missing | `pointer_object_mismatch` | falls back to `last_complete`, else a typed error                          |

### 4.1 Partial migration

A storage amendment that landed in part (for example the pointer row
exists and the log does not, or cursor rows were copied without their
events) is `partial`. Then:

- the prior reader stays authoritative, as the component requires;
- the new write path does not publish, because HYD-03 §6 says a
  backend without its three properties does not publish (proposed
  reason `store_partially_migrated`);
- nothing is "completed" by HYD-09 code. Finishing or reverting the
  storage step belongs to the migration owner and their cited
  command.

### 4.2 Pointer/object mismatch

The pointer names `g` but `g`'s manifest digest is not the one in the
pointer, or `g`'s directory or object is absent.

- The read returns HYD-01's typed invalid-digest error for `current`
  and falls back to `last_complete` (HYD-03 §8).
- If `last_complete` fails the same way, the result is a typed error
  with no bundle. The reader never scans `generations/` for "the
  newest that verifies"; that would publish something no CAS
  published.
- The pointer is never edited to point elsewhere. Repair is a fresh
  publication.

### 4.3 API drift

- A changed API version header or query shape gives a different
  fingerprint, so the cursor is `unknown` and the walk is a full
  snapshot (HYD-05 §2.3).
- A response body that no longer parses is a source failure under
  HYD-04's outcome mapping, never an empty result and never `complete`.
  The cursor for that source does not advance.
- Published generations from before the drift are not re-validated
  against the new API. They state what they held when sealed.

### 4.4 Deleted referenced artifact

A generation named by `current`, `last_complete`, the log or a
checkpoint's `HydrationRef` has lost an object (a manual delete, a
disk fault, or an unapproved collection).

- The read is a typed error naming the missing object (proposed
  reason `artifact_missing`), never a substitute generation and never
  a regenerated one under the same `publication_seq`.
- A checkpoint whose `HydrationRef` cannot resolve reports exactly
  that. It is not rewritten to point at another generation.
- Collection candidates are computed from HYD-03 §7's table only, so
  a pinned or published generation is never a candidate. HYD-09 adds
  an oracle that the candidate set never contains one; it adds no
  collector.

## 5. Proposed reason codes

For the `ReasonCode` registry owner, as HYD-08 §8 does: none of these
exist on main.

| Code                       | When                                                           |
| -------------------------- | -------------------------------------------------------------- |
| `hydration_disabled`       | a write-side command runs while the capability is off          |
| `store_partially_migrated` | `assess_store` returns a `partial` state                       |
| `artifact_missing`         | a referenced generation's object is absent or fails its digest |

Unknown `schema_version`, an invalid digest or path, and identity
failure keep HYD-01's own error kinds.

## 6. Performance evidence

HYD-09 sets no new threshold. It fixes what is measured, so the
numbers can be compared:

| Field                      | Deterministic in unit/model? | Source                                   |
| -------------------------- | ---------------------------- | ---------------------------------------- |
| requests, by source        | yes (fake adapter counts)    | HYD-04 §5 request budget                 |
| pages, by source           | yes                          | HYD-05 walk                              |
| bytes, by source and total | yes                          | HYD-04 §5 byte budgets                   |
| completeness per source    | yes                          | HYD-01 §4                                |
| incremental vs full bytes  | yes (same fixture twice)     | component V4                             |
| elapsed time               | no                           | staging, deployed and hosted stages only |
| peak memory                | no                           | staging, deployed and hosted stages only |

- One evidence record per run, keyed by fixture, commit, budget
  object version (HYD-04 §5) and backend.
- Unit/model runs assert the deterministic columns exactly against a
  golden per fixture, so a change in request count is a test change,
  not drift.
- Time and memory are recorded, never asserted in unit tests. A gate
  number for them is the release-gate owner's call, made against
  recorded staging evidence, not a number HYD-09 invents.

## 7. Hosted parity

Parity is the same fixtures run through two backends and compared on
semantics, not storage.

**Must be equal:**

- work identity and HYD-02 source keys and artifact paths;
- each generation's `fleet.context-manifest/v1` bytes and digest for
  the same fixture and budget (HYD-04's summary is deterministic);
- the pointer's logical fields: `publication_seq`, `current`,
  `last_complete`, and their order of change across a fixture run;
- each cursor's key and state after each step (continuation values
  are the server's and may differ by host);
- per-source completeness and reason codes;
- HYD-07 attribution classes for the same activity fixture;
- the §3.5 timeline's outcomes and the §4 negatives' outcomes.

**May differ:** where bytes live, the CAS mechanism (rename vs
conditional update), log representation, elapsed time, memory.

At unit/model level this is
`pmo_hyd_parity_semantics_match_across_backends`: the same fixtures
through the in-memory local-filesystem model and the in-memory
broker-row model of HYD-03 §6, compared field by field.

The hosted stage itself waits for a hosted hydration surface (§
Verified state). When one exists, its owner runs the same fixture set
and records the comparison as the hosted-parity stage (§8). Parity
uses read-only GitHub access; it never writes to GitHub to set up a
fixture.

## 8. Acceptance stages, recorded separately

Each stage gets its own evidence record. None implies another; a
green unit run is not migration evidence, and a staging run is not
deployed observation.

| Stage                   | Evidence                                                                           | Who                              |
| ----------------------- | ---------------------------------------------------------------------------------- | -------------------------------- |
| unit/model              | the §9 oracles listed then run with the done command, on a recorded commit         | implementation issue             |
| DB migration            | the migration owner's cited command, before and after `assess_store`, no data loss | #9198 storage owner              |
| staging                 | fixture runs with §6 records including time and memory                             | implementation and storage owner |
| deployed observation    | the deployed build's pointer and cursor states observed read-only                  | deployment owner                 |
| authenticated live read | one bounded hydrate against real GitHub with the read-only credential class        | separately authorized            |
| hosted parity           | §7 comparison on a hosted surface                                                  | #9079 / #9081 line               |

Each record states: stage, commit, environment, command as cited (not
reconstructed), result, and an explicit "not claimed" list.

No recurring parity, migration or refresh job is added. Anything
recurring lands with its row in `doc/fleet-schedule-registry.md`
first.

## 9. Oracles

All in `work/context.rs`'s test module, `pmo_hyd_` prefix, run with
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker <name>`
after `-- --list` shows them.

| Test                                                                 | Asserts                                                                                  |
| -------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `pmo_hyd_migration_rollback_preserves_generation_and_unknown_cursor` | §3.5, the slice's named first oracle                                                     |
| `pmo_hyd_legacy_last_checked_at_never_becomes_cursor`                | every step of §3.5 leaves `last_checked_at` unread by the write path and unchanged       |
| `pmo_hyd_disable_changes_no_stored_byte`                             | while off, every write-side command is refused before any request; store bytes unchanged |
| `pmo_hyd_reenable_after_fingerprint_change_is_full_snapshot`         | §3.4; an `unknown` cursor is never promoted                                              |
| `pmo_hyd_partial_migration_keeps_old_reader_authoritative`           | §4.1 for `partial: log` and `partial: pointer`; no publication                           |
| `pmo_hyd_pointer_object_mismatch_falls_back_to_last_complete`        | §4.2, including both bad → typed error, no scan                                          |
| `pmo_hyd_api_drift_invalidates_cursor_not_data`                      | §4.3; unparseable body is a source failure; prior generations unchanged                  |
| `pmo_hyd_deleted_referenced_artifact_is_typed_missing`               | §4.4; no substitute, no regeneration under the same sequence                             |
| `pmo_hyd_gc_candidates_exclude_pinned_and_published`                 | candidates come from HYD-03 §7 only                                                      |
| `pmo_hyd_parity_semantics_match_across_backends`                     | §7 "must be equal" list over both in-memory backends                                     |
| `pmo_hyd_perf_counters_are_deterministic_for_fixture`                | §6 deterministic columns equal their golden on two runs                                  |

## 10. Gates and sequencing

- HYD-01's record merges (#15184), and HYD-02 through HYD-08's
  implementations land in `work/context.rs` first (the issue's
  shared-file edge after HYD-08).
- CEN-04 §3.8's storage amendment is filed by the implementation
  issue and admitted through CEN-04 §5 before any cursor storage; the
  same for the pointer row and log if the broker backend is chosen
  (HYD-03 §6).
- Any DB migration step cites the migration owner's own command and
  follows CEN-04 §7's move shape. HYD-09 does not choose it.
- The hosted-parity stage waits for a hosted hydration surface.
- QTI stays unlocated (#9241); no persisted field overlapping it moves
  until its crosswalk and named owner exist.
- The ratified `expedite=15` / `next-up=25` values and existing
  queue, status, identity, Project-writer and `fleet gh` spellings
  are untouched.

## 11. Requirement trace (design-level)

| Requirement | Where                                                                                     |
| ----------- | ----------------------------------------------------------------------------------------- |
| C-R117      | §4.3: response bodies are parsed as data; drift is a source failure, never an instruction |
| H-R001      | §3 keeps the hydration track's storage, cursor and journal links coherent across rollback |
| P-R008      | §2: no new service, database or scheduler; existing owners keep storage and hosting       |
| P-R036      | §3.5 walks resume-after-rollback and incremental hydration as one oracle                  |

These are design-level only. Each still needs the implementation and
live evidence its trace row names.

## 12. Non-goals

- No DB migration, table, column or migration command.
- No collector, cleanup or deletion of any generation, cursor, log
  entry or legacy field.
- No hosted hydration surface and no hosted run.
- No schedule, watch loop or recurring parity job.
- No new performance threshold.
- No change to any earlier HYD record's contract; where this record
  restates one, the earlier record governs.
