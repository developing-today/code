# The mail Worker API

Source: https://github.com/developing-today/code/blob/main/nixos/services/id-mail/worker/src/index.js, https://github.com/developing-today/code/blob/main/nixos/services/id-mail/worker/src/transports/email.js, https://github.com/developing-today/code/blob/main/nixos/services/id-mail/worker/src/limiter.js, https://github.com/developing-today/code/blob/main/nixos/services/id-mail/worker/config.json, https://github.com/developing-today/code/blob/main/nixos/services/id-mail/worker/cloudflare.config.ts, https://github.com/developing-today/code/blob/main/nixos/services/id-mail/worker/DEPLOYMENT.md, https://github.com/developing-today/code/blob/main/nixos/services/id-mail/default.nix, https://github.com/developing-today/code/blob/main/nixos/services/id-mail/cloudflare-mail.sh

This chapter documents the ID mail Worker **as it stands**: the production mail surface for ID, in `nixos/services/id-mail/worker/`. It is a different package, with a different status, from the hosted edition in [The hosted Worker API](hosted-worker.md): that Worker is a local-only scaffold that is not deployed, while this one is the Worker the live ID node's mail wiring points at.

| Item | Value |
|---|---|
| Worker name | `mail` (the `worker` key of `config.json`; the package name in `package.json` is also `mail`) |
| Source | `nixos/services/id-mail/worker/` — entry point `src/index.js`, one transport in `src/transports/email.js`, the rate limiter in `src/limiter.js` |
| Compatibility date | `2026-10-01` (`compatibilityDate` in `config.json`) |
| `workers.dev` | `true` (`workersDev` in `config.json`) |
| Retires | `id-mail` (the `retire` list in `config.json`; see "Deployment" below for which path acts on it) |
| Sender | `id@security.cab` (the only entry in `senders`, and the only address either caller may send from) |

There is **no `wrangler.toml`** in this package. The configuration of record is `config.json`, consumed two ways: `cloudflare.config.ts` builds the Worker definition for the pinned `cf` CLI from it, and `deploy.mjs` generates a Wrangler configuration (`.generated/wrangler.json`) from it for the operator deploy path. A reader looking for a Wrangler file to edit will not find one; `config.json` is the file to change, and [Keeping this book current](../development/keeping-this-book-current.md) applies to it.

## What it does, and for whom

The Worker sends ID's address-confirmation and sign-in mail. It does not generate codes, hold accounts, or decide who may confirm an address: all of that is native ID code. The flow, end to end:

1. The directory code in `pkgs/id/crates/id-directory/` generates a confirmation or sign-in code and builds the message with `code_mail` in `directory_view.rs`: subject `id: confirm your address` or `id: sign-in code`, body `Your code is {code}. It works for 15 minutes.` with the code substituted. Code generation, the 15-minute lifetime, and the check when a code is presented all live on that native side.
2. `id serve` hands the message to its configured mail sink. The sinks are the three in [serve and publications](cli-serve.md); the production node uses the command sink, `--world-mail-command`, which sets `ID_MAIL_TO`, `ID_MAIL_SUBJECT` and `ID_MAIL_BODY` in the child process environment (see [Configuration and environment](configuration.md)).
3. On the live host, that command is `id-cloudflare-mail`, built from `nixos/services/id-mail/cloudflare-mail.sh` by the NixOS module `nixos/services/id-mail/default.nix`. The module wires it into the primary `id` instance (port `3005`, `--world`) and supplies its environment from a sops-rendered file.
4. The script POSTs the message to this Worker (its `worker` transport; see "The host side: two transports" below).
5. The Worker authenticates the caller, checks the message against that caller's grant and — for the template-mode caller — against the configured templates, applies the rate limits, and delivers through its Cloudflare send-email binding, one send per recipient. It returns the message IDs Cloudflare issued.

The template check is the Worker's part of the confirmation design: the production caller may only send messages whose subject matches a configured template exactly and whose body matches that template with a single six-digit code in the `{code}` position. A compromised or misbehaving caller in template mode therefore cannot send arbitrary text through this Worker; it can only send the two code messages, to one recipient at a time.

## Routes

`config.json` declares four routes, each attached as a custom domain by the generated configuration:

| Route | Notes |
|---|---|
| `mail.security.cab` | The route the live host configuration uses: `nixos/hosts/default.nix` sets `services.id-mail.workerUrl = "https://mail.security.cab"`. |
| `mail.id.security.cab` | Also declared in `config.json`. |
| `id-mail.security.cab` | Also declared in `config.json`. |
| `mail-id.security.cab` | Also declared in `config.json`. |

All four routes reach the same Worker and the same single handler: the handler does not branch on the host. There is exactly one HTTP route in the code — `POST` on any path — specified under "The request" below. The promotion workflow's health probe (see "Deployment") POSTs to each configured route and expects the unauthenticated `401` described there.

## The request

Implementation: `src/index.js`. The handler accepts one shape:

```text
POST /            (any path; only the method is checked)
Authorization: Bearer <caller token>
Content-Type: application/json

{"to": "ada@example.org", "subject": "id: confirm your address", "text": "Your code is 123456. It works for 15 minutes.\n"}
```

The toy values above are invented; the subject and body shape are the first configured template with a six-digit code.

| Field | Shape | Rules (enforced in `src/transports/email.js` unless noted) |
|---|---|---|
| `to` | string, or array of strings | 1 to the caller's `maxRecipients` addresses. Each address must match the transport's address pattern, be at most 254 characters, and contain no line break. Rate-limit keys are the lowercased addresses. |
| `subject` | string | One line, at most 200 characters. |
| `text` | string | At most 10,000 characters. |
| `from` | string, optional | Defaults to the first address in the caller's grant. Must be one line and must be in the caller's grant `from` list, or the request is refused with `403`. |
| `transport` | string, optional | Defaults to `defaultTransport` from the configuration (`email`). Must be an enabled transport the caller is granted, or the request is refused (`400` unknown, `403` not granted). `email` is the only transport that exists. |

The whole body is capped at 64 KiB (`MAX_BODY_BYTES` = 64 × 1024 in `src/index.js`): a body that declares or streams more than that is refused with `413` before it is buffered whole or parsed (#11331).

On success the Worker answers `200` with `{"messageIds": [...]}` — one Cloudflare message ID per recipient, in recipient order, from `env.EMAIL.send({from, to, subject, text})`.

## Authentication and caller grants

Authentication is a bearer token per caller, checked against the `CALLERS` secret binding:

- The Worker SHA-256-hashes the presented token and compares the hex digest against each caller's stored `sha256` in `CALLERS`, using a constant-time-style comparison over the digest text. The secret therefore holds digests, never the tokens themselves.
- `deploy.mjs` builds `CALLERS` at deploy time: for each caller in `config.json` it reads the token from the Bitwarden secret named by that caller's `tokenKey` and stores only its SHA-256 hex.
- A missing or unmatched token is `401 {"error":"unauthorized"}`. This response is also the deployment health signal: the promotion workflow probes each route with an unauthenticated POST and expects exactly this `401`, returned after the Worker has validated its `CALLERS` and `MAIL_CONFIG` bindings and before it touches the Durable Object or the email transport, so the probe can never send mail.

The callers declared in `config.json`:

| Caller | Bitwarden token name (`tokenKey`) | Mode | `from` | `maxRecipients` |
|---|---|---|---|---|
| `amd` | `cloudflare.id-mail-worker-token` | `template` | `id@security.cab` | 1 |
| `admin` | `cloudflare.id-mail-admin-token` | `free` | `id@security.cab` | 5 |

- `template` mode (`amd`, the production node's caller): subject and body must match a configured template, as described above.
- `free` mode (`admin`): any subject and body that pass the general field checks may be sent, to up to five recipients.

Grants are validated twice: `deploy.mjs` validates `config.json` before deploying (modes must be `template` or `free`, grant senders must be within `senders`, recipient caps must be positive integers, template mode requires templates), and the Worker re-validates the shape of both `MAIL_CONFIG` and `CALLERS` on every request, failing closed with `500` rather than acting on a with a value of the wrong shape (#6970).

## Configuration and environment reference

The Worker reads four bindings, declared in `cloudflare.config.ts` and mirrored in the Wrangler configuration `deploy.mjs` generates:

| Binding | Type | Content |
|---|---|---|
| `MAIL_CONFIG` | Text (plain-text var in the generated Wrangler config) | JSON built from `config.json` by both config paths, containing exactly `defaultTransport`, `transports`, `limits` and `templates`. Caller definitions and routes are **not** in it. |
| `CALLERS` | Secret | JSON object keyed by caller name: `{sha256, transports}` per caller, where `transports` maps a transport name to its grant `{mode, from, maxRecipients}`. Set by `wrangler secret put CALLERS` on the operator deploy path; inherited from the current active version on the workflow publish path. |
| `EMAIL` | Send-email binding | `allowedSenderAddresses` is the `senders` list from `config.json`: `id@security.cab` only. |
| `LIMITER` | Durable Object namespace | The `Limiter` class exported from `src/limiter.js`, in the same Worker. Migration tag `v1`, SQLite-backed (`new_sqlite_classes = ["Limiter"]` in the generated config). |

The `MAIL_CONFIG` values as configured today, from `config.json`:

| Key | Value |
|---|---|
| `defaultTransport` | `email` |
| `transports` | `["email"]` |
| `limits.windowSeconds` | `60` |
| `limits.callerPerWindow` | `60` |
| `limits.recipientPerWindow` | `5` |
| `templates[0]` | subject `id: confirm your address`; body `Your code is {code}. It works for 15 minutes.` plus a trailing newline |
| `templates[1]` | subject `id: sign-in code`; body as above |

A template matches when the subject is exactly equal and the body matches the template body with `{code}` replaced by exactly six digits (`\d{6}`) and nothing else changed. The two templates are the same two messages the native `code_mail` function builds; changing one side without the other breaks confirmation mail for the template-mode caller, in the `400 {"error":"template"}` shape below.

## Rate limits

Limits are enforced by the `Limiter` Durable Object, one object instance per limited key, because — as the comment in `src/limiter.js` states — the platform rate-limiting binding is per-machine and too loose for a recipient cap. Each instance keeps a fixed-window counter (`count`, `windowStart`) in its storage and sets an alarm at the end of the window that deletes the stored state, so dormant keys do not persist (#5739).

Order matters, and is deliberate in `src/index.js`:

1. The caller key (`caller:{name}`) is taken first, at `callerPerWindow` (60) per 60-second window. A caller denial is final: the handler returns `429` without resolving or mutating any recipient object (#5739).
2. Only then is each recipient key (`target:email:{lowercased address}`) taken, at `recipientPerWindow` (5) per window. If any recipient is over the limit, the request is refused with `429` — after the caller token has already been consumed for this window.

## Failure shapes

Every response is JSON of the form `{"error": ...}` except the success response. The complete set the handler and the email transport produce:

| Status | `error` | Cause |
|---|---|---|
| `405` | `method not allowed` | Any method other than `POST`. Checked first, before configuration or authentication. |
| `500` | `worker is not configured` | `CALLERS` or `MAIL_CONFIG` missing, unparseable, or with the wrong shape (#6970). |
| `401` | `unauthorized` | No bearer token, or its digest matches no caller. |
| `413` | `body is too large` | The body declares or streams more than 64 KiB (#11331). |
| `400` | `body is not JSON` | The body does not parse as JSON. |
| `400` | `body must be a JSON object` | The JSON value is not a plain object. |
| `400` | `unknown transport` | `transport` names no enabled, implemented transport. |
| `403` | `caller may not use this transport` | The transport exists but is not in the caller's grants. |
| `400` | `to, subject, text and from must be strings` | A required field is missing or not a string (including a `to` that is neither a string nor a list of strings). |
| `400` | `to, subject and from must be one line` | A line break in the subject, the sender, or a recipient. |
| `400` | `to must have 1 to <maxRecipients> addresses` | Recipient count outside the caller's grant. |
| `400` | `to must be email addresses` | An address fails the pattern or exceeds 254 characters. |
| `400` | `subject or text is too long` | Subject over 200 or text over 10,000 characters. |
| `403` | `caller may not send from that address` | `from` is not in the caller's grant. |
| `400` | `template` | Template mode: subject/body match no configured template (including a code that is not exactly six digits). The error text is the single word `template`. |
| `500` | `caller grant is misconfigured` | A grant reaches the transport with the wrong shape despite the boundary validation. |
| `429` | `rate limited` | Caller or recipient window exhausted, in the order above. |
| `502` | the send error's `code`, or `send_failed` | The send-email binding threw; the response also carries a `message` field with the error's message. A multi-recipient send that fails part-way has already delivered to the earlier recipients; the Worker does not roll back or report which recipients succeeded. |

There are no other routes or responses: no health endpoint, no GET surface, and no path routing — a `GET` to any route is the `405` above, and a `POST` to any path is the send handler.

## The host side: two transports

The Worker is one of two ways confirmation mail can leave the host. The script `id-cloudflare-mail` (source `cloudflare-mail.sh`, run by `id serve --world-mail-command` with `ID_MAIL_TO`, `ID_MAIL_SUBJECT` and `ID_MAIL_BODY` set) selects on `ID_MAIL_TRANSPORT`:

| Transport | Path | Credentials it reads |
|---|---|---|
| `rest` | POSTs `{from, to, subject, text}` directly to the Cloudflare Email Sending REST API, `https://api.cloudflare.com/client/v4/accounts/<account>/email/sending/send`, and checks the response reports success with no permanent bounces. `from` defaults to `id@security.cab` (`ID_MAIL_FROM` overrides). | `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_ACCOUNT_ID` |
| `worker` | POSTs `{to, subject, text}` — no `from` — to `ID_MAIL_WORKER_URL` with the bearer token, discarding the response body; `curl -f` makes any non-success status a script failure. The Worker's grant supplies the sender. | `ID_MAIL_WORKER_URL`, `ID_MAIL_WORKER_TOKEN` |

The NixOS module `services.id-mail` chooses between them: its `transport` option is `rest` or `worker` (default `rest`), `workerUrl` is required for `worker`, and it renders the chosen pair plus the world admin token into the sops template `id-mail.env` for the primary instance. Its secrets are `cloudflare_email_sending_token`, `cloudflare_account_id` and `id_mail_worker_token`. The live host configuration (`nixos/hosts/default.nix`) sets `transport = "worker"` and `workerUrl = "https://mail.security.cab"`, so the production node sends through this Worker, as the `amd` template-mode caller, one recipient at a time.

## Relationship to the hosted Worker and the IdP

- **To the IdP and directory:** the Worker is a transport, not an identity component. Accounts, codes, sessions and the confirmation decision are the native directory's (`pkgs/id/crates/id-directory/`); the Worker never sees a code except as six digits inside a template body it pattern-matches, never verifies a code, and keeps no account state — its only stored state is the `Limiter` counters.
- **To the hosted Worker:** none in code. [The hosted Worker API](hosted-worker.md) is a separate package (`pkgs/id/hosted-worker`, Worker name `id-hosted-worker`) implementing the per-user data plane of [The hosted edition](../concepts/hosted.md), local-only and undeployed. The mail Worker shares no source, bindings or routes with it, and the mail publish/promote workflows are not evidence about the hosted Worker's status — a point [Running a node: serve, worlds and the hosted edition](../operations/services.md) also makes from the operator side.
- **To the other mail sinks:** `id serve` can also write mail to an outbox directory or relay through a loopback SMTP server instead of running a command (see [serve and publications](cli-serve.md)). Those sinks bypass this Worker entirely; it exists for the host whose configured sink is the Cloudflare command.

## Operations

Tests: the package's `npm test` runs four Node suites — `runtime.test.mjs` (the handler contract: authentication, the failure envelopes above, template mode, the limiter ordering, the bounded body read), `deploy.test.mjs`, `upload-version.test.mjs` and `promote.test.mjs`. The workflow `id-mail-worker-tests.yml` runs them on changes to this package.

Deployment has two distinct paths, and `DEPLOYMENT.md` in the package is the operator record for the second:

1. **The operator command `id-mail-deploy`** (from `deploy.nix`, running `deploy.sh` → `deploy.mjs`). It validates `config.json`, generates `.generated/wrangler.json`, dry-runs the deploy, then — on a real run — retires each Worker in the `retire` list (`id-mail`) **before** deploying, because a hostname can belong to only one Worker; deploys `mail`; and sets the `CALLERS` secret from the Bitwarden tokens. `deploy.mjs` accepts exactly one optional argument, `--dry-run`; any other or duplicated argument aborts with exit `2` before any deploy work (#3281, #7855). `deploy.sh` supplies `CLOUDFLARE_ACCOUNT_ID` from sops, and `deploy.mjs` additionally needs `BWS_ACCESS_TOKEN` for the token lookups.
2. **The GitHub workflows `id-mail-worker-publish` and `id-mail-worker-promote`.** Both are manual, main-only, and default-off (a confirmation input defaults to false). Publish uploads a new version with the pinned `cf` CLI and the Workers Versions API but does **not** deploy it; it records a provenance artifact (version ID, run tag, commit SHA, the active deployment observed before upload) and fails if the active deployment changes during the upload or if the latest uploaded version is not the active one — so the new version inherits `CALLERS` from the current live version, never from an unpromoted upload. Promote takes a successful publish run ID and attempt, re-verifies the artifact, the exact version and tag, the unchanged active deployment, the inherited `CALLERS` binding descriptor and the version's bindings and settings; probes every configured route with the unauthenticated POST that must answer `401`; then creates a 100% deployment of only that version and repeats the metadata checks and probes. Neither workflow calls route/trigger operations, changes script settings, writes secrets, or retires or deletes Workers — the `retire` list is acted on only by the operator path above, which is the distinction `DEPLOYMENT.md`'s scope notes are making.

The workflows run in the protected GitHub environment `mail-worker-production` (main branch only, independent reviewer required) and read exactly one credential there, the environment-only secret named `ID_MAIL_WORKER_CF_TOKEN_MAIL_WORKER_PRODUCTION` — a Cloudflare API token with the Individual Workers Editor permission scoped to the account and the existing `mail` Worker. They never reference the generic `CLOUDFLARE_API_TOKEN`. `DEPLOYMENT.md` records that the environment did not yet exist in the repository's environment list at its writing and that neither workflow is to be dispatched until an administrator has created and protected it; this chapter repeats that as the package's own record, not as a live check. Publishing also assumes the `mail` Worker already exists: upload is not a bootstrap operation, and the repository variable `CLOUDFLARE_ACCOUNT_ID` must be set.

Rollback on the operator path is the path itself: `DEPLOYMENT.md` states the `id-mail-deploy` command and its rollback/fallback path are unchanged by the workflow cutover. On the workflow path, promotion is the only step that changes what serves, and it refuses to proceed against any drift from the uploaded artifact — an operator recovering from a bad promotion redeploys a known version rather than editing routes, bindings or secrets, none of which promotion touches.

## Worked example

Toy data: the production caller (`amd`, template mode), one recipient, the first template.

```text
POST https://mail.security.cab/
Authorization: Bearer <toy caller token>
{"to": "ada@example.org", "subject": "id: confirm your address", "text": "Your code is 123456. It works for 15 minutes.\n"}
→ 200  {"messageIds": ["<cloudflare message id>"]}

Same request with no Authorization header
→ 401  {"error": "unauthorized"}

Same request, body "Your code is 12345. It works for 15 minutes.\n" (five digits)
→ 400  {"error": "template"}

Same request, a sixth send to ada@example.org inside one minute
→ 429  {"error": "rate limited"}

GET https://mail.security.cab/
→ 405  {"error": "method not allowed"}
```
