# Survey dump — the mail worker (`mail`, id-mail worker)

Source tree: `nixos/services/id-mail/worker/` in developing-today/code.
Files in the package (from `find`): `cloudflare-mail.sh` is one level up in
`nixos/services/id-mail/`; the worker dir holds `config.json`,
`cloudflare.config.ts`, `DEPLOYMENT.md`, `package.json`, `package-lock.json`,
`deploy.nix`, `deploy.sh`, `deploy.mjs`, `deploy.test.mjs`,
`upload-version.mjs`, `upload-version.test.mjs`, `promote.mjs`,
`promote.test.mjs`, `verify-upload-run.mjs`, `runtime.test.mjs`,
`.gitignore`, `src/index.js`, `src/limiter.js`, `src/transports/email.js`.

There is NO `wrangler.toml` in the package (see disagreements MW-3).

## config.json (verbatim)

```json
{
  "worker": "mail",
  "compatibilityDate": "2026-10-01",
  "workersDev": true,
  "routes": ["mail.security.cab", "mail.id.security.cab", "id-mail.security.cab", "mail-id.security.cab"],
  "retire": ["id-mail"],
  "senders": ["id@security.cab"],
  "defaultTransport": "email",
  "transports": ["email"],
  "limits": {
    "windowSeconds": 60,
    "callerPerWindow": 60,
    "recipientPerWindow": 5
  },
  "templates": [
    {
      "subject": "id: confirm your address",
      "body": "Your code is {code}. It works for 15 minutes.\n"
    },
    {
      "subject": "id: sign-in code",
      "body": "Your code is {code}. It works for 15 minutes.\n"
    }
  ],
  "callers": {
    "amd": {
      "tokenKey": "cloudflare.id-mail-worker-token",
      "transports": {
        "email": { "mode": "template", "from": ["id@security.cab"], "maxRecipients": 1 }
      }
    },
    "admin": {
      "tokenKey": "cloudflare.id-mail-admin-token",
      "transports": {
        "email": { "mode": "free", "from": ["id@security.cab"], "maxRecipients": 5 }
      }
    }
  }
}
```

## cloudflare.config.ts (the cf-CLI config path)

- Imports `bindings, defineConfig, exports as cfExports` from `cf/config`;
  entrypoint is `./src/index.js` imported `with { type: "cf-worker" }`.
- Reads `config.json`; builds `mailConfig` = exactly `{defaultTransport,
transports, limits, templates}` from it (callers/routes/senders NOT in it).
- `defineConfig({ worker: { name: config.worker, compatibilityDate,
entrypoint, exports: { Limiter: cfExports.durableObject({ state: "created",
storage: "sqlite" }) }, env: { MAIL_CONFIG: bindings.text(JSON.stringify(mailConfig)),
EMAIL: bindings.sendEmail({ allowedSenderAddresses: config.senders }),
LIMITER: bindings.durableObject({ worker: config.worker, exportName: "Limiter" }),
CALLERS: bindings.secret() } } })`.

## package.json (verbatim)

```json
{
  "name": "mail",
  "private": true,
  "type": "module",
  "devDependencies": {
    "cf": "1.0.0-rc.2",
    "wrangler": "4.166.0"
  },
  "scripts": {
    "test": "node --test deploy.test.mjs upload-version.test.mjs promote.test.mjs runtime.test.mjs"
  }
}
```

## src/index.js — handler behaviour (read in full)

- Exports `Limiter` from `./limiter.js`; `TRANSPORTS = { email }`.
- `fetch(request, env)` order of operations:
  1. Method !== POST → `405 {"error":"method not allowed"}`.
  2. `!env.CALLERS || !env.MAIL_CONFIG` → `500 {"error":"worker is not configured"}`.
  3. `parseMailConfig` / `parseCallers` fail → same 500. (Comment cites #6970:
     malformed/wrong-shape operator JSON is a config error in the handler's
     own vocabulary, never a raw SyntaxError/TypeError escaping fetch, never
     substring-matched into acceptance. Grants validated at this boundary.)
  4. `authenticate` fails → `401 {"error":"unauthorized"}`.
  5. Bounded body read: `MAX_BODY_BYTES = 64 * 1024`; `readTextUpTo` checks
     declared content-length first, then streams and cancels past the cap →
     `413 {"error":"body is too large"}` (comment cites #11331).
  6. JSON parse fail → `400 body is not JSON`; non-plain-object →
     `400 body must be a JSON object`.
  7. `name = body.transport ?? config.defaultTransport`; not a string / not in
     `config.transports` / not in TRANSPORTS → `400 unknown transport`;
     not in caller's grants → `403 caller may not use this transport`.
  8. `transport.check(body, grant, config)` refusal → `{error, status}` from
     the transport.
  9. Rate limits: caller key first — `take(env, "caller:" + caller.name,
limits.callerPerWindow, windowMs)`; denial → `429 rate limited`, final,
     before any recipient DO work (comment cites #5739). Then recipient keys
     `transport.rateKeys(body)` → `take(env, "target:" + name + ":" + key,
limits.recipientPerWindow, windowMs)` in parallel; any false → 429.
  10. `transport.deliver(body, grant, env)` → 200 with its return value;
      throw → `502 {"error": error.code ?? "send_failed", "message": error.message}`.
- `take()`: `env.LIMITER.get(env.LIMITER.idFromName(key))`, POSTs
  `https://limiter.internal/` with `{limit, windowMs}`, returns `.success`.
- `parseMailConfig` validation: transports must be array of strings;
  defaultTransport string if present; templates (defaulted to `[]` if absent)
  array of `{subject: string, body: string}`; limits must be a plain object
  with `windowSeconds`, `callerPerWindow`, `recipientPerWindow` each a finite
  number ≥ 0 (`isLimit`).
- `parseCallers` validation: each caller a plain object with `sha256: string`
  and `transports` a plain object; each grant `{mode: string, from: string[],
maxRecipients: isLimit}`.
- `authenticate`: `Authorization: Bearer <token>` regex; SHA-256 hex of token
  via `crypto.subtle.digest`; compared per caller with `sameText` (length
  check + XOR-accumulate over char codes — constant-time-style). Returns
  `{name, ...caller}`.
- `json()` helper: JSON body, `content-type: application/json`.

## src/transports/email.js — transport (read in full)

Constants: `EMAIL_ADDRESS = /^[^\s@,;<>"]+@[^\s@,;<>"]+\.[^\s@,;<>"]+$/`,
`MAX_ADDRESS = 254`, `MAX_SUBJECT = 200`, `MAX_TEXT = 10000`,
`PLACEHOLDER = "{code}"`.

`check(body, grant, config)` refusal order:

1. Grant/config shape wrong → refuse(500, "caller grant is misconfigured").
2. `from = body.from ?? grant.from[0]`; recipients = `recipientList(body)`
   (`to` string → [to]; array of strings → itself; else null). If no list or
   subject/text/from not strings → 400 "to, subject, text and from must be strings".
3. CR/LF in subject, from, or any recipient → 400 "to, subject and from must be one line".
4. `recipients.length === 0 || > grant.maxRecipients` → 400
   `to must have 1 to ${grant.maxRecipients} addresses`.
5. Any recipient failing EMAIL_ADDRESS or > 254 chars → 400 "to must be email addresses".
6. subject > 200 or text > 10000 → 400 "subject or text is too long".
7. `!grant.from.includes(from)` → 403 "caller may not send from that address".
8. `grant.mode === "template"` and no template matches → 400 "template"
   (the literal error string is the single word `template`).
   Otherwise null (pass).

`matchesTemplate`: subject must be exactly equal; body pattern = template
body split on `{code}`, regex-escaped parts joined with `\d{6}`, anchored
`^…$`. So the code slot is exactly six digits.

`rateKeys(body)`: lowercased recipient addresses.

`deliver(body, grant, env)`: sequential `env.EMAIL.send({from, to, subject,
text: body.text})` per recipient; collects `sent.messageId`; returns
`{messageIds}`. NOTE (survey observation, stated in the chapter): a failure
part-way through a multi-recipient send has already delivered to earlier
recipients; no rollback and no per-recipient outcome in the 502.

## src/limiter.js — Limiter Durable Object (read in full)

- Comment: "One instance per limited key, so the count is exact; the Rate
  Limiting binding is per machine and too loose for a recipient cap."
- `fetch`: reads `{limit, windowMs}`; `now = Date.now()`; gets `count`,
  `windowStart` from storage; window expired → count=0, windowStart=now;
  `count >= limit` → `{success:false}`; else put `{count+1, windowStart}`,
  `setAlarm(windowStart + windowMs)`, return `{success:true}`.
- `alarm()`: `storage.deleteAll()` — bounds state lifetime (#5739 comment).

## deploy.mjs — operator deploy path (read in full)

- Header: "Validates config.json, generates the Wrangler config, retires old
  Workers, deploys, and sets CALLERS. Usage: deploy.mjs [--dry-run]. Needs
  CLOUDFLARE_ACCOUNT_ID and BWS_ACCESS_TOKEN; deploy.sh sets them."
- Strict argv validation (#3281 comment): only `deploy.mjs [--dry-run]`
  accepted; unknown args → `unrecognized argument(s): …` + usage, exit 2;
  duplicated `--dry-run` → `duplicate argument(s)` (#7855), exit 2. (The old
  `argv.includes` check silently treated typos as a real deploy.)
- `validate(config)` rules: worker `/^[a-z0-9-]+$/`; compatibilityDate
  YYYY-MM-DD; workersDev boolean; routes non-empty string list; retire string
  list not containing the worker itself; senders non-empty email list;
  transports non-empty, each in KNOWN_TRANSPORTS=["email"], includes
  defaultTransport; limits positive integers; templates list, each subject
  one line, each body containing `{code}`; callers non-empty; per caller
  tokenKey string, ≥1 transport grant; per grant: transport enabled, mode in
  ["template","free"], from non-empty and ⊆ senders, maxRecipients positive
  int, template mode requires ≥1 template.
- Secrets: `bws secret list -o json`; `secretValue(key)` throws if missing.
  API token from Bitwarden key `cloudflare.api-token-account-wide` (value
  never reproduced here). Per caller: `sha256 = createHash("sha256")
.update(secretValue(caller.tokenKey)).digest("hex")`; CALLERS =
  `{name: {sha256, transports}}`.
- Generated wrangler config (`.generated/wrangler.json`): `{name, main:
"../src/index.js", compatibility_date, workers_dev, routes: [{pattern,
custom_domain: true}], vars: {MAIL_CONFIG}, send_email: [{name: "EMAIL",
allowed_sender_addresses: senders}], durable_objects: {bindings: [{name:
"LIMITER", class_name: "Limiter"}]}, migrations: [{tag: "v1",
new_sqlite_classes: ["Limiter"]}]}`.
- Sequence: `wrangler deploy --dry-run` always; if `--dry-run`, log and exit 0. Else: for each `retire` name, `retire(name)` BEFORE deploy ("A hostname
  can belong to only one Worker"); then `wrangler deploy`; then
  `wrangler secret put CALLERS` with the JSON. Logs
  `deployed ${worker} with callers: …`.
- `retire(name)`: GET `https://api.cloudflare.com/client/v4/accounts/${accountId}/workers/scripts/${name}`
  with the API token; 404 → already gone; !ok → throw; else
  `wrangler delete --name <name> --force`.

## deploy.sh / deploy.nix — operator entry

- `deploy.sh`: `set -euo pipefail`; cd to script dir; sources
  `${repo_root}/lib/bitwarden-env.sh`; `CLOUDFLARE_ACCOUNT_ID` from
  `sops decrypt --extract '["cloudflare_account_id"]' secrets/sops/common/cloudflare.yaml`;
  `exec node deploy.mjs "$@"`.
- `deploy.nix`: `writeShellApplication` named `id-mail-deploy`, runtimeInputs
  [bws, git, jq, nodejs, sops, wrangler]; text resolves repo root via git and
  execs `deploy.sh`.

## DEPLOYMENT.md — publish/promote workflows (read in full; key facts)

- `id-mail-deploy` and its rollback/fallback path unchanged by the workflows.
- Publish workflow (`id-mail-worker-publish`): manual, main-only, confirmation
  input defaults false. Uploads a version with pinned `cf` CLI + Workers
  Versions API; does NOT deploy. Artifact bound to exact run+attempt: version
  ID, unique run tag, commit SHA, active deployment ID/version observed
  before upload. Upload fails if the active deployment changes during upload.
  Requires latest uploaded version == active version, so `CALLERS` inherits
  from the current version; Workers API strict inheritance check fails the
  upload if the secret cannot be resolved.
- Promote workflow (`id-mail-worker-promote`): manual, main-only, default-off.
  Takes a successful upload run ID + attempt; verifies that exact GH Actions
  attempt completed successfully from main under the publisher workflow;
  downloads its artifact; verifies artifact commit SHA, exact Cloudflare
  version ID + tag, unchanged active deployment, inherited CALLERS secret
  binding descriptor, unchanged existing version bindings + versioned
  settings. Probes each current configured route with an unauthenticated
  POST expecting 401 (returned after CALLERS/MAIL_CONFIG validation, before
  DO or email access; no body/auth header, so it cannot send). Creates a 100%
  deployment of only the selected version; re-checks active deployment +
  binding/settings metadata; repeats route probes.
- Both workflows: pinned `cf` CLI (pinned Wrangler build backend), never
  direct Wrangler deploy. Promotion uses `cf workers deployments create`.
  Neither calls route/trigger operations, changes script settings, writes
  secrets, or retires/deletes Workers. "MAIL_CONFIG, EMAIL, LIMITER, the
  SQLite Limiter export, and CALLERS remain as declared in
  cloudflare.config.ts. Routes and the existing workers.dev setting are not
  part of version promotion. The existing retire list in config.json is not
  acted on." (Scoped to these workflows — see MW-2.)
- Required setup: repo variable `CLOUDFLARE_ACCOUNT_ID`; protected GitHub
  environment `mail-worker-production` (main only, independent reviewer, no
  self-review/bypass where supported); Cloudflare credential ONLY as
  environment secret `ID_MAIL_WORKER_CF_TOKEN_MAIL_WORKER_PRODUCTION` (unique
  name must not exist as repo/org secret; workflows never reference generic
  `CLOUDFLARE_API_TOKEN`). Token: Individual Workers Editor permission,
  resource-scoped to the account and the existing `mail` Worker only.
- DEPLOYMENT.md states the repo environment list "currently does not contain
  `mail-worker-production`" at its writing; do not dispatch until created.
  (Chapter repeats this as the package's own record, not a live check.)
- Scope notes: confirm `mail` Worker already exists (upload is not
  bootstrap); publish manually on main with `confirm_upload=true`, retain run
  ID + attempt; promote with same run ID/attempt + `confirm_promotion=true`
  after review; promotion fails closed on drift; "No caller, route, binding,
  secret, or Worker retirement changes are included in this cutover."

## GitHub workflows (from .github/workflows/)

- `id-mail-worker-publish.yml` — name "id-mail worker publish"; concurrency
  group `id-mail-worker-lifecycle`; working dir
  `nixos/services/id-mail/worker`; env includes
  `ID_MAIL_WORKER_CF_TOKEN_MAIL_WORKER_PRODUCTION` from the same-named
  secret, `ID_MAIL_WORKER_PROVENANCE_FILE`; artifact name
  `id-mail-worker-upload-${{ github.run_id }}-${{ github.run_attempt }}`.
- `id-mail-worker-promote.yml` — name "id-mail worker promote"; input
  description "Successful id-mail publish run ID"; same concurrency group;
  downloads artifact `id-mail-worker-upload-<run>-<attempt>`; runs
  `node promote.mjs --manifest …/id-mail-worker-provenance.json --promote`
  with env `ID_MAIL_WORKER_CF_TOKEN_MAIL_WORKER_PRODUCTION`,
  `ID_MAIL_UPLOAD_RUN_ID`, `ID_MAIL_UPLOAD_RUN_ATTEMPT`,
  `ID_MAIL_VERIFIED_UPLOAD_RUN_FILE`.
- `id-mail-worker-tests.yml` — name "id-mail worker tests"; comment cites
  #6351; runs on changes to `nixos/services/id-mail/worker/` and on dispatch;
  explicitly errors if `runtime.test.mjs`, `deploy.test.mjs` or
  `package.json` is missing (so a missing suite cannot pass silently).

## runtime.test.mjs — test names (grep of test titles)

- "caller denial returns 429 before any recipient Durable Object work"
- "caller-allowed requests deliver and consult recipient limiters"
- "recipient limit still rejects an over-limit caller-allowed request"
- "per-request recipient cap still applies"
- "limiter state is alarm-bounded and alarm cleans up dormant keys"
- "malformed or wrong-shape MAIL_CONFIG fails closed with the 500 envelope (#6970)"
- "malformed or wrong-shape CALLERS fails closed with the 500 envelope (#6970)"
- "template mode with no templates configured refuses instead of throwing (#6970)"
- "an oversized declared body is refused with 413 without being read (#11331)"
- "a drip-fed oversized body stops being read at the cap (#11331)"
- "bad JSON and non-object bodies keep their 400 envelopes after the bounded read (#11331)"
  (Plus promote.test.mjs pinning: CALLERS inherited by name+type, secret
  contents never compared; upload-version.test.mjs pinning the
  ID_MAIL_WORKER_CF_TOKEN_MAIL_WORKER_PRODUCTION requirement.)

## Host wiring — nixos/services/id-mail/default.nix (read in full)

- `mailSender`: `writeShellApplication` named `id-cloudflare-mail`,
  runtimeInputs [curl, jq], text = `./cloudflare-mail.sh`.
- Options: `services.id-mail.transport` enum ["rest","worker"], default
  "rest", description "How confirmation mail leaves: the Cloudflare Email
  Sending REST API, or the mail Worker's send_email binding.";
  `services.id-mail.workerUrl` nullOr str, default null, "Required when
  transport is worker." Assertion enforces it.
- `mailEnv` sops template `id-mail.env`: rest →
  `ID_MAIL_TRANSPORT=rest`, `CLOUDFLARE_API_TOKEN` (sops placeholder
  cloudflare_email_sending_token), `CLOUDFLARE_ACCOUNT_ID` (placeholder
  cloudflare_account_id); worker → `ID_MAIL_TRANSPORT=worker`,
  `ID_MAIL_WORKER_URL=${cfg.workerUrl}`, `ID_MAIL_WORKER_TOKEN` (placeholder
  id_mail_worker_token). Both append `ID_WORLD_ADMIN_TOKEN` placeholder.
  (Secret VALUES are sops placeholders in the source; none reproduced here.)
- sops secrets: `cloudflare_email_sending_token`, `cloudflare_account_id`,
  `id_mail_worker_token`, all from `secrets/sops/common/cloudflare.yaml`.
- Wires `services.id`: package `inputs.self.packages.<system>.id-web`;
  `instances.primary` enable, port 3005, extraArgs `--world`,
  `--world-mail-command <mailSender>/bin/id-cloudflare-mail`;
  `systemd.services.id-primary.serviceConfig.EnvironmentFile` = the
  `id-mail.env` template path.
- Imports `pkgs/id/nix/id-module.nix` via `lib.from-root`.

## cloudflare-mail.sh (read in full)

- Header: "Sends one confirmation mail. Run by `id serve
--world-mail-command`, which sets ID_MAIL_TO, ID_MAIL_SUBJECT and
  ID_MAIL_BODY." Requires those three (`:?`). `transport=${ID_MAIL_TRANSPORT:-rest}`.
- rest: requires CLOUDFLARE_API_TOKEN + CLOUDFLARE_ACCOUNT_ID;
  `from=${ID_MAIL_FROM:-id@security.cab}`; jq payload `{from,to,subject,text}`;
  curl to `https://api.cloudflare.com/client/v4/accounts/$CLOUDFLARE_ACCOUNT_ID/email/sending/send`
  with the Authorization header passed via `curl -K -` on stdin (not argv);
  success check: `jq -e '.success and ((.result.permanent_bounces | length) == 0)'`.
- worker: requires ID_MAIL_WORKER_URL + ID_MAIL_WORKER_TOKEN; payload
  `{to,subject,text}` (no from); POST to `$ID_MAIL_WORKER_URL`, same `-K -`
  header route, output discarded; `curl -fsS` makes non-2xx a failure.
- Unknown transport → stderr + exit 2.

## Live host selection — nixos/hosts/default.nix

- `nixos/hosts/default.nix` imports "services/id-mail" and sets, in the host
  modules list: `services.id-mail = { transport = "worker"; workerUrl =
"https://mail.security.cab"; }`. So the production node uses the worker
  transport at the `mail.security.cab` route.

## Native side (id) — where codes and mail sinks live

- `pkgs/id/crates/id-directory/src/directory_view.rs`: `code_mail(address,
code, subject)` builds `Mail { to, subject: format!("id: {subject}"), body:
format!("Your code is {code}. It works for 15 minutes.\n") }`; call sites
  pass "confirm your address" (confirmation) and "sign-in code" (sign-in) —
  exactly the two worker templates. `directory_auth.rs` holds the
  confirmation/sign-in code state (independent slots per purpose, per its
  test `confirmation_and_sign_in_codes_have_independent_slots`).
- `pkgs/id/crates/id-directory/src/directory_mail.rs`: module doc "Where
  confirmation mail goes" — a world is configured with an outbox directory, a
  program to run, a relay on loopback, or nothing (nothing → email actions
  refused). `Mail {to, subject, body}`; `validate_header_fields` (ASCII
  recipient, plain address, no control chars in subject). `MailSink` trait;
  `OutboxSink` writes `<8 random bytes hex>.eml` mode 0600 create_new;
  the command sink sets env `ID_MAIL_TO`, `ID_MAIL_SUBJECT`, `ID_MAIL_BODY`
  (its `send_with_timeout`, ~line 134-141) and kills the child's whole
  process group on timeout.
- Book's existing coverage of the sink side: reference/cli-serve.md
  (`--world-mail-outbox/-command/-smtp/-from` rows; the three sinks mutually
  conflicting, all requiring `--world`; SMTP loopback-only, requires
  `--world-mail-from`), reference/configuration.md (ID_MAIL_TO/SUBJECT/BODY
  as variables the process writes), operations/services.md (same).

## Relationship checks (grep evidence)

- `grep -rn "worker-poc|2271" pkgs/id/hosted-worker/ pkgs/id/src/
pkgs/id/crates/` → no hits: no code relationship between mail worker /
  worker-poc and the hosted worker package.
- The mail worker is referenced from the book (pre-existing) only at:
  development/release-and-rollout.md (the publish/promote workflows "concern
  the separate mail worker; their existence is not evidence that
  id-hosted-worker is published or promoted") and research/releases.md
  (workflow inventory). No chapter covered it — census GAP confirmed.
