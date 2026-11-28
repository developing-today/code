# The aggregator website, page by page

The aggregator dashboard is a Bun application: routing in `pm-stack/aggregator/src/server.ts`, the page manifest in `pm-stack/aggregator/src/pages.ts`, renderers in `pm-stack/aggregator/src/render.ts`. It binds `127.0.0.1:4870` and the tailnet address. In public it sits behind the console gate at `pm.security.cab`: the console proxies non-`/console` GET and HEAD paths to the dashboard for a signed-in owner.

This chapter covers all 29 aggregator routes in the survey census: the 23 manifest pages in `PAGES` (`pages.ts`) and the 6 detail and sub-routes under them. Manifest entries carry a navigation group from `PAGE_GROUPS` in the same file — Overview, Work, Ops, System — and a curation flag, `nav: primary` or `nav: dropdown` (the #1497 curation). Every entry below follows the same shape: what the page is, its inputs, its defaults where the survey gives them, its behaviour, and its failure shapes where the survey records them. The HTTP and API surface behind these pages is in [Aggregator HTTP routes and API](aggregator-routes.md); the phone surface is in [The console, view by view](console-views.md).

## `/` — Overview

- **What it is:** The stack front door (#1134): needs-you, projects, quotas, and health at a glance. Manifest group Overview, `nav: primary`.
- **Inputs:** None recorded in the survey. Machine twin: `/api/overview.json`.
- **Behaviour:** The landing page of the dashboard; the item detail and discuss sub-routes (below) name it as their parent. In-progress and blocked counts here are board-derived work items, not fleet queue tasks — the console Now page counts the queue broker's live tasks instead (same words, different source). The Needs-you count is the NEEDS USER intake queue, not the console's need-you set (open questions plus hands, reviews, escalated).
- **Failure shapes:** None recorded in the survey.

## `/today` — Today

- **What it is:** The day's top objectives and per-project momentum. Manifest group Overview, `nav: primary`.
- **Inputs:** The full form is `/today?full=1`. Machine twin: `/api/today.json`. Fragment form: `/frag/today`.
- **Behaviour:** Renders the day view from the canonical projection; the fragment form serves the Datastar partial for in-page refresh.
- **Failure shapes:** None recorded in the survey.

## `/digest` — Digest

- **What it is:** The scheduled PM digest (#1278): moved, blocked, needs Drew, done, and quota outlook, drawn from the canonical projection. Manifest group Overview, `nav: dropdown`.
- **Inputs:** The archive form is `/digest?date=&lt;date&gt;`. Machine twin: `/api/digest.json`; the digest list is `/api/digests.json`.
- **Behaviour:** With no date, the page shows the current digest; with a date, the archived digest for that date.
- **Failure shapes:** None recorded in the survey.

## `/attention` — Needs you

- **What it is:** The attention queue: NEEDS USER against NEEDS ORCHESTRATOR, and every live session. Manifest group Overview, `nav: primary`.
- **Inputs:** Machine twin: `/api/attention.json`. Fragments exist under `/frag/attention/*` (recorded in a comment at the `/attention` branch of `server.ts`).
- **Behaviour:** The page the needs-Drew and blocked-age push notifications land on, anchored to the item acted on. This is the NEEDS USER intake queue the orchestrator works first — different from the console's need-you set (open questions plus hands, reviews, escalated); each surface names its own source (#14063).
- **Failure shapes:** None recorded in the survey.

## `/decisions` — Decision sets

- **What it is:** The index of every raised decision set (#8867): new today, this week, the rolling last 7 days, and all sets, with open-question counts per set. Manifest group Overview, `nav: primary` (promoted in #9354, per the comment in `pages.ts`).
- **Inputs:** Each row opens the set's own page, `/decisions/&lt;setId&gt;` (below).
- **Defaults:** `pages.ts` declares no `api:` twin for this page — one of only three manifest pages without one. The server implements a decisions API regardless; see D7 in the Sources note and [The question sets, enumerated](question-sets.md).
- **Behaviour:** The index is a navigation surface over the decision sets; answering happens on the per-set page or in the console. The index has four views, chosen with `?view=` (`handleDecisionsIndex` in `server-decision-routes.ts`; `decisionSetsForView` in `decisions.ts`): `today` (raised on the current America/Chicago calendar day), `week` (the current Chicago calendar week, Monday–Sunday), `recent` (raised in the rolling last 7 days), and `all` (everything). Since the decision-set aging policy landed (#14064, PR #14413), `recent` is the default view, so the landing view is the current workload, not the backlog; a set whose raise time cannot be parsed appears only in `all`. Every set also carries an aging status (`decisionSetAging` in `decisions.ts`): `settled` (nothing open — settled wins over idle), `abandoned` (still open but idle a full 7 days since its last activity — the latest resolution time, else its raise time; a set with no datable instant ages as `active`, never retired undated), else `active`. The `all` view renders the abandoned sets as a fold, with their set ids and a dismiss-all backed by `POST /api/decisions/dismiss` (see [Aggregator HTTP routes and API](aggregator-routes.md)).
- **Failure shapes:** None recorded in the survey.

## `/reports` — Reports

- **What it is:** Review report versions and resolve prose conflicts without overwriting stored text. Manifest group Work, `nav: dropdown`.
- **Inputs:** Machine twin: `/api/reports.json`. Writes: `POST /api/reports/resolve` and `POST /api/reports/late-update`.
- **Behaviour:** A conflict resolves by choosing a stored version; the stored text itself is not overwritten. This is the aggregator's `/reports` route, distinct from the console's on-demand prose reports at `#more/reports` (see [The console, view by view](console-views.md)).
- **Failure shapes:** None recorded in the survey.

## `/board` — Board

- **What it is:** The curated cross-repo work-in-progress board. Manifest group Work, `nav: dropdown`.
- **Inputs:** Machine twin: `/api/board.json`. Fragment form: `/frag/board`.
- **Behaviour:** Board items carry provenance from their sources; the Kaneo poll feeds the board, and board policy filters retired "ROADMAP Done table" cards out.
- **Failure shapes:** None recorded in the survey.

## `/roadmap` — Roadmap

- **What it is:** The program roadmap at every grain. Manifest group Work, `nav: primary`.
- **Inputs:** Machine twin: `/api/roadmap.json`. The route preserves the view and filter parameters `view`, `viewer`, and `vf.*`.
- **Behaviour:** Rows merge the Grist table, the repo files, and the project pool; on a title collision the repo file wins over Grist (#1738). Annotations write back through `POST /api/roadmap/annotations` and `POST /api/roadmap/annotations/alias`.
- **Failure shapes:** None recorded in the survey.

## `/conflicts` — Source conflicts

- **What it is:** One read-only queue of every live source disagreement, with both values and its age. Manifest group Work, `nav: dropdown`.
- **Inputs:** Machine twin: `/api/conflicts.json`.
- **Behaviour:** Read-only by design: the page presents disagreements; it does not resolve them.
- **Failure shapes:** None recorded in the survey.

## `/quota` — Usage &amp; billing

- **What it is:** Token quotas, PAYG dollars, and every SaaS service's usage against its limits. Manifest group Ops, `nav: primary`.
- **Inputs:** Machine twin: `/api/quota.json`. Refresh: `POST /quota/refresh` and `POST /api/quota/refresh`. One service's detail page is `/quota/service/&lt;id&gt;` (below).
- **Behaviour:** Data comes from the quota module family in `src/quota-*.ts` (about 25 modules, including current state, forecast, anomaly, reweight, admission, and the weekly review) and the charges and spend machinery behind it (`src/charges-*.ts`, `src/spend-*.ts`, `src/saas-extract.ts`).
- **Failure shapes:** None recorded in the survey.

## `/dns` — DNS register

- **What it is:** Every domain: next renewal, unused, earmarked (#902's register). Manifest group Ops, `nav: dropdown`.
- **Inputs:** Machine twin: `/api/dns.json`.
- **Behaviour:** A register view over the domain estate.
- **Failure shapes:** None recorded in the survey.

## `/aa` — Cost Explorer

- **What it is:** Two tabs: the AA Explorer (Artificial Analysis measurements, frontier charts, plan, API, and per-task efficiency) and the Model Explorer (every model in the fleet's catalogs, pool seats, plan, measured value). Manifest group Ops, `nav: dropdown`.
- **Inputs:** Machine twin: `/api/aa.json`. The static computation asset is `/assets/aa-explorer-computations.js`.
- **Behaviour:** The page reads the deployed OpenCode config's provider catalogs and route definitions and joins them to Artificial Analysis measurements; it measures models and makes no model calls — see [Model dependencies](models.md).
- **Failure shapes:** None recorded in the survey.

## `/sessions` — Sessions

- **What it is:** The agent and session identity registry: who ran what, tokens, task links. Manifest group Ops, `nav: dropdown`.
- **Inputs:** Machine twin: `/api/registry.json`. The twin is not `/api/sessions.json`.
- **Behaviour:** Session records carry the session's model or route as a provenance string.
- **Failure shapes:** None recorded in the survey.

## `/search` — Search

- **What it is:** Search over board, roadmap, registry sessions, DNS, services, and tier items, by title or by linked issue or PR reference. Manifest group Ops, `nav: dropdown`.
- **Inputs:** Machine twin: `/api/search.json`.
- **Behaviour:** A title and reference search over the aggregator's own projections. It is not a vector search and uses no embedding model — see [Model dependencies](models.md).
- **Failure shapes:** None recorded in the survey.

## `/events` — Events

- **What it is:** The cross-stack chronological stream of dashboard actions, orchestration publishes, and session outcomes. Manifest group Ops, `nav: dropdown`.
- **Inputs:** Machine twin: `/api/events.json`, paginated; `server.ts` emits a `rel=next` link for the next page.
- **Behaviour:** The HTML page links to its twin.
- **Failure shapes:** None recorded in the survey.

## `/comments` — Comments queue

- **What it is:** Dashboard comments still waiting on a loop (#1210): awaiting triage, GitHub comments pending and failed, kind and state counts, and the last error per failed item. Manifest group Work, `nav: dropdown`.
- **Inputs:** Machine twin: `/api/comments/queue.json`; the full-list twin is `/api/comments.json`.
- **Behaviour:** The dashboard itself never writes GitHub: a fleet job applies and posts comments and reports the result back through `POST /api/comments/github-result`.
- **Failure shapes:** A failed item shows its last error on this page.

## `/chat` — Chat

- **What it is:** The prompt chat and session surface (#10035): every prompt submission in its thread — reply, cross-references, archive — plus the unread count behind the navigation badge. Manifest group Work, `nav: dropdown`.
- **Inputs:** The manifest names the twin `/api/prompt/threads`. The full prompt route family is in [Aggregator HTTP routes and API](aggregator-routes.md).
- **Behaviour:** A prompt submitted with notifications on receives a `prompt`-class push when it is answered or fails.
- **Failure shapes:** A failed prompt is one of the two events that trigger the `prompt`-class push.

## `/program` — Program status

- **What it is:** The projects hub: every project's status, roadmap, and tracked work. Manifest group Work, `nav: primary`.
- **Inputs:** Machine twin: `/api/program.json`; the per-project twin is `/api/program-project.json`. One page per project lives under `/program/&lt;id&gt;` (below).
- **Behaviour:** The projects-pool section of this page is the target of the `/projects` redirect (below).
- **Failure shapes:** None recorded in the survey.

## `/queue` — Queue

- **What it is:** In progress, blocked, recently completed, and next-up work across projects. Manifest group Work, `nav: primary`.
- **Inputs:** Machine twin: `/api/queue.json`. Fragment form: `/frag/queue`. Orchestration twin: `/api/orchestration.json`.
- **Behaviour:** A projection of queue state across projects; the queue itself is covered in the fleet book at [https://developing-today.github.io/code/fleet/](https://developing-today.github.io/code/fleet/). Rows are board-derived work items (board card dedupe: one row per work item, issue-first), not fleet queue tasks — the console Now page counts the queue broker's live tasks instead (same words, different source, #14063).
- **Failure shapes:** None recorded in the survey.

## `/health` — Health

- **What it is:** Service and page health (#1147): live probes for every stack service, feed section ages, last job runs, this dashboard's render history, and the services and ports topology (#1137). Manifest group System, `nav: primary`.
- **Inputs:** Machine twin: `/api/health` — no `.json` suffix. One health assembly feeds both the page and the twin.
- **Behaviour:** Sources named in code: `src/health.ts`, `src/stack-health.ts`, `src/host-health.ts`, `src/build-health.ts`, the `src/watchdog*.ts` and `src/slo*.ts` modules, and `pm-stack/host-health-collector.py`, the laptop user-service snapshot the packaged aggregator cannot collect itself (claimant mode file, backup last run, and user-unit and timer states for pm-git-sync, pm-projector-reconcile, pm-kaneo-sync-guard, and pm-dashboard-backup). The twin additionally returns per-page render state: each manifest page's route, last-rendered time, and ok flag — the #1147 render registry.
- **Failure shapes:** A page whose renders fail appears in the twin's render state with its ok flag false.

## `/alerts` — Alerts

- **What it is:** The alert throat (#3676): sentinel incidents, Web Push delivery with one escalation, ack and close, and the Turso ledger behind them. Manifest group System, `nav: primary`.
- **Inputs:** Machine twin: `/api/alerts.json`. Incidents carry the lifecycle `open`, `acked`, `closed` (`src/alerts.ts`); ack and close happen on this page. The preferences sub-route is `/alerts/preferences` (below).
- **Behaviour:** External producers enter through `POST /api/alerts`; the engine in `src/alerts.ts` decides delivery per subscription.
- **Failure shapes:** None recorded in the survey.

## `/notifications` — Notifications

- **What it is:** Notification measurement (#3319): the weekly sent-against-acted-on roll-up over the alert throat's send ledger, and display-only default-off proposals under 10%. Manifest group System, `nav: dropdown`.
- **Inputs:** Machine twin: `/api/notifications.json`.
- **Behaviour:** Display and measurement only; proposals on this page are default-off and display-only.
- **Failure shapes:** None recorded in the survey.

## `/pages` — All pages

- **What it is:** The `pages.ts` manifest rendered as a page: every registered view in one place. Manifest group System, `nav: dropdown`.
- **Inputs:** `pages.ts` declares no twin for this page.
- **Behaviour:** This page is the manifest's self-listing; this chapter's 23 manifest entries are its content.
- **Failure shapes:** None recorded in the survey.

## `/item/&lt;subject&gt;` — Item detail

- **What it is:** The detail page for one item. Parent: `/`. Recorded in `SUBROUTE_PARENTS` in `pages.ts`.
- **Inputs:** Subject forms handled in `server.ts` are object references with the prefixes `gh:`, `session:`, and `orch:`. The machine twin pattern is `/api/item/&lt;id&gt;.json`.
- **Behaviour:** One page per subject, resolved from the reference in the path.
- **Failure shapes:** An unresolved id is a 404 case (#4969, comment in `server.ts`).

## `/discuss/&lt;subject&gt;` — Discuss

- **What it is:** The discussion and comment surface for one item. Parent: `/`. Recorded in `SUBROUTE_PARENTS`.
- **Inputs:** Writes go through `POST /api/comments`.
- **Behaviour:** Comments written here enter the dashboard comment store; the dashboard itself never writes GitHub — a fleet job applies the comment and reports back.
- **Failure shapes:** None recorded in the survey.

## `/program/&lt;id&gt;` — One project

- **What it is:** One project's page. Parent: `/program`. Recorded in `SUBROUTE_PARENTS`.
- **Inputs:** Data twin: `/api/program-project.json`.
- **Behaviour:** The per-project cut of the program hub: status, roadmap, and tracked work for the one project named in the path.
- **Failure shapes:** None recorded in the survey.

## `/quota/service/&lt;id&gt;` — One service

- **What it is:** One SaaS or quota service's detail page. Parent: `/quota`. Recorded in `SUBROUTE_PARENTS`.
- **Inputs:** Twin: `/api/quota/service.json`, read in `server.ts` in a `?service=` style. UNVERIFIED: the exact query contract beyond the route existing.
- **Behaviour:** The per-service cut of the usage and billing page.
- **Failure shapes:** None recorded in the survey.

## `/alerts/preferences` — Notification preferences

- **What it is:** The preferences surface for every notification class: per-class toggles, quiet hours, and a per-class test send. Parent: `/alerts`. Recorded in `SUBROUTE_PARENTS`.
- **Inputs:** Twin: `/api/alerts/preferences.json`. The page reads stored subscriptions through the `/api/push/subscriptions` upsert path (comment in `server.ts`).
- **Defaults:** All eight preference classes default on for a new subscription; quiet hours default to 23:00–07:00 America/Chicago. The eight classes are `incident`, `needs-drew`, `quota-wall`, `quota-near-wall`, `section-risk`, `stack-health`, `blocked-age`, and `prompt`. D14: issue #8882's premise of six classes is stale; the code defines eight.
- **Behaviour:** Toggles write the subscription's class map, which the engine reads on its next tick. Breakthrough classes ignore quiet hours: an incident is always immediate.
- **Failure shapes:** None recorded in the survey.

## `/decisions/&lt;setId&gt;` — One decision set

- **What it is:** One decision set's answer page, reached from `/decisions` and from the decision-set push. The model is in the header of `pm-stack/aggregator/src/decisions.ts`.
- **Inputs:** The token in the URL, `?t=&lt;token&gt;`, is part of the address. Answers post to `/api/decisions/&lt;setId&gt;/submit`, `/defer`, and `/request-info`, with draft and edit events to `/api/decisions/&lt;setId&gt;/events`.
- **Behaviour:** The page is the answer surface for a set: one push covers the whole set and its tap opens this page. Parent by navigation is the `/decisions` manifest page. D8: `SUBROUTE_PARENTS` does not list this page, so `manifestRouteFor` returns null for it and its renders sit outside the #1147 render registry that `/api/health` reports.
- **Failure shapes:** None recorded in the survey.

## `/projects` — Redirect, not a page

- **What it is:** Not a page. The route redirects to the projects-pool section of `/program` (manifest comment in `pages.ts`; route branch in `server.ts`).
- **Inputs:** The redirect preserves the `view`, `viewer`, and `vf.*` parameters. Its machine twin `/api/projects.json` still serves.
- **Behaviour:** Included in the census as the sixth sub-route so the redirect is on record; it renders no page of its own.
- **Failure shapes:** None recorded in the survey.

Sources: PM-SURVEY website census (`pm-stack/aggregator/src/pages.ts`, `pm-stack/aggregator/src/server.ts`, `pm-stack/aggregator/src/render.ts`, `pm-stack/aggregator/src/decisions.ts`, `pm-stack/aggregator/src/notifications.ts`, `pm-stack/aggregator/src/alerts.ts`), survey files `website-pages.md`, `push.md`, and `disagreements.md` (D7, D8, D14), census date 2026-11-23.
