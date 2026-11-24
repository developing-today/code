# Tenancy control plane (component E)

The central tenancy database: one Turso/SQLite database holding the
account registry, binding pointers, the tenant/database registry,
role grants, peer policy, publish records, credential metadata and
the SSO identity links — and nothing else. Design record:
`doc/2026-11-17T16-55-00Z_design_hosted-tenancy-architecture`
(component E of epic #9078, issue #9083); the identity-link tables
are H2 of #9086 (issue #14260), designed in
`doc/2026-11-24T04-50-00Z_design_sso-relying-party-9086` §4.3.

This directory is the schema home for that database. The tenant
**data** plane (per-user databases and the `UserDO` that serves them)
is `../src/`; this is the control plane above it, queried by tenancy
Workers and driven, per tenant, by the `TenantOpsDO` in
`../src/tenant-ops.ts` (E2, #12797): the §4 provisioning state
machine over the `tenant_db` row — intent-first idempotent creation,
retry, compensation, and the two-phase deletion. E2 needed no schema
change: the baseline row already carries every field the machine
writes, and the machine's own journal lives in the object's storage,
not in this database.

## The invariant (design §2.3, "never central")

The tenancy DB holds **no user content, no keys, and no token
values**. Tokens are metadata only: the `credential` table records
scope, expiry and a `jti` revocation handle, never the token. The
public root key appears as `iroh_id` and nothing more. A tenancy-DB
dump must not yield content or database access.

`../test/tenancy-schema.test.ts` enforces this: it applies the
migrations and fails if any content/key/token column appears, if the
schema drifts from the schema of record (the §2.1 baseline plus the
H2 link tables), or if a participant kind gets special-cased.

## Peer principle

Hosted, Rust-binary and self-hosted nodes are the same participant
kinds. `tenant_db.participant_kind`
(`user_hosted | user_native | self_hosted_node | public`) is
configuration on the shared registry row; there are no hosted-only
tables and the sync/publish protocols never branch on "hosted".

## Migrations

Numbered, append-only SQL files in `migrations/`, applied in order.
`0001_tenancy_baseline.sql` is the §2.1 baseline (E1, #12796); later
E slices add migrations here rather than editing landed ones when
they need schema the baseline lacks (E2 provisioning needed none;
E4 credentials and E5 publish are the expected next authors).
`0002_identity_link.sql` (H2 of #9086, #14260) adds the
`identity_link` table — which login doors open which account — and
its append-only audit table `identity_link_event`; the link/unlink
API over them is `../src/identity-link.ts`. `meta.schema_version`
records the applied schema version, mirroring the `UserDO`
bootstrap pattern in `../src/user-do.ts`.

Two deliberate statements of the design's rules over its sketch DDL
are documented in the baseline's header: the active-binding rule is
a partial unique index (not the sketch's table-level UNIQUE, which
would break the rebind chain), and the vocabularies the record names
as closed carry CHECK constraints.
