# Offline Postgres COPY recovery archive

`python3 pg_copy_archive.py source.sql archive.db report.json`

Run locally with private source/output paths. Output must not already exist. No network is used. No SQL from the input is executed. Table names retain the source `schema.table` as one quoted SQLite identifier. Every decoded COPY field is TEXT or NULL; this preserves large integer/decimal precision, timestamp spelling, JSON, arrays and bytea lexical values. **This is not a production schema migration.** Defaults, constraints, triggers, RLS, functions and native types are not installed. Keep the original dump as authoritative schema evidence.

Verification oracle: source decoded rows versus ordered SQLite readback, per-table SHA256 and row count; SQLite integrity_check. Source SQL SHA256 also recorded. Empty COPY tables are retained; DDL-only tables (e.g. a partitioned parent) are retained as empty all-TEXT tables with columns parsed from the DDL; no production defaults or partitioning is invented.

`python3 -m unittest -v test_pg_copy_archive` tests COPY escapes, 1,000 deterministic randomly generated escaping round trips, null versus literal backslash-N, large integers and truncated streams. Malformed input fails, leaving an incomplete file for diagnosis; never deploy a file without the successful report. No destination records are overwritten.
