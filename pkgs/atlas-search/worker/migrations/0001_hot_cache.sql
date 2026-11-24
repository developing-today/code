-- Atlas D1 tenant hot cache (#9134 design §5; implementation #13100).
-- One shared D1 database; the tenant is carried in the key, never in
-- a per-tenant database. Applied with `wrangler d1 migrations apply`
-- (or the cf equivalent) against the HOT_CACHE binding's database.
CREATE TABLE hot_cache (
  tenant_id        TEXT    NOT NULL,
  cache_key        TEXT    NOT NULL,
  class            TEXT    NOT NULL,  -- 'A' | 'B' | 'C'
  payload          BLOB    NOT NULL,
  payload_bytes    INTEGER NOT NULL,
  source_watermark TEXT    NOT NULL,
  filled_at        INTEGER NOT NULL,  -- unix seconds
  last_hit_at      INTEGER NOT NULL,
  hit_count        INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (tenant_id, cache_key)
);
CREATE INDEX hot_cache_eviction ON hot_cache (tenant_id, last_hit_at);
