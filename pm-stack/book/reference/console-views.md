# The console, view by view

The PM console is a Bun server (`pm-stack/console/src/server.ts`, base path `/console`, port 4872) serving a static single-page application: shell `pm-stack/console/public/index.html` (title "PM", one `&lt;main id="view"&gt;`, a bottom tab bar, and a global Ask/confirm `&lt;dialog id="ask"&gt;`), logic in `pm-stack/console/public/app.js`, with `outbox.js` and `sw.js` alongside. Views are hash routes in the `VIEWS` map in `app.js` (at about line 2955); routing is `parse()` in the same file. An unknown hash falls back to `now`. The console is also served at the edge by the pm-console-edge Worker and wrapped as an Android TWA.

This chapter covers all 21 console hash views in the survey census, one entry each, in census order. Every entry follows the same shape: what the view is, its inputs, its defaults where the survey gives them, its behaviour, and its failure shapes where the survey records them. Console actions post to the generic endpoint `POST /console/api/act`, covered in [The question sets, enumerated](question-sets.md) and [Aggregator HTTP routes and API](aggregator-routes.md).

## `#now` — Now

- **What it is:** The current board: in-progress and next work (`viewNow`).
- **Inputs:** Data from `Work.board()` (`console/src/work.ts`): queue counts, claimed, queued, and blocked-reason groups.
- **Defaults:** The fallback view: an unknown hash lands here.
- **Behaviour:** The console's home view, drawn from the queue's current state.
- **Failure shapes:** None recorded in the survey.

## `#next` — Next

- **What it is:** Next-up work and the ops preview (`viewNext`, extra `ops`).
- **Inputs:** None recorded in the survey beyond the view's `VIEWS` entry.
- **Behaviour:** The forward-looking companion to `#now`.
- **Failure shapes:** None recorded in the survey.

## `#needs` and `#needs/&lt;sub&gt;` — Needs me

- **What it is:** Everything waiting on the owner (`viewNeeds`, extra `questions`), split into segmented sub-views: `questions`, `hands`, `reviews`, `other`.
- **Inputs:** The aggregator's needs-Drew census, `GET /api/needs-drew.json`, split by `console/src/work.ts` (`Work.census()`): hands items carry a `hands:v1` marker, reviews carry the `user/review` label with no hands marker, and the rest are escalated items with no stated action. Session-derived waiting items come from `/api/attention.json` (`Work.sessions()`, tier `needs-drew`). Decision sets appear under `questions`, read from the decisions file by `Work.questions()`: open questions only, newest set first, context clipped to 280 characters in the list.
- **Defaults:** The default segment is chosen by counts: questions first, then hands, then reviews.
- **Behaviour:** Answers happen in place: Reply posts `act("need.comment")`; **I did it** on a hands item posts `act("hands.done")`. The full answer mechanics are in [The question sets, enumerated](question-sets.md). D13: the phone names the buckets Questions, Only you can do, Reviews, and Escalated; the census vocabulary in code is hands, reviews, and escalated/other.
- **Failure shapes:** None recorded in the survey.

## `#ask` and `#ask/say=&lt;text&gt;` — Ask

- **What it is:** The owner's own-words instruction composer (`viewAsk`): a textarea with Dictate via Web Speech where available, and the thread list of everything the owner has asked, with fleet replies, Reply, and Mark done.
- **Inputs:** The `say=` parameter is a prefill handed over by the Android app (share, selected text, or dictation; comment in `viewAsk`). Submitting posts `act("ask.send")`.
- **Behaviour:** `ask.send` files a GitHub issue and enqueues a top-band (priority 10) triage packet; replies read back from the issue. The aggregator-side prompt surface is `/chat` in [The aggregator website, page by page](website-pages.md).
- **Failure shapes:** None recorded in the survey.

## `#more` — More

- **What it is:** The index of the `more/*` views (`viewMore`, extra `ops`).
- **Inputs:** Links to every aggregator dashboard page, grouped by `PAGE_GROUPS`, rendered from the dashboard manifest data (`app.js` at about line 1185).
- **Behaviour:** The console's table of contents: the sixteen views below are reached from here.
- **Failure shapes:** None recorded in the survey.

## `#more/queue` — Queue

- **What it is:** Queue tasks by status (`viewQueue`).
- **Inputs:** Actions post to `POST /console/api/queue/&lt;task&gt;/&lt;action&gt;` (`console/src/server.ts`).
- **Behaviour:** The console's working cut of the queue; the aggregator's cross-project cut is `/queue`.
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
- **Behaviour:** Submitting files a GitHub issue and a triage/prompt queue packet, routed by kind through the `ROUTES` table.
- **Failure shapes:** None recorded in the survey.

Sources: PM-SURVEY website census (`pm-stack/console/public/app.js`, `pm-stack/console/src/server.ts`, `pm-stack/console/src/work.ts`, `pm-stack/console/src/ops.ts`, `pm-stack/console/src/plan.ts`), survey files `website-pages.md` and `disagreements.md` (D4, D12, D13), census date 2026-11-23.
