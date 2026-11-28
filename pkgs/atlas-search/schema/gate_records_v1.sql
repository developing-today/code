-- gate_records v1 DDL — the staging-shard ingest gate's per-batch record
-- (developing-today/code#7709, design
-- doc/2026-11-08T13-45-00Z_design_atlas_staging_shard_ingest_gate
-- sections 2, 4 and 5).
--
-- One row per batch, kept in the staging registry DB beside
-- staging_units. The row is written at the sensitivity of its batch, so
-- it holds counts, enumerated reason codes, row ids, content hashes and
-- the publish manifest id, never content (design section 8).

CREATE TABLE IF NOT EXISTS gate_records (
  batch_id            TEXT PRIMARY KEY,
  unit_id             TEXT NOT NULL,
  tier                TEXT NOT NULL CHECK (tier IN ('public', 'internal', 'secret')),
  state               TEXT NOT NULL CHECK (state IN ('open', 'closing', 'gated', 'published', 'quarantined')),
  stage_reached       TEXT,
  verdicts_json       TEXT NOT NULL,
  strikes_json        TEXT NOT NULL,
  needs_review        INTEGER NOT NULL CHECK (needs_review IN (0, 1)),
  publish_manifest_id TEXT,
  updated_at          TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS gate_records_unit ON gate_records (unit_id);
CREATE INDEX IF NOT EXISTS gate_records_state ON gate_records (state);
