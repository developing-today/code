-- PM tenant store — schema + migrations (component A slice 1, #12825).
--
-- Design: doc/2026-11-23T03-22-03Z_design_pm-service-workers-edition
-- §3 (data homes) and §5 (writer-of-record lease); shapes pinned to
-- #9060 (doc/2026-11-12T22-50-00Z_design_pm-board-hierarchy-genealogy)
-- and to the laptop collector's output types
-- (pm-stack/aggregator/src/model.ts, board-registry.ts, projects.ts).
--
-- This schema lives in ONE tenant's Turso DB (component E tenant
-- plane, #9083 §2.2). It is never applied to the central tenancy DB:
-- no PM content is central (E §2.3), and no table here holds a token,
-- key, secret, or credential value — the tenant-store test suite
-- asserts that by column name. Timestamps are TEXT (ISO-8601) and
-- JSON documents are TEXT, per the pm-stack Turso type mapping
-- (pm-stack/external-comms/migration-0002).
--
-- Applied by src/tenant-store/migrate.ts, which records each applied
-- migration in pm_schema_migrations (the schema-version record).

-- ---------------------------------------------------------------------
-- Board registry (#9060 §1: BoardDef, stored as data exactly as on
-- the laptop; the collector's landed registry is the seed shape).
-- ---------------------------------------------------------------------
CREATE TABLE pm_boards (
  board_id TEXT PRIMARY KEY CHECK (trim(board_id) <> ''),
  org TEXT NOT NULL CHECK (trim(org) <> ''),
  number INTEGER NOT NULL CHECK (number >= 1),
  node_id TEXT NOT NULL CHECK (trim(node_id) <> ''), -- PVT_…
  url TEXT NOT NULL,
  role TEXT NOT NULL CHECK (role IN ('overall', 'project', 'wip', 'register', 'ideas')),
  parent_board_id TEXT REFERENCES pm_boards (board_id),
  sibling_order INTEGER NOT NULL,
  title TEXT NOT NULL,
  recorded_at TEXT NOT NULL,
  UNIQUE (org, number)
);

-- ---------------------------------------------------------------------
-- Collector snapshots: one header row per landed per-board snapshot
-- (the collector's ProjectSnapshot/ProjectAudit pair). Pool rows and
-- the audit document hang off this header; writer + epoch are the
-- write provenance (component A design §7 invariant 2).
-- ---------------------------------------------------------------------
CREATE TABLE pm_snapshots (
  snapshot_id TEXT PRIMARY KEY CHECK (trim(snapshot_id) <> ''),
  board_id TEXT NOT NULL REFERENCES pm_boards (board_id),
  generated_at TEXT NOT NULL,
  reported_total INTEGER NOT NULL CHECK (reported_total >= 0),
  emitted_entries INTEGER NOT NULL CHECK (emitted_entries >= 0),
  writer TEXT NOT NULL CHECK (writer IN ('laptop', 'hosted')),
  epoch INTEGER NOT NULL CHECK (epoch >= 0),
  audit_json TEXT NOT NULL, -- full ProjectAudit for this board's snapshot
  recorded_at TEXT NOT NULL
);

CREATE INDEX pm_snapshots_board_idx ON pm_snapshots (board_id, generated_at);

-- ---------------------------------------------------------------------
-- The merged item pool: one row per board item per snapshot, in the
-- collector's ProjectPoolItem shape. item_json is the full emitted
-- item, byte-faithful; the typed columns are the extracted keys the
-- read API and genealogy walker query (the #9060 §15 pattern: full
-- document as JSON plus extracted key fields).
-- ---------------------------------------------------------------------
CREATE TABLE pm_items (
  board_id TEXT NOT NULL REFERENCES pm_boards (board_id),
  item_id TEXT NOT NULL, -- PVTI_… project item node id
  snapshot_id TEXT NOT NULL REFERENCES pm_snapshots (snapshot_id),
  ref TEXT NOT NULL, -- "owner/repo#123", the canonical item key
  repo TEXT NOT NULL,
  number INTEGER NOT NULL CHECK (number >= 1),
  content_kind TEXT NOT NULL CHECK (content_kind IN ('issue', 'pr')),
  state TEXT NOT NULL CHECK (state IN ('open', 'closed', 'merged')),
  draft INTEGER NOT NULL DEFAULT 0 CHECK (draft IN (0, 1)),
  title TEXT NOT NULL,
  url TEXT NOT NULL,
  project_url TEXT NOT NULL,
  issue_url TEXT NOT NULL,
  database_id INTEGER,
  author TEXT,
  labels_json TEXT NOT NULL DEFAULT '[]',
  status TEXT, -- Project "Status" field; NULL when unset on the item
  area TEXT, -- Project "Area" field
  horizon TEXT, -- Project "Horizon" field
  kind TEXT, -- Project "Kind" field (Task | Epic), not the content kind
  milestone_title TEXT,
  milestone_due_on TEXT, -- release date for release milestones (#9060 §4)
  milestone_closed_at TEXT,
  sub_issues_total INTEGER CHECK (sub_issues_total IS NULL OR sub_issues_total >= 0),
  sub_issues_completed INTEGER CHECK (sub_issues_completed IS NULL OR sub_issues_completed >= 0),
  parent_ref TEXT,
  epic_ref TEXT,
  blocked_by_json TEXT NOT NULL DEFAULT '[]',
  blocks_json TEXT NOT NULL DEFAULT '[]',
  historical_parent_refs_json TEXT NOT NULL DEFAULT '[]',
  spawn_rollup_json TEXT,
  edge_diagnostics_json TEXT NOT NULL DEFAULT '[]',
  item_updated_at TEXT,
  closed_at TEXT,
  merged_at TEXT,
  item_json TEXT NOT NULL, -- full ProjectPoolItem as emitted
  recorded_at TEXT NOT NULL,
  PRIMARY KEY (board_id, item_id)
);

CREATE INDEX pm_items_ref_idx ON pm_items (ref);
CREATE INDEX pm_items_snapshot_idx ON pm_items (snapshot_id);

-- ---------------------------------------------------------------------
-- Hierarchy / link records (#9060 §11): every link is a record with
-- relation, status, and provenance — never a bare pointer. Board →
-- board links use the same shape with kind 'board' (the registry
-- stores board links as link records, §11). A historical link must
-- carry until + reason; a removed link must carry its reason
-- (misclassification) so the removal stays auditable.
-- ---------------------------------------------------------------------
CREATE TABLE pm_links (
  link_id TEXT PRIMARY KEY CHECK (trim(link_id) <> ''),
  source_kind TEXT NOT NULL CHECK (source_kind IN ('item', 'board')),
  source_ref TEXT NOT NULL, -- item ref "owner/repo#123", or board_id
  target_kind TEXT NOT NULL CHECK (target_kind IN ('item', 'board')),
  target_ref TEXT NOT NULL, -- item ref, or board_id
  target_repo TEXT, -- #9060 §12 target.repo (item targets only)
  target_number INTEGER, -- #9060 §12 target.number (item targets only)
  relation TEXT NOT NULL CHECK (trim(relation) <> ''), -- parent | spawned-by | blocks | blocked-by | relates | board-parent | primary-parent | …
  status TEXT NOT NULL CHECK (status IN ('active', 'historical', 'removed')),
  since TEXT,
  until TEXT,
  reason TEXT, -- split | carryover | reparent-on-close | misclassification | …
  provenance_json TEXT NOT NULL DEFAULT '{}', -- writer / epoch / session provenance
  recorded_at TEXT NOT NULL,
  CHECK (status <> 'historical' OR (until IS NOT NULL AND reason IS NOT NULL)),
  CHECK (status <> 'removed' OR reason IS NOT NULL),
  CHECK (
    (target_kind = 'item' AND target_repo IS NOT NULL AND target_number IS NOT NULL)
    OR (target_kind = 'board' AND target_repo IS NULL AND target_number IS NULL)
  )
);

CREATE INDEX pm_links_source_idx ON pm_links (source_ref, relation, status);
CREATE INDEX pm_links_target_idx ON pm_links (target_ref, relation, status);

-- ---------------------------------------------------------------------
-- Metadata copies (#9060 §12): the tenant DB's copy of each item's
-- canonical Metadata block. metadata_json is the full block,
-- byte-faithful; extracted columns serve the §13/§15 query surface
-- (by direct parent, session, lane, queue).
-- ---------------------------------------------------------------------
CREATE TABLE pm_item_metadata (
  ref TEXT PRIMARY KEY, -- "owner/repo#123"
  repo TEXT NOT NULL,
  number INTEGER NOT NULL CHECK (number >= 1),
  kind TEXT NOT NULL CHECK (kind IN ('issue', 'pr')),
  pm_meta INTEGER NOT NULL CHECK (pm_meta >= 1), -- block format version
  direct_parent_ref TEXT,
  direct_parent_kind TEXT,
  lineage_design_ref TEXT,
  lineage_roadmap_item_ref TEXT,
  lineage_priority_ref TEXT,
  provenance_session_id TEXT,
  provenance_lane TEXT,
  provenance_queue_id TEXT,
  provenance_json TEXT NOT NULL, -- the §12 provenance object
  metadata_json TEXT NOT NULL, -- the full canonical block
  recorded_at TEXT NOT NULL
);

CREATE INDEX pm_item_metadata_parent_idx ON pm_item_metadata (direct_parent_ref);
CREATE INDEX pm_item_metadata_session_idx ON pm_item_metadata (provenance_session_id);

-- ---------------------------------------------------------------------
-- Roadmap rows: the "Program roadmap" row (aggregator RoadmapRow)
-- plus the genealogy overlay the hosted read API serves
-- (priority / afterRefs / releaseRef, #9060 §6).
-- ---------------------------------------------------------------------
CREATE TABLE pm_roadmap_rows (
  row_id INTEGER PRIMARY KEY, -- Grist row id (RoadmapRow.id)
  subject TEXT NOT NULL UNIQUE, -- "grist:<docId>/<tableId>/<rowId>"
  title TEXT NOT NULL,
  area TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT '',
  horizon TEXT NOT NULL DEFAULT '',
  horizon_error TEXT,
  source TEXT NOT NULL DEFAULT '',
  notes TEXT NOT NULL DEFAULT '',
  links_json TEXT NOT NULL DEFAULT '[]',
  priority_ref TEXT, -- overlay: overall-priority item ref
  after_refs_json TEXT NOT NULL DEFAULT '[]', -- overlay: sequencing refs
  release_ref TEXT, -- overlay: release item ref
  grist_url TEXT,
  file_url TEXT,
  recorded_at TEXT NOT NULL
);

-- ---------------------------------------------------------------------
-- Milestones, two kinds (#9060 §4): program milestones are rolling
-- standing homes (due_on NULL, never close); release milestones are
-- bounded containers whose due_on is the release date.
-- ---------------------------------------------------------------------
CREATE TABLE pm_milestones (
  repo TEXT NOT NULL,
  number INTEGER NOT NULL CHECK (number >= 1), -- GitHub milestone number
  title TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('program', 'release')),
  due_on TEXT,
  closed_at TEXT,
  state TEXT NOT NULL CHECK (state IN ('open', 'closed')),
  recorded_at TEXT NOT NULL,
  PRIMARY KEY (repo, number),
  CHECK (kind <> 'release' OR due_on IS NOT NULL)
);

-- ---------------------------------------------------------------------
-- Release-milestone close-out snapshots (#9060 §4): counts of issues,
-- designs, and epics covered, each split closed / carried-over, plus
-- the release date — the accounting /api/releases.json serves.
-- ---------------------------------------------------------------------
CREATE TABLE pm_release_snapshots (
  repo TEXT NOT NULL,
  milestone_number INTEGER NOT NULL,
  release_date TEXT NOT NULL,
  issues_covered INTEGER NOT NULL CHECK (issues_covered >= 0),
  issues_closed INTEGER NOT NULL CHECK (issues_closed >= 0),
  issues_carried_over INTEGER NOT NULL CHECK (issues_carried_over >= 0),
  designs_covered INTEGER NOT NULL CHECK (designs_covered >= 0),
  designs_closed INTEGER NOT NULL CHECK (designs_closed >= 0),
  designs_carried_over INTEGER NOT NULL CHECK (designs_carried_over >= 0),
  epics_covered INTEGER NOT NULL CHECK (epics_covered >= 0),
  epics_closed INTEGER NOT NULL CHECK (epics_closed >= 0),
  epics_carried_over INTEGER NOT NULL CHECK (epics_carried_over >= 0),
  carried_over_epics_json TEXT NOT NULL DEFAULT '[]', -- refs of epics carried over (§4 carryover)
  computed_at TEXT NOT NULL,
  snapshot_json TEXT NOT NULL, -- full accounting document
  PRIMARY KEY (repo, milestone_number),
  FOREIGN KEY (repo, milestone_number) REFERENCES pm_milestones (repo, number),
  CHECK (issues_closed <= issues_covered AND issues_carried_over <= issues_covered),
  CHECK (designs_closed <= designs_covered AND designs_carried_over <= designs_covered),
  CHECK (epics_closed <= epics_covered AND epics_carried_over <= epics_covered)
);

-- ---------------------------------------------------------------------
-- Reconcile findings: the drift / invariant report rows both
-- editions produce (primary-parent divergence, non-record-writer
-- writes, lease violations, Grist drift, …). Findings are the
-- enforcement surface where GitHub cannot hard-block (#9060 §7).
-- ---------------------------------------------------------------------
CREATE TABLE pm_reconcile_findings (
  finding_id TEXT PRIMARY KEY CHECK (trim(finding_id) <> ''),
  board_id TEXT REFERENCES pm_boards (board_id),
  kind TEXT NOT NULL CHECK (trim(kind) <> ''),
  severity TEXT NOT NULL DEFAULT 'warning' CHECK (severity IN ('info', 'warning', 'error')),
  subject_ref TEXT, -- item ref or board id the finding is about
  summary TEXT NOT NULL,
  detail_json TEXT NOT NULL DEFAULT '{}',
  writer TEXT CHECK (writer IS NULL OR writer IN ('laptop', 'hosted')),
  epoch INTEGER CHECK (epoch IS NULL OR epoch >= 0),
  recorded_at TEXT NOT NULL,
  resolved_at TEXT
);

CREATE INDEX pm_reconcile_findings_board_idx ON pm_reconcile_findings (board_id, recorded_at);

-- ---------------------------------------------------------------------
-- Writer-of-record lease (component A design §5): one row per board
-- — {board_id, writer, epoch, heartbeat_at}. The epoch is a fencing
-- counter: it increments on every flip, and every reconcile run
-- reads this row before its first mutation and aborts if it is not
-- the recorded writer at the current epoch.
-- ---------------------------------------------------------------------
CREATE TABLE pm_writer_leases (
  board_id TEXT PRIMARY KEY REFERENCES pm_boards (board_id),
  writer TEXT NOT NULL CHECK (writer IN ('laptop', 'hosted')),
  epoch INTEGER NOT NULL CHECK (epoch >= 1),
  heartbeat_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
