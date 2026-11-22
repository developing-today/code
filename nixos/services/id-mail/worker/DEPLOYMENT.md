# id-mail Worker publishing and promotion

The `id-mail-worker-publish` workflow is a manual, main-only publisher. Its
confirmation input defaults to false. A confirmed run uploads a version with
the pinned `cf` CLI and the Workers Versions API; it does not deploy it. It
records an artifact bound to the exact workflow run and attempt, including the
version ID returned by Cloudflare, a unique run tag, commit SHA, and the active
deployment ID/version observed before the upload. The upload step fails if the
active deployment changes while the version is being uploaded.
Before upload, it also requires the latest uploaded version to be the active
version. The required `CALLERS` secret therefore inherits from that current
version rather than an unpromoted upload; the Workers API strict inheritance
check fails the upload if the secret cannot be resolved.

The separate `id-mail-worker-promote` workflow is also manual, main-only, and
default-off. It accepts a successful upload run ID and attempt, checks that
exact GitHub Actions attempt completed successfully from `main` using the
publisher workflow, then downloads that attempt's artifact. Before promotion,
it verifies that the artifact commit SHA matches the selected successful run,
then checks the exact Cloudflare version ID and tag, unchanged active
deployment, inherited `CALLERS` secret binding descriptor, and unchanged
existing version bindings and versioned settings. It probes each current
configured route with an unauthenticated `POST`; the expected `401` response is
returned after the Worker validates its `CALLERS` and `MAIL_CONFIG` bindings,
and before it accesses the Durable Object or email transport in `src/index.js`.
The probe sends no body or authorization header, so it cannot perform a
caller-authorized send. It then creates a 100% deployment of only the selected
version, checks the active deployment and binding/settings metadata again, and
repeats the route health probes.

Both workflows use the project-pinned `cf` CLI (with the pinned Wrangler build
backend) and never call direct Wrangler deploy. Promotion uses
`cf workers deployments create`; neither workflow calls route/trigger
operations, changes script settings, writes secrets, or retires/deletes
Workers. `MAIL_CONFIG`, `EMAIL`, `LIMITER`, the SQLite `Limiter` export, and
`CALLERS` remain as declared in `cloudflare.config.ts`. Routes and the existing
`workers.dev` setting are not part of version promotion. The existing retire
list in `config.json` is not acted on.

## Required GitHub and Cloudflare setup

Before dispatching either workflow, configure the existing repository variable
`CLOUDFLARE_ACCOUNT_ID` and create the protected GitHub environment
`mail-worker-production`. Restrict it to the `main` branch, require an
independent reviewer, and prevent self-review or bypass where supported. Add
the Cloudflare credential to that environment only as
`ID_MAIL_WORKER_CF_TOKEN_MAIL_WORKER_PRODUCTION`. This unique name must not
exist as a repository or organization secret: GitHub resolves same-name
secrets by scope precedence, so the environment must be the only source. The
workflow never references the generic `CLOUDFLARE_API_TOKEN` secret and does
not fall back to it.

Create a Cloudflare API token with **Individual Workers Editor** permission,
resource-scoped to the account and the existing `mail` Worker only. Do not use
an account-wide Workers Scripts permission. No token value is read by this
repository workflow outside the two Cloudflare CLI invocation steps.

The environment and secret must be configured before dispatch. The repository
environment list currently does not contain `mail-worker-production`; do not
dispatch until a repository administrator creates and protects it and verifies
the unique secret exists only there. The checked-in workflow does not create
or administer GitHub settings or secrets.

## Scope and cutover notes

- Confirm that the `mail` Worker already exists. Upload is not a bootstrap
  operation.
- Run the publisher manually on `main` with `confirm_upload=true`, then retain
  its successful run ID and attempt number. A successful upload is not a live
  deployment.
- After reviewing the exact uploaded artifact and code, run the promotion
  workflow manually on `main` with the same run ID/attempt and
  `confirm_promotion=true`. Environment protection approval is still required.
- Promotion fails closed if another deployment became active since upload, if
  the selected version/tag or `CALLERS` binding does not match, if binding or
  versioned setting metadata differs from the current active version, or if
  any existing route fails its safe unauthenticated health probe.
- No caller, route, binding, secret, or Worker retirement changes are included
  in this cutover.
