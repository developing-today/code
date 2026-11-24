-- Per-tenant invalidation state and the operator purge audit trail
-- for the D1 tenant hot cache (design #9134 §4; implementation
-- #13106). Applied after 0001_hot_cache.sql against the HOT_CACHE
-- binding's database.
CREATE TABLE hot_cache_tenant_state (
  tenant_id                  TEXT PRIMARY KEY,
  forced_revalidation        INTEGER NOT NULL DEFAULT 0,  -- 0 | 1
  last_invalidation_at       INTEGER,  -- unix seconds, NULL until the first event or purge
  last_invalidated_watermark TEXT,     -- watermark carried by the newest event
  updated_at                 INTEGER NOT NULL  -- unix seconds
);
CREATE TABLE hot_cache_purge_audit (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  tenant_id       TEXT    NOT NULL,
  operator        TEXT    NOT NULL,
  reason          TEXT    NOT NULL,
  purged_at       INTEGER NOT NULL,  -- unix seconds
  entries_deleted INTEGER NOT NULL
);
