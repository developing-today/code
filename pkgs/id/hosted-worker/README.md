# id-hosted-worker — hosted data-plane scaffold (#5199)

First local-only vertical slice of the hosted data plane (slice 3 of
#2852, child of #3811, grounded in #2258 §5 and the #2046 ruling):

- `src/worker.ts` — the front door. Consumes the already-authenticated
  resolved context (`X-Binding-Id`, `X-Jurisdiction`, set by the
  authenticated internal caller; the binding ceremony itself lives in
  `pkgs/id/src/hosted_binding.rs`), strips client-supplied internal
  headers, and forwards to the per-tenant Durable Object.
- `src/routing.ts` — the only module that touches the raw `USERS`
  namespace. Objects are named by the stable `binding_id` minted at
  binding time (32 lowercase hex chars) — never email, never a provider
  subject. Both the ID and the stub come from the same
  `env.USERS.jurisdiction(jurisdiction)` restricted namespace. There is
  no unrestricted fallback: an unavailable restriction fails closed.
- `src/user-do.ts` — `UserDO`: versioned SQLite schema bootstrapped
  idempotently in `blockConcurrencyWhile` before serving; the first
  routed request binds the object immutably to its
  (binding_id, jurisdiction); a common guard runs before every read or
  write and fails closed on unrouted requests, identity mismatch, or a
  runtime-reported jurisdiction mismatch. Routes (13 paths,
  15 method routes): `GET /v1/state`; `GET /v1/usage` (usage,
  ceiling, and `at_cap`); `GET /v1/export` (full KV plus blob index;
  stays available at cap); `PUT|GET|DELETE /v1/kv/<key>` (per-tenant
  store; the key must match `[A-Za-z0-9._-]{1,128}`); `PUT /v1/blobs`,
  `GET|DELETE /v1/blobs/<64-hex>` (content-addressed blobs in
  jurisdiction-bound R2 — `BLOBS_US`/`BLOBS_EU`/`BLOBS_FEDRAMP` picked
  strictly from the authoritative jurisdiction and deduped within each
  bucket — except client-encrypted blobs (§7 of the #2258 spec), whose
  R2 keys are scoped to the owning binding and dedupe only per user.
  DELETE drops only this user's reference); and the signed-object bridge routes (`GET
  /v1/bridge/challenge`, `GET /v1/bridge/heads`, `POST
  /v1/bridge/push`, `POST /v1/bridge/pull`, `GET /v1/bridge/export`,
  `GET /v1/bridge/ws`), covered in the bridge section below. Storage
  metering and the per-user ceiling (`USER_CEILING_BYTES`) are
  implemented, not later steps. So is the per-binding request cap
  (#15485): with `USER_REQUEST_CEILING` set, requests past that many
  in a `USER_REQUEST_WINDOW_SECONDS` window (default one day) get a
  retryable 429 with `Retry-After`; `GET /v1/usage` stays available
  and reports the window. A malformed value fails requests closed.
- The durable source fence for jurisdiction migration (#3404,
  `src/migration-fence.ts`): the migration principal (an internal
  header the front door strips) can `POST /v1/migration/freeze`,
  `/seal` and `/unfreeze` with `{migration_id, routing_epoch}`, and
  `GET /v1/migration/fence`. While frozen or sealed, every mutation
  (bridge push and WebSocket batches, blob and KV writes and deletes,
  new bridge sessions) gets a retryable 503 `source_frozen` or
  `source_sealed`, checked at its commit point; reads and export stay
  open. A freeze closes live bridge sessions and acknowledges the
  mutation watermark. Unfreeze (abort) needs a newer epoch, a sealed
  source never unfreezes, and a replayed freeze at an old epoch is
  refused.
- Directory migration epochs (#3404, `planDirectoryTransition` and
  `PersistentHostedAccountDirectory.transition`): an account record
  moves stable, pending, frozen, verified, switched and back to stable
  by compare-and-set on the whole record it was read as. Begin and
  switch advance the routing epoch, abort advances it again, a retried
  step returns the applied row, and a stale or competing step gets 409
  `directory_cas_conflict`. Migrating records still route to the
  source; a switched record gets a retryable 503 `migration_switching`
  at the front door until the migration completes.
- Migration snapshot and staged target (#3404, `src/migration-snapshot.ts`):
  the source answers `POST /v1/migration/snapshot` only while its
  fence stands for that migration and epoch, with every data table
  (KV, blob references, the bridge chain store), the freeze watermark
  and a SHA-256 digest. The target takes it at `POST /v1/migration/import`
  only when fresh, checks the digest before and after writing, and
  stays staged: every client route gets a retryable 503
  `target_not_serving`. `POST /v1/migration/activate` at the switch
  epoch serves it once every referenced blob is in the target's
  bucket; `/discard` empties a staged target for a retry.
- The migration coordinator (#3404, `src/migration-coordinator.ts`):
  `driveMigration` takes the account's migration lease
  (`migrations/0003_migration_leases.sql`, a token that rises whenever
  the lease changes hands) and drives begin, freeze, snapshot, blob
  copy, import, check, switch, activate, seal and complete, every
  directory step a compare-and-set that also checks the lease. It keeps
  no state of its own and resumes from the directory's phase after a
  crash or a lost response. `abortMigration` discards the staged copy,
  moves the directory back to the source at a fresh epoch and reopens
  the source, which then refuses a stale freeze. Nothing exposes the
  coordinator to callers yet.

## Tenancy control plane (component E, #9083)

`tenancy/` holds the central tenancy DB schema — the control plane
above this data plane: account registry, binding pointers,
tenant/database registry, role grants, peer policy, publish records
and credential metadata, as numbered migrations. It holds no user
content, no keys and no token values (design §2.3, "never central");
`test/tenancy-schema.test.ts` fails if a content/key/token column
ever appears. See `tenancy/README.md`.

- `src/tenant-ops.ts` — `TenantOpsDO` (E2 of #9083, #12797): one
  object per tenant, named by `binding_id`, implementing the design
  §4 provisioning state machine over the `tenant_db` row —
  `requested → db_creating → db_ready → active`, suspension,
  export-only, and two-phase deletion (credential revocation,
  cooling-off window, drop, tombstone). Creation is intent-first
  under a deterministic request id, so concurrent sign-ups create
  exactly one database; every step checks the actor §4 names for
  it. Bound as `TENANT_OPS` in `wrangler.toml` but not routed by
  the front door — the tenancy Worker drives it. The Turso
  management API sits behind a provider seam; unconfigured, it
  fails closed. `test/tenant-ops.test.ts` exercises the machine
  against the real tenancy migrations.

## Local reality check (verified 2026-11-03, wrangler dev / workerd)

Local workerd does **not** implement jurisdiction restrictions:
`namespace.jurisdiction("us")` throws `Jurisdiction restrictions are
not implemented in workerd.` and an unrestricted object id reports no
jurisdiction. So locally the application worker fails closed with
`503 jurisdiction_unavailable` for every fully-valid request — that is
the designed behaviour, not a bug: falling back to the unrestricted
namespace would silently create the second, unrestricted store the
design forbids. Residency on a real account can only be established by
the deployed canary and resource read-back; local tests do not establish
where production data resides. Production signup and migration remain
disabled until that validation passes.

The hosted account directory is one shared D1 database and is the **single
shared durable-directory residency exception**. Its allowlisted records
contain pseudonymous subject keys and minimal binding, jurisdiction, and
migration metadata. A hashed subject is potentially personal data, not
anonymous. The directory does not hold graph content, journals, blob bytes,
or private profile data. This exception describes D1 only and is not an
end-to-end residency claim: KV caches may be global; logs and request
processing may occur outside a tenant's jurisdiction; vendor backup
placement is a separate, unverified residency gap. These locations and
processing paths need their own disclosure and evidence.

## The signed-object bridge (#3818)

The hosted half of the #2046 bridge: production HTTPS/WebSocket
push/pull between native ID nodes and this per-tenant store. The
store is a sync peer — it holds signed objects and proof-carrying
chain/fork evidence, assigns no order, re-signs nothing, and
serves no authoritative snapshot.

- `src/event.ts` + `src/blake3.ts` — the signed event as
  `chain.rs::decode` verifies it (canonical CBOR via
  `src/canonical-cbor.ts`, Ed25519 over the signing bytes, BLAKE3
  operation IDs), and a signer for clients/tests.
- `src/bridge-store.ts` — native ingest semantics (accept /
  duplicate / fork / waiting / frozen, the release drain, fork
  proofs, the derived applied prefix) mirrored on the object's
  SQLite (schema v3).
- `src/range.ts` — range reconciliation transcribed from
  `crates/id-events`' `range.rs`.
- `src/bridge-auth.ts` — ID-key proof against the binding record:
  the front door forwards the bound Iroh root key as the internal
  `x-hosted-iroh-id` header; callers prove possession against a
  single-use nonce. No bound key → bridge routes fail closed.
- `src/bridge-session.ts` — the session protocol (challenge/auth,
  heads, resume tokens, range flights, bounded acknowledged
  batches, withholding detection), the server session, and a
  reference client.
- Routes: `GET /v1/bridge/challenge`, `GET /v1/bridge/heads`,
  `POST /v1/bridge/push`, `POST /v1/bridge/pull`,
  `GET /v1/bridge/ws`. Bridge bytes count toward the storage
  ceiling. Design record:
  `doc/2026-11-23T05-00-00Z_design_bridge_v1_implementation/`.
  Cross-language fixtures: `pkgs/id/tests/bridge_fixtures.rs`.
- The `/v1/bridge/ws` socket hibernates (#15484): the DO accepts it
  with `state.acceptWebSocket`, never `server.accept()`, so an idle
  session does not keep the object resident. The socket's
  attachment names its session, and the session's state is written
  to `bridge_ws_sessions` after every frame, so a fresh instance
  after an eviction continues the same session. A socket whose
  state is gone gets a `session_lost` error and is closed; the
  client reconnects and resumes with its token.
- `src/export.ts` — §6 export production (slice 4b of #2852,
  #13157): enumerates the DO's applied events and the R2 blobs
  they cite into the object/blob sets `build_export`
  (`pkgs/id/src/hosted_export.rs`) assembles, under the
  object-model mapping recorded in the spec as §6.3. Route:
  `GET /v1/bridge/export`, ID-key proof like the other bridge
  HTTPS legs; a pure read, available at cap, and a referenced
  blob R2 cannot produce fails it loudly (502 naming the hash).
  The produced sets are pinned to
  `test/fixtures/hosted-export-sets.json`, which the Rust suite
  verifies through `build_export` + `verify_import`.

## Tests

- `npm test` — routing unit tests (validation, restricted-namespace
  selection, fail-closed on unavailable restriction) with plain
  `node --test`; the bridge suites (codec, store, range, session
  acceptance, DO routes) run in the same command.
- `npm run smoke` — two local `wrangler dev` passes: (1) the app
  worker fail-closed matrix (401/403/503 above); (2) `UserDO`
  bootstrap / identity-guard / isolation through the test-only harness
  (`wrangler.test.toml`, `src/test-harness.ts`), which supplies the
  internal routing context directly because workerd cannot produce a
  restricted stub. The harness is never an application entry point.
- `npm run typecheck` — `tsc --noEmit`.

## Currency gateway PoC (#2298)

`src/currency/` is the gated #2298 proof of concept (gates cleared
2026-11-12/13; the user's ruling `hosted-boundary-2298`), flagged for a
review round — not the permanent currency service:

- `ledger.ts` — in-process ledger stub behind the gateway interface
  (TigerBeetle-shaped semantics: unique request IDs,
  `exists_with_different_fields`, two-phase holds, watermarks).
- `gateway.ts` — single-process stub gateway, the only ledger
  client: transfers, holds + post/void, gateway-signed balance
  snapshots (server-scoped, token admin key), request-ID lookup,
  bilateral signed double-entry for cross-server transfers.
- `relay.ts` — the hosted-edition UserDO relay: persists
  {request ID, intent hash, state} before the gateway fetch, caches
  signed snapshots only, reconciles by request-ID lookup on wake.
- `crypto.ts` — Ed25519 (WebCrypto) + canonical signing payloads.

Proofs: `test/currency.test.ts` (run by `npm test`). Results +
the explicit simplification list:
`doc/2026-11-14T06-22-00Z_poc_currency_gateway_boundary_results/`.
