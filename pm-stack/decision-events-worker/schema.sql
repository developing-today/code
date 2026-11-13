-- Decision-set events in D1 (#9335 Track B). Identical shape to the
-- Turso table the aggregator sync writes (decision_set_events in
-- fleet-platform; see pm-stack/aggregator/src/decision-events.ts) —
-- keep the two DDLs in step.
CREATE TABLE IF NOT EXISTS decision_set_events (
  event_id TEXT PRIMARY KEY,
  set_id TEXT NOT NULL,
  question_ref TEXT,
  type TEXT NOT NULL,
  payload TEXT NOT NULL,
  ts TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS decision_set_events_set_ts ON decision_set_events (set_id, ts);
CREATE INDEX IF NOT EXISTS decision_set_events_type_ts ON decision_set_events (type, ts);
