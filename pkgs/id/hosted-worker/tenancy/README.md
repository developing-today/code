# Tenancy control plane (component E)

The central tenancy database: one Turso/SQLite database holding the
account registry, binding pointers, the tenant/database registry,
role grants, peer policy, publish records and credential metadata —
and nothing else. Design record:
`doc/2026-11-17T16-55-00Z_design_hosted-tenancy-architecture`
(component E of epic #9078, issue #9083).

This directory is the schema home for that database. The tenant
**data** plane (per-user databases and the `UserDO` that serves them)
is `../src/`; this is the control plane above it, queried by tenancy
Workers and the `TenancyOpsDO` of later E slices.

## The invariant (design §2.3, "never central")

The tenancy DB holds **no user content, no keys, and no token
values**. Tokens are metadata only: the `credential` table records
scope, expiry and a `jti` revocation handle, never the token. The
public root key appears as `iroh_id` and nothing more. A tenancy-DB
dump must not yield content or database access.

`../test/tenancy-schema.test.ts` enforces this: it applies the
migrations and fails if any content/key/token column appears, if the
schema drifts from the §2.1 baseline, or if a participant kind gets
special-cased.

## Peer principle

Hosted, Rust-binary and self-hosted nodes are the same participant
kinds. `tenant_db.participant_kind`
(`user_hosted | user_native | self_hosted_node | public`) is
configuration on the shared registry row; there are no hosted-only
tables and the sync/publish protocols never branch on "hosted".

## Migrations

Numbered, append-only SQL files in `migrations/`, applied in order.
`0001_tenancy_baseline.sql` is the §2.1 baseline (E1, #12796); later
E slices (E2 provisioning, E4 credentials, E5 publish) add migrations
here rather than editing landed ones. `meta.schema_version` records
the applied baseline, mirroring the `UserDO` bootstrap pattern in
`../src/user-do.ts`.

Two deliberate statements of the design's rules over its sketch DDL
are documented in the baseline's header: the active-binding rule is
a partial unique index (not the sketch's table-level UNIQUE, which
would break the rebind chain), and the vocabularies the record names
as closed carry CHECK constraints.
