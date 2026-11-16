-- Run before explicitly activating any external steering dispatcher.
CREATE TABLE IF NOT EXISTS steering_outbox (
 id TEXT PRIMARY KEY, store_id TEXT NOT NULL, request_id TEXT NOT NULL,
 request_revision INTEGER NOT NULL, actor_id TEXT NOT NULL, request_json TEXT NOT NULL,
 destination TEXT NOT NULL, recipient TEXT NOT NULL, state TEXT NOT NULL DEFAULT 'pending'
 CHECK(state IN ('pending','dispatching','sent','uncertain','completed','revoked')),
 created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, lease_token TEXT,
 UNIQUE(store_id,request_id,request_revision)
);
CREATE TABLE IF NOT EXISTS steering_callbacks (
 callback_id TEXT PRIMARY KEY, job_id TEXT NOT NULL UNIQUE REFERENCES steering_outbox(id),
 payload_json TEXT NOT NULL, received_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS steering_outbox_pending ON steering_outbox(state,created_at);
