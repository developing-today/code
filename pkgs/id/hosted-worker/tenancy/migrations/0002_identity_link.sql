-- H2 of #9086 (issue #14260): the SSO relying-party link table.
--
-- Oracle: doc/2026-11-24T04-50-00Z_design_sso-relying-party-9086
-- section 4.3 (the table, its home and its semantics) and section
-- 4.8 (the audit rule). This migration adds the table to the CENTRAL
-- tenancy DB beside E's `account`/`binding` tables (the
-- 0001_tenancy_baseline.sql schema, E1 of #9083): it is control-plane
-- data — who may open which account through which login door —
-- never content.
--
-- The never-central invariant (E design section 2.3) holds here as
-- everywhere in this schema: a row holds a door name, the provider's
-- stable subject for that door, and timestamps. No content, no keys,
-- no token values. The provider subject is an identifier, not a
-- credential: knowing it opens nothing without the door's own login.
--
-- Two places where this migration states the design's rule rather
-- than its sketch DDL verbatim (the baseline header sets the
-- precedent for recording these):
--
--   1. The door vocabulary is closed in the sketch (the column
--      comment enumerates github | google | microsoft | devtoday),
--      so it carries a CHECK constraint, like the baseline's other
--      closed vocabularies.
--   2. Sections 4.3 and 4.8 require every link AND unlink to be an
--      audited account event (who, when, which door). The sketch's
--      row records who performed a link (linked_by) but has no
--      column for who performed an unlink, so the audit trail lives
--      in the append-only `identity_link_event` table below: one row
--      per link and per unlink, with actor and timestamp. The
--      `identity_link` row remains the current-state record; its
--      history rows are retained (never hard-deleted while the
--      account lives), per section 4.3.
--
-- The sketch's UNIQUE (door, provider_sub) is a table-level unique
-- over ALL rows, active and historical — its comment states the
-- rule: "one account per door-identity, ever". A door-identity an
-- account has unlinked is therefore NOT free for another account;
-- the same account re-linking the same door-identity reactivates
-- its retained row (see src/identity-link.ts), and the event table
-- keeps the full link/unlink/relink history.

CREATE TABLE identity_link (
  link_id       TEXT PRIMARY KEY,
  account_id    TEXT NOT NULL REFERENCES account(account_id),
  door          TEXT NOT NULL,             -- github | google | microsoft | devtoday
  provider_sub  TEXT NOT NULL,             -- stable subject at that door
  linked_at     TEXT NOT NULL,
  linked_by     TEXT NOT NULL,             -- binding_id that performed the link
  unlinked_at   TEXT,                      -- NULL while active
  UNIQUE (door, provider_sub),             -- one account per door-identity, ever
  CHECK (door IN ('github', 'google', 'microsoft', 'devtoday'))
);

CREATE INDEX identity_link_account_idx ON identity_link (account_id);

-- The audit trail for identity_link (design sections 4.3, 4.8):
-- every link and every unlink, with the actor and the timestamp.
-- Append-only: rows are never updated or deleted.
CREATE TABLE identity_link_event (
  event_id         TEXT PRIMARY KEY,
  link_id          TEXT NOT NULL REFERENCES identity_link(link_id),
  account_id       TEXT NOT NULL REFERENCES account(account_id),
  door             TEXT NOT NULL,          -- github | google | microsoft | devtoday
  action           TEXT NOT NULL,          -- linked | unlinked
  actor_binding_id TEXT NOT NULL,          -- binding_id that performed the action
  at               TEXT NOT NULL,
  CHECK (door IN ('github', 'google', 'microsoft', 'devtoday')),
  CHECK (action IN ('linked', 'unlinked'))
);

CREATE INDEX identity_link_event_account_idx ON identity_link_event (account_id, at);

UPDATE meta SET value = '2' WHERE name = 'schema_version';
