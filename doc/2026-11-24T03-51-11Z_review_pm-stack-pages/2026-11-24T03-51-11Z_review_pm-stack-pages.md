# PM stack pages review — every page, live, with screenshots

- **Date:** 2026-11-24 (live pass 2026-11-24T03:25–03:35Z; commissioned 2026-11-23 ~21:21 CST)
- **Reviewer:** Archer (PM-PAGES-REVIEW lane)
- **Scope:** every page of the PM stack — the pm-aggregator's 22 deployed pages, 5 subroutes, the pm-console, and the hosted apps it binds together (dispatch, projector, Kaneo, Pad, Grist, ready-for-agent) — plus the fleet feed underneath all of it.
- **Method:** full rendered text read for every page (in `supplements/page-texts/`), a full-page screenshot of every page (in `screenshots/`, 37 files), live timing/size probes from the laptop's own loopback, the aggregator's `/api/health` (in `supplements/agg-health.json`), and the pm-stack sources on main.
- **Two caveats that are themselves findings:** (1) the deployed aggregator is built from a 2026-11-15 commit — 4,687 commits behind main — so several defects below are defects of the _deployment_, already fixed in code nobody is running (see Finding 1 and issue #14059). (2) The aggregator was restarted at 2026-11-23 21:24 CST, mid-review, by the #13902 fix (token re-read); the feed and pages were re-probed after the restart, and the post-restart push proof succeeded.

## Headline findings, worst first

1. **The deployed aggregator is nine days and 4,687 commits behind main.** Built from `7d3dcebe` (2026-11-15). The Chat page 404s live, Sessions cold-hangs, and an unknown number of landed fixes simply aren't in the app Drew uses. The console has the same disease (#13833). Filed: #14059.
2. **Search is unusable and the registry read is far too heavy.** `/search?q=fleet` took 33.5 s and returned 941 KB; `/api/registry.json` is 6.7 MB; `/sessions` timed out at 30 s cold. Filed: #14060.
3. **When a data source dies, facets render FIXTURES — invented sample data — as if it were real.** Board, Roadmap, Items, Comments, Quota, Attention, DNS and Registry all list fixtures as their fallback in the aggregator's own provenance matrix. In a stack whose brief is "works with no machine", the failure shape is fabrication, not an honest unavailable state. Filed: #14065.
4. **The Notifications measurement is blind.** All 269 send-ledger rows were excluded for having no usable timestamp, so the sent-vs-acted-on page reports "no sends" and computes its proposals over an empty set — while pushes demonstrably send. Filed: #14061.
5. **The Health page contradicts itself.** Top-level `status: "ok"` beside `degraded: true`; the SLO table marks facets "exhausted (0.0% remaining)" that are inside their targets; the feed is permanently "stale" because the freshness threshold (5 min) equals the collector's own cadence (~5 min), so the "Something below is degraded" banner is effectively always on. Filed: #14062.
6. **The surfaces contradict each other on basic counts.** Aggregator Queue: 523 in progress / 266 blocked. Console Now: 58 in progress / 316 blocked / 5 queued. Aggregator "Needs you": 2. Console "need you": 394. Same words, different numbers, no on-page reconciliation. Filed: #14063.
7. **Source conflicts: 77 live, 74 of one class, oldest ≥34 days, no resolution path.** The derived-state logic systematically disagrees with the GitHub Project board; the conflicts page is read-only pending trust rulings (slice 2 of #1282). Every other page carries the fallout as ⚠ markers — 122 on the Board page alone.
8. **The Decisions index is a graveyard.** 89 decision sets from a single day (2026-11-16) still sit pending with every question open; nothing ages, expires, or bulk-dismisses them. Filed: #14064.
9. **Today's "Top objectives" are polluted by quota spam.** After two real objectives, entries 3–15+ are wall notices ("OpenAI is walled" ×9, "at 100%" ×6). The same wall notices repeat three times in different forms on /quota and again on Overview and Today. The WIP readout ("in-review 366/10") is labelled "informational only" — a violated cap nobody acts on.
10. **Overview's needs-you items render as full evidence essays** — a wall of giant bold text where a glance surface should be — and its in-progress list (~200 items) contains literal duplicates (the same issue/PR twice in one list).
11. **Muse sessions are not wired into the stack's own session surfaces.** The Attention page says so itself: no launch records in the feed yet; the runtime exposes no session list to scrape. The main agent's sessions are invisible to the system that tracks every other worker. (Known gap, tracked from #906 §3 — recorded here because it is the stack's biggest blind spot.)
12. **The 404 page is an unstyled browser-default "Not found"** with no app chrome, no nav, and no way back — and it is what every mistyped or not-yet-deployed route (including /chat) shows.
13. **Decision links hard-code the tailnet hostname and embed a bearer token in the URL** (`https://amd-6.raptor-pumpkinseed.ts.net/decisions/<id>?t=…`). They work on-tailnet and break everywhere else; the token sits in browser history and any shared link.
14. **Kaneo's root renders a black page** in a fresh profile — no spinner, no message (capture at 3.5 s; at minimum the first-paint/loading state is a void). Ready-for-agent's root sits on "Loading catalog… / Save is blocked until the catalog is ready".
15. **Page weight is out of hand.** /program ships 1.43 MB of HTML, /board 878 KB, /aa 644 KB; /health took 28.9 s to render in-browser; most pages are 8,000–10,000 px tall. These are documents, not dashboards.

## Fixed in this pass

- **PR #14058 (merged `e68cf267`)** — `ageMsText()` already returns "2m old", but seven call sites appended a second " old": the Overview health strip rendered **"feed 2m old old"** (live) and six watchdog alert strings read "…old old" in Drew's pushes. Seven one-line edits across `render.ts` + `watchdog.ts`; no test asserted the doubled strings. It becomes visible when the deployment next follows main — which is Finding 1.

## Filed from this review

- #14059 — pm-aggregator deployment is 4,687 commits behind main — PM apps must follow main
- #14060 — search takes 33 s and the registry twin ships 6.7 MB — bound the reads
- #14061 — all 269 send-ledger rows lack a usable timestamp — the measurement page is blind
- #14062 — /health SLO budgets read "exhausted" inside target; feed permanently "stale" at its own cadence
- #14063 — PM surfaces contradict each other: queue 523 vs 58, needs-you 2 vs 394
- #14064 — 89 decision sets from 2026-11-16 still pending — decision sets need an aging policy
- #14065 — fixture fallbacks render invented data when sources die — fail visibly instead
- #14066 — /aa dataset-version picker lists the same version six times with identical labels

Findings 7 (conflicts) and 11 (Muse sessions unwired) are already owned — #1282 slice 2 and #906 §3 respectively — and are recorded here rather than re-filed.

## Per-page review — pm-aggregator (127.0.0.1:4870)

### / — Overview (`screenshots/overview.png`)

Purpose: the front door — needs-you queue, health strip, work summary. What works: it is genuinely the right shape (needs-you first, health one line, live SSE). Defects: needs-you items render as full evidence essays (the Porkbun domain item is a wall of giant bold paragraphs — unreadable as a glance surface); the in-progress list runs to ~200 undifferentiated items with literal duplicates (D3 design ×2, atlas D1 hot-cache ×2, claimant-latency ×2, id phase-14 ×2); the health strip carried the "feed 2m old old" doubling (fixed, PR #14058) and says "N of 10 services up · degraded" without naming which; the page-picker dropdown at the top shows the current page's blurb truncated mid-word ("(#1134): n…").

### /today (`screenshots/today.png`)

Purpose: the day's shape — objectives, queue pressure, pushes. Defects: "Top objectives" are polluted past position 2 by quota wall notices (Finding 9); the WIP violation "in-review 366/10" is displayed and immediately excused as "informational only"; wall/at-100% notices appear here, on Overview, and three times on /quota — the same fact, four costumes, no single home.

### /digest (`screenshots/digest.png`)

Purpose: the periodic rollup. Renders 3.6 s for a text page. It inherits every upstream defect (conflict markers, wall spam) without adding a freshness line of its own that a reader can trust — the feed age it would cite is the permanently-"stale" one (Finding 5).

### /attention (`screenshots/attention.png`)

Purpose: sessions needing a human or the orchestrator. Counts: Needs User 2 · Needs Orchestrator 89 · All sessions 61 — the 89 orchestrator items are the real queue and get the same visual weight as the 2 user items. Defect (Finding 11): the page itself states Muse sessions are **not wired** — "no launch records in the feed yet… the runtime exposes no session list to scrape". The flagship agent is invisible to the attention system.

### /decisions (`screenshots/decisions.png`)

Purpose: index of decision sets. Finding 8: 89 sets from 2026-11-16, all pending; 112 "Raised Nov" lines; no aging, no fold, no bulk action. The sets Drew is actually asked about in chat drown in a week-old batch. Filed: #14064.

### /reports (`screenshots/reports.png`)

Purpose: status reports per project. Live state: **"0 report versions available"** — the page's entire purpose has no data. Either the report writer stopped or the reader points at an empty store; the page does not say which, or since when.

### /board (`screenshots/board.png`)

Purpose: the Project board mirror. 878 KB of HTML; carries **122 ⚠ conflict markers** — the derived-state disagreement (Finding 7) rendered as decoration on every affected card, with no path from a marker to a resolution. A board that cries wolf 122 times teaches the reader to ignore all of them.

### /roadmap (`screenshots/roadmap.png`)

Purpose: epics vs the Grist roadmap. Its own drift readout: **171 Grist rows with no matching Project epic · 25 Project epics with no roadmap row** (#1144's known drift, still unclosed). 400 KB page. The drift is reported faithfully — and has been actionable-looking-but-unactioned for weeks; there is no "file/fix" affordance next to a drift row.

### /conflicts (`screenshots/conflicts.png`)

Purpose: the source-conflict inbox. 77 live conflicts, 74 of one class (Project Status vs dashboard-derived state), oldest ≥34 days. The page is honest that resolution waits on per-class trust rulings (slice 2 of #1282) — which means the queue has been read-only for over a month while the class keeps growing. Finding 7.

### /quota (`screenshots/quota.png`) and /quota/service/<id> (`screenshots/quota-service.png`)

Purpose: plan usage + credential lifecycle. Defects: leads with "Cloudflare — R2 storage — **1103% of limit used**" with no inline explanation (the number is real — storage vs a small included tier — but reads as an emergency); the same wall/at-100% facts repeat three times in different forms on this one page; the credential lifecycle table runs ~50 rows of "unknown · expiry unknown" with duplicates (OpenCode Go (drewrypope) ×3, Antigravity ×3) — and buries the one actionable row: **"Claude credential expired · Mon, Nov 23, 3:03 PM CST · user action: Open Claude Code to sign in again"**. A page whose job is "what needs me" hides its only answer in row ~40 of a table of unknowns.

### /dns (`screenshots/dns.png`)

Purpose: domain portfolio. **The best page in the stack.** 40 domains, $612.54/yr, 3 in use / 18 earmarked / 19 unused, registrar + renewal state per row. One defect: drewrypope.com has auto-renew OFF with the note "identify which account holds it" — an actionable item with no owner and no link to act. More pages should be built like this one.

### /aa — Cost Explorer (`screenshots/aa.png`)

Purpose: AI model cost/benchmark inventory. Defects: the dataset-version picker lists the same 2026-10-31 inventory **six times with identical truncated labels** (Finding 15 / #14066); the underlying snapshots end 2026-11-16 while collection ran 2026-11-23 — a week of staleness the page does state, to its credit. 644 KB, 4.1 s render.

### /sessions (`screenshots/sessions.png`)

Purpose: the agent-session registry (28,479 records). Defects: 30 s+ cold hang (Finding 2); the page ships **100 heavy cards**, each with an Act-as dropdown **defaulting to "Drew"**, a Poke button, an answer box, and a two-step Stop — a lookup page carrying 100 impersonation panels; the default window ("last 7 days") silently hides 17,601 records; the header brags "26,780,469 tokens recorded on this page" — a vanity metric on a page whose job is finding one session.

### /search (`screenshots/search.png`)

Finding 2: 33.5 s and 941 KB for a one-word query; the JSON twin takes 5.6 s / 901 KB. There is no visible result cap or pagination story. As shipped, the fastest way to find something in the PM stack is to not use its search.

### /events (`screenshots/events.png`)

Purpose: the cross-source event stream. Defect: every actor renders as "aggregator (unknown)" / "kaneo (unknown)" — the actor identity is dropped somewhere in the pipeline, so the audit stream can't say who did anything. 342 KB, 4.6 s.

### /comments (`screenshots/comments.png`)

Purpose: the outbound-comment queue. **Healthy**: 0 queued of 2 total, a clean empty state that says what the page does. One of only two pages with nothing wrong found (with /dns).

### /program (`screenshots/program.png`) and /program/<id> (`screenshots/program-project.png`)

Purpose: programme rollups (Fleet, ID, Atlas, mcpx…). 1.43 MB — the heaviest page in the stack. The per-programme drill-down for the PM dashboard itself reads **"No status report yet for this project"** — the programme that runs the stack has no report in it (same empty store as /reports). Programme slugs truncate mid-word in links ("…stale-ope").

### /queue (`screenshots/queue.png`)

Purpose: work-item visibility — but **not** the fleet queue: it reconciles board + Project pool + orchestration inputs, which is why its 523/266 counts contradict the console's real queue numbers (Finding 6 / #14063). The initial HTML is only 22 KB but the browser DOM expands to ~36,000 lines of datastar fragments; the page never states plainly "this is not the task queue".

### /health (`screenshots/health.png`)

Purpose: service + feed + gatherer health. Finding 5 in full: contradictory top-level status, an SLO table that marks in-target facets "exhausted", a permanently-stale feed banner, a gatherer table with ~44 rows of duplicate providers under raw internal keys (`_antigravity`, `_claude`, `_openai · default`), 11 providers in "error" with no error text, the identical useless lag sentence ~44 times, "Behind by: unknown" on 13 of 14 feed sections, the Meta gatherer stale since 2026-11-21, and a collector checkout-drift line that says "drift unknown" and then states the drift as fact ("1 local commit not on origin/main"). Also: push sources github/orchestration/reconcile show "silent" with zero events ever; Kaneo shows 600 notices vs fleet-feed's 271 with no explanation of the two counting worlds; the RFA panel shows the dispatch layer nearly idle (1 repo, merge policy OFF, 1 work item) while the fleet queue holds hundreds of tasks. Slowest page in the stack in-browser: 28.9 s.

### /alerts (`screenshots/alerts.png`) and /alerts/preferences (`screenshots/alerts-preferences.png`)

Purpose: incident list + push preferences. The alerts list itself works and the preferences subpage is coherent. The structural defect is upstream: the throat's token mismatch (#13902, fixed by restart during this review) meant broker-raised pushes dropped silently for days — the page had no way to show "pushes are being dropped", which is exactly the class of failure an alerts page exists to reveal. (See also Finding 4: the send ledger that would prove delivery is timestamp-blind.)

### /notifications (`screenshots/notifications.png`)

Finding 4: "No sends in the rolling four-week window" printed directly above "269 ledger row(s) carried no usable send timestamp and were excluded". The page presents a broken instrument as a calm empty state. Filed: #14061.

### /pages (`screenshots/pages.png`)

Purpose: the page directory. It lists the pages with their blurbs — and is how this review found the manifest mismatch: the deployed directory has 22 pages; main's manifest has 23 (Chat). A directory that silently reflects a stale build is a small thing; combined with Finding 1 it means even the map is out of date.

### /item/<ref> (`screenshots/item.png`)

Purpose: drill-down for a work item. For orchestration subjects it shows **"1 linked · 0 resolved — none resolved"**: the page cannot resolve its own linked issue from current sources, and the activity stream is empty. The drill-down drills into a wall.

### /discuss/<ref> (`screenshots/discuss.png`)

Purpose: per-item discussion surface. Renders, but inherits the item page's resolution failure — a discussion about an item the stack can't identify is a comment box attached to a reference number.

### /projects → /program#projects-pool

A 302 redirect into an anchor. Works; noted only because the anchor target (the projects pool) is mid-page on the heaviest page in the stack, so the "redirect" lands the reader in a 1.43 MB document hunting for a section.

### /chat (main only — 404 live)

Exists in main's manifest (#10035) and returns the bare unstyled 404 on the deployed build. The single clearest proof of Finding 1: a whole page of the product is missing in production while present in the code.

### The 404 itself (`screenshots/chat.png` shows it)

"Not found / no page at /chat" — 68 bytes, no chrome, no nav, no search, no way back. Every unknown URL in a 23-page app deserves better than the browser default.

## The other surfaces

### pm-console (127.0.0.1:4872, `screenshots/console-page.png`, `console-root.png`)

The Now page is the real queue-broker view (58 in progress / 5 queued / 316 blocked / 394 need you) and in several ways the more honest surface. Defects: it serves its full UI at loopback with no sign-in (the aggregator's pages at least sit behind the tailnet door in normal use); it is light-themed with a completely different visual language from the aggregator's dark UI — two apps, no shared design system, same product; cards leak raw worker identity strings ("any-occoddegnaahgonpnikmaidpdbemleeelopfjocc") and machine slugs as human-facing next steps ("NEXT STEP batch-a-13567-merged"); and its counts contradict the aggregator's without either page acknowledging the other (Finding 6). `/` 302s and `/console` 308s to `/console/` — fine, but three hops of redirects to reach a dashboard is its own small tax.

### Decision-set page (console-hosted, tailnet 443, `screenshots/decision-set.png`)

The individual decision-set page is genuinely good: questions are explained in plain words, consequences are stated, and answering is one tap. Defects: bulk actions ("Submit everything I've filled in", "Submit defaults", "Defer ALL remaining") are displayed on a **completed** set with 0 open questions — actions that cannot apply, offered anyway; and the links that lead here hard-code the tailnet hostname with a bearer token in the query string (Finding 13).

### pm-dispatch (127.0.0.1:4879) and pm-projector (127.0.0.1:7801)

Both answer `/` with a bare JSON 404 `{"error":"not found"}`. As API-only services that's defensible; as members of a stack a human is asked to trust, a one-line landing (name, role, health link) costs nothing. The projector matters beyond its size: it is the component the off-laptop design (#12831 ReconcileDO) mirrors, and nothing anywhere in the stack links to its state.

### Kaneo (127.0.0.1:5173, `screenshots/kaneo-root.png`)

`/` 200s and redirects to /dashboard — which rendered as a **black page** in a fresh profile: correct title, empty body, no spinner or message at 3.5 s. Caveat: this may be a slow first paint rather than a dead app; either way the loading state is a void, and Kaneo is a load-bearing source (600 notices on /health) whose own front door gives no sign of life.

### Pad (127.0.0.1:7777, `screenshots/pad-root.png`)

`/` → /login, as expected for a shared editor. No defect found at the door; it was not reviewed past the login wall in this pass.

### Grist (127.0.0.1:8484)

`/` → 302 to its login/home, as expected. Not reviewed past the door; its PM-stack role (roadmap source) is covered under /roadmap's drift finding.

### ready-for-agent (127.0.0.1:6056, `screenshots/rfa-root.png`)

The root is a settings page stuck on "Loading catalog… / Save is blocked until the catalog is ready" in a fresh profile — the same first-paint disease as Kaneo. Its operating state (from /health) is near-idle: 1 repository, merge policy OFF, 1 work item — the dispatch layer is provisioned but barely used while the fleet queue carries hundreds of tasks.

## The fleet feed

The feed itself is healthier than the stewardship outage suggested. At capture (2026-11-24T03:27–03:32Z) all 14 sections were present and ok (quota, attention, orchestration, dns, registry, projects, coverage, saas, payg, charges, chargesAttention [empty], subscriptions, snapshots, aa); the file was ~2 min old. The cycles 483/484 "fleet-feed unreachable" reports were the **sandbox-side reader** failing to reach the laptop over the tailnet hairpin — not the feed or the collector failing. That distinction matters and nothing in the stack records it: the same symptom (no fresh feed visible) has two completely different causes (collector dead vs channel dead) and the health surfaces can't tell them apart.

Real feed findings: the orchestration section runs an hourly cadence and sits 19–21 min old at capture (fine, but /health's "Behind by: unknown" for 13 of 14 sections means nobody can tell fine from stuck); the registry facet carries 28,479 session records of which 120 came from the latest feed — the facet is a growing archive being republished as a "feed"; and the aggregator README's provenance matrix still describes the collector as "(sandbox)" for Items/Quota/DNS although the collector moved to the laptop and the sandbox timer retired — stale documentation in the exact section a reader consults to understand the data path (fix noted for the next aggregator docs pass; the matrix's fallback column is also where Finding 3's fixture fallbacks are documented).

## Laptop-reliance map

Everything in the stack runs on the laptop today: aggregator, console, dispatch, projector, Kaneo (+Postgres), Pad, Grist and ready-for-agent are NixOS units/containers on `amd`, and the public names resolve through the laptop's tunnel (`amd-6-pm-stack`). **If the laptop is off, every page below is gone** — not degraded, gone — except the two data-plane pieces that already live hosted (the decision-events Cloudflare Worker and the Turso alerts/decision ledgers: the _data_ survives laptop-off; the _pages_ don't).

| Surface                    | Data path today                                                                                                                                 | Laptop's role                                      | State                                                                                                                                                                                                                                                                                                                                      |
| -------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| All aggregator pages       | aggregator process reads the fleet-feed file + live laptop services (Kaneo, Grist, projector, OpenCode server) + `~/.local/state/pm-aggregator` | hosts the process, the collector, and most sources | **Laptop-bound.** Hosted edition in flight: #9079 slices #12826 (hosted collector), #12829 (Workers dashboard surfaces), #12831 (ReconcileDO), #12833 (conformance) — designed/sliced, not the serving path. Move: finish #12829 so the dashboard reads Turso/D1 projections from Workers; the laptop becomes one collector among several. |
| Fleet feed                 | collector on the laptop writes the feed file the aggregator reads                                                                               | sole writer                                        | **In flight** (#12826 hosted collector). Move: multi-writer or hosted-writer with the laptop as fallback; a file on one machine is a single point of failure by construction.                                                                                                                                                              |
| pm-console (Now/queue)     | console process reads the queue broker                                                                                                          | hosts process + broker                             | **Laptop-bound.** The broker itself is the fleet's control plane (moving it is the fleet programme's question, not this stack's); the console _view_ could read the same hosted projections as #12829.                                                                                                                                     |
| Decision sets              | console-hosted pages; events in Turso via the hosted Worker                                                                                     | hosts the pages, not the data                      | **Half-migrated — the model to copy.** Data plane hosted already; serving still dies with the laptop. Move: serve the set pages from the same Worker that takes the events.                                                                                                                                                                |
| Alerts / notifications     | aggregator pages over the Turso alerts ledger                                                                                                   | hosts the pages                                    | **Data hosted, pages laptop-bound** (same move as decision sets).                                                                                                                                                                                                                                                                          |
| Kaneo / Pad / Grist        | own stores on the laptop (Postgres/files)                                                                                                       | hosts app + data                                   | **Laptop-bound.** Owned by #6816 (stores to Supabase + laptop-off drill) and #3142 (Supabase migration) — open, not started in earnest.                                                                                                                                                                                                    |
| Dispatch / projector / RFA | laptop services                                                                                                                                 | hosts                                              | **Laptop-bound.** Projector's hosted twin is #12831 (ReconcileDO). Dispatch/RFA have no hosted counterpart designed.                                                                                                                                                                                                                       |
| Quota gatherers            | gatherer processes on the laptop write the feed's quota section                                                                                 | hosts                                              | **Laptop-bound**; the hosted collector slice (#12826) is the move.                                                                                                                                                                                                                                                                         |
| Whole stack                | public names via the laptop's tunnel                                                                                                            | single egress                                      | **Laptop-bound.** #3989 (move stateful services off the laptop + prove host failover) is the owning epic and remains the umbrella for all of the above.                                                                                                                                                                                    |

Programme state, honestly: the direction is set and the first hosted pieces (decision events, alerts ledger) are real and live. But the serving path — every page a human opens — is 100% laptop today, and the slices that change that (#12826/#12829) are not yet the path anyone uses. Until one full page serves from Workers with the laptop off, "not reliant on the laptop" is a plan, not a property.

## What already works (worth saying)

- /dns is a model portfolio page; /comments has a model empty state.
- The decision-set page's plain-language question rendering is the best writing in the stack.
- /health's provenance and coverage sections are unusually honest about their sources — the failures there are in the summary math, not the disclosure instinct.
- The feed's section discipline (14 named sections, per-section ok flags) made this review's data-path tracing possible at all.

## Supplements in this record

- `screenshots/` — 37 full-page screenshots (every aggregator page + subroutes, console, decision-set page, Kaneo, Pad, RFA, 404).
- `supplements/page-texts/` — the full rendered text of every page as captured.
- `supplements/agg-health.json` — `/api/health` as captured 2026-11-24T03:32Z.
- `supplements/progress2.log` — the capture probe log (per-page status/size/timing).
