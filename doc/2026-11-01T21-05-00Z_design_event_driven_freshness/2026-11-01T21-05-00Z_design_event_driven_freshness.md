# Design: event-driven freshness for the PM-stack aggregator

- Date: 2026-11-01
- Status: **design** — implementation is filed as children of #1184; nothing
  in this note changes runtime behavior.
- Issue: #1252 (epic #1184).
- Revalidated against `main` at `70a595a7735c208ccbb1ad0de115b3156cd572d4`.
  The queue packet's old sandbox-timer path is retired: the collector now runs
  on the laptop. The underlying issue remains: the feed is still produced on a
  five-minute timer, and the aggregator has no source-to-browser push path.

## Decision in brief

Treat upstream events as **invalidation signals**, not as replacements for
the authoritative source records. A verified source event wakes the
aggregator, which rereads the affected source and then sends a small,
replayable refresh notice to browsers. The notice contains no source payload or
secret. Pages and machine clients continue to read the existing canonical HTML
and JSON APIs.

Use one bounded, durable freshness journal/outbox in the aggregator's runtime
state. It is not the audit/activity river and does not become another source of
truth. Reuse the projector's GitHub receiver and, when implemented, the source
adapters tracked by #3321; do not add a competing webhook receiver or a second
activity log. Keep the current five-minute collector as anti-entropy and the
degraded-mode fallback. Use **SSE**, extending the existing Datastar rendering
path; do not add WebSockets for a one-way server-to-browser signal.

```text
GitHub App ──signed webhook──┐
Kaneo ───────signed webhook──┤
collector/orchestrator ─local┤──► verified adapter + durable invalidation
                             │       outbox/journal ──► aggregator reread
5-minute reconciliation ────┘                             │
                                    HTML: Datastar SSE ◄──┤
                                    JSON: /api/events.json ◄┘
```

## 1. Current evidence and boundary

At the cited `main` HEAD:

- `pm-stack/aggregator/src/feed.ts:1-8` says the fleet feed is produced by
  `scripts/collect-fleet-feed.ts` on the laptop and written into the
  aggregator's state directory; stale data keeps rendering with its age.
- `nixos/services/pm-stack/default.nix:216-249,343-350` runs that collector
  every five minutes. `pm-stack/aggregator/README.md:583-593` confirms the
  old sandbox producer is retired.
- `pm-stack/aggregator/src/health.ts:33-35,344-356` currently marks the feed
  stale after 15 minutes. Keep this honest stale threshold in v1; a received
  push is not proof that its data is fresh.
- `pm-stack/projector/README.md:23-30,46-56,80` documents a GitHub App
  webhook receiver with raw-body HMAC-SHA256 verification, delivery-GUID
  idempotency, and a separate 15-minute GitHub reconciliation timer
  (`nixos/services/pm-stack/default.nix:321-329`). This updates the projector,
  not the aggregator's page stream.
- `pm-stack/aggregator/src/server.ts:413-428` has Datastar SSE formatting for
  one-shot fragment responses. It is not a long-lived, replayable source event
  stream.
- Every current HTML page has a machine-first JSON twin
  (`pm-stack/aggregator/README.md:75-87`). Preserve that as the data contract.

This design is specifically **freshness notification and view refresh**. It is
not the cross-stack audit/activity river in #1261/#1739 or its source-coverage
children #3321–#3323. An invalidation says “reread this projection”; it is not
an actor/action history row, and it must not claim complete activity coverage.
Where #3321 supplies verified GitHub/Kaneo adapters, this design consumes
those adapters rather than duplicating them. #1258 remains the design for
fleet-wide durable storage; the short-lived refresh cursor here must be
recoverable by reconciliation and must not be represented as audit retention.

## 2. V1 sources and delivery paths

| Source                                                        | V1 decision                                                                                | Delivery path and trust boundary                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| ------------------------------------------------------------- | ------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| GitHub                                                        | Include issue, PR, and comment changes already covered by the GitHub App.                  | GitHub App → existing projector `POST /github/webhook` through the current tunnel/ingress → verify `X-Hub-Signature-256` over the bounded raw body → apply/dedupe the projector update → append a freshness invalidation only after durable acceptance. Use GitHub's delivery GUID as the source idempotency key. Reuse the existing receiver; do not expose a second copy.                                                                                                                            |
| Kaneo                                                         | Include task create/update/status/assignee/comment changes for the configured PM projects. | Kaneo generic project webhook → a source adapter on the same verified ingress boundary → verify `X-Kaneo-Signature` over the exact bounded raw body → durably accept and dedupe → invalidate the affected project/task view. Kaneo's documented generic webhook is outgoing, supports per-project event selection and optional HMAC, and documents no signed timestamp; do not trust payload actor fields as authenticated identity. Confirm the deployed fork/version and payload before enabling it. |
| Collector-local events                                        | Include feed publication and other local collector writes.                                 | After atomically publishing the feed, the laptop collector writes a small event to the local outbox/loopback or a same-user Unix socket. Include `generatedAt` plus a content hash/generation so repeated publication is idempotent. Do not rely only on filesystem mtime or a best-effort watcher.                                                                                                                                                                                                    |
| Orchestration publishes                                       | Include each successfully published orchestration snapshot.                                | The publisher emits after its atomic artifact write, using `runId` plus artifact hash/generation. The laptop collector continues importing the artifact as reconciliation until the publisher hook exists. Preserve the source-generated timestamp; do not invent a run when the file is absent or invalid.                                                                                                                                                                                            |
| Other pull-only sources (for example Grist and quota exports) | Do not pretend these are push sources in v1.                                               | Keep their existing source readers and schedules. A successful reconciliation that detects a changed source snapshot emits the same invalidation envelope. A source may move to direct push only when it has a supported authenticated delivery path.                                                                                                                                                                                                                                                  |

The official Kaneo references checked for this design are [Outgoing webhooks]
(https://kaneo.app/docs/core/integrations/outgoing-webhooks) and [Create
webhook integration](https://kaneo.app/docs/api-reference/generic-webhook/create-webhook-integration)
(accessed 2026-11-01). They describe the generic HTTP integration, raw-body
HMAC-SHA256 via `X-Kaneo-Signature`, event payload fields, and the lack of a
separate signed timestamp. Reconfirm this contract against the deployed Kaneo
version before configuring production deliveries.

### Invalidation envelope

Persist only the information needed to route and replay a refresh:

```text
schemaVersion: 1
sequence: monotonically increasing journal cursor
eventId: stable source id or source-specific idempotency key
source: github | kaneo | fleet-feed | orchestration | reconcile
scope: project/ref/page facet(s) to invalidate
occurredAt: source time when known, otherwise null
observedAt: RFC3339 time accepted by the aggregator
reason: source-event | reconcile | recovery
```

Do not persist webhook bodies, comments, credentials, or copied source records
in this journal. Event delivery is a hint; clients reread canonical endpoints.
The source's current record remains authoritative.

## 3. Browser transport and JSON twins

Use a dedicated long-lived `GET /freshness/stream` SSE endpoint. The browser uses
`EventSource`; the server can render Datastar `patch-elements` frames through
the existing helpers when an invalidation matches the open page. The existing
`/frag/*` one-shot SSE responses remain useful for initial fragment loads but
are not the stream or its replay store.

- SSE is server-to-browser, which is the only new direction required. Existing
  dashboard writes continue through their current authenticated POST routes;
  WebSocket upgrade, client message protocol, and bidirectional session state
  add no value here.
- Scope a connection to the current page/facets (for example board, health,
  or a project page). The server owns the scope mapping; clients cannot use an
  arbitrary scope to expand access. Coalesce repeated invalidations while a
  render is in flight, then read once and patch the current view.
- Give every delivered event its journal `sequence` as the SSE `id`. On
  reconnect, honor `Last-Event-ID` and replay later retained notices. Send a
  comment heartbeat about every 25 seconds to keep intermediaries from
  silently expiring an idle connection. Do not send a “fresh” state merely
  because the stream is connected.
- Add `GET /api/freshness-events.json?after=<sequence>&scope=<page>` as the machine-first
  twin. It returns the same envelope, ordered by `sequence`, plus the current
  cursor and per-source push/reconciliation health. A JSON client can poll this
  endpoint; after a notice it fetches the relevant existing `/api/*.json`
  endpoint. The HTML and JSON surfaces therefore agree on the invalidations
  without maintaining two event formats.
- Retain a bounded replay window (recommend 24 hours or 10,000 notices,
  whichever limit is reached first). If a requested cursor predates retained
  history, send a `reset` event / JSON `resetRequired: true`; the client refetches
  its complete current view and advances to the current cursor. These notices
  are not historical activity and do not require indefinite retention.

## 4. Ordering, deduplication, replay, reconciliation

Source timestamps do not define a trustworthy total order across GitHub,
Kaneo, the collector, and orchestration. Keep `occurredAt` and `observedAt`
separate; use the aggregator-assigned monotonic `sequence` only for journal
replay order. Invalidation is idempotent, so out-of-order delivery cannot
overwrite canonical state: the handler rereads the current source projection
instead of applying event deltas.

Deduplicate before advancing the source checkpoint:

- GitHub: `(installation/repository, X-GitHub-Delivery)`.
- Kaneo: use a provider delivery ID if the deployed version supplies one;
  otherwise use a stable hash of the verified raw body plus the source
  instance, and treat it as an invalidation only. A body hash can collapse
  identical retries but is not proof of event ordering or a substitute for a
  source cursor.
- Collector: `(producer, generatedAt, content hash)`.
- Orchestration: `(runId, artifact hash)`.
- Reconciliation: `(source, observed source checkpoint or snapshot hash)`.

Persist the accepted notice and its source cursor/outbox state before
acknowledging a webhook. Retries with the same key produce at most one
effective invalidation. If the event journal is corrupt or lost, mark its
cursor unavailable and force full-view refresh; never silently claim replay
completeness.

Run anti-entropy every **five minutes**, matching the existing collector
cadence; keep that collector as the degraded-mode polling fallback. Each
adapter compares its last successfully observed source cursor or canonical
snapshot fingerprint with the authoritative source. If it finds a newer
revision without a corresponding accepted invalidation, it records a
`reason: reconcile` notice and advances the checkpoint only after rereading
and persisting the current projection. Reconciliation is the missed-event
detector; a healthy SSE socket or source heartbeat alone does not prove that
no event was missed. Keep the current 15-minute feed stale threshold initially
and measure before tightening it. The projector's existing 15-minute GitHub
reconcile remains source-level recovery; the five-minute collector reconciliation
is the dashboard fallback, not a replacement for that source cursor.

When an SSE client is disconnected, the browser retries automatically and
replays from `Last-Event-ID`. If the stream remains unavailable, switch to a
five-minute JSON/API refresh (the same fallback cadence), show degraded push
state, and resume SSE without losing the current view. A source outage is
reported independently from a quiet source: lack of events is not evidence of
failure.

## 5. Failure posture and freshness UX

Expose stream and source health on `/health` and `/api/health`, and attach the
relevant source state to existing freshness chips:

- `push: up | degraded | down`, `last accepted event`, `last successful
reconciliation`, source age, and any open gap/checkpoint lag;
- distinguish `push disconnected` from `source stale`, `source unavailable`,
  `reconcile failed`, and `journal reset required`;
- show the existing 15-minute feed-stale threshold honestly. A fresh push
  cannot make old source data fresh, and a missed event remains a gap until
  reconciliation proves the current projection;
- if a producer is down, retain the last good source data with its measured
  age, visibly mark it stale/degraded, and continue the five-minute fallback
  where the source can still be polled. Do not replace stale state with an
  empty or fixture-backed “healthy” view.

## 6. Authentication and #1182 Access model

Browser HTML, JSON APIs, and `/events/stream` use the same-origin browser
session behind the Cloudflare Access policy in #1182. EventSource sends the
same-origin Access cookie; the stream must not accept a caller-supplied actor,
bearer token in a URL, or a broader scope than that session can read. No new
browser subscription secret is needed.

Machine webhooks cannot complete Drew's Google SSO. Therefore, when #1182 is
implemented, either (a) carve out only the exact `POST` webhook paths from the
Access login and require source HMAC on every request, with bounded bodies,
constant-time signature comparison, rate limits, and no read/API access; or
(b) route them to a dedicated source-only hostname on the existing tunnel with
the same HMAC protections. Do not bypass Access for the dashboard, JSON APIs,
or SSE. GitHub reuses its existing App secret and verifier; Kaneo uses its
per-project signing secret and raw-body verifier. Local publishers use a
same-user Unix socket or loopback-only interface, never the public route.

This consumes #1182's settled browser policy but does not authorize its
implementation. Do not publish a new external endpoint or configure live
webhook secrets until Drew records the go required by #1182.

## 7. Implementation issues (children of #1184)

1. **Source invalidation outbox and publisher hooks** — consume the verified
   GitHub/Kaneo adapters (coordinate with #3321), emit after durable source
   acceptance, and add post-commit notifications for the fleet-feed and
   orchestration publishers. No second webhook receiver, audit river, or
   event payload store. Dedupe/retry behavior is covered by source-specific
   tests.
2. **Replayable SSE stream and JSON event twin** — implement scoped
   `/freshness/stream`, Datastar refresh integration, `Last-Event-ID` replay,
   reset/full-refresh behavior, heartbeat, and `/api/freshness-events.json` with the
   same cursor/envelope. Browser and JSON consumers continue reading the
   canonical page APIs.
3. **Five-minute reconciliation, fallback, and freshness health** — compare
   source cursors/snapshot fingerprints, synthesize a missing invalidation,
   retain the five-minute collector fallback, and report push/source/gap
   status on health and freshness chips without smoothing stale data to green.

All three issues reference #1252 and #1184. Issue 1 can start with local
publishers while coordinating the GitHub/Kaneo adapter contract with #3321;
issue 2 follows the outbox envelope; issue 3 can be developed alongside 2
against the same health contract. These are freshness notices, not a second
implementation of #1739/#1261.

## 8. Open questions for Drew

1. **Public ingress under #1182:** when granting the required go for #1182,
   should the GitHub/Kaneo machine webhook paths be narrowly exempted from
   Access on the dashboard hostname, or should the existing tunnel expose a
   separate source-only hostname? Until answered, external webhook
   configuration stays off; local publisher and reconciliation work can
   proceed.
2. **Kaneo production coverage:** approve enabling the documented generic
   webhook for all configured PM projects and the task/comment event set, or
   name a smaller v1 project/event subset. Confirm the deployed Kaneo build's
   payload/retry behavior before enabling deliveries.
3. **Reconciliation budget:** approve five-minute anti-entropy for the
   existing pull sources, including the API-read cost, or keep a source's
   current slower reconciliation and explicitly accept its longer missed-event
   detection window. The design default is five minutes; measure API use
   before broad rollout.
4. **Cursor survival:** should the bounded invalidation journal remain in the
   laptop aggregator state directory with full resync after loss, or must it
   move to the durable storage selected by #1258 before any remote client
   relies on replay? Either way it is not an audit log and a lost cursor must
   trigger full refresh.

No other design choice blocks local implementation. These questions do not
authorize production ingress or secret configuration.

## Non-goals

- No WebSocket server, new browser write channel, or source-of-truth change.
- No audit/activity record model, retention policy, global timeline, or
  replacement for #1739/#1261.
- No implementation of #1182's public Access deployment, and no production
  webhook secret or tunnel changes in this design-only task.
- No claim that pull-only Grist, quota, or other sources become push-capable
  without a supported event producer.
