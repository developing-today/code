# Tiny config tables: D1 cutover design (#5695)

- **Date:** 2026-11-24
- **Status:** design record; no production read/write switch, copy, or deletion authorized here.
- **Tracking:** [developing-today/code#5695](https://github.com/developing-today/code/issues/5695)
- **Related migration:** [#5692](https://github.com/developing-today/code/issues/5692) (sheet-derived staging and curated rows), [#6581](https://github.com/developing-today/code/issues/6581) (Turso wave).
- **Decision context:** Drew's §9.4 Q2 ruling selects a real D1 home as a scoped pilot; the pilot is complete at 54/54 rows in Cloudflare D1 `fleet-config` (account UUID `234b866b99ce2b142d7963d8d139cbfa`, database UUID `71995a94-7184-4d69-a730-ff7604df02f1`). This record designs the remaining consumer cutover, not a second data move.
- **Supersedes:** none. Complements, but does not amend, the broader placement and recovery rules in the Atlas/Turso data architecture and the #5692 design addendum.

## 1. Problem and context

Issue #5695 covers seven small `sources` registries—`ainventory.sources` (42 rows), `automation.sources` (2), `engineering.sources` (2), `finance.sources` (1), `home.sources` (1), `research.sources` (1), and `ideas.sources` (3)—plus `public.muse_notes` (2), eight tables and 54 rows in the completed pilot. The former source was Supabase project `muse-dev2`; the configured D1 target is `fleet-config`. The rows are configuration/registry data, not the larger sheet-staging or curated tables in #5692.

The issue history records Drew's answer to §9.4 Q2 and the completed D1 pilot. The November 7 pilot receipt says the five remainder rows were extracted from the verified November 6 full pgdump, not reconstructed; it confirms every field, including a genuine SQL NULL, and independently read all eight tables back at 54/54. This proves the pilot data load at that time. It does **not** prove current row parity, that every deployed ingest job reads D1, that a complete natural ingest cycle has used D1, or that Postgres copies have been retired. The original Supabase project is deleted and is not a live rollback endpoint.

The remaining design problem is therefore an eight-table reader/writer cutover with no dependency on the deleted service. The `sources` rows are read at ingest start. As the sheet-domain work moves staging/curated data toward Turso (#5692), a D1 configuration read is intentionally a cross-platform read. `ideas.sources` is also logically referenced by the separately placed `ideas.ideas` data; a cross-database foreign key cannot enforce that relationship. `public.muse_notes` has no evidenced relational consumer in the issue record, so its live consumer and write path must be found rather than assumed absent.

Evidence is bounded: the pilot receipt establishes its 2026-11-07 state. Before each operational slice, inventory current D1 DDL/rows, deployed producers and consumers, schedules, and any extant Postgres/archive copy. No current source endpoint or remote consumer is assumed available merely because it appears in repository code.

## 2. Goals and non-goals

### Goals

- Make `fleet-config` D1 the one authoritative live home for the eight tables, preserving all 54 pilot rows and the established table names, columns, IDs, nulls, and values.
- Move each identified reader/writer through a bounded, observable, per-consumer cutover; keep the seven source registries usable by their ingest jobs.
- Preserve the logical `ideas.sources` ↔ `ideas.ideas` relationship with application-level validation and integrity checks, without pretending D1 can enforce a foreign key into Turso.
- Establish parity, authorization, failure, and recovery evidence before retiring any accessible Postgres copy.
- Keep `muse_notes` in scope only if a live consumer or an explicit retained-data requirement is verified; retain and report it if no consumer is found.

### Non-goals

- No production write/read switch, data copy, Postgres deletion, credential rotation, or Cloudflare deployment is authorized by this design record.
- Do not move the 19 #5692 sheet-staging/curated tables to D1; the working design for those remains `fleet-platform` Turso pending that work's gates.
- Do not add a second D1 database, mirror the registries to Turso, or redesign the registry row schema as part of cutover.
- Do not infer a live Supabase rollback path. The verified dump is archival recovery input, not a running service.
- Do not treat completion of the 54-row pilot as proof of reader cutover, natural ingest, remote-consumer coverage, or Postgres retirement.

## 3. Alternatives considered

| Alternative | Benefits | Costs / risks | Decision |
|---|---|---|---|
| Leave the pilot rows in D1 but keep all readers on Postgres | No immediate reader change. | Leaves two competing copies, does not finish #5695, and relies on a deleted Supabase project for any writes/reads that still target it. | Rejected as the end state; acceptable only as a short pre-cutover state while consumers are inventoried. |
| Use the existing `fleet-config` D1 database | Follows Drew's Q2 ruling and the already completed, independently verified 54/54 load; small read-mostly registries fit the chosen config/KV placement. | Ingest now reads configuration across the D1/Turso boundary; D1/Worker access and remote consumers must be explicitly mapped; a cross-database logical reference needs application checks. | **Chosen.** One D1 system of record; one consumer at a time, with a parity gate. |
| Fold `sources` into the Turso staging tier and keep only `muse_notes` on D1 | Puts registry reads near some future sheet-domain data and reduces stores for those consumers. | Reverses the explicit Q2 ruling and pilot, creates a second copy/migration, and leaves `muse_notes` consumers unresolved. | Rejected absent a new explicit architecture decision; no dual-homing. |
| Rebuild a live Postgres source or use another database | Preserves SQL/relational semantics and could support cross-row constraints. | Reintroduces operations for 54 config-shaped rows, conflicts with the completed pilot and current direction, and does not restore the deleted endpoint. | Rejected for this slice. |

## 4. Chosen approach and contracts

### 4.1 Authority, data model, and access

`fleet-config` is the sole writable/live authority after the cutover gate. Keep its eight existing tables and deployed column definitions as discovered in the first execution slice. Preserve each row's primary identity and field values exactly; preserve SQL NULL distinctly from the literal string `N`; preserve existing defaults and any stable ordering/selection semantics. Do not make up missing columns or reconstruct rows from naming patterns. No new schema migration is implied by this design.

Use one repository-level configuration access seam per runtime family, with table-specific typed operations rather than arbitrary SQL exposed to consumers. A Cloudflare Worker that owns a consumer should use its configured D1 binding and least-privilege query path. A consumer outside the D1-bound Worker must use an existing authenticated private service path if one is found; otherwise its access mechanism is an explicit implementation prerequisite, not a new public endpoint silently introduced by this design. No browser or operator gets direct database credentials. Credentials remain in the supported secret/binding configuration and never enter source, logs, issue comments, or artifacts.

All registry writes go through the identified configuration editor/administrative path and target D1 only. Before cutover, either pause edits for the short parity-and-flip window or provide a reviewed versioned export/replay procedure; do not dual-write D1 and Postgres. Once D1 is authoritative, read failure is visible and fails closed for the affected ingest/config operation—no stale silent fallback to Postgres. A future cached fallback requires its own freshness contract and is outside this design.

### 4.2 Logical links and table boundary

- The seven `sources` tables remain separate by domain, with their present keys and fields. `ideas.sources` references the ideas corpus semantically, but D1 cannot enforce the Turso-side `ideas.ideas` relation. The consumer must validate referenced source IDs at the write boundary and expose an integrity check that detects a missing or orphaned reference; it must not claim transactional cross-database integrity.
- `public.muse_notes` stays in the same D1 database, but its live read/write consumer is an inventory gate. If none exists, preserve the rows and record the disposition; do not delete the table or rows as cleanup in this slice.
- D1 table names stay as loaded by the pilot (`ainventory_sources`, `automation_sources`, `engineering_sources`, `finance_sources`, `home_sources`, `research_sources`, `ideas_sources`, `public_muse_notes`). Verify actual deployed names and DDL before applying any change.

### 4.3 State transitions and failure behavior

For each consumer: inventory its deployed revision and access path; verify target table and sample/full key parity; exercise a read-only shadow comparison; pause config edits or capture them; flip that consumer to D1; observe one complete natural ingest or consumer cycle; then remove only that consumer's obsolete Postgres path after evidence. Keep unrelated consumers on their known paths until their own gates pass. If D1 is unavailable, fail that job visibly and retain its source input for retry; do not route writes to the deleted Supabase endpoint or silently read stale data.

## 5. Dependency-ordered implementation slices

All slices below are implementation proposals; they do not claim any production work is already authorized. A slice may be split further if the consumer inventory shows independent owners or a deployment boundary.

| Order | Slice and dependency | Acceptance criteria |
|---:|---|---|
| 1 | **Consumer and D1 inventory.** No data dependency. Enumerate all eight tables, current D1 DDL/row keys/counts, available archives, editors, ingest readers, `muse_notes` consumers, deployments, and schedules. Include Worker bindings and any non-Worker access. | Timestamped matrix names every table and every known consumer with code/deployment evidence, access path, owner, and last-observed revision. Unknown remote consumers are marked unknown, not absent. Target is exactly one `fleet-config` D1 and no new database. |
| 2 | **Parity and access adapter.** Depends on slice 1. Implement/verify the typed D1 access seam and read-only parity tooling; no reader flip. | Fresh D1 queries compare all 54 pilot rows (or current inventoried rows) by stable key and every field; nulls/types are checked explicitly. DDL, indexes, access binding/authorization, and failure behavior match the adapter. A missing/extra/different row blocks cutover with a reproducible report. |
| 3 | **First consumer cutover.** Depends on slices 1–2. Select the lowest-risk verified ingest consumer; capture a baseline and pause edits or stage/replay them. | Shadow comparison has zero unexplained key/value differences; a controlled and then one natural ingest cycle reads D1 successfully; logs identify consumer/table and outcome without data secrets; retry and D1-unavailable behavior are demonstrated. No Postgres row/table is deleted. |
| 4 | **Remaining consumers, one deployment boundary at a time.** Depends on slice 3's adapter and runbook, but not on unrelated consumers. Migrate each source domain reader/editor; investigate `muse_notes` separately if necessary. | For every row in the inventory, deployed revision is recorded, D1 is the only live read/write authority for that consumer, and one complete representative cycle or explicit non-use disposition is evidenced. The `ideas.sources` integrity check passes against the relevant Turso-side IDs. |
| 5 | **Retire old copies and closeout.** Depends on all in-scope consumers passing and archival recovery being verified. | Final receipt lists exact D1 counts/keys, consumer revisions, parity and cycle evidence, remaining unknowns, and retained archive checksum/location. Only confirmed obsolete Postgres copies are retired, with before/after checks and a recovery reference. #5695 and its relation to #5692/#6581 are updated without treating this design PR as cutover proof. |

## 6. Rollout and rollback

### Rollout

1. Re-inventory live D1 and all reachable consumers. Confirm the 54-row pilot state against D1; do not reload the old dump over newer rows.
2. Prepare and exercise the adapter and parity command read-only. Preserve a machine-readable before snapshot (row keys, field-level export, schema, and checksum) in the execution record, excluding live credentials.
3. Announce a bounded configuration edit freeze for the single-consumer shadow/flip window, or use a reviewed versioned change capture/replay procedure. Keep the freeze only as long as required and verify no edit was lost.
4. Shadow-read and compare, then switch one consumer. Keep all other consumers and any retained archival copy untouched. Observe the natural cycle and record exact revisions and results before proceeding.
5. Retire an old live copy only after every inventoried consumer is cut over and the archive has been independently verified. The deletion/retirement action is its own gated implementation step, not part of the pilot confirmation.

### Rollback and recovery

- Before the flip, discard only the shadow comparison state; leave D1 and all source evidence unchanged.
- After a consumer flip, revert that consumer's code/config to its last known-good revision only if that revision points at a verified, reachable source. Because `muse-dev2` was deleted, reverting to it is not a valid rollback. If there is no live prior source, fail closed, preserve queued/source inputs, and repair forward into D1.
- For a D1 write with an ambiguous response, read back by stable row key and compare all fields before retrying. Never blindly replay a non-idempotent administrative update.
- Keep the verified pgdump/archive as recovery material until the final gate. Restore from it only through a reviewed additive/idempotent procedure, never by replacing newer D1 values wholesale.
- Roll back one consumer at a time; do not switch unrelated domains or introduce Turso mirrors as an emergency shortcut.

## 7. Verification plan

1. **Scope oracle:** exactly the eight named tables are in #5695; #5692's 19 staging/curated tables, D1 candidates outside this issue, and the ainventory relational core are unchanged.
2. **Schema oracle:** compare deployed D1 DDL, columns, nullability, key/index definitions, and row conversion to the pilot receipt and the canonical consumer queries. No schema-name-only or repository-grep-only proof.
3. **Data oracle:** query all rows from D1 and compare stable keys and every field to the latest authoritative export/archive available. Report missing, extra, duplicate, and changed rows; distinguish SQL NULL, empty string, and literal text. Counts alone are insufficient.
4. **Access oracle:** exercise each consumer using its real configured binding/service identity; verify unauthorized access is denied and no credentials or sensitive row payloads appear in logs.
5. **Parity oracle:** shadow results match the old reachable source where one exists. Where Postgres is deleted/unreachable, compare to the verified archive plus D1 pilot receipt and label that as archive parity, not live-source parity.
6. **Behavior oracle:** prove source selection/config semantics with representative tests, including empty/missing registry behavior and the `ideas.sources` logical-reference check; then observe one natural complete cycle per live consumer after flip.
7. **Failure/retry oracle:** simulate D1 unavailability and an uncertain write on a safe target; verify visible failure, retained input, read-before-retry, and no silent stale fallback.
8. **Retirement oracle:** before retiring any copy, confirm no inventoried deployment, schedule, script, or operator workflow still reads/writes it; verify archive checksum and a restore/readback rehearsal.

Report focused tests and live-cycle evidence separately from full repository CI. A passing unit test or a healthy Worker alone does not prove that the migrated rows drove the expected ingest.

## 8. Open questions and decisions

1. **Exact consumer inventory and D1 access path — decider: #5695 execution owner, accountable: Drew.** The issue evidence does not enumerate every deployed ingest job or identify a live `public.muse_notes` reader/editor. **Default:** inspect deployments, schedules, and current consumer source before coding; use a Worker D1 binding for a Worker-owned consumer, and do not create a public API for a non-Worker caller without a separate security review.
2. **Write-freeze versus change capture — decider: implementation owner for each editor, accountable: Drew.** No current update rate or editor protocol is established by the pilot receipt. **Default:** use a short explicit edit freeze during shadow/flip; if an editor cannot pause, stop and design a versioned replay before switching it. Never dual-write.
3. **`muse_notes` disposition — decider: Drew, with the inventory owner supplying evidence.** Its consumers were unevidenced in the issue. **Default:** retain both D1 rows and table; classify consumer status only after inspecting likely manual and deployed paths. No deletion based on a search miss.
4. **`ideas.sources` ↔ Turso `ideas.ideas` integrity — decider: #5692 execution owner for the cross-store contract, accountable: Drew.** D1 cannot enforce the cross-database reference. **Default:** validate IDs in the application and run an explicit orphan check; do not promise atomicity across D1 and Turso.
5. **Postgres-copy retirement timing — decider: Drew as migration owner.** The deleted source endpoint cannot be used for live rollback, and an archive is not an online fallback. **Default:** retain any extant accessible copy until every consumer is evidenced on D1 and the archive restore/readback gate passes; then retire only copies verified obsolete.

These defaults allow the design and implementation inventory to proceed without waiting for a new owner answer; none authorizes a production cutover or irreversible deletion.

## 9. Record, tracking, and acceptance

This is the dated design artifact for code#5695. The issue remains the tracker for the consumer cutover tail; #6581 tracks the broader execution wave and #5692 tracks the separate sheet-derived staging/curated transition. This record neither closes those issues nor claims production cutover.

**Design acceptance:**

- [x] Problem and bounded evidence, goals/non-goals, alternatives including do-nothing, selected contracts and access/security boundaries.
- [x] Dependency-ordered implementation slices with testable acceptance criteria.
- [x] Rollout and rollback/recovery limits, verification oracles, and named-decider open questions with defaults.
- [x] Dated `doc/` record tied to #5695 and related #5692/#6581; distinguishes pilot/archive evidence from deployed cutover evidence.
- [ ] After merge, add the permanent document URL to #5695 and verify the issue comment and document.

### Evidence references

- [#5695 and its Q2/pilot/cutover history](https://github.com/developing-today/code/issues/5695)
- [#5692 sheet-derived staging and curated-row design addendum](2026-11-24T00-50-11Z_design_sheet-staging-turso/2026-11-24T00-50-11Z_design_sheet-staging-turso.md)
- [#6581 Turso cutover wave plan](fleet/queue-turso-cutover.md)
- [#5695's §9 placement design](2026-11-04T21-27-34Z_design_atlas_scaler_revision/2026-11-04T21-27-34Z_design_atlas_scaler_revision.md)
- [November 7 D1 remainder pilot receipt and backup-dump correction](https://github.com/developing-today/code/issues/5695) (recorded in the tracking issue history; #6619 is the related backup-transport incident, not the pilot receipt)
- [Design sufficiency audit and nine-point standard](2026-11-24T00-24-54Z_audit_design-gaps/2026-11-24T00-24-54Z_audit_design-gaps.md)
