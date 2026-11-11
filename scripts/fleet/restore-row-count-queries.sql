-- Restore row-count gate: query set (#6994)
-- Companion to doc/fleet/restore-procedure.md and
-- scripts/fleet/restore-row-count-diff.sh.
--
-- Produce two TSV shapes from these queries:
--   inventory: one source table name per line (no header)
--   live counts: header "table<TAB>live_count", one row per table
-- Run the inventory + counts against the SOURCE scope before the
-- export is built, and the counts against the LIVE store after load.
-- Counts must be exact (count(*)), never planner estimates, at the
-- moment the gate runs; take them in one pass per store so the diff
-- is internally consistent.

-- === SQLite / Turso (libSQL) ======================================

-- Inventory: every table in scope.
SELECT name
FROM sqlite_schema
WHERE type = 'table'
  AND name NOT LIKE 'sqlite_%'
ORDER BY name;

-- Counts: one UNION ALL leg per manifest table, e.g.
--   SELECT 'tasks' AS table_name, count(*) AS live_count FROM tasks
--   UNION ALL
--   SELECT 'lanes', count(*) FROM lanes
-- Generate the legs from the inventory so no table can be skipped:
SELECT 'SELECT ' || quote(name) || ' AS table_name, count(*) AS live_count FROM ' || quote(name)
FROM sqlite_schema
WHERE type = 'table'
  AND name NOT LIKE 'sqlite_%'
ORDER BY name;
-- (Run the generated legs joined by UNION ALL, with the header row
-- added by your export wrapper: table <TAB> live_count.)

-- === PostgreSQL (snapshot source) =================================

-- Inventory: every table in the schema(s) in scope.
SELECT table_schema || '.' || table_name AS table_name
FROM information_schema.tables
WHERE table_type = 'BASE TABLE'
  AND table_schema NOT IN ('pg_catalog', 'information_schema')
ORDER BY 1;

-- Counts: exact per-table counts via a generated UNION ALL, e.g.
--   SELECT 'agent.lanes' AS table_name, count(*) AS live_count FROM agent.lanes
-- Generate the legs from the inventory query above (same pattern as
-- the SQLite generator), so the counts cover exactly the inventory.
--
-- A pg_dump also carries the inventory in its COPY headers:
--   zstd -dc latest-backup.pgdump.sql.zst | grep '^COPY '
-- lists every exported table; a source table absent from that list
-- and absent from the manifest is the #6853 failure shape.
