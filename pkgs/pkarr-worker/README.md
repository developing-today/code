# pkarr-worker

Our own door into [pkarr](https://github.com/pubky/pkarr) for the id
system (P7.2): a Cloudflare Worker that publishes and resolves signed
pkarr records over plain HTTP and DNS-over-HTTPS, instead of requiring
clients to speak the Mainline DHT protocol. Live at
<https://pkarr.developing-today.workers.dev>.

Design: `doc/2026-10-14T10-18-19Z_design_p7_2_cloudflare_workers.md`.
Deployment standard: `doc/2026-10-31T23-00-00Z_design_cloudflare_worker_deployment_standard.md`
(issue #1828); this package is its pilot (issue #1829).

## What it does

| Route                            | Behaviour                                                                                                                                                                                                                                                                                                               |
| -------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `PUT /pkarr/<z32>`               | Publish a signed pkarr relay payload (`signature ‖ timestamp_micros_be ‖ dns_packet`, ≤ 1072 bytes). The signature is verified against the key in the path; the packet must be a well-formed DNS message ≤ 1000 bytes. Newest timestamp wins; a valid but stale packet still gets `204` (iroh clients accept only 2xx). |
| `GET /pkarr/<z32>`               | Fetch the current payload verbatim (`application/x-pkarr-signed-packet`), or `404`.                                                                                                                                                                                                                                     |
| `POST /dns-query`                | DNS-over-HTTPS (RFC 8484) for `_iroh.<z32>.<ORIGIN>` TXT lookups. Out-of-zone names get `REFUSED`; in-zone non-pkarr names and unpublished keys get NODATA.                                                                                                                                                             |
| `GET /dns-query?dns=<base64url>` | The RFC 8484 GET form of the same endpoint.                                                                                                                                                                                                                                                                             |
| `GET /`                          | Plain-text statement of scope.                                                                                                                                                                                                                                                                                          |
| `GET /healthz`                   | `ok`.                                                                                                                                                                                                                                                                                                                   |

It is a lookup service only: not a DHT node, not a general DNS server,
not a traffic relay. Storage is one SQLite-backed Durable Object per
public key (named by the key's z-base-32 form), which gives strongly
consistent newest-wins semantics per key. Records expire after
`RETENTION_SECONDS` (default 7 days) without a refresh, via the object's
alarm. Configured values must be positive whole base-10 seconds (digits
only); missing, malformed, non-positive, or unsupported values fall back
to seven days. Per-IP rate limits guard all three routes. Clients verify
signatures themselves; the Worker also verifies before storing, but it
is trusted for availability, not integrity.

## Layout

- `src/pkarr.ts` — protocol primitives: z-base-32 codec, relay payload
  framing, Ed25519 verification (WebCrypto), strict bounded DNS wire
  parsing, DoH response building.
- `src/index.ts` — the Worker fetch handler and the `PkarrStore`
  Durable Object.
- `wrangler.jsonc` — config for local dev and the test suite.
- `cloudflare.config.ts` — the same Worker in cf's config format; this
  is the deployment source of truth. Keep the two in step.
- `test/` — vitest suite (`@cloudflare/vitest-pool-workers`) covering
  the codec, publish/resolve, signature and bounds checks, newest-wins,
  and the DoH paths. Test payloads are signed with Node's Ed25519 so the
  Worker's WebCrypto verification is anchored to an independent
  implementation.

## Develop

```sh
npm ci
npm test          # vitest run (workers pool)
npm run typecheck # tsc --noEmit
npm run dev       # wrangler dev
```

## Deploy

Deploys go through the flake entrypoint — never `wrangler deploy` or
`cf` by hand:

```sh
just deploy-pkarr              # = nix run .#pkarr-deploy
just deploy-pkarr --dry-run    # build + validate only
```

The entrypoint (`deploy.nix` → `deploy.sh` → `deploy.mjs`) gathers
secrets at deploy time — the Cloudflare account id from the SOPS file
`secrets/sops/common/cloudflare.yaml`, the API token from Bitwarden by
name (`cloudflare.api-token-day-to-day`, override with
`PKARR_CF_TOKEN_SECRET`) — so no secret ever lands in the repo or the
Nix store. SOPS decryption needs the age key, so deploys run on a
machine that has it (the laptop), not in the sandbox.

The deploy itself uses the **cf versions flow**:

1. `cf workers versions create` uploads a new version, tagged
   `git-<sha12>` with the full git SHA in the message. Nothing goes
   live at this step.
2. A REST pre-check confirms the version is in the account's
   `?deployable=true` set.
3. `cf workers deployments create` puts that version at 100%.
4. Before reporting success, the deployer confirms the newest deployment
   serves that version at 100% and `GET /healthz` returns HTTP success with
   the exact body `ok`. A failed check exits nonzero; it does not roll back
   automatically.

First deploy only: `cf workers versions create` cannot create a
brand-new Worker (it refuses and points at the deploy command), so
when the Worker does not exist yet the script bootstraps with
`cf deploy` — same tag and message — which creates the first version
and its deployment together. Every later deploy uses the versions
flow above.

### Secrets and rotation

`CLOUDFLARE_ACCOUNT_ID` comes from the encrypted
`secrets/sops/common/cloudflare.yaml`; update it with the SOPS editor and
commit only the encrypted file. The Cloudflare API token is read at deploy
time from Bitwarden Secrets Manager under
`cloudflare.api-token-day-to-day` (or the key named by
`PKARR_CF_TOKEN_SECRET`). The bootstrap token used to read BWS is itself
loaded by `lib/bitwarden-env.sh` from the encrypted common Bitwarden SOPS
file. Secret values stay in SOPS/BWS and process memory: do not put them in
Git, command arguments, terminal output, or the Nix store.

For onboarding, provision a least-privilege Cloudflare API token, add it to
BWS under the expected key, and run the normal laptop deploy entrypoint.
For rotation without a gap, create a replacement token and store it under a
temporary BWS key, then run:

```sh
PKARR_CF_TOKEN_SECRET=cloudflare.api-token-day-to-day-next just deploy-pkarr
```

After the version and health checks pass, update the stable BWS key to the
replacement value and revoke the old Cloudflare token. If verification fails,
inspect the latest deployment before revoking the old credential. The API
token authenticates the deployer; it is not part of a Worker version, so
rolling back Worker code does not restore an old API token.

### Failure and resource removal

An uploaded version is not live until a deployment is created. If upload
fails, the existing deployment remains the intended serving version; a
failed or ambiguous deployment request must be checked with
`cf workers deployments list --worker pkarr` because the API may have
applied a request even when the client reports an error. A failed post-deploy
version or health check also exits nonzero after the deployment request may
have taken effect. Inspect deployments and `/healthz`; if needed, roll back
to a known-good version with `just deploy-pkarr --deploy-version <id>`.
There is no automatic rollback.

This version deployer is not a resource reconciler: removing a binding or
resource from config does not delete the deployed Worker, routes, triggers,
or storage. Teardown is a separate, reviewed manual operation. In
particular, never infer deletion of data-bearing KV, R2, D1, or Durable
Object resources from their absence in config; preserve the data-bearing
resource deletion guard for future deployables. Version rollback restores
Worker code/configuration only, not stored data or external resources.

### Rollback

Versions and deployments are separate objects; rollback is a new
deployment pointing at an older version id:

```sh
just deploy-pkarr --deploy-version <version-id>
```

List versions with `cf workers versions list --worker-id pkarr` and
deployments with `cf workers deployments list --worker pkarr` (both
need the same env `deploy.sh` sets up).
