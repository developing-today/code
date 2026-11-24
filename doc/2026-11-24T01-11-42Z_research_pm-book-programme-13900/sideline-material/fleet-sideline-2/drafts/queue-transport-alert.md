# The queue watchdog: queue-transport-alert

<!-- id: queue-transport-alert since: pre-release -->

`queue-transport-alert` is a Cloudflare Worker
(`pkgs/queue-transport-alert/worker/`, plain JavaScript, #6369) that
watches the repo-queue broker **from off the broker's own host**. It
exists because of a blind spot: when the queue broker's transport is
down, every worker lane exits silently at its Step 0 health gate, so
a total queue outage produced no signal. Issue #5708 (2026-11-04) was
filed by hand; the 2026-11-18 outage — tailscaled stopped during a
rebuild — was noticed only from lane reports. The Worker closes that
hole: every 10 minutes it probes the broker's public route through
Cloudflare Access, and on a confirmed outage it files or comments a
GitHub incident, pushes to the phone, and — once the broker is back —
enqueues one triage task into the queue it watches.

**Status (2026-11-23): live and in service.** The platform registry
(`pm-stack/platform/workers.json`) records `state: "in service"` with
the address `https://queue-transport-alert.developing-today.workers.dev`,
and the Worker has filed a real incident (#12414, 2026-11-22, closed).

## The probe

The probe is `GET {BROKER_URL}/healthz`, where `BROKER_URL` is
`https://queue-broker.security.cab`, sent with the Cloudflare Access
service-token headers `CF-Access-Client-Id` and
`CF-Access-Client-Secret`, `redirect: "manual"` and a 10-second
timeout (`PROBE_TIMEOUT_MS=10000`). That route is the tunnel to the
broker's Tailscale listener, so the probe fails when the laptop is
off, when tailscaled is stopped, or when the broker itself is down.
A probe is healthy only on HTTP 200 with a JSON body of
`{"ok": true}` (`classifyHealthz`).

The failure class is the diagnosis, and the probe carries it in its
detail string:

| Probe result | What it means |
| --- | --- |
| 301/302/303/307/401/403 | "Cloudflare Access refused the probe's service token, or it is unset" |
| 502/503/504/530 | "The tunnel could not reach the broker" |
| 200 with a non-`ok:true` or non-JSON body | The body itself, clipped to 200 characters |
| Timeout | The URL and the timeout, named |

When the Access secrets are missing, the probe returns `skipped` and
the pass only logs. A watchdog that cannot authenticate its own probe
does not alert — that posture is deliberate.

## The state machine

All state lives in one Durable Object: the binding `TRANSPORT_STATE`
(class `TransportState`, SQLite storage, migration tag `v1` with
`new_sqlite_classes`). A single instance (`idFromName("broker")`)
holds everything, so passes never race. Its keys are `state`,
`push_subs`, `last_push` and `vapid` — the VAPID P-256 key pair is
generated once inside the Durable Object and never leaves it.

The machine (`src/logic.js` and `src/run.js`, a direct port of the
local script below) is small:

- `THRESHOLD` (2) consecutive failed probes open an alert window.
  Below the threshold, a failing window stays quiet.
- One alert per outage window: the Worker finds an open issue titled
  with the prefix `Queue transport incident:` and comments on it, or
  files one titled `Queue transport incident: repo-queue broker
  unreachable since <window_start>` with labels `kind/bug`,
  `priority/high`, `area/infra` and `source/fleet-monitor`.
- The first healthy probe after an alerted window posts exactly one
  recovery comment carrying the outage span.
- The Worker **never closes** the incident. There is deliberately no
  `close()` in `src/github.js` — rule #10136.

## The three side-effect channels

Alert and recovery each fan out to the same channels, under one retry
discipline: notifications and the recovery enqueue ride a `pending`
list in Durable Object state (capped at 20) and retry on later ticks.
A failed GitHub write leaves the window un-alerted, so the next tick
retries it rather than losing the incident.

1. **GitHub.** Find-or-file by the title prefix, with the labels
   above. This is the durable record of the outage.
2. **Web Push.** Payload-free: the push carries no incident text; the
   service worker reads the current state from `/status.json`'s
   `last_push` field. Push endpoints the push service reports gone
   (404/410) are dropped, and at most 20 devices may subscribe.
3. **Telegram.** One message per alert and per recovery, through the
   existing OpenClaw bot; the chat id is the `TELEGRAM_CHAT_ID`
   variable in the Worker config.

The master switch is `APPLY`: the deployed value is `true`, which
files, comments, pushes and enqueues. Any other value — or a missing
`GITHUB_TOKEN`, `TELEGRAM_BOT_TOKEN` or `BROKER_AGENT_TOKEN` —
degrades that one side effect to log-only, independently of the
others.

## The recovery triage task

On recovery the Worker also enqueues one triage task into the queue
it watches (`src/queue.js`): a `POST {BROKER_URL}/enqueue` with the
broker's `X-Agent-Token` header plus the Access headers, carrying a
repo-task/v1 `triage` packet anchored to the incident issue, on queue
`QUEUE` (`repo-health-code-pilot`) at priority 20, with dedupe key
`repo-health:v1:triage:<repo>:issue:<N>`. A dedupe error from the
broker counts as success — the task already exists, which is the
point. The enqueue is only attempted while the broker answers, which
is precisely why it is filed on recovery and not during the outage:
the queue that would carry the triage is the thing that was down.

## The status surface

| Route | What it serves |
| --- | --- |
| `GET /` | The status page: PWA-installable, auto-refreshing every 60 seconds |
| `GET /status`, `GET /status.json` | The same view as JSON, CORS-open |
| `GET /sw.js`, `/manifest.webmanifest`, `/icon.svg`, `/vapid.json` | The PWA and push plumbing |
| `POST /subscribe`, `POST /unsubscribe` | Device push registration |
| `POST /test-push` | A push to that device only, at most once per 30 seconds |
| `POST /admin/test-push` | A push to all devices; `X-Admin-Token` must equal the Access client secret, compared in constant time |

The status values are `up`, `degraded`, `down`, `stale` (no probe in
25 minutes) and `unknown`, with `last_check`, `last_ok`,
`failing_since`, `consecutive_failures`, `last_failure`, `incident`,
`last_outage` (start, end, duration, incident), `pending_deliveries`,
`last_errors`, `last_push` and `issue_title_prefix`. The status
surface carries no secrets.

HTTP edges: a subscribe body over 2048 bytes is a 413; an invalid
push endpoint a 400; the 21st device a 409; a test-push from an
unsubscribed device a 404 and a repeat within 30 seconds a 429; a bad
or missing admin token a 403; other POSTs and methods a 405; an
unknown GET a 404.

## Two implementations, one contract

The Worker is a port of the local script
`scripts/fleet/queue-transport-alert/check_transport.py`, which still
exists with its systemd units. The two share one contract and one
incident title, and dedupe onto the same GitHub issue when both run —
the find-by-title-prefix step is what makes the overlap safe.

## Configuration, deploy and rollback

The deploy source of truth is `cloudflare.config.ts` (the cf versions
flow); `wrangler.jsonc` mirrors it for `wrangler dev`, and
`test/config.test.mjs` keeps the two in step. Name
`queue-transport-alert`, main `src/index.js`,
`compatibility_date = "2026-10-25"`, observability on, cron
`*/10 * * * *`. The plain variables are `BROKER_URL`, `REPO`
(`developing-today/code`), `THRESHOLD`, `PROBE_TIMEOUT_MS`, `QUEUE`,
`TELEGRAM_CHAT_ID`, `PUBLIC_URL` and `APPLY`. The secrets are named
here and never written in any config file; `deploy.mjs` uploads them
per version from Bitwarden: `ACCESS_CLIENT_ID` and
`ACCESS_CLIENT_SECRET` (the Access service token for the broker
route), `GITHUB_TOKEN`, `TELEGRAM_BOT_TOKEN` and `BROKER_AGENT_TOKEN`
(the broker's `X-Agent-Token`).

The primary operator path is `nix run .#queue-transport-alert-deploy`:
upload a version tagged `git-<sha12>` with its secrets piped in,
deploy it at 100%, apply the triggers, and verify the serving version
and `/status.json`. Passing `--deploy-version <id>` rolls back to an
older version as it was. An optional, manual-only GitHub Actions
release also exists (`.github/workflows/queue-transport-alert-deploy.yml`):
main-only, behind the `cloudflare-queue-transport-alert` environment
with an independent required reviewer, no self-review and an
admin-bypass attestation checked by a verify-environment job, with
upload and promote as separate dispatches and no automatic rollback.
Tests are `node --test test/*.test.mjs`, also run in the flake
package's `checkPhase`. The Worker is registered in the deploy
registry as `queue-transport-alert`.

One record lags the code: the platform registry still describes this
Worker's deployment as "by hand" with Actions deployment as future
work. The workflow exists and #12550 (deploy through trusted GitHub
Actions) is closed; this chapter follows the repo (disagreement D32).

## Sources

Package `pkgs/queue-transport-alert/worker/` (`cloudflare.config.ts`,
`wrangler.jsonc`, `src/index.js`, `src/logic.js`, `src/run.js`,
`src/github.js`, `src/queue.js`); local peer
`scripts/fleet/queue-transport-alert/check_transport.py`; issues #6369,
#10136, #12006–#12026, #12414, #12550; platform registry
`pm-stack/platform/workers.json`. Survey: programme #13900, lane
FLEET-GAPS-SURVEY, 2026-11-23 (disagreement D32).
