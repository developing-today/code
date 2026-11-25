# The hosted Worker API

Source: https://github.com/developing-today/code/blob/main/pkgs/id/hosted-worker/src/worker.ts, https://github.com/developing-today/code/blob/main/pkgs/id/hosted-worker/src/user-do.ts, https://github.com/developing-today/code/blob/main/pkgs/id/hosted-worker/src/tenant-ops.ts, https://github.com/developing-today/code/blob/main/pkgs/id/hosted-worker/wrangler.toml, https://github.com/developing-today/code/blob/main/pkgs/id/worker-poc/README.md

This chapter documents the hosted Worker **as it stands**. Two packages exist, and neither is a live service:

| Package                 | Status                                               |
| ----------------------- | ---------------------------------------------------- |
| `pkgs/id/hosted-worker` | A local-only scaffold. Not deployed.                 |
| `pkgs/id/worker-poc`    | A completed proof of concept. Not a production path. |

## Disposition: local-only scaffold, not deployed

The header of `hosted-worker/wrangler.toml` states the position in full:

> LOCAL ONLY: this config is for wrangler dev / tests. No account-level deployment, production binding, or billable resource is authorized by this package — deployment and the real-account jurisdiction canary are gated on #2285 / #3403.

Two consequences follow from that gate:

1. There is no production deployment, no production binding, and no billable resource for this package.
2. Local `workerd` does not implement jurisdiction restrictions. The app worker therefore **fails closed**: every fully-valid request run locally is answered `503` with the error `jurisdiction_unavailable`, by design. A local `503 jurisdiction_unavailable` is the expected result of a correct request, not a bug.

Deployment stays gated on issues #2285 and #3403 until both close.

## What it would host, and for whom

The hosted data plane for bound hosted-edition users. There is one `UserDO` Durable Object per `binding_id`, holding that user's:

- per-user key/value store,
- blob references (the bytes live in R2),
- a signed-object bridge store that acts as a _sync peer_: it holds signed objects and fork evidence, assigns no order, re-signs nothing, and serves no authoritative snapshot.

## The front door

`src/worker.ts` (73 lines) is the entry point. It does not authenticate users itself. It consumes an _already-authenticated_ resolved context set by an authenticated internal caller; the binding ceremony that produces that context is native code, in `pkgs/id/src/hosted_binding.rs`.

Request handling, in order:

1. Read the resolved context headers `X-Binding-Id` and `X-Jurisdiction` (and `X-Iroh-Id`).
2. Strip every client-supplied internal header.
3. Re-set the internal headers itself: `x-hosted-binding-id`, `x-hosted-jurisdiction`, `x-hosted-routed: restricted`, `x-hosted-iroh-id`.
4. Enforce the front-door body limit (see `USER_CEILING_BYTES` below; the cap is 100 MiB).
5. Forward to the per-user Durable Object through the jurisdiction-restricted namespace only.

Routing rules in `src/routing.ts` — the only module that touches the raw `USERS` namespace:

- Durable Objects are named by `binding_id`: 32 lowercase hex characters.
- Objects are never named by email or by provider subject.
- There is no unrestricted fallback. If the restricted namespace cannot serve the request, the request fails; it is not retried against an unrestricted namespace.

## Bindings

From `wrangler.toml`: Worker name `id-hosted-worker`, `compatibility_date` `2025-06-01`.

| Binding / var               | Type                                     | Notes                                                                                                                                                                                               |
| --------------------------- | ---------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `USERS`                     | Durable Object namespace → `UserDO`      | Migration tag `v1`, `new_sqlite_classes = ["UserDO"]`.                                                                                                                                              |
| `TENANT_OPS`                | Durable Object namespace → `TenantOpsDO` | Migration tag `v2`, `new_sqlite_classes = ["TenantOpsDO"]`. One object per tenant, named by `binding_id`. Exported by `worker.ts` but never routed by the front door; the tenancy Worker drives it. |
| `BLOBS`                     | R2 bucket `id-hosted-blobs`              | Content-addressed blob bytes. Local emulation only. In the `UserDO` interface the binding is optional, and the code that needs it fails closed without it.                                          |
| `USER_CEILING_BYTES`        | String var, optional                     | Per-user storage ceiling. Absent or empty means no ceiling. A malformed value fails writes closed. Also read by the front door for the body limit: `MAX_BODY_BYTES` is a 100 MiB cap.               |
| `BRIDGE_WAITING_PER_AUTHOR` | String var, optional                     | Bridge waiting-pool bound, per author. Absent means the native default in `bridge-store.ts`. A malformed value fails bridge writes closed.                                                          |
| `BRIDGE_WAITING_TOTAL`      | String var, optional                     | Bridge waiting-pool bound, total. Same default and failure rule.                                                                                                                                    |
| `BRIDGE_WAITING_MAX_AGE_MS` | String var, optional                     | Bridge waiting-pool bound, maximum age in milliseconds. Same default and failure rule.                                                                                                              |

Test harness: `wrangler.test.toml` defines a separate Worker, name `id-hosted-worker-test`, main `src/test-harness.ts`, with binding `TEST_USERS` → `UserDO`, bucket `id-hosted-blobs-test`, and `USER_CEILING_BYTES = "1073741824"`. It is test-only and is never an application entry point.

## UserDO routes

Implementation: `src/user-do.ts`. There are **13 paths and 15 method routes**.

Every route first runs a common guard:

1. The routing context is present.
2. The runtime jurisdiction matches the routed jurisdiction.
3. The persisted `(binding_id, jurisdiction)` pair matches. That pair is bound immutably by the first routed request and cannot be changed by later requests.

The object's SQLite schema is `SCHEMA_VERSION = 3`, with tables `meta`, `kv` and `blobs`, plus the bridge schema listed in [Data stores and schemas](data-stores.md).

| Method | Path                   | Purpose                                                                                                                                                        |
| ------ | ---------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| GET    | `/v1/state`            | `binding_id`, `jurisdiction`, `schema_version`, object name.                                                                                                   |
| GET    | `/v1/usage`            | Usage, ceiling, and `at_cap`.                                                                                                                                  |
| GET    | `/v1/export`           | Full KV plus blob index export. Stays available at cap, by design.                                                                                             |
| PUT    | `/v1/kv/{key}`         | Per-user KV write. The key must match `[A-Za-z0-9._-]{1,128}`.                                                                                                 |
| GET    | `/v1/kv/{key}`         | KV read.                                                                                                                                                       |
| DELETE | `/v1/kv/{key}`         | KV delete. Available at cap.                                                                                                                                   |
| PUT    | `/v1/blobs`            | Store a content-addressed blob. The key is the SHA-256 hex of the content. Bytes are deduplicated across users in R2; each user gets a per-user reference row. |
| GET    | `/v1/blobs/{64-hex}`   | Fetch a blob this user references.                                                                                                                             |
| DELETE | `/v1/blobs/{64-hex}`   | Drop this user's reference only. The R2 bytes are deliberately not deleted; garbage collection is a separate, unimplemented family of work.                    |
| GET    | `/v1/bridge/challenge` | Issue a single-use ID-key proof nonce for the bridge.                                                                                                          |
| GET    | `/v1/bridge/heads`     | Bridge chain heads. Requires a fresh ID-key proof per call.                                                                                                    |
| POST   | `/v1/bridge/push`      | Ingest a batch of signed events. Per-event outcomes: `accepted`, `duplicate`, `fork`, `waiting`, `refused`.                                                    |
| POST   | `/v1/bridge/pull`      | Pull events beyond per-author positions. Returns a binary batch, with `x-bridge-more` and `x-bridge-heads` headers.                                            |
| GET    | `/v1/bridge/export`    | Export sets for the native `build_export`. Answers `502` naming the hash if R2 cannot produce a cited blob.                                                    |
| WS     | `/v1/bridge/ws`        | Bridge session: in-band `CHALLENGE`/`AUTH`, binary frames only.                                                                                                |

Failure shapes:

- Any other path or method: `404` with body `{"error":"not_found"}`.
- Locally, any fully-valid request: `503` with `jurisdiction_unavailable` (see "Disposition" above).
- Bridge routes additionally fail closed when the binding record names no ID (Iroh root) key.
- A KV key outside `[A-Za-z0-9._-]{1,128}` is not a valid route key.
- A blob path whose segment is not 64 hex characters does not name a blob.

## TenantOpsDO routes

Implementation: `src/tenant-ops.ts` (E2 of the hosted-tenancy component, #12797), implementing the provisioning lifecycle of the tenancy design record §4 over the central tenancy DB's `tenant_db` row (schema baseline in `pkgs/id/hosted-worker/tenancy/`). One object per tenant, named by `binding_id`. The front door does not route to it; the caller is the tenancy Worker, which sets the actor on internal headers (`x-tenancy-actor-role`, `x-tenancy-actor-id`, `x-tenancy-trigger`) after authentication, and every step refuses an actor §4 does not name for it. All routes are POST except the state read.

| Method | Path                            | Who (§4)                                       | Effect                                                                                                                                                                                                                            |
| ------ | ------------------------------- | ---------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| POST   | `/v1/tenant/request`            | owner (the bound binding)                      | Record the row `requested` with its provisioning intent; a missing or inactive binding fails validation on the row (`failed`). Idempotent.                                                                                        |
| POST   | `/v1/tenant/provision`          | platform (the object itself)                   | `requested` → `db_creating` → `db_ready`: create the tenant database through the provider seam under the deterministic request id; retries adopt the database the intent already made instead of creating a second one.           |
| POST   | `/v1/tenant/bootstrap-observed` | platform                                       | Record that the TenantDO's schema bootstrap has been observed; activation is refused until this exists.                                                                                                                           |
| POST   | `/v1/tenant/activate`           | platform                                       | `db_ready` → `active`.                                                                                                                                                                                                            |
| POST   | `/v1/tenant/suspend`            | operator, or the abuse/ceiling trigger         | `active` → `suspended`. Reversible; data untouched.                                                                                                                                                                               |
| POST   | `/v1/tenant/resume`             | operator                                       | `suspended` → `active`.                                                                                                                                                                                                           |
| POST   | `/v1/tenant/export-only`        | owner, operator, or the ceiling trigger        | `active`/`suspended` → `export_only`: reads and export stay, writes refuse.                                                                                                                                                       |
| POST   | `/v1/tenant/delete`             | owner (initiate); owner or platform (complete) | Two-phase: initiation (from `export_only` only) revokes all credential metadata and opens the cooling-off window (`deleting`); completion after the window drops the database and writes the tombstone (`deleted`, row retained). |
| GET    | `/v1/tenant/state`              | owner, operator, platform                      | The registry row. Answers in every state — suspension must not look like loss.                                                                                                                                                    |

Failure shapes: no or unknown actor → `401`; an actor §4 does not name for the step (including an owner naming a different binding) → `403 actor_not_permitted`; a step attempted from a state §4 does not allow → `409 invalid_transition`; activation before observed bootstrap → `409 bootstrap_not_observed`; deletion inside the cooling-off window → `409 cooling_off`. With no database provider configured, provisioning and drops fail closed with `503 provider_unavailable` and the row does not advance. Credential issuance is a later slice (E4); this surface mints nothing and only revokes, as deletion's first phase.

### The README route list is a first-slice fossil (WEB-9)

The `hosted-worker` README describes the UserDO routes as `GET /v1/state` and `PUT|GET /v1/kv/<key>`, calls the KV store minimal, and states that R2 blobs, metering and caps are later steps and out of scope. The code implements blobs (PUT, GET, DELETE), usage and metering, export, KV DELETE, and the full bridge. The README's later sections cover the bridge and export but never revise that paragraph, and blobs, usage and KV-delete appear in no README route list. That is disagreement WEB-9; the table above is the implemented surface.

## In-package surfaces that the front door does not route

These modules exist in the package but are not routed by `worker.ts`:

| Module                                               | What it is                                                                                                                                                                                                                                                              |
| ---------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/provisioning.ts`, `src/provisioning-surface.ts` | The provisioning surface.                                                                                                                                                                                                                                               |
| `src/tenant-ops.ts`                                  | `TenantOpsDO`, the tenancy provisioning state machine (see TenantOpsDO routes above). Exported and bound, but driven by the tenancy Worker, not the front door.                                                                                                         |
| `src/account-directory.ts`                           | Account directory.                                                                                                                                                                                                                                                      |
| `src/identity-link.ts`                               | SSO relying-party identity links (H2 of #9086, #14260): the link/unlink API over the tenancy `identity_link` table (migration `0002_identity_link.sql`). One row per linked login door, keyed by the provider's subject at that door; email never appears in the table. |
| `src/admission-gate.ts`                              | Admission gate.                                                                                                                                                                                                                                                         |
| `src/export.ts`                                      | Export support behind `/v1/bridge/export`.                                                                                                                                                                                                                              |
| `src/currency/{gateway,ledger,relay,crypto}.ts`      | A currency gateway proof of concept. Its own documentation states it is "not the permanent currency service": an in-process ledger stub modelled on TigerBeetle, gateway-signed balance snapshots, and a UserDO relay that reconciles by request-ID lookup.             |

## worker-poc: a completed proof, not live

`pkgs/id/worker-poc` is a proof of concept for issue #2271: run one example world unmodified in a Worker isolate and prove journal equality against the native wasmtime host. All of its acceptance boxes are checked (2026-11-01). Nothing in it is a production path.

Its Worker (`src/worker.ts`) exposes a `WorldObject` Durable Object (binding `WORLD`, migration `v1-sqlite`, with a CompiledWasm rule for the prebuilt guest):

| Method | Path          | Purpose                                       |
| ------ | ------------- | --------------------------------------------- |
| POST   | `/step`       | Apply one step. Body: `{participant, event}`. |
| POST   | `/cold-start` | Cold-start the object.                        |
| GET    | `/state`      | Read the state.                               |

The SQLite `journal` table is the source of truth. The hosted loader (`src/manifest.ts`) refuses a manifest with `portability=native`, and refuses a manifest declaring more than 64 MB of memory, up front.

The `d1-journal-poc/` subfolder is a second, local-only D1 fit probe. It uses a placeholder database ID and always runs `--local`. Binding `DB` (D1):

| Method | Path        | Purpose                                                                                                                                   |
| ------ | ----------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| GET    | `/health`   | Health check.                                                                                                                             |
| POST   | `/append`   | Append a journal entry.                                                                                                                   |
| GET    | `/snapshot` | Read a snapshot.                                                                                                                          |
| GET    | `/tail`     | Read the journal tail. Journal header version is `1`; entries include `created` and `caps_granted`. A D1 bookmark is not a stream cursor. |

The two proofs of concept have their own chapters, which carry their full surfaces and results: [id-worker-poc: the World Durable Object proof of concept](worker-poc.md) and [id-d1-journal-poc: the D1 journal proof of concept](d1-journal-poc.md). A third Worker belongs to ID but is not part of the hosted edition at all: the production mail Worker covered in [The mail Worker API](mail-worker.md).

## Worked example

Toy data: a bound user with `binding_id` written here as `BINDING_ID`, run under the local test harness, where the ceiling is `1073741824` bytes.

```text
PUT  /v1/kv/profile.name        body: "Ada (toy)"   → 200
GET  /v1/kv/profile.name                            → 200  "Ada (toy)"
PUT  /v1/blobs                  body: <toy bytes>   → 200  (SHA-256 hex key returned)
GET  /v1/usage                                      → 200  {usage, ceiling: 1073741824, at_cap: false}
GET  /v1/export                                     → 200  (KV plus blob index)
DELETE /v1/kv/profile.name                          → 200
DELETE /v1/blobs/<64-hex>                           → 200  (reference dropped; R2 bytes remain)
GET  /v1/nope                                       → 404  {"error":"not_found"}
```

The same sequence against the local app worker (not the test harness) stops earlier: each fully-valid request is answered `503 jurisdiction_unavailable`, because local `workerd` has no jurisdiction restriction support and the worker fails closed. That is the designed local behaviour until the #2285 / #3403 gates clear.
