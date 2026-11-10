# github-ci Worker

The GitHub webhook receiver (developing-today/code#5693), re-homed
off the Supabase Edge Function of the same name — the last Edge
Function on muse-dev2. Ported behavior-for-behavior from the Edge
Function (ACTIVE v8) so the two receivers can run side by side
during the dual-run diff period.

## Shape

- `src/index.ts` — HMAC-SHA256 gate (Web Crypto, same semantics as
  the Edge Function), routing, workflow_run ingest. The gate is
  unconditional (#6936): with `GITHUB_WEBHOOK_SECRET` unset or empty,
  every POST is rejected 503 "webhook secret not configured" before
  any write — the Worker fails closed, never open.
- `src/policy.ts` — the v4 ingest policy as pure functions:
  completed runs only, conclusion != skipped, raw payload kept
  only for failures, dedupe_key `github:<repo>:<run>:<attempt>:<action>`.
- `src/turso.ts` — minimal libSQL `/v2/pipeline` client with typed
  args (no string interpolation).
- `src/sink.ts` — the #5690 agent-events sink (POST the queue
  broker's `/events-append` when `EVENTS_BROKER_URL` is set; the
  broker route is public since #6925 — see below — and the Worker's
  own INSERT into fleet-queue `events` remains as the outage
  fallback when the var is unset), plus the fleet-queue stores that
  replace PG `agent.repo_snapshots` (`agent_repo_snapshots`) and
  `agent.watermarks` (`agent_watermarks`, `dirty:<repo>` keys).

## Broker route (#6925)

`EVENTS_BROKER_URL = https://queue-broker.security.cab`: a
Cloudflare Tunnel ingress row on the pm-stack tunnel
(`0ee3bf69-e5d1-4ed8-95de-4a17a18156b5`) forwarding to the
broker's tailnet listener (`100.99.113.103:4801`). A Cloudflare
Access application on the hostname admits only the
`github-ci-worker-queue-broker` service token; the Worker sends
its `CF-Access-Client-Id` / `CF-Access-Client-Secret` headers
plus the broker's own `X-Agent-Token` (the broker does not read
`Authorization: Bearer`). Auth chain, verified live 2026-11-07:
no Access headers → 403 at the edge; Access headers with a wrong
agent token → 401 from the broker; both correct → the broker
answers. Unsetting `EVENTS_BROKER_URL` reverts the sink to the
direct-Turso INSERT fallback with no code change.

## Writes (Turso only — no Postgres anywhere)

- `workflow_run` → `ci_events` in **fleet-platform**,
  `INSERT OR IGNORE` on `dedupe_key`, `id` unset (dedupe_key is
  the identity; readers use `received_at`).
- Fleet-repo `push` → head_sha advance in `agent_repo_snapshots`
  (fleet-queue) + a `push` sink event.
- Fleet-repo `pull_request` / `issues` → sink event +
  `dirty:<repo>` watermark in `agent_watermarks` (fleet-queue).

## Secrets (names only; set with `wrangler secret put`)

`GITHUB_WEBHOOK_SECRET` (new value minted for this Worker, vaulted
in BWS as `cloudflare.github-ci-webhook-secret`; the org/App flip
rotates it again per #5693), `TURSO_PLATFORM_TOKEN`
(`turso.fleet-platform-rw-token`), `TURSO_QUEUE_TOKEN`
(`turso.fleet-queue-rw-token`), `EVENTS_BROKER_TOKEN` (the queue
broker's own agent token — the value in
`~/.config/queue-broker/token` on the laptop), and
`EVENTS_BROKER_ACCESS_CLIENT_ID` / `EVENTS_BROKER_ACCESS_CLIENT_SECRET`
(the #6925 Access service token, vaulted in BWS as
`cloudflare.access.queue-broker-client-id` /
`cloudflare.access.queue-broker-client-secret`).

## Run

```sh
npm install
npm test        # vitest + @cloudflare/vitest-pool-workers
npm run deploy  # wrangler deploy (fleet Cloudflare account)
```

Cutover state lives on #5693: deploy dark → replay smoke → one
repo-level dual-run webhook → (later, separately) org/App flip.
The Edge Function stays deployed as rollback until then.
