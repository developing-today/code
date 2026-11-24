# Project Management Space — Priorities and maps (structured data)

> Pack file 7 of 8 · Programme developing-today/code#13900 · Coordinator PM-BOOK
> Sources ONLY: `survey/roadmap.md`, `survey/website-pages.md`,
> `survey/apps.md`, `survey/push.md`, `survey/models.md`,
> `survey/llm-evidence.md`, `coverage-ledger.md`, `skeleton.md`.
> **No invented edges, no invented ranks.** Every list and every edge pair
> below names its source. Where a survey names a table but does not enumerate
> its rows, the gap is stated instead of filled.

---

## Part 1 — Prioritised lists

### L1. Programs P1–P9 with queue bands
Source: `survey/roadmap.md` §1/§3 (`pm-stack/design-reprioritization-2026-11-12.md`).
Re-banding recorded as APPLIED 2026-11-12 (queue lane) in that doc.

| Program | Name (as in the design doc) | Queue band |
|---|---|---:|
| P1 | ID | 20 |
| P2 | Atlas (rolling) | 20 |
| P3 | Fleet | 20 |
| P4 | SaaS for ID/Atlas/fleet | 30 |
| P5 | PM stack | 30 |
| P6 | Turso+Cloudflare | 30 |
| P7 | GitHub+dynamic compute | 40 |
| P8 | Storage | 40 |
| P9 | Lab | 40 |

### L2. Fleet queue priority bands (execution order)
Source: `survey/roadmap.md` §3. Tasks are claimed in priority-then-age order
(`ORDER BY (priority IS NULL), priority ASC, created_at ASC`).

```
bands (ascending = claimed first): [10, 20, 30, 40, 50, 70, 90]
band 10: owner prompts from the console (top band — promptPacket, console/src/work.ts)
band 20: programs P1–P3 (L1)
band 30: programs P4–P6 (L1)
band 40: programs P7–P9 (L1)
bands 50, 70, 90: in the band map; no program assignment stated in the survey
console scale: PRIORITY_SCALE = [10,20,30,40,50,70,90] (console/src/ops.ts)
```

### L3. Console issue rank labels (ranked-issues view order)
Source: `survey/roadmap.md` §3 (`console/src/ops.ts`: `RANKS`, `RANK_PRIORITY`).
`Ops.priorities()` sorts by rank, then issue number. `Ops.setRank()` adds the
new label first, then removes the others.

```
ranks (highest first): [expedite, priority/critical, now, priority/high, next]
```

### L4. Intake routes (console `#more/intake`)
Source: `survey/website-pages.md` §E51 + `survey/roadmap.md` §3/§4
(`console/src/plan.ts`: `ROUTES`, `INTAKE_LABEL = "owner/intake"`).

```
kinds: [note, bug, change]
each kind has a route: {labels, rank} — per-kind label/rank values live in
  plan.ts ROUTES; the survey names the mechanism, not each kind's values —
  do not invent them here.
on submit: files a GitHub issue (route labels + rank) AND enqueues a queue
  packet (triage.enqueue, or prompt.enqueue once an issue exists)
bugs view: #more/bugs lists Plan issues with kind: "bug" (plan.ts)
```

### L5. Notification delivery priority (breakthrough order)
Source: `survey/push.md` §2. Full class table: `push-flow.md` §2.

```
breakthrough (bypass quiet hours 23:00–07:00 America/Chicago + digest):
  - incident (always immediate; one escalation delivery)
  - quota-wall (sole-door case only)
  - section-risk (section FAILED only; SEVERE RISK is not breakthrough)
  - blocked-age (48-hour point only; 24h and weekly are digest-only)
digest-queued (one digest push; cancelled if the state disappears or the
  class is muted): all other passing non-breakthrough candidates
suppression order per candidate: class disabled → quiet hours (unless
  breakthrough) → dedupe (subscription, class, objectRef, fingerprint)
  → 6h cooldown
```

### L6. LLM pool/route ordering rules (priority of spend)
Source: `survey/llm-evidence.md` §2.3 (REG/EFF/GUIDE, dated there);
condensed with dates in `llm-plans.md` §2.

```
1. free-first: free legs lead every pool their capability fits (Drew, 2026-11-23)
2. burn-by-expiry: soonest-expiring pool first
3. cheapest sufficient first, on a quota read ≤60 minutes old (measurement gate)
4. burn each pool to 100%; at a known 100% the pool is CLOSED until reset/fresh read
5. NEVER FUNDS: a 402/quota error is a wall, never a PAYG fallback; ≤1 retry, then rotate or stop
   (dated exception: three TEMPORARY PAYG tails, Drew 2026-11-23 13:20 CST — llm-plans.md rows 25–27)
6. OpenAI is the final reserve — drained last
7. family preference, all else equal: claude → openai → gemini where the job fits
   (right-sizing beats family preference; the ladder wins on disagreement)
gates: Astra — only when Aster calls for it or a reason is logged;
  Antigravity Argon — sparingly (price doubles 2026-12-01 per EFF 2026-10-25);
  Cloudflare gateway — one session at a time, nothing important (and disabled fleet-wide)
```

### L7. Book-programme gap work lists (the programme's own prioritised build lists)
Source: `coverage-ledger.md` gap summary (settled 2026-11-23). Status of every
item: `coverage.md`.

```
PM book (12 workers): decision-events, voice-line, pm-notify (notify-relay),
  pm-console-edge, pkarr, lab-ops-hub, cf-container-hello, muse-agent-handoff,
  github-ci, exec door (security.cab), pm-stack aggregator worker,
  pm-stack dashboard worker
PM book (15 Turso DBs): fleet-platform, fleet-mirror, muse-meta,
  notes-submissions, bedside-and-away, voice-line-ledger, great-star-lord,
  + the frozen eight (one grouped PM chapter can cover all 8 — ledger note)
fleet book (3 workers + 1 DB): desktop-commander, fleet-hosted,
  queue-transport-alert; fleet-dispatch-jobs
id book (3 workers): mail (id-mail worker), id-worker-poc, id-d1-journal-poc
mcpx book (1 worker): mcpx (mcpx.security.cab door)
```

### L8. PM book chapter plan (build order as structured in the skeleton)
Source: `skeleton.md` (RECONCILED 2026-11-23). ~48 chapters + reference census
additions. Sections in order:

```
index → getting-started (installation, quickstart, shape)
→ concepts (architecture, aggregator, projector, shared-schema,
  roadmap-hierarchy, prioritisation, questions-and-answers,
  push-notifications, kaneo-sync, dispatch-methods, claimant,
  scheduled-jobs, health-surfaces)
→ reference (website-pages [29 routes], console-views [21 views],
  aggregator-routes, question-sets [F1–F8], workers, configuration,
  data-stores [50 DBs], models)
→ tutorials (answer-a-question, trace-a-push, follow-an-item,
  read-the-quota-view)
→ library (shared-schema-package, aggregator-internals, projector-internals)
→ opencode-openchamber (the liftable section — see oc-programme.md)
→ llm-plans (the-plans, pools-and-routes, models-by-tier,
  benchmarks-and-evidence, walls-and-resets, priorities)
→ development (layout, building-and-testing, add-a-page, program-roadmap,
  issue-map)
→ operations (services, health-and-healing, deploying-this-book,
  troubleshooting, faq, glossary)
deploy note (skeleton): book at pm-stack/book/ + pm-stack/book.toml; new
  flake output + build workflow mirroring fleet-docs.yml; joins the
  mcpx-docs.yml deploy job under subpath pm/ (siteUrl "/code/pm/") — the
  deploy-job edit is the high-blast-radius change.
```

### L9. Question sets F1–F8 (the answer-back catalogue, in survey order)
Source: `survey/website-pages.md` §F.

| ID | Set | Answer route (one line) |
|---|---|---|
| F1 | Decision sets (aggregator) | Page submit/defer/request-info → decisions store → ledger catcher; one push per set |
| F2 | Needs-Drew single questions (aggregator, #8568) | Notification actions or answer page → `POST /api/needs-drew/<ledgerId>/answer` → decisions machinery |
| F3 | Needs-Drew census sets (console `#needs`) | Reply (issue comment) / I did it (comment + label delete) via console actions |
| F4 | Prompt/chat questions (`/chat`, console `#ask`) | Prompt thread reply; console ask files an issue + top-band (10) triage packet |
| F5 | Dashboard comments / discuss | `POST /api/comments`; a fleet job applies/posts to GitHub and reports back |
| F6 | Dashboard actions (aggregator) | `POST /api/actions[/batch]` → orchestration pass → `POST /api/actions/result` |
| F7 | Console generic actions | `POST /console/api/act` audited action table; edge holds + replays when laptop unreachable |
| F8 | Reports, roadmap annotations, alerts | Resolve/late-update; annotation writes; ack/close + preference toggles |

### L10. Effort weights (progress accounting)
Source: `survey/roadmap.md` §2 (`aggregator/src/weights.ts`, #1108).

```
effort/xs = 1, effort/s = 4, effort/m = 12, effort/l = 40 (floor, open-ended)
unsized items: no weight, counted visibly as unsized (never zero-weighted)
```

---

## Part 2 — Dependency edges (pairs)

Edge types: `hierarchy` (native sub-issue parent), `dependency`
(`blockedBy`/`blocking`), `reads` (data consumption), `writes` (data
production), `sends` (notification/action delivery), `serves` (UI/API
surface), `guards` (invariant enforcement). **Every pair is stated in its
cited source; no pair is inferred.**

### E-H. Hierarchy & dependency semantics (the only work-item edge types)
Source: `survey/roadmap.md` §1.

| From | To | Type | Note |
|---|---|---|---|
| Epic (Project item, Kind=Epic) | Child task/item | `hierarchy` | Native GitHub sub-issue parent; resolved as `epicRef`; never inferred from labels |
| Work item | Work item | `dependency` | Native `blockedBy` / `blocking` relations (`orchestration-dependency.ts`, `epic-graph.ts`, `graph.ts`) — kept distinct from hierarchy; prose is a substitute for neither |

### E-D. Data-flow edges inside the PM stack

| From | To | Type | Source |
|---|---|---|---|
| Aggregator | Kaneo | `reads` (read-only) | `apps.md` §1: sign-in + workspace/project/task APIs; cards → board items |
| Aggregator | Grist Roadmap table | `reads` | `apps.md` §2: `gristRoadmap()` in `sources/live.ts` |
| Aggregator | Repo roadmap files (`planning/roadmaps/*.md`, `ROADMAP.md`) | `reads` | `roadmap.md` §2: `sources/repo-roadmap.ts` (#1738); repo file wins title collisions over Grist |
| Aggregator | Projector | `reads` | `apps.md` §7: `aggregator/src/sources/projector.ts` (links + latest comments per subject) |
| Aggregator | ready-for-agent | `reads` | `apps.md` §4: source adapter, `PM_AGG_RFA_URL` |
| Aggregator | Pad | `reads` (health/topology only — no planning data) | `apps.md` §3; D3 |
| Aggregator | GitHub Project #4 pool | `reads` (through the fleet feed) | `roadmap.md` §3: the aggregator reads the pool; it does not edit Project fields |
| Projector | GitHub | `reads` (webhook intake + reconcile polls) | `apps.md` §7: `POST /github/webhook` (HMAC), `src/reconcile.ts` |
| Projector | GitHub / Kaneo / Pad / Grist | — **explicit non-edge** | `apps.md` §7: the projector never writes back to any of them |
| Shared-schema orchestration writer | Turso `fleet-platform` | `writes` (one transaction per pass) | `apps.md` §25 (#5005/#3142) |
| Aggregator decision-events sync | Turso `fleet-platform` (`decision_set_events`) | `writes` | `push.md` §1/§4b: `scripts/sync-decision-events.ts` |
| Aggregator decision-events sync | decision-events Worker (D1 + per-set Durable Object) | `writes` / fan-out | `push.md` §4b; `apps.md` §10 |
| decision-events Worker | Decision-set page subscribers | `sends` (WebSocket live batches) | `apps.md` §10 |
| Voice-line Worker | Turso queue snapshot | `reads` | `models.md` §2: `buildAnswer(intent, snapshot)` — template answers, no LLM |
| Voice-line Worker | Workers AI (Whisper STT / MeloTTS) | `sends` (inference calls — the stack's only in-tree inference) | `models.md` §2: `env.AI.run(...)` |
| Quota-check helper | Turso `fleet-platform` / `fleet-mirror` | `reads` (per-DB tokens by name) | `apps.md` §23 (#6828) |
| Scheduled-history collector | OpenChamber schedule list + OpenCode session store | `reads` | `apps.md` §24 (#2174) |
| OpenCode usage export | OpenCode session store (`opencode.db`) | `reads` | `apps.md` §21 (#2072) |

### E-A. Action / answer-back edges

| From | To | Type | Source |
|---|---|---|---|
| Dashboard actions (`POST /api/actions`) | Orchestration pass | `sends` (consumed by polling) | `website-pages.md` §F6; `roadmap.md` §4 |
| Orchestration pass | GitHub Project fields | `writes` (**single writer**) | `roadmap.md` §3/§4; `pm-stack/README.md` rule |
| Orchestration pass | Dashboard (`POST /api/actions/result`) | `sends` (applied/rejected outcome) | `website-pages.md` §F6 |
| Console | GitHub labels + issue comments | `writes` (via `gh`, audit-logged first) | D4; `roadmap.md` §4 — the exception path; the aggregator service itself never writes labels |
| Console | Queue broker | `writes` (queue actions via `POST /console/api/queue/<task>/<action>`) | `roadmap.md` §4 |
| Console (`Work.submit()`) | Aggregator decisions API (`/api/decisions/<setId>/submit`) | `sends` | `push.md` §4c |
| Console edge Worker | Console host (laptop) | `sends` (held actions replayed through the same action table) | `website-pages.md` §E; `push.md` §4c |
| Decisions store resolutions | Muse-side decisions ledger | `writes` (applied per question by the dropped-thread catcher) | `website-pages.md` §F1; `push.md` §4a |
| Dashboard comments (`POST /api/comments`) | Fleet job → GitHub | `sends` (the dashboard itself never writes GitHub; a fleet job applies/posts and reports back via `POST /api/comments/github-result`) | `website-pages.md` §F5 |

### E-X. Execution edges

| From | To | Type | Source |
|---|---|---|---|
| pm-dispatch (`POST /v1/sessions`) | GitHub (tracking issue, label `source/dispatch`) | `writes` | `apps.md` §8 |
| pm-dispatch | Fleet queue (repo-task/v1 task) | `writes` (enqueue) | `apps.md` §8 |
| pm-dispatch | OpenCode server on laptop (`127.0.0.1:13420`) | `sends` (session creation; caller-supplied model hint) | `apps.md` §8; `models.md` §2 |
| pm-dispatch | Queue task for `dispatch-opencode` | `writes` (immediate claim, 6 h lease) | `apps.md` §8 |
| Claimant (`poll.py` candidate gate) | Four queues | `reads` | `apps.md` §19 |
| Claimant | OpenCode sessions | `sends` (launch+bind+prompt; supervision/recovery of stuck bound sessions) | `apps.md` §19 |
| Kaneo GitHub integration (developing.today GitHub App) | Kaneo task status | `writes` (push → in-progress; PR open/merge → in-review; issue close → done) | `apps.md` §1 (`KANEO-GITHUB-SYNC.md`, #7610) |
| Kaneo (task reaches Done) | GitHub issue | `writes` (`status:done` label / close — reverse sync) | `apps.md` §1 |

### E-P. Push edges
Source: `survey/push.md`. Diagram form: `push-flow.md` §1.

| From | To | Type | Source |
|---|---|---|---|
| External producers / health hooks | Aggregator alert throat (`POST /api/alerts`) | `sends` (bearer intake token) | `push.md` §1 |
| Aggregator alert/notification engine | Subscribed browsers/phones | `sends` (Web Push, VAPID, direct from laptop) | `push.md` §1 |
| Aggregator (`NotifyRelayClient`) | Notify relay | `sends` (60 s heartbeat + subscription mirror) — **edge not live: relay undeployed (D5)** | `push.md` §1 |
| Notify relay | Phones | `sends` (`stack.silent` / recovered pushes) — **not live (D5)** | `push.md` §1 |
| Phone service worker (single-question tap) | Aggregator (`POST /api/needs-drew/<ledgerId>/answer`) | `sends` (direct POST, no page open) | `push.md` §4a |
| Host-health collector / sentinel | Aggregator health assembly | `sends` (snapshot / incident POSTs) | `push.md` §5; `apps.md` §30 |

### E-G. Guard edges

| From | To | Type | Source |
|---|---|---|---|
| Kaneo sync guard | Kaneo GitHub integrations (all 4) | `guards` (asserts transitions; heals drift) | `apps.md` §1/§32 |
| Closure watchdog | Kaneo-covered repos' issue closes | `guards` (auto-reopens automation closes violating the closure invariant) | `apps.md` §29 (#10105) |
| Invariant sweeper | Four Kaneo-synced repos | `guards` (reports; `--act` repairs (a)/(b) only) | `apps.md` §31 (#10139) |
| Health hooks | Alert throat | `sends` (incident POST with `stackHealth` extension, on 3-failure transition + recovery) | `apps.md` §12 |
| Fleet-monitor `epic_close_check.py` | Aggregator epic closure gate | `reads` (report-only adapter) | `apps.md` §17 |

### Explicit non-edges (recorded so the Space build does not draw them)
- Aggregator → Pad planning data: **none** (health only) — `apps.md` §3, D3.
- Aggregator service → GitHub labels: **none** (console writes labels; orchestrator writes Project fields) — D4.
- Projector → GitHub/Kaneo/Pad/Grist writes: **none** — `apps.md` §7.
- PM aggregator/console/dispatch/projector → any LLM or embedding endpoint: **none** — `models.md` headline finding. (Atlas's embedding model — `Snowflake/snowflake-arctic-embed-m-v2.0`, 768 dims — belongs to Atlas; no PM component consumes Atlas embeddings in the code surveyed.)
- Notify relay → phone answers in production: **none today** — routes not routed, Worker not deployed (D5).
- Dashboard → GitHub writes: **none directly** — a fleet job applies dashboard comments; the orchestration pass applies dashboard actions (`website-pages.md` §F5–F6).

---

## Part 3 — Maps (what exists, where it is drawn)

| Map | Where the data lives | Survey pointer |
|---|---|---|
| Portfolio map | GitHub Project #4 ("Developing Today") — Status/Area/Horizon/Kind per item | `roadmap.md` §1 |
| Program map | Aggregator `/program` + `/program/<id>` (registry + tracked issues + Project pool) | `website-pages.md` §A18/B26 |
| Roadmap map | Aggregator `/roadmap` (Grist + repo files + pool, #1738 merge) and, separately, console `#more/roadmap` (plan-doc parse) — two implementations (D12) | `roadmap.md` §2 |
| Epic graph | `aggregator/src/epic-graph.ts`, `graph.ts`; rollups in `epics.ts` | `roadmap.md` §1–§2 |
| Conflicts map | Aggregator `/conflicts` — every live source disagreement with both values and age | `website-pages.md` §A9 |
| Forks dependency map | Book section `opencode-openchamber/dependency-map.md` (planned) — survey input pending; see `oc-programme.md` | `skeleton.md`; `oc-programme.md` (this pack) |
| PM issue map | Book chapter `development/issue-map.md` (planned) | `skeleton.md` |
