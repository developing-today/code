-- PM tenant store — hosted search index (CF-7, #14947; #14107 M7).
--
-- The heavy PM surfaces must never port the laptop's unbounded reads
-- to the edge (#14060: the 33 s search, the 6.7 MB registry read).
-- Hosted /search is an indexed projection read instead: this FTS5
-- index over the item pool's titles and refs, queried with a bounded
-- LIMIT by src/read-api/search.ts (GET /api/search.json).
--
-- The index is a projection of pm_items and nothing else: it is
-- rebuildable at any time and holds no data of its own. Rebuild
-- procedure (after any bulk change made outside the triggers, or to
-- repair a suspect index):
--
--   INSERT INTO pm_items_fts(pm_items_fts) VALUES('rebuild');
--
-- The triggers below keep the index in step with the collector's
-- landing path (landSnapshot deletes a board's rows and re-inserts
-- them, so delete + insert is the common case; updates are covered
-- for the writers that upsert in place).
--
-- Scope, stated plainly: the collector emits titles and refs, not
-- issue bodies, so this index is a title/ref index. Body search
-- stays on the tunnel tier until the collector emits bodies; the
-- index gains a body column in a later migration when it does.
--
-- The schema target is D1 (owner ruling on #14866, 2026-11-26: D1
-- for all fleet state); this DDL is SQLite SQL and applies unchanged
-- to D1 and to the libSQL tenant plane the store runs on today.
--
-- Down-path (the migration runner is forward-only; recorded on
-- #14947): DROP TRIGGER pm_items_fts_au; DROP TRIGGER
-- pm_items_fts_ad; DROP TRIGGER pm_items_fts_ai; DROP TABLE
-- pm_items_fts; and delete the version-2 row from
-- pm_schema_migrations. No existing table is altered by this
-- migration and no data moves.

CREATE VIRTUAL TABLE pm_items_fts USING fts5(
  title,
  ref,
  content='pm_items',
  content_rowid='rowid'
);

CREATE TRIGGER pm_items_fts_ai AFTER INSERT ON pm_items BEGIN
  INSERT INTO pm_items_fts(rowid, title, ref) VALUES (new.rowid, new.title, new.ref);
END;

CREATE TRIGGER pm_items_fts_ad AFTER DELETE ON pm_items BEGIN
  INSERT INTO pm_items_fts(pm_items_fts, rowid, title, ref) VALUES ('delete', old.rowid, old.title, old.ref);
END;

CREATE TRIGGER pm_items_fts_au AFTER UPDATE ON pm_items BEGIN
  INSERT INTO pm_items_fts(pm_items_fts, rowid, title, ref) VALUES ('delete', old.rowid, old.title, old.ref);
  INSERT INTO pm_items_fts(rowid, title, ref) VALUES (new.rowid, new.title, new.ref);
END;

-- Populate from whatever the pool already holds on an existing store
-- (a no-op rebuild on a fresh one).
INSERT INTO pm_items_fts(pm_items_fts) VALUES ('rebuild');
