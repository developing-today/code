# Lab Ops Hub — Cloudflare full-stack demo

One Worker that exercises the whole Workers Paid surface at personal scale,
sized to stay inside the $5/mo plan's included amounts ($0 overage by design):

| Primitive                                | How it's used here                                                                                                                                                                                             |
| ---------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Workers**                              | API + routing, queue consumer, cron handler, assets host                                                                                                                                                       |
| **D1** (`labops-db`)                     | machines / notes / tasks / files / crawls / usage ledger                                                                                                                                                       |
| **R2** (`labops-files`)                  | uploaded files + full crawl extracts (zero egress fees)                                                                                                                                                        |
| **KV** (`CACHE`)                         | machines-list cache, refreshed by cron                                                                                                                                                                         |
| **Queues** (`labops-jobs`)               | async file indexing (SHA-256 + mark indexed in D1)                                                                                                                                                             |
| **Durable Objects** (`LiveRoom`, SQLite) | live status board: WebSocket presence + broadcast, board persisted in DO SQLite                                                                                                                                |
| **Containers** (`CrawlerContainer`)      | on-demand Python URL crawler; starts only when `/api/crawl` is called, `sleepAfter=2m` so idle minutes aren't burned. Worker-side fallback crawler with the same JSON contract if the container is unavailable |
| **Workers AI**                           | one-click note summarisation (`@cf/meta/llama-3.2-3b-instruct`, free-neuron allowance)                                                                                                                         |
| **Cron Triggers**                        | daily 04:23 UTC: trim usage ledger (90d), refresh KV snapshot                                                                                                                                                  |
| **Workers Assets**                       | the dashboard in `public/`                                                                                                                                                                                     |
| **Observability**                        | Workers Logs enabled in `wrangler.jsonc`                                                                                                                                                                       |

## The budget endpoint is the point

`GET /api/budget` reads the Worker's own D1 usage ledger (every request,
write, queue message, DO call, AI call and container run is counted) and
reports month-to-date usage against the Workers Paid included amounts —
so the demo continuously proves it is costing $0 over the $5 plan.
`estimated_overage_usd` is computed from the counted usage (for each
resource with an included amount and a rate, `max(0, used − included) ×
rate`, summed — see `computeBudget` in `src/lib.ts`), and the D1
rows-written count covers every application write path plus the usage
ledger's own rows (one D1 write per usage event, added explicitly).
Cloudflare dashboard analytics remain authoritative; this is a self-check.

## API

**Auth (required):** every `/api/*` route except `GET /api/health`
requires the `API_TOKEN` Worker secret, sent as
`Authorization: Bearer <token>` (the dashboard prompts for it once and
keeps it in `sessionStorage`; WebSocket and file-download calls may pass
`?token=` instead). Unauthenticated calls get 401. If the secret is not
set on the deployment, the API fails closed with 503 — set it before
deploying:

```bash
wrangler secret put API_TOKEN   # one-time per deployment
```

**Crawl targets:** `POST /api/crawl` only fetches public `http:`/`https:`
URLs. Both crawlers (Worker fallback and container, independently)
refuse other schemes, localhost-style names, and private / loopback /
link-local / otherwise non-global IP literals, including redirect
targets in both crawlers. Known residual: validation is on the literal
host — a public hostname that _resolves_ to a private address (DNS
rebinding) is not re-checked.

- `GET /api/health` — binding check + row counts (the only public route)
- `GET /api/budget` — self-counted usage vs included amounts
- `GET|POST /api/machines` — cluster inventory (GET is KV-cached)
- `GET|POST /api/notes`, `POST /api/notes/:id/summarize` — notes + AI summary
- `GET|POST /api/tasks`
- `GET|POST /api/files` (multipart `file`), `GET /api/files/:key` — R2 + queue indexing
- `GET|POST /api/live` — Durable Object board; WebSocket upgrade for live clients
- `POST /api/crawl {url}` — container crawl (or `{"via":"worker"}` to force fallback), full text archived to R2
- `GET /api/crawls`

## Testing

```bash
python3 -m pip install -r container/requirements.txt
npm test   # from pkgs/lab-ops-hub — runs BOTH suites, fails if either fails
```

That one command runs the TypeScript suite (`test/lib.test.ts`, the
Worker copy of the crawl validator) and the Python suite
(`test/test_container_validate.py`, the container copy) — the two
implementations share one SSRF policy, so both are pinned together.
The Python suite needs the pinned `idna` dependency installed first:
`python3 -m pip install -r container/requirements.txt`. Neither suite
needs `npm install`. CI installs that Python dependency and runs the same
test command on every pull request touching
`pkgs/lab-ops-hub/**` (`.github/workflows/lab-ops-hub.yml`).

## Develop / release

```bash
npm install
# one-time resource creation (ids land in wrangler.jsonc)
wrangler d1 create labops-db
wrangler r2 bucket create labops-files
wrangler kv namespace create CACHE
wrangler queues create labops-jobs
npm run db:init        # remote D1 schema
npm run dev            # local dev (containers run via workerd locally too)
```

Worker releases use the two manually dispatched workflows from `main`:

- `lab-ops-hub-worker-release.yml` builds Build Output v1, checks the
  mapped config against `wrangler.jsonc`, and uploads a tagged Worker
  Version with `cf workers versions create`. It does not route traffic.
- `lab-ops-hub-promote.yml` takes an uploaded version UUID and a separate
  confirmation, then explicitly deploys that version at 100% with
  `cf workers deployments create`.

Both workflows use Node `24.22.0`, `cf@1.0.0-rc.1`, and Wrangler
`4.163.0`. The upload workflow keeps a default hold: dispatch it from
`main`, review the offline build and dry run, then affirm both that the
existing Worker reports migration tag `v2` and that the pinned crawler
image is available. It needs the repository variable
`CLOUDFLARE_ACCOUNT_ID` and secret `CLOUDFLARE_API_TOKEN`; scope the token
to Cloudflare `Workers Scripts Write`. The secret is present only on the
version-upload step, never on pull-request workflows. The promotion
workflow requires the exact version UUID and its own explicit confirmation.

Cutover prerequisites: the `lab-ops-hub` Worker must already exist,
Cloudflare must report migration tag `v2` as current, and the pinned
`lab-ops-hub-crawlercontainer:v5` image must already be present in the
configured registry. `cf workers versions create` refuses a pending
Durable Object migration and cannot create a Worker that does not exist.
The existing `v1` and `v2` migration records stay in `wrangler.jsonc`;
this release path does not apply them. If any prerequisite is unknown or
false, keep the upload held and resolve it before promotion.

Runtime API auth is separate: the existing `API_TOKEN` Worker secret
gates the deployed API itself.

## Container image without a Docker daemon (what this sandbox does)

The Worker config references a pre-pushed registry image; the Worker release
does not build or publish a container. Build and push the image separately:

```bash
# 1. Build the OCI image with Nix (dockerTools) — pinned nixpkgs in container-image.nix
nix build -f container-image.nix --no-link --print-out-paths
#    -> /nix/store/<hash>-lab-ops-hub-crawlercontainer.tar.gz
# 2. Mint temporary registry push credentials (needs the ACCOUNT-WIDE token;
#    the day-to-day token gets Forbidden on this command)
wrangler containers registries credentials --push   # prints a JWT
# 3. Push with crane, username = account id, password = that JWT
#    (docker config auth entry for registry.cloudflare.com)
nix run nixpkgs#crane -- push <image.tar> \
  registry.cloudflare.com/<account-id>/lab-ops-hub-crawlercontainer:v5
# 4. wrangler.jsonc references the registry image (not ./Dockerfile);
#    the separate GHA Worker release then uploads code referencing this pin.
```

The Nix-built `container-image.nix` artifact is the only image build path.
`wrangler.jsonc` points `CrawlerContainerV2` at the matching `:v5` registry
image. The operator app `nix run .#lab-ops-hub-image-push` builds and pushes
only that image (use `-- --dry-run` to build without pushing). It does not
upload or promote a Worker Version. `wrangler.jsonc` and the Build Output
mapping both pin `CrawlerContainerV2` to the matching `:v5` image.

### Known behaviour (observed 2026-10-20, D1-verified)

- **Root cause #1 (fixed): `:latest` tags are rejected.** Cloudflare
  Containers refuses application creation with "Latest tags are not
  allowed on images" (VALIDATE_INPUT). The config uses the versioned `:v5`
  image.
  Container deploys and `wrangler containers registries credentials`
  both require the account-wide token (day-to-day gets Forbidden).
- **The container DID run** (image v1): it served real 502s from the
  Python app proving the process boots and handles requests. Its
  outbound HTTPS failed with CERTIFICATE_VERIFY_FAILED — a Nix image
  has no CA bundle configured. Fixed two ways in v3: `pkgs.cacert` in
  the image + `SSL_CERT_FILE`, and app.py passes an explicit
  `cafile=` context (glob of the store bundle) so it does not depend
  on runtime env propagation.
- **Root cause #2 (open, platform-side):** from ~19:24Z onward, every
  container start fails fast with "Failed to start container: internal
  error; reference = …" — across v1/v2/v3 images, two applications
  (one deleted + recreated), and two instance names (`crawler-1`,
  `crawler-2`). Starts succeeded before that time on the identical v1
  image. DO instances are sticky: a DO reuses its existing instance
  across rollouts, which is why image bumps appeared to "not take".
  If starts recover, the current config (image `:v5`, instance name
  `crawler-4`) should serve crawls with `source='container'` in D1 —
  that row is the only accepted proof.
- The Worker falls back to its in-Worker crawler on any container error
  and reports `container_error` in the response, so the API never
  hard-fails regardless.

## Containers: FIXED (2026-10-25)

The five-day container start failure (2026-10-20 → 25) is resolved;
`/api/crawl` is container-served again (verified in D1: rows with
source='container' plus container_run usage events). Two stacked causes,
isolated with Cloudflare's official hello-world image
(github.com/cloudflare/hello-world-container-image) in a clean-room
Worker (pkgs/cf-container-hello):

1. **Image shape.** The v3 image (relative `Cmd`, no PATH env, no shell)
   is rejected by the current start path — it failed 3/3 even inside a
   fresh container application, while the official image started
   instantly on both the default and `lite` instance types. Fixed by
   the :v5 image (see container-image.nix header).
2. **Wedged application.** The container application created on
   2026-10-20 (during the incident window) never starts again, even with
   the fixed image. The binding therefore uses a new class,
   `CrawlerContainerV2`, whose application was created fresh; it started
   on the first attempt. The legacy class stays exported (API error
   10064 otherwise) and its orphaned application is left in place,
   unused.

Also unblocked at the same time: the `API_TOKEN` Worker secret (pending
since #441) was generated, filed in Bitwarden Secrets Manager as
`cloudflare.lab-ops-hub-api-token`, and set — so the merged #441 auth
gate is live for the first time.
