# Local schema evidence gate (#6581 / #5692)

This slice supplies a read-only catalog inspector, **not a writer repair or a
cleared live gate**. No production access, token creation, migration, deployment,
SQL replay, or data mutation is performed. Real producers remain unknown.

## Run on an existing artifact

```sh
python3 -I scripts/inspect_sheet_staging_schema.py --database /path/to/downloaded.sqlite
```

The input must be a SQLite-compatible database artifact, including a schema-only
SQLite database. A text SQL dump is deliberately not accepted or executed. Obtain
the artifact separately through an already-authorized read/export path; record
its origin, acquisition timestamp, target/schema revision and checksum alongside
the JSON output. A local fixture cannot prove what is currently deployed.

The complete CLI is `--database PATH` and `--help`. Unknown/abbreviated options,
extra positionals and missing values exit 2 before opening the artifact. Opening
uses an escaped file URI with `mode=ro` plus `query_only`; a missing path is not
created. JSON goes to stdout; exit 1 means incomplete evidence, and exit 2 means
usage/open failure. This version always exits 1 for a valid artifact because
canonical-writer and guard-behavior evidence cannot be established by catalogs.

## Evidence and interpretation

All 19 names in `inventory-19-tables.md` are explicitly covered. For each table,
the report retains `sqlite_master` table DDL, `table_info` (including nullability
and primary-key ordinal), `index_list`/`index_info`, raw index SQL,
`foreign_key_list`, and installed trigger definitions. Only unconditional unique
indexes with the exact ordered columns qualify for the expected key; partial or
expression indexes do not silently count as a full-column dedupe guarantee.
Trigger presence is not proof of guard semantics: definitions are retained for
review, never parsed heuristically or executed. Absent triggers are observations
with an unknown required guard policy, not blanket migration defects. The `id`
primary key is checked independently of dedupe keys. Nullable SQLite text primary
keys are flagged; true INTEGER rowid aliases are not mistaken for nullable keys.

Expectations are deliberately bounded by migrations 0003/0004/0011/0013/0016/0017
and the design's dedupe requirement, not a proposed universal workbook schema:

- **0016 engineering_events** is a runtime findings log (`run_date`, `topic`,
  `headline`, `payload`, unique `dedupe_key`). It does not require workbook
  `import_batch`, `source_id`, `sheet_name`, `row_index` or `fields` columns.
- **0017 automation/research/finance** are frozen workbook captures. Check the
  capture-row composite key, event batch-row key and event dedupe uniqueness.
  Their source DDL declares `source_id` but no source FK: the inspector reports
  an unenforced association as a gap rather than claiming 0017 installed it.
  No refresh writer is invented for frozen workbooks.
- Source FK observations may refer to out-of-scope `*_sources` registries; the
  inspector does not add them to the 19-table scope or modify them.
- Staging parent identity is inventoried, but batch linkage and any recovered
  parent key still require actual deployed DDL and canonical-writer evidence.
- `engineering_records` and `ideas_ideas` receive only the design-level dedupe
  check, not a claim of complete column compatibility. A missing dedupe column
  is a reported mismatch, not permission to rewrite source identities.
- **home remains unknown**, whether its tables are present or absent. Existence
  does not establish a contract. Missing tables are gaps, never synthetic passes.

Catalog reads do not test orphan rows, FK enforcement on producer connections,
retry behavior, uncertain commits, row parity, trigger semantics, deployed
readers, or a natural ingest cycle. Every report states `live_gate_cleared=false`.
Before a domain cutover, separately establish those facts on an isolated target,
review the real writer's conflict target/update set, and observe live consumers.

## Focused verification receipt

Oracle test file: `scripts/tests/test_inspect_sheet_staging_schema.py`.
Command: `python3 -I -m unittest discover -s scripts/tests -p test_inspect_sheet_staging_schema.py -v`.
Only focused tests are requested; no full Python suite or production test.
Execution receipts (2026-11-25 UTC):

- Failing-first commit `1812db7e020` was pushed before implementation.
  Fleet dry-run selected `fleet-personal` (GitHub Actions); dispatched run
  [dezren39/fleet#45528581726](https://github.com/dezren39/fleet/actions/runs/45528581726),
  handle `fp_d466ef45a63809193c4dd50acd57c064`, completed **failure**.
  At 01:54:54Z the focused discovery command reported `Ran 1 test in 0.000s`,
  `FAILED (errors=1)`: import raised `FileNotFoundError` for the not-yet-existing
  `target/scripts/inspect_sheet_staging_schema.py`. This is the expected red,
  not an infrastructure failure or a production-schema observation.
- Implementation/review-fix commit
  `3416fd2e7b559e4d1dfe061afc5d9c14c1cade5a` is the pinned green-test input.
  Initial green run [45529363441](https://github.com/dezren39/fleet/actions/runs/45529363441)
  (handle `fp_018a8de5aa55b38d66c5fdb3a5f85b4f`, `fleet-personal`) completed
  **failure before tests**: Actions checkout treated the abbreviated SHA
  `3416fd2e7b5` as a branch/tag glob and fetch failed at 01:59:51Z. Retry uses
  the full 40-character SHA, unchanged code.
- Full-SHA retry [dezren39/fleet#45529947275](https://github.com/dezren39/fleet/actions/runs/45529947275),
  handle `fp_aa5de8ec6fbf49eee1936c1cf1e465a9`, completed **success** on
  `fleet-personal` (GitHub Actions). The runner printed
  `Target HEAD: 3416fd2e7b559e4d1dfe061afc5d9c14c1cade5a`; at 02:03:12Z the
  exact focused command reported `Ran 9 tests in 0.149s`, `OK`. All nine named
  oracles passed, including independent identity/nullability checks and the
  faithful 0016 runtime-findings fixture without synthetic guards.
  No output artifact handoff is needed for this stdlib-only test; the runner
  reported its dependency cache saved. These fixture tests prove inspector behavior,
  not deployed-schema compatibility or any live writer/reader gate.
- `git diff --check` passed locally. No local test fallback or full suite ran.
