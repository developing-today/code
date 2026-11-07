# pm-aggregator-ui

The PM-stack **aggregator UI** — the dashboard Drew lives in while the
fleet works in GitHub, Kaneo, and Pad underneath. It is a **read-mostly
projection**: it never writes back to GitHub, Kaneo, Pad, or Grist
directly, and it is disposable by design — if it rots, every layer
underneath still works standalone (design record, "Data flow"). Slice 3
(#906) adds the app's first writes, and they are dashboard-local:
comments Drew and agents leave on items live in the aggregator's own
store (below), and GitHub-bound ones are posted by Muse side, never
by this server.

Design record (source of truth):
[`doc/2026-10-24T11-05-38Z_design_project-management-stack/`](../../doc/2026-10-24T11-05-38Z_design_project-management-stack/).

## The views

Nav tiers (#1497 curation, confirmed 2026-11-01): the top nav bar carries the
**primary** pages only (Overview, Today, Needs you, Roadmap, Usage & billing,
Program status, Health). Board, DNS, Cost Explorer, Sessions, and All pages are
**dropdown-only** — reachable from the Pages dropdown on every page. Projects is
**merged into Program status**: `/projects` redirects to the pool section of
`/program` (`#projects-pool`); its JSON twin is unchanged. All pages (`/pages`)
is a live index — last-rendered time + status per page from the render registry.

| Route             | View                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | Machine-first JSON                                                                                                             |
| ----------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `/today`          | **Today** (slice 2, #898) — the day's top objectives, prioritized, plus per-project momentum (moving / stalled / blocked-by-what, recent accomplishments). From the orchestration job's goal measurements (#899) when a section exists; derived from board + roadmap + item activity, and labelled as derived, until then.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | `/api/today.json`                                                                                                              |
| `/attention`      | **Needs you** (#852, tiered in slice 2) — **NEEDS USER** (default; the intake queue the orchestrator works first — only items it cannot resolve escalate here, each with its reason), **NEEDS ORCHESTRATOR** (the fleet's own queue), then every current session with source labels and plain-language states. Finished runs and stale items expire out.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | `/api/attention.json`, `/api/coverage.json`                                                                                    |
| `/`               | **Overview** (#1134) — the stack front door: what needs you, what is moving, what is broken, and where the quotas sit. Every number is the one its source page shows; the board, roadmap, projects, and health it links are the dedicated views.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | `/api/overview.json`                                                                                                           |
| `/board`          | **Cross-repo WIP board** (dropdown-only since #1497 — drill-down from Overview/Today) — the curated in-flight set across all four repos, with an execution strip per card (implementing → review → merged → queued).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | `/api/board.json`                                                                                                              |
| `/roadmap`        | **Roadmap** — Grist's `Program roadmap` table as a timeline; `?h=` switches the horizon: `quarter` (default), `month`, `week`, `3d`, `today` — one data model at every grain.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | `/api/roadmap.json`                                                                                                            |
| `/quota`          | **Usage & billing** (hub, design 2026-10-25; pools from #851) — needs-attention strip (bubble rules: >80% of a limit, >$5 projected unbilled PAYG), token-quota pools + PAYG dollars card, SaaS service cards grouped by kind with highlight metrics, charges evidence strip. Per-service detail: `/quota/service/<id>`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | `/api/quota.json` (extended: `saas`/`payg`/`charges`/…), `/api/quota/service.json?id=`                                         |
| `/aa`             | **Cost Explorer** (dropdown-only since #1497) — two tabs over the fleet-feed AA section. **AA Explorer**: the Artificial Analysis workbook measurements (rankings ⋈ Test_Runs) as three frontier charts (completion time × score, cost × score, cost × time, log axes, Pareto frontiers, effort ladders), with client-side filters (search, creator, family, benchmark, effort rungs, eligible-only) and three efficiency perspectives — **plan** (score/sec, dollars ignored), **API** (score/$, cost per task), **per-task** (fastest/cheapest finished task). **Model Explorer**: every model in the fleet config's provider catalogs with its pool seats, plan + effective value multiplier, and AA measurement, from a `catalog` section the collector builds out of the same `.opencode/opencode.jsonc` read as coverage. The AA tab also carries the reference design's companion views: tasks-completed vs duration (nominal-budget circles + published Go-cap diamonds), multiples-of-a-baseline chart + table (baseline selectable), a scores-only bar view (no-cost configurations included), per-chart dot-sizing by the third metric, PNG download, and a hide-a-configuration tray. Coverage lists (measured-not-pooled etc.) sit under the tabs. | `/api/aa.json` (extended: `catalog`)                                                                                           |
| `/dns`            | **DNS / domain register** (dropdown-only since #1497) (slice 3, #906; data owned by #902) — the register rendered next-renewal-first: renewing-soon spend, then in-use / unused-but-earmarked / completely-unused views and the full table.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | `/api/dns.json`                                                                                                                |
| `/sessions`       | **Sessions** (dropdown-only since #1497) (slice 3, #906) — the identity registry: every known session with its stable id, agent identity, parent, task refs, and token total, grouped by source.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | `/api/registry.json`                                                                                                           |
| `/program`        | **Program status — projects index** (#1046, generalized #1121) — one card per configured project (title, live Project-pool counts by status for its Area, Grist roadmap row count, home-doc link), the whole-pool summary, orchestration freshness, and the pool's unset-Area count as a visible gap, and — since the #1497 merge — the full Projects pool section (`#projects-pool`: raw pool cards, area filter, Project-vs-derived disagreements). Driven by the project registry in `src/project-config.ts`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | `/api/program.json` (incl. per-project summaries)                                                                              |
| `/projects`       | **Projects — merged into Program status** (#1497) — redirects (302) to `/program#projects-pool` (the `?area=` filter is preserved as `/program?area=`); the pool view itself is the Program section above.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | `/api/projects.json` (unchanged)                                                                                               |
| `/program/<id>`   | **Project page** (#1121) — one per registry entry (pm-dashboard, lab-cluster, id, repo-fleet, mcpx, cloud-program): intro + home-doc/dashboard links, live tracked-issue states, its Area's pool items grouped by status, its roadmap area's rows, and editorial done/next/long-term/gaps where configured. Null Area/roadmap/home-doc render as honest gap notes, never empty sections. The full PM-dashboard document (#1046) lives at `/program/pm-dashboard`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | `/api/program-project.json?id=<id>`                                                                                            |
| `/health`         | **Health & topology** (#1147 + #1137) — every stack service probed live (60 s probe cache): the aggregator, Kaneo, its Postgres, ready-for-agent, Pad, Grist, the fleet-feed collector, the orchestration pass, and the projector (off by config, stated not implied); the services & ports topology table (port, bind, runs-as, consumers, what breaks); feed section ages; last job runs; per-source facets; and this dashboard's own per-page render history (in-process registry — an unrendered page says so, never fake-green).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           | `/api/health` (extended in place: `services` / `feed` / `pages` / `jobs` beside the original `status` / `source` / `sections`) |
| `/pages`          | **All pages** (dropdown-only since #1497) — the page manifest as a live index: every registered page, its route, JSON twin, nav tier, and last-rendered time + status from the render registry (#1147).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | — (the manifest is `src/pages.ts`)                                                                                             |
| `/discuss/<ref>`  | **Comment thread** for one object ref (a `gh:` ref, an `orch:` tier item, a `session:` id, a board item, a roadmap subject) — dashboard comments + the post form.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | `/api/comments.json?object=<ref>`                                                                                              |
| `/item/<subject>` | Drill-down for one board card or roadmap row: linked refs + a unified comment stream, plus its dashboard comments and the sessions that worked it.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | —                                                                                                                              |

Pad is a standalone planning surface; the aggregator has no live Pad-to-item
association, so item pages do not show or imply Pad planning notes. Pad's
existing health probe and browser-safe service-home link remain registered in
`src/topology.ts` and `src/service-links.ts`.

All views are mobile-first (single-column stacking, scrollable tables,
touch-sized switchers) — verify at phone width, not by resizing a desktop
window. The canonical tailnet URL is
`http://amd-6.raptor-pumpkinseed.ts.net:4870` (MagicDNS); links never use
the raw tailnet IP (bind addresses and the feed-push transport still do —
that is plumbing, not links).

### Needs-you tiers + session clarity (slice 2, #898)

- **Tiering** (`buildAttention` in `src/derive.ts`): every classified
  session lands in NEEDS USER (with an explainable `drewReason`),
  NEEDS ORCHESTRATOR (with the fleet's next step), or in-motion. Drew
  rules, in order: the orchestration job's own classification (#899 —
  its tiers win for anything they name, reason verbatim) → a keeper
  "escalate" supervision note → a decision-shaped question → a
  destructive permission request → a credential/sign-in wall. Routine
  questions, errors, and tool permissions are the orchestrator's.
- **Expiry**: a finished run (the runtime reported an outcome) expires
  2 h after its last activity; anything expires at 24 h. The page counts
  what expired.
- **Sources**: every entry is stamped `opencode-sandbox` /
  `opencode-laptop` / `claude-code`; OpenCode sub-sessions
  carry `parentId` and collapse under their parent. **Muse** has no runtime registry to
  read; slice 3 (#906) wires it via launch records instead — a Muse
  session that never wrote one stays invisible, and the coverage note
  on the page says which (see "Session identity registry" above).

Each view is **machine-first**: the `/api/*.json` endpoint returns the
exact same data the GUI renders, so Muse can poll it in one call.

### Pages are pluggable (slice 3, #906)

Every page is registered once in `src/pages.ts` (`PAGES`): the nav
bar, the "Pages ▾" dropdown, and `/pages` all render from that
manifest, so a new page never touches the chrome. To add one:

1. Add a `PAGES` entry — `{ id, title, route, api?, blurb }`.
2. Write its `renderX` function in `src/render.ts`.
3. Add the route branch in `src/server.ts`, plus the `/api/*.json`
   twin when the page has data worth polling.
4. Wire any new data through `Source` (with its fixture twin), and
   through the fleet feed when the data originates on the sandbox.
5. Pass the mobile checklist (#1180) at **390x844**: compare the
   document's `scrollWidth` with its viewport `clientWidth` and attempt a
   horizontal scroll (no page-level horizontal movement; a 1px rounding
   residual is acceptable only when `scrollX` stays zero and no content
   escapes its wrapper). Keep intentionally wide tables and charts inside
   `.tblwrap` / `.chart`, and `pre` blocks scrolling inside themselves.
   Check actual rendered targets: nav links, chips, buttons, selects,
   form fields, disclosure summaries, and checkbox/radio labels are at
   least 40px tall (inline text links in prose and table cells are exempt).
   Follow an in-page link or call `scrollIntoView()` and confirm its target
   starts below the sticky nav; use enough `scroll-margin-top` for the
   wrapped mobile nav, not a desktop-only offset. Re-run the per-page audit
   when a page's layout changes materially.

`/dns` is the worked example: the register file lives on the sandbox
(#902 owns it), the collector parses it into a `dns` feed section
(`src/dns.ts`, keyed on the register's table header), and the page
renders its views from that section alone.

### Projects are configured, not coded (#1120)

Every project on `/program` is one entry in the registry in
`src/project-config.ts` (`PROJECT_CONFIGS`) — the single source the
index, the per-project pages, and both JSON twins read. To add a
project (target: under 30 minutes):

1. Add one `ProjectConfig` entry: `id` (the `/program/<id>` slug),
   `title`, `intro`, `poolArea` (its org Project #4 Area value, or
   `null` — the page then states honestly that it has no dedicated
   Area yet), `roadmapArea` (its Grist Program-roadmap area, or
   `null`), `homeDoc` (its `planning/roadmaps/<project>.md` home, as
   `{ repo, path, url }`), an optional `statusReport` (`{ label, url }`
   for the latest dated status report), `dashboardLinks` (dashboard
   routes or https source-doc URLs, with an optional `note` caveat),
   and `trackedRefs` (the issues whose live state the page resolves).
2. Optionally add `editorial` sections (`done` / `next` / `longTerm` /
   `gaps`, `ProgramEntry[]` — every entry should carry its tracker
   refs) and, for a program-level doc like pm-dashboard's,
   `architecture` entries.
3. Nothing else: the index card, the `/program/<id>` page, and the
   JSON twins all derive from the entry, and every countable thing
   (pool items by status, roadmap rows, issue states) is derived from
   the Source at request time — never written into the config.

Validation is fail-soft at render and strict in tests:
`validateProjectConfigs` (duplicate ids, duplicate pool/roadmap
areas, malformed slugs/refs, unusable home-doc / status-report /
dashboard-link URLs) is asserted clean over the shipped registry in
`test/program.test.ts`.

### Dashboard comments (slice 3, #906)

Comments live in a dashboard-side store (`src/comments.ts`, one JSON
file with atomic writes; path `PM_AGG_COMMENTS_FILE`) keyed by
**object ref**, so they survive republishes of the underlying data:
`gh:<owner/repo#N>` for a GitHub issue/PR, `orch:<tierItem.id>` for an
orchestration tier item, `session:<sessionId>` for a session, and
board item ids / roadmap subjects as-is. Every comment carries an
unambiguous author — `drew`, `muse` (the orchestrator), or an agent
identity from the registry — and the API refuses authorless comments.
Two kinds, visually distinct everywhere they render:

- **note** — plain context; read by whoever picks the item up next.
  Triggers nothing.
- **message** — a message to Claude: the orchestrator's queue input.
  Stays `triageState: new` until the orchestrator consumes it.
- **github** — a message that should also land on GitHub. Requires an
  explicit `githubRef` target chosen at post time (never guessed);
  waits in `githubState: pending` until a poster (Muse side, `gh`)
  posts it and reports back via `POST /api/comments/github-result`,
  which can also record a failure — a failed post stays visible.

**Orchestrator consumption contract** (also posted on #899/#906): each
pass polls `GET /api/comments.json?kind=message&state=new` (and
`?kind=github&state=new` for pending GitHub posts), triages each into
its normal flow — update the item, queue work, escalate to Drew via
the usual NEEDS USER path; github-kind ones get posted and reported —
then marks them consumed with `POST /api/comments/triage { ids }`.
Notes are never polled; they are read from
`/api/comments.json?object=<ref>` when an item is picked up. Item
pages merge the item's own thread with the threads of its linked
`gh:` refs.

### Dashboard actions — the triage write path (#1152/#1154/#1156)

Actions are the dashboard's second write surface: where comments are
conversation, actions are **requests a fleet job applies**. They live
in their own store (`src/actions.ts`, one JSON file with atomic
writes; path `PM_AGG_ACTIONS_FILE`) on the comments pattern, keyed by
the same object refs, and every action carries a validated author
(self-asserted on this loopback+tailnet-only surface — the trust
model #941 tracks hardening).

**How dashboard actions flow:** a form on a card posts
`POST /api/actions` → the record sits in the store at state `new`,
rendered on the card as a ⏳ badge → the consuming fleet job polls
`GET /api/actions.json?state=new`, applies the action as the single
writer of its surface, and reports the outcome via
`POST /api/actions/result { id, outcome, byRun, note }` → the card
then shows ✓ applied (or ✗ rejected, with the note). A result only
transitions out of `new` (consume-once), a reposted form returns its
existing record (per-form `clientToken` dedupe), and undo is a
superseding action — records are never deleted, so
`GET /api/actions.json` is the inspectable audit log.

Kinds and consumers (one consumer per kind/source slice):

- **triage** — the orchestration pass: `acknowledge` (seen; the next
  published tier entry carries the ack), `snooze` (the item leaves
  the Needs User tier until `payload.until`, and renders meanwhile in
  the page's visible "Snoozed by you" strip), `escalate` /
  `de-escalate` (tier override with reason), `answer` (text, a picked
  `optionId`, or survey `answers[]` — one content verb), plus the
  needs-user layer's `delegate` / `dismiss` / proposal verbs
  (#1290–#1293; `dismiss` requires a reason at the boundary).
- **status-request** — the orchestration pass again: `request-status`
  asks for a Project Status change (the card shows "change requested"
  until applied). The dashboard never writes the Project; the pass —
  its single writer, #910 — applies it and its run log names
  "requested via dashboard, action da-…".
- **session** — routed by the recorded `sessionSource`: `poke` /
  `answer` / `stop` on live sessions. OpenCode
  sessions are consumed by the orchestration pass, and sources with
  no live control channel (Muse, cron) render no enabled
  controls. Stop sits behind a two-step confirm.
- **preference** — the orchestration pass: `set-delegation-default`
  records the standing auto toggle (below). The aggregator also reads
  these records to compute the effective default.

Controls render only where they are honest: while the orchestration
tier source is not live, section-derived items on `/attention`
disable theirs with the reason shown (session-derived items stay
enabled — the session feed is live).

### Needs-user item contract (#1290–#1293)

Needs-user tier items may carry optional extension fields, published
by the orchestration pass, validated **per item** in the live source
(`src/tier-items.ts`) — the section gate never learns them, so a
malformed extension drops (the whole field for `delegation`, the
offending entry for `options`/`questions`), never the item or the
section. Naming is role-based; the legacy published tier array name
(`needsDrew`) is the existing shape and stays.

- `whyUser` — why the item needs the user: `decision` |
  `missing-info` | `approval` | `access` | `policy`. Fallback when
  absent: `decision` kind → `decision`; an item carrying a question →
  `missing-info`; otherwise `decision`.
- `summary`, `ifIgnored` — the plain-language item and what happens
  if it sits. `ifIgnored` falls back to a line computed from `since`.
- `options[]` (≤6) — `{ id (slug), label (≤80), description?,
effect? }`; duplicate ids drop the later entry. Picking one posts
  `answer` with `optionId`.
- `questions[]` (≤5) — `{ id (slug), prompt (≤200), type:
single|multi|text|yesno, options? (≥2, required for single/multi),
required? (default true), covers (mandatory) }`. Answering posts
  `answer` with `answers[]`; the POST boundary checks the ids against
  the currently published item and enforces required questions with a
  legible error.
- `delegation` — `{ mode: user|first-pass|auto, status:
idle|working|proposal-ready|unable|done, updatedAt, runId?,
proposal?, unable? }`, validated as a unit: `proposal` is present
  exactly when the status is `proposal-ready`, `unable` exactly when
  `unable` (with a non-empty `whatItTried`). An `auto` delegation on
  an `access`/`policy` item drops — those classes never auto.

**Delegation default (the standing toggle).** When an item carries
no `delegation`, the aggregator stamps the effective default: mode
`auto` when the standing toggle is ON and the item's class is
`decision`/`missing-info`/`approval`, else `user`. The toggle is a
dashboard preference action (kind `preference`, verb
`set-delegation-default`, object `needs-user-delegation-default`);
the latest non-superseded record wins, and before any exists Drew's
2026-10-27 decision — ON — is the initial record. A per-item
`delegate` action always overrides the default.

**Proposals (#1293).** One live proposal per item; its identity is
(item id, `delegation.runId`, `delegation.updatedAt`).
`approve-proposal` / `reject-proposal` (reason required) carry that
identity, and the boundary refuses a stale triple as
superseded/withdrawn. A proposal older than 48 h reads expired on
the card and can no longer be approved. A proposal-pending or
unable item is still one needs-user item — counts on `/attention`,
`/today`, and the JSON twins count it once.

### Session identity registry (slice 3, #906)

`src/registry.ts` normalizes every session source into one durable
registry (`PM_AGG_REGISTRY_FILE`; merges keep the max cumulative
token total so counts never go backwards):

- **OpenCode** (laptop live + sandbox via the feed): session id,
  parent id, agent identity (`opencode:<agent>`, `opencode (sub)` for
  unnamed sub-sessions), status, per-session token totals.
- **Muse**: launch records appended at spawn to a JSONL the
  collector carries — `scripts/record-claude-launch.ts` writes them,
  `CLAUDE_LAUNCH_RECORDS_PATH` locates the file; a record is
  `{ id, label, parentId?, taskRefs?, model?, status? }`.

Task↔session links are loose v1: a record's `taskRefs` come from its
PR, an issue URL in its title, or its launch record; item pages list
the sessions that worked them. The OpenCode tool in
`opencode-tools/session-identity.ts` lets a session report its own
`{ sessionId, parentId, agent, directory }` on demand — copy it to
`~/.config/opencode/tool/` on each host and restart that server.

### Alerts — the alert throat (#3676) + PWA/Web Push (#1254 slices a+b)

Every sentinel posts incidents to ONE intake instead of inventing its
own alert path; one engine owns dedupe, delivery, escalation, and
ack; every transition lands in a durable ledger. Drew's amendments to
the #3450 design: delivery is Web Push (the #1254 design, doc
`doc/2026-11-01T03-54-00Z_design_pwa_push_notifications.md`), and the
ledger lives in **Supabase muse-dev2**, schema `alerts` (`incidents`
with a partial unique index on `(source, incident_key)` for
non-closed rows, `incident_events`, `push_subscriptions`,
`notification_sends`) — reached through the same Management-API SQL
path as the fleet-feed collector's AA read (`src/supabase.ts`).

- **PWA origin**: the aggregator is served through Tailscale Serve —
  `https://amd-6.raptor-pumpkinseed.ts.net` (443) → `127.0.0.1:4870`.
  That HTTPS origin is the only secure context here, so it is the
  PWA origin: `PM_AGG_PUBLIC_ORIGIN` (default exactly that) builds
  push deep links. `/manifest.webmanifest` + icons (incl. maskable +
  apple-touch) make it installable; iOS Web Push needs the installed
  PWA (16.4+).
- **Service worker**: `/sw.js` is the REAL worker now (push +
  notificationclick). Per design §6's fallback (no probe data
  existed), the eviction job for the stranded opencode worker is
  carried forward: the worker purges origin caches on activate, and
  the page-side probe script (`/assets/push-client.js`, loaded by
  every page) unregisters foreign registrations and reports what it
  sees to `/api/sw-probe` (GET returns the running summary).
- **Subscriptions**: `POST/GET/DELETE /api/push/subscriptions` over
  `alerts.push_subscriptions` — per-class map + quiet prefs
  (defaults: `incident` on; quiet 23:00–07:00 America/Chicago).
  A 404/410 from a push service bumps `gone_count`; two consecutive
  gones prune the row (design §4).
- **Intake**: `POST /api/alerts` with `Authorization: Bearer <token>`
  — the token is the Bitwarden secret `pm-aggregator.
alerts-intake-token`, resolved by name at runtime
  (`src/secrets.ts`, the laptop bws flow). Sources post via
  `bun run scripts/post-alert.ts --source … --key … --severity … --title …`.
  Dedupe rides the partial unique index: a repeat post bumps
  `occurrences`/`last_seen_at` (and writes a `duplicate_suppressed`
  event only when a delivery would otherwise repeat).
- **Engine**: a ~60s tick in the server (live mode only). A new open
  critical pushes immediately to subscriptions with the `incident`
  class on (the breakthrough class — sentinels are external, so
  quiet hours never hold it), recording `delivered_at` + one
  `notification_sends` row per attempt. An unacked critical older
  than `ALERT_ACK_WINDOW_MIN` (default 15) escalates with exactly
  one more push (`escalated_at`). Warnings never push in v1.
- **Measurement**: `/notifications` (#3319) is the read-only weekly
  sent-vs-acted-on roll-up over `notification_sends`: per class and
  rolling week, only `outcome = sent` rows count as sends, and a send
  counts as acted-on when its `acted_on_at` (stamped by Ack) falls
  within 24 hours. A class under 10% acted-on over the rolling
  four-week window renders as a default-off _proposal_ — display
  only; the page never changes a preference or a default.
- **Ack surface**: `/alerts` lists open incidents with Ack/Close.
  Ack is exactly-once: only the open→acked transition writes an
  event (and stamps the sends' `acted_on_at`); a repeat ack returns
  the current state. Push payloads deep-link to
  `/alerts#incident-<id>`.
- **VAPID**: the public key ships in `src/push.ts`; the private key
  is the Bitwarden secret `pm-aggregator.vapid-private-key`, fetched
  by name at runtime, memory only. Subject
  `mailto:drewrypope@gmail.com`.

## Ingest contract (#2576)

Every document the aggregator ingests is validated **once, at its
ingest boundary**, before any consumer, store append, or render sees
it. Validation covers shape, element types, value ranges, and real
calendar dates, and it is written with the shared primitives in
`src/validate.ts` (`isRecord`, `isNonEmptyString`, `isStringArray`,
`isFiniteNumber`, `isNonNegative`, `isPercentOf(limit)`,
`isIsoDate`/`parseIsoDate`, `arrayOf(guard)`,
`isTimestampNotFuture`) — a guard never re-implements a check those
primitives already express.

A validation failure produces exactly one of three sanctioned
outcomes, chosen per boundary and recorded in its guard:

- **Reject and retain** — the document/section/entry is refused; the
  previously retained value stays and ages.
- **Drop the element, count it** — where elements are independent,
  the malformed element alone is dropped and the drop is counted
  somewhere visible, never silently and never at the cost of its
  siblings.
- **Refuse the append** — a snapshot record that fails validation is
  not appended, and the append reports the refusal. A record the
  store cannot read back is a record it must not accept.

Three outcomes are forbidden everywhere: throwing across the boundary
and discarding valid siblings; fabricating rows, ids, or zeros from
malformed input; and publishing an out-of-range value as live
measured data. A genuine zero is a successful validation whose value
is 0.

The contract's existing embodiment is `src/token-adapter.ts` (#1153):
it validates its document once at the boundary and rejects-and-retains
on failure, and new boundaries are written to its standard.

## First-class requirements (from the design record)

1. **Cross-repo links.** Any board card or roadmap row links any GitHub
   issue/PR from any of the four repos (`developing-today/{code,opencode,openchamber}`,
   `dezren39/nix`), served live from the projector's link registry. Kaneo
   binds one repo per project and cannot do this; the registry keys are
   repo-qualified (`owner/repo#N`).
2. **Unified comment stream.** One activity stream per item covering its
   linked issues/PRs across all repos, PR-interior comments included
   (reviews and line comments), from the projector's comment projection.

Both render in the board cards (execution strip) and the item drill-down.

## Run it

```bash
cd pm-stack/aggregator
bun install
bun run dev          # http://127.0.0.1:4870
```

Tests (unit derivations + render + a boot smoke test that serves every
view from fixtures):

```bash
bun test
```

## Configuration (env, secrets by NAME only)

Everything defaults to **fixtures** and works with no configuration,
except the default Source itself: serving fixtures without an explicit
`PM_AGG_SOURCE` requires the dev-mode switch `PM_AGG_DEV=1` (#1195), so
a deployment that forgets `PM_AGG_SOURCE=live` fails at startup instead
of silently showing canned data. `bun run dev` sets the switch for you.

| Env var                    | Default                                                | Purpose                                                                                                                                                                                                |
| -------------------------- | ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `PM_AGG_SOURCE`            | `fixtures` (dev-gated, see below)                      | `live` reads Kaneo/Grist/projector, falling back to fixtures per facet; explicit `fixtures` stays available for deliberate fixture work; unset/invalid without `PM_AGG_DEV=1` refuses to start (#1195) |
| `PM_AGG_DEV`               | —                                                      | Set to `1` for local development: allows the implicit (unset or invalid `PM_AGG_SOURCE`) fixture default; never set in deployment — `install.sh` renders `PM_AGG_SOURCE=live` instead                  |
| `PM_AGG_PORT`              | `4870`                                                 | HTTP port                                                                                                                                                                                              |
| `PM_AGG_HOSTS`             | `127.0.0.1`                                            | Comma-separated bind addresses (deployment: loopback + tailnet; no public route)                                                                                                                       |
| `PM_AGG_FEED_FILE`         | `./fleet-feed.json`                                    | The fleet feed (quota + sandbox attention) written by the collector (below)                                                                                                                            |
| `PM_AGG_SNAPSHOTS_DIR`     | `./snapshots`                                          | Usage snapshot store (hub §4): monthly `usage-YYYY-MM.jsonl` files; read by the hub, appended by the background feed-ingest tick, writer jobs + the one-shot backfill                                  |
| `PM_AGG_QUOTA_VIEW`        | stewardship goal's `visibility/quota-view.md`          | Hub fallback source when the feed carries no `saas`/`payg` sections yet                                                                                                                                |
| `PM_AGG_QUOTA_REGISTRY`    | repo-health goal's `provider-quota-registry.md`        | The hub's second dated-read source (billing incidents) for the same fallback                                                                                                                           |
| `PM_AGG_OPENCHAMBER_STATE` | `~/.config/openchamber/managed-opencode/state.json`    | Laptop OpenCode server port + password source (laptop attention, live)                                                                                                                                 |
| `KANEO_URL`                | `http://127.0.0.1:5173`                                | Kaneo base URL (board reader)                                                                                                                                                                          |
| `KANEO_ADMIN_EMAIL`        | —                                                      | Kaneo admin email (secret `kaneo.admin-password` pairs with it)                                                                                                                                        |
| `KANEO_ADMIN_PASSWORD`     | —                                                      | Kaneo admin password (Bitwarden `kaneo.admin-password`)                                                                                                                                                |
| `GRIST_URL`                | `http://127.0.0.1:8484`                                | Grist base URL                                                                                                                                                                                         |
| `GRIST_API_KEY`            | —                                                      | Grist API key (Bitwarden `grist.api-key`)                                                                                                                                                              |
| `GRIST_ROADMAP_DOC`        | `czYHYKgG9SHcm9CPwiKEq1`                               | The `Program roadmap` document id (Phase 2 seed)                                                                                                                                                       |
| `PM_PROJECTOR_URL`         | `http://127.0.0.1:7801`                                | Projector + link registry base URL (`/items`, `/comments`, `/links`); `off` disables it                                                                                                                |
| `PM_AGG_COMMENTS_FILE`     | `./comments.json`                                      | Dashboard comment store (#906) — aggregator-local; created on first comment                                                                                                                            |
| `PM_AGG_ACTIONS_FILE`      | `./actions.json`                                       | Dashboard action store (#1152/#1154/#1156) — aggregator-local; created on first action                                                                                                                 |
| `PM_AGG_REGISTRY_FILE`     | `./registry.json`                                      | Durable session identity registry (#906) — merged records persist across restarts                                                                                                                      |
| `PM_AGG_ACTIVITY_FILE`     | `~/.local/state/pm-aggregator/activity-ingestion.json` | Activity-ingestion store (#3321) — per-source checkpoints + dedupe seen sets + gap records for the watchdog/Kaneo stream writers                                                                       |

Secrets are read from the environment only and are never committed or
printed. Names here reference Bitwarden entries (`kaneo.*`, `grist.*`).

### Activity ingestion into the event stream (#3321)

Watchdog breach episodes and Kaneo board changes are ingested into the
#1739 event stream by `src/activity.ts`, with persisted per-source
checkpoints: replaying a duplicate delivery is a no-op, and a restart
resumes from the checkpoint instead of re-emitting. Known source
outages are recorded as gap records (source + time range) and rendered
on `/events` as holes in the history — never as complete coverage; a
backfill/recovery pass flips a gap's status. Per-source coverage
(supported / unsupported / historical ranges) is documented in
`ACTIVITY_SOURCE_COVERAGE` in `src/activity.ts`, rendered on `/events`,
and served at `/api/activity.json` together with the checkpoints and
gaps. GitHub activity is explicitly **unsupported** until the #2072
webhook/hot-list ingestion lands — #3321's boundary forbids a
competing receiver.

## Capability provenance matrix (#1200)

The single source of truth for what the aggregator reads, facet by
facet. **Live source** is what the facet reads in the deployed
configuration (`PM_AGG_SOURCE=live`, rendered by `install.sh`;
`PM_PROJECTOR_URL` pinned by the NixOS module). **Deployment** states
what the stack definition declares — configuration, not observed
runtime; the one observed check on record is dated in the notes under
the table. Everything else in this README and in `program.ts` must
agree with this table; `scripts/check-provenance-matrix.ts` verifies
the load-bearing claims against the code and configuration (see
"Keeping the matrix honest" below).

| Facet                        | Live source (provenance)                                                                                                                                                | Fallback                                                   | Deployment                                                                                                                                                                                                                    |
| ---------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Board                        | Kaneo REST (`/api/task/tasks/<project>`, all four projects), enriched with projector registry links                                                                     | fixtures                                                   | Kaneo container (compose, `pm-stack/`)                                                                                                                                                                                        |
| Roadmap                      | Grist REST (`/api/docs/<doc>/tables/Roadmap/records`), enriched with projector registry links                                                                           | fixtures (the real Grist seed rows)                        | Grist container (compose, `pm-stack/`)                                                                                                                                                                                        |
| Registry links               | Projector `GET /links?subject=…`, unioned into board cards and roadmap rows                                                                                             | Kaneo external links / text refs                           | projector (below)                                                                                                                                                                                                             |
| Items (refs)                 | Fleet feed's GitHub Project pool FIRST (#910 stage 5); refs outside the pool resolve via projector `GET /items?refs=…`                                                  | fixtures, per ref                                          | feed collector (sandbox) + projector                                                                                                                                                                                          |
| Comments (GitHub projection) | Projector `GET /comments?refs=…`                                                                                                                                        | fixtures                                                   | projector (below)                                                                                                                                                                                                             |
| Dashboard comments           | Aggregator-local store (`PM_AGG_COMMENTS_FILE`, default `./comments.json`) — written by the dashboard itself, never projected                                           | none (local file)                                          | aggregator unit                                                                                                                                                                                                               |
| Quota (#851)                 | Fleet feed file (`fleet-feed.json`), overlaid by the quota gatherer's `quota-current.json`                                                                              | fixtures when no feed exists                               | collector (sandbox) + gatherer timer                                                                                                                                                                                          |
| Attention (#852)             | Laptop OpenCode server live (state file, 45s cache) + sandbox via fleet feed                                                                                            | fixtures when neither is reachable                         | OpenCode servers (both hosts)                                                                                                                                                                                                 |
| DNS (#906)                   | Fleet feed `dns` section, parsed collector-side from the #902 register                                                                                                  | fixtures when no section exists                            | collector (sandbox)                                                                                                                                                                                                           |
| Registry (#906)              | Feed registry records (sandbox OpenCode, Muse launches) merged with live laptop OpenCode records into the durable registry file                                         | fixtures when the feed is absent                           | aggregator unit + collector                                                                                                                                                                                                   |
| **Projector** (service)      | GitHub projection + link registry at `PM_PROJECTOR_URL` (default `http://127.0.0.1:7801`); 15-minute reconcile timer keeps it fresh (no public webhook route by design) | facets fall back per the rows above when it is unreachable | **Deployed**: `pm-projector` user unit declared in `nixos/services/pm-stack`; the aggregator unit `Wants`/`After` it and pins `PM_PROJECTOR_URL` to it                                                                        |
| **Pad** (service)            | — (the aggregator reads no Pad data; Pad is not a facet source)                                                                                                         | —                                                          | **Deployed as the planning layer only**: NixOS OCI container `pm-pad`, declared by `services.pm-apps` (`nixos/services/pm-apps/default.nix`); named here so its absence from the facet rows is a stated fact, not an omission |

Observed runtime (checked 2026-11-02 by task 6104 via the stack's status
surfaces): at `06:04:56.726Z`, the aggregator briefly reported the
projector `down` (1.5s probe timeout) and Comments `fixtures`, while Pad
was `up` (HTTP 200). At `06:07:46.665Z`, its `/api/health` recheck showed
Pad and projector `up` (HTTP 200), Comments `live (projector)`, and
`degraded=false`. Direct checks during the same pass found `pm-pad`
running/healthy with `/api/v1/health` version `0.18.1`; projector `/health`
returned 8,189 items and 41,961 comments, and a direct `/comments` read
returned a projected issue comment. This later read supersedes the brief
timeout and the earlier task 4902 successful read (8,134 items, 41,169
comments). These are point-in-time observations, not standing health
claims. The deployment cells above describe declared configuration; they
do not imply that a service is currently healthy or that its live data is
reaching the aggregator.

### Keeping the matrix honest

```bash
bun run scripts/check-provenance-matrix.ts
```

The check derives the load-bearing facts from the configuration and
code — the `PM_PROJECTOR_URL` pin and the `pm-projector` unit in
`nixos/services/pm-stack/default.nix`, `PM_AGG_SOURCE=live` in
`install.sh`, the reader wiring in `src/sources/live.ts` (pool-first
items, projector comments) — and fails if this matrix, the pm-stack
README pointer, or `/program` (`src/program.ts`) contradicts them.
Run it after changing any facet's source, the projector's deployment,
or the matrix itself.

The projector's wire schema is its `schema.sql` (Session J, PR #859):
snake_case fields, `state` is only `open|closed` with `merged` as a
separate 0/1 flag, `labels`/`assignees` are JSON array strings, and
comment rows carry `created_at` / `review_state`. The mapping to this
app's read model lives in `src/sources/live.ts`
(`mapProjectorItem` / `mapProjectorComment` / `parseProjectorLabels`),
covered by `test/projector.test.ts` — including a stub-server test that
refs survive the query string (`#` is percent-encoded, otherwise it
would start a URL fragment and the projector would never see the
number). Registry subjects are `kaneo:<projectSlug>/<taskId>` and
`grist:<docId>/<tableId>/<rowId>`; board cards and roadmap rows are
enriched with `GET /links?subject=…` so cross-repo links (the
requirement Kaneo cannot provide) render live.

The footer of every page states, facet by facet, whether it is live or
fixture-backed. When a live reader is unreachable, that facet alone
falls back to fixtures and the footer says why.

## The fleet feed (quota + sandbox attention, slice 1 — #890)

The sandbox OpenCode server binds its own loopback and the quota
sources of record live on the sandbox, so those two facets travel as
a file. `scripts/collect-fleet-feed.ts` runs on the sandbox:

- **Quota** — per-pool extractors (`src/quota-extract.ts`) over the
  stewardship quota view's dated reads. Fail-soft by design: a pool
  whose read pattern stops matching renders with its identity and a
  "no current read" note — never a vanished pool, never an invented
  number. Live per-pool burn (active sessions + their token totals)
  is attributed from the sandbox server at collection time, and
  model-choice guidance (`src/quota-guidance.ts`) is computed then.
  The pool map is the corrected one from the provider-quota
  registry: exactly one Meta pool (Muse API High Usage, `meta`
  provider the only door); no "muse plan" pool exists.
- **Usage & billing hub sections** — the same collector run extracts
  quota-view Part (b) via `src/saas-extract.ts` into `saas` / `payg` /
  `charges` / `subscriptions` feed sections (the provider registry's
  billing incidents supply the gateway accrual + card-alert charges).
  The aggregator's `Source.usage()` serves them with snapshot history
  from the §4 store; older feeds without the sections fall back to a
  direct read of the two source documents. A five-minute background
  writer tick lands feed snapshots into the local store; `Source.usage()`
  and its GET routes are read-only. Seed historical observations once
  with `bun run scripts/backfill-usage-snapshots.ts` (idempotent one-shot
  recovery, not normal history startup).
- **Snapshot writers (hub §4)** — history accumulates three ways, all
  riding the feed: the collector derives records from its own hub
  sections (`src/snapshot-records.ts`) and embeds them in the feed's
  `snapshots` field; stewardship-cycle (after a quota-view refresh)
  and the keeper's check-usage append records to
  `usage-snapshot-outbox.jsonl` beside the keeper state — via
  `bun run scripts/append-usage-snapshots.ts` (add
  `--from-provider-usage` for the keeper snapshot) or, for
  check-usage, from Python directly — and the collector drains the
  outbox into the same field. The server's background feed-ingest tick
  copies feed records into the local store; dedupe on (service, metric,
  timestamp, value) makes every writer safe to re-run.

## The quota gatherer (Stage 2 — #1151)

`scripts/gather-quota.ts` is the quota plane's 15-minute tick. It
runs on the **laptop** (nothing executes on the sandbox), as the
`quota-gatherer` systemd **user** timer (units in this package's
`systemd/`, installed by `install.sh`; linger keeps it
boot-persistent). The unit runs the script from the laptop's repo
clone (`~/git/code`) and writes its outputs under
`~/.local/state/pm-aggregator`, outside the checkout.

Each tick:

- runs the OpenDesign session keeper (Drew's provider browser-auth
  auto-renewal rule): the one browser-session credential in the
  system is read, slid, or re-captured over CDP before it is used,
  with its horizon recorded in `opendesign-renewal.json`. The keeper
  maintains a credential; it is not a quota source.
- invokes the ONE quota source - the opencode fork's quota
  subcommand, `opencode quota --json` (#6125/#6127) - whose
  per-provider quota functions live in the providers that own the
  credentials. A second invocation asks explicitly for the
  route-disabled live pools (`meta`, `command-code`), which a named
  ask always fetches. The gatherer fetches no provider itself and
  imports no quota module; a provider failing degrades only its own
  rows, and #1268 retention keeps last-known rows for errored runs.
- writes **`quota-current.json`** - the latest-state contract
  (`QuotaCurrentFile` in `src/model.ts`: per provider/credential,
  windows carrying value, % of limit, reset, measured-at, source,
  state; the file's `source` is `opencode-quota`) - and
  `quota-gatherer-heartbeat.json` (tick time, per-provider
  state/duration; the heartbeat also rides inside
  quota-current.json), both in the aggregator state directory beside
  `fleet-feed.json`.
- appends S4 snapshot records for windows whose value **moved**
  since the last appended record, directly to the snapshot store
  (`SnapshotStore.append` dedupes; change-tracking state in
  `quota-gatherer-state.json`). Setting `USAGE_SNAPSHOT_OUTBOX_PATH`
  reroutes records to the writers' outbox instead.
- appends one Supabase `agent.quota_records` row per pool entry to
  muse-dev2 (source `opencode-quota`), so the readings land in
  Supabase on the same tick as the dashboard file. Fail-soft: a sink
  failure is logged and never fails the tick.

Fail-soft: a subcommand failure or unparseable output writes a
heartbeat with the error, exits non-zero, and leaves the previous
`quota-current.json` untouched. **Remaining hop:** the aggregator
does not read `quota-current.json` yet — `Source` pickup (freshness
rendering, lazy pull on page load) is the follow-up that makes the
tick visible on /quota.

- **Sandbox attention** — sessions mapped by the shared OpenCode v2
  mapper (`src/sources/opencode.ts`), stamped `opencode-sandbox` with
  parent/agent/outcome.
- **Orchestration section** (slice 2, #899 — the unified contract) —
  the orchestration job republishes
  `~/workspace/goals/keep-the-four-repos-healthy/hidden_files/orchestration.json`
  on the sandbox every run: an `OrchestrationSection` v1 (types in
  `src/model.ts`) with tiered queue items (`tiers.needsDrew` /
  `tiers.needsOrchestrator`, each with its reason), a flat dependency
  view, per-goal measurements (momentum, tasks lined up by horizon,
  what progressed), and the actions the run pushed back into motion.
  The collector embeds it as `feed.orchestration` after a structural
  check (fail-soft; override the path with `ORCHESTRATION_PATH`). The
  Before publishing, run the publisher's pre-publish gate from this
  checkout:

  ```sh
  bun run contract:check -- --publisher /path/to/orchestration.json
  ```

  `scripts/check-contract-corpus.ts` runs the aggregator's actual
  validators over the file exactly as the collector gates it, so a
  #1110-class drift fails at the source, not as missing sections
  downstream. The
  aggregator serves it raw at `/api/orchestration.json`; the needs-you
  view lets its classification win for anything it names (non-session
  items render as tier cards), and the today view maps its goals —
  until a section exists, the today view derives and labels itself.
  The shared publisher/collector/aggregator contract corpus (#1176)
  lives in `test/contract-corpus/v1/` (scrubbed captures of real
  published orchestration, quota, DNS, registry, and projects sections;
  synthetic producer-adapter inputs are in `publisher/`), exercised by
  `test/contract-corpus.test.ts` and `test/contract-collector.test.ts`.

- **DNS register** (slice 3, #906) — the register Markdown (#902's
  file; override with `DNS_REGISTER_PATH`) parsed by `src/dns.ts`
  into a `dns` section: entries with renewal dates, list prices, and
  an active/earmarked/unused status read from the register's own
  Plan cell. Absent file or unparseable table → no section (the page
  says why).
- **Registry records** (slice 3, #906) — normalized session records
  (`src/registry.ts`) for sandbox OpenCode sessions (with per-session
  token totals read off the server at collection time) and
  Muse launch records (JSONL at
  `CLAUDE_LAUNCH_RECORDS_PATH`, written by
  `scripts/record-claude-launch.ts`). The aggregator merges them with
  live laptop records into its durable registry.

The collector runs on the laptop: the `fleet-feed-collector` user
timer (declared in the pm-stack NixOS module, #3363/#3368) runs
`scripts/collect-fleet-feed.ts` every ~5 minutes and writes the feed
straight to `~/.local/state/pm-aggregator`. The aggregator states the
feed's age in the page provenance; a stale feed keeps rendering.
The collector originally ran on the sandbox and pushed the feed
over the tailnet proxy route (`scripts/push-fleet-feed.sh`); that
sandbox timer is retired (#3362) — a stale sandbox copy kept
overwriting the laptop feed (its AA section 403'd against the
retired muse-dev project). The push script remains for manual use
only.

**Deployment (laptop):** run `install.sh` from the `~/git/code`
checkout. The service runs directly from
`~/git/code/pm-stack/aggregator`; `install.sh` renders the mode-600
environment file and runtime paths under `~/.local/state/pm-aggregator`
from `~/secrets/pm-stack.sops.yaml` (keys by name), installs the
repository-owned systemd units, and health-checks loopback + tailnet.
The legacy `~/pm-stack/aggregator` symlink to that state directory is a
compatibility bridge for sandbox feed pushes and must remain until the
sandbox has pulled the updated push script and delivery is verified.
Slice 2 (per #890) replaces the feed with projector-ized live
per-source feeds.

## What is fixture-backed vs live

Per the capability provenance matrix above: in the deployed
configuration nothing is fixture-backed by default — every facet
reads its live source, and each falls back to fixtures independently
when its source is unreachable (the fixtures are shaped like the
live readers' output and sit behind the same `Source` interface,
`src/source.ts`). Two facets deserve the explicit sentence the
matrix's brevity drops:

- **Items** resolve pool-first: the fleet feed's GitHub Project pool
  is the primary source (#910 stage 5), the projector serves refs
  outside the pool, and fixtures are the last resort per ref.
- **Comments** (the GitHub projection) read live from the projector,
  which is a deployed service of this stack — see the matrix's
  Projector row for the deployment evidence and the dated observed
  check. Roadmap fixtures are the real Grist seed rows; the design
  record's known refs (#752, #754, #762, #763, nix #501–#508,
  opencode #365) are real; the rest are illustrative.

The live board is the curated in-flight set (in-progress / in-review
/ recent done columns; the to-do backlog is excluded). Kaneo project
slugs map to repos as in `pm-stack/kaneo-projects.json` (Session H):
`code`, `opencode`, `openchamber` under `developing-today/`, `nix`
under `dezren39/`.

## Honesty + freshness (#1139, #1267, #1138)

- **No silent zeros (#1139).** The orchestration section is
  diagnosed on every read — live / stale (>90 min) / absent /
  invalid (with the failing check named) — via
  `diagnoseOrchestrationSection` in `src/feed.ts` and
  `Source.diagnostics()`. `/attention` banners an absent, invalid, or
  stale tier source at the point of display and renders the tier
  counts as "unknown" rather than a confident 0 when the source is
  missing; `/today`'s derived-view banner names the section's true
  state. The stale/absent banner is also the aggregator-side
  dead-man's switch for #1148: if the orchestration job dies, the
  surfaces Drew reads say so. Visible tier wording is role-based —
  NEEDS USER, not NEEDS DREW (Drew's 2026-10-27 direction; internal
  field names and the orchestration contract are unchanged). The
  NEEDS USER queue is an intake the orchestrator works first; only
  items it cannot resolve escalate to the user.
- **Health primes (#1267).** Provenance strings populate lazily, one
  facet per method call, so `/api/health` exercises every facet
  (60 s-cached) before answering — its report no longer depends on
  which pages were visited since boot. The response also carries the
  structured `sections` diagnosis beside the provenance strings.
- **Freshness chips (#1138).** Pages render per-source chips
  (source · state · age, counts always with their noun) for exactly
  the sources they use, from `Source.provenanceDetail()`; the full
  provenance line moves to a collapsed "source detail" disclosure in
  the footer. The registry chip counts the merged store total — the
  same number the /sessions header renders.

## Design notes — the two new views (#851, #852)

Both were added as **design issues** (2026-10-25) and are implemented
here as machine-first surfaces with a GUI over the same data.

- **#851 quota + throughput:** the data model is `QuotaPool` /
  `QuotaWindow` / `BurnSegment` / `QuotaView` (`src/model.ts`); wall
  detection and near-limit thresholds live in `src/derive.ts`
  (`poolState`, `NEAR_PCT = 85`). Slice 1 (#890) reads quotas from
  the fleet feed (above) rather than the projector; per-source live
  readers (projector-ized) are slice 2. No estimator duplicates the
  OpenChamber quota picker; picker values are logged, not
  re-estimated (standing rule 2026-10-24).
- **#852 needs-you:** the state taxonomy and classification rules are
  `AttentionState` + `classifyAttention`/`attentionSorted` in
  `src/derive.ts`. Ordering is stopped-for-question and errored first,
  then needs-poke / stuck-retrying / stalled-on-CLI, then in-motion;
  every non-in-motion entry carries its evidence inline. Slice 1
  maps the OpenCode v2 servers directly (`src/sources/opencode.ts`);
  a projector-ized session feed is slice 2.

Neither view vendors anything from the CCC reference design
(`claude-command-center`, FSL-1.1-MIT) — only its idea of a
machine-first data surface with a GUI on top is used.

## Layout

```
src/
  model.ts      read model (refs, board, roadmap, quota, attention,
                dns, comments, registry)
  source.ts     the one Source interface
  pages.ts      the page manifest — nav, dropdown, /pages (#906)
  program.ts    the /program page's editorial record + live-data
                builder (tracked issues, pool summary, freshness) (#1046)
  project-config.ts  the project registry: one ProjectConfig per
                project + validation + the per-project live builders
                the index, project pages, and JSON twins read (#1120)
  derive.ts     execution strip, timeline lanes, quota/attention rules
  topology.ts   the services & ports registry: what runs where, bound
                how, consumed by whom (#1137)
  health.ts     /health assembly: cached service probes, feed ages,
                job runs, the render registry (#1147)
  render.ts     pure HTML rendering of every view
  server.ts     Bun HTTP server (pages, JSON twins, comment writes)
  config.ts     env config
  feed.ts       fleet-feed file format + loader
  dns.ts        the #902 register parser (#906)
  comments.ts   the dashboard comment store (#906)
  registry.ts   session records + identity normalization (#906)
  quota-extract.ts  per-pool extractors over the quota view's dated reads
  quota-guidance.ts headroom / burn-before-reset / do-not-start derivation
  sources/
    fixtures.ts   fixture data (real Grist seed + design-record refs)
    live.ts       Kaneo/Grist/projector readers with per-facet fallback
    opencode.ts   OpenCode v2 server -> SessionFeedEntry mapper (shared)
opencode-tools/
  session-identity.ts  OpenCode tool: a session reports its own id (#906)
scripts/
  collect-fleet-feed.ts  collector: quota + attention + dns + registry
                         records -> feed (runs on the laptop, ~5 min timer)
  push-fleet-feed.sh     RETIRED sandbox producer: collect + push the feed
                         to the laptop (manual use only, #3362)
  record-claude-launch.ts append a Muse launch record to the JSONL (#906)
test/
  render.test.ts    unit derivations + render smoke
  boot.test.ts      boots the server, serves all views from fixtures
  projector.test.ts projector wire-schema mapping + stub-server reads
  opencode.test.ts  mapper against a stub OpenCode v2 server
  quota.test.ts     extractors + guidance against the dated-read format
  slice2.test.ts    tiers, expiry, hierarchy, roadmap
                    grains, today derivation, slice-2 render smoke
  slice3.test.ts    dns parser, registry normalization, comment store,
                    page manifest, slice-3 renders + HTTP flows
  program.test.ts   /program manifest entry, live-data builder
                    (empty/absent/null inputs), render smoke (#1046)
  health.test.ts    health assembly vs fake fetchers (up/down/timeout/
                    cache/degraded), fixtures story, /health boot
                    (#1137, #1147)
```

## Store contract: define semantics (#2953)

Define semantics for externally-spelled keys (#2953). A value keyed by a
name a caller, document, feed, file, peer or user can spell is stored with
define semantics, never assignment semantics: the store is built in a Map
or a null-prototype object and converted at the boundary with
Object.fromEntries / spread, or the entry is written with
Object.defineProperty. The non-conforming forms for external keys are
obj[key] = v, obj[key] ??= ..., dst[k] = src[k] copy loops, and
Object.assign(dst, src) with an externally-keyed src — all go through
[[Set]], and for the one spelling "**proto**" [[Set]] invokes the
Object.prototype setter instead of creating an own property, silently
dropping the entry or moving the container's prototype. Keys are data:
preserve, don't reject. Code-constructed tables whose keys are all
internal are exempt. Probe matrix for every covered site — keys
"**proto**" (string value), "**proto**" (object value), "constructor",
"toString", and an ordinary key: the entry exists as an own property with
its value, a JSON round-trip preserves it, and neither the container's
prototype nor Object.prototype moves or gains own properties.
