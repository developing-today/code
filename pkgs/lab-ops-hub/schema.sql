-- Lab Ops Hub — D1 schema
-- Machines inventory for the basement cluster, notes, tasks, file metadata,
-- crawl results, and a self-counting usage ledger for /api/budget.

CREATE TABLE IF NOT EXISTS machines (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL UNIQUE,
  role TEXT NOT NULL DEFAULT 'compute',        -- compute | storage | control | network
  cpu TEXT, ram_gb INTEGER, storage TEXT, gpu TEXT,
  status TEXT NOT NULL DEFAULT 'planned',       -- planned | building | online | offline
  notes TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS notes (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  title TEXT NOT NULL,
  body TEXT NOT NULL,
  summary TEXT,                                  -- filled by Workers AI on demand
  tags TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS tasks (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  title TEXT NOT NULL,
  done INTEGER NOT NULL DEFAULT 0,
  machine_id INTEGER REFERENCES machines(id),
  due TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS files (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  r2_key TEXT NOT NULL UNIQUE,
  filename TEXT NOT NULL,
  content_type TEXT,
  size INTEGER,
  sha256 TEXT,
  indexed INTEGER NOT NULL DEFAULT 0,            -- set by Queue consumer
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS crawls (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  url TEXT NOT NULL,
  source TEXT,                                   -- container | worker-fallback
  title TEXT, excerpt TEXT, links INTEGER, status_code INTEGER,
  r2_key TEXT,                                   -- full extracted JSON in R2
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS usage_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  kind TEXT NOT NULL,                            -- worker_request | d1_write | r2_put | queue_msg | do_request | ai_request | container_run
  detail TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_usage_kind ON usage_events(kind, created_at);
