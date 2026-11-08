-- Synthetic, isolated search fixture for developing-today/code#2469.
-- This is not the production Atlas corpus. It mirrors the small subset
-- of the publish schema needed to prove that an indexed DNS source can
-- be returned by Atlas's vector-search path.

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
  ('dns-source', 'infra/dns/dns_config.yaml', 'atlas-fixture', '2026-11-08T00:00:00Z');
INSERT INTO session_labels (session_id, client, kind, topics, projects, sensitivity, confidence, scheme) VALUES
  ('dns-source', 'atlas-fixture', 'code', '["dns", "infrastructure"]', '["developing-today/code"]', 'shareable', 'high', 'fixture');
INSERT INTO messages (id, session_id, role, text) VALUES
  ('dns-message', 'dns-source', 'document', '01b.us MX priority 10: monday.mxrouting.net');
INSERT INTO chunks (id, message_id, chunk_idx, text, content_hash, sensitivity) VALUES
  (1, 'dns-message', 0, 'source: infra/dns/dns_config.yaml\ndomain: 01b.us\nrecord: MX\npriority: 10\ncontent: monday.mxrouting.net', 'dns-config-01b-us-mx', 'shareable');
INSERT INTO chunks_fts (rowid, text) VALUES
  (1, 'source infra/dns/dns_config.yaml domain 01b.us record MX priority 10 content monday.mxrouting.net');
