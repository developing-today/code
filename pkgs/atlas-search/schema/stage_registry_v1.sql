-- stage_registry v1 DDL — the staging-unit registry
-- (developing-today/code#5669, design
-- doc/2026-11-04T21-27-34Z_design_atlas_scaler_revision section 4.1).
--
-- "A registry table (in the internal core tier) maps unit id ->
-- staging DB, owner, sensitivity level, state (open / closing /
-- merged), schema version."
--
-- This is the registry's own store; it is created and owned by the
-- provisioning tool (atlas_core::stage::Registry). One row per staging
-- unit. `db_uri` is the staging database's location as the tool that
-- created it understands it (a local file path today); promotion and
-- the aggregator read it to find the unit's staging DB.

CREATE TABLE IF NOT EXISTS staging_units (
  unit_id        TEXT PRIMARY KEY,
  db_uri         TEXT NOT NULL,
  owner          TEXT NOT NULL,
  sensitivity    TEXT NOT NULL,
  state          TEXT NOT NULL CHECK (state IN ('open', 'closing', 'merged')),
  schema_version INTEGER NOT NULL,
  created_at     TEXT NOT NULL,
  updated_at     TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS staging_units_state ON staging_units (state);
