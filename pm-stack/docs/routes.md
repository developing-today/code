# pm-stack aggregator routes

HTTP routes served by the aggregator (`pm-stack/aggregator/src/server.ts`).
Routing is a single handler: each path is a branch in sequence, and unknown
paths return the not-found response. The page list is in
`pm-stack/aggregator/src/pages.ts` (`PAGES`); derive any table from it rather
than maintaining one by hand.

Method rule: a route marked GET answers GET as its contract. Other methods
on GET-only routes are unsupported. Some routes are not method-guarded in the
code, so do not rely on them rejecting other methods.

## GET JSON (machine surface)

Each HTML page has a JSON twin, plus a few JSON-only endpoints:

- `/api/overview.json`, `/api/today.json`, `/api/digest.json`,
  `/api/digests.json`, `/api/attention.json`, `/api/needs-drew.json`
- `/api/queue.json`, `/api/orchestration.json`, `/api/board.json`,
  `/api/roadmap.json`, `/api/releases.json`
- `/api/quota.json`, `/api/quota/service.json`, `/api/quota-analytics.json`,
  `/api/flow-analytics.json`, `/api/forecasts.json`
- `/api/dns.json`, `/api/aa.json`, `/api/search.json`, `/api/registry.json`,
  `/api/conflicts.json`, `/api/projects.json`, `/api/program.json`,
  `/api/program-project.json`
- `/api/events.json`, `/api/activity.json`, `/api/views.json`,
  `/api/actions.json`, `/api/comments.json`, `/api/comments/queue.json`,
  `/api/coverage.json`, `/api/alerts.json`, `/api/alerts/preferences.json`,
  `/api/notifications.json`, `/api/freshness-events.json`
- `/api/health`, `/api/author-identities`
- `/api/item/<id>.json`

## GET HTML pages

`/`, `/board`, `/today`, `/digest`, `/queue`, `/roadmap`, `/quota`,
`/quota/service/<id>`, `/attention`, `/decisions`, `/decisions/<id>`,
`/answer/<id>`, `/dns`, `/aa`, `/sessions`, `/search`, `/events`,
`/projects` (redirects to `/program`), `/conflicts`, `/program`,
`/program/<id>`, `/health`, `/pages`, `/comments`, `/alerts`,
`/alerts/preferences`, `/notifications`, `/item/<id>`, `/discuss/<id>`.

Fragments for live updates: `/frag/overview`, `/frag/board`, `/frag/today`,
`/frag/queue`, `/frag/attention/header`, `/frag/attention/queues`,
`/frag/attention/sessions`.

Stream: `GET /freshness/stream` (server-sent events).

## Writes (POST unless noted)

| Route | Methods | Purpose |
|-------|---------|---------|
| `/api/roadmap/annotations`, `/api/roadmap/annotations/alias` | POST | Roadmap annotations. |
| `/api/comments`, `/api/comments/triage`, `/api/comments/github-result` | POST | Comment submission, triage and result. |
| `/api/actions`, `/api/actions/batch`, `/api/actions/result` | POST | Action submission and results. |
| `/quota/refresh`, `/api/quota/refresh` | POST | Refresh quota data. |
| `/api/alerts` | POST | Create an alert. |
| `/api/alerts/<id>/ack`, `/api/alerts/<id>/close` | POST | Acknowledge or close an alert. |
| `/api/decision-sets`, `/api/decision-sets/submit` | POST | Create and submit decision sets. |
| `/api/decision-sets/<id>/events` | GET, POST | Decision-set events. |
| `/api/decisions/<id>/submit`, `/defer`, `/request-info` | POST | Decision actions. |
| `/api/decisions/<id>/drafts` | GET, POST | Decision drafts. |
| `/api/decisions/<id>/events` | GET, POST | Decision events. |
| `/api/needs-drew/<id>/answer` | POST | Answer a needs-Drew item. |
| `/api/sw-probe` | POST records; GET summarises | Service-worker probe. |
| `/api/push/subscriptions` | POST (create), DELETE (remove) | Push subscriptions; other methods return 405. |
| `/api/push/test`, `/api/push/test-class` | POST | Push test sends. |
| `/api/admin/identity-registry` | GET, POST, DELETE | Identity registry administration. |

## Static and PWA assets

`/sw.js`, `/manifest.webmanifest`, `/icons/icon-192.png`,
`/icons/icon-512.png`, `/icons/icon-maskable-512.png`,
`/icons/apple-touch-icon.png`, `/assets/app.css`, `/assets/datastar.js`,
`/assets/aa-explorer-computations.js`, `/assets/push-client.js`,
`/assets/freshness-client.js`.

## Other listeners

`freshness-ipc.ts` exposes `POST /publish` on its own listener, separate
from the dashboard handler.
