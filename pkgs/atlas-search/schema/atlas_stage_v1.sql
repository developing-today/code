-- atlas_stage v1 DDL — the standard versioned schema every Atlas
-- staging database carries (developing-today/code#5669, design
-- doc/2026-11-04T21-27-34Z_design_atlas_scaler_revision section 4.1:
-- "Standard versioned schema across every staging DB (`atlas_stage`
-- v1: `items`, `chunks_raw`, `notes`, `kv`, `_schema_version`). Same
-- DDL file, applied by one tool".
--
-- Version: 1. Bump this file (and STAGE_SCHEMA_VERSION in
-- crates/atlas-core/src/stage.rs) only by adding a new vN artifact;
-- never edit an applied version in place — a staging DB is promoted
-- by being READ, never by being migrated.
--
-- The design names the five tables but not their columns; the columns
-- below are the v1 realization of that table set and the design's
-- staging-tier rules (unmediated free writes, unlimited detail,
-- sensitivity recorded on the row, promotion decides later). Column
-- choices that go beyond the design's wording are recorded on
-- issue #5669.
--
-- Applied exactly once, to an empty database (see
-- atlas_core::stage::provision_staging_db). No IF NOT EXISTS: a
-- re-provision over an existing staging set is an error, not a
-- silent no-op.

-- ------------------------------------------------------- version ------

CREATE TABLE _schema_version (
  version INTEGER NOT NULL
);

INSERT INTO _schema_version (version) VALUES (1);

-- --------------------------------------------------------- items ------
-- One row per staged item (a message, note, doc fragment, ...). The
-- unit's owner writes here directly; nothing is mediated at write
-- time (design 4.1), so `sensitivity` is recorded, not enforced, and
-- `meta` is an open JSON bag for producer-specific detail.

CREATE TABLE items (
  id          INTEGER PRIMARY KEY,
  unit_id     TEXT    NOT NULL,
  kind        TEXT    NOT NULL,
  source      TEXT,
  text        TEXT    NOT NULL,
  sensitivity TEXT    NOT NULL DEFAULT 'internal',
  created_at  TEXT    NOT NULL,
  meta        TEXT
);

CREATE INDEX items_unit_kind ON items (unit_id, kind);

-- ---------------------------------------------------- chunks_raw ------
-- Raw, un-embedded chunks. The aggregator embeds at promotion; a
-- staging DB never stores vectors, so a staging DB is cheap to write
-- and cheap to branch.

CREATE TABLE chunks_raw (
  id           INTEGER PRIMARY KEY,
  item_id      INTEGER NOT NULL REFERENCES items (id) ON DELETE CASCADE,
  idx          INTEGER NOT NULL,
  text         TEXT    NOT NULL,
  content_hash TEXT    NOT NULL,
  created_at   TEXT    NOT NULL
);

CREATE INDEX chunks_raw_item ON chunks_raw (item_id, idx);
CREATE INDEX chunks_raw_hash ON chunks_raw (content_hash);

-- --------------------------------------------------------- notes ------
-- Free working notes the unit's owner keeps against the unit
-- (design 4.1: "an epic/repo staging DB is shared working notes for
-- the life of the epic").

CREATE TABLE notes (
  id         INTEGER PRIMARY KEY,
  unit_id    TEXT NOT NULL,
  body       TEXT NOT NULL,
  created_at TEXT NOT NULL,
  meta       TEXT
);

CREATE INDEX notes_unit ON notes (unit_id);

-- ------------------------------------------------------------ kv ------
-- Key/value scratch space: cursors, checkpoint state, producer
-- bookkeeping. Last write wins.

CREATE TABLE kv (
  key        TEXT PRIMARY KEY,
  value      TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
