-- Per-account migration lease (#3404). One coordinator at a time may
-- drive an account's migration; the token rises whenever the lease
-- changes hands, and every directory step names the token it holds,
-- so a coordinator that lost its lease cannot move the record.
CREATE TABLE IF NOT EXISTS migration_leases (
  subject_key TEXT PRIMARY KEY NOT NULL CHECK (length(trim(subject_key)) > 0),
  owner TEXT NOT NULL CHECK (length(trim(owner)) > 0),
  token INTEGER NOT NULL CHECK (token >= 1),
  expires_ms INTEGER NOT NULL CHECK (expires_ms >= 0)
);
