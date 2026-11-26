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
  updated_at     TEXT NOT NULL,
  -- Per-corpus embedding-model identity (#8623 slice 2): the model +
  -- pipeline identity every corpus's vectors are produced with. Fresh
  -- databases record it at provision time; pre-slice-2 databases gain
  -- it through the ensure_registry migration below. Ingestion refuses
  -- (or loudly flags) a corpus whose recorded model is not the
  -- mandated one, so a model change is an explicit re-embed slice,
  -- never silent drift.
  embedding_model TEXT NOT NULL DEFAULT 'snowflake-arctic-embed-m-v2.0'
);

-- Durable audit record for each successful staging-unit promotion.
-- Kept in the registry DB after the unit leaves the active states.
CREATE TABLE IF NOT EXISTS merge_manifests (
  unit_id TEXT PRIMARY KEY,
  rows_in INTEGER NOT NULL,
  rows_promoted INTEGER NOT NULL,
  content_hash TEXT NOT NULL,
  embedding_model TEXT NOT NULL,
  promoted_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS staging_units_state ON staging_units (state);
