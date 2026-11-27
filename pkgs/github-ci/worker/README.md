# github-ci Worker

The GitHub webhook receiver (developing-today/code#5693). It writes
only to Turso; it is the sole receiver for every fleet repo.

## Shape

- `src/index.ts` — HMAC-SHA256 gate (Web Crypto), routing,
  workflow_run ingest. The gate is
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
npm test        # vitest + @cloudflare/vitest-plugin
npm run deploy  # wrangler deploy (fleet Cloudflare account)
```

The cutover from the earlier Postgres receiver is complete (#5693,
closeout below).

## Cutover closeout (2026-11-14, task 16806)

The flip is complete and the Edge Function is retired:

- **Dedupe diff clean.** Every `dedupe_key` in the Postgres
  `ci.events` ledger (11,203 keys) exists in Turso `ci_events`.
  The only gap was the Postgres ledger's final 4 rows
  (2026-11-06T17:47–18:00Z, received after the backfill watermark);
  those 4 rows were backfilled into Turso and the diff re-ran at
  zero missing. Turso held 27,648 rows at closeout, 16,445 of them
  written after the Postgres ledger stopped.
- **Postgres write-silence verified.** `ci.events` last write is
  2026-11-06T18:00:26Z — eight days of silence while the fleet
  produced ~16k Turso events. No sender still reaches the old
  receiver; every fleet repo's webhook points at this Worker
  (verified per repo: code, opencode, openchamber, darwin-nix).
- **Old receiver retired.** The previous receiver was deleted on
  2026-11-14, and its database project has since been deleted too;
  Turso is the only store. The Worker → Turso write path was
  verified live after the deletion (new `ci_events` rows within a
  minute). Rollback, if ever needed, is a redeploy of this Worker.

## Retention (90 days hot)

`ci_events` keeps 90 days hot in Turso, per the #5693 design.
The prune is `scripts/prune-ci-events.py` in this package:

```sh
python3 scripts/prune-ci-events.py            # delete past 90 days
python3 scripts/prune-ci-events.py --dry-run  # count only
```

It resolves the Turso token by name (`turso.fleet-platform-rw-token`)
and prints counts only. First run 2026-11-14 deleted 0 rows — the
ledger's oldest row is the 2026-10-30 backfill head, so the window
first binds in late January 2027; run the prune on fleet maintenance
cadence so the window is enforced from then on.

## GitHub mirror (#8580)

Fleet-repo `issues` / `pull_request` / `issue_comment` deliveries also
upsert the GitHub mirror in **fleet-queue** (`src/mirror.ts`; DDL in
`mirror.sql`): `github_mirror_issues` (title, body hash, state, labels,
assignees, comments count, latest agent-note pointer),
`github_mirror_pull_requests` (+ merged, draft, head/base), and
`github_mirror_meta` (per-repo staleness: `last_webhook_at` /
`last_webhook_event` stamped here; `last_backfill_at` stamped by the
backfill/repair sweep).

The mirror is a **read model**: rebuildable from GitHub at any time,
never a second truth. Staleness is reported, never gated on (#8575).
Mirror write failures are fatal (500 -> GitHub retries), matching the
ci_events policy.

### Backfill / repair sweep

`scripts/backfill.ts` reconciles the mirror from the GitHub API —
the repair path for anything the webhooks missed (a Worker outage,
a repo whose hook was added late, a deletion, which GitHub never
replays). It diffs every issue/PR in a repo against the stored rows
on `(gh_updated_at, body_hash)`, re-upserts changed rows through the
same statement builders the webhook paths use (all guards apply
identically), tombstones + deletes stored numbers the API no longer
returns, re-derives the latest-agent-note pointer for changed items
from their surviving comments, and stamps
`github_mirror_meta.last_backfill_at`. It never stamps the webhook
fields. Run it from this directory:

```sh
GH_TOKEN=$(gh auth token) \
TURSO_QUEUE_TOKEN=$(fleet secrets get turso.fleet-queue-rw-token) \
npm run backfill -- [owner/repo ...]   # default: every FLEET_REPOS repo
```

Output is one JSON summary line per repo (seen / upserted /
vanished_deleted / pointers_set / pointers_cleared / unchanged).
