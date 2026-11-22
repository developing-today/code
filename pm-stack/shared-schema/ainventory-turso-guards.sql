-- Apply after recovery creates the ainventory_* tables. Never weakens access
-- controls or rewrites recovered records. SQLite counterpart of PG event guards.
CREATE TRIGGER IF NOT EXISTS ainventory_findings_no_update BEFORE UPDATE ON ainventory_findings BEGIN SELECT RAISE(ABORT,'findings is append-only'); END;
CREATE TRIGGER IF NOT EXISTS ainventory_findings_no_delete BEFORE DELETE ON ainventory_findings BEGIN SELECT RAISE(ABORT,'findings is append-only'); END;
CREATE TRIGGER IF NOT EXISTS ainventory_interpretation_runs_no_update BEFORE UPDATE ON ainventory_interpretation_runs BEGIN SELECT RAISE(ABORT,'interpretation_runs is append-only'); END;
CREATE TRIGGER IF NOT EXISTS ainventory_interpretation_runs_no_delete BEFORE DELETE ON ainventory_interpretation_runs BEGIN SELECT RAISE(ABORT,'interpretation_runs is append-only'); END;
CREATE TRIGGER IF NOT EXISTS ainventory_notes_no_update BEFORE UPDATE ON ainventory_notes BEGIN SELECT RAISE(ABORT,'notes is append-only'); END;
CREATE TRIGGER IF NOT EXISTS ainventory_notes_no_delete BEFORE DELETE ON ainventory_notes BEGIN SELECT RAISE(ABORT,'notes is append-only'); END;
CREATE TRIGGER IF NOT EXISTS ainventory_measurement_events_no_update BEFORE UPDATE ON ainventory_measurement_events BEGIN SELECT RAISE(ABORT,'measurement_events is append-only'); END;
CREATE TRIGGER IF NOT EXISTS ainventory_measurement_events_no_delete BEFORE DELETE ON ainventory_measurement_events BEGIN SELECT RAISE(ABORT,'measurement_events is append-only'); END;
CREATE TRIGGER IF NOT EXISTS ainventory_price_events_no_update BEFORE UPDATE ON ainventory_price_events BEGIN SELECT RAISE(ABORT,'price_events is append-only'); END;
CREATE TRIGGER IF NOT EXISTS ainventory_price_events_no_delete BEFORE DELETE ON ainventory_price_events BEGIN SELECT RAISE(ABORT,'price_events is append-only'); END;
CREATE TRIGGER IF NOT EXISTS ainventory_provider_policies_no_update BEFORE UPDATE ON ainventory_provider_policies BEGIN SELECT RAISE(ABORT,'provider_policies is append-only'); END;
CREATE TRIGGER IF NOT EXISTS ainventory_provider_policies_no_delete BEFORE DELETE ON ainventory_provider_policies BEGIN SELECT RAISE(ABORT,'provider_policies is append-only'); END;

-- Explicit identity collision guards also reject INSERT OR REPLACE when
-- recursive_triggers is off. These source tables have no other UNIQUE keys.
CREATE TRIGGER IF NOT EXISTS ainventory_findings_no_replace BEFORE INSERT ON ainventory_findings WHEN EXISTS (SELECT 1 FROM ainventory_findings WHERE id=NEW.id) BEGIN SELECT RAISE(ABORT,'findings identity already exists'); END;
CREATE TRIGGER IF NOT EXISTS ainventory_interpretation_runs_no_replace BEFORE INSERT ON ainventory_interpretation_runs WHEN EXISTS (SELECT 1 FROM ainventory_interpretation_runs WHERE id=NEW.id) BEGIN SELECT RAISE(ABORT,'interpretation_runs identity already exists'); END;
CREATE TRIGGER IF NOT EXISTS ainventory_notes_no_replace BEFORE INSERT ON ainventory_notes WHEN EXISTS (SELECT 1 FROM ainventory_notes WHERE id=NEW.id) BEGIN SELECT RAISE(ABORT,'notes identity already exists'); END;
CREATE TRIGGER IF NOT EXISTS ainventory_measurement_events_no_replace BEFORE INSERT ON ainventory_measurement_events WHEN EXISTS (SELECT 1 FROM ainventory_measurement_events WHERE id=NEW.id) BEGIN SELECT RAISE(ABORT,'measurement_events identity already exists'); END;
CREATE TRIGGER IF NOT EXISTS ainventory_price_events_no_replace BEFORE INSERT ON ainventory_price_events WHEN EXISTS (SELECT 1 FROM ainventory_price_events WHERE id=NEW.id) BEGIN SELECT RAISE(ABORT,'price_events identity already exists'); END;
CREATE TRIGGER IF NOT EXISTS ainventory_provider_policies_no_replace BEFORE INSERT ON ainventory_provider_policies WHEN EXISTS (SELECT 1 FROM ainventory_provider_policies WHERE id=NEW.id) BEGIN SELECT RAISE(ABORT,'provider_policies identity already exists'); END;
