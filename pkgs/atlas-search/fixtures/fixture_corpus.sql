-- Fixture corpus for atlas-core parity tests (developing-today/code#5352).
--
-- A miniature publish-shaped Atlas DB: sessions -> messages -> chunks,
-- session_labels, a chunks_fts FTS5 table, and sensitivity denormalized
-- onto chunks (the publish schema). All content is synthetic.
--
-- Sensitivity layout is the point of the fixture:
--   chunk 5 (secret-bearing) carries the query's exact vector (sim 1.0)
--   and FTS terms -- it must be ABSENT at the default ceiling, not
--   down-ranked, and PRESENT (top) when the ceiling is spelled out.
--   chunk 6 (local-only) is nearly as close and likewise excluded by
--   the default `personal` ceiling.
-- Vectors live in fixture_spec.json (this file cannot hold blobs);
-- both the Python fixture generator and the Rust tests insert them
-- into vec_snowflake_m_v2_768_f32 from that one source.

CREATE TABLE sessions (
  id TEXT PRIMARY KEY,
  title TEXT,
  client TEXT,
  created_at TEXT
);
CREATE TABLE messages (
  id TEXT PRIMARY KEY,
  session_id TEXT,
  role TEXT,
  text TEXT
);
CREATE TABLE chunks (
  id INTEGER PRIMARY KEY,
  message_id TEXT NOT NULL,
  chunk_idx INTEGER,
  text TEXT NOT NULL,
  content_hash TEXT UNIQUE NOT NULL,
  sensitivity TEXT
);
CREATE TABLE session_labels (
  session_id TEXT PRIMARY KEY,
  client TEXT,
  kind TEXT,
  topics TEXT,
  projects TEXT,
  sensitivity TEXT,
  confidence TEXT,
  scheme TEXT
);
CREATE TABLE vec_snowflake_m_v2_768_f32 (
  chunk_id INTEGER PRIMARY KEY,
  vec BLOB NOT NULL
);
CREATE VIRTUAL TABLE chunks_fts USING fts5(text);

INSERT INTO sessions (id, title, client, created_at) VALUES
  ('sA', 'NixOS backup chat',       'claude-code', '2026-10-05T14:23:00Z'),
  ('sB', 'Rust refactor session',   'codex',       '2026-10-20T09:12:00Z'),
  ('sC', 'Secret plans',            'claude-code', '2026-11-02T03:04:05Z'),
  ('sD', 'Local scratch',           'opencode',    '2026-09-15T22:40:10Z');

INSERT INTO session_labels (session_id, client, kind, topics, projects, sensitivity, confidence, scheme) VALUES
  ('sA', 'claude-code', 'chat', '["nixos", "backup"]', '["atlas"]', 'personal',       'high', '1.0'),
  ('sB', 'codex',       'code', '["rust"]',            '["atlas"]', 'shareable',      'high', '1.0'),
  ('sC', 'claude-code', 'chat', '["plans"]',           '[]',        'secret-bearing', 'high', '1.0'),
  ('sD', 'opencode',    'chat', '["nixos"]',           '[]',        'local-only',     'high', '1.0');

INSERT INTO messages (id, session_id, role, text) VALUES
  ('mA1', 'sA', 'user',      'how do we back up the database with restic and check the secret backup database master plan'),
  ('mA2', 'sA', 'assistant', 'the backup script uses restic and rclone; database backup verification steps follow'),
  ('mB1', 'sB', 'user',      'refactoring the rust parser for speed; cargo test failures in the queue worker'),
  ('mC1', 'sC', 'user',      'secret backup database master plan'),
  ('mD1', 'sD', 'assistant', 'local notes about database backup');

INSERT INTO chunks (id, message_id, chunk_idx, text, content_hash, sensitivity) VALUES
  (1, 'mA1', 0, 'how do we back up the database with restic',        'fxh1', 'personal'),
  (2, 'mA1', 1, 'restic snapshots go to the turso bucket nightly',  'fxh2', 'personal'),
  (3, 'mA2', 0, 'the backup script uses restic and rclone',         'fxh3', 'personal'),
  (4, 'mB1', 0, 'refactoring the rust parser for speed',            'fxh4', 'shareable'),
  (5, 'mC1', 0, 'secret backup database master plan',               'fxh5', 'secret-bearing'),
  (6, 'mD1', 0, 'local notes about database backup',                'fxh6', 'local-only'),
  (7, 'mB1', 1, 'cargo test failures in the queue worker',          'fxh7', 'shareable'),
  (8, 'mA2', 1, 'database backup verification steps',               'fxh8', 'personal');

INSERT INTO chunks_fts (rowid, text) VALUES
  (1, 'how do we back up the database with restic'),
  (2, 'restic snapshots go to the turso bucket nightly'),
  (3, 'the backup script uses restic and rclone'),
  (4, 'refactoring the rust parser for speed'),
  (5, 'secret backup database master plan'),
  (6, 'local notes about database backup'),
  (7, 'cargo test failures in the queue worker'),
  (8, 'database backup verification steps');
