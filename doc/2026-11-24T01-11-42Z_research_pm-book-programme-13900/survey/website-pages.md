# PM stack website — every page / route (PM-SURVEY, 2026-11-23)

Ground truth: laptop clone `~/git/code` at `4d937b364097e4e518efaa7669ee516ae7295b2a` (2026-11-23 17:58 -0600), read via `~/tools/laptop-ssh.sh`. Code wins over docs (see `disagreements.md`).

The website is **two first-party web apps plus proxied external apps**:

- **Aggregator dashboard** — Bun, `pm-stack/aggregator/src/server.ts` (routing), `pm-stack/aggregator/src/pages.ts` (page manifest), `pm-stack/aggregator/src/render.ts` (renderers). Bound `127.0.0.1:4870` + tailnet (`pm-stack/README.md`, `pm-stack/PORTS.md`). Public only behind the console gate at `pm.security.cab` (`pm-stack/console/src/server.ts`: non-`/console` GET/HEAD paths are proxied to `dashboardUrl` for a signed-in owner).
- **PM console** — Bun + static SPA, `pm-stack/console/src/server.ts`, `pm-stack/console/public/index.html`, `pm-stack/console/public/app.js` (`VIEWS` map at ~line 2955), `pm-stack/console/public/outbox.js`, `pm-stack/console/public/sw.js`. Base path `/console` (`BASE` in `pm-stack/console/src/server.ts`), port 4872 (`pm-stack/PORTS.md`). Also served at the edge by `pm-stack/console/edge/src/{worker,entry,edge,direct}.ts` and wrapped as an Android TWA (`pm-stack/console/android/`).
- External app UIs proxied/linked, not written here: Kaneo, Grist, Pad, ready-for-agent — see `apps.md`.

**Count used in this survey: 50 primary pages/routes** = 23 aggregator manifest pages + 6 aggregator sub-route/detail pages + 21 console hash views. API twins, fragments, POST/write endpoints and static/PWA routes are listed separately below and are not in that 50.

## A. Aggregator manifest pages — `pm-stack/aggregator/src/pages.ts` (`PAGES`, 23 entries)

Nav groups are `PAGE_GROUPS` in the same file: Overview / Work / Ops / System. `nav: primary|dropdown` is the #1497 curation recorded in that file.

1. `/` — Overview — "The stack front door (#1134): needs-you, projects, quotas, and health at a glance." API twin `/api/overview.json`. Group overview, primary.
2. `/today` — Today — "The day's top objectives + per-project momentum." Twin `/api/today.json`; also `/today?full=1` and fragment `/frag/today` (`pm-stack/aggregator/src/server.ts`). Primary.
3. `/digest` — Digest — scheduled PM digest (#1278): moved / blocked / needs Drew / done / quota outlook from the canonical projection; archive form `/digest?date=<date>`. Twin `/api/digest.json`, list `/api/digests.json`. Dropdown.
4. `/attention` — Needs you — "NEEDS USER vs NEEDS ORCHESTRATOR, and every live session." Twin `/api/attention.json`. Fragments under `/frag/attention/*` exist (`server.ts` comment at the `/attention` branch). Primary.
5. `/decisions` — Decision sets — index of every raised decision set (#8867): new today / this week / all, open-question counts; each row opens the set's own page (see A1). **No `api:` twin is declared in `pages.ts`** — see `disagreements.md` D7. Primary (promoted in #9354, comment in `pages.ts`).
6. `/reports` — Reports — "Review report versions and resolve prose conflicts without overwriting stored text." Twin `/api/reports.json`; writes `POST /api/reports/resolve`, `POST /api/reports/late-update` (`server.ts`). Dropdown.
7. `/board` — Board — "The curated cross-repo WIP board." Twin `/api/board.json`; fragment `/frag/board`. Dropdown.
8. `/roadmap` — Roadmap — "The program roadmap at every grain." Twin `/api/roadmap.json`; view/filter params `view`, `viewer`, `vf.*` are preserved by the route (`server.ts`). Primary. See `roadmap.md`.
9. `/conflicts` — Source conflicts — "One read-only queue of every live source disagreement, with both values and its age." Twin `/api/conflicts.json`. Dropdown.
10. `/quota` — Usage & billing — "Token quotas, PAYG dollars, and every SaaS service's usage vs its limits." Twin `/api/quota.json`. Primary. Detailed in §D below.
11. `/dns` — DNS register — "Every domain: next renewal, unused, earmarked. (#902's register)" Twin `/api/dns.json`. Dropdown.
12. `/aa` — Cost Explorer — two tabs: AA Explorer (Artificial Analysis measurements, frontier charts, plan/API/per-task efficiency) and Model Explorer (every model in the fleet's catalogs, pool seats, plan, measured value). Twin `/api/aa.json`; static computation asset `/assets/aa-explorer-computations.js`. Dropdown.
13. `/sessions` — Sessions — agent/session identity registry: who ran what, tokens, task links. Twin `/api/registry.json` (note: not `/api/sessions.json`). Dropdown.
14. `/search` — Search — board, roadmap, registry sessions, DNS, services, tier items by title or linked issue/PR ref. Twin `/api/search.json`. Dropdown.
15. `/events` — Events — cross-stack chronological stream of dashboard actions, orchestration publishes, session outcomes; HTML page links to its twin. Twin `/api/events.json` (paginated, `rel=next` link in `server.ts`). Dropdown.
16. `/comments` — Comments queue — dashboard comments still waiting on a loop (#1210): awaiting triage, GitHub comments pending/failed, kind/state counts, last error per failed item. Twin `/api/comments/queue.json` (full list twin `/api/comments.json`). Dropdown.
17. `/chat` — Chat — prompt chat/session surface (#10035): every prompt submission in its thread — reply, cross-references, archive — plus the unread count behind the nav badge. Twin named in manifest: `/api/prompt/threads`. Dropdown. See question sets §F4.
18. `/program` — Program status — projects hub: every project's status, roadmap, tracked work; one page per project under `/program/<id>` (see A2). Twin `/api/program.json`, per-project twin `/api/program-project.json`. Primary.
19. `/queue` — Queue — in progress, blocked, recently completed, next-up work across projects. Twin `/api/queue.json`; fragment `/frag/queue`; orchestration twin `/api/orchestration.json`. Primary.
20. `/health` — Health — service & page health (#1147): live probes for every stack service, feed section ages, last job runs, this dashboard's render history, services & ports topology (#1137). Twin `/api/health` (note: no `.json` suffix). Primary. Detailed in §C.
21. `/alerts` — Alerts — "The alert throat (#3676): sentinel incidents, Web Push delivery + one escalation, ack/close, the Turso ledger." Twin `/api/alerts.json`. Primary. See `push.md`.
22. `/notifications` — Notifications — notification measurement (#3319): weekly sent-vs-acted-on roll-up over the alert throat's send ledger, display-only default-off proposals under 10%. Twin `/api/notifications.json`. Dropdown.
23. `/pages` — All pages — the `pages.ts` manifest rendered as a page; every registered view in one place. No twin declared. Dropdown, group System.

## B. Aggregator sub-route / detail pages (in `SUBROUTE_PARENTS`, `pages.ts`, + `server.ts`)

24. `/item/<subject>` — item detail, parent `/`. Subject forms are handled in `server.ts` (`gh:`, `session:`, `orch:` object refs); machine twin pattern `/api/item/<id>.json` (an unresolved id is a 404 case, #4969 comment in `server.ts`).
25. `/discuss/<subject>` — discussion/comment surface for an item, parent `/`. Writes go through `POST /api/comments` (see §F5).
26. `/program/<id>` — one project page, parent `/program`; data twin `/api/program-project.json`.
27. `/quota/service/<id>` — one SaaS/quota service detail, parent `/quota`; twin `/api/quota/service.json` (`?service=` style read in `server.ts` — exact query contract UNVERIFIED beyond the route existing).
28. `/alerts/preferences` — notification preferences: per-class toggles + quiet hours + per-class test send. Twin `/api/alerts/preferences.json`. Parent `/alerts`. Reads stored subscriptions through the `/api/push/subscriptions` upsert path (comment in `server.ts`). This is the preferences surface for all classes in `push.md`.
29. `/decisions/<setId>` — one decision set's answer page, reached from `/decisions` and from the decision-set push (token in URL, `?t=<token>`; model in `pm-stack/aggregator/src/decisions.ts` header). Parent is the `/decisions` manifest page by navigation, although `SUBROUTE_PARENTS` does **not** list it — see `disagreements.md` D8.
30. `/projects` — **not a page**: redirects to the projects-pool section of `/program` (manifest comment in `pages.ts`; route branch in `server.ts` preserving `view`/`viewer`/`vf.*`). Its machine twin `/api/projects.json` still serves.

Other non-page GET routes in `server.ts`: `/frag/overview`, `/frag/board`, `/frag/today`, `/frag/queue` (Datastar fragments), `/freshness/stream` (SSE) + `/api/freshness-events.json`, `/sw.js`, `/manifest.webmanifest`, `/assets/app.css`, `/assets/datastar.js`, `/assets/aa-explorer-computations.js`, `/assets/push-client.js`, `/assets/prompt-badge.js`, `/assets/freshness-client.js`, `/api/sw-probe`, `POST /webhooks/github` (metadata webhook), `GET /ws/prompt/<id>` (WebSocket upgrade, `promptWsUpgrade` in `server.ts`).

## C. Health view — detail

- Page `/health`, twin `/api/health`: one health assembly feeds both ("The one health assembly behind both /health and /api/health", `server.ts`; `healthContext`).
- Sources named in code: `pm-stack/aggregator/src/health.ts`, `stack-health.ts`, `host-health.ts`, `build-health.ts`, `watchdog*.ts`, `slo*.ts`, and `pm-stack/host-health-collector.py` (laptop user-service snapshot the packaged aggregator cannot collect itself: claimant mode file, backup last-run, user-unit + timer states for pm-git-sync / pm-projector-reconcile / pm-kaneo-sync-guard / pm-dashboard-backup).
- `/api/health` additionally returns per-page render state: `body.pages = view.pages.map(p => ({route, lastRenderedAt, ok}))` (`server.ts`) — i.e. the manifest pages' render registry (#1147).
- Health-adjacent machine routes: `/api/author-identities`, `/api/admin/identity-registry` (identity registry administration, `server.ts`), `POST /api/stack-token/exchange` (break-glass/stack token, `src/stack-token.ts`, `src/break-glass.ts`; script `scripts/issue-break-glass-token.ts`).

## D. Quota / usage / billing views — detail

- `/quota` + `/api/quota.json`: token quotas per pool, PAYG dollars, every SaaS service vs its limits (`pages.ts` blurb; data in `src/quota-*.ts` — ~25 quota modules including `quota-current.ts`, `quota-forecast.ts`, `quota-anomaly.ts`, `quota-reweight.ts`, `quota-admission*.ts`, `quota-weekly-review.ts`), `POST /quota/refresh` and `POST /api/quota/refresh`, service detail §B27.
- `/aa` Cost Explorer (manifest #12 above): AA measurements + model catalog/pool seats — `src/aa.ts`, `src/aa-batches.ts`, `src/assets/aa-explorer-computations.js`.
- Billing/charges machinery behind `/quota`: `src/charges-{catalog,collect,match}.ts`, `src/spend-{attribution,cards,dollars,rollups}.ts`, `src/saas-extract.ts`, `src/sources/github-billing.ts`, scripts `collect-github-billing.ts`, `produce-charge-candidates.ts`, `confirm-charges-surfaced.ts`, `append/backfill/compact-usage-snapshots.ts`, `gather-quota.ts`, `forecast-snapshot.ts`; accounting records `aggregator/releases-accounting-2026-11-17.json`, `aggregator/COST-ANOMALY.md`, `src/cost-anomaly.ts`.
- Console-side equivalents: console `#more/platform`, `#more/releases`, `#more/machines`, `#more/services` (below) and console `GET /console/api/push`, `…/api/machines`, `…/api/signin` (`console/src/server.ts`).

## E. PM console views — `pm-stack/console/public/app.js` (`VIEWS`, 21 hash routes under `/console/`)

The SPA shell is `console/public/index.html` (title "PM", one `<main id="view">`, bottom tab bar, a global Ask/confirm `<dialog id="ask">`). Hash routing: `parse()` in `app.js`; unknown hashes fall back to `now`.

31. `#now` — Now (`viewNow`) — current board: in-progress + next from `Work.board()` (`console/src/work.ts`: queue counts, claimed, queued, blocked-reason groups).
32. `#next` — Next (`viewNext`, extra `ops`) — next-up work / ops preview.
33. `#needs` / `#needs/<sub>` — Needs me (`viewNeeds`, extra `questions`) — segmented sub-views `questions | hands | reviews | other` (see §F). Default segment is chosen by counts: questions, then hands, then reviews.
34. `#ask` / `#ask/say=<text>` — Ask (`viewAsk`) — owner's own-words instruction composer (textarea + Dictate via Web Speech where available) and the thread list of everything he has asked, with Fleet replies, Reply and Mark done. `say=` is prefill handed over by the Android app (share/selected text/dictation; comment in `viewAsk`).
35. `#more` — More (`viewMore`, extra `ops`) — index of the `more/*` views plus links to every aggregator dashboard page grouped by `PAGE_GROUPS` (rendered from the dashboard manifest data, `app.js` ~line 1185).
36. `#more/queue` — Queue (`viewQueue`) — queue tasks by status; actions `POST /console/api/queue/<task>/<action>` (`console/src/server.ts`).
37. `#more/prs` — Pull requests (`viewPrs`, `ops`) — open PR list/detail; merge via `POST /console/api/prs/<n>/merge` (`Ops.mergePr`).
38. `#more/issues` — Ranked issues (`viewIssues`, `ops`) — issues ordered by rank labels; re-rank via `POST /console/api/priorities/<issue>/rank`, comment via `POST /console/api/issues/<n>/comment` (`Ops.priorities/setRank/comment` in `console/src/ops.ts`).
39. `#more/agents` — Agents (`viewAgents`, `ops`) — agent/session list (`GET /console/api/agents`).
40. `#more/services` — Services (`viewServices`, `ops`) — systemd units the console may manage; per-unit action `POST /console/api/services/<unit>/<action>`, logs `GET /console/api/services/<unit>/logs?lines=` (`Ops.services/serviceAct/serviceLogs`, allowlist `manageable()`).
41. `#more/notify` — Notifications (`viewNotify`) — push status/subscription for this device (`GET /console/api/push`, `console/src/push.ts`, `console/src/notify.ts`).
42. `#more/phone` / `#more/phone/<sub>` — Phones (`viewPhone`) — linked phones/devices (`GET /console/api/devices`, `console/src/devices.ts`); redraws on `more/phone` route changes (device-link flow).
43. `#more/signin` — Sign-in (`viewSignin`) — sign-in state and Google sign-in / edge-direct controls (`GET /console/api/signin`, `Ops.signin/googleSignin/edgeDirect`).
44. `#more/away` — Cloudflare (`viewAway`) — the "away" (edge/Cloudflare) surface: what the edge holds when the laptop is unreachable (held actions, kept answers — `console/edge/src/edge.ts`, `console/src/sync.ts`).
45. `#more/machines` — Machines (`viewMachines`) — machine list (`GET /console/api/machines` in `server.ts`).
46. `#more/platform` — Platform (`viewPlatform`) — Cloudflare Workers platform screen, drawn from `pm-stack/platform/workers.json` (see `apps.md`); workflow/deploy state from `Ops.platform()`.
47. `#more/device` — On this device (`viewDevice`) — this device's identity/link state.
48. `#more/releases` — Releases (`viewReleases`) — release/deploy workflow runs (`Ops.releases()`, deploy via `Ops.deploy()`).
49. `#more/roadmap` — Roadmap (`viewRoadmap`) — console roadmap: capabilities/phases parsed from the plan doc (`console/src/plan.ts`: `CAPABILITIES`, `PHASES = doing|now|next|later|built`, `parseRoadmap`). See `roadmap.md`.
50. `#more/bugs` — Bugs (`viewBugs`) — bug-kind plan issues (`Plan` issues with `kind: "bug"` in `console/src/plan.ts`).
51. `#more/intake` — Intake (`viewIntake`) — intake routing list: kinds note/bug/change with their label/rank routes (`plan.ts` `ROUTES`, `INTAKE_LABEL = "owner/intake"`); submitting files an issue + a triage/prompt queue packet (see §F6).

Console non-view routes (`console/src/server.ts`): `GET /console/health` (unauthenticated `{ok:true}`), `GET /.well-known/assetlinks.json` (Android App Links, unauthenticated), `GET /manifest.webmanifest`, `POST /console/device/link` + `GET /console/device/summary` (linked-phone auth, checked by phone link not sign-in), `GET /console/edge/key` (edge verification public key), `GET /console/<apk>` (the Android APK itself, signed-in owner only), static `app.js/app.css/outbox.js/sw.js/icon.svg`, `GET /console/api/me`, `…/api/overview`, `…/api/agents`, `…/api/messages`, `…/api/services`, `…/api/doc/<name>` (`DOC_NAMES` in `console/src/docs.ts` — the named docs the edge/console serve, including the questions doc), `…/api/question` (single-question read, `deps.work`), and the generic action endpoint `POST /console/api/act` (see §F).
Console edge Worker routes: `pm-stack/console/edge/src/worker.ts` + `entry.ts` + `edge.ts` serve the same SPA/API at `pm.security.cab` when the laptop is away, holding actions until the host returns (`server.ts` "edge-held" audit path). Exact edge-only route table: UNVERIFIED beyond `edge.ts`/`direct.ts` being the direct-to-host and held-action implementations.

## F. Question sets — every question/answer-back surface

### F1. Decision sets (aggregator) — the main batched question set

- Model: `pm-stack/aggregator/src/decisions.ts`. A set mirrors a group of items from the Muse-side decisions ledger (`pending-questions.json` per that file's header); a set has items (`ledgerId`), an item has 1–16 questions, a question has `qid` (= `<ledgerId>#<1-based index>`), 0–12 options (`{id,label,tags?,combinationOf?}`), `mode: single|multi` (v1 defaults single), `recommendedOptionIds`, context/description/detail, group/order, and version history (`DecisionQuestionVersion`, #9335 v2). Free-text-only questions (0 options) are legitimate.
- Surfaces: index `/decisions`, per-set page `/decisions/<setId>?t=<token>`, console `#needs/questions` (same sets, read from the decisions file by `console/src/work.ts` `Work.questions()` — open questions only, newest set first, context clipped to 280 chars in list, full context via `Work.question()`).
- Answers: on the aggregator page, client JS in `server.ts` POSTs `/api/decisions/<setId>/submit` `{submissions}`, `/defer`, `/request-info`, and draft/edit events to `/api/decisions/<setId>/events` (also aliased `/api/decision-sets/<setId>/events`). Resolution kinds (`DecisionResolution` in `decisions.ts`): `answered` (optionId/optionIds + freeText + comment), `deferred` (with lean `none|skeptical|approve`), `request-info`. First resolution wins per question; drafts are deleted when a question resolves (`pruneResolvedDrafts`).
- Write-back: resolutions are recorded in the aggregator's decisions store (`defaultDecisionsFile`, `recordResolutions` in `decisions.ts`), exported as `DecisionResolutionRecord`s for the ledger/`listDecisionResolutions`/fleet-feed path, and — per the header of `decisions.ts` — "the dropped-thread catcher applies them to the ledger per question." Decision events also sync out via `scripts/sync-decision-events.ts` to Turso + the decision-events Worker (see `push.md`/`apps.md`). Producers: `scripts/raise-decision-set.ts` (via `POST /api/alerts` with the `decisionSet` extension, one push per set) and `scripts/submit-decision-set.ts` (via `POST /api/decision-sets`, v2, `--refresh` for standing ledger-mirror sets such as the standing open-questions set).
- Bulk answers: page submit-all; console "Accept <n> recommended" per set (`Work.acceptRecommended`, batched at 200, dashboard cap 256/request noted in code).

### F2. Needs-Drew single questions (aggregator, #8568)

- A raised needs-Drew question rides its notification candidate (`NotificationCandidate.needsDrew = {ledgerId, question, options}` in `src/notifications.ts`); the push's first two options become notification actions (`src/sw-source.ts`).
- Answer page + endpoint: `POST /api/needs-drew/<ledgerId>/answer` `{token, optionId}` — the per-question token in the answer-page URL is the authority (client code generated in `server.ts` ~line 884; service-worker tap path in `sw-source.ts`). Write-back is the same decisions-ledger resolution machinery as F1 (the ledger item's question resolves; see `decisions.ts` header: sets and the per-question store in `needs-drew.ts` are explicitly _not_ the same store — UNVERIFIED: the exact file the single-question answer writes, beyond `src/needs-drew.ts` being named "the question store" there).

### F3. Needs-Drew census sets (console `#needs`)

- Source: `GET /api/needs-drew.json` (aggregator census, `src/needs-drew.ts`): open issues/PRs labelled `user/review` or `user/escalated` across the fleet repos, deduped, with `hands:v1` markers (`kind: physical|interactive-login|payment|signature`).
- Console splits it (`Work.census()` in `console/src/work.ts`) into three sets: **hands** (a `hands:v1` action), **reviews** (`user/review`, no hands marker), **other/escalated** (no stated action — "the fleet's to decide"). Plus session-derived waiting items from `/api/attention.json` (`Work.sessions()`, tier `needs-drew`).
- Answers: **Reply** (`act("need.comment")` → `Work.needComment` → GitHub issue comment with `<!-- owner-reply:v1 -->`) and, for hands items, **I did it** (`act("hands.done")` → `Work.handsDone` → GitHub comment "Done by the owner…" + deletes the `user/escalated` label). See `disagreements.md` D4: the aggregator census itself never writes labels; the console does.

### F4. Prompt / chat questions (aggregator `/chat`, console `#ask`)

- Aggregator: `src/prompts.ts`, `prompt-ws.ts`, `prompt-notify.ts`, `prompt-widget.ts`, `prompt-attachments.ts`. Routes: `POST /api/prompt`, `GET /api/prompt`, `GET /api/prompt/threads`, `GET /api/prompt/thread/<id>`, `GET /api/prompt/<id>`, attachments `GET /api/prompt/<id>/attachments/…`, `PATCH /api/prompt/<id>`, `GET /api/prompt/unread-count`, WS `/ws/prompt/<id>`. A prompt submitted with notifications on gets a `prompt`-class push when it is answered or fails (`notifications.ts` class `prompt`, #10035).
- Console `#ask` is the owner-facing version of the same idea against GitHub+queue, not the aggregator store: `act("ask.send")` files an issue and enqueues a top-band (priority 10) triage packet (`promptPacket`/`notePacket` patterns in `console/src/work.ts`/`plan.ts`, marker `<!-- owner-prompt:v1 -->`); replies are read back from the issue (`ask.reply`, `ask.close` actions in `console/src/actions.ts`).

### F5. Dashboard comments / discuss (aggregator)

- `POST /api/comments` (dashboard-native comment; store `src/comments.ts`), `POST /api/comments/triage`, `POST /api/comments/github-result` (the external poster reports the GitHub result back), `GET /api/comments.json`, queue page `/comments`. The dashboard itself never writes GitHub — a fleet job applies/posts and reports back (`src/actions.ts` header). This is a question/answer-back surface in the loose sense: a comment on an item is answered by the applying run's outcome.

### F6. Dashboard actions (aggregator) — structured answers on tier items

- `POST /api/actions`, `POST /api/actions/batch`, consumers poll `GET /api/actions.json?state=new`, outcomes return via `POST /api/actions/result` `{id, outcome: applied|rejected, byRun, note}` (`src/actions.ts`). Kinds: `triage` (acknowledge / snooze / escalate / de-escalate / **answer** / delegate / dismiss), `status-request`, `session` (poke/**answer**/stop, consumed by the fast session-action consumer for `opencode-laptop`), `preference` (set-delegation-default). The triage `answer` verb is an answer-back: "the pass delivers payload text/option/answers like its own classifications" (`actions.ts` header).
- Outage queue variant: `GET /api/actions/outage-queue.json`, `POST /api/actions/outage-queue/confirm`, `…/reject` (`server.ts`, `src/outage-queue.ts`).

### F7. Console generic actions (`POST /console/api/act`)

- Action table in `console/src/actions.ts` (`ownActions` + `actions(...)` + `perform`): includes the question actions `questions.answer`, `questions.defer` ("Decide for me", lean default `approve`), `questions.accept`, `need.comment`, `hands.done`, `ask.send`, `ask.reply`, `ask.close`, plus queue/priority/issue/service/PR actions. Every action is audit-logged before it is taken ("one that cannot be recorded is not taken", `console/src/server.ts` edge-held path); when the laptop is unreachable the edge holds the action and replays it through the same table.

### F8. Reports, roadmap annotations, alerts — smaller answer-backs

- Reports: `POST /api/reports/resolve` (resolve a prose conflict choosing a stored version), `POST /api/reports/late-update` (`server.ts`).
- Roadmap: `POST /api/roadmap/annotations`, `POST /api/roadmap/annotations/alias` (`server.ts`, `src/roadmap-annotations.ts`) — annotations write back onto roadmap rows.
- Alerts: ack/close on `/alerts` (incident lifecycle `open|acked|closed` in `src/alerts.ts`); preferences toggles on `/alerts/preferences` write the subscription's class map that the engine reads on its next tick (`src/notifications.ts` header on `NOTIFICATION_CLASSES`).

## G. Other first-party web UIs in the pkgs / pm-stack

- **Projector queue view**: `pm-stack/projector/src/queue-view.ts` + `queue-view-html.ts` — an HTML queue view served by the projector (port 7801, `pm-stack/PORTS.md`). Route path: UNVERIFIED (file names only).
- **OpenCode claimant page**: `pm-stack/opencode-claimant/index.html` — a single static page in the claimant directory; what it displays: UNVERIFIED (not read in this survey beyond existence).
- **Dispatch**: `pm-stack/dispatch/` is API-only by its README (`GET /health`, `POST /v1/sessions`, enqueue endpoint) — no web UI found in `dispatch/src/`.
- **Kaneo / Grist / Pad / ready-for-agent UIs**: external apps, enumerated in `apps.md` (Kaneo at `https://kaneo.security.cab`, Grist `127.0.0.1:8484`, Pad `127.0.0.1:7777`, ready-for-agent UI+GraphQL `6056`; deep-link shapes in `aggregator/src/service-links.ts`).
