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
