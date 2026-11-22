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
