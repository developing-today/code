# The console, view by view

The PM console is a Bun server (`pm-stack/console/src/server.ts`, base path `/console`, port 4872) serving a static single-page application: shell `pm-stack/console/public/index.html` (title "PM", one `&lt;main id="view"&gt;`, a bottom tab bar, and a global Ask/confirm `&lt;dialog id="ask"&gt;`), logic in `pm-stack/console/public/app.js`, with `outbox.js` and `sw.js` alongside. Views are hash routes in the `VIEWS` map in `app.js` (at about line 4884); routing is `parse()` in the same file. An unknown hash falls back to `now`. The console is also served at the edge by the pm-console-edge Worker and wrapped as an Android TWA.

This chapter covers all 27 console hash views in the census re-derived 2026-11-27 (#14428 item 11), one entry each, in census order. The counting rule: one view per key of the `VIEWS` map in `app.js` after the page modules in `public/pages/index.js` register their routes (the app's own `routesKnown()`) — 24 keys written in the map itself plus the three page-module routes (`more/trends`, `more/reports`, `more/components`). A sub-address (`needs/<sub>`, `ask/say=<text>`, `now/done/<segment>`, `more/phone/<sub>`, `more/look/<segment>`) is a segment of its parent view, not a view of its own; `now/done` counts separately because it is a key of its own in the map. Every entry follows the same shape: what the view is, its inputs, its defaults where the survey gives them, its behaviour, and its failure shapes where the survey records them. Console actions post to the generic endpoint `POST /console/api/act`, covered in [The question sets, enumerated](question-sets.md) and [Aggregator HTTP routes and API](aggregator-routes.md).

## `#now` — Now

- **What it is:** The current board: in-progress and next work (`viewNow`).
- **Inputs:** Data from `Work.board()` (`console/src/work.ts`): queue counts, claimed, queued, and blocked-reason groups.
- **Defaults:** The fallback view: an unknown hash lands here.
- **Behaviour:** The console's home view, drawn from the queue's current state. Counts are fleet queue tasks (queue-broker state: claimed is in progress, queued is next, blocked is blocked) — same words as the aggregator's board-derived work-item counts (`/queue`, Overview in-progress), different source; the page says so and cross-links the aggregator's wording (#14063). The need-you tile is the console's own set, not the aggregator's NEEDS USER intake queue.
- **Failure shapes:** None recorded in the survey.

## `#now/done` and `#now/done/<segment>` — Done

- **What it is:** What was finished, in three segments — merged pull requests, finished queue tasks, closed issues (`viewDone`, reached as `now/done/<merged|tasks|closed>`, default `merged`).
- **Inputs:** The done document (`docs.done`): the last week's merged pull requests, finished tasks with their verdicts, and closed issues.
- **Behaviour:** Finished tasks with a recorded result are listed before those without one. Each segment names how much of the last week it shows, newest first.
- **Failure shapes:** Before the document is read the view says it is reading; a document with nothing read yet says so rather than showing an empty week.

## `#next` — Next

- **What it is:** Next-up work and the ops preview (`viewNext`, extra `ops`).
- **Inputs:** None recorded in the survey beyond the view's `VIEWS` entry.
- **Behaviour:** The forward-looking companion to `#now`.
- **Failure shapes:** None recorded in the survey.

## `#needs` and `#needs/&lt;sub&gt;` — Needs me

- **What it is:** Everything waiting on the owner (`viewNeeds`, extra `questions`), split into segmented sub-views: `questions`, `hands`, `reviews`, `other`.
- **Inputs:** The aggregator's needs-Drew census, `GET /api/needs-drew.json`, split by `console/src/work.ts` (`Work.census()`): hands items carry a `hands:v1` marker, reviews carry the `user/review` label with no hands marker, and the rest are escalated items with no stated action. Session-derived waiting items come from `/api/attention.json` (`Work.sessions()`, tier `needs-drew`). Decision sets appear under `questions`, read from the decisions file by `Work.questions()`: open questions only, newest set first, context clipped to 280 characters in the list.
- **Defaults:** The default segment is chosen by counts: questions first, then hands, then reviews.
- **Behaviour:** Answers happen in place: Reply posts `act("need.comment")`; **I did it** on a hands item posts `act("hands.done")`. The full answer mechanics are in [The question sets, enumerated](question-sets.md). D13: the phone names the buckets Questions, Only you can do, Reviews, and Escalated; the census vocabulary in code is hands, reviews, and escalated/other. The view names its source (the console's need-you set: open questions plus hands, reviews, escalated) and cross-links the aggregator `/attention` NEEDS USER intake queue as the different count (#14063).
- **Failure shapes:** None recorded in the survey.

## `#ask` and `#ask/say=&lt;text&gt;` — Ask

- **What it is:** The owner's own-words instruction composer (`viewAsk`): a textarea with Dictate via Web Speech where available, and the thread list of everything the owner has asked, with fleet replies, Reply, and Mark done.
- **Inputs:** The `say=` parameter is a prefill handed over by the Android app (share, selected text, or dictation; comment in `viewAsk`). Submitting posts `act("ask.send")`.
- **Behaviour:** `ask.send` files a GitHub issue and enqueues a top-band (priority 10) triage packet; replies read back from the issue. The aggregator-side prompt surface is `/chat` in [The aggregator website, page by page](website-pages.md).
- **Failure shapes:** None recorded in the survey.

## `#more` — More

- **What it is:** The index of the `more/*` views (`viewMore`, extra `ops`).
- **Inputs:** Links to every aggregator dashboard page, grouped by `PAGE_GROUPS`, rendered from the dashboard manifest data (`app.js` at about line 1185).
- **Behaviour:** The console's table of contents: the twenty-one views below are reached from here.
- **Failure shapes:** None recorded in the survey.

## `#more/trends` — Trends

- **What it is:** Day-by-day and, for longer windows, week-by-week charts of merged pull requests, features by project, issues and bugs opened or closed, time from pull request opening to merge, and finished queue tasks.
- **Inputs:** `GET /console/api/trends?days=<7|14|30|90>` reads GitHub issue and pull request history plus queue-store task-finish events (`console/src/trends.ts`, `console/src/reads.ts`). The GitHub history is kept and progressively filled in; the view says when older days are not whole yet. No machine is required for this read.
- **Behaviour:** The window choices are 7, 14, 30, and 90 days. Up to 31 days the charts are daily; longer windows are grouped by week. Refresh asks for a fresh read. The feature list links to the latest merged feature pull requests in each project.
- **Failure shapes:** When task history is unavailable, task charts are omitted; an incomplete GitHub history is identified in the view.

## `#more/reports` — Reports

- **What it is:** A prose report made on demand from the same history as Trends, for a day, a week, or 30 days. It can be read, copied, or shared as text.
- **Inputs:** `GET /console/api/trends?days=<1|7|30>&end=<date>` reads the underlying history; queue counts are included only for a report ending today (`console/public/pages/reports.js`, `console/src/trends.ts`).
- **Behaviour:** Earlier and later controls navigate historical periods. The report summarizes merged pull requests, feature work by project, issues and bugs, and finished queue tasks when task history is available; links in the feature section open the pull requests. This is the console's `#more/reports` view, not the aggregator's separate `/reports` review-conflict page documented in [The aggregator website, page by page](website-pages.md).
- **Failure shapes:** Partial or not-yet-whole history is called out in the report; unavailable task history omits the queue section.

## `#more/components` — Components

- **What it is:** Every shared element the app is built with, shown once each on one screen (`public/pages/components.js`, #14433).
- **Inputs:** The page's own `ELEMENTS` census: 15 shared elements (card, chip, row, tile, link, option, seg, stat, fold, empty, the chart kit's stat tiles, stacked columns and line chart, and the app frame's toast and sheet), each drawn with the class `app.css` gives it or the chart-kit function that makes it.
- **Behaviour:** Change the theme, the text size or the spacing in `#more/look` and this screen shows what every element does under it, so a look can be judged on all of them at a glance — and a new screen has something to copy instead of inventing its own.
- **Failure shapes:** None recorded.

## `#more/queue` — Queue

- **What it is:** Queue tasks by status (`viewQueue`).
- **Inputs:** Actions post to `POST /console/api/queue/&lt;task&gt;/&lt;action&gt;` (`console/src/server.ts`).
- **Behaviour:** The console's working cut of the queue; the aggregator's cross-project cut is `/queue` (board-derived work items, not live broker tasks).
- **Failure shapes:** None recorded in the survey.

## `#more/prs` — Pull requests

- **What it is:** The open pull request list and detail (`viewPrs`, `ops`).
- **Inputs:** Merge posts to `POST /console/api/prs/&lt;n&gt;/merge` (`Ops.mergePr`).
- **Behaviour:** Review and merge from the phone.
- **Failure shapes:** None recorded in the survey.

## `#more/issues` — Ranked issues

- **What it is:** Issues ordered by rank labels (`viewIssues`, `ops`).
- **Inputs:** Re-rank posts to `POST /console/api/priorities/&lt;issue&gt;/rank`; comment posts to `POST /console/api/issues/&lt;n&gt;/comment` (`Ops.priorities`, `Ops.setRank`, `Ops.comment` in `console/src/ops.ts`).
- **Behaviour:** Ranking here writes GitHub rank labels through the console — see D4 in [The aggregator website, page by page](website-pages.md): the aggregator service never writes labels; the console does.
- **Failure shapes:** None recorded in the survey.

## `#more/agents` — Agents

- **What it is:** The agent and session list (`viewAgents`, `ops`).
- **Inputs:** `GET /console/api/agents`.
- **Behaviour:** The console's cut of the session registry; the aggregator's cut is `/sessions`.
- **Failure shapes:** None recorded in the survey.

## `#more/messages` — Messages

- **What it is:** The owner's message pipeline and composer (`viewMessages`, `ops`): every message on every surface (connector, fleet, notes) with its real sender, plus the offers list.
- **Inputs:** `GET /console/api/pipeline` (filters: kind, surface, sender, receiver, batch, work state, unread), `GET /console/api/offers`; sends post to the action table as `message.send`, `message.broaden`, `message.offer`.
- **Behaviour:** The composer takes the bridge's whole addressing grammar — names, globs (`agent:f*`), groups, `all` — with a kind toggle (message / instruction) and an optional limit. A send answers with the bridge's match report (matched, delivered, recipients named, remaining), shown in place; while agents remain, **Broaden** delivers the send's own remainder on its batch, never a fresh fan-out. **Offer instead** posts the text as a claim offer with a slot count; the offer's row shows its claims as they land, and its claims and replies read together through the batch filter. Instructions reach agents sender-stripped (the bridge's job); this screen always shows the owner as sender. Sends ride the PM stack's bridge credential at the edge; without it they are held for a machine, like any other held action.
- **Failure shapes:** A send whose answer carries no match report is said so ("the bridge did not say who that reached"), never recounted console-side. The offers list failing to read is shown as its own empty state, apart from the pipeline's.

## `#more/services` — Services

- **What it is:** The systemd units the console may manage (`viewServices`, `ops`).
- **Inputs:** A per-unit action posts to `POST /console/api/services/&lt;unit&gt;/&lt;action&gt;`; logs read from `GET /console/api/services/&lt;unit&gt;/logs?lines=` (`Ops.services`, `Ops.serviceAct`, `Ops.serviceLogs`; the allowlist is `manageable()`).
- **Behaviour:** Only allowlisted units are manageable from this view.
- **Failure shapes:** None recorded in the survey.

## `#more/notify` — Notifications

- **What it is:** Push status and the subscription for this device (`viewNotify`).
- **Inputs:** `GET /console/api/push` (`console/src/push.ts`, `console/src/notify.ts`).
- **Behaviour:** The device-side companion to the aggregator's `/alerts/preferences` page.
- **Failure shapes:** None recorded in the survey.

## `#more/phone` and `#more/phone/&lt;sub&gt;` — Phones

- **What it is:** Linked phones and devices (`viewPhone`).
- **Inputs:** `GET /console/api/devices` (`console/src/devices.ts`).
- **Behaviour:** The view redraws on `more/phone` route changes; that redraw is the device-link flow's surface.
- **Failure shapes:** None recorded in the survey.

## `#more/signin` — Sign-in

- **What it is:** Sign-in state, with Google sign-in and edge-direct controls (`viewSignin`).
- **Inputs:** `GET /console/api/signin` (`Ops.signin`, `Ops.googleSignin`, `Ops.edgeDirect`).
- **Behaviour:** The console gate at `pm.security.cab` proxies dashboard paths only for a signed-in owner; this view is where that state is inspected and changed.
- **Failure shapes:** None recorded in the survey.

## `#more/away` — Cloudflare

- **What it is:** The "away" surface (`viewAway`): what the edge holds when the laptop is unreachable — held actions and kept answers (`console/edge/src/edge.ts`, `console/src/sync.ts`).
- **Inputs:** None recorded in the survey beyond the view's `VIEWS` entry.
- **Behaviour:** Actions taken while the laptop is away are held by the edge and replayed through the same audited action table when the host returns.
- **Failure shapes:** None recorded in the survey.

## `#more/machines` — Machines

- **What it is:** The machine list (`viewMachines`).
- **Inputs:** `GET /console/api/machines` in `console/src/server.ts`.
- **Behaviour:** A plain list view over the machine registry the console server holds.
- **Failure shapes:** None recorded in the survey.

## `#more/platform` — Platform

- **What it is:** The Cloudflare Workers platform screen (`viewPlatform`).
- **Inputs:** Drawn from `pm-stack/platform/workers.json`; workflow and deploy state from `Ops.platform()`.
- **Behaviour:** This view is the phone surface of the platform registry counted in [The workers](workers.md), where the 22/20/18 count reconciliation (D6) is recorded.
- **Failure shapes:** None recorded in the survey.

## `#more/device` — On this device

- **What it is:** This device's identity and link state (`viewDevice`).
- **Inputs:** None recorded in the survey beyond the view's `VIEWS` entry.
- **Behaviour:** The per-device companion to `#more/phone`, which lists all linked devices.
- **Failure shapes:** None recorded in the survey.

## `#more/look` and `#more/look/<segment>` — Appearance

- **What it is:** How the app looks on this device, in three segments — colours, typeface, size and spacing (`viewLook`, reached as `more/look/<colours|type|layout>`, default `colours`).
- **Inputs:** The device-local Look settings: follow the system's light/dark or one theme, the day and night themes, typeface, text size and spacing choices.
- **Behaviour:** A sample card shows each choice on text, small print, the three status colours and each kind of button. Choices are kept on this device only.
- **Failure shapes:** If the Look module cannot load on the device the view says so; a choice the device would not keep lasts only until the app is closed, and the view says that too.

## `#more/releases` — Releases

- **What it is:** Release and deploy workflow runs (`viewReleases`).
- **Inputs:** Runs from `Ops.releases()`; a deploy starts from `Ops.deploy()`.
- **Behaviour:** The console's release surface: inspect runs, start a deploy.
- **Failure shapes:** None recorded in the survey.

## `#more/roadmap` — Roadmap

- **What it is:** The console roadmap (`viewRoadmap`): capabilities and phases parsed from the plan doc (`console/src/plan.ts`: `CAPABILITIES`; `PHASES` is `doing`, `now`, `next`, `later`, `built`; `parseRoadmap`).
- **Inputs:** The plan doc the console parses; this view does not read the aggregator's `/api/roadmap.json`.
- **Behaviour:** D12: this is an independent roadmap implementation with its own vocabulary, not a client of the aggregator roadmap. The two surfaces coexist; neither is a view of the other.
- **Failure shapes:** None recorded in the survey.

## `#more/bugs` — Bugs

- **What it is:** Bug-kind plan issues (`viewBugs`): `Plan` issues with `kind: "bug"` in `console/src/plan.ts`.
- **Inputs:** The plan issue set the console holds.
- **Behaviour:** A filtered cut of the plan: bugs only.
- **Failure shapes:** None recorded in the survey.

## `#more/intake` — Intake

- **What it is:** The intake routing list (`viewIntake`): kinds note, bug, and change, with their label and rank routes (`plan.ts` `ROUTES`; `INTAKE_LABEL = "owner/intake"`).
- **Inputs:** A submission from this view.
- **Behaviour:** Submitting files a GitHub issue and a triage/prompt queue packet, routed by kind through the `ROUTES` table. A change against a capability with a roadmap file is also made by the platform itself, as a pull request against that file alone (`console/src/roadmap-change.ts`), when it can be made mechanically — a row added, moved, marked built, dropped, or retitled; the pull request is recorded on the issue, and a change that cannot be made mechanically says why in the answer and stands as issue and task alone.
- **Failure shapes:** None recorded in the survey.

Sources: PM-SURVEY website census (`pm-stack/console/public/app.js`, `pm-stack/console/public/pages/trends.js`, `pm-stack/console/public/pages/reports.js`, `pm-stack/console/src/server.ts`, `pm-stack/console/src/reads.ts`, `pm-stack/console/src/trends.ts`, `pm-stack/console/src/work.ts`, `pm-stack/console/src/ops.ts`, `pm-stack/console/src/plan.ts`), survey files `website-pages.md` and `disagreements.md` (D4, D12, D13), census date 2026-11-23.
