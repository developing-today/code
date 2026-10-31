-- PM projector + link registry schema (libSQL / Turso).
-- The projector is glue: GitHub stays the source of truth for code work,
-- Kaneo/Grist stay the source of truth for their own layers, and this
-- database only holds links and projections. It is disposable — every row
-- can be rebuilt from GitHub + the link writes. It never writes back.

PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

-- ---------------------------------------------------------------------------
-- GitHub items (issues and PRs), keyed by repo-qualified ref "owner/repo#N".
-- A ref is unique and stable across repos; it is the join key everywhere.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS pmp_items (
  ref               TEXT PRIMARY KEY,        -- "owner/repo#123"
  repo              TEXT NOT NULL,           -- "owner/repo"
  number            INTEGER NOT NULL,
  kind              TEXT NOT NULL,           -- 'issue' | 'pr'
  url               TEXT NOT NULL,           -- html_url
  api_url           TEXT NOT NULL,           -- api url, for reconciliation
  title             TEXT NOT NULL,
  state             TEXT NOT NULL,           -- 'open' | 'closed'
  state_reason      TEXT,                    -- 'completed' | 'not_planned' | null
  draft             INTEGER NOT NULL DEFAULT 0, -- PRs only
  merged            INTEGER NOT NULL DEFAULT 0, -- PRs only
  author            TEXT,                    -- login
  labels            TEXT NOT NULL DEFAULT '[]', -- JSON array of label names
  assignees         TEXT NOT NULL DEFAULT '[]', -- JSON array of logins
  created_at        TEXT NOT NULL,           -- ISO-8601 (UTC)
  updated_at        TEXT NOT NULL,
  closed_at         TEXT,
  merged_at         TEXT,
  source_updated_at TEXT NOT NULL,           -- updated_at, monotonic guard
  fetched_at        TEXT NOT NULL            -- ISO-8601 when we last wrote it
);

CREATE INDEX IF NOT EXISTS pmp_items_repo ON pmp_items(repo);
CREATE INDEX IF NOT EXISTS pmp_items_state ON pmp_items(state);
CREATE INDEX IF NOT EXISTS pmp_items_kind ON pmp_items(kind);

-- ---------------------------------------------------------------------------
-- Comments across all four repos, including PR-interior comments that Kaneo
-- never syncs (reviews and review/line comments). This is the unified
-- comment-activity projection (aggregator requirement #2).
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS pmp_comments (
  id             TEXT NOT NULL,              -- GitHub comment/review id
  ref            TEXT NOT NULL,              -- item ref this comment belongs to
  kind           TEXT NOT NULL,              -- 'issue_comment' | 'review' | 'review_comment'
  author         TEXT,
  body           TEXT,
  created_at     TEXT NOT NULL,
  updated_at     TEXT,
  url            TEXT,
  -- review comments only:
  path           TEXT,
  line           INTEGER,
  diff_hunk      TEXT,
  -- reviews only:
  review_state   TEXT,                       -- 'commented' | 'approved' | 'changes_requested' | 'dismissed'
  submitted_at   TEXT,
  commit_id      TEXT,
  deleted        INTEGER NOT NULL DEFAULT 0, -- set to 1 on GitHub delete events
  source_updated_at TEXT NOT NULL,           -- created_at (or updated_at) monotonic guard
  fetched_at     TEXT NOT NULL,
  PRIMARY KEY (id, kind)
);

CREATE INDEX IF NOT EXISTS pmp_comments_ref ON pmp_comments(ref);
CREATE INDEX IF NOT EXISTS pmp_comments_kind ON pmp_comments(kind);
CREATE INDEX IF NOT EXISTS pmp_comments_created ON pmp_comments(created_at);

-- ---------------------------------------------------------------------------
-- The link registry (aggregator requirement #1): repo-qualified GitHub
-- issue/PR references attached to any board item or roadmap row, regardless
-- of which repo the item belongs to. Subjects are external keys:
--   kaneo:<projectSlug>/<taskId>      board items
--   grist:<docId>/<tableId>/<rowId>   roadmap rows
--   gh:<ref>                          (an item, for one-hop "related" links)
-- relation is a loose, human-chosen label: 'tracks' | 'implements' | 'relates'.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS pmp_links (
  subject    TEXT NOT NULL,           -- external key above
  ref        TEXT NOT NULL,           -- "owner/repo#N"
  relation   TEXT NOT NULL DEFAULT 'relates', -- 'tracks' | 'implements' | 'relates'
  note       TEXT,
  created_by TEXT,
  created_at TEXT NOT NULL,
  PRIMARY KEY (subject, ref, relation)
);

CREATE INDEX IF NOT EXISTS pmp_links_subject ON pmp_links(subject);
CREATE INDEX IF NOT EXISTS pmp_links_ref ON pmp_links(ref);

-- ---------------------------------------------------------------------------
-- Webhook deliveries: idempotency + audit. The GitHub delivery GUID is the
-- unique key, so a redelivered webhook is applied at most once.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS pmp_deliveries (
  delivery_id TEXT PRIMARY KEY,
  event       TEXT NOT NULL,
  action      TEXT,
  ref         TEXT,                   -- item ref touched, if any
  received_at TEXT NOT NULL,
  outcome     TEXT NOT NULL,          -- 'applied' | 'duplicate' | 'ignored' | 'error'
  detail      TEXT
);

-- ---------------------------------------------------------------------------
-- Reconciliation cursors, one per (repo, stream). Idempotent upserts keyed by
-- repo-qualified IDs are the safety net; these cursors make the poll cheap.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS pmp_sync_state (
  repo    TEXT NOT NULL,
  stream  TEXT NOT NULL,              -- 'issues' | 'prs' | 'issue_comments' | 'review_comments' | 'reviews'
  cursor  TEXT,                       -- opaque continuation token / etag / since
  updated_at TEXT NOT NULL,
  PRIMARY KEY (repo, stream)
);

-- ---------------------------------------------------------------------------
-- Push events: recorded for audit and for later exec-strip aggregation.
-- The projector does not write back to GitHub; pushes are read-only here.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS pmp_push_events (
  delivery_id TEXT PRIMARY KEY,
  repo        TEXT NOT NULL,
  ref         TEXT,                   -- git ref pushed, e.g. "refs/heads/main"
  head_sha    TEXT,
  before_sha  TEXT,
  pusher      TEXT,
  created_at  TEXT NOT NULL
);

-- ---------------------------------------------------------------------------
-- Read model (views). Session K (aggregator UI) consumes these directly from
-- the same libSQL database, or over the HTTP API.
-- ---------------------------------------------------------------------------

-- Every link, joined to the item's live state and title.
CREATE VIEW IF NOT EXISTS pmp_v_links_items AS
  SELECT l.subject, l.ref, l.relation, l.note, l.created_by, l.created_at,
         i.repo, i.number, i.kind, i.url, i.title, i.state, i.state_reason,
         i.draft, i.merged, i.author, i.labels, i.updated_at, i.closed_at
  FROM pmp_links l
  LEFT JOIN pmp_items i ON i.ref = l.ref;

-- One unified activity stream per subject: every comment on every linked
-- item, newest first, with the item's ref and the link relation included.
CREATE VIEW IF NOT EXISTS pmp_v_subject_activity AS
  SELECT l.subject, l.ref, l.relation,
         c.id, c.kind, c.author, c.body, c.created_at, c.updated_at, c.url,
         c.path, c.line, c.review_state, c.deleted,
         i.title AS item_title, i.state AS item_state, i.kind AS item_kind
  FROM pmp_links l
  JOIN pmp_items i ON i.ref = l.ref
  JOIN pmp_comments c ON c.ref = l.ref
  ORDER BY c.created_at DESC, c.id DESC;
