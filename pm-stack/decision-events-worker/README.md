# decision-events Worker (#9335 Track B)

The Cloudflare half of the decision-set event pipeline. Decision-set
activity events — `{eventId, setId, questionRef, type, payload, ts}` —
land in three places, all idempotent on `eventId`:

1. **Turso** (fleet-platform DB, table `decision_set_events`) — written
   by the aggregator-side sync, following the fleet-platform pattern of
   code#5690.
2. **D1** (this Worker's `decision-events` database, same table shape —
   see `schema.sql`).
3. **A Durable Object per set** (class `DecisionSetEvents`, keyed by
   `idFromName(setId)`) — coordinates per-set fan-out: it counts the
   set's events, remembers the latest, and pushes new batches live to
   the set's WebSocket subscribers.

## Endpoints

| Route                                    | Auth          | Purpose                                                                                                                                                                                                          |
| ---------------------------------------- | ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `POST /events` (alias `/webhook`)        | shared secret | Accept an event batch: `{events: [...]}` (or a bare array / single event). Stores new events in D1, fans them out to each set's Durable Object, returns `{accepted, inserted, duplicates, errors, fanout}`.      |
| `GET /sets/<setId>/events?limit=&after=` | shared secret | Read a set's events back from D1, in `(ts, event_id)` order; the response's `nextAfter` is the composite cursor for the next page (see below).                                                                   |
| `GET /sets/<setId>/state`                | shared secret | The set's Durable Object state merged with D1's stored count.                                                                                                                                                    |
| `GET /sets/<setId>/stream`               | shared secret | WebSocket upgrade routed to the set's Durable Object; subscribers get `{type:"decision-events", setId, events}` per new batch, plus the current state on connect. Send `ping` → `pong`, `state` → current state. |
| `GET /health`                            | none          | Liveness + whether the webhook secret is configured.                                                                                                                                                             |

Read-back pagination: pass the previous page's `nextAfter` value as
`after` to get the next page. The cursor is composite — `<ts>|<eventId>`,
the (ts, event_id) pair of the page's last event — so paging returns
every event exactly once, including groups of events that share one
timestamp (#9614). `nextAfter` is null on an empty page. A bare ISO
timestamp is still accepted as `after` with its legacy meaning
(strictly after that ts, skipping any remaining same-ts events).

Auth is the shared secret as `Authorization: Bearer <secret>` or the
`X-Webhook-Secret` header, compared in constant time. With no secret
configured the Worker fails closed (503 on every data route).

## Producer side

`pm-stack/aggregator/scripts/sync-decision-events.ts` drains the
aggregator's decision-event outbox (JSONL) to both this Worker and
Turso. Credentials are resolved **by name** only:

- Turso token: `turso.fleet-platform-rw-token`
- Webhook secret: `cloudflare.decision-events.webhook-secret`

## GitHub Actions release (#9335 Track B)

The manual Wrangler deploy path has been retired. Use
`.github/workflows/decision-events-worker-release.yml` from `main`. It
builds and checks the Worker offline, uploads a version with
`cf workers versions create`, and optionally promotes the selected
version to 100% with `cf workers deployments create`. The `promote`
workflow input defaults to false, so a run can upload a version without
changing the active deployment.

The release uses `cf@1.0.0-rc.2` and Wrangler `4.163.0`, the compatible
Build Output toolchain pinned in this package. The programmatic
`cloudflare.config.ts` is the deployment declaration; `wrangler.toml`
continues to support local Wrangler development and tests. Both retain
Worker `decision-events`, compatibility date `2025-09-15`,
`workers_dev = true`, D1 binding `DB` to database ID
`68ade85e-6835-46a8-af6a-a9f2f2d966ae`, DO binding `SET_EVENTS` to class
`DecisionSetEvents`, and SQLite storage. `wrangler.toml` retains the
existing `v1` migration declaration; the release's offline parity check
asserts that it is unchanged. The Worker has no custom domain or custom
route configured.

### GitHub environment prerequisites

Configure the protected GitHub Environment `decision-events-production`
with:

- Variable `CLOUDFLARE_ACCOUNT_ID`, set to the account that already owns
  the `decision-events` Worker and the configured D1 database.
- Secret `CLOUDFLARE_API_TOKEN`, scoped to that account and permitted to
  read Worker/D1 identity, upload Worker versions, and create Worker
  deployments.
- Required reviewers before any version upload or promotion.

The Worker and D1 database must already exist in that account. This
workflow deliberately does not create either resource or alter D1 schema
or Durable Object migration state. If the Worker does not exist,
`cf workers versions create` cannot bootstrap it; stop and handle that
separately. `WEBHOOK_SECRET` must already be installed on the Worker
before producer cutover, and its value stays in the secret store.

The release changes the Worker version/deployment only. It does not
submit decision events, change producer settings, apply SQL, modify
custom routes/domains, or retire the Turso sink. Producer cutover still
requires a separate decision: verify `/health` reports the secret
configured, check the aggregator outbox/state with `--dry-run`, confirm
the exact `workers.dev` URL and credentials, then explicitly approve
the first real sync. With the webhook URL set, the aggregator defaults
to writing to both Turso and this Worker.

## Tests

```sh
bun test                 # contract tests for src/events.ts (no CF runtime needed)
npm run typecheck        # TypeScript types from generated config
npm run test:release     # offline target/version selection tests
./node_modules/.bin/cf workers versions create --worker decision-events \
  --tag offline-local --message "offline Build Output check" --dry-run
npm run check:build-output # checks D1, DO, migration, and route invariants
```
