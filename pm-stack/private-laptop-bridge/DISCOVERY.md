# Private staging discovery and smoke approval gate

Read-only checkpoint: 2026-11-22. No Cloudflare resource, secret, laptop,
network setting or access grant changed. No AMD or proposed-host probe ran.

## Verified metadata and limits of inference

- Historical Wrangler verification (2026-10-21) mentions a Developing.today
  account, but is insufficient to establish current account ownership or its
  identifier. No current Cloudflare One Gateway configuration was available
  through connected tools or inspected tracked configuration.
- `pm-stack/console/scripts/expose.py` configures `amd-6-pm-stack`,
  `pm.security.cab`, and an owner default; account and Access
  organization are discovered dynamically. These defaults do not verify current
  ownership, Gateway enrollment or a suitable caller runner.
- Existing `lab-cluster` and `sandbox-link` tunnels use public hostnames. They
  are not a private Gateway ingress substitute. `home/common/default.nix`'s
  Gateway reference is AI Gateway, not Cloudflare One evidence. Referenced local
  account files and secret values were not read.
- `nixos/users/dot-ssh/default.nix` defines Aster's `dot-ssh`, UID 1339, no sudo,
  with empty authorized keys. This is not evidence of an installed runner.
- This Work runtime's provider configuration reports no outbound identities or
  secret bindings. Its local proxy's read-only status at
  **2026-11-22T12:07:24Z** reports Tailscale configured, state `Running`,
  `routing_ready: true`, and 301 accepted routes. Route contents and DNS data were
  not printed. Tailscale lives in the managed proxy; missing local VPN binaries
  or interfaces does not disprove it. Neither proxy readiness nor Tailscale
  establishes a Cloudflare Gateway on-ramp or target reachability.

**No existing account/Gateway/runner can yet be selected as verified suitable.**
No live probe was authorized, so it would be incorrect to claim that the private
host is demonstrably unreachable. The verified blocker is missing Gateway
on-ramp evidence and exact account/policy ownership metadata, not a measured
network failure. Do not install WARP/Tailscale or grant Aster access to resolve it.

Next safe route: owner selects an already enrolled, approved Gateway runner and
supplies read-only full account ID, Gateway organization, existing on-ramp/TLS
configuration, policy ordering, runner ownership and remaining allowance evidence.
Use that runner for the narrow smoke only. If Aster's managed proxy cannot use
that on-ramp, an owner-operated caller on the approved runner is the first pilot
route; native Codex may operate it only under separate authorization. A future
fixed-operation relay reachable through existing Tailscale would require its own
reviewed authentication/scopes and approval. No generic relay or proxy is proposed
or implemented here. A successful owner-run smoke alone does not make Aster ready.

## Separate smoke implementation

`smoke.mjs` exports `GatewayMiddleware` for trusted Gateway RPC and denies ordinary
HTTP. `wrangler.smoke.jsonc` is disabled with blank IDs and `.invalid` hostname;
no public routes, workers.dev, preview URLs, DO, secrets, storage, service bindings,
observability or upstream are configured. It accepts only `GET /smoke/v1` at the
pinned HTTPS host, from the pinned Gateway account/policy context, during an
explicit maximum ten-minute window. Authorization/cookie/upgrade headers and
bodies are denied; response is the constant `private-bridge-smoke-v1`.
It neither invokes identity/posture getters nor logs requests. No Fleet or laptop
operation is reachable. The required entrypoint and no-public-route deployment
pattern follow [Cloudflare Forward documentation](https://developers.cloudflare.com/cloudflare-one/traffic-controls/integrate-workers/forward-traffic/). Platform/Gateway metadata logging is separate and must
be inspected before approval; it is not disabled by this Worker setting.

This is genuinely **bridge-credential-free**: no new credentials, registry,
signing keys, API tokens, enrollment, secret reads or secret transfer. Existing
Gateway transport authentication may still be necessary; it is not a promise of
an entirely unauthenticated private network. Deployment uses the owner's existing
approved administrative path outside the smoke request.

Offline reproduction (Node 24, from this directory):

```sh
npm ci
node --test smoke.test.mjs
WRANGLER_SEND_METRICS=false npx wrangler deploy --dry-run --config wrangler.smoke.jsonc --outdir .wrangler/smoke-dry-run
npm run rehearse
```

The Miniflare harness synthesizes Gateway context using a test-only service
binding. Never deploy that harness or give any other Worker such a binding.
It proves handler success/denial, not actual Gateway routing or private DNS.

## Exact approval request to finalize after metadata is verified

**Not executable approval yet:** replace every bracketed field with inspected
metadata and attach the resolved config/policy diff before presenting to Drew.
No implicit authority to perform any step below is claimed.

> Approve a single ten-minute routing smoke in Cloudflare account [full ID,
> verified owner], using existing Gateway organization [ID] and already approved
> runner [exact identity/runtime/on-ramp]. Deploy only Worker
> `private-laptop-bridge-smoke` from [reviewed commit], without public routes,
> workers.dev, preview URLs, bindings, storage or secrets. Create temporary
> Gateway-only DNS policy `private-laptop-bridge-smoke-dns` for
> `private-bridge-smoke.security.cab` -> `192.0.2.1` (TTL 60, no public DNS or
> origin), and HTTP Forward policy `private-laptop-bridge-smoke-forward`, scoped
> to that host, GET `/smoke/v1`, and the existing approved runner identity.
> Use [exact policy ordering and IDs], [START_MS, END_MS UTC], no new enrollment,
> certificates, TLS trust or access grants. Perform at most twelve manual
> requests: normal fixed response, wrong path/method, and controlled no-Forward,
> expired-window and off-on-ramp denials using [already authorized runtimes].
> No Authorization/Cookie header, client secret, Fleet payload or AMD request is
> sent. Record only status, fixed response, time and redacted routing outcome.
> Then disable and delete exactly these two temporary policies and Worker;
> verify removal and retain the existing fallback. Approve this cleanup in the
> same action set. Stop on unexpected public resolution, successful bypass,
> resource mismatch or any need for new permissions. Maximum incremental spend
> $0, no upgrade; require existing allowance headroom for twelve requests.

`192.0.2.1` is a proposed non-serving documentation sink, not proof of routing.
Before the run, inspect public DNS/wildcards, Workers routes and Gateway policy
ordering using authorized read-only metadata. A Forward miss must not reach any
origin. The HTTP Block-before-Forward ordering described in STAGING.md still
applies. Do not test CONNECT/WebSocket by creating a tunnel; verify rejection
policy and absent origin. If DNS sink composition fails, stop rather than adding
public ingress. Off-on-ramp denial needs a separately approved runtime and cannot
be inferred from direct Worker denial in Miniflare.

No recurring polling or persistent access is introduced. The Worker expiration
bounds response availability, not resource lifetime; explicit approved cleanup
is mandatory even after failure. No budget reservation or current-plan assertion
is made without account metadata. The full bridge pilot and AMD installation
remain separate later approvals under STAGING.md.

## Review checkpoint

At `dab58829a6d17498da3cae7e9f9456e85f6f6b64`, GitHub reported zero check runs,
zero commit statuses, zero Actions runs, zero reviews and zero inline review
comments. The combined status `pending` meant absent statuses, not running CI.
The draft was mergeable/clean. CodeRabbit's sole comment skipped draft review.
There were no external findings to resolve. The independent local review found
that the original signed bridge was unsuitable for a credential-free smoke;
this separate artifact addresses that concrete gap. See VERIFICATION.md for
final-head offline test evidence; no CI pass or live reachability is claimed.

## GitHub Actions release boundaries and account preflight

Official Cloudflare docs now confirm the compatibility pair. The November 17
release-candidate changelog says `cf@1.0.0-rc.1` switches Build Output to v1 and
requires Wrangler 4.163.0 or later for Wrangler-built projects. The November 18
project docs say `cf build` writes `.cloudflare/output/v1/`, `cf workers versions
create --prebuilt` consumes it, and `--prebuilt` skips automatic configuration.
The lockfile pins `cf@1.0.0-rc.1`, `@cloudflare/build-output-utils@1.0.0-rc.1`,
and its build producer `wrangler@4.166.0`. Node is pinned at 24.22.0 to meet the
RC's declared engine range. The prebuilt dry-run is run after a separate local
build; this prevents CI from triggering unreviewed auto-configuration.

The manual workflow `.github/workflows/private-laptop-bridge-release.yml` runs only
for this repository's `main` ref, checks out the event SHA, has `contents: read`
only, serializes per Worker, and pins Actions, Node, cf, Wrangler and lockfile
versions. The `smoke` target is `private-laptop-bridge-smoke` under GitHub
Environment `cloudflare-staging-smoke`. The real bridge target is
`private-laptop-bridge-staging` under `cloudflare-staging`. The bridge
environment uses `CF_BRIDGE_ACCOUNT_ID`, `CF_BRIDGE_API_TOKEN`, and
`CF_BRIDGE_OWNER_PROOF_JSON`; the smoke environment uses the corresponding
`CF_SMOKE_*` names. These are distinct environment-only secrets with no
repository-secret fallback. Each token must be independently account-owned and
scoped as **Workers Editor for that one individual existing Worker**. It must
not be scoped to the Workers product or whole account. No secret values are
configured or read by this change. The owner proof stays private in the
matching protected environment secret; only its digest is bound to a candidate.
The tracked `staging-preflight.json` is a redacted, unverified public contract
fixture. It contains no reviewer email, account ID, or DO namespace ID. The
private proof does not require a reviewer email; `evidence_ref` must be opaque
and non-PII. GitHub environments must have required reviewers and reject
untrusted refs before secrets are provisioned.

Each upload path runs named runtime tests, `cf build` (bridge build uses mode
`bridge`), a source/config binding equivalence test, then
`cf workers versions create --prebuilt --dry-run` and the actual immutable
version upload. It verifies the uploaded UUID appears in the deployable set.
Promotion is a distinct manual invocation with the exact UUID; it repeats the
deployable check, captures the create response deployment ID, and retries
read-back at most five times to verify that exact deployment is latest and
serves 100%. Immediately before promotion, the workflow requires the candidate
SHA to equal the freshly fetched protected `main` head and revalidates the
private owner proof. Recovery is
an explicit promotion of a recorded known-good retained UUID. Fix-forward uploads
a new version and separately promotes it. These actions do not apply Gateway
routes, Workers routes/domains, DNS, TLS settings or access policies.

**Target existence is not verified.** The tracked Wrangler configs and `cf`
config give only the proposed names `private-laptop-bridge-smoke` and
`private-laptop-bridge-staging`; they prove neither is present in Cloudflare.
No current account ID or live metadata was obtained. The first workflow step calls `cf workers get` and fails
closed if the selected exact target is absent; it never bootstraps, creates
account resources or widens scopes. Before adding secrets, an authorized owner
must independently confirm full account ID/ownership and existence of the exact
Worker named by that GitHub Environment, then verify the resulting token cannot
read or edit another Worker. No target was probed in this work session.

**Smoke Worker bootstrap is newly authorized as an empty metadata-only init,**
separate from the Versions release workflow. Cloudflare's official API exposes
`POST /accounts/{account_id}/workers/workers` to create a Worker metadata
object, while its script `PUT /workers/scripts/{script_name}` creates and
deploys a version to 100% immediately. The latter is not a safe substitute for
the requested empty init. The metadata endpoint supports disabling the
`workers.dev` subdomain, previews, logs and observability in its creation
payload, and returns a null `deployed_on` for a Worker that has never been
deployed. Sources: [Create Worker API](https://developers.cloudflare.com/api/resources/workers/subresources/beta/subresources/workers/methods/create/),
[Script Upload API](https://developers.cloudflare.com/api/resources/workers/subresources/scripts/methods/update/),
[Workers deployment behavior](https://developers.cloudflare.com/workers/platform/infrastructure-as-code/).

**The separate smoke init workflow is implemented but not dispatched.**
[`private-laptop-bridge-smoke-init.yml`](../../.github/workflows/private-laptop-bridge-smoke-init.yml)
has no inputs, accepts only the protected default-branch `workflow_dispatch`
context, checks out the dispatch SHA (no caller-selected ref), and hard-codes
`private-laptop-bridge-smoke`. It uses the dedicated `cloudflare-smoke-bootstrap`
environment, not the routine release environment. The pinned Node script uses
only built-in APIs. It checks account ID suffix `cbfa`, inventories all account
zones and routes, and is idempotent: a safe existing empty Worker is verified
without writing; an absent Worker gets one metadata-only Beta Create Worker
POST; any mismatch fails without repair or retry. Read-only verification then
requires disabled `workers.dev` and preview URLs, null `deployed_on`, no
versions, no deployments, no custom domains and no account route targeting the
name. The only write is that one POST; this workflow contains no code or
Versions upload step.

The environment must be configured and protected on `main` before a dispatch:
required reviewer approval, self-review disabled, and environment-only secrets
`CF_SMOKE_INIT_ACCOUNT_ID`, `CF_SMOKE_INIT_WRITE_TOKEN`, and
`CF_SMOKE_INIT_READ_TOKEN`. The write token must be short-lived and limited to
Cloudflare `Workers Scripts Write` (the Create Worker endpoint's minimum
documented write permission); it is separate from the routine
`cloudflare-staging-smoke` Individual Workers Editor credential. The read-only
token must cover Worker metadata/version/deployment read, `Zone Read`, and
`Workers Metadata Read-Only` for complete route enumeration. Missing or
unauthorized credentials fail before any create. No GitHub environment or
secret was read, created, or changed here. GitHub requires the workflow to be
on the default branch before `workflow_dispatch` can run ([GitHub docs](https://docs.github.com/actions/managing-workflow-runs/manually-running-a-workflow));
the draft PR cannot yet run it. No merge or dispatch was attempted.

The route check uses Cloudflare's paginated account zone list and zone-scoped
Worker Routes API, failing closed if any page or route read is unavailable
([route API](https://developers.cloudflare.com/api/resources/workers/subresources/routes/methods/list/),
[zone API](https://developers.cloudflare.com/api/resources/zones/methods/list/)).
The [Sep. 2025 New Workers API changelog](https://developers.cloudflare.com/changelog/post/2025-09-03-new-workers-api/)
explicitly demonstrates metadata-only Worker creation, Beta Worker version
creation by returned ID, and the legacy `/scripts/{name}/deployments` API using
the returned Worker name. This supports resource identity across the new Worker
and legacy deployment APIs. It does not document the pinned `cf` Scripts
Versions upload cold-start path. That compatibility remains a separate check:
`cf` can be run only after the init action has successfully completed, and this
work did not dispatch either action or access Cloudflare.

The real bridge has an additional Versions API constraint. Its Build Output
declares `PrivateLaptopBridge` as a new SQLite Durable Object export and binds
`BRIDGE` to that export. The pinned upload helper rejects pending Durable Object
migrations because Versions upload cannot apply them. It also handles
`ACTOR_BINDING_DEPENDS_ON_EXPORT_CODE` with the explicit explanation that
declarative exports are reconciled only when a version is deployed, so a
namespace for an unprovisioned export cannot exist at upload time. Therefore
the bridge can use this Versions-only release path only if an owner has already
verified that the exact existing `private-laptop-bridge-staging` Worker owns a
created SQLite `PrivateLaptopBridge` namespace and has completed the `v1`
migration, with no pending migration. The `BRIDGE` binding namespace ID must
match that same Worker namespace. The smoke Worker has no Durable Object and
still must already exist.

`staging-preflight.json` is currently intentionally unverified and empty. CI
checks only validate a future owner-supplied read-only snapshot; the new contract
tests reject a missing Worker, mismatched namespace/class/Worker, an uncreated
export, a different migration tag, and pending migrations. An offline `cf
workers versions create --prebuilt --dry-run` does not exercise these remote
checks: the pinned helper skips API preflight on dry-run, and dry-run builds can
prove only the local Build Output. No live Worker, namespace, migration, route,
Gateway, or Cloudflare account was queried in this session. If the exact target
or completed namespace state is absent or cannot be proven, this workflow has
no allowed path forward; stop for Drew to decide whether the release scope
should change.

Do not create or store a Workers-product/whole-account token in GitHub. If the
required targets and bridge namespace are already in place and independently
verified, routine CI uses separate account-owned **Individual Workers → Editor**
tokens, each restricted to one exact existing Worker. The smoke Worker and DO
bridge use separate GitHub environments/tokens. The bridge environment cannot
modify Gateway policies; those use a separate approval and administrative
path.

Offline results on the current follow-up commit (recorded in the PR): Node 24.22.0 + cf 1.0.0-rc.1 + Wrangler
4.166.0; both smoke and `--mode bridge` pass `cf build` and the corresponding
`cf workers versions create --prebuilt --dry-run`. The bridge dry-run reports
only the DO namespace binding `BRIDGE -> PrivateLaptopBridge`, disabled
`ENABLED=false`, placeholder Gateway identifiers and empty registry; it creates
nothing. `deployment-config.test.mjs` compares the local/staging Wrangler DO
class and SQLite migration declarations against the cf config. Gateway runtime
unit tests and smoke tests pass. These results prove local Build Output and
configuration consistency, not Cloudflare account reachability, live target
existence, deployment, Gateway routing or laptop readiness.


Official references: [cf 1.0.0-rc.1 changelog](https://developers.cloudflare.com/changelog/post/2026-11-17-release-candidate/), [cf project builds and prebuilt uploads](https://developers.cloudflare.com/cf/projects/), and [Workers individual resource Editor permissions](https://developers.cloudflare.com/workers/authorization/workers/).

Pinned implementation references: [cf 1.0.0-rc.1 package](https://www.npmjs.com/package/cf/v/1.0.0-rc.1), [its version-upload JSON shape](https://github.com/cloudflare/workers-sdk/blob/%40cloudflare/deploy-helpers%400.27.2/packages/deploy-helpers/src/deploy/versions-upload.ts#L98-L116), [output-file-only behavior](https://github.com/cloudflare/workers-sdk/blob/%40cloudflare/deploy-helpers%400.27.2/packages/workers-utils/src/output.ts#L7-L26), the [missing-Worker Versions guard](https://github.com/cloudflare/workers-sdk/blob/%40cloudflare/deploy-helpers%400.27.2/packages/deploy-helpers/src/deploy/helpers/validate-worker-props.ts#L290-L301), [dry-run API-check skip](https://github.com/cloudflare/workers-sdk/blob/%40cloudflare/deploy-helpers%400.27.2/packages/deploy-helpers/src/deploy/helpers/validate-worker-props.ts#L135-L159), [pending-migration rejection](https://github.com/cloudflare/workers-sdk/blob/%40cloudflare/deploy-helpers%400.27.2/packages/deploy-helpers/src/deploy/versions-upload.ts#L171-L193), and [unprovisioned DO export rejection](https://github.com/cloudflare/workers-sdk/blob/%40cloudflare/deploy-helpers%400.27.2/packages/deploy-helpers/src/deploy/versions-upload.ts#L378-L396). Cloudflare's [deployment management docs](https://developers.cloudflare.com/workers/versions-and-deployments/deployment-management/) separately specify first-upload and DO lifecycle constraints.
