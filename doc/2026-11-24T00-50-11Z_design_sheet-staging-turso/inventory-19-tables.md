# #5692 sheet staging + curated rows — Slice 1 fresh-inventory gate manifest

Date: 2026-11-24. Slice 1 of the nine-point design addendum
(`2026-11-24T00-50-11Z_design_sheet-staging-turso.md` §5, order 1).
No production cutover, no data movement, no credential change is authorized
by this manifest. Method: repo-code inventory only (PG DDL in
`pkgs/fleet/migrations/`, Turso readers in `pm-stack/aggregator/src/`,
Turso recovery scripts in `scripts/recovery/`), reconciled against the
2026-11-04 snapshot names, the 2026-11-18 recovery receipts, and the
2026-11-23 audit. Live Turso row counts and deployed Worker writer/reader
deployments were not re-read from this lane; anything not proven from the
repository is marked `unknown`, never `none`.

Destination default (unresolved conflict recorded): Turso `fleet-platform`
with the existing `schema_table` translation. The 2026-11-23 audit proposes
D1 for broad ingest families; per the design addendum §3/§8.1 no D1 action
occurs until Drew records a family-level ruling. Until then Turso is the
working destination.

| Source | Turso | Producer / writer evidence (repo) | Reader evidence (repo) | Disposition | Proof date / source |
|---|---|---|---|---|---|
| ainventory.sheet_rows | ainventory_sheet_rows | `scripts/recovery/import_aa_missing.py` (Turso additive recovery, EXPECTED ainventory_sheet_rows 157); PG DDL `pkgs/fleet/migrations/0004_ainventory.sql` | `pm-stack/aggregator/src/aa-turso-sql.ts` (Turso reads); `pkgs/fleet/migrations/0010_ainventory_views.sql` (PG views over ainventory.sheet_rows) | in-scope #5692, Turso working destination | 2026-11-24 repo read; Nov-23 audit reports ainventory_sheet_rows 85,450 live |
| ainventory.ingest_batches | ainventory_ingest_batches | `scripts/recovery/import_runtime.py` (parent table, additive-only); PG DDL `pkgs/fleet/migrations/0013_aa_event_log.sql` | `pm-stack/aggregator/src/aa-turso-sql.ts` (joins ainventory_ingest_batches); `pm-stack/aggregator/src/aa-batch-sql.ts` (PG-side join) | in-scope #5692, Turso working destination | 2026-11-24 repo read |
| automation.sheet_rows | automation_sheet_rows | PG DDL `pkgs/fleet/migrations/0017_workbook_final_captures.sql`; Turso writer unknown (no repo Turso writer found) | unknown (no repo Turso reader found; Nov-23 audit reports automation_sheet_rows 1,853 live, reader is the dashboard Worker — unverified from repo) | in-scope #5692, Turso working destination; writer/reader repair pending Slice 3/4 | 2026-11-24 repo read; Nov-23 audit counts |
| automation.ingest_batches | automation_ingest_batches | PG DDL `pkgs/fleet/migrations/0017_workbook_final_captures.sql`; Turso writer unknown | unknown (dashboard Worker reader per Nov-23 audit, unverified from repo) | in-scope #5692, Turso working destination; writer/reader repair pending Slice 3/4 | 2026-11-24 repo read |
| automation.events | automation_events | PG DDL `pkgs/fleet/migrations/0017_workbook_final_captures.sql`; repo writer unknown (cutover doc records orchestration-append path with no repo helper; `doc/fleet/queue-turso-cutover.md` notes appends failing since outage) | unknown (Nov-23 audit reports automation_events 5,908 live; consumers unverified from repo) | in-scope #5692, Turso working destination; writer repair pending Slice 3 | 2026-11-24 repo read; cutover doc + Nov-23 audit |
| engineering.sheet_rows | engineering_sheet_rows | PG DDL `pkgs/fleet/migrations/0011_engineering.sql`; Turso writer unknown | unknown (dashboard Worker reader per Nov-23 audit family shape, unverified from repo) | in-scope #5692, Turso working destination; writer/reader repair pending Slice 3/4 | 2026-11-24 repo read |
| engineering.ingest_batches | engineering_ingest_batches | PG DDL `pkgs/fleet/migrations/0016_engineering_events.sql`; Turso writer unknown | unknown (unverified from repo) | in-scope #5692, Turso working destination; writer/reader repair pending Slice 3/4 | 2026-11-24 repo read |
| engineering.events | engineering_events | PG DDL `pkgs/fleet/migrations/0016_engineering_events.sql`; Turso writer unknown | unknown (unverified from repo) | in-scope #5692, Turso working destination; writer/reader repair pending Slice 3/4 | 2026-11-24 repo read |
| engineering.records | engineering_records | PG DDL `pkgs/fleet/migrations/0011_engineering.sql`; Turso writer unknown | unknown (unverified from repo) | in-scope #5692 curated, Turso working destination; writer/reader repair pending Slice 3/4 | 2026-11-24 repo read |
| finance.sheet_rows | finance_sheet_rows | PG DDL `pkgs/fleet/migrations/0017_workbook_final_captures.sql`; Turso writer unknown | unknown (Nov-23 audit family shape lists finance_sheet_rows; reader unverified from repo) | in-scope #5692, Turso working destination; writer/reader repair pending Slice 3/4 | 2026-11-24 repo read |
| finance.ingest_batches | finance_ingest_batches | PG DDL `pkgs/fleet/migrations/0017_workbook_final_captures.sql`; Turso writer unknown | unknown (unverified from repo) | in-scope #5692, Turso working destination; writer/reader repair pending Slice 3/4 | 2026-11-24 repo read |
| finance.events | finance_events | PG DDL `pkgs/fleet/migrations/0017_workbook_final_captures.sql`; Turso writer unknown | unknown (unverified from repo) | in-scope #5692 curated, Turso working destination; writer/reader repair pending Slice 3/4 | 2026-11-24 repo read |
| home.sheet_rows | home_sheet_rows | unknown (no home DDL or writer found in repo; Nov-04 snapshot: smallest domain, 63 live rows) | unknown (Nov-23 audit family shape lists home_sheet_rows; reader unverified from repo) | in-scope #5692, Turso working destination; existence re-verified against live Turso in Slice 3, not assumed from stale snapshot | 2026-11-24 repo read (absent); Nov-23 audit family list |
| home.ingest_batches | home_ingest_batches | unknown (no home DDL or writer found in repo) | unknown (unverified from repo) | in-scope #5692, Turso working destination; existence re-verified against live Turso in Slice 3 | 2026-11-24 repo read (absent) |
| home.events | home_events | unknown (no home DDL or writer found in repo) | unknown (unverified from repo) | in-scope #5692 curated, Turso working destination; existence re-verified against live Turso in Slice 3 | 2026-11-24 repo read (absent) |
| research.sheet_rows | research_sheet_rows | PG DDL `pkgs/fleet/migrations/0017_workbook_final_captures.sql`; Turso writer unknown | unknown (Nov-23 audit reports research_sheet_rows 945 live; reader unverified from repo) | in-scope #5692, Turso working destination; writer/reader repair pending Slice 3/4 | 2026-11-24 repo read; Nov-23 audit counts |
| research.ingest_batches | research_ingest_batches | PG DDL `pkgs/fleet/migrations/0017_workbook_final_captures.sql`; Turso writer unknown | unknown (unverified from repo) | in-scope #5692, Turso working destination; writer/reader repair pending Slice 3/4 | 2026-11-24 repo read |
| research.events | research_events | PG DDL `pkgs/fleet/migrations/0017_workbook_final_captures.sql`; Turso writer unknown | unknown (Nov-23 audit reports research_events 1,714 live; reader unverified from repo) | in-scope #5692 curated, Turso working destination; writer/reader repair pending Slice 3/4 | 2026-11-24 repo read; Nov-23 audit counts |
| ideas.ideas | ideas_ideas | PG DDL `pkgs/fleet/migrations/0003_ideas.sql`; Turso shadow landed per #6581 Wave 0 record; Turso writer unknown from repo | unknown (unverified from repo) | in-scope #5692 curated, Turso working destination; excluded from #5695 D1 pilot scope (only ideas.sources/rowsets/import_batches are D1) | 2026-11-24 repo read; #6581 Wave 0 record |

## Gate verdict

- Manifest contains all 19 in-scope tables (12 staging + 7 curated): PASS, mechanically checked by `scripts/tests/test_sheet_staging_inventory.py`.
- Turso names follow the `schema_table` translation for every row: PASS, same test.
- Repository-proven Turso read path exists for 2/19 (ainventory pair via `pm-stack/aggregator/src/aa-turso-sql.ts`); the remaining 17 are `unknown`, not `none`: next slices must inventory deployed Workers, schedules, and external consumers before any writer/reader change.
- Out of scope (not in this manifest by design): ainventory relational core (KEEP-on-Postgres), the seven `sources` registries (#5695), ci.events (#5693), agent append/mirror tables (#5690/#5691), the 49-table RETIRE set (Wave 4).
- Live Turso counts were not re-read by this lane; the Nov-23 audit numbers cited above are the latest checked-in evidence, not fresh verification.

Next slice (design §5 order 2): schema and idempotence verification on an
isolated target — inspect deployed DDL and the canonical writer's
conflict/identity semantics for staging parent/row relation and curated
dedupe upserts, with additive-only, non-destructive tests. After that PR,
per-domain writer repair (order 3) begins with the lowest-risk domain named
by the fresh live inventory, not by stale row counts.
