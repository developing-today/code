# Queue transport alert Worker (`queue-transport-alert`)

Implements code#6369 off the broker host: a Cloudflare Worker that
alerts when the repo-queue broker is unreachable, so a total queue
outage (every lane exiting at its health gate) no longer goes unseen.

## What it does

- **Probe:** every 10 minutes (cron `*/10 * * * *`) it sends
  `GET https://queue-broker.security.cab/healthz` with the Cloudflare
  Access service-token headers. That route is the pm-stack tunnel to
  the broker's Tailscale listener, so it fails when the laptop is off,
  when tailscaled is stopped (the 2026-11-18 outage) or when the broker
  is down. A pass is healthy only on HTTP 200 with `{"ok":true}`.
- **Alert:** two failed probes in a row (`THRESHOLD`) file one GitHub
  issue in `REPO` titled `Queue transport incident: ...`, labelled
  `kind/bug`, `priority/high`, `area/infra`, `source/fleet-monitor`, or
  comment on an open one with that title prefix.
- **Recovery:** the first healthy probe after an alert comments once with
  the outage span. It never closes the issue: automation may label and
  comment but must not close an issue (#10136), so a human closes it.
- **Phone push (Web Push):** the alert and the recovery each send one
  standard Web Push to every device that turned on alerts on the status
  page (open it, tap **Turn on phone alerts**, allow notifications).
  Pushes carry no payload: the page's service worker reads
  `/status.json` (`last_push`) and shows the notification, so nothing
  needs encrypting. The VAPID key pair is made once inside the Durable
  Object and never leaves it. Devices that the push service reports
  gone (404/410) are dropped; at most 20 devices can subscribe.
- **Telegram:** the alert and the recovery also send one Telegram
  message to `TELEGRAM_CHAT_ID` through the existing OpenClaw bot
  (`TELEGRAM_BOT_TOKEN`), with the incident link and the status page.
- **Queue task:** a queue task needs a working broker, so it can't be
  filed while the broker is down. On recovery the Worker enqueues one
  `triage` task in `QUEUE` (default `repo-health-code-pilot`), anchored
  to the incident issue (dedupe key
  `repo-health:v1:triage:<repo>:issue:<N>`), asking for the cause and
  any follow-up fix. It never asks the task to close the incident.
- **Retries:** pushes and queue tasks wait in a small `pending` list in
  the Durable Object until they go through, so a failed delivery is
  retried on later ticks without being sent twice. A failed GitHub
  write leaves the state as it was, so the next tick retries it.

The decision logic (`src/logic.js`, `src/run.js`) is a port of
`scripts/fleet/queue-transport-alert/check_transport.py`, and the two
share one contract and one incident title, so they dedupe onto the same
issue when both run. State lives in one SQLite-backed Durable Object.

## Status endpoint

- `https://queue-transport-alert.developing-today.workers.dev/` is a
  small page (auto-refresh every minute).
- The page is installable (`/manifest.webmanifest`, `/sw.js`) and has
  **Turn on phone alerts** and **Send a test** buttons. Its POST
  routes are `/subscribe`, `/unsubscribe` and `/test-push` (that device
  only, at most every 30 seconds). `POST /admin/test-push` with
  `X-Admin-Token` set to the probe's Access client secret pushes a test
  to every device.
- `/status` (or `/status.json`) is the same view as JSON, CORS-open, for
  the PM stack and other tools: `status` (`up`, `degraded`, `down`,
  `stale` when no probe in 25 minutes, `unknown` before the first),
  `up`, `last_check`, `last_ok`, `failing_since`,
  `consecutive_failures`, `last_failure`, `incident`, `last_outage`
  (`start`, `end`, `duration`, `incident`), `pending_deliveries`,
  `last_errors`, `last_push` and `issue_title_prefix`. No secrets.

## Deploying

From a checkout of this repo, with Bitwarden and sops access:

```bash
nix run .#queue-transport-alert-deploy -- --dry-run                 # build only
nix run .#queue-transport-alert-deploy                              # upload + deploy HEAD
nix run .#queue-transport-alert-deploy -- --deploy-version <id>     # roll back
```

It uses the `cf` CLI's versions flow, the same standard as
`pkarr-deploy`, with `cloudflare.config.ts` as the deploy config
(`wrangler.jsonc` mirrors it for `wrangler dev`; `test/config.test.mjs`
keeps the two in step). The app stages the flake package
(`package.nix`), links `cf/config` and wrangler (cf's build bridge) from
the Nix store, then:

1. `cf workers versions create --tag git-<sha12>` uploads a version
   with its secrets read from stdin (`--secrets-file /dev/stdin`);
   nothing goes live yet.
2. `cf workers deployments create` puts that version at 100%.
3. `cf workers triggers deploy` applies the cron schedule.
4. It checks the latest deployment serves that version and that
   `/status.json` answers.

Every version carries its own secrets, so `--deploy-version <id>` rolls
back to an older version as it was, without uploading anything
(`cf workers versions list --worker-id queue-transport-alert` lists the
ids). The secrets come from Bitwarden by name (override with the env
vars); one left out of an upload keeps the previous version's value:

| Worker secret          | Bitwarden name (env override)                                                                           |
| ---------------------- | ------------------------------------------------------------------------------------------------------- |
| `ACCESS_CLIENT_ID`     | `cloudflare.access.queue-broker-client-id` (`QTA_ACCESS_ID_SECRET`)                                     |
| `ACCESS_CLIENT_SECRET` | `cloudflare.access.queue-broker-client-secret` (`QTA_ACCESS_SECRET_SECRET`)                             |
| `GITHUB_TOKEN`         | `github.developing-today-code-automation` (`QTA_GITHUB_TOKEN_SECRET`)                                   |
| `TELEGRAM_BOT_TOKEN`   | `openclaw.telegram-bot-token` (`QTA_TELEGRAM_SECRET`)                                                   |
| `BROKER_AGENT_TOKEN`   | the file `~/.config/queue-broker/token` (`QTA_BROKER_TOKEN_FILE`); carried over when the file is absent |

cf itself uses `cloudflare.api-token-day-to-day`
(`QTA_CF_TOKEN_SECRET`). It is also in the deploy registry as
`queue-transport-alert` (`deploy list`).

Without the Access secrets the Worker skips its probe; without
`GITHUB_TOKEN`, or with `APPLY` other than `true`, it only logs. Without
the Telegram secret it sends no push, and without `BROKER_AGENT_TOKEN`
it files no queue task.

## Tests

```bash
node --test test/*.test.mjs
```

They also run in the package's `checkPhase`, so a flake build fails on
a broken contract.
