# Project Management Space — Overview: the PM stack on one page

> Pack file 1 of 8 · Programme developing-today/code#13900 · Coordinator PM-BOOK
> Sources: `skeleton.md`, `survey/website-pages.md`, `survey/apps.md`,
> `survey/roadmap.md`, `survey/push.md`, `survey/models.md`,
> `coverage-ledger.md` (settled 2026-11-23: 22 workers, 50 Turso DBs).
> Ground truth for all surveys: laptop clone `~/git/code` at `4d937b3`
> (2026-11-23). Code wins over docs; disagreements cited as D1–D14.
> This pack gathers and organises only — the Space is built after the book lands.

## What it is

The Project Management stack (the PM stack) is the system Drew works in while
the fleet works underneath it. Its own description, in
`pm-stack/aggregator/src/program.ts`, is exact: the dashboard is the aggregator
of the PM stack — the one screen Drew lives in while the fleet works in
GitHub, Kaneo, and Pad underneath.

The stack joins work that lives in many places — GitHub issues and pull
requests, a Kaneo board, Grist roadmap tables, roadmap files in repositories,
the fleet work queue, agent sessions, and quota and health data — into one
state that can be read on a laptop and answered from a phone.

There is no single source of truth inside the stack, and the stack does not
pretend otherwise. Each source keeps its own record. The aggregator reads the
sources, the projector links items across them, and disagreements between
sources are shown on a dedicated page (`/conflicts`) instead of being hidden.

Work in this estate is spread across four repositories in the core set —
`developing-today/code`, `developing-today-forks/opencode`,
`developing-today-forks/openchamber`, and `dezren39/darwin-nix` — and across
the services that track it. Without the PM stack, answering three ordinary
questions means opening each source in turn: what needs Drew, what is moving,
and what is blocked. The stack exists to answer those questions in one place,
and to carry the answer back.

## The shape — six parts

| Part           | What it is                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | Where it lives                                                                                                                              |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------- |
| **Aggregator** | Bun dashboard service; the one state, many sources. Polls and reads GitHub (via the fleet feed / Project pool), Kaneo (read-only), Grist, repo roadmap files, the projector, the queue, quota and health sources; publishes 29 routes (23 manifest pages + 6 sub-route/detail pages) each with a machine JSON twin, plus Datastar fragments, SSE freshness, and the alert throat (`POST /api/alerts`). Port 4870, bound `127.0.0.1` + tailnet; public only behind the console gate at `pm.security.cab`.                                                                                                                                                                                                                                                                                         | `pm-stack/aggregator/` (`src/server.ts`, `src/pages.ts`, `src/render.ts`); unit `pm-aggregator`                                             |
| **Projector**  | Bun link-and-project service. Webhook intake (`POST /github/webhook`, HMAC, delivery-GUID dedupe), reconcile polls, link registry (`GET                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          | POST                                                                                                                                        | DELETE /links`), reads (`GET /items`, `GET /comments`, `GET /subjects/<subject>`), SQL views `pmp_v_links_items`/`pmp_v_subject_activity`, plus an HTML queue view. It links and projects; it never becomes a fifth source of truth and never writes back to GitHub / Kaneo / Pad / Grist. Port 7801. The aggregator consumes it via `aggregator/src/sources/projector.ts`. | `pm-stack/projector/`, `pkgs/pm-projector/` |
| **Dispatch**   | Bun intake service; authenticated launch-or-enqueue. `POST /v1/sessions` creates the tracking GitHub issue (label `source/dispatch`), enqueues a repo-task/v1 queue task, creates an OpenCode session on the laptop server (`127.0.0.1:13420`), and immediately claims the task for `dispatch-opencode` with a 6 h lease. A placement guard pins explicit OpenCode Go / ClinePass model hints to the active account's quota pool before launch and fails pre-commit on a walled pool. Port 4879, public as `dispatch.security.cab`. API-only — no web UI.                                                                                                                                                                                                                                        | `pm-stack/dispatch/`, `pkgs/pm-dispatch/`                                                                                                   |
| **Workers**    | Cloudflare Workers and doors around the core: the console edge Worker (`pm-console-edge`), the decision-events Worker, the voice-line Worker, and the notify relay (`pm-notify`) — plus the wider estate of 22 censused workers/doors (see `systems.md` and `coverage.md`). The registry of record is `pm-stack/platform/workers.json` (20 Workers listed, checked 2026-11-23 — see D6). The notify relay is recorded there as **not deployed** (D5).                                                                                                                                                                                                                                                                                                                                            | `pm-stack/platform/`, `pm-stack/console/edge/`, `pm-stack/decision-events-worker/`, `pm-stack/voice-line-worker/`, `pm-stack/notify-relay/` |
| **Website**    | Two first-party web apps. (1) The aggregator dashboard above — 29 routes in four nav groups (Overview / Work / Ops / System). (2) The PM console — Bun + static SPA at base path `/console`, port 4872, with 21 hash views, also served at the edge by the console edge Worker and wrapped as an Android TWA. External app UIs (Kaneo, Grist, Pad, ready-for-agent) are proxied/linked, not written here. Total primary pages/routes: **50** (29 aggregator + 21 console). Full census: `survey/website-pages.md`; Space build source: book reference chapters `website-pages.md` + `console-views.md`.                                                                                                                                                                                          | `pm-stack/aggregator/src/pages.ts`; `pm-stack/console/` (`src/server.ts`, `public/index.html`, `public/app.js` `VIEWS` map)                 |
| **Phone**      | The answering surface, not only an alert surface. Three phone-side pieces: the console Android app (TWA/APK served by the console itself; assetlinks at `/.well-known/assetlinks.json`), the aggregator PWA service worker (`aggregator/src/sw-source.ts`, served as `/sw.js`), and the console service worker (`console/public/sw.js` — push + notificationclick + offline kept answers). Web Push is VAPID, sent directly from the laptop aggregator. A single needs-Drew question can be answered by tapping a notification action with no page open; a decision set deliberately opens its page — the page is the answer surface. The edge relay that would answer while the laptop is dead is built but **not deployed**, its answer routes **not routed** (D5). Full flow: `push-flow.md`. | `pm-stack/console/android/`, `pm-stack/console/public/sw.js`, `aggregator/src/sw-source.ts`, `aggregator/src/push.ts`                       |

Deployment frame (`pm-stack/README.md`, `pm-stack/PORTS.md`): one
`nixos-rebuild switch` on laptop host `amd` deploys the stack; user units
`pm-aggregator`, `pm-projector`, `pm-dispatch`, `pm-tunnel` are declared in
`nixos/services/pm-stack`; Kaneo / Postgres / Pad / Grist are containers.
Port registry: 4870 aggregator, 4871 freshness IPC, 4872 console, 4879
dispatch, 5173 Kaneo, 6056 ready-for-agent, 7777 Pad, 7801 projector,
8484 Grist.

## What it does — four jobs

1. **One state to read.** Overview, needs-you, board, roadmap, program,
   queue, quota and health views over the same projections, each page with a
   machine twin of the same data.
2. **Questions that wait in one place.** When work needs a decision, the
   question is raised as a decision set, a needs-Drew item, a prompt, or a
   console ask — eight question sets in total (F1–F8, enumerated in
   `survey/website-pages.md` §F and the book's `question-sets.md` reference).
   Each set has a defined answer route and a defined write-back.
3. **Answers that reach the work.** A decision resolution is recorded and
   applied to the decisions ledger; a dashboard action is consumed by the
   orchestration pass, which is the single writer of GitHub Project fields;
   a console action is audit-logged before it is taken and then performed.
   The flows differ by surface and are kept separate — see
   `roadmap-mechanics.md` §Answer-back.
4. **Push that reaches a phone.** Waiting questions and incidents are pushed
   to subscribed devices in eight preference classes (D14 — see
   `push-flow.md`). Today answers go to the laptop aggregator.

## Who it serves

- **Drew, the owner.** The primary reader and the only person the needs-you
  surfaces are built for. He reads the overview and the console's Now view,
  answers questions in the Needs views, ranks issues, and follows items from
  issue to done.
- **The orchestrator.** The supervising agent work that keeps the fleet
  moving. It is the single writer of GitHub Project fields, consumes
  dashboard actions, and applies decision resolutions to the ledger. The
  aggregator service itself never writes GitHub labels; the console, as part
  of the same stack, does write labels through its own audited actions (D4).
- **Agents and sessions.** OpenCode and Muse sessions whose work is tracked
  as queue tasks, sessions, and items. Dispatch launches or enqueues their
  sessions; the claimant keeps claimed work moving; the registry records who
  ran what.
- **Operators of the stack itself.** Anyone keeping the laptop services,
  containers, edge Workers, timers, and guards running. The health page, the
  heal stack, and the operations chapters serve this reader.

## Scale markers (all counted, 2026-11-23)

| Marker                                      |                                  Count | Source                                               |
| ------------------------------------------- | -------------------------------------: | ---------------------------------------------------- |
| Primary pages/routes                        |        50 (29 aggregator + 21 console) | `survey/website-pages.md`                            |
| Components under `pm-stack/` + Nix packages |                                     36 | `survey/apps.md` §1–§36                              |
| Cloudflare workers/doors censused           |           22 (3 covered, 19 book gaps) | `coverage-ledger.md` Table A                         |
| Workers in `platform/workers.json` registry | 20 (README's "eighteen" is stale — D6) | `survey/apps.md` §13, D6                             |
| Turso databases in the estate               |          50 (34 covered, 16 book gaps) | `coverage-ledger.md` Table B; `turso-estate-list.md` |
| Question sets                               |                              8 (F1–F8) | `survey/website-pages.md` §F                         |
| Notification preference classes             |                                8 (D14) | `survey/push.md` §2                                  |
| Book chapters planned                       |       ~48 + reference census additions | `skeleton.md`                                        |

## Model surface — one line

The aggregator, console, dispatch, and projector make **no LLM calls and no
embedding calls of their own**. The real model surface is the voice-line
Worker's Workers AI inference (Whisper STT + MeloTTS) and the Atlas embedding
dependency (`Snowflake/snowflake-arctic-embed-m-v2.0`, 768 dims), which
belongs to Atlas, not to the PM stack. Detail: `survey/models.md`; plans and
evidence: `llm-plans.md`.

## Reading order for the Space build

`overview.md` (this file) → `systems.md` (every component and worker) →
`roadmap-mechanics.md` (hierarchy, roll-up, prioritisation, answer-back) →
`push-flow.md` (push, step by step) → `oc-programme.md` (forks programme) →
`llm-plans.md` (plans, tiers, dated evidence) → `priorities-and-maps.md`
(prioritised lists + dependency edges as data) → `coverage.md`
(completion test: 22 workers + 50 DBs).
