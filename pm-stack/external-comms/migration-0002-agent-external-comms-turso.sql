-- External-comms send-time ledger — Turso home (repointed 2026-11-07).
-- Rule: memory/topics/external-comms-rule-2026-11-05.md; design:
-- pm-stack/external-comms/README.md in developing-today/code.
--
-- The ledger moved from `agent.external_comms` in Supabase muse-dev2
-- (migration-0001) to the Turso database fleet-platform as
-- `agent_external_comms`, because muse-dev2's data plane has been
-- down since 2026-11-06 (code#6619) and the migration program
-- (code#5690) classifies this stream as Turso-bound. Type mapping
-- follows the program: timestamptz -> TEXT (ISO-8601), jsonb -> TEXT
-- (JSON-encoded). The 58 rows recorded in Postgres before the
-- repoint stay there until the finalize pass tops this table up
-- by id (INSERT OR IGNORE makes that safe).
--
-- Executed against fleet-platform on 2026-11-07 via the turso CLI;
-- kept here as the DDL of record.

CREATE TABLE IF NOT EXISTS agent_external_comms (
  id TEXT PRIMARY KEY,              -- stable record id; pusher retries dedupe on it
  sent_at TEXT NOT NULL,            -- when the send happened (ISO-8601)
  recorded_at TEXT NOT NULL,        -- when the ledger row was written (set by the writer)
  identity TEXT NOT NULL,           -- 'drew' | 'fleet:<name>' (e.g. fleet:openclaw)
  channel TEXT NOT NULL,            -- github | gmail | slack | facebook | ...
  destination TEXT NOT NULL,        -- repo / addressee / channel the send went to
  item_ref TEXT,                    -- URL of the item, when one exists
  kind TEXT NOT NULL,               -- issue | pr | comment | review | email | post | dm
  approval_basis TEXT NOT NULL,     -- per-item approval | surfaced-with-link | allowlist:<name> | honest backfill labels
  approver TEXT,                    -- who approved, when approval applies
  approval_evidence TEXT,           -- where the approval is recorded (chat/memory/issue ref)
  summary TEXT NOT NULL,            -- one line: what was sent
  payload TEXT NOT NULL DEFAULT '{}',  -- channel detail as JSON (slack ts, case no, provenance)
  source TEXT NOT NULL DEFAULT 'record-external-send'  -- writer: CLI | backfill-2026-11-05-audit
);

CREATE INDEX IF NOT EXISTS agent_external_comms_sent_at_idx
  ON agent_external_comms (sent_at);
CREATE INDEX IF NOT EXISTS agent_external_comms_channel_idx
  ON agent_external_comms (channel, sent_at);
