# Offline verification checkpoints — 2026-11-22

## Follow-up review and private staging candidate

The next revision adds `npm run rehearse` (all bridge, Gateway, Aster, freshness,
publisher and bounded-refresh tests) and `npm run check:staging` (Wrangler bundle
dry run, never deployment). The PR records the final tested commit and results;
the original checkpoint below remains historical evidence for `0ba3dd75a`.

Concrete findings fixed in this follow-up:

* Raw malformed broker counters could normalize to zero and look idle. Bridge
  publication now requires explicit validated broker provenance and fresh,
  error-free observations. Eight integration cases prove idle versus unavailable,
  exact TTL boundaries, missing data, stale completed results and retained-file
  behavior after failed publication.
* Awaiting cancellation of a denied response could hang the device client.
  Cleanup is nonblocking; pending/rejecting cancellation regression tests pass.
* Slight clock differences rejected legitimate short leases. `serverNow` and
  conservative request-start-relative deadlines tolerate valid offsets while
  charging transport time against the lease. Regression coverage includes
  ±500 ms, ±29 seconds, delayed responses and inconsistent advertised lifetimes.
* Idle polling consumed the entire per-minute device quota. Two-second polling
  reserves capacity for results.

The disabled refresh pilot has eight deterministic scheduler tests proving
strict opt-in, cache-safe cadence, no overlap, monotonic one-hour stop, cancellation
and no rearming after a hung/late collector. The real Gateway RPC harness verifies
trusted context/identity/posture gating, direct HTTP denial, and zero upstream
continuation calls. Aster's four-command CLI has seven tests including actual
CLI → signed SDK → workerd/SQLite → laptop-client roundtrips.

Wrangler 4.166.0 successfully bundled the private staging configuration in a
local dry run. Bun 1.4.5 ran the existing queue suite: 18 passed, zero failed.
These checks do not prove real Gateway routing or AMD service behavior. The
DNS sink/on-ramp composition, exact account/device inventory and certificate
trust remain mandatory approval-gated live tests in [STAGING.md](STAGING.md).

## Original checkpoint

Implementation based on developing-today/code `ad4e69670`, refreshed by merge to
`7bfcaff23` before review. Executed as non-root UID 1000 in an isolated cloud
worktree, not on Drew's laptop. Node v24.19.0; Miniflare 4.20260730.0;
compatibility date 2026-07-30; SQLite Durable Object storage.

| Check | Result |
| --- | --- |
| Clean `npm ci` using checked-in lockfile | Passed |
| `npm test` from `pm-stack/private-laptop-bridge` | 39 passed, 0 failed (~33 s) |
| Control plane | 12 passing tests |
| Laptop client/adapters | 15 passing tests |
| Model-based authorization | 1 passing test, 50 seeds × 40 transitions |
| Real workerd acceptance + caller SDK | 11 passing tests |
| `node --test pm-stack/aggregator/test/bridge-snapshot.test.ts` | 7 passed, 0 failed |
| `git diff --check` | Passed |

The generated model achieved 150 accepted calls, 62 leases, 16 completions,
59 invalid/stale-lease denials, 72 replay denials, 4 completed reads and
231 non-owner read denials. The coverage assertions prevent vacuous success.

Real-runtime evidence includes all four named operations through the actual
producer, fixed-file adapter, client, Worker, SQLite object and caller SDK. A
synthetic Unix socket server stands in for the queue broker liveness endpoint;
its assertions permit only GET /healthz with no broker token. Synthetic keypairs
exist only in memory. Temporary snapshots/databases are removed after tests;
Worker outbound I/O is explicitly rejected by the harness. Restart tests use the
same temporary persistent SQLite store and reject an already accepted nonce.
The disconnect test waits for the actual 15-second lease boundary, obtains a
new lease, refuses the stale completion and accepts the new completion.

Review found and fixed an inherited-object identity-name edge case, explicit
lease deadline/ack validation, queue freshness at completion/retrieval, and audit
retention/alarm scheduling. The final independent security review found no
blocking issue within this offline scope. This is engineering test evidence,
not a security certification.

## Not verified / not performed

* No Cloudflare deployment, routing, public management endpoint or identity
  enrollment; no live key or secret creation/transfer.
* No AMD connection, NixOS service installation/network/security change, or
  Desktop Commander/native Codex modification.
* No actual Aster connector call, private ingress validation, live queue parity,
  observed workflow inventory, production revocation propagation or cutover.
* No full aggregator Bun suite/typecheck or real systemd sandbox validation.
* No automatic PR CI claim. Current repo policy disables it; focused local
  offline execution was explicitly requested for this task.

Follow the README's rollout/rollback gates. Keep the PR a draft and keep the
fallback until owner-approved end-to-end laptop acceptance is complete.

## Routing-smoke follow-up (2026-11-22)

The separate `smoke.mjs` has three real Miniflare RPC acceptance tests: constant
success without credentials/storage, ordinary HTTP and route/context/credential
header denials, and disabled/invalid/expired/future/overlong-window denials.
The test oracle is the fixed response or exact denial status. It never calls
AMD, Cloudflare or an origin. Independent review corrected the exported entrypoint
to Cloudflare's required `GatewayMiddleware` and added rejection of accidental
bridge credential headers. `wrangler.smoke.jsonc` dry run bundles 1.96 KiB
(0.98 KiB gzip), with only disabled non-secret configuration variables.

The final commit is tested again after committing; exact SHA and command results
are recorded in the draft PR checkpoint to avoid a self-referential commit hash.
Read-only discovery and the unapproved, metadata-blocked ten-minute smoke packet
are in DISCOVERY.md. Proxy routing readiness is not private-host reachability.


## cf release-candidate toolchain follow-up

Offline verification used Node 24.22.0, `cf@1.0.0-rc.1`,
`@cloudflare/build-output-utils@1.0.0-rc.1`, and Wrangler 4.166.0. Commands
ran in `pm-stack/private-laptop-bridge` with a writable XDG config directory:

```sh
cf build
cf workers versions create --worker private-laptop-bridge-smoke --tag offline-smoke --message offline-smoke --prebuilt --dry-run
cf build --mode bridge
cf workers versions create --worker private-laptop-bridge-staging --tag offline-bridge --message offline-bridge --mode bridge --prebuilt --dry-run
node --test deployment-config.test.mjs smoke.test.mjs gateway.test.mjs
```

Both Builds completed and both prebuilt Version API dry-runs exited without
credentials or upload. The bridge dry-run displayed only the `BRIDGE`
`PrivateLaptopBridge` Durable Object binding and default-disabled placeholder
vars; bundle was 17.84 KiB (5.31 KiB gzip). Smoke was 1.96 KiB (0.98 KiB gzip).
Nine targeted tests passed (3 binding/release config, 3 Gateway RPC, 3 smoke).
The workflow YAML and all embedded shell blocks passed local parsing. This is
not evidence of target existence or a live Cloudflare deployment.

## Versions-only release and bridge namespace preflight

The lockfile pins `cf@1.0.0-rc.1`. Its installed package metadata records
`@cloudflare/deploy-helpers@0.27.2`, and its bundled Versions upload code
contains the missing-Worker rejection, the pending-DO-migration rejection, and
the API error handler for a binding to an export that has not yet been
provisioned. It states that declarative exports are reconciled when the version
is deployed. Cloudflare's [deployment management docs](https://developers.cloudflare.com/workers/versions-and-deployments/deployment-management/)
also require a deploy command for the first upload and for lifecycle-changing
DO exports/migrations. Current scope permits only Versions uploads, so the
workflow has no first-Worker bootstrap fallback and refuses absent Workers.

The pinned `cf workers versions create` record is written to
`WRANGLER_OUTPUT_FILE_PATH`, not stdout; the workflow sets that path, clears it
first, and parses the single `version-upload` JSON record. The parser requires
`type: "version-upload"`, schema `version: 1`, the exact `worker_name`, and a
server-returned UUID `version_id`. Its `worker_tag` field is the stable Worker
identity, not the source commit. The workflow verifies the full source SHA and
provenance digest on that exact UUID's version annotations in the exact Worker
version list.

The separate `Versions release proof` test accepts a synthetic snapshot only
when the bridge Worker already exists and its `BRIDGE` binding matches the same
Worker's created SQLite `PrivateLaptopBridge` namespace at completed migration
`v1`. It separately rejects absent Workers, namespace/class/Worker mismatches,
an uncreated export, a different migration tag, and pending migrations. The
smoke preflight requires its existing Worker but no DO. The tracked staging
proof remains `verified: false`; these tests validate the fail-closed gate and
do not assert live account state.

Owner evidence is supplied only through separate protected environment
secrets: `CF_BRIDGE_ACCOUNT_ID`, `CF_BRIDGE_API_TOKEN`,
`CF_BRIDGE_OWNER_PROOF_JSON` or their `CF_SMOKE_*` equivalents. No generic
repository secret fallback is used. The public fixture contains no reviewer
email or account/namespace identifiers; the private proof requires an opaque
evidence reference, not a person's email. Promotion re-parses and revalidates
the current private proof immediately before creating a deployment and requires
its digest to equal the candidate's protected proof digest.

Deployment readback binds to the deployment `result.id` returned by the create
response, checks its exact one-version 100% set, then retries the list readback
at most five times, two seconds apart. Success requires that same ID to be the
latest deployment and to carry the exact 100% version. Tests cover the
create-response identity and stale/newer/split deployment cases. This path was
contract-tested offline; no create request or live readback was sent.

After adding these tests, the focused local command
`node --test --test-reporter=spec release-contract.test.mjs deployment-config.test.mjs smoke.test.mjs gateway.test.mjs`
passed 21/21 under Node 24.22.0. Both `cf build` variants and both
`cf workers versions create --prebuilt --dry-run` variants were rerun; the
effective Build Output checks passed for the smoke and bridge targets. Dry-run
output confirms the bridge declares its `BRIDGE` Durable Object export and
disabled placeholders, but the pinned helper skips API checks during dry-run,
so this cannot validate missing-Worker or live namespace behavior. No upload,
deployment, promotion, account query, or secret use occurred. The local build
printed a proxy-environment warning; this should not be taken as proof of
network isolation.

The workflow YAML parsed with PyYAML and all embedded `run:` blocks passed
`bash -n` after substituting GitHub expressions. `actionlint` was unavailable.

### Empty smoke Worker init request (2026-11-23)

Drew authorized an empty Worker metadata init for only
`private-laptop-bridge-smoke`. Official Cloudflare docs distinguish
`POST /accounts/{account_id}/workers/workers` (metadata-only Worker creation)
from `PUT /accounts/{account_id}/workers/scripts/{script_name}` (script upload,
which implicitly creates and deploys a version). The metadata endpoint allows
the request to disable `subdomain.enabled`, `subdomain.previews_enabled`,
observability, and logpush. It returns `deployed_on: null` when the Worker has
never been deployed. Sources: [Create Worker API](https://developers.cloudflare.com/api/resources/workers/subresources/beta/subresources/workers/methods/create/),
[Upload Worker API](https://developers.cloudflare.com/api/resources/workers/subresources/scripts/methods/update/),
[Cloudflare's create/deploy API notes](https://developers.cloudflare.com/workers/platform/infrastructure-as-code/).

The init action and dependency-minimal verifier are implemented in the draft;
no init workflow or provider action was run. The write token required by the
official Create Worker endpoint is short-lived `Workers Scripts Write`,
separate from the routine per-Worker Editor credential. The dedicated protected
environment is `cloudflare-smoke-bootstrap` with environment-only secrets
`CF_SMOKE_INIT_ACCOUNT_ID`, `CF_SMOKE_INIT_WRITE_TOKEN`, and
`CF_SMOKE_INIT_READ_TOKEN`. The last token requires Worker metadata/version/
deployment read, `Zone Read`, and `Workers Metadata Read-Only` for the complete
route scan. The workflow checks required secrets and the account suffix before
network calls; the script uses the read-only token for every GET and the write
token only for its single metadata-create POST. The environment and credentials
were not read or changed. Because `workflow_dispatch` must be on the default
branch before it can run, this draft cannot dispatch it yet; no merge or
dispatch was attempted. No Cloudflare resource or setting was changed, and no
smoke version upload was attempted.

The script covers synthetic missing/existing target, exact target/account,
authorization failure, public/preview/domain exposure, route conflict,
create/readback identity mismatch, empty versus non-empty version/deployment
history, and missing credential behavior. Its only provider write path is the
one exact Beta `POST /workers/workers` call; it never updates, deletes, uploads
code, creates a version/deployment, or changes routes/settings. Cloudflare's
[Sep. 2025 New Workers API changelog](https://developers.cloudflare.com/changelog/post/2025-09-03-new-workers-api/)
supports creating the metadata Worker then using the returned name for the
legacy deployment API. The exact pinned `cf workers versions create` cold-start
path remains deliberately separate and unverified; no live or CLI upload was
attempted.

One further compatibility question remains unproven: the metadata creation
endpoint is in the Beta Workers API and its version path uses Worker ID
(`/workers/workers/{worker_id}/versions`); pinned `cf` uses the Scripts API
path keyed by `script_name` (`/workers/scripts/{script_name}/versions`). The
changelog demonstrates cross-surface identity for deployments but not the
pinned `cf` cold version upload. This remains unverified and is not inferred
from a synthetic API fixture.
