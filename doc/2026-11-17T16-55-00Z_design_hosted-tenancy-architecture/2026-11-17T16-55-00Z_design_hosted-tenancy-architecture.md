# Design (component E): hosted tenancy architecture — per-user Turso databases + a central tenancy DB

Status: design record for issue #9083, component **E** of epic #9078 (hosted
editions program). Docs-only; no implementation in this record.
Author: fleet lane `code-j`, task 20279, 2026-11-17.
Written against `origin/main` at `2a7cbd15d9`.

Basis, and the inputs this record must answer rather than restate:

- Drew's sketch, 2026-11-12 (issue #9083 §What): per-user ID databases in
  Turso; a central tenancy database as the control plane; Workers and
  Durable Objects serving routing, auth, sync coordination and provisioning;
  hosted per-user instances sync with their local/native instances in both
  directions; one large public database everyone can connect to.
- **The peer principle** (Drew amendment, 2026-11-12 17:03, restated on the
  issue): hosted-CF nodes, Rust-binary nodes and clustered self-hosted nodes
  (component I, #9087) are the _same participant kinds_ with different
  backends. Our own server is a peer, not a special case. No protocol
  special-casing.
- **The #9106 federation input** (Drew, 2026-11-12 18:29, restated on the
  issue): peer classes (fully trusted / send-only / accept-signed-from),
  signed-data-only acceptance, federated query fan-out and reputation are
  designed in Part B of #9106 (#9141–#9148, under #9108). The tenancy
  peer/account model **expects those classes as configuration** and must not
  invent a parallel trust model.
- Settled neighbours this record aligns with and does not override:
  #2258 account/binding records
  (`doc/2026-10-30T22-45-00Z_design_hosted_provisioning_sso_binding.md`);
  the #9106 Part B wire/data model
  (`doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-b/versioned-design.md`)
  and its landed trust-class slice B31
  (`pkgs/id/crates/id-sync/src/federation/slices/b31_peer_classes.rs`);
  the landed hosted data plane scaffold in `pkgs/id/hosted-worker/` (#5199);
  the ephemeral-currency boundary PoC (noted in that package's README, do not
  treat as settled tenancy).

## 0. Shape in one paragraph

Three planes, deliberately separated. A **central tenancy database** (one
Turso database, not per-user) holds accounts, the database registry, binding
pointers, roles, peer configuration, publish records and credential metadata —
it holds **no user content and no private keys**. Each user's data lives in
**that user's own Turso database**, reached only through a scoped, expiring
Turso token minted by a tenancy Worker from a tenancy row and an authenticated
session. A **public database** is a fourth, shared-tenant plane: a normal
per-tenant database whose tenant is a publish service, made readable to all.
Durable Objects are per-tenant coordination, not per-request routing: one
`TenantDO` per tenant, keyed by the stable `binding_id` (never email, never a
provider subject), plus one `SyncSessionDO`-shaped coordinator per
(tenant, peer) sync session, plus one singleton `PublishIndexDO`. Provisioning
is a state machine on the tenancy row driven by Workers, with Turso database
creation and token issuance as its two irreversible external effects. Sync is
**not** Turso replication as the protocol: it is the existing signed-event
`LogSync`/range-sync path (#179, `pkgs/id/crates/id-sync`) carrying the same
envelopes the native nodes already exchange, with the hosted tenant as an
ordinary peer. #3811's Worker/DO data plane is **built on as the tenant data
plane, not superseded** — this record adds the control plane and the tenancy
naming above it.

## 1. The three planes, and why the split is the design

| Plane                                                      | Holds                                                                                                                                                     | Never holds                                                   | Owner                                       |
| ---------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------- | ------------------------------------------- |
| **Tenancy (central, one Turso DB)**                        | accounts, bindings pointers, tenant/database registry, tenant state machine, roles, peer policies, publish records, credential metadata, metering rollups | user content, content keys, private keys, raw Turso tokens    | tenancy Workers + `TenancyOpsDO`            |
| **Tenant data (one Turso DB per tenant)**                  | that tenant's event log, documents, notes, references, metadata, blob _references_                                                                        | other tenants' rows, tenancy authority, plaintext credentials | the tenant's own token holders + `TenantDO` |
| **Public (one shared Turso DB, tenant = publish service)** | published, signed, content-addressed content and its index                                                                                                | private content, per-user data, secrets                       | `PublishIndexDO` + the publish service      |

The split is the design because it makes three failure modes structural rather
than procedural:

1. **A tenancy-DB compromise is not a content compromise.** The central DB
   cannot be exfiltrated into content, because no content is there; and it
   cannot mint a readable token without the tenant's account being in a state
   that permits one (§5.3, §8).
2. **A per-tenant database compromise is one tenant** (plus whatever that
   tenant published). No cross-tenant SQL path exists: tokens are minted per
   database and per tenant, and the tenancy DB never joins into tenant DBs.
3. **Jurisdiction stays assignable.** A tenant DB is created in a
   jurisdiction; a tenancy row records which. #2283/#3401 requirements land on
   the tenant plane, and the tenancy row is the only place that decision is
   recorded — the same shape the existing
   `env.USERS.jurisdiction(<j>)` restricted-namespace rule already enforces for
   DOs, with no unrestricted fallback (fail closed).

**Peer principle applied at the plane level.** The tenancy DB is _our_
control plane, not a privileged protocol participant. Its rows describe a
hosted tenant exactly the way they describe a Rust-binary node or a clustered
self-hosted node: `participant_kind` + backend descriptors + a peer policy
(§9). Nothing in the sync or publish protocol may branch on "hosted" as a
special case; the branch is on backend capability declared in the row.

## 2. Q1 — Tenancy DB schema: what is central, what is per-user, what is never central

### 2.1 Central (schema sketch)

SQLite/Turso DDL sketch. `--` comments state the rule; the DDL is a sketch for
the implementation issues, not a migration.

```sql
-- One row per identity that can own things. Aligned with #2258 §1.1.
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

-- The stable pointer from account to identity and to the tenant. #2258 §1.2
-- is the authority on the ceremony; this table stores the pointer + status,
-- not the ceremony transcript (that stays in the binding record store).
CREATE TABLE binding (
  binding_id      TEXT PRIMARY KEY,          -- 32 lowercase hex; names the DO and the tenant
  account_id      TEXT NOT NULL REFERENCES account(account_id),
  iroh_id         TEXT NOT NULL,             -- current root key; public key only
  status          TEXT NOT NULL,             -- active | rebind_pending | superseded
  bound_at        TEXT NOT NULL,
  superseded_by   TEXT REFERENCES binding(binding_id),
  CHECK (status IN ('active','rebind_pending','superseded')),
  UNIQUE (account_id, status)                -- at most one active row per account
);

-- The registry. One row per data plane a tenant owns (private tenant DB,
-- later maybe a second DB); the public DB is a tenant too, see §7.
CREATE TABLE tenant_db (
  tenant_id       TEXT PRIMARY KEY,          -- = binding_id for user tenants
  participant_kind TEXT NOT NULL,            -- user_hosted | user_native | self_hosted_node | public
  turso_db_id     TEXT,                      -- provider handle; NULL until provisioned
  turso_db_url    TEXT,                      -- https://<db>.turso.io; public endpoint only
  jurisdiction    TEXT NOT NULL,             -- us | eu | fedramp (closed vocabulary)
  backend         TEXT NOT NULL,             -- turso | redb | k8s_tiered (capability, §9)
  state           TEXT NOT NULL,             -- see §5.1
  schema_version  INTEGER NOT NULL,
  created_at      TEXT NOT NULL,
  deleted_at      TEXT,
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

-- Peer configuration in the #9106/B31 shape, not a parallel trust model (§9).
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
  PRIMARY KEY (peer_id, tenant_id, policy_version)
);

-- Publish records: what a tenant has offered to the public plane (§7).
CREATE TABLE publish_record (
  publish_id      TEXT PRIMARY KEY,
  tenant_id       TEXT NOT NULL,
  content_hash    TEXT NOT NULL,             -- content address; the row is the only grant
  kind            TEXT NOT NULL,             -- document | note | tag | artifact
  signed_artifact TEXT NOT NULL,             -- the signed bytes live in the public DB; this is the digest + envelope id
  published_at    TEXT NOT NULL,
  retracted_at    TEXT,
  policy_version  INTEGER NOT NULL
);

-- Credential metadata only. Tokens are NEVER stored here (§8).
CREATE TABLE credential (
  credential_id   TEXT PRIMARY KEY,
  tenant_id       TEXT NOT NULL,
  audience        TEXT NOT NULL,             -- turso_db_id
  scope           TEXT NOT NULL,             -- read | readwrite | ddl (closed)
  issued_at       TEXT NOT NULL,
  expires_at      TEXT NOT NULL,
  revoked_at      TEXT,
  issuer          TEXT NOT NULL,             -- which Worker/DO issued it
  jti             TEXT NOT NULL UNIQUE       -- revocation handle; not the token
);
```

Deliberately absent from the central schema: any column holding a document,
a note body, a blob, a key, or a token value. The tenancy DB can answer "who
is this, what do they own, what may they do, what did they publish, and is
their credential still valid" and nothing else.

### 2.2 Per-user (lives only in the tenant DB)

The tenant's own event log and content layer, in the shape the ID store
already uses: events/chain, documents, notes, tags, media **references**
(content-addressed hashes; bytes in R2, access iff a reference row exists in
this tenant — the rule the landed `blobs` table in
`pkgs/id/hosted-worker/src/user-do.ts` already implements and which component F
owns program-wide), grants, and a `tenant_meta` table carrying
`schema_version` and the tenant's own peer-policy _cursors_. The tenant DB also
holds the tenant's own copy of the pointers it needs (its binding id, its
account id) so that an export is self-describing — but never the authority for
them.

### 2.3 Never central

Named so a later PR cannot drift into it:

- **Content.** No document bodies, note text, message text, or blob bytes in
  the tenancy DB, in any form, including "temporarily" for a search index.
- **Keys.** No Iroh root keys, no device keys, no per-tenant content keys. The
  tenancy DB holds `iroh_id` (a public key) and nothing more.
- **Tokens.** No Turso token values, only credential metadata with a `jti`.
  A tenancy-DB dump must not yield database access.
- **Tenant-readable row counts.** Metering rollups are aggregates the tenant
  already knows (bytes and request counts); no central table counts a tenant's
  objects for anyone else's benefit.
- **A global content index.** Publish records are per-`content_hash` grants
  with the publishing tenant, not a global search index; the public DB's own
  index is the discovery path (§7).

Why these four and not more: they are exactly the fields whose leakage turns
a control-plane breach into a content-or-access breach. Anything else can be
added to the central schema by a normal PR with a stated reason.

## 3. Q2 — Durable Object inventory

Four DO classes, three of them per-scope, one singleton. The existing
`UserDO` (#5199) is the per-tenant _data_ object and keeps that job; this
record adds coordination objects above it.

| DO                                                   | Cardinality                                                    | Key               | Responsibility                                                                                                                                              | Explicitly not                                                           |
| ---------------------------------------------------- | -------------------------------------------------------------- | ----------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| `TenantDO` (existing `UserDO`, renamed in docs only) | one per tenant                                                 | `binding_id`      | versioned schema bootstrap, tenant data + blob references, ceiling/usage, export, fail-closed identity guard                                                | tenancy authority, token minting, cross-tenant anything                  |
| `TenantOpsDO`                                        | one per tenant, created lazily                                 | `binding_id`      | the provisioning state machine (§5): serialised transitions, idempotent external effects, retry/compensation                                                | serving reads/writes of tenant data                                      |
| `SyncCoord`                                          | one per (tenant, peer) pair, ephemeral-ish, hibernating        | `sha256(tenant_id |                                                                                                                                                             | peer_id)`                                                                | owns a sync session's ordering and presence: who is in a session, the last agreed watermark, conflict report delivery, rate accounting for the session | storing events (they go to the tenant DB), deciding acceptance (§9's gate does) |
| `PublishIndex`                                       | one singleton (plus one per public DB if we ever split public) | fixed name        | the public DB's ingest serialisation: accepts a signed publish, validates, writes the row, updates the public index, records the `publish_record` back-link | moderation policy content (that is F + #6007 tiers), tenant-private data |

Rules that make the inventory defensible:

1. **Routing is a pure function; state is per-tenant.** The front door computes
   `(binding_id, jurisdiction)` and the tenant object from it, exactly as
   `src/routing.ts` does today (`parseBindingId` → `userStub`), with only the
   raw `USERS` namespace touched there and only via
   `env.USERS.jurisdiction(j)`. No new global mutable state on the hot path.
2. **One DO per user, not per database.** A user who later holds two databases
   gets two tenant rows and one `TenantDO` keyed by `binding_id`, holding both
   database handles. Per-database DOs would multiply hibernation state for no
   isolation gain (Turso already isolates databases) and would break the
   stable-name rule (the DO must never move on a rebind or a new database).
3. **Per-session coordination is a DO because sync needs a serialisation
   point**, not because it needs durability forever: `SyncCoord` may drop its
   state when idle, since the authoritative watermark is in the tenant DB and
   is re-derived on the next session.
4. **Metering stays where #3811 put it.** `TenantDO` derives usage from its own
   tables on read (`/v1/usage` today) and rolls up to the tenancy row nightly.
   `TenantOpsDO` does not meter; `SyncCoord` counts session bytes only to
   enforce its own rate limit and hands the number to `TenantDO`.
5. **No always-on process.** Every object hibernates; nothing here needs a
   container, consistent with #3811's gate.
6. **Jurisdiction is a property of the object's name**, not a parameter:
   `jurisdiction(j)` restricted namespaces only, fail closed, as today.

Anti-goal recorded: a single "tenancy DO" holding all tenants would be a
global hot partition and a single-object blast radius; the central tenancy
_data_ is a Turso database queried by stateless Workers, and
`TenantOpsDO`/`SyncCoord`/`PublishIndex` are the only strongly-serialised
scopes.

## 4. Q3 — Provisioning lifecycle

One state machine, on `tenant_db.state` (§2.1), driven by `TenantOpsDO`, with
two irreversible external effects (Turso DB creation, token issuance) each made
idempotent by a recorded intent.

```
requested ──► db_creating ──► db_ready ──► active
    │              │               │          │
    │              └──► failed ◄───┘          ├──► suspended ──► active
    └──► failed (validation)                  ├──► export_only ──► deleting
                                              └──► deleting ──► deleted
```

| Step                   | Who may do it                                                                         | What happens                                                                                                                                                                               | Irreversible effect                                                                |
| ---------------------- | ------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------- |
| 1. signup              | the identity layer (#9077, component H)                                               | `account` row created on first sign-in; **nothing else** — an account with no binding owns nothing (#2258 §1.1)                                                                            | none                                                                               |
| 2. binding ceremony    | the user (proof of possession) + the service (attestation)                            | `binding` row minted, `binding_id` fixed (§2258 §1.2); no database yet                                                                                                                     | none                                                                               |
| 3. tenant request      | `TenantOpsDO`, triggered by first authenticated tenant request or explicit onboarding | `tenant_db` row `requested` → `db_creating`; intent recorded with a deterministic request id; jurisdiction read from the account's chosen tier                                             | none                                                                               |
| 4. database creation   | `TenantOpsDO` only, holding the platform Turso management credential                  | create database in the recorded jurisdiction; write `turso_db_id`/`turso_db_url`; `db_creating` → `db_ready`                                                                               | **yes** — retried with the same request id until the row is `db_ready` or `failed` |
| 5. schema bootstrap    | `TenantDO` on first routed request                                                    | idempotent `blockConcurrencyWhile` bootstrap at `schema_version` (the pattern already in `user-do.ts`)                                                                                     | none                                                                               |
| 6. credential issuance | tenancy Worker, on an authenticated session, for an `active` tenant                   | mint a scoped, short-TTL Turso token (§8); record `credential` metadata                                                                                                                    | yes (revocable)                                                                    |
| 7. activation          | `TenantOpsDO`                                                                         | `db_ready` → `active` once schema bootstrap has been observed                                                                                                                              | none                                                                               |
| 8. suspension          | operator role, or abuse/ceiling trigger                                               | `active` → `suspended`: refuse new tokens, do not drop existing data; existing tokens expire on their own TTL                                                                              | none (reversible)                                                                  |
| 9. export-only         | ceiling/over-quota, or a user request                                                 | `active`/`suspended` → `export_only`: reads and export stay, writes refused (the rule #3811 states)                                                                                        | none                                                                               |
| 10. deletion           | the account holder, plus a cooling-off window                                         | `export_only` → `deleting` → `deleted`: revoke all credentials, drop the tenant DB, mark `deleted_at`; the tenancy rows are retained as a tombstone (account, binding, publish back-links) | **yes**                                                                            |

Who may do what is a role check (#1253 vocabulary), not a hosted-only
privilege table: `owner` (the bound binding) may request, export and delete;
`operator` may suspend and force export-only; nothing but the platform's
management credential may create or drop a database, and that credential never
leaves the Worker.

Rules:

- **The state machine lives in one object per tenant.** `TenantOpsDO` is where
  transitions serialise, so two concurrent sign-ups cannot create two
  databases for one binding. The row is the record; the object is the lock.
- **Creation is intent-first.** The intent (request id, jurisdiction,
  `tenant_id`) is persisted before the provider call, so a retry after a
  crash adopts the existing database instead of creating a second one.
- **No database is created before a binding exists.** An account without a
  binding is an empty hosted account and gets no storage. This keeps
  storage cost tied to a proven identity and matches #2258.
- **Deletion is two-phase.** Credential revocation first, then the drop,
  then the tombstone: a half-deleted tenant whose token still works is the
  failure this prevents.
- **Suspension must not look like loss.** A suspended tenant's `TenantDO` still
  answers `/v1/state` and `/v1/export`; only writes and new tokens refuse.

## 5. Q4 — Sync protocol: native ↔ hosted

### 5.1 The decision

Sync rides the **existing signed-event sync path**, not Turso replication:
`LogSync` + range sync on `/id-sync/1` and the range-sync ALPN, carrying
`SignedEnvelope`/`SignedAnnouncement`/`SignedArtifact`
(`pkgs/id/crates/id-sync`: `log_sync.rs`, `envelope*.rs`, `announcement.rs`,
`artifact.rs`, `federation/`), with the hosted tenant as an ordinary peer.

Turso's own replication (embedded replicas, native libsql) stays available as
a **backend-local optimisation** for a Rust-binary node that wants a local
replica of its own tenant DB — it is not the interoperability protocol, because
it cannot serve a clustered self-hosted node, a browser client, or a peer that
isn't Turso. Choosing it as the protocol would be exactly the hosted-only
special-casing the peer principle forbids.

### 5.2 Change capture

- **Native → hosted.** The native instance is authoritative for its own
  authors' prefixes. It sends applied prefixes with heights (what `LogSync`
  already does), so the hosted tenant ingests an ordered, append-only event
  stream. Arrival order is the write path, batched, one transaction per batch —
  the semantics already implemented and documented in `log_sync.rs`.
- **Hosted → native.** Symmetric: the hosted tenant is a normal store holding
  the tenant's log, and answers the same request. Nothing in the hosted path
  is "download-only".
- **Per-object signed acceptance at every ingress.** The #9106/B32 gate runs
  on the hosted side exactly as on a native node: signature valid, signer
  authorized, semantically valid, scope authorized. A hosted ingress is not a
  trusted shortcut, and the peer principle means the hosted side cannot skip
  the gate "because it is us".

### 5.3 Ordering and conflict

Ordering is per-author prefix order (the model the store already implements),
never a global clock. Conflict resolution is:

1. **Fork detection is the conflict signal.** A fork in an author's chain is a
   conflict; the store reports it rather than silently picking a branch
   (`log_sync.rs` sends the prefix up to the fork and the proof exchange
   reports it; the hosted side must surface the same report to the user, not
   swallow it).
2. **Deterministic last-writer for metadata classes** where the schema already
   defines it (LWW on a stated field with a stated tiebreak), and
   **append-only union** for event/log classes.
3. **Content is content-addressed**, so a conflict never duplicates bytes: two
   branches referencing the same hash reference the same object, and blob GC
   is reference-counted until the last reference drops (component F's rule).

### 5.4 Offline behaviour

A native instance works fully offline against its own store and syncs when it
can reach the hosted tenant (or any peer). A hosted session that dies mid-batch
leaves the accepted prefix at the last committed batch; the next session
resumes from the stored watermark. Neither side requires the other to be up:
the hosted tenant keeps serving its own clients, and the native instance keeps
accepting local writes — the whole point of a per-user database rather than a
thin client.

### 5.5 Transports (component G seam, stated, not designed here)

The session is transport-agnostic: component G owns whether it runs over
WebSocket, HTTP, QUIC/iroh where available. The tenancy plane's only concern
is that a session is _attributable_: `SyncCoord` knows the tenant and the peer,
and the peer's policy version, before the first event is accepted. Frames
carry the peer's identity and the policy version so a policy change takes
effect at the next evaluation, never retroactively.

## 6. Q5 — The public database

**Decision: one shared public Turso database, tenant = the publish service;
publishing is an explicit, signed, retractable grant; per-user databases
reference public content by content hash, never by copying.**

- **Shape.** The public DB is a row in `tenant_db` with
  `participant_kind = 'public'` and `backend = 'turso'`. It is a normal tenant
  in every mechanical respect (it has a `TenantDO`-shaped owner for ingest
  serialisation, credentials, and a state machine), which is what keeps it
  inside the same rules instead of beside them. A federation of public DBs is
  _possible later_ by adding rows with `participant_kind='public'` and a peer
  class between them; it is not chosen now, because one database is simpler to
  moderate and to keep consistent, and the reference model (§6.3) makes moving
  to several a data-plane change rather than a protocol change.
- **Write path.** Only through `PublishIndex`, only with a signed artifact from
  a tenant that has the `publisher` role in its own scope, and only after the
  #9106 gate passes. Direct Turso write tokens onto the public DB are not
  issued to users; users get read tokens (§8). Publishing is therefore an
  operation on the tenancy plane, which is what makes retraction, moderation
  and abuse accounting possible at all.
- **Moderation and abuse.** The append-only publish log plus the
  `publish_record.retracted_at` marker gives the substrate; the _policy_ is
  #6007's tier model (Open / Standard / Strict) applied by F, and rate limits
  live on `PublishIndex` and the publish Worker. Takedown is a retraction on
  the public DB plus a note on the publishing tenant; it never reaches into the
  tenant's private database.
- **References, not copies.** A per-user database that wants to expose a
  public object holds a reference row (content hash + kind). Access follows
  the program rule (S10): a user may access an object iff their database holds
  a reference to it or it is public-published. Because the public copy is the
  published one, a retraction removes readability of the _public_ address while
  a tenant's own referenced copy keeps its own reference rule — the two are
  distinct, and the record must not conflate "retracted from public" with
  "deleted for the publisher".
- **Read path for "everyone can connect".** Anonymous/low-privilege readers get a
  short-TTL read-only token scoped to the public DB, or read through a Worker
  route when a token is impractical; both are the same authorisation decision
  (`scope=read`, audience = public DB) recorded in `credential`.

## 7. Q6 — Credential model for direct Turso access

**Decision: short-lived, audience-scoped, per-(tenant, database) Turso tokens
minted by the tenancy plane from an authenticated session; revocation by
`jti`; no long-lived user database passwords.**

- **Audience** is the `turso_db_id`. A token for one tenant's database cannot
  be replayed against another tenant's or against the tenancy DB.
- **Scope** is a closed vocabulary: `read`, `readwrite`, `ddl`. Users get
  `read`/`readwrite` to their own database only; `ddl` is reserved to the
  provisioning path; the tenancy DB issues no user tokens at all.
- **TTL** is short (target: minutes, with a refresh on an authenticated
  session). A leaked token's blast radius is bounded by TTL and by the
  per-database audience, not by trust.
- **Issuance** requires: `account.state='active'`, `binding.status='active'`,
  `tenant_db.state='active'`, and a live session. Any of those failing is a
  refusal, checked in that order (cheap first).
- **Revocation** is `credential.revoked_at` plus the `jti` deny list checked at
  issuance and by the front door; suspension revokes all credentials for the
  tenant in one write; deletion revokes before dropping (§4, step 10).
- **Rotation** for the platform's own management credential is a fleet
  concern (`doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/`),
  not a user-facing flow.
- **Never** does the tenancy DB store a token value; it stores the metadata
  needed to refuse the next mint and to audit the last one.

## 8. The #9106 federation amendment, mapped

This record does not design trust; it makes the tenancy DB the place those
decisions are _configured_, in B31's exact shape:

- `peer_policy` (§2.1) is `PeerPolicy`'s tenancy-side projection: `peer_id`,
  `class`, configured `operations`, separate `relay_granted`, `scope`,
  monotonic `policy_version`, optional `valid_from`/`expires_at`. The B31
  matrix is the conformance oracle; where this record's prose and the matrix
  disagree, the matrix wins.
- **Class is a ceiling, configuration is the grant.** A `fully_trusted` peer
  still needs the operation configured, and still passes the four-check gate.
  No tenancy row can bypass the gate; it can only refuse more.
- **Our own hosted nodes are ordinary peers.** A hosted tenant's
  `peer_policy` row is created with a sensible default (its own account's
  native nodes: `fully_trusted` with `accept_data`/`send_data`) and nothing
  more powerful. Self-hosted and Rust-binary peers are configured identically;
  `participant_kind` and `backend` describe capability, never trust.
- **Signed-data-only acceptance is invariant on every ingress** (B32). A
  hosted ingress path that accepts unsigned data is a defect, not a config.
- **Federated query fan-out and reputation are out of scope here** and stay in
  #9141–#9148/#9108. The tenancy contract with them: fan-out reads use
  `send_data`/relay grants recorded per peer; reputation attaches to
  `peer_id` (not to a hosted-only identity).

## 9. Relationship to #3811 — decided

**We build on #3811. We do not wrap it, and we do not supersede it.**

- **Built on:** the Worker front door + per-user `UserDO` keyed by
  `binding_id`, jurisdiction-restricted namespace selection with no unrestricted
  fallback, versioned idempotent schema bootstrap, R2 blob references,
  usage/ceiling and read-only-at-cap behaviour. Those are the tenant data
  plane and they stay (`pkgs/id/hosted-worker/`, #5199 onward).
- **Added above it:** the tenancy control plane (accounts, registry, roles,
  peers, publish, credentials), the provisioning state machine (`TenantOpsDO`),
  sync coordination (`SyncCoord`), the public plane (`PublishIndex`), and the
  per-user **Turso** database as the tenant's data store alongside the DO's
  SQLite. #3811's acceptance criteria are a subset of this record's tenant
  plane; nothing in #3811 is invalidated.
- **Superseded: nothing.** The one thing that _changes meaning_ is the word
  "user" in #3811: it becomes "tenant", because the peer principle admits
  tenants that are not hosted users. That is a vocabulary change, recorded
  here, applied in the docs as those packages are touched.
- **Scope note:** the metering in #3811 stays authoritative for tenant usage;
  the tenancy DB holds the rollup, not a second meter.

## 10. Rejected alternatives

| Alternative                                            | Why rejected                                                                                                                                                                                                                                           |
| ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| One shared Turso DB, row-level tenant isolation        | Blast radius is every tenant; per-database tokens become one token with a WHERE clause; jurisdiction isolation (#2283/#3401) cannot be per-tenant; and it contradicts the sketch's per-user databases without buying anything a second DB row doesn't. |
| Turso replication as _the_ sync protocol               | Not available to clustered self-hosted nodes, browser clients or non-Turso peers — hosted-only special-casing, forbidden by the peer principle. Kept as a backend-local optimisation (§5.1).                                                           |
| One DO per database                                    | Multiplies hibernating state without isolation gain; breaks the stable-name rule when a user gains a second database. One DO per tenant, keyed by `binding_id`.                                                                                        |
| A single global tenancy DO                             | Hot partition and single-object blast radius for every tenant's provisioning. Central data is a Turso DB; only per-tenant/per-session serialisation is an object.                                                                                      |
| Copy public content into each tenant DB                | Duplicates bytes and creates N divergent copies of one published object; contradicts F's content-addressed reference model and S10.                                                                                                                    |
| Long-lived per-user Turso tokens                       | Unbounded leak window, and no revocation story short of rotating the whole database's auth.                                                                                                                                                            |
| Tenancy DB as the content index / search plane         | Turns a control-plane breach into a content breach, and makes the central DB scale with content rather than with accounts.                                                                                                                             |
| Storing Turso tokens in the tenancy DB for convenience | A tenancy dump would then be database access. Metadata only.                                                                                                                                                                                           |

## 11. Granular implementation split (issues to file under #9083)

Each is independently reviewable and states the invariant it protects.

1. **E1 — tenancy schema + migration baseline.** Create the tenancy DB schema
   (§2.1) as a migration with the "never central" columns absent by test
   (a test that fails if a content/key/token column appears).
2. **E2 — `TenantOpsDO` provisioning state machine.** Transitions of §4 with
   intent-first idempotent creation, retry, and compensation; tests prove
   concurrent sign-ups create one database.
3. **E3 — `SyncCoord` + hosted sync ingress.** Session ownership, watermark
   persistence in the tenant DB, the #9106/B32 gate on ingress, conflict-report
   surfacing; golden test: a forked author is reported, not silently resolved.
4. **E4 — credential issuance/revocation service.** Mint scoped expiring tokens
   from the state checks in §7; `jti` deny list; suspension revokes in one
   write.
5. **E5 — public plane: `PublishIndex` + publish/retract.** Signed publish,
   `publish_record`, read token for public DB, retraction path, F's tier policy
   hook.
6. **E6 — tenant DB store adapter in `id-store` (with component D).** The
   Turso/SQLite-over-HTTP backend behind `store_traits` so the hosted tenant
   and native nodes share one storage interface.
7. **E7 — documentation pass: "user"→"tenant" in #3811-era docs**, plus the
   tenancy pointer in `planning/architecture/README.md`.

Sequencing: E1 and E6 are parallel; E2 needs E1; E3 needs E1 + E6; E4 needs
E1 + E2; E5 needs E1 + E3; E7 last.

## 12. Open questions left open on purpose

- **Tier numbers and ceilings** (#2046 parameter) — not decided here; the record
  only requires that the ceiling is read on the tenant plane and that
  `export_only` is the degradation.
- **Whether the public plane ever splits into a federation** — left as the
  row-level extension in §6; needs a moderation-cost argument, not a protocol
  one.
- **Jurisdiction vocabulary beyond `us|eu|fedramp`** — owned by #2283/#3401.
- **Reputation storage** — owned by #9106/#9108; the tenancy contract is that
  it keys on `peer_id`.

## 13. Acceptance mapping (issue #9083)

- [x] Tenancy schema sketch — §2 (central DDL, per-user, never-central list).
- [x] DO inventory — §3 (four classes, cardinality, keys, exclusions).
- [x] Provisioning lifecycle — §4 (ten states, who may do each, irreversible
      effects, rules).
- [x] Sync protocol — §5 (change capture, ordering, conflict, offline,
      transport seam).
- [x] Public-DB rules — §6 (single shared DB, write path, moderation, references).
- [x] Credential model (Q6, folded in as §7).
- [x] Peer principle and #9106 amendment honoured — §8 and throughout (§1, §5,
      §9's vocabulary change).
- [x] Relationship to #3811 decided and recorded — §9: build on, add above,
      supersede nothing.
- [ ] Granular implementation issues filed under #9083 — this record names the
      split (§11); the issues are filed by the follow-up pass that lands the PR
      (the design mill's rule: the record lands, then the issues).
