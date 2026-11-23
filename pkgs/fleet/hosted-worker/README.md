# fleet-hosted

The hosted fleet front door: one Cloudflare Worker whose route modules
are the component-C front doors — stamping, query, queue, ingestion,
sync — over one shared auth/tenancy plumbing layer.

Design: `doc/2026-11-22T14-28-45Z_design_fleet-tool-workers-edition`
(component C of epic #9078, design issue #9081), §3.1. This package is
the service skeleton from #12583; each door is filled by its own issue.

## What is real in this slice

- **RP-token auth validation** (`src/jwt.ts`, `src/auth.ts`): callers
  present #9077 relying-party tokens as Bearer credentials. Validation
  is the standard relying-party check — JWS signature against the
  provider's JWKS (WebCrypto only, RS256/ES256), then issuer, audience
  and lifetime claims. The provider is configuration (`RP_ISSUER`,
  `RP_AUDIENCE`, `RP_JWKS_URI`), because #9077 itself is still a
  design-stage dependency; tests mint a local provider key.
- **Caller kinds** (design §4.1): people/agent runtimes via RP tokens;
  agent-role credentials (#1253), whose lane/session claims are
  required and become the caller's ceiling; per-caller **service
  tokens** recorded in the tenancy plane, looked up by SHA-256 and
  scoped to named doors.
- **Tenancy mapping** (`src/tenancy.ts`): a validated subject maps to
  a tenancy account keyed by its minted `binding_id` (32 lowercase
  hex, the component E / #2258 contract) — never an email or provider
  subject. Unmapped or inactive accounts fail closed, and a request
  asserting a different tenancy than its credential's
  (`x-binding-id` / `x-tenancy-binding` header) is refused as a
  cross-tenant attempt.
- **Data-plane refusal** (design §3.3 rule 3): a per-user Turso DB
  credential (libsql access-claim shape) presented as service auth is
  refused with `data_plane_credential_refused`. It is a read
  credential for the data plane, never an authentication here.
- **Health + capabilities** (`src/health.ts`, `src/capabilities.ts`):
  public identity surface. `/capabilities` mirrors the native
  `fleet capabilities` JSON shape — same `build` / `capabilities` /
  `compiled_capabilities` / `domains` fields, native registry values —
  with hosted additions (`domains[].hosted` disposition from design
  §2, `filling_issue`, and a `doors` list). In this skeleton no
  business domain reports available: the doors are stubs, and the
  document says so.

## What is a stub

Every door route authenticates, then answers `501 not_implemented`
naming the issue that fills it:

| Door      | Routes                                  | Filled by |
| --------- | --------------------------------------- | --------- |
| stamping  | `POST /v1/stamps`                       | #12585    |
| query     | `GET|POST /v1/query`                    | #12587    |
| queue     | `POST /v1/queue`                        | #12589    |
| ingestion | `POST /v1/ingestion/github`             | #12590    |
| sync      | `POST /v1/sync/project`, `POST /v1/sync/queue-github` | #12591 |

The session registry (#12584) and stamp signing (#12586) are separate
slices, deliberately not absorbed here.

## Seams left for later slices

- **Tenancy directory**: component E's tenancy DB does not exist yet,
  so `TenancyDirectory` is an interface with a fixture implementation
  (`TENANCY_FIXTURE_JSON`, tests/local dev only). A DB-backed
  implementation replaces it behind the same interface.
- **Route paths** under `/v1/` are the skeleton's; each filling issue
  owns its door's final verb shapes (the queue door's broker-protocol
  transport is #12589's design surface).

## Develop

```sh
npm test        # node --test over test/*.test.ts (no network, no wrangler)
npm run smoke   # in-process local run answering the acceptance checks
npm run typecheck
npm run dev     # wrangler dev (set RP_ISSUER / TENANCY_FIXTURE_JSON first)
```

Local-only: `wrangler.toml` authorizes no account deployment; a later
C issue owns production deployment.
