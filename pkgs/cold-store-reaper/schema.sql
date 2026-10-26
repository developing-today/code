-- Cold-store manifest schema v1 (2026-10-25)
-- Canonical index for the Drive cold store. Drive holds bundles; this DB is the system.
-- v1.2 (reaper v4): objects.state gains 'evicted' — still cold in Drive, but
-- deleted from R2 by `cold-store evict`. The D1 copy of this schema already
-- carries the same CHECK change (verified 2026-10-26, #984: D1 sqlite_master
-- read plus a probe sync-manifest with an 'evicted' row succeeded; the probe
-- rows were removed afterwards).
CREATE TABLE IF NOT EXISTS objects (
  hash          TEXT PRIMARY KEY,            -- sha256 hex of object content
  size          INTEGER NOT NULL,
  content_type  TEXT,
  source_bucket TEXT,                        -- R2 bucket it was evicted from
  source_key    TEXT,                        -- R2 key it was evicted from
  first_seen    TEXT NOT NULL DEFAULT (datetime('now')),
  last_seen_hot TEXT,                        -- last time it was present in R2
  state         TEXT NOT NULL DEFAULT 'hot'
                CHECK (state IN ('hot','staging','cold','rehydrating','gone','evicted'))
);
CREATE INDEX IF NOT EXISTS idx_objects_state ON objects(state);

CREATE TABLE IF NOT EXISTS bundles (
  bundle_id     TEXT PRIMARY KEY,            -- 'b_' + first 16 hex of bundle sha256
  drive_file_id TEXT,                        -- null until upload verified
  sha256        TEXT NOT NULL,
  drive_md5     TEXT,                        -- MD5 reported by Drive after upload
  bytes         INTEGER NOT NULL,
  member_count  INTEGER NOT NULL DEFAULT 0,
  live_bytes    INTEGER NOT NULL DEFAULT 0,  -- bytes still referenced by live objects
  sealed_at     TEXT,
  uploaded_at   TEXT,
  state         TEXT NOT NULL DEFAULT 'open'
                CHECK (state IN ('open','sealed','uploading','verified','compacting','trashed','deleted'))
);
CREATE INDEX IF NOT EXISTS idx_bundles_state ON bundles(state);

CREATE TABLE IF NOT EXISTS members (
  bundle_id   TEXT NOT NULL REFERENCES bundles(bundle_id),
  object_hash TEXT NOT NULL REFERENCES objects(hash),
  offset      INTEGER NOT NULL,              -- byte offset of the member's zstd frame in the bundle file
  length      INTEGER NOT NULL,              -- frame length as stored (range-pull exactly these bytes)
  PRIMARY KEY (bundle_id, object_hash)
);
CREATE INDEX IF NOT EXISTS idx_members_hash ON members(object_hash);

-- Reaper/rehydrator budget ledger: one row per action, 24h rolling window computed from ts.
-- v1.1 (reaper v3): direction gains 'trash' for backup-prune rows (kind='backup-prune').
-- The D1 copy of this schema already carries the same CHECK change
-- (verified 2026-10-26, #984 — D1 already held real 'trash' ledger rows).
CREATE TABLE IF NOT EXISTS transfer_ledger (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  ts        TEXT NOT NULL DEFAULT (datetime('now')),
  direction TEXT NOT NULL CHECK (direction IN ('upload','download','trash')),
  bytes     INTEGER NOT NULL,
  kind      TEXT,                            -- bundle | db-backup | manifest | rehydrate | compact
  ref       TEXT                             -- bundle_id or file name
);
CREATE INDEX IF NOT EXISTS idx_ledger_ts ON transfer_ledger(ts);

CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
INSERT OR IGNORE INTO meta (key, value) VALUES ('schema_version', '1');
INSERT OR IGNORE INTO meta (key, value) VALUES ('project_budget_bytes', '3298534883328'); -- 3 TiB
INSERT OR IGNORE INTO meta (key, value) VALUES ('daily_upload_hard_stop_bytes', '700000000000'); -- 700 GB
