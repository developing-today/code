-- Durable legacy inventory for #3399. Pre-existing accounts whose
-- jurisdiction is unknown are recorded here, never in hosted_accounts
-- and never relabelled: this table has no jurisdiction column at all,
-- so no insert or update can assign one, and the classification
-- vocabulary is closed to 'legacy-unrestricted'. The only way out is
-- the separately reviewed migration procedure. No user content here;
-- production D1 provisioning and residency evidence are separate gates.
CREATE TABLE IF NOT EXISTS legacy_hosted_accounts (
  subject_key TEXT PRIMARY KEY NOT NULL CHECK (length(trim(subject_key)) > 0),
  binding_id TEXT NOT NULL UNIQUE CHECK (length(binding_id) = 32 AND binding_id NOT GLOB '*[^0-9a-f]*'),
  classification TEXT NOT NULL CHECK (classification = 'legacy-unrestricted')
);
