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
| `GET /sets/<setId>/events?limit=&after=` | shared secret | Read a set's events back from D1, in `ts` order.                                                                                                                                                                 |
| `GET /sets/<setId>/state`                | shared secret | The set's Durable Object state merged with D1's stored count.                                                                                                                                                    |
| `GET /sets/<setId>/stream`               | shared secret | WebSocket upgrade routed to the set's Durable Object; subscribers get `{type:"decision-events", setId, events}` per new batch, plus the current state on connect. Send `ping` → `pong`, `state` → current state. |
| `GET /health`                            | none          | Liveness + whether the webhook secret is configured.                                                                                                                                                             |

Auth is the shared secret as `Authorization: Bearer <secret>` or the
`X-Webhook-Secret` header, compared in constant time. With no secret
configured the Worker fails closed (503 on every data route).

## Producer side

`pm-stack/aggregator/scripts/sync-decision-events.ts` drains the
aggregator's decision-event outbox (JSONL) to both this Worker and
Turso. Credentials are resolved **by name** only:

- Turso token: `turso.fleet-platform-rw-token`
- Webhook secret: `cloudflare.decision-events.webhook-secret`

## Deploy

```sh
# Token by name, piped — never on disk/argv (fleet wrangler pattern):
fleet secrets get cloudflare.api-token-day-to-day | \
  python3 ~/tools/cloudflare/wrangler-authed.py d1 create decision-events
# put the returned database_id in wrangler.toml, then:
fleet secrets get cloudflare.api-token-day-to-day | \
  python3 ~/tools/cloudflare/wrangler-authed.py d1 execute decision-events --remote --file schema.sql
fleet secrets get cloudflare.api-token-day-to-day | \
  python3 ~/tools/cloudflare/wrangler-authed.py deploy
fleet secrets get cloudflare.decision-events.webhook-secret | \
  python3 ~/tools/cloudflare/wrangler-secret.py  # or: wrangler secret put WEBHOOK_SECRET
```

## Tests

```sh
bun test          # contract tests for src/events.ts (no CF runtime needed)
bunx tsc --noEmit # typecheck (needs bun install for workers-types)
```
