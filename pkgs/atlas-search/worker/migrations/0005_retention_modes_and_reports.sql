-- Retention modes and dry-run reports for retained Atlas objects (#9140
-- build slice 6, wiring to the retention rule engine design #6041).
-- Applied after 0004_retained_objects_retirement.sql.
--
-- Every class starts in dry-run: its versions are reported, never
-- retired. A class reaches live-cold (retired versions are marked cold;
-- nothing is deleted) only after it appears unchanged in two consecutive
-- scheduled reports (rule engine §5). Changing a class's window or floor
-- returns it to dry-run. There is no delete mode here.
ALTER TABLE atlas_retention_classes ADD COLUMN mode TEXT NOT NULL DEFAULT 'dry-run' CHECK (mode IN ('dry-run', 'live-cold'));
ALTER TABLE atlas_retention_classes ADD COLUMN promoted_from TEXT;

-- One row per report run, kept so the next report can show deltas and a
-- promotion can name the reports that earned it.
CREATE TABLE atlas_retention_reports (
  run_id     TEXT    PRIMARY KEY CHECK (length(run_id) > 0),
  kind       TEXT    NOT NULL CHECK (kind IN ('scheduled', 'on-demand')),
  at_ms      INTEGER NOT NULL,
  report     TEXT    NOT NULL
);
CREATE INDEX atlas_retention_reports_kind ON atlas_retention_reports (kind, at_ms);
