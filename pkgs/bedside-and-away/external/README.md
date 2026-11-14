# External Turso Worker adapter

Source and operator contract. Check the actual release/version before making any deployment claim.

## Topology

A tailnet-only local gateway serves the existing frontend, verifies trusted Tailscale identity and same-origin browser writes, and signs API requests. This Worker hosts only `/api/*` and reuses `../worker/api.mjs`. One dedicated Turso database stores application data. No Atlas or Snowflake integration.

`turso.mjs` provides a D1-shaped prepare/bind/first/all/run/batch adapter using Hrana HTTP `/v2/pipeline`. `auth.mjs` verifies signatures. `worker.mjs` consumes persistent one-use nonces, strips all caller identity headers, and reconstructs trusted API requests. `migrate.mjs` is an explicit operator schema tool, not a request handler. `test.mjs` is local validation, not a live service test.

## Signing v1

Gateway must reject browser identity headers and enforce same-origin POSTs BEFORE signing. Use a stable trusted tailnet identity, not a device UUID. Mapping an existing ChatGPT Site account requires verified account-link approval, never display-name matching.

Headers: `x-bedaway-time` (epoch seconds, exactly 10 digits, ±60 seconds), `x-bedaway-nonce` (fresh lowercase UUID), `x-bedaway-actor` (base64url UTF-8 JSON `{id,email}`), `x-bedaway-origin` (exact GATEWAY_ORIGIN), and `x-bedaway-signature` (lowercase hex HMAC-SHA256).

Canonical UTF-8 string is the following array joined with newline, no trailing newline:

`['bedaway-gateway-v1', time, nonce, METHOD, pathname + search, actorBase64url, gatewayOrigin, lowercaseHexSHA256(rawBody)]`

Actor IDs must match `[A-Za-z0-9_-]{1,100}`; email is a trusted verified claim. The signature binds the exact raw body, path and query. Each valid request consumes its nonce once in Turso. Missing nonce schema fails closed. Retry with a fresh signature/nonce while preserving the logical event ID. Do not blindly retry uncertain creates.

No CORS or public data path. The Worker public hostname only returns an authenticated-gateway rejection without a signature. Gateway serves static assets locally.

## Deployment preparation

1. Review/create the dedicated database in the selected account; confirm paid costs if required. Existing credentials are used only in approved process/provider secret storage.
2. Supply TURSO_URL and TURSO_AUTH_TOKEN through the operator environment, not command text. Run `node external/migrate.mjs` for plan/hashes, then `--apply` only at the verified destination. This applies schema and immutable migration hashes only, not user data. Ordinary API calls never create tables.
3. Configure Worker secret bindings TURSO_AUTH_TOKEN and GATEWAY_HMAC_KEY with the provider secret mechanism. Generate at least 256 random bits for HMAC; minimum string length alone is not an entropy guarantee. Secrets must not appear in Git, frontend, output, arguments or state snapshots.
4. Set non-secret URL/origin and optional Worker name through the approved environment. Never include a token in the database URL. Use native cf on this external target, not the separate Sites installation.
5. Verify live unsigned rejection, one-use nonce rejection, signed reads/writes, personal/shared isolation, conflict handling and failed-batch rollback against the real service. Local tests do not verify service compatibility.
6. Gateway remains tailnet-only, without Funnel, public frontend, broad ACL or identity-header bypass. Network security changes require applicable confirmation.
7. Preserve current Site until authorized export, verified account mapping, import, counts/hashes and cutover are complete. Schema tooling does not migrate user records or pass through ChatGPT login.

No Amazon credential collection or automatic credential refresh is implemented. Rotation remains an explicit operational action.

## Atomicity

Pipeline errors do not stop later statements automatically. Each adapter batch is wrapped in BEGIN IMMEDIATE; each statement is conditioned on the previous success, COMMIT on final success, and ROLLBACK unless COMMIT succeeded. User results are returned only after a successful commit. HTTP failure may still be an unknown committed outcome, so no blind retry is used.

Primary protocol reference: https://github.com/tursodatabase/turso/blob/main/serverless/PROTOCOL.md

## Tests

`node external/test.mjs` uses native SQLite with a Hrana-shaped fetch harness. It checks encoding, atomic rollback, skipped writes, signed authentication, stale/tampered/unsigned rejection, replay prevention and forged identity stripping. It is not evidence of deployment, live Turso compatibility or migration completeness.

## Native cf deployment

Use the native `cf` binary, not Wrangler. `node external/cf-build.mjs` writes Cloudflare's prebuilt v0 output from the staged source, using non-secret TURSO_URL and GATEWAY_ORIGIN. Run `cf deploy --prebuilt --dry-run --provision=false` before any upload. The Worker route must reject all unsigned requests. Never replace an existing unrelated Worker or Tailscale listener; require exact project and host ownership before enabling the app listener. Verify against a real authenticated Tailnet client and confirm the old Site tables are still empty before cutover. The only browser-local state on the old app is device UUID and remembered store selection; it is not an account key or private draft storage. The new origin intentionally creates a new browser ID.
