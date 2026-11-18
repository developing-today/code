# id-hosted-worker — hosted data-plane scaffold (#5199)

First local-only vertical slice of the hosted data plane (slice 3 of
#2852, child of #3811, grounded in #2258 §5 and the #2046 ruling):

- `src/worker.ts` — the front door. Consumes the already-authenticated
  resolved context (`X-Binding-Id`, `X-Jurisdiction`, set by the
  authenticated internal caller; the binding ceremony itself lives in
  `pkgs/id/src/hosted_binding.rs`), strips client-supplied internal
  headers, and forwards to the per-user Durable Object.
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
  runtime-reported jurisdiction mismatch. Routes: `GET /v1/state`,
  `PUT|GET /v1/kv/<key>` (a minimal per-user store proving lazy
  creation and isolation; R2 blobs, metering and caps are later
  #3811 steps and are out of scope here).

## Local reality check (verified 2026-11-03, wrangler dev / workerd)

Local workerd does **not** implement jurisdiction restrictions:
`namespace.jurisdiction("us")` throws `Jurisdiction restrictions are
not implemented in workerd.` and an unrestricted object id reports no
jurisdiction. So locally the application worker fails closed with
`503 jurisdiction_unavailable` for every fully-valid request — that is
the designed behaviour, not a bug: falling back to the unrestricted
namespace would silently create the second, unrestricted store the
design forbids. Residency on a real account can only be established by
the deployed canary gated on #2285 (still open/escalated) — this
package makes no residency claim and authorizes no deployment, no
production binding/config, and no billable resource (#3403 stays
blocked pending Drew authorization).

## Tests

- `npm test` — routing unit tests (validation, restricted-namespace
  selection, fail-closed on unavailable restriction) with plain
  `node --test`.
- `npm run smoke` — two local `wrangler dev` passes: (1) the app
  worker fail-closed matrix (401/403/503 above); (2) `UserDO`
  bootstrap / identity-guard / isolation through the test-only harness
  (`wrangler.test.toml`, `src/test-harness.ts`), which supplies the
  internal routing context directly because workerd cannot produce a
  restricted stub. The harness is never an application entry point.
- `npm run typecheck` — `tsc --noEmit`.

## Currency gateway PoC (#2298)

`src/currency/` is the gated #2298 proof of concept (gates cleared
2026-11-12/13; Drew ruling `hosted-boundary-2298`), flagged for a
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
