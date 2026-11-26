-- Retained Atlas objects: recommendations, search history and hot-data
-- snapshots as versioned, timestamped snapshots under per-class
-- retention (#9140 design record, build slice 1). Applied after
-- 0002_hot_cache_invalidation.sql. Nothing writes or reads these tables
-- until a family's writer lands behind its enable flag (design §6).

-- The four retention classes. A class may lengthen its window or raise
-- its keep-newest floor, never go below the design default (§5.3): the
-- CHECKs refuse such a row at write time.
CREATE TABLE atlas_retention_classes (
  class              TEXT    PRIMARY KEY CHECK (class IN ('hot-short', 'recommendation', 'search-history', 'snapshot-audit')),
  window_ms          INTEGER NOT NULL CHECK (window_ms >= CASE class
                       WHEN 'hot-short' THEN 86400000
                       WHEN 'recommendation' THEN 2592000000
                       WHEN 'search-history' THEN 7776000000
                       WHEN 'snapshot-audit' THEN 31536000000
                     END),
  keep_newest        INTEGER NOT NULL CHECK (keep_newest >= CASE class
                       WHEN 'hot-short' THEN 2
                       WHEN 'recommendation' THEN 10
                       WHEN 'search-history' THEN 20
                       WHEN 'snapshot-audit' THEN 50
                     END),
  allows_query_text  INTEGER NOT NULL DEFAULT 0 CHECK (allows_query_text IN (0, 1)),
  enabled            INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
  updated_at_ms      INTEGER NOT NULL
);

-- One row per version of one logical object. Versions are appended,
-- never overwritten; the current version is the highest.
CREATE TABLE atlas_retained_objects (
  object_id    TEXT    NOT NULL CHECK (length(object_id) > 0),
  version      INTEGER NOT NULL CHECK (version >= 1),
  family       TEXT    NOT NULL CHECK (family IN ('recommendation', 'search_history', 'hot_snapshot')),
  class        TEXT    NOT NULL REFERENCES atlas_retention_classes (class),
  captured_at  TEXT    NOT NULL,
  inputs_ref   TEXT    NOT NULL CHECK (length(inputs_ref) > 0),
  payload      TEXT    NOT NULL,
  written_at_ms INTEGER NOT NULL,
  PRIMARY KEY (object_id, version),
  -- Each family has its own class; snapshot-audit holds any family.
  CHECK (class = 'snapshot-audit'
    OR (class = 'hot-short' AND family = 'hot_snapshot')
    OR (class = 'recommendation' AND family = 'recommendation')
    OR (class = 'search-history' AND family = 'search_history'))
);
CREATE INDEX atlas_retained_objects_class ON atlas_retained_objects (class, captured_at);
