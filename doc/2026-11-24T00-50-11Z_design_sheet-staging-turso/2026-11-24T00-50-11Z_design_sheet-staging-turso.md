# Sheet-derived staging and curated rows: Turso design addendum

Date: 2026-11-24 00:50 UTC  
Status: design record; no data movement or production cutover authorized  
Tracking issue: [developing-today/code#5692](https://github.com/developing-today/code/issues/5692)  
Wave plan: [developing-today/code#6581](https://github.com/developing-today/code/issues/6581)  
Supersedes: none. This addendum makes the #5692 slice self-contained and records later recovery and cutover evidence; it does not replace the broader placement rules in §9 of the [Atlas/Turso data architecture](../2026-11-04T21-27-34Z_design_atlas_scaler_revision/2026-11-04T21-27-34Z_design_atlas_scaler_revision.md).

## 1. Problem and context

The 2026-11-04 read-only `muse-dev2` inventory classified 19 sheet-derived staging and curated tables as candidates to leave PostgreSQL: 12 raw-ingest tables and 7 curated/corpus tables, approximately 96,946 rows and 56.4 MB at that snapshot. Raw `sheet_rows` are JSON row blobs associated with import batches; curated rows are sheet-derived, insert-mostly, dedupe-keyed, and read as a query/search corpus. The audited sheet-domain schemas had no foreign keys requiring PostgreSQL cross-row integrity. See the dated inventory, §9.2, rows for ainventory and the five sheet domains.

This is no longer a greenfield Supabase-to-Turso copy. The November 18 recovery record says the `muse-dev2` project was deleted and not recreated. A verified archive was retained, missing operational rows were recovered additively into Turso, and newer target rows were not overwritten. The current cutover reference says the #5692 Turso shadows have landed but consumer cutovers remain pending. It also records an unconverted `automation.events` writer and other remaining Supabase-bound consumers. Thus the actual problem is to finish a safe, per-domain writer/reader transition around the recovered Turso data—not to assume the old Supabase service is still available as a live rollback target.

The currently documented target is Turso database `fleet-platform`, with source `schema.table` mapped to `schema_table` (for example, `engineering.records` → `engineering_records`). The November 23 audit also contains a newer proposal to place ingest families in D1. That conflicts with the #5692/#6581 Turso wave record and must be decided explicitly; this document uses the already-selected and populated Turso target as its working default, not as a new D1 ruling.

Evidence is time-bounded. The table counts below are the 2026-11-04 Supabase audit snapshot, not current live counts. The November 18 receipts and November 23 database audit provide later recovery and destination evidence. Before any operational slice, the executing lane must inventory the live source sheets, Turso tables, active producers, and readers again. A repository design record is not proof that every deployed or remote consumer has been found.

## 2. Goals and non-goals

### Goals

- Keep the 19-table #5692 scope in one explicit inventory, divided into raw staging and curated rows.
- Treat `fleet-platform` Turso as the working destination and preserve the existing schema-prefix naming, batch identity, and dedupe behavior.
- Complete remaining writer and reader changes per domain, with observable parity gates and no cross-domain cutover dependency.
- Preserve recovered and newly written target rows; make retries idempotent and retain enough evidence to recover from partial or ambiguous writes.
- Keep the source-sheet ingest pipeline and curated corpus usable without requiring PostgreSQL relational semantics that this data does not use.

### Non-goals

- No production write, migration, cutover, cron change, database deletion, or credential change is authorized by this design task.
- Do not move the ainventory relational core (`models`, `measurement_events`, `price_events`, `provider_policies`, `benchmark_results`, or their dependent views); it remains KEEP-on-Postgres by the existing placement record.
- Do not move the `sources` configuration registries or other D1 candidates; they belong to #5695 and require their own destination decision.
- Do not migrate `ci.events`, agent append/mirror tables, or the 49-table retire set; those have separate wave items.
- Do not replay queue/control state, replace existing Turso rows from an older archive, or claim remote-consumer coverage based only on repository searches.
- Do not claim that Supabase is a presently usable rollback service. Its retained backup is archival recovery input, not a live endpoint.

## 3. Alternatives considered

| Alternative                                                        | Benefits                                                                                                                                                                                 | Costs / risks                                                                                                                                                                                                             | Decision                                                                                                                                                                   |
| ------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Do nothing; leave #5692 at its existing state                      | No new cutover risk or write path.                                                                                                                                                       | Leaves the documented pending consumers unresolved; some old producers already fail against the deleted Supabase endpoint. It does not establish that all sheet-derived reads and writes use the recovered Turso data.    | Rejected as the completion strategy. Freeze risky cutovers until an inventory and parity gate pass, but do not represent the migration as complete.                        |
| Continue the existing per-domain Turso wave in `fleet-platform`    | Matches #5692/#6581, the recovered operational rows, current schema-prefix code, and the November 23 enumeration of live Turso families. Enables independently verified domain cutovers. | Requires a fresh inventory because the original source project is gone, plus careful handling of legacy producers and a rollback plan that repairs forward rather than pointing at Supabase.                              | **Chosen working approach.** Preserve the current Turso target while the destination conflict below is resolved; do not perform another bulk copy from the stale snapshot. |
| Re-target all or some families to D1                               | The November 23 audit argues the repeated ingest-family shape is Worker-owned and could fit D1; D1 could isolate family writes.                                                          | Conflicts with the merged Turso wave and existing recovery, stores, readers, and Turso shadows. Requires a new D1 schema/copy/cutover and a policy for already-populated Turso rows; doing both creates split-brain risk. | Not selected here. Drew, the migration owner, must explicitly reconcile the newer D1 proposal with #6581 before any D1 work or second destination is introduced.           |
| Keep the data in PostgreSQL or move it to another Postgres service | Retains familiar SQL and source-side behavior.                                                                                                                                           | The source project is deleted; no live Postgres rollback endpoint is established. Rebuilding a new Postgres service adds operations and does not resolve the active Turso state or the data's staging/corpus shape.       | Rejected for this slice; any new Postgres destination is a separate architecture decision.                                                                                 |

## 4. Chosen approach: bounded Turso cutover by domain

### Table inventory

Counts and sizes are estimates from the November 4 snapshot; names and placements are the scope oracle, not those old counts.

| Tier        | Source tables                                                                                                        | Count | Snapshot rows / size | Destination names                                       |
| ----------- | -------------------------------------------------------------------------------------------------------------------- | ----: | -------------------: | ------------------------------------------------------- |
| Raw staging | `ainventory.sheet_rows`, `ainventory.ingest_batches`                                                                 |     2 |     84,599 / 43.3 MB | `ainventory_sheet_rows`, `ainventory_ingest_batches`    |
| Raw staging | `sheet_rows`, `ingest_batches` in each of `automation`, `engineering`, `finance`, `home`, `research`                 |    10 |       3,962 / 3.5 MB | `<schema>_sheet_rows`, `<schema>_ingest_batches`        |
| Curated     | `events` in `automation`, `engineering`, `finance`, `home`, `research`; plus `engineering.records` and `ideas.ideas` |     7 |       8,385 / 9.6 MB | `<schema>_events`, `engineering_records`, `ideas_ideas` |

The 2026-11-23 live audit independently reports post-recovery families in `fleet-platform` (including `ainventory_sheet_rows` at 85,450 rows); those later numbers must not be added to the older snapshot totals. The same audit classifies some broader ingest families for D1, so its destination recommendation is a conflict to resolve, not evidence that these 19 tables were moved to D1.

### Data and interface contracts

1. **Destination and names.** Use the configured Turso `fleet-platform` endpoint and the existing `schema_table` translation. Do not invent a second database or a second copy of the family. Use the repository's shared Turso/Hrana client and configured secret resolution; credentials stay in the supported secret store and are never embedded in docs, commands, or artifacts.
2. **Staging parent/row relation.** Preserve each `ingest_batches` identity and its associated `sheet_rows`; `sheet_rows` are batch-keyed JSON row payloads. The recovery work added/audited an ingest-parent unique index. Before changing a writer, inspect the actual deployed DDL and the canonical writer's key/column semantics; this design does not authorize a schema rewrite or invent a new batch key.
3. **Curated-row idempotence.** Preserve the existing `dedupe_key` upsert contract for `events`, `engineering.records`, and `ideas.ideas`. A retry of the same source row must not create another logical curated row. Match the existing writer's conflict target and update set; do not silently replace identity, timestamps, or fields not currently mutable.
4. **Type and precision behavior.** Follow the existing reviewed operational schema and converter. Recovery receipts document JSON/text and timestamp normalization and small, bounded REAL differences; they do not assert bitwise equality for every PostgreSQL float. Verification must report the normalization and any allowed numeric tolerance, not hide it behind counts alone.
5. **Write safety.** Keep append-only/identity guards where installed. The recovery receipts state that 18 guards were exercised on staging, while explicitly not proving production installation; the cutover lane must inspect destination schema and prove required guard behavior on a safe test target before relying on it.
6. **Access boundary.** All access goes through the existing service/Worker client path with least-privilege configured tokens. No browser or operator receives direct database credentials. Do not broaden public access or weaken Worker authorization as a migration shortcut.
7. **Scope boundary.** The seven `sources` registries are not among these 19 tables; `ainventory` relational core remains on Postgres. `ideas.ideas` is in curated scope; `ideas.sources` is not.

### State transition

For each domain independently: (a) enumerate source-sheet producer, deployed writer, downstream readers, scheduled jobs, and exact Turso table; (b) verify schema/key compatibility and current target contents; (c) enable or repair the Turso writer without destructive synchronization; (d) compare a bounded fresh ingest against source intent and target, including batch membership, row keys, dedupe-key conflicts, content samples/hashes, and error/uncertain-commit handling; (e) switch that domain's readers only after the gate passes; (f) observe at least one natural ingest/consumer cycle; (g) disable obsolete Supabase-bound code only after no live references remain. Keep domains independently operable and record the exact domain, commit, and evidence at every transition.

No one domain's failure should stall another domain. Do not declare the whole #5692 scope complete because the shared table schema exists or a shadow copy has rows.

## 5. Dependency-ordered implementation slices

Each slice is sized for one lane/session. These are proposed executable slices under #6581; this design does not enqueue duplicates or claim they are already authorized/ready.

| Order | Slice and dependency                                                                                                                                                                                                                                                                 | Acceptance criteria                                                                                                                                                                                                                                                                                                                                                                            |
| ----: | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
|     1 | **Fresh inventory and gate manifest.** No data dependency. Reconcile the 19 source names against live `fleet-platform`, current source sheets/backups, writer code/deployments, readers, schedules, and the conflicting D1 proposal.                                                 | A checked-in or issue-linked per-table matrix lists source, Turso name, current row/key counts, active producer and readers, proof source/date, and disposition. Every row is classified in or out of #5692. The decision between Turso and D1 is recorded by Drew before any D1 action; until then Turso is the working destination. Unknown remote consumers are marked unknown, not “none.” |
|     2 | **Schema and idempotence verification.** Depends on slice 1. No production cutover. Inspect deployed DDL and code for all staging and curated tables; run safe isolated tests for batch association, unique keys, dedupe upserts, append-only guards, and ambiguous commit recovery. | The verified DDL/key manifest matches the canonical writer; duplicate delivery is idempotent; batch rows cannot become detached silently; guard behavior is demonstrated on a safe target; mismatches have an explicit repair plan. No production destructive test is used.                                                                                                                    |
|     3 | **Repair/enable writers per domain.** Depends on slices 1–2. One domain per PR or independently revertible unit, beginning with the lowest-risk domain identified by the fresh inventory (not assumed from stale row counts).                                                        | A real natural or controlled non-destructive ingest writes the expected Turso batch/rows; read-back proves batch linkage, expected row keys, dedupe behavior, and no unexplained loss/duplication. Logs identify the target and batch without exposing secrets. Failure does not prevent other domains from ingesting.                                                                         |
|     4 | **Reader cutover per domain.** Depends on slice 3 for that domain and a passing parity gate.                                                                                                                                                                                         | Every inventoried reader for the domain uses the intended Turso table; representative query results match the approved baseline; errors/latency are observed for one natural consumer cycle; the previous path is disabled only after evidence is recorded. Unverified remote consumers remain open findings.                                                                                  |
|     5 | **Closeout and legacy-reference disposition.** Depends on slices 3–4 for all in-scope domains and the migration-owner decision on the #5692/#6581 gate.                                                                                                                              | A dated per-table cutover receipt records final counts, parity method, writer/reader commits, and unresolved deltas. No active #5692 consumer still depends on the deleted Supabase project. #5692/#6581 dependency and closure state is reconciled without treating this design PR as evidence of production cutover.                                                                         |

Per-domain rollout can overlap only after the inventory identifies independent producers/readers and the schema gate passes; table names alone are not sufficient proof of independence.

## 6. Rollout and rollback

### Rollout

- First revalidate live state; the old Supabase endpoint is not assumed to exist. Preserve the recovered Turso state and verified archives. Do not bulk-reload a November 17 snapshot over current rows.
- Use additive-only repair for missing rows (`INSERT OR IGNORE` or the reviewed equivalent), with before/after counts and a second gate. Never run a sync that can shrink or overwrite the destination as a shortcut.
- Cut over one domain at a time. Keep unrelated domains running; retain the prior application artifact/config and a machine-readable receipt for each narrowly changed writer or reader.
- Do not drop or retire historical source/archive material as part of this slice. Any archival deletion needs a separate, explicit inventory and verified-copy gate.

### Rollback and recovery

- **Before a domain's reader switch:** disable the new writer if unsafe, preserve the batch and logs, and repair forward; do not destroy candidate Turso rows.
- **After the reader switch:** revert only that domain's application/config to a known-good version that still reads a verified store. Because Supabase was deleted, “point back to Supabase” is not a valid rollback. If no verified previous live store exists, fail closed for that domain and repair forward from the source sheet or verified archive using additive-only semantics.
- For an ambiguous write result, read back by stable batch/row/dedupe keys before retrying. Never blindly retry a non-idempotent operation. A failed transaction must be followed by fresh absence/readability checks before another attempt.
- Retain the prior units, configs, batch identifiers, source archive manifest, checksums, and outcome receipts. Recovery is incomplete until fresh-connection verification passes and the relevant consumer health check recovers.

These rules follow the existing recovery receipts: an earlier unique-key conflict was explicitly rolled back and absence-checked; a busy preflight was not retried; retained cutover artifacts are evidence, and the documented recovery direction is “repair forward.” They do not claim rollback procedures were tested for every one of the 19 tables.

## 7. Verification plan

For every slice and every table touched, retain evidence with a timestamp and exact code/schema revision:

1. **Inventory oracle:** the manifest contains exactly 19 in-scope source tables (12 staging + 7 curated) and maps each to one expected Turso table. Compare against live schema/catalog reads, not a grep-only inventory.
2. **Schema oracle:** compare deployed columns, nullability, indexes/unique constraints, triggers/guards, and the canonical writer's SQL. Test uniqueness and append-only behavior on an isolated target. A successful migration script alone is not the oracle.
3. **Row oracle:** compare source intent and destination by stable batch/row/dedupe keys; record counts, missing/extra keys, duplicate groups, representative content hashes, and timestamp/number normalization. Counts alone do not prove parity. Preserve and explain any known conflict rather than overwriting it.
4. **Retry oracle:** replay a bounded identical input on the isolated target. The row count and logical content must remain unchanged; simulate an uncertain response and prove read-before-retry.
5. **Live producer/reader oracle:** observe one natural post-change ingest and one representative consumer read per domain. Verify HTTP/service health and application-level results; a green process exit without reading the new rows is insufficient.
6. **Security oracle:** verify the deployed service uses configured secrets and intended authorization; confirm no credentials appear in logs and no public access was introduced.
7. **Scope oracle:** check that excluded Postgres core tables, #5695 source registries, and unrelated wave tables were not modified. Report full-repository CI status separately; do not imply it ran if only focused tests ran.

Existing evidence is useful but bounded: the November 18 recovery receipt verified imported values on fresh connections and reported five measurement REAL cells within one ULP; the 61 focused recovery tests passed, while full repository CI was not run. Those receipts do not substitute for per-domain live cutover checks or prove remote-consumer coverage.

## 8. Open questions and decisions

1. **Destination conflict — decider: Drew (migration owner).** The November 23 audit proposes D1 for broad sheet-ingest families, while #5692/#6581 and the populated stores/readers use Turso. **Default:** keep `fleet-platform` as the working destination and do not add or cut over to D1 until Drew records a family-level ruling. This design does not silently resolve the conflict.
2. **#5692/#6581 dependency and closure cycle — decider: Drew (migration owner).** Current issue records describe #5692's completion/closure as tied to #6581 Wave 1, while Wave 1 is blocked on #5692. **Default:** this design supplies no production cutover evidence and changes neither issue's dependency nor status; reconcile the gate in the wave tracker before closeout. Do not wait on this answer to complete the design record.
3. **Remote producer/reader coverage — decider: the #6581 execution owner, with Drew accountable for scope.** Repository and laptop receipts explicitly leave some remote coverage unproven. **Default:** each execution slice inventories deployed Workers, scheduled jobs, and external consumers; mark anything unreachable as unverified and do not declare complete.
4. **Preservation of existing row conflicts — decider: Drew for any business-level conflict; executing lane for mechanical classification.** Recovery receipts retain 79 pre-existing row conflicts pending reconciliation and do not prove a full before/after hash of every old row. **Default:** preserve both source evidence and current Turso row; no last-write-wins replacement absent a specific reviewed rule.
5. **Numeric equality tolerance — decider: Drew for business acceptance, with the data-verification owner proposing the bound.** Recovery has an observed ≤1 ULP bound for specific REAL cells, not a blanket guarantee. **Default:** compare exact values where representable and document any per-column tolerance with an independent value-level check; never generalize the historic bound to untested fields.

## 9. Record, tracking, and acceptance

This dated record is the design artifact for code#5692 and remains linked from that tracking issue. The existing issue is the implementation/wave tracker; #6581 is the umbrella for execution. Queue item 21553 tracks delivery of this design-only slice. A link from the source issue establishes artifact discoverability; it does not close #5692 or establish any production migration fact.

**Design acceptance:**

- [x] Problem and evidence, goals/non-goals, honest alternatives including do-nothing, chosen contracts and capability boundary.
- [x] Dependency-ordered, lane-sized slices with individual acceptance criteria.
- [x] Rollout, rollback/recovery limits, verification oracles, and named-decider open questions with defaults.
- [x] Date-stamped `doc/` record tied to #5692 and #6581; explicitly distinguishes design evidence from deployed cutover evidence.
- [ ] After merge, add this document's permanent GitHub link to #5692 and verify the issue comment and document.

### Evidence references

- [§9 Supabase placement audit and table counts](../2026-11-04T21-27-34Z_design_atlas_scaler_revision/2026-11-04T21-27-34Z_design_atlas_scaler_revision.md#9-addendum-2026-11-04-later--supabase-placement-audit-what-moves-out-of-postgres)
- [#6581 wave plan and current cutover boundaries](../fleet/queue-turso-cutover.md#what-still-touches-postgres-after-the-queue-cutover)
- [November 18 recovery receipt](../2026-11-18_supabase-turso-recovery.md)
- [November 18 producer cutover receipt](../2026-11-18_turso-producer-cutover.md)
- [November 18 final data receipt](../2026-11-18_turso-recovery-final-receipt.md)
- [November 23 Turso/D1 migration audit, including the conflicting D1 proposal](../2026-11-23T23-15-07Z_design_turso-d1-migration-audit/2026-11-23T23-15-07Z_design_turso-d1-migration-audit.md)
- [November 24 design-gap audit and task 21553](../2026-11-24T00-24-54Z_audit_design-gaps/2026-11-24T00-24-54Z_audit_design-gaps.md)
