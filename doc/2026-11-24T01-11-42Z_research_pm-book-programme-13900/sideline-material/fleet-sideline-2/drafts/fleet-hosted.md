# The hosted edition: fleet-hosted

<!-- id: fleet-hosted since: pre-release -->

`fleet-hosted` is a Cloudflare Worker (`pkgs/fleet/hosted-worker/`,
TypeScript, package name `fleet-hosted`, version `0.1.0`) that is the
hosted peer of the native fleet binary. It is component C of the
hosted-editions epic #9078, designed in #9081 (design record
`doc/2026-11-22T14-28-45Z_design_fleet-tool-workers-edition`, §3.1). One
Worker puts five front doors — **stamping, query, queue, ingestion,
sync** — plus a session registry over one shared auth/tenancy plumbing
layer. Work enqueued at the hosted queue door is claimed at the native
door and vice versa, because both editions speak the same broker
protocol over (backends intended to be) the same stores.

The edition-parity rule runs through the whole package: the same
checks, the same envelopes and the same id formats in both editions.
Where the hosted edition deliberately diverges, the divergence is
named in the code and in this chapter — the queue door's refusal of
anonymous claims is the one deliberate case.

**Status, stated plainly (2026-11-23): implemented and tested, never
deployed — local-only.** The package's own `wrangler.toml` header says
the config is for `wrangler dev` and tests and authorises no
account-level deployment; the platform registry
(`pm-stack/platform/workers.json`) records `state: "not deployed"`,
`deploy: "none"`, `address: null`. The package documentation's phrase
"every door is live" is true of the code — `/capabilities` reports all
five doors `state: "live"` and an in-process smoke run answers the
acceptance checks end to end — and false of the service, which has
never run in production. This chapter follows the code and the config,
not the shorter phrase (disagreement D35 in the survey record for
programme #13900).

## The plumbing layer

Every non-public route runs the same pipeline, in this order: route
match (an unmatched path is a 404 that advertises nothing) →
authenticate (401/403/503) → caller-tenancy assertion (a cross-tenant
assertion is a 403) → service-token route scope (403) → handler.

Three caller kinds share one Bearer credential form:

| Caller kind | Credential | What caps it |
| --- | --- | --- |
| `rp` | A #9077 relying-party token: a JWS validated against the provider JWKS (WebCrypto, RS256/ES256), then issuer, audience and lifetime | The tenancy mapping for its subject |
| `agent` | An RP token carrying the #1253 agent role | Its lane and session claims are required, and cap what the caller may assert |
| `service` | An opaque per-caller service token recorded in the tenancy plane, looked up by SHA-256 | The named doors its scope lists |

A validated subject maps to a tenancy account keyed by its minted
`binding_id` — 32 lowercase hex characters, the component E / #2258
contract — never by an email address or a provider subject. An
unmapped or inactive subject fails closed. An `x-binding-id` or
`x-tenancy-binding` header that disagrees with the credential is
refused as a cross-tenant attempt. A per-user Turso database
credential (the libsql access-claim shape) presented as service auth
is refused with its own code, `data_plane_credential_refused` — design
§3.3 rule 3: a data-plane credential is never a service credential.

Configuration is by name only. The environment names are verified in
`src/config.ts` (`HostedEnv`): `RP_ISSUER`, `RP_AUDIENCE` (the one
`[vars]` entry in `wrangler.toml`, value `fleet-hosted`), `RP_JWKS_URI`,
`TENANCY_FIXTURE_JSON`, `STAMP_SIGNING_KEY_PKCS8`,
`STAMP_SIGNING_KEY_ID`, `STAMP_VERIFY_KEYS_JSON`, `GITHUB_APP_ID`,
`GITHUB_APP_PRIVATE_KEY_PKCS8`, `GITHUB_APP_INSTALLATION_IDS`,
`GITHUB_WEBHOOK_SECRET`, `INGESTION_REPOS`, `METADATA_DB_ID`,
`QUEUE_DB_ID`, `DIRECT_READ_SIGNING_KEY_PKCS8`,
`DIRECT_READ_SIGNING_KEY_ID`, `DIRECT_READ_VERIFY_KEYS_JSON` and
`DIRECT_READ_MAX_TTL_SECONDS` (default 3600). Malformed key material
parses to null and the dependent path fails closed; it never falls
open.

## The five doors

### Stamping — the hosted world's only stamping authority

`POST /v1/stamps` (#12585) parses the caller's pm-metadata block — a
malformed block refuses the create — stamps every fleet-owned
provenance field from service sources only, signs the provenance
(#12586, Ed25519), creates the GitHub item through the App
installation-token rotation point, and inserts the full record in the
same request. Signing is refused *before* any GitHub write when the
signing material is unconfigured (503). A create whose database insert
failed is not failed back to the caller: the response reports
`metadata_inserted: false`, and the ingestion check below heals it.

GitHub writes go through the fleet App installation-token rotation
(`AppRotationGitHubWriter`, #12585) when App material is configured:
a rate-limit or permission refusal fails over to the next
installation, the native #8887 rule. Without App material, writes go
through a fixture writer — one of the seams listed below.

### Query

The query door (#12587) answers `GET|POST /v1/query`: it lists and
filters issues and pull requests by Metadata fields from the metadata
store, tenancy-scoped. GitHub is the fallback for fields an item has
not been ingested with yet; the fallback scan is bounded at 10 pages
of 100 per repo. Where the stored record and the item body diverge,
the divergence is reported and the body wins.

### Queue

The queue door (#12589) answers 25 verb routes under
`POST /v1/queue/<verb>` — validate, enqueue, claim, lane, bind,
heartbeat, checkpoint, park, release, wake, block, unblock,
dep-recheck, cancel, fail, complete, list, show, timeline, health,
revive, events-append, events-seen, events-count, events-prune — plus
four fallback-claim routes under `POST /v1/queue/fallback/`
(status, claim, heartbeat, release). It answers the broker envelope
at HTTP 200 exactly as the native broker does, and stamps the caller's
credential-resolved `binding_id` into every event detail as claim
provenance. The one deliberate divergence from the native broker: the
hosted door requires authentication and refuses anonymous claims.

### Ingestion — one check, two triggers

The ingestion door (#12590; design §7.3, #9060 §16) is the hosted half
of ingestion; native ingestion (#9076) is the peer, and both run *the
same check*. The check reads the item fresh through the GitHub item
source — never the delivery payload's copy — parses its pm-metadata
block, and inserts or refreshes the database record keyed on the item
under the block's own stamped tenancy. A block-less item gets an
ascertained block (its parent taken from its `primary-parent/<slug>`
label, its provenance signed when the service holds stamp material),
stored and written back where the fleet has write access. Metadata
that stays insufficient is forwarded as one deduped repo-task/v1
triage packet per item.

Two triggers run that one check:

1. **The webhook route.** `POST /v1/ingestion/github` is public at the
   router — GitHub deliveries carry no fleet credential — and the
   handler authenticates each delivery itself, by the
   `X-Hub-Signature-256` HMAC-SHA256 over the raw body, before any
   parsing. An unconfigured webhook secret is a 503 and an unverified
   signature a 401, both before parsing; out-of-scope events are
   acknowledged 200 as ignored.
2. **The sweeper.** `POST /v1/ingestion/sweep` behind the normal
   plumbing, and the Worker's `scheduled` handler on its one cron
   trigger (`*/15 * * * *`), re-scan the configured repo scope
   (`INGESTION_REPOS`) to catch listener misses.

The check is idempotent by construction: duplicate deliveries
converge on one database row and at most one triage task, and a create
that reported `metadata_inserted: false` at the stamping door is
healed by the first check that sees the item.

### Sync

The sync door (#12591) answers `POST /v1/sync/project` and
`POST /v1/sync/queue-github`: it hosts project-reconcile's write set
and the named queue/GitHub flows, as marker-checked posts and serial
operations. The package README's skeleton-era table still describes
these routes as stubs answering `501 not_implemented`; the code
disagrees — `src/capabilities.ts` marks the door live and
`src/routes/sync.ts` implements it — and this chapter follows the
code (disagreement D31).

## Sessions and the CLI

The session registry (#12584) answers
`POST /v1/sessions{,/resume,/resolve,/close}`. Session ids are
server-minted in the native #9075 format: `ses_` plus 128 bits in hex.
The CLI's session resolution order is explicit argument >
`FLEET_SESSION_ID` > `OPENCODE_SESSION_ID` > per-workspace stored
state, with a parity test transcribed from the native resolver's own
tests. Another tenancy's session answers identically to an unknown
one (404 `session_unknown`) — there is no existence oracle — and a
closed session is a 409 `session_closed`.

The package also carries a thin TypeScript CLI (`cli.ts` /
`src/cli.ts`, #12588) whose verb set is a fixed, caller-owned
allowlist, and the two public routes `GET /health` and
`GET /capabilities`; `/capabilities` mirrors the native
`fleet capabilities` JSON shape.

## The direct-read path

The direct-read mint (#12592) issues scoped, expiring, read-only
libsql tokens bound to the metadata database only. The token claims
are `a: "read-only"`, `id`, `binding_id` and `exp`, and the package
ships a scope inspector beside the mint. The queue database is
refused: the mint is for the metadata store, not the broker's store.
The wiring today is **library only** — no HTTP route and no CLI verb
expose the mint (grep-verified in the survey).

## Seams: why implemented is not deployed

`src/worker.ts` wires fixture, in-memory, module-scope
implementations of every store interface:

| Seam in the Worker | Production home |
| --- | --- |
| `FixtureSessionRegistry` | The session registry's production store |
| `FixtureMetadataStore` | The #9074 metadata database |
| `FixtureQueueStore` | The existing `fleet-queue` Turso database (#6741), behind a libSQL `QueueStore` |
| The fixture tenancy directory (`TENANCY_FIXTURE_JSON`) | Component E's tenancy database, which does not exist yet |

`wrangler.toml` declares no bindings at all — no D1, KV, R2, Durable
Objects or queues — and `INGESTION_REPOS` is commented out, so the
scheduled sweep currently has nothing to scan. A later component-C
issue owns the production deployment; neither `wrangler.toml` nor the
README names its number, and this chapter does not invent one.

Verification is in-package: there is no GitHub Actions workflow for
fleet-hosted. `npm test` runs node --test over 18 test files with no
network and no wrangler, `npm run smoke` runs the acceptance checks
end to end in-process, and `npm run typecheck` covers the types.

## Failure shapes

Every refusal is a `FailClosed` rendered as
`{ "error": <code>, "message": ... }`:

| Shape | Meaning |
| --- | --- |
| 404 `not_found` | Unmatched path; the Worker advertises nothing |
| 401 `missing_token` | No credential on a route that requires one |
| 403 | Cross-tenant assertion, or a service token outside its door scope |
| 503 | One row per unconfigured dependency: auth, stamp signing (refused before any GitHub write), session registry, metadata store, queue store, webhook secret |
| 500 `{error: "internal"}` | An unhandled error |

Door-specific shapes sit on top of the plumbing ones: sessions add
404 `session_unknown`, 409 `session_closed` and 400
`invalid_session_id`, and an agent lane beyond the caller's credential
is refused; stamping reports `metadata_inserted: false` instead of
failing the create; queue verbs answer `{ok:false, exit, error}`
envelopes at HTTP 200 while plumbing refusals stay 400/503, and
fallback-claim answers 200/409/400/502 by failure class. The rule that
runs through every door: unconfigured means fail closed, never fall
open.

## Sources

Package `pkgs/fleet/hosted-worker/` (`wrangler.toml`, `src/config.ts`,
`src/router.ts`, `src/routes/`, `src/auth.ts`, `src/jwt.ts`,
`src/tenancy.ts`, `src/capabilities.ts`, `src/worker.ts`); design
#9081; issues #9078, #12583–#12593; platform registry
`pm-stack/platform/workers.json`. Survey: programme #13900, lane
FLEET-GAPS-SURVEY, 2026-11-23 (disagreements D31 and D35).
