-- GitHub mirror schema (#8580, fleet unification slice 2).
-- Home: the fleet-queue Turso database. The github-ci Worker ensures
-- these tables idempotently on every mirror write (src/mirror.ts
-- MIRROR_DDL mirrors this file); this file is the human-readable DDL
-- of record and the rebuild starting point.
--
-- The mirror is a READ MODEL: rebuildable from GitHub by the
-- backfill/repair sweep, never a second truth. github_mirror_meta
-- carries the staleness readout (last webhook, last successful
-- backfill) — it informs readers; it never gates work (#8575).

CREATE TABLE IF NOT EXISTS github_mirror_issues (
  repo TEXT NOT NULL, number INTEGER NOT NULL,
  title TEXT, body_hash TEXT, state TEXT,
  labels TEXT NOT NULL DEFAULT '[]',
  assignees TEXT NOT NULL DEFAULT '[]',
  comments INTEGER NOT NULL DEFAULT 0,
  comments_gh_updated_at TEXT,
  latest_agent_note_id INTEGER, latest_agent_note_url TEXT,
  author TEXT, html_url TEXT,
  gh_created_at TEXT, gh_updated_at TEXT,
  mirrored_at TEXT NOT NULL,
  PRIMARY KEY (repo, number));

CREATE TABLE IF NOT EXISTS github_mirror_pull_requests (
  repo TEXT NOT NULL, number INTEGER NOT NULL,
  title TEXT, body_hash TEXT, state TEXT,
  merged INTEGER NOT NULL DEFAULT 0,
  draft INTEGER NOT NULL DEFAULT 0,
  head_sha TEXT, head_ref TEXT, base_ref TEXT,
  labels TEXT NOT NULL DEFAULT '[]',
  assignees TEXT NOT NULL DEFAULT '[]',
  comments INTEGER NOT NULL DEFAULT 0,
  comments_gh_updated_at TEXT,
  latest_agent_note_id INTEGER, latest_agent_note_url TEXT,
  author TEXT, html_url TEXT,
  gh_created_at TEXT, gh_updated_at TEXT,
  mirrored_at TEXT NOT NULL,
  PRIMARY KEY (repo, number));

CREATE TABLE IF NOT EXISTS github_mirror_meta (
  repo TEXT PRIMARY KEY,
  last_webhook_at TEXT, last_webhook_event TEXT,
  last_backfill_at TEXT);

-- Deletion tombstones: an issues "deleted" delivery removes the
-- mirror row and records the deleted issue's updated_at here, so a
-- delayed pre-deletion snapshot or comment is absorbed by the
-- marker instead of resurrecting the row (the projector's
-- deletionBlocks rule; Codex P2 on PR #8604).
CREATE TABLE IF NOT EXISTS github_mirror_tombstones (
  repo TEXT NOT NULL, number INTEGER NOT NULL,
  gh_updated_at TEXT,
  mirrored_at TEXT NOT NULL,
  PRIMARY KEY (repo, number));
