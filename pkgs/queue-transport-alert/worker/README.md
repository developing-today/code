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
- A failed GitHub write leaves the state as it was, so the next tick
  retries it.

The decision logic (`src/logic.js`, `src/run.js`) is a port of
`scripts/fleet/queue-transport-alert/check_transport.py`, and the two
share one contract and one incident title, so they dedupe onto the same
issue when both run. State lives in one SQLite-backed Durable Object.
`GET /status` on the Worker returns that state (no secrets).

## Deploying

From a checkout of this repo, with Bitwarden and sops access:

```bash
nix run .#queue-transport-alert-deploy -- --dry-run   # bundle only
nix run .#queue-transport-alert-deploy                # deploy + set secrets
```

The app stages the flake package (`package.nix`), runs
`wrangler deploy`, then `wrangler secret bulk` from stdin with these
values read from Bitwarden by name (override with the env vars):

| Worker secret | Bitwarden name (env override) |
| --- | --- |
| `ACCESS_CLIENT_ID` | `cloudflare.access.queue-broker-client-id` (`QTA_ACCESS_ID_SECRET`) |
| `ACCESS_CLIENT_SECRET` | `cloudflare.access.queue-broker-client-secret` (`QTA_ACCESS_SECRET_SECRET`) |
| `GITHUB_TOKEN` | `github.developing-today-code-automation` (`QTA_GITHUB_TOKEN_SECRET`) |

Wrangler itself uses `cloudflare.api-token-day-to-day`
(`QTA_CF_TOKEN_SECRET`). It is also in the deploy registry as
`queue-transport-alert` (`deploy list`).

Without the Access secrets the Worker skips its probe; without
`GITHUB_TOKEN`, or with `APPLY` other than `true`, it only logs.

## Tests

```bash
node --test test/*.test.mjs
```

They also run in the package's `checkPhase`, so a flake build fails on
a broken contract.
