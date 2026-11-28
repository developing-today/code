# Offline Postgres COPY recovery archive

`python3 pg_copy_archive.py source.sql archive.db report.json`

Run locally with private source/output paths. Output must not already exist. No network is used. No SQL from the input is executed. Table names retain the source `schema.table` as one quoted SQLite identifier. Every decoded COPY field is TEXT or NULL; this preserves large integer/decimal precision, timestamp spelling, JSON, arrays and bytea lexical values. **This is not a production schema migration.** Defaults, constraints, triggers, RLS, functions and native types are not installed. Keep the original dump as authoritative schema evidence.

Verification oracle: source decoded rows versus ordered SQLite readback, per-table SHA256 and row count; SQLite integrity_check. Source SQL SHA256 also recorded. Empty COPY tables are retained; DDL-only tables (e.g. a partitioned parent) are retained as empty all-TEXT tables with columns parsed from the DDL; no production defaults or partitioning is invented.

`python3 -m unittest -v test_pg_copy_archive` tests COPY escapes, 1,000 deterministic randomly generated escaping round trips, null versus literal backslash-N, large integers and truncated streams. Malformed input fails, leaving an incomplete file for diagnosis; never deploy a file without the successful report. No destination records are overwritten.

Supported COPY identifiers are unquoted ASCII SQL identifiers; quoted/comma-bearing names fail closed. Both complete source passes must have identical SHA256 or no success report is written. The inspected recovery source is immutable.

## Reviewed-candidate operational staging

`python3 prepare_runtime.py STAGING_DIR OUTPUT_DIR` prepares only the named 19 PM/AA tables from the audited November 17 schema inventory and lossless archive. It is **not** a general Postgres DDL translator and never connects to Turso. Inspect generated SQL before deployment. Outputs include a private SQLite candidate, DDL and column-type/row-count plan. Timestamp values become UTC ISO milliseconds; exact original microseconds and decimal representations remain in the lossless archive. Runtime numeric fields use SQLite REAL. One-dimensional PostgreSQL text arrays are converted to JSON arrays with NULL/quoting preserved; nested/dimensioned arrays fail closed. AA immutable-policy append-only guards cover UPDATE, DELETE and duplicate-key INSERT (including REPLACE).

Apply only after verifying Turso-specific constraint/trigger behavior. Existing live tables require missing-key-only reconciliation, canonical temporal key matching, and explicit conflict review, not replacement. The local candidate includes all ingest parent rows for FK verification; this is not permission to overwrite live parents. No data or credentials belong in the repository.

`import_runtime.py` defaults to dry-run and requires explicit reviewed SQLite and SQL hashes for apply. It guards existing parent uniqueness, preserves existing parent values, and creates new runtime tables atomically. An uncertain COMMIT is never automatically retried. `import_aa_missing.py` separately restores only the reviewed three source and 157 sheet IDs, refusing existing IDs or drift in baseline counts/column layout. Existing 79 AA conflicts, including source 41, are never updated. Both importers refuse HTTP redirects and require intact transaction batons.
