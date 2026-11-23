-- Shared jurisdiction-aware directory metadata only. Do not put user content
-- here; production D1 provisioning and residency evidence are separate gates.
CREATE TABLE IF NOT EXISTS hosted_accounts (
  subject_key TEXT PRIMARY KEY NOT NULL CHECK (length(trim(subject_key)) > 0),
  binding_id TEXT NOT NULL UNIQUE CHECK (length(binding_id) = 32),
  jurisdiction TEXT NOT NULL CHECK (jurisdiction IN ('us', 'eu', 'fedramp')),
  jurisdiction_pending TEXT CHECK (jurisdiction_pending IS NULL OR jurisdiction_pending IN ('us', 'eu', 'fedramp')),
  migration_id TEXT,
  phase TEXT NOT NULL,
  routing_epoch INTEGER NOT NULL CHECK (routing_epoch >= 0)
);
