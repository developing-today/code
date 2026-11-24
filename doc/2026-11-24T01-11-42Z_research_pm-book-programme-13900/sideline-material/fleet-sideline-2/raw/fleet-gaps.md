# Fleet-gaps survey — raw material for the fleet book's three remaining coverage gaps

**Lane FLEET-GAPS-SURVEY (programme #13900, developing-today/code; coordinator PM-BOOK), 2026-11-23.**
Gather-only: no book files were drafted or edited. Source of truth: laptop clone
`~/git/code` at HEAD `44aaf731ad285f25723f9198f7dabed8a2d21992`, read via
`~/tools/laptop-ssh.sh`; related issues via `fleet gh api` on the laptop. Every
load-bearing name (routes, bindings, env names, table/column names, secret names)
was verified in the source named beside it, not from memory. Where a fact comes
from a document rather than code, the survey says so.

Raw dumps deposited in `~/workspace/pm-book/sideline-material/fleet-gaps-survey/`:

- `fleet-hosted-wrangler.toml.txt` — verbatim wrangler config, fleet-hosted
- `fleet-hosted-routes-and-env.txt` — full route inventory + every env name in `src/config.ts`
- `queue-transport-alert-wrangler.jsonc.txt` — verbatim wrangler config, queue-transport-alert
- `queue-transport-alert-surface.txt` — routes, Durable Object, probe/alert state machine, status fields, deploy paths
- `fleet-dispatch-jobs-sql-and-config.txt` — config constants + every SQL statement touching the `jobs` table
- `issue-search-results.txt` — the three issue searches, verbatim titles/states, plus the `pm-stack/platform/workers.json` registry entries

One-line verdicts:

1. **fleet-hosted** — fully implemented, heavily tested hosted front door that has never been deployed: local-only by its own config, all state still behind fixture seams.
2. **queue-transport-alert** — live and in service: the off-host watchdog that probes the queue broker every 10 minutes through Cloudflare Access and files/comments GitHub incidents, pushes to the phone, and enqueues a triage task on recovery.
3. **fleet-dispatch-jobs** — a live, tiny Turso store owned solely by `fleet dispatch-test`; already described operationally inside the book's dispatch chapters, but never named as a database in its own right (see D30).

---

## 1. `fleet-hosted` — the hosted fleet front door (Worker)

### What it is

One Cloudflare Worker (`pkgs/fleet/hosted-worker/`, TypeScript, package name
`fleet-hosted`, version `0.1.0`) that is component C of the hosted-editions epic
#9078 / design #9081 (design record
`doc/2026-11-22T14-28-45Z_design_fleet-tool-workers-edition`, §3.1). It puts five
front doors — **stamping, query, queue, ingestion, sync** — plus a session
registry over one shared auth/tenancy plumbing layer, as the hosted peer of the
native fleet binary: work enqueued at the hosted queue door is claimed at the
native door and vice versa, because both speak the same broker protocol over
(backends intended to be) the same stores. It also carries a thin TypeScript CLI
(`cli.ts` / `src/cli.ts`, #12588) and a library-only direct-Turso read-token mint
(#12592). The fleet book today contains **zero** occurrences of the string
`fleet-hosted` (grep over `pkgs/fleet/book/` returned nothing).

### Surface dump

Full detail: sideline `fleet-hosted-wrangler.toml.txt` and
`fleet-hosted-routes-and-env.txt`. Headlines:

- **Config (`wrangler.toml`):** `name = "fleet-hosted"`, `main = "src/worker.ts"`,
  `compatibility_date = "2025-06-01"`. **No bindings at all** — no D1, KV, R2,
  Durable Objects, or queues. The only `[vars]` entry is `RP_AUDIENCE = "fleet-hosted"`.
  One cron trigger: `*/15 * * * *` (the ingestion sweeper). Header comment, verbatim
  in substance: _LOCAL ONLY — this config is for wrangler dev/tests; no account-level
  deployment or billable resource is authorized by this package._
- **Routes** (verified in `src/router.ts` + `src/routes/*.ts`): public
  `GET /health`, `GET /capabilities`; `POST /v1/sessions{,/resume,/resolve,/close}`;
  `POST /v1/stamps`; `GET|POST /v1/query`; 25 queue verb routes
  `POST /v1/queue/<verb>` (validate, enqueue, claim, lane, bind, heartbeat,
  checkpoint, park, release, wake, block, unblock, dep-recheck, cancel, fail,
  complete, list, show, timeline, health, revive, events-append, events-seen,
  events-count, events-prune) plus 4 fallback-claim routes
  `POST /v1/queue/fallback/{status,claim,heartbeat,release}`;
  `POST /v1/ingestion/github` (webhook receiver), `POST /v1/ingestion/sweep`;
  `POST /v1/sync/project`, `POST /v1/sync/queue-github`. The `scheduled` handler
  runs the same ingestion sweep over `INGESTION_REPOS`.
- **Dual ingestion (the brief's "webhook-route dual ingestion"):** the ingestion
  door (#12590, design §7.3, #9060 §16) is the hosted half; native ingestion
  (#9076) is the peer, and both run _the same check_. Route 1 — webhook:
  `POST /v1/ingestion/github` is public at the router (GitHub deliveries carry no
  fleet credential) and the handler authenticates each delivery itself by
  `X-Hub-Signature-256` HMAC-SHA256 over the raw body, before any parsing.
  Route 2 — sweeper: `POST /v1/ingestion/sweep` behind the normal plumbing, and
  the Worker's `scheduled` handler on the cron trigger, re-scanning the
  configured repo scope to catch listener misses. One check code path serves
  both: read the item fresh through the GitHub item source (never the payload's
  copy), parse its pm-metadata block, insert/refresh the DB record keyed on the
  item under the block's own stamped tenancy; a block-less item gets an
  ascertained block (parent from its `primary-parent/<slug>` label, provenance
  signed when the service holds stamp material) stored and written back where
  the fleet has write access; metadata that stays insufficient is forwarded as
  one deduped repo-task/v1 triage packet per item. Idempotent by construction:
  duplicate deliveries converge on one DB row and at most one triage task, and a
  create whose insert failed at the stamping door (`metadata_inserted: false`)
  is healed by the first check that sees the item.
- **Auth/tenancy plumbing** (`src/auth.ts`, `src/jwt.ts`, `src/tenancy.ts`):
  pipeline per non-public route is route match (404) → authenticate (401/403/503)
  → caller-tenancy assertion (403 cross-tenant) → service-token route scope (403)
  → handler. Three caller kinds on one Bearer <redacted> (i) `rp` — #9077
  relying-party tokens, JWS validated against the provider JWKS (WebCrypto,
  RS256/ES256), then issuer/audience/lifetime; (ii) `agent` — RP tokens carrying
  the #1253 agent role, whose lane/session claims are required and cap what the
  caller may assert; (iii) `service` — opaque per-caller service tokens recorded
  in the tenancy plane, looked up by SHA-256, scoped to named doors. A validated
  subject maps to a tenancy account keyed by its minted `binding_id` (32 lowercase
  hex, component E / #2258 contract) — never an email or provider subject;
  unmapped/inactive fails closed; an `x-binding-id` / `x-tenancy-binding` header
  disagreeing with the credential is refused as a cross-tenant attempt. A
  per-user Turso DB credential (libsql access-claim shape) presented as service
  auth is refused with its own code, `data_plane_credential_refused` (design
  §3.3 rule 3).
- **Env names** (all verified in `src/config.ts` `HostedEnv`; full list in the
  sideline dump): `RP_ISSUER`, `RP_AUDIENCE`, `RP_JWKS_URI`,
  `TENANCY_FIXTURE_JSON`, `STAMP_SIGNING_KEY_PKCS8`, `STAMP_SIGNING_KEY_ID`,
  `STAMP_VERIFY_KEYS_JSON`, `GITHUB_APP_ID`, `GITHUB_APP_PRIVATE_KEY_PKCS8`,
  `GITHUB_APP_INSTALLATION_IDS`, `GITHUB_WEBHOOK_SECRET`, `INGESTION_REPOS`,
  `METADATA_DB_ID`, `QUEUE_DB_ID`, `DIRECT_READ_SIGNING_KEY_PKCS8`,
  `DIRECT_READ_SIGNING_KEY_ID`, `DIRECT_READ_VERIFY_KEYS_JSON`,
  `DIRECT_READ_MAX_TTL_SECONDS` (default 3600). Secrets are configuration by
  name only; malformed material parses to null and the dependent path fails
  closed — it never falls open.
- **State seams (why "implemented" ≠ "deployed"):** `src/worker.ts` wires
  fixture, in-memory, module-scope implementations of every store interface —
  `FixtureSessionRegistry`, `FixtureMetadataStore` (production home: #9074's
  metadata DB), `FixtureQueueStore` (production home: the existing `fleet-queue`
  Turso DB, #6741, behind a libSQL `QueueStore`), and a fixture tenancy
  directory (`TENANCY_FIXTURE_JSON`; component E's tenancy DB does not exist
  yet). GitHub writes go through the fleet App installation-token rotation
  (`AppRotationGitHubWriter`, #12585; rate-limit/permission refusals fail over
  to the next installation, the native #8887 rule) when App material is
  configured, and through a fixture writer otherwise.
- **Doors in brief:** stamping (`POST /v1/stamps`, #12585) is the hosted world's
  only stamping authority — parses the caller's pm-metadata block (malformed
  refuses the create), stamps every fleet-owned provenance field from service
  sources only, signs the provenance (#12586, Ed25519), creates the GitHub item
  through the rotation point, inserts the full record in the same request.
  Query (#12587) lists/filters issues/PRs by Metadata fields from the metadata
  store, tenancy-scoped, with GitHub as fallback for not-yet-ingested fields
  (fallback scan bounded at 10 pages × 100 per repo) and body-vs-DB divergence
  reported with bodies winning. Queue (#12589) answers the broker envelope at
  HTTP 200 exactly as the native broker does, stamps the caller's
  credential-resolved `binding_id` into every event detail as claim provenance,
  and deliberately diverges from native in one way: it requires authentication
  and refuses anonymous claims. Sync (#12591) hosts project-reconcile's write
  set and the named queue/GitHub flows (marker-checked posts, serial ops).
  Sessions (#12584): server-minted ids `ses_` + 128 bits hex (the native #9075
  format); CLI resolution order explicit > `FLEET_SESSION_ID` >
  `OPENCODE_SESSION_ID` > per-workspace stored state, with a parity test
  transcribed from the native resolver's own tests. Direct-read (#12592) mints
  scoped, expiring, read-only libsql tokens bound to the metadata DB only
  (claims `a: "read-only"`, `id`, `binding_id`, `exp`) plus a scope inspector —
  **library only: no HTTP route and no CLI verb wire it** (grep-verified).
  `/capabilities` mirrors the native `fleet capabilities` JSON shape;
  `HOSTED_DOORS` in `src/capabilities.ts` marks all five doors `state: "live"`.
- **Failure shapes:** every refusal is a `FailClosed` rendered as
  `{ "error": <code>, "message": ... }` — 404 `not_found` (unmatched paths
  advertise nothing), 401 `missing_token`, 403 cross-tenant / out-of-scope,
  503 for each unconfigured dependency (auth, stamp signing — refused _before_
  any GitHub write, session registry, metadata store, queue store, webhook
  secret), 500 `{error: "internal"}` for unhandled errors. Door-specific:
  sessions 404 `session_unknown` (another tenancy's session answers identically
  — no existence oracle), 409 `session_closed`, 400 `invalid_session_id`, agent
  lane beyond credential refused; webhook 503 unconfigured / 401 unverified,
  both before parsing, out-of-scope events acknowledged 200 as ignored;
  stamping reports `metadata_inserted: false` instead of failing the create;
  queue verbs answer `{ok:false, exit, error}` envelopes at 200 while plumbing
  refusals stay 400/503, and fallback-claim answers 200/409/400/502 by class.
- **Workflows/tests:** there is **no** GitHub Actions workflow for fleet-hosted
  (grep of `.github/workflows/` for the package found only other packages'
  worker-test workflows). Verification is in-package: `npm test`
  (node --test over 18 test files, no network, no wrangler), `npm run smoke`
  (in-process run answering the acceptance checks end to end), `npm run typecheck`.

### Related issues

Full list with verbatim titles: sideline `issue-search-results.txt`. Spine:
epic #9078, design #9081 (+ #13913, component-C design #12593 closed), skeleton
#12583, sessions #12584, stamping #12585, query #12587, CLI #12588 (cited in
package docs), queue #12589, ingestion #12590, sync #12591 (closed), direct-read
#12592. Note the issue-state lag: the original impl issues #12583/#12584/#12585/
#12587/#12589/#12590/#12592 still stand **open** while duplicate landing issues
(#12856, #12872, #12904, #12923, #12935, #13077) are **closed** for the same
landed code. Dependencies cited in code: #9073/#9074/#9075/#9076/#9077 (native
counterparts + metadata DB + OIDC provider), #9060 (Metadata spec), #9016,
#8887, #1253, #2258, #6741, #9085, #9086. **No production-deployment issue
number was found** — both `wrangler.toml` and the README say a later C issue
owns it without naming it.

### Status

**Implemented and tested, never deployed — local-only.** Evidence:
`wrangler.toml`'s own header; package README ("Local-only: `wrangler.toml`
authorizes no account deployment"); `pm-stack/platform/workers.json`:
`state: "not deployed"`, `deploy: "none"`, `address: null`. All five doors plus
sessions and the direct-read mint are real code over fixture backends; the
production wiring (metadata DB #9074, tenancy DB, libSQL queue store) is owned
elsewhere and has not landed.

### Proposed chapter outline (for the drafting lane)

- What the hosted edition is: component C of #9078/#9081, one Worker as the
  hosted peer of the native binary, and the edition-parity rule (same checks,
  same envelopes, same id formats in both editions).
- The plumbing layer: RP/agent/service caller kinds, tenancy by `binding_id`,
  cross-tenant refusal, and the data-plane-credential refusal (§3.3 rule 3).
- The five doors, one section each: stamping as sole hosted authority (stamp
  sourcing table, sign-before-write, `metadata_inserted: false` + heal), query
  (DB-first, GitHub fallback, divergence with bodies winning), queue (broker
  protocol over hosted transports, authenticated-claims divergence, fallback
  claims over GitHub comments), ingestion (dual ingestion — see below), sync.
- Dual ingestion as its own section: webhook route (HMAC, fail-closed 503/401)
  and sweeper route + cron as two triggers of one check; idempotence, triage
  forwarding, and the create-path heal.
- Sessions and the CLI: server-minted ids, resolution-order parity with the
  native resolver, the thin TS CLI's verb set and fixed caller-owned allowlist.
- The direct-read path: scoped read-only tokens for the metadata DB only, the
  queue-DB refusal, and its current library-only wiring.
- Seams and status: fixture stores and their production homes (#9074 metadata
  DB, `fleet-queue` #6741, component E tenancy), local-only posture, and what
  the (unnumbered) deployment issue must land.
- Failure-shape table: FailClosed codes per door, the queue envelope exception,
  and the fail-closed-on-unconfigured rule that runs through every door.

---

## 2. `queue-transport-alert` — the queue broker's off-host watchdog (Worker)

### What it is

A Cloudflare Worker (`pkgs/queue-transport-alert/worker/`, plain JavaScript,
`code#6369`) that exists because of a blind spot: when the repo-queue broker's
transport is down, every worker lane exits silently at its Step 0 health gate,
so a total queue outage produced no signal (#5708, 2026-11-04, was filed by
hand; the 2026-11-18 outage — tailscaled stopped during a rebuild — was noticed
only from lane reports). The Worker watches the broker **from off the broker
host**: every 10 minutes it probes the broker's public route through Cloudflare
Access, and on a confirmed outage it files/comments a GitHub incident, pushes to
the phone (Web Push + Telegram), and — once the broker is back — enqueues one
triage task into the queue it watches. It is a port of the local script
`scripts/fleet/queue-transport-alert/check_transport.py`, which still exists
with systemd units; the two share one contract and one incident title and
dedupe onto the same issue when both run. The fleet book contains **zero**
occurrences of `queue-transport-alert` (grep over `pkgs/fleet/book/`).

### Surface dump

Full detail: sideline `queue-transport-alert-wrangler.jsonc.txt` and
`queue-transport-alert-surface.txt`. Headlines:

- **Config:** deploy source of truth is `cloudflare.config.ts` (cf versions
  flow); `wrangler.jsonc` mirrors it for `wrangler dev` and
  `test/config.test.mjs` keeps the two in step. Name `queue-transport-alert`,
  main `src/index.js`, `compatibility_date = "2026-10-25"`, observability on,
  cron `*/10 * * * *`, public address
  `https://queue-transport-alert.developing-today.workers.dev`.
- **Binding:** one Durable Object binding, `TRANSPORT_STATE` → class
  `TransportState`, SQLite storage, migration tag `v1`
  (`new_sqlite_classes`). A single instance (`idFromName("broker")`) holds all
  state so passes never race: keys `state`, `push_subs`, `last_push`, `vapid`
  (the VAPID P-256 pair is generated once inside the DO and never leaves it).
- **Vars** (identical in both configs): `BROKER_URL=https://queue-broker.security.cab`,
  `REPO=developing-today/code`, `THRESHOLD=2`, `PROBE_TIMEOUT_MS=10000`,
  `QUEUE=repo-health-code-pilot`, `TELEGRAM_CHAT_ID` (Drew's Telegram user id,
  printed in the file), `PUBLIC_URL`, `APPLY=true` ("true" files/comments,
  pushes and enqueues; anything else only logs).
- **Secrets** (by name only, uploaded per version by `deploy.mjs` from
  Bitwarden; never in any config file): `ACCESS_CLIENT_ID` /
  `ACCESS_CLIENT_SECRET` — the Cloudflare Access service token for the broker
  route (Bitwarden `cloudflare.access.queue-broker-client-id` / `-secret`);
  `GITHUB_TOKEN` (`github.developing-today-code-automation`); `TELEGRAM_BOT_TOKEN`
  (`openclaw.telegram-bot-token`); `BROKER_AGENT_TOKEN` (the broker's
  `X-Agent-Token`, sourced from the file `~/.config/queue-broker/token`, carried
  over when the file is absent). The cf CLI itself uses
  `cloudflare.api-token-day-to-day`.
- **Probe:** `GET {BROKER_URL}/healthz` with `CF-Access-Client-Id` /
  `CF-Access-Client-Secret` headers, `redirect: "manual"`, 10s timeout. That
  route is the pm-stack tunnel to the broker's Tailscale listener, so it fails
  when the laptop is off, tailscaled is stopped, or the broker is down. Healthy
  only on HTTP 200 with a JSON body of `{"ok": true}` (`classifyHealthz`).
- **Routes:** `GET /` (status page, PWA-installable, auto-refresh 60s),
  `GET /status` + `/status.json` (same view as JSON, CORS-open),
  `GET /sw.js`, `/manifest.webmanifest`, `/icon.svg`, `/vapid.json`;
  `POST /subscribe`, `/unsubscribe`, `/test-push` (that device only, ≥30s
  apart), `POST /admin/test-push` (all devices; `X-Admin-Token` must equal the
  Access client secret, constant-time compared). Status values:
  `up | degraded | down | stale` (no probe in 25 minutes) `| unknown`, plus
  `last_check`, `last_ok`, `failing_since`, `consecutive_failures`,
  `last_failure`, `incident`, `last_outage {start,end,duration,incident}`,
  `pending_deliveries`, `last_errors`, `last_push`, `issue_title_prefix` —
  no secrets.
- **Alert/recovery behaviour** (`src/logic.js` + `src/run.js`, a direct port of
  `check_transport.py`): `THRESHOLD` (2) consecutive failed probes → one alert
  per outage window: find an open issue titled with the prefix
  `Queue transport incident:` and comment on it, else file one titled
  `Queue transport incident: repo-queue broker unreachable since <window_start>`
  with labels `kind/bug`, `priority/high`, `area/infra`, `source/fleet-monitor`.
  First healthy probe after an alerted window → one recovery comment with the
  outage span; the Worker **never closes** the incident (there is deliberately
  no `close()` in `src/github.js`; rule #10136). Alert and recovery each send
  one Telegram message and one Web Push (payload-free: the service worker reads
  `/status.json`'s `last_push`); on recovery it also enqueues the triage task
  (below). Notifications and the enqueue ride a `pending` list in DO state
  (cap 20) and retry on later ticks; a failed GitHub write leaves the window
  un-alerted so the next tick retries it; push endpoints reported gone
  (404/410) are dropped; at most 20 devices may subscribe.
- **Recovery queue task** (`src/queue.js`): `POST {BROKER_URL}/enqueue` with
  `X-Agent-Token` + the Access headers, a repo-task/v1 `triage` packet anchored
  to the incident issue, queue `QUEUE` (`repo-health-code-pilot`), priority 20,
  dedupe key `repo-health:v1:triage:<repo>:issue:<N>` (a dedupe error from the
  broker counts as success). It is only attempted while the broker answers —
  which is precisely why it is filed on recovery, not during the outage.
- **Failure shapes:** probe classification carries the diagnosis in the detail
  string — 301/302/303/307/401/403 → "Cloudflare Access refused the probe's
  service token, or it is unset"; 502/503/504/530 → "the tunnel could not reach
  the broker"; 200 with a non-`ok:true`/non-JSON body names the body (clipped
  to 200 chars); timeouts name the URL and the timeout. Missing Access secrets
  → the probe returns `skipped` and the pass only logs (no alert, by design).
  `APPLY != "true"` or a missing `GITHUB_TOKEN`/`TELEGRAM_BOT_TOKEN`/
  `BROKER_AGENT_TOKEN` degrades that one side effect to log-only. HTTP edges:
  subscribe body >2048 bytes → 413; invalid push endpoint → 400; 21st device →
  409; test-push from an unsubscribed device → 404, repeated within 30s → 429;
  bad/missing admin token → 403; other POSTs/methods → 405; unknown GET → 404.
- **Deploy:** primary operator path `nix run .#queue-transport-alert-deploy`
  (cf versions flow: upload a version tagged `git-<sha12>` with secrets piped
  via `--secrets-file`, deploy it at 100%, apply triggers, verify the serving
  version and `/status.json`; `--deploy-version <id>` rolls back to an older
  version as it was). An optional, manual-only GitHub Actions release also
  exists (`.github/workflows/queue-transport-alert-deploy.yml`): main-only,
  behind the `cloudflare-queue-transport-alert` environment (independent
  required reviewer, no self-review, admin-bypass attestation checked by a
  verify-environment job), with upload and promote as separate dispatches and
  no automatic rollback. Tests: `node --test test/*.test.mjs`, also run in the
  flake package's `checkPhase`. It is registered in the deploy registry as
  `queue-transport-alert`.

### Related issues

Full list: sideline `issue-search-results.txt`. Founding issue #6369 (still
open); local-script landing #12006; Worker landings #12009 (Worker, deployed
from the flake), #12010 (recovery comment, never close), #12012 (phone push,
queue task, status page), #12013 (cron registration), #12016 (Web Push),
#12023 (cf versions flow), #12025 (secrets through a pipe), #12026 (deploy
registry), #12550 (trusted GitHub Actions deploy, closed). A real incident the
Worker filed: #12414 (2026-11-22, closed). Follow-ons still open: #13493
(deploy the twelve hand-deployed Workers from Actions), #13503 (fold the
queue's alert into the PM console). Rule cited throughout: #10136.

### Status

**Live, in service.** Evidence: `pm-stack/platform/workers.json` records
`state: "in service"` with its workers.dev address; it has filed a real
incident (#12414, 2026-11-22); both deploy paths (Nix operator command and the
guarded Actions workflow) are in the repo. (The registry's deploy bookkeeping
is stale — see D32.)

### Proposed chapter outline (for the drafting lane)

- Why this Worker exists: the Step-0 health-gate blind spot, #5708 filed by
  hand, the 2026-11-18 tailscaled outage, and the off-host vantage point.
- The probe: the Access service-token route to `queue-broker.security.cab/healthz`,
  what each failure class means (Access refusal vs tunnel down vs broker down),
  and the skipped-probe posture when the token is unconfigured.
- The state machine: threshold, one alert per window, quiet sub-threshold
  windows, one recovery comment, never-close (#10136), all in one SQLite
  Durable Object.
- The three side-effect channels and their retry discipline: GitHub incident
  (find-or-file by title prefix + labels), Web Push (payload-free, VAPID keys
  inside the DO, 20-device cap), Telegram via the existing OpenClaw bot.
- The recovery triage task: why it can only be filed once the broker answers,
  the repo-task/v1 packet, and its canonical dedupe key.
- The status surface: `/`, `/status.json` fields and the stale rule, the PWA
  subscribe/test-push routes and the admin test-push's Access-secret auth.
- Two implementations, one contract: the Worker as a port of
  `check_transport.py`, the shared incident title, and dedupe when both run.
- Deploy and rollback: the cf versions flow via the Nix deploy app, secrets by
  Bitwarden name per version, and the guarded optional Actions release.

---

## 3. `fleet-dispatch-jobs` — the dispatch-test jobs store (Turso DB)

### What it is

A Turso (libSQL) database whose single table, `jobs`, is the stored record of
explicit-dispatch CI test jobs submitted by the `fleet dispatch-test` domain of
the fleet multicall binary (design #6580). A caller runs
`fleet dispatch-test submit --submodule <name> …`; the client inserts a job row,
picks an execution-home copy of the fleet repo (`dezren39/fleet` or a fork) with
free GitHub Actions capacity, dispatches the generic workflow
`fleet-dispatch-test.yml` in that copy with the job id as its only input, and
marks the row dispatched; the workflow itself then writes back its run id and,
at the end, its terminal status and output. `fleet dispatch-test status <id>`
reads the row back. The #13844 Turso audit counted **`jobs`, 6 rows** and
grouped the DB under §G8 MOVE-CLEAN. Naming caution for the drafting lane:
three different things share the word "dispatch" — the **dispatch** domain
(thin client over the pm-dispatch service; does not touch this DB),
**pm-dispatch** (the laptop service, `pkgs/pm-dispatch` / `pm-stack/dispatch`;
does not touch this DB), and **dispatch-test** (this DB's sole owner in this
repo).

### Surface dump

Full detail: sideline `fleet-dispatch-jobs-sql-and-config.txt`. Headlines:

- **Connection names (verified in `pkgs/fleet/crates/fleet-config/src/lib.rs`):**
  default URL constant `DEFAULT_DISPATCH_TEST_TURSO_URL` =
  `https://fleet-dispatch-jobs-dezren39.aws-us-east-1.turso.io` (config key
  `[dispatch-test] turso_url`); Turso token resolved **by name only** through
  fleet-secrets, default name `turso.fleet-dispatch-jobs-token` (config key
  `[dispatch-test] turso_token_name`). The GitHub token for the same domain
  defaults to the name `github.ghcr-classic-pat`. Token values enter exactly
  one `Authorization` header per request and are never printed, logged, or
  stored. Requests are `POST {url}/v2/pipeline` (hrana-over-HTTP v2: one
  execute + one close per call).
- **Owner code paths:** the only owner crate is
  `pkgs/fleet/crates/fleet-dispatch-test/src/lib.rs` (1,802 lines; verbs
  `scan` / `submit` / `status`, entry `pub fn run`). Config lives in
  `pkgs/fleet/crates/fleet-config/src/lib.rs` (`DispatchTestConfig`, accessors
  ~lines 1444–1490). A scoped grep for `fleet-dispatch-jobs` over `pkgs/fleet`,
  `pkgs/pm-dispatch`, `pm-stack/dispatch`, `home/`, and `pkgs/fleet/migrations/`
  found **no other reader or writer** in those paths (a repo-wide grep timed
  out on the laptop; see the caveat in the sideline dump).
- **Schema (`jobs`, 14 columns):** `id, created_at, name, target_repo,
target_ref, script, status, run_id, run_url, exit_code, output_tail,
updated_at, submodule, commands`. Phase-2 semantics: `target_ref` carries
  the _submodule_ ref (empty = the umbrella's recorded SHA) and `script`
  mirrors `commands` so MVP-era readers still see the payload. **No
  `CREATE TABLE` for `jobs` exists anywhere in the searched repo** — the
  table predates the crate (MVP era); the only DDL the code runs is the
  idempotent phase-2 migration in `ensure_schema()` —
  `ALTER TABLE jobs ADD COLUMN submodule TEXT` and
  `ALTER TABLE jobs ADD COLUMN commands TEXT` — applied by `submit` and
  `status`, where a "duplicate column" error means already-migrated and is
  ignored. (`pkgs/fleet/migrations/*` belong to the `fleet-queue` DB, not
  this one.) Job ids: `dt-<unix_millis base36>-<4 hex chars>`, generated
  client-side.
- **Writers:** (1) the client crate — `INSERT` with `status='queued'`,
  `target_repo=NULL`, `run_id/run_url/exit_code/output_tail=NULL`; then, in
  the load-bearing #6598 order, `UPDATE target_repo`, the GitHub workflow
  dispatch POST, and only then `UPDATE status='dispatched'` (a failed
  dispatch leaves the row `queued` with its target repo recorded; no eligible
  scope is a transport error, exit 3, with the row left queued). (2) the
  generic workflow `fleet-dispatch-test.yml` **in the fleet repos**
  (`dezren39/fleet` and its copies — not in this repo): per the fleet book's
  own description it writes `status='running'` + `run_id`/`run_url` when it
  fetches the job, and the terminal status + `exit_code`/`output_tail` at the
  end, always, even on failure (a cancelled run is best-effort and may leave
  the row `running`). No code for that write-back exists in
  developing-today/code to verify it against.
- **Readers:** `verb_status` (`SELECT` of all 14 columns `WHERE id = ?`; an
  unknown id is HTTP 404, exit 4 — it reads the store only, never GitHub) and
  the workflow's job fetch. Notably, `verb_scan` never opens the store at
  all: capacity counting is GitHub-only (active = jobs `in_progress`/`queued`
  across `in_progress` runs, paginated at 100/page to a 100-page bound, plus
  queued runs counted as 1 active unit each from `total_count`; eligible =
  workflow present and active < threshold, default 40).
- **Failure shapes:** exit codes 0 ok / 2 usage / 3 transport (credentials
  unresolved, store unreachable, no eligible scope) / 4 HTTP status from
  GitHub or Turso (including unknown job id). Turso specifics: non-2xx →
  exit 4 with the first 300 body chars; a top-level `{"error": …}` or a
  per-request `results[i].type != "ok"` → transport error, never an empty
  result; a non-JSON body → transport error. `--threshold 0` is a usage
  error; `[dispatch-test] threshold = 0` is a config validation error (#6603).

### Related issues

Design #6580 (closed; "Design: fleet dispatch-test — explicit-dispatch CI over
fleet runner capacity"), domain landing #6589 (closed), phase 2 #6595
(closed; fleet-repo copies + submodule jobs), ordering fix #6598 via #6721
(closed). Cited in code: #6603 (threshold config), #6605 (250-job pagination
case), #6622 (pagination contract), #6623 (repo-wide pagination inventory).
Estate-level: #13844 (Turso→D1 audit; `jobs`, 6 rows, §G8 MOVE-CLEAN). Full
searches: sideline `issue-search-results.txt`.

### Status

**Live, small, and current.** The DB exists in the live Turso estate (audit
read 2026-11-23: `jobs`, 6 rows), the owner crate is in the fleet binary's
workspace and covered by the book's dispatch chapters operationally, and its
only in-repo consumer is `fleet dispatch-test`. Its phase-2 migration runs
idempotently on every submit/status, so the schema in the table above is the
schema the code maintains today. The audit's MOVE-CLEAN recommendation (D1)
is a recommendation only; nothing in the code moves it.

### Proposed chapter outline (for the drafting lane)

- One store, one owner: `fleet-dispatch-jobs` as the dispatch-test domain's
  Turso store, and the three-way "dispatch" naming disambiguation (dispatch /
  pm-dispatch / dispatch-test).
- Connection by name: the `[dispatch-test]` config keys, the default URL, and
  secrets-by-name through fleet-secrets (GitHub and Turso token names).
- The `jobs` table: the 14 columns, phase-2's `submodule`/`commands` and the
  `script`-mirrors-`commands` rule, `target_ref` as a submodule ref, and the
  client-generated `dt-…` job id.
- Schema provenance and the migration that isn't a creation: no `CREATE TABLE`
  in the repo, the idempotent ALTER pair in `ensure_schema()`, and the
  duplicate-column rule.
- The write path: submit's insert → target update → dispatch → dispatched
  order (#6598), and what each failure leaves behind in the row.
- The second writer: the generic workflow in the fleet repos — `running` +
  run linkage, terminal write-back always, the cancelled-run exception — and
  the honesty rule that `status` reports the stored row, never live GitHub state.
- Reads: `status` as the only in-repo reader, and the fact that `scan` never
  opens the store (capacity is a GitHub-side count).
- Estate position: 6 rows at audit time, the #13844 §G8 MOVE-CLEAN
  recommendation, and why the store stays Turso-shaped (hrana pipeline,
  typed cells, pipeline failure shapes) until a programme decides otherwise.

---

## Disagreements — docs/designs vs code (numbering continues from D30)

Writers cite these numbers. Where a doc and the code disagree, **the code
wins**, per the bookwright rule already used in `survey/disagreements.md`
(D1–D14; D15–D29 are understood taken by sibling lanes, so this lane starts
at D30 as instructed).

**D30. The census ledger understates the fleet book's existing dispatch-jobs
coverage.** `coverage-ledger.md` Table B says of `fleet-dispatch-jobs`:
"none — fleet book dispatch chapters do not name this DB", and the brief for
this lane repeated "the fleet book's dispatch chapters never name it". The
book in fact names the store's endpoint URL
(`https://fleet-dispatch-jobs-dezren39.aws-us-east-1.turso.io` — the DB name
is the URL's subdomain), its token name (`turso.fleet-dispatch-jobs-token`),
its table and all 14 columns, and its full row lifecycle, in both
`pkgs/fleet/book/concepts/dispatch.md` (§"The Turso jobs store") and
`pkgs/fleet/book/reference/dispatch-test.md`. What is genuinely missing is
narrower: the DB is never named _as a database in its own right_ — there is
no data-stores inventory chapter in the fleet book's SUMMARY at all, and no
chapter treats the DB (provenance, estate position, migration posture) as
its subject. Code + book win over the ledger: the drafting lane should
write the DB-centred chapter the outline in §3 proposes, not re-write the
operational coverage that already exists.

**D31. fleet-hosted's README still lists the sync door as an unfilled stub.**
The README's "What is a stub" table says the sync routes
(`POST /v1/sync/project`, `POST /v1/sync/queue-github`) "authenticate, then
answer `501 not_implemented`", filled by #12591. The code disagrees:
`src/capabilities.ts` marks the sync door `state: "live"`,
`src/worker.ts`'s header records the sync door among the landed doors,
`src/routes/sync.ts` + `src/sync.ts` implement it, and #12591 is closed.
The same README's own door sections and `package.json` ("every component-C
door is live") agree with the code. Code wins: no door in the package is a
stub today; the stub table is a leftover from the #12583 skeleton slice.

**D32. The platform registry's queue-transport-alert deploy record is stale.**
`pm-stack/platform/workers.json` records `queue-transport-alert` as
`deploy: "by hand"`, `workflow: null`, with a "next" that frames Actions
deployment as future work ("Deployed from Actions, stood up, checked and
rolled back the way the PM console is"). The repo disagrees:
`.github/workflows/queue-transport-alert-deploy.yml` exists and #12550
("deploy through trusted GitHub Actions") is closed; the Worker's README
documents the Actions release in detail. Code wins: the registry entry
predates the Actions path. (The same entry's `state: "in service"` and
address do agree with the code, and its `fleet-hosted` entry — "not
deployed", deploy "none" — agrees with `wrangler.toml`; only the QTA
deploy fields are stale.)

**D33. The Turso audit files `fleet-dispatch-jobs` under "small product
stores"; its owner is the fleet plane.** Audit §G8 lists
`fleet-dispatch-jobs` (`jobs`, 6 rows) alongside `notes-submissions`,
`bedside-and-away`, and `voice-line-ledger` as a small product store,
MOVE-CLEAN. The code disagrees about what it is: the DB's sole owner is
`pkgs/fleet/crates/fleet-dispatch-test` — fleet-plane CI infrastructure,
configured from the fleet binary's `[dispatch-test]` config section — not
a product surface. The audit's _facts_ (one table, 6 rows, tiny, join-free)
are not in dispute, and the ledger's assignment of the gap to the fleet
book follows the code. Code wins on classification; a chapter should not
inherit the "product store" framing.

**D34. The `jobs` table's creation exists nowhere in the repo, though book
and code both present its schema as settled.** `reference/dispatch-test.md`
and `concepts/dispatch.md` state the full 14-column schema flatly, and the
crate's `ensure_schema()` maintains it — but the only DDL in the code is
the phase-2 ALTER pair; no `CREATE TABLE jobs` exists in `pkgs/fleet`,
`pkgs/pm-dispatch`, `pm-stack/dispatch`, `home/`, or
`pkgs/fleet/migrations/` (those migrations belong to the `fleet-queue`
DB). The table is an MVP-era artifact created outside the repo's recorded
history. Code wins: a chapter may state the schema the code maintains,
but must not imply a repo migration creates the table.

**D35. "Every door is live" (fleet-hosted docs) vs the package's own
local-only config.** `package.json` and the README say every component-C
door "is live", and `/capabilities` reports all five doors
`state: "live"`. The deployment facts in the same package disagree with
any production reading of "live": `wrangler.toml` authorizes no deployment,
`INGESTION_REPOS` is commented out (so the scheduled sweep has nothing to
scan), every store is a fixture in module scope, and `workers.json`
records the Worker "not deployed". Both are true at different layers —
the _code_ is complete and smoke-proven in-process; the _service_ has
never run in production. Code/config win: the book should say
"implemented, undeployed", never "live", for fleet-hosted. (Contrast
queue-transport-alert, where "in service" is accurate.)

**Disagreement count for this lane: 6 (D30–D35).**

— Lane FLEET-GAPS-SURVEY, 2026-11-23. Survey only; no chapters written, no book files touched.
