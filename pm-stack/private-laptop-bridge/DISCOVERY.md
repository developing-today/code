# Private staging discovery and smoke approval gate

Read-only checkpoint: 2026-11-22. No Cloudflare resource, secret, laptop,
network setting or access grant changed. No AMD or proposed-host probe ran.

## Verified metadata and limits of inference

- Historical Wrangler verification (2026-10-21),
  `doc/platform/toolchains/cloudflare-wrangler.md`, names
  `Developing.today.llc.domains.0@gmail.com's Account`, ID prefix `234b866b…`.
  No complete account ID or current Cloudflare One Gateway configuration was
  available through the connected tools or inspected tracked configuration.
- `pm-stack/console/scripts/expose.py` configures `amd-6-pm-stack`,
  `pm.security.cab`, and owner default `drewrypope@gmail.com`; account and Access
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
`private-laptop-bridge-staging` under `cloudflare-staging`. Both environments
use names `CF_ACCOUNT_ID` and `CF_API_TOKEN`; each token must be independently
account-owned and scoped as **Workers Editor for that one individual existing
Worker**. It must not be scoped to the Workers product or whole account. There
are no environment secrets configured by this change. GitHub environments must
have required reviewers and reject untrusted refs before secrets are provisioned.

Each upload path runs named runtime tests, `cf build` (bridge build uses mode
`bridge`), a source/config binding equivalence test, then
`cf workers versions create --prebuilt --dry-run` and the actual immutable
version upload. It verifies the uploaded UUID appears in the deployable set.
Promotion is a distinct manual invocation with the exact UUID; it repeats the
deployable check, creates a 100% deployment, and verifies read-back. Recovery is
an explicit promotion of a recorded known-good retained UUID. Fix-forward uploads
a new version and separately promotes it. These actions do not apply Gateway
routes, Workers routes/domains, DNS, TLS settings or access policies.

**Target existence is not verified.** The tracked Wrangler configs and `cf`
config give only the proposed names `private-laptop-bridge-smoke` and
`private-laptop-bridge-staging`; they prove neither is present in Cloudflare.
The only account evidence found is historical from 2026-10-21:
`Developing.today.llc.domains.0@gmail.com's Account`, partial ID `234b866b…` in
`doc/platform/toolchains/cloudflare-wrangler.md`. No full account ID or live
metadata was obtained. The first workflow step calls `cf workers get` and fails
closed if the selected exact target is absent; it never bootstraps, creates
account resources or widens scopes. Before adding secrets, an authorized owner
must independently confirm full account ID/ownership and existence of the exact
Worker named by that GitHub Environment, then verify the resulting token cannot
read or edit another Worker. No target was probed in this work session.

If a Worker is absent, its first creation is a **separate owner-reviewed
bootstrap**, because Cloudflare requires Workers product-level Admin to create a
new Worker and per-Worker Editor cannot be granted until the Worker exists. A
candidate owner-operated bootstrap for the bridge is `cf deploy --mode bridge`
from this reviewed commit, using the selected Cloudflare owner/admin identity.
It creates and deploys the first version together, so require an explicit review
of the resolved Build Output and resources first: the Worker must be
`ENABLED=false`, have no routes/workers.dev/previews, and the only resource
creation must be the intended SQLite Durable Object class. Stop if `cf deploy`
shows any other binding/resource or if the account/target is not positively
identified. The smoke Worker requires its own reviewed first deployment. These
are documented commands/plans only; no bootstrap was run. Do not create or store
a Workers-product/whole-account token in GitHub. After bootstrap, provision the
account-owned **Individual Workers → Editor** token for that one exact existing
Worker. The smoke Worker and DO bridge have separate GitHub environments/tokens
so each credential names only one Worker. The bridge environment also cannot
modify Gateway policies; those use a separate approval and administrative path.

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
