# PoC results — currency gateway boundary (#2298)

**Status:** proof of concept, flagged for a review round. Do NOT build
the permanent currency service on top of this unreviewed (the issue's
own gate note). Code: `pkgs/id/hosted-worker/src/currency/`
(`crypto.ts`, `ledger.ts`, `gateway.ts`, `relay.ts`); proofs:
`pkgs/id/hosted-worker/test/currency.test.ts`.

**Gate:** respected. Work started only after (a) the #2043 currencies
v1 synthesis accepted the #2066 boundary — Drew's recorded ruling
`hosted-boundary-2298` (2026-11-12): hosted tier holds signed
snapshots only, server-scoped tokens signed by the server's token
admin key, cross-server transfers are bilateral signed double-entry,
do not block on TigerBeetle — and (b) the #2062 control-plane design
fixed the gateway shape (closed; PR #2315 merged).

## The four proofs, demonstrated

Run in `pkgs/id/hosted-worker`:

```
$ npm test
✔ proof 1: gateway transfer + signed snapshot verifies and is server-scoped
✔ proof 1b: gateway rejects an unsigned or wrongly-signed transfer
✔ proof 3: client-signed transfer and hold -> post cycle through the relay
✔ proof 1c: hold -> void releases the reservation
✔ proof 2: the intent is persisted before the gateway fetch
✔ proof 4: hibernation — response lost, wake reconciles, retry stays exactly one transfer
✔ proof 4b: mutated retry under the same request ID is a hard error
✔ ruling: cross-server transfer is bilateral signed double-entry
(plus the 4 pre-existing routing tests) — 12/12 pass
$ npm run typecheck   # tsc --noEmit — clean
```

1. **Stub gateway** (`gateway.ts`, single process, the only ledger
   client) in front of the ledger stub: `POST /v1/transfers`,
   `POST /v1/holds` + `POST /v1/holds/{id}/post|void`,
   `GET /v1/balances/{account}` returning a gateway-signed snapshot
   (balance + pending + transfer watermark, Ed25519 signature by the
   server's token admin key, domain `currency-snapshot/v1`), and
   `GET /v1/transfers/{request_id}` for reconciliation lookups.
   Verification is server-scoped: a tampered snapshot fails, and a
   well-signed snapshot from server A does not verify as server B.
2. **Hosted-edition client** (`relay.ts`, `UserDORelay`): persists
   `{request ID, intent hash, state}` in its store **before** the
   gateway fetch (proven by a spying gateway that reads the store
   mid-fetch), caches the signed snapshot verbatim after verifying
   it, and reconciles by request-ID lookup on wake. Its only balance
   read is the cached signed snapshot, re-verified on every read —
   the hosted tier holds signed snapshots only, never ledger truth.
3. **Client-signed transfer + hold → post cycle through the relay:**
   the client signs the intent (domain `currency-intent/v1`); the
   gateway holds only the client's public key and rejects a
   wrong-key signature with 401. The relay/Worker/DO never holds the
   signing key — it forwards the client's signature.
4. **Hibernation idempotency:** the test wraps the gateway so the
   first submit lands the transfer but the response is lost; the
   intent stays `submitted`. A fresh relay over the same store (the
   wake) reconciles by lookup → `completed`. A retry with the same
   request ID and identical fields is deduplicated — exactly one
   transfer exists. A retry with a mutated amount under the same ID
   surfaces the ledger's `exists_with_different_fields` as a hard
   error (HTTP 409 at the gateway; `RelayError` at the relay), never
   a second transfer.

Cross-server (Drew's ruling): `executeCrossServerTransfer` records
the transfer on both servers — debit side against server A's
settlement account, credit side against server B's — each entry
signed by its own server's admin key (domain
`currency-double-entry/v1`). Neither side treats the other's ledger
as truth.

## Simplifications (explicit, per the issue's acceptance)

- **Ledger:** in-process stub behind the gateway interface (Drew's
  ruling explicitly allows this; do not block on TigerBeetle).
  Semantics mirror TigerBeetle where the proofs depend on them
  (unique transfer ID, `exists_with_different_fields`, two-phase
  pending/post/void, per-account watermark). Single currency,
  integer minor units, no conversions/corrections, no multi-replica
  cluster, no persistence.
- **UserDO:** the relay is a class over a two-method store
  interface; `SqlIntentStore` targets a DO's SQLite storage with the
  user-do.ts exec shape, tests use an in-memory store where
  "hibernation" is a fresh relay instance over the same store. The
  relay is not yet routed through `worker.ts`/`user-do.ts`, and
  nothing is deployed — consistent with this package's local-only
  posture (gate #2285).
- **Keys:** standalone generated Ed25519 keys (WebCrypto) instead of
  keys derived from the Iroh root key; the client public-key
  registry is a constructor map instead of the #2062 account
  factory's output; production auth is out of scope per the issue.
- **Gateway transport:** the gateway is exercised through its
  `fetch` handler in-process, not over a real socket; no registry
  UI, no central token provider (the longer-term solution per
  Drew's ruling).
