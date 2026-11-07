# github-ci Worker

The GitHub webhook receiver (developing-today/code#5693), re-homed
off the Supabase Edge Function of the same name — the last Edge
Function on muse-dev2. Ported behavior-for-behavior from the Edge
Function (ACTIVE v8) so the two receivers can run side by side
during the dual-run diff period.

## Shape

- `src/index.ts` — HMAC-SHA256 gate (Web Crypto, same semantics as
  the Edge Function), routing, workflow_run ingest.
- `src/policy.ts` — the v4 ingest policy as pure functions:
  completed runs only, conclusion != skipped, raw payload kept
  only for failures, dedupe_key `github:<repo>:<run>:<attempt>:<action>`.
- `src/turso.ts` — minimal libSQL `/v2/pipeline` client with typed
  args (no string interpolation).
- `src/sink.ts` — the #5690 agent-events sink (POST the queue
  broker's `/events-append` when `EVENTS_BROKER_URL` is set; until
  a public broker route exists — the broker is tailnet-only — the
  Worker performs the sink's own INSERT into fleet-queue `events`),
  plus the fleet-queue stores that replace PG `agent.repo_snapshots`
  (`agent_repo_snapshots`) and `agent.watermarks` (`agent_watermarks`,
  `dirty:<repo>` keys).

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
(`turso.fleet-queue-rw-token`).

## Run

```sh
npm install
npm test        # vitest + @cloudflare/vitest-pool-workers
npm run deploy  # wrangler deploy (fleet Cloudflare account)
```

Cutover state lives on #5693: deploy dark → replay smoke → one
repo-level dual-run webhook → (later, separately) org/App flip.
The Edge Function stays deployed as rollback until then.
