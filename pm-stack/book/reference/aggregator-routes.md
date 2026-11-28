# Aggregator HTTP routes and API

The census counts 56 primary pages and views: 29 aggregator routes in [The aggregator website, page by page](website-pages.md) and 27 console views in [The console, view by view](console-views.md). This chapter covers the routes the census enumerates separately from that count: the aggregator's JSON twins, fragment and streaming routes, POST and write endpoints, and static and PWA routes; the console server's non-view routes; the console edge Worker's routes; and the other first-party HTTP surfaces in `pm-stack/`. Entries are grouped by surface and follow one shape: what the route is, its inputs, its defaults where the survey gives them, its behaviour, and its failure shapes where the survey records them.

## JSON twins of the manifest pages

One twin per page serves the same data as JSON. The twins are named on their pages in [The aggregator website, page by page](website-pages.md); the irregular spellings are the contract:

- `/api/overview.json` for `/`; `/api/today.json` for `/today`; `/api/digest.json` and the list `/api/digests.json` for `/digest`; `/api/attention.json` for `/attention`; `/api/reports.json` for `/reports`; `/api/board.json` for `/board`; `/api/roadmap.json` for `/roadmap`; `/api/conflicts.json` for `/conflicts`; `/api/quota.json` for `/quota`; `/api/dns.json` for `/dns`; `/api/aa.json` for `/aa`; `/api/search.json` for `/search`; `/api/comments/queue.json` and the full list `/api/comments.json` for `/comments`; `/api/program.json` and `/api/program-project.json` for `/program` and `/program/&lt;id&gt;`; `/api/queue.json` and the orchestration twin `/api/orchestration.json` for `/queue`; `/api/alerts.json` for `/alerts`; `/api/notifications.json` for `/notifications`.
- `/api/registry.json` is the twin of `/sessions` — not `/api/sessions.json`.
- `/api/health` is the twin of `/health` — no `.json` suffix. One health assembly feeds page and twin; the twin additionally returns per-page render state (`route`, `lastRenderedAt`, `ok` per manifest page).
- `/api/events.json` is the twin of `/events`, paginated, with a `rel=next` link emitted by `server.ts`.
- `/api/prompt/threads` is the twin named in the manifest for `/chat`.
- `/api/projects.json` still serves although `/projects` itself is a redirect, not a page.
- `/decisions` and `/pages` declare no twin in `pages.ts`. For decisions the server implements its own API family regardless — D7; see the decisions group below.

## Decisions and needs-Drew endpoints

- `POST /api/decisions/&lt;setId&gt;/submit` — **What it is:** the answer endpoint for one decision set; the body carries `{submissions}`. **Behaviour:** resolutions record first-resolution-wins in the aggregator's decisions store; drafts for a resolved question are deleted. **Failure shapes:** none recorded in the survey.
- `POST /api/decisions/&lt;setId&gt;/defer` and `POST /api/decisions/&lt;setId&gt;/request-info` — **What it is:** the other two resolution kinds from the page client: defer (with a lean of `none`, `skeptical`, or `approve`) and request-info.
- `POST /api/decisions/&lt;setId&gt;/events` — **What it is:** draft and edit events from the page client. **Alias:** `/api/decision-sets/&lt;setId&gt;/events`.
- `POST /api/decision-sets` — **What it is:** the v2 set-submission endpoint used by `scripts/submit-decision-set.ts`, with a `--refresh` mode for standing ledger-mirror sets such as the standing open-questions set. The older producer path is `scripts/raise-decision-set.ts`, which posts a set through `POST /api/alerts` with the `decisionSet` extension, one push per set.
- `POST /api/needs-drew/&lt;ledgerId&gt;/answer` — **What it is:** the answer endpoint for a single raised needs-Drew question. **Inputs:** the body is `{token, optionId}`; the per-question token in the answer-page URL is the authority. **Behaviour:** the service worker posts to this endpoint directly on a notification tap, with no page open; the generated page client is the fallback. The resolution enters the same decisions-ledger machinery as a set answer. **Store (D9):** `recordAnswer` in `pm-stack/aggregator/src/needs-drew.ts` writes it to the needs-Drew question store, the file at `cfg.needsDrewFile`. That path is `PM_AGG_NEEDS_USER_FILE`, else the legacy `PM_AGG_NEEDS_DREW_FILE`, else `needs-user.json` (or the legacy `needs-drew.json` where only that exists) under `~/.local/state/pm-aggregator/`, resolved by `needs-user-state-path.ts`. Decision sets keep their own store, `decisions.json`. The first answer wins. The route also matches `/api/needs-user/&lt;ledgerId&gt;/answer`.
- `GET /api/needs-drew.json` — **What it is:** the needs-Drew census: open issues and PRs labelled `user/review` or `user/escalated` across the fleet repos, deduped, with `hands:v1` markers (`kind: physical`, `interactive-login`, `payment`, `signature`). **Behaviour:** the console splits this census into hands, reviews, and other/escalated. The aggregator census itself never writes labels — D4.

## Actions, comments, and reports writes

- `POST /api/actions` and `POST /api/actions/batch` — **What it is:** dashboard actions on tier items (`src/actions.ts`). **Inputs:** kinds are `triage` (acknowledge, snooze, escalate, de-escalate, answer, delegate, dismiss), `status-request`, `session` (poke, answer, stop), and `preference` (set-delegation-default). **Behaviour:** consumers poll `GET /api/actions.json?state=new`; outcomes return through the result endpoint below. Session actions are consumed by the fast session-action consumer for `opencode-laptop`.
- `POST /api/actions/result` — **What it is:** the outcome return path. **Inputs:** the body is `{id, outcome, byRun, note}`, with `outcome` one of `applied` or `rejected`.
- `GET /api/actions/outage-queue.json`, `POST /api/actions/outage-queue/confirm`, `POST /api/actions/outage-queue/reject` — **What it is:** the outage-queue variant of the action flow (`src/outage-queue.ts`).
- `POST /api/comments` — **What it is:** a dashboard-native comment (store `src/comments.ts`), the write behind `/discuss/&lt;subject&gt;`.
- `POST /api/comments/triage` — **What it is:** triage of a dashboard comment.
- `POST /api/comments/github-result` — **What it is:** the external poster reports the GitHub result back. **Behaviour:** the dashboard itself never writes GitHub; a fleet job applies and posts, and this endpoint records the outcome.
- `POST /api/reports/resolve` — **What it is:** resolve a prose conflict by choosing a stored version. `POST /api/reports/late-update` — the late-update write for a report.
- `POST /api/roadmap/annotations` and `POST /api/roadmap/annotations/alias` — **What it is:** roadmap annotations (`src/roadmap-annotations.ts`); annotations write back onto roadmap rows.

## Prompt and chat endpoints

- `POST /api/prompt` and `GET /api/prompt` — **What it is:** submit a prompt and list prompts (`src/prompts.ts`).
- `GET /api/prompt/threads`, `GET /api/prompt/thread/&lt;id&gt;`, `GET /api/prompt/&lt;id&gt;` — **What it is:** thread list, one thread, one prompt.
- `GET /api/prompt/&lt;id&gt;/attachments/…` — **What it is:** a prompt's attachments (`src/prompt-attachments.ts`). `PATCH /api/prompt/&lt;id&gt;` — update a prompt. `GET /api/prompt/unread-count` — the unread count behind the `/chat` navigation badge.
- `GET /ws/prompt/&lt;id&gt;` — **What it is:** the WebSocket upgrade for a prompt thread (`promptWsUpgrade` in `server.ts`; `src/prompt-ws.ts`).

## Alerts, push, and quota writes

- `POST /api/alerts` — **What it is:** the alert throat intake. **Inputs:** a bearer intake token, resolved by secret name (`pm-aggregator.alerts-intake-token`, via `src/secrets.ts`); extensions include `decisionSet` (decision-set producers) and `stackHealth` (health hooks). **Behaviour:** posts land in the alerts store and engine in `src/alerts.ts`.
- `POST /api/push/subscriptions` — **What it is:** the push subscription upsert path; the preferences page reads stored subscriptions through it. `POST /api/push/test` — a test send (object reference `test:push`). `POST /api/push/test-class` — a per-class test send (references `test:&lt;class&gt;`, landing URLs per class from `notifications.ts`). `GET /api/sw-probe` — the service-worker probe.
- `GET /api/alerts/preferences.json` — **What it is:** the twin of `/alerts/preferences`.
- `POST /quota/refresh` and `POST /api/quota/refresh` — **What it is:** the quota refresh writes behind `/quota`. `GET /api/quota/service.json` — the twin of `/quota/service/&lt;id&gt;`, read in a `?service=` style; UNVERIFIED: the exact query contract beyond the route existing.

## Fragments, streaming, identity, and miscellany

- `/frag/overview`, `/frag/board`, `/frag/today`, `/frag/queue` — **What it is:** Datastar fragments for in-page refresh; `/frag/attention/*` fragments exist under the attention branch.
- `/freshness/stream` — **What it is:** the server-sent-events freshness stream; its snapshot twin is `/api/freshness-events.json`.
- `/api/item/&lt;id&gt;.json` — **What it is:** the machine twin pattern for `/item/&lt;subject&gt;`. **Failure shapes:** an unresolved id is a 404 case (#4969).
- `/api/author-identities` and `/api/admin/identity-registry` — **What it is:** identity registry reads and administration (`server.ts`).
- `POST /api/stack-token/exchange` — **What it is:** the break-glass and stack token exchange (`src/stack-token.ts`, `src/break-glass.ts`; script `scripts/issue-break-glass-token.ts`). **Inputs:** the target must be `kaneo`, `pad`, or `grist`.
- `POST /webhooks/github` — **What it is:** the aggregator's GitHub metadata webhook.

## Static and PWA routes (aggregator)

- `/sw.js` — the aggregator PWA service worker, generated from `src/sw-source.ts`. `/manifest.webmanifest` — the PWA manifest.
- `/assets/app.css`, `/assets/datastar.js`, `/assets/aa-explorer-computations.js`, `/assets/push-client.js`, `/assets/prompt-badge.js`, `/assets/freshness-client.js` — the dashboard's static assets.

## Console server routes (non-view)

From `pm-stack/console/src/server.ts`, base path `/console`:

- `GET /console/health` — unauthenticated; returns `{ok:true}`. `GET /.well-known/assetlinks.json` — Android App Links, unauthenticated. `GET /manifest.webmanifest` — the console PWA manifest.
- `POST /console/device/link` and `GET /console/device/summary` — linked-phone authentication, checked by phone link, not by sign-in. `GET /console/edge/key` — the edge verification public key. `GET /console/&lt;apk&gt;` — the Android APK itself, signed-in owner only.
- Static files: `app.js`, `app.css`, `outbox.js`, `sw.js`, `icon.svg`.
- Reads: `GET /console/api/me`, `GET /console/api/overview`, `GET /console/api/agents`, `GET /console/api/messages`, `GET /console/api/services`, `GET /console/api/push`, `GET /console/api/machines`, `GET /console/api/signin`, `GET /console/api/devices`, `GET /console/api/doc/&lt;name&gt;` (the named docs the edge and console serve, `DOC_NAMES` in `console/src/docs.ts`, including the questions doc), and `GET /console/api/question` (a single-question read, `deps.work`).
- Writes: `POST /console/api/act` — the generic audited action endpoint; the action table is in [The question sets, enumerated](question-sets.md). Route-shaped writes: `POST /console/api/queue/&lt;task&gt;/&lt;action&gt;`, `POST /console/api/prs/&lt;n&gt;/merge`, `POST /console/api/priorities/&lt;issue&gt;/rank`, `POST /console/api/issues/&lt;n&gt;/comment`, `POST /console/api/services/&lt;unit&gt;/&lt;action&gt;`, and the logs read `GET /console/api/services/&lt;unit&gt;/logs?lines=`.
- Gate behaviour: non-`/console` GET and HEAD paths are proxied to the dashboard URL for a signed-in owner. Every action is audit-logged before it is taken; an action that cannot be recorded is not taken (the edge-held path in `server.ts`).

## Console edge Worker routes

`pm-stack/console/edge/src/worker.ts`, `entry.ts`, `edge.ts`, and `direct.ts` serve the same SPA and API at `pm.security.cab` when the laptop is away, holding actions until the host returns. UNVERIFIED: the exact edge-only route table, beyond `edge.ts` and `direct.ts` being the held-action and direct-to-host implementations.

## Other first-party HTTP surfaces

- **Projector** (port 7801): webhook intake `POST /github/webhook` (HMAC, delivery-GUID dedupe), link registry `GET`, `POST`, and `DELETE /links`, reads `GET /items`, `GET /comments`, `GET /subjects/&lt;subject&gt;`, and an HTML queue view (`src/queue-view.ts`, `src/queue-view-html.ts`). UNVERIFIED: the queue view's route path — the survey records file names only.
- **Dispatch** (port 4879, public as `dispatch.security.cab`): `GET /health`, `POST /v1/sessions`, and the enqueue endpoint. Dispatch is API-only by its README; no web UI exists in `dispatch/src/`.
- **OpenCode claimant page**: `pm-stack/opencode-claimant/index.html` is a single static page in the claimant directory. UNVERIFIED: what it displays — the survey records its existence only.
- **Decision-events Worker**: `POST /events` (alias `/webhook`), `GET /sets/&lt;setId&gt;/events`, `GET /sets/&lt;setId&gt;/state`, `GET /sets/&lt;setId&gt;/stream`, `GET /health` — shared-secret authentication, fail-closed with no secret. See [The workers](workers.md).

Sources: PM-SURVEY website census (`pm-stack/aggregator/src/server.ts`, `pm-stack/aggregator/src/pages.ts`, `pm-stack/console/src/server.ts`), survey files `website-pages.md`, `apps.md`, `push.md`, and `disagreements.md` (D4, D7, D9), census date 2026-11-23.
