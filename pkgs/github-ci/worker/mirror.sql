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
  parent_number INTEGER,
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

-- Deleted agent-note markers: an agent note's "deleted" delivery
-- clears the latest-note pointer AND records the comment id here, so
-- a delayed created/edited redelivery of the same comment cannot
-- resurrect the pointer through the empty-pointer branch (Codex P2
-- on PR #8604). A recreated note always has a new comment id, so a
-- marker never suppresses a live note.
CREATE TABLE IF NOT EXISTS github_mirror_deleted_notes (
  repo TEXT NOT NULL, number INTEGER NOT NULL,
  comment_id INTEGER NOT NULL,
  mirrored_at TEXT NOT NULL,
  PRIMARY KEY (repo, number, comment_id));

-- GitHub-side change history (#14170 Q7; design
-- doc/2026-11-24T07-23-14Z_design_queue-metadata-history §8): one
-- append-only row per observed change the mirror's upserts would
-- otherwise overwrite silently — body edits (hashes only, never
-- bodies), comment-count deltas (counts only, never comment
-- bodies), and sub-issue parent changes (parent_number above is the
-- projection; this table is its change log). kind is one of
-- body_edited | comment_added | comment_removed |
-- comment_count_changed | parent_changed (linked | unlinked |
-- state_changed are reserved vocabulary, not written by Q7).
-- Capture is FAIL-OPEN: a failed history write never fails the
-- mirror upsert it describes, so history reads as "as observed"
-- and the backfill sweep's own diff rows repair missed deliveries.
CREATE TABLE IF NOT EXISTS github_issue_history (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  repo TEXT NOT NULL, number INTEGER NOT NULL,
  is_pr INTEGER NOT NULL DEFAULT 0,
  kind TEXT NOT NULL,
  detail TEXT NOT NULL DEFAULT '{}',
  actor TEXT, gh_updated_at TEXT,
  mirrored_at TEXT NOT NULL);

CREATE INDEX IF NOT EXISTS github_issue_history_issue
  ON github_issue_history(repo, number, id);

-- Label add/remove history (#14170 Q5; design §7.2): one append-only
-- row per observed change to an issue's or PR's label set — the
-- exact names added and removed (diffed stored → incoming under the
-- upserts' own acceptance rules) and the full set after the change.
-- action is the webhook action (labeled | unlabeled | opened |
-- edited | synchronize | …) or 'backfill' for sweep-written rows;
-- actor is the webhook sender login or 'backfill'. Each item gets a
-- defined START row the first time its labels are observed (the
-- whole set as added, removed = '[]'), written only while the item
-- has no label history, so any item's timeline replays from row 1.
-- Labels belong to the issue, not to queue tasks: queue items read
-- this by joining on their issue ref (design D6). FAIL-OPEN like
-- github_issue_history.
CREATE TABLE IF NOT EXISTS github_label_history (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  repo TEXT NOT NULL, number INTEGER NOT NULL,
  is_pr INTEGER NOT NULL DEFAULT 0,
  added TEXT NOT NULL DEFAULT '[]',
  removed TEXT NOT NULL DEFAULT '[]',
  label_set TEXT NOT NULL,
  action TEXT NOT NULL,
  actor TEXT, gh_updated_at TEXT,
  mirrored_at TEXT NOT NULL);

CREATE INDEX IF NOT EXISTS github_label_history_issue
  ON github_label_history(repo, number, id);
