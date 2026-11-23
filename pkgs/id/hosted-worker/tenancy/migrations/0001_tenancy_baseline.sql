-- 0001_tenancy_baseline.sql
--
-- Central tenancy DB baseline (component E1 of #9083, issue #12796).
--
-- Implements section 2.1 of the hosted-tenancy design record
-- (doc/2026-11-17T16-55-00Z_design_hosted-tenancy-architecture) as the
-- first migration of the central tenancy database: one Turso/SQLite
-- database holding the account registry, the binding pointers, the
-- tenant/database registry, role grants, peer policy, publish records
-- and credential metadata.
--
-- THE INVARIANT (design section 2.3, "never central"): this schema
-- holds NO user content, NO keys and NO token values. Tokens are
-- metadata only (the `credential` table records a `jti` revocation
-- handle, never the token). A tenancy-DB dump must not yield content
-- or database access. test/tenancy-schema.test.ts fails if a
-- content/key/token column ever appears in this schema.
--
-- Peer principle (design sections 0-1, 9): hosted, Rust-binary and
-- self-hosted nodes are the same participant kinds. `tenant_db`
-- carries one `participant_kind` vocabulary for all of them; there
-- are no hosted-only tables and nothing in this schema special-cases
-- a hosted tenant.
--
-- Two places where this migration states the design's rule rather
-- than its sketch DDL verbatim (the record calls the DDL "a sketch
-- for the implementation issues, not a migration"):
--
--   1. `binding`: the sketch's UNIQUE (account_id, status) would cap
--      an account at one superseded binding ever, which breaks the
--      rebind chain the same sketch defines via `superseded_by`. The
--      sketch's stated rule is "at most one active row per account",
--      so that rule is enforced with a partial unique index over
--      active rows only.
--   2. Closed vocabularies the record names as closed get CHECK
--      constraints: `tenant_db.participant_kind`,
--      `tenant_db.jurisdiction` ("closed vocabulary" in the sketch),
--      `peer_policy.class` (the #9106/B31 classes) and
--      `credential.scope` ("(closed)" in the sketch).
--
-- `meta` mirrors the bootstrap pattern of the tenant data plane
-- (src/user-do.ts): a name/value table carrying `schema_version`.
-- (Its columns are name/value, not key/value, on purpose: the
-- never-central guard bans any column with "key" in its name, with
-- no exceptions to argue about.)
-- Later E slices add numbered migrations; this file never changes.

CREATE TABLE meta (
  name  TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

INSERT INTO meta (name, value) VALUES ('schema_version', '1');

-- One row per identity that can own things. Aligned with #2258 s.1.1.
-- sub is the identity provider's stable subject; email is contact-only.
CREATE TABLE account (
  account_id      TEXT PRIMARY KEY,          -- opaque, immutable, service-minted
  sub             TEXT NOT NULL UNIQUE,      -- #9077 provider subject; never an email
  email_contact   TEXT,                      -- notifications only; never a key
  created_at      TEXT NOT NULL,
  state           TEXT NOT NULL,             -- active | suspended | closed
  tier            TEXT NOT NULL,             -- #2046 parameter
  meter_rollup    TEXT,                      -- latest daily rollup (content-free)
  CHECK (state IN ('active','suspended','closed'))
);

-- The stable pointer from account to identity and to the tenant. #2258 s.1.2
-- is the authority on the ceremony; this table stores the pointer + status,
-- not the ceremony transcript (that stays in the binding record store).
CREATE TABLE binding (
  binding_id      TEXT PRIMARY KEY,          -- 32 lowercase hex; names the DO and the tenant
  account_id      TEXT NOT NULL REFERENCES account(account_id),
  iroh_id         TEXT NOT NULL,             -- current root key; public key only
  status          TEXT NOT NULL,             -- active | rebind_pending | superseded
  bound_at        TEXT NOT NULL,
  superseded_by   TEXT REFERENCES binding(binding_id),
  CHECK (status IN ('active','rebind_pending','superseded'))
);

-- At most one active binding per account (design note 1 above).
CREATE UNIQUE INDEX binding_one_active_per_account
  ON binding (account_id) WHERE status = 'active';

-- The registry. One row per data plane a tenant owns (private tenant DB,
-- later maybe a second DB); the public DB is a tenant too, see s.7.
-- participant_kind is the peer principle in one column: a hosted user,
-- a native (Rust-binary) user, a self-hosted node and the public plane
-- are the same kind of participant, differing only in backend.
CREATE TABLE tenant_db (
  tenant_id       TEXT PRIMARY KEY,          -- = binding_id for user tenants
  participant_kind TEXT NOT NULL,            -- user_hosted | user_native | self_hosted_node | public
  turso_db_id     TEXT,                      -- provider handle; NULL until provisioned
  turso_db_url    TEXT,                      -- https://<db>.turso.io; public endpoint only
  jurisdiction    TEXT NOT NULL,             -- us | eu | fedramp (closed vocabulary)
  backend         TEXT NOT NULL,             -- turso | redb | k8s_tiered (capability, s.9)
  state           TEXT NOT NULL,             -- see s.5.1 of the design (provisioning lifecycle)
  schema_version  INTEGER NOT NULL,
  created_at      TEXT NOT NULL,
  deleted_at      TEXT,
  CHECK (participant_kind IN ('user_hosted','user_native','self_hosted_node','public')),
  CHECK (jurisdiction IN ('us','eu','fedramp')),
  CHECK (state IN ('requested','db_creating','db_ready','active',
                   'suspended','export_only','deleting','deleted','failed'))
);

-- Roles are #1253's vocabulary, not a new one. Scope is a tenant or '*'.
CREATE TABLE role_grant (
  binding_id      TEXT NOT NULL REFERENCES binding(binding_id),
  role            TEXT NOT NULL,             -- #1253 role name
  scope           TEXT NOT NULL,             -- tenant_id or '*'
  granted_at      TEXT NOT NULL,
  granted_by      TEXT NOT NULL,
  revoked_at      TEXT,
  PRIMARY KEY (binding_id, role, scope)
);

-- Peer configuration in the #9106/B31 shape, not a parallel trust model (s.9).
CREATE TABLE peer_policy (
  peer_id         TEXT NOT NULL,
  tenant_id       TEXT NOT NULL,             -- whose tenancy this governs
  class           TEXT NOT NULL,             -- fully_trusted | send_only | accept_signed_from | unknown
  operations      TEXT NOT NULL,             -- JSON array: accept_data | send_data | ...
  relay_granted   INTEGER NOT NULL DEFAULT 0,
  scope           TEXT NOT NULL,
  policy_version  INTEGER NOT NULL,
  valid_from      TEXT,
  expires_at      TEXT,
  PRIMARY KEY (peer_id, tenant_id, policy_version),
  CHECK (class IN ('fully_trusted','send_only','accept_signed_from','unknown'))
);

-- Publish records: what a tenant has offered to the public plane (s.7).
CREATE TABLE publish_record (
  publish_id      TEXT PRIMARY KEY,
  tenant_id       TEXT NOT NULL,
  content_hash    TEXT NOT NULL,             -- content address; the row is the only grant
  signed_artifact TEXT NOT NULL,             -- the signed bytes live in the public DB; this is the digest + envelope id
  published_at    TEXT NOT NULL,
  retracted_at    TEXT,
  policy_version  INTEGER NOT NULL
);

-- Credential metadata only. Tokens are NEVER stored here (s.8).
CREATE TABLE credential (
  credential_id   TEXT PRIMARY KEY,
  tenant_id       TEXT NOT NULL,
  audience        TEXT NOT NULL,             -- turso_db_id
  scope           TEXT NOT NULL,             -- read | readwrite | ddl (closed)
  issued_at       TEXT NOT NULL,
  expires_at      TEXT NOT NULL,
  revoked_at      TEXT,
  issuer          TEXT NOT NULL,             -- which Worker/DO issued it
  jti             TEXT NOT NULL UNIQUE,      -- revocation handle; not the token
  CHECK (scope IN ('read','readwrite','ddl'))
);
