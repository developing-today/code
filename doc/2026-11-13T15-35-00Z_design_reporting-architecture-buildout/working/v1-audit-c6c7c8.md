# V1 audit — what exists TODAY for C6 / C7 / C8 (baseline, 2026-11-13)

Read-only audit of branch `design/reporting-architecture-buildout` (repo
`developing-today/code` checkout at `/home/user/git/fleet/16121-reporting-architecture-buildout`).
No live changes except this file. Method: repo grep + read, `gh issue view`
for #1184/#9060/#9091/#9106/#9180/#9178/#8983/#9568/#9574/#9575/#9576, plus the
intake docs under `doc/2026-11-13T15-12-00Z_design_reporting-architecture-event-driven/`
(note.md, sources/audit-side-chats.md, sources/source-brief.md, sources/statement-inventory.md).

Env note: repo-root `scratch` was a **broken tracked symlink**
(`scratch -> doc/hardware/scratch`, whose own parent `doc/hardware -> ../../hardware-doc`
also does not resolve in this worktree). Replaced the dangling symlink with a real
`scratch/` directory to hold this deliverable. No other tree changes.

Intake anchor: Drew's directive, Reports side chat 2026-11-13 09:11 CST, compressed in
`sources/source-brief.md`. Starter epic #9568 (OPEN) has 8 sub-issues = C1–C8 (#9569–#9576),
all OPEN. C6=#9574, C7=#9575, C8=#9576.

---

## 1. Spaces / reports today (C6 substrate)

### 1a. Fleet Reports Space — EXISTS (external to this repo)

- **Exists?** YES as a live surface. 17 streams, per intake audit
  (`sources/audit-side-chats.md:27,37`, `note.md:138`).
- **Write path:** rebuilt jobs write full reports to the Space via `recordReport`;
  only exception notices go to the Reports chat (`audit-side-chats.md:20-21,27-30`).
  JSONL fallback exists ("recordReport + fallback JSONL", `note.md:138`).
- **Paths:** UNKNOWN in-repo. Zero hits for `recordReport` outside the intake docs
  themselves; no stream definitions, no JSONL paths, no Space SDK code in this checkout.
  Implementation lives outside this repo (hatch-personal workspace / OpenChamber Spaces infra).
- **Stream names:** NOT enumerated anywhere in-repo. The audit names 12 rebuilt jobs whose
  outputs land in the Space (usage-quota-report, pool-quota-steward, pool-intelligence,
  pool-composition-review, pool-meta-reviewer, fleet-watchdog, git-hygiene-sweep,
  stewardship-cycle, go-plan-expiry-watch, coding-plan-watch-daily, hatch-untracked-review,
  daily-token-usage-checkin — `audit-side-chats.md:167-176`), which accounts for ~12 of the
  claimed 17 streams. The other ~5 stream names are UNKNOWN.
- **Freshness behavior:** refresh-on-each-job-run (full report per run), not continuous.
  No per-stream freshness SLA found in-repo; C6's contract ("stale beyond class budget =
  defect with exception path") is new, not current practice.
- **Known defect (job-run status vs pool status display):** recorded ONLY as a one-liner
  (`note.md:139-140`: "the display defect (job-run status vs pool status) is a C6 input").
  No elaboration, no repro, no owning issue found in-repo. Detail is UNKNOWN — presumably
  lives in chat/job lore.
- **Owner:** 6 space-owner chats (below); no in-repo owning code.

### 1b. Space-owner chats (6) + refresh jobs — EXIST (chats external; jobs external)

- Chats (ids from `audit-side-chats.md:180-183`): "Space owner: Fleet Reports" `fcf52a31`,
  "Projects Overview" `3d1d2f4a`, "AI Benchmarks" `d30275f6`,
  "OpenCode & Muse Activity" `785419b6`, "AI Plans & Usage" `f2178de6`,
  "ID & Atlas" `f707171a`. Verdict in audit: KEEP all six as freshness owners.
- Refresh jobs named: id-atlas-refresh, fleet-live-status-refresh,
  fleet-roadmap-stats-refresh (`audit-side-chats.md:182-183,228-232`).
- **Paths/schedules:** NOT in this repo (no hits in `doc/fleet-schedule-registry.md`,
  no scripts). One external sighting noted in-repo:
  `workspace/cron.d/hourly/id-atlas-refresh__interval@6h.md` under hatch-personal
  (cited in `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/inventory-live/candidate-provenance.md:11`
  — a generic live-source refresh instruction, 6h interval). Job bodies, exact schedules,
  and what each refresh writes are UNKNOWN from this checkout.

### 1c. Space staleness evidence

- Directive itself (verbatim compression, `source-brief.md:53-57`): "no space should be
  **6 days or 5 hours stale** for information that needs to be current" — i.e. 6-day and
  5-hour staleness have both been OBSERVED (two classes of surface).
- #1184 body (Drew's 2026-10-26 review): "the data is stale or only partially adopted";
  /attention showed "Needs Drew (0)" vs 11 NEEDS-DREW items in orchestration.json;
  footer falsely claimed the orchestrator "has not published" (it publishes hourly;
  the aggregator rejects it — #1110).
- C6 scope (`#9574` body): "Known wound: spaces have been found days stale."
- Per-space dated stale examples (which space, which date, how stale): NOT found in-repo.
  UNKNOWN — likely in chat transcripts / job logs only.

### 1d. PM-stack pages relevant to quota/queues — EXIST (partial, see §2)

Covered in §2. Short version: /attention, /quota (+/quota/service/:id), /today, /queue,
/board, /activity, decisions page, health page, quota-actions derivation — all in
`pm-stack/aggregator/src/` (server.ts, render.ts, derive.ts, decisions.ts, today-cache.ts,
quota-*.ts, health.ts, freshness*.ts, snapshots.ts).

---

## 2. PM-stack today (C6 consumers + #1184 verdict)

### 2a. Coherence epic #1184 — OPEN, AT the 100-sub-issue cap

- State OPEN. Sub-issues: **exactly 100 total (81 CLOSED, 19 open)** — verified via
  `gh issue view 1184 --json subIssues` (`nodes` count). The intake claim
  ("#1184 is at GitHub's 100-sub-issue cap → cross-linked, not parented", `note.md:120`)
  is CONFIRMED empirically.
- Children include the four architecture buckets from the body: front-door/nav
  (#1134–#1138), freshness/honesty (#1110, #1139–#1146), health/observability
  (#1147, #1148, #1150), triage (#1152, #1154, #1156, #1158), queue visibility +
  hierarchy + quotas + contracts/perf (…#1183), plus later additions (#1195–#1201…,
  #1252–#1285, #3315…, #3933, #3973–3975).
- Yardstick verdict per dimension (Drew's requirements, #1184 body):
  - **Single-URL?** NO. Aggregator pages + Kaneo + Grist + Pad + ready-for-agent remain
    separate surfaces (#1134 front-door, #1136 deep-links, #1182 one-URL-behind-Access —
    #1182 OPEN per register; #1134 state unchecked, likely open).
  - **Fresh data?** NO (the headline wound). #1110 orchestration-section rejection,
    #1139 silent-0 honesty, #1140 absent-vs-invalid, #1141 pool coverage, #1144
    Grist↔Project 1:1. #1147 (health page) and #1148 (staleness watchdog) are CLOSED —
    mechanism exists, data honesty still the gap.
  - **Triage-able?** PARTIAL. #1152 (ack/snooze/escalate/answer) CLOSED; needsDrew
    contract #1290 + action UI #1291 + delegation #1292/#1293 CLOSED (verified in slice 5
    per register); decisions page #8766 CLOSED; answerable pushes #8568 Phase 1 CLOSED
    (#8569 Phase 2 OPEN, #8570 Phase 3 OPEN). #1154/#1156 state not individually verified.
  - **Hierarchical?** PARTIAL. Native sub-issue chains + epic/design label families exist;
    current lineage in `pm-stack/lineage-manifest-2026-11-12.md`; the multi-board
    hierarchy program is #9060 (below).

### 2b. Questions module + decision machinery — EXISTS, running, three render surfaces

- **Ledger:** `~/workspace/goals/pending-decisions-hourly-questions-for-drew/hidden_files/pending-questions.json`
  (external path). Snapshot 2026-11-11: 150 items (84 answered / 34 asked / 20 pending /
  12 dropped). Doc: `pm-stack/decisions-ledger-system.md`; register P2.1
  (`design-register-2026-11-11.md:750-765`): shipped 2026-11-04 (PRs #8767, #8770, #8784).
- **Ask flow:** hourly pending-questions job → ledger → Outstanding Questions side chat
  (mirror/working surface) → decisions page (P2.2, #8766: batched sets, one push per set,
  per-item/partial submit, defer-to-orchestrator). Ask/answer mechanics in
  `needs-user-actions-design-2026-10-26.md` (§1.4 survey `questions[]` ≤5, options ≤6,
  delegation modes; `/attention` card extension + `.aform`).
- **Decision-set-activity-processor:** routes to category #4 in the target map
  (`audit-side-chats.md:77,226-227`). In-repo: `decision-events-worker/` (Cloudflare half
  of the decision-set event pipeline, #9335 Track B), `aggregator/src/decisions.ts`,
  `decision-events.ts`, `decision-events-sync.ts`, `scripts/raise-decision-set.ts`,
  `/api/decisions/:set/submit|defer|request-info` + WS `/events` (server.ts ~L1075-1230).
- **Confirmation-bubble actions:** DO NOT EXIST for quota/reset flows. The only
  confirmation pattern found in-repo is the session-stop two-step ("tick Confirm stop,
  then Stop", `render.ts:346,1425`). No confirmable OpenAI/Anthropic/Fable reset, no
  cloud-reset rule wiring. Quota actions that DO exist are read-only derivations:
  `buildQuotaActions` (`derive.ts:1055-1130`, #1174) — expiring periods (14d window),
  walled pools, high-usage (>85% with reset >24h out) — surfaced on /quota strip, /today,
  /attention orchestrator tier. Entries self-expire; no action buttons, no queueing.
- **Owner/freshness:** owning goal `pending-decisions-hourly-questions-for-drew`; hourly
  job; counts above are a 2026-11-11 snapshot, live count UNKNOWN from this checkout.

### 2c. Mobile-first — CLAIMED, not proven

- Evidence FOR: aggregator README "All views are mobile-first (single-column stacking…)"
  (`pm-stack/aggregator/README.md:82`); needs-user-actions design "single column,
  mobile-first… action controls ≥44px touch targets" (`needs-user-actions-design:197`);
  playwright mobile viewport 360×740 (`aggregator/playwright.config.ts:29`);
  `<meta viewport>` in `opencode-claimant/index.html:5`; #1135 (nav overflow on mobile)
  is CLOSED.
- Evidence AGAINST / gaps: no viewport/CSS/layout audit found in-repo; desktop-first
  artifacts remain (`opencode-claimant` max-width:1200px desktop container);
  "mobile nav" work was a fix for overflow, not a mobile-first redesign proof.
- Verdict: treat as **NOT mobile-first until proven** — matches the task default.
  C6's hard requirement is new work, and no mobile test matrix / device evidence exists
  in-repo.

---

## 3. Rollups today (C7) — almost entirely ABSENT

Searched: `pm-stack/**`, `doc/living/`, intake docs, aggregator routes/views.
The C7 scope (`#9575` body) describes the target; essentially none of it exists:

| C7 item | Exists? | Path / evidence or where absence was checked |
|---|---|---|
| Daily pages (midnight–midnight CT prose) | NO | No daily-page route, template, or generator in `pm-stack/aggregator/src/`; no `daily-*` docs outside dated design notes; grep for "midnight" in pm-stack + doc/living: zero relevant hits |
| Weekly pages (Mon–Mon CT) | NO | Same; `weekly-token-usage-review` is a quota cron, not an activity page |
| Monthly / quarterly pages | NO | Same; monthly usage files are raw JSONL (see snapshots row), not prose rollups |
| "What did we do today?" prose (per-project, pivots, remediations) | NO | `/today` page EXISTS (`renderTodayBody`, `today-cache.ts`, `server.ts:2489`) but is a live current-day view (today + quota + coverage + health + queue), not a frozen dated record; no pivot/remediation prose |
| Soft edit windows + restricted post-window edit forms | NO | No edit-window code, no quote-block/append/banner/event-log-injection forms anywhere in aggregator |
| Period snapshot library spaces (dated, frozen, unpinned) | NO | `snapshots.ts`/`snapshot-records.ts` are the **usage-metrics** JSONL store (`usage-YYYY-MM.jsonl`, 13-mo retention → `.rollup.jsonl` compaction) — unrelated to period page snapshots; no page-snapshot mechanism |
| Per-project spaces (Fleet / PM stack / Atlas / ID) | NO in-repo | Spaces are external (OpenChamber); only owner-chat pointers in-repo (`audit-side-chats.md:180-190`); no space definitions, no exists/wanted/in-progress/designed content in this checkout |
| Per-design pages + status ladders (idea→…→fully-designed+issues) | NO as pages | Data substrate exists: `design-register-2026-11-11.md` (register entries w/ states), `lineage-manifest-2026-11-12.md`, `project-tracking-evaluation-2026-10-23.md`; no rendered per-design pages or ladders |
| Implementation rollups (done/in-progress/blocked/queued → roadmap/epic) | NO as rollups | Board shows 95/107 cards Done per #1184 (a "completion log", not a rollup); `epic-graph.ts`, `spend-rollups.ts`, `flow-analytics.ts` are adjacent analytics, not design→epic→roadmap rollups |
| Reports timeline page (quarter→month→week→day) | NO | No timeline route/view; `activity.ts`, `assembly-deadline.ts`, `roadmap-horizon.ts` are not it |
| Sitemap / tree + full search over reports | NO | No sitemap; search exists only as `search-view.ts` (stack search) and Atlas (separate corpus); no all-reports index page (the "question-sets list" is cited as shape precedent only, S3.17/C7) |
| Stored PM-stack records as permanent report records | PARTIAL | Usage snapshots JSONL + comments/registry stores exist and are append-only; but report *pages* are not stored records |

Net: C7 is greenfield except (a) the `/today` live view, (b) usage-metrics JSONL history,
(c) design-register/lineage data that rollup pages could render from.

---

## 4. Atlas today (C8 substrate)

### 4a. #9091 Atlas v1 — OPEN, 0 sub-issues (workstreams live elsewhere)

- Starter #9091 OPEN; sub-issue count 0 because workstreams A–I (#9092–#9101) are native
  sub-issues of program epic #8983 (which has 79 sub-issues), linked from the proto design §J.
- Proto design: `doc/2026-11-12T23-59-00Z_design_atlas_v1_proto/note.md` (merged to main).
  V2–V4 direction: `doc/2026-11-11T03-45-00Z_design_atlas_v2_v4/` (capability roadmap with
  stage gates; explicitly "not… proof of running software").

| C8-relevant V1 fact (proto §0, verified 2026-11-12) | Status |
|---|---|
| Ingestion cadence | Manual/script-driven ONLY; the sole Atlas schedule on the laptop is the daily backup. No always-on ingestion; aggregator/embedder + session-close hook unbuilt (#7969, #6018) |
| Corpus | Canonical publish 2026-10-31: 18,325 sessions / 122,124 msgs / 107,639 chunks; Turso monolith since grown to 7.3 GB **without verified recount**; 29 topic shards partition the same chunk set; 638 secret-bearing chunks in prod monolith (#8479, remediation undecided) |
| Recency / dataset bias | NONE. Planner/spell/query-class flags accepted-but-unimplemented; hosted search caches **indefinitely, no TTL/eviction/invalidation** (#6805, #8312) |
| Metadata links (page→item) | NONE. VFS #9100 unbuilt; §H location-index future. Fleet Reports Space, ask-ledger logs, quota records, laptop state explicitly NOT in corpus (§E sources-inventory is a deliverable) |
| Cross-linked embeddings (1–2-step skips) | NONE. §G query-as-data + result archive unbuilt |
| Page metadata schema | DESIGNED, not implemented: §C doctrine (version-always, additive-only, extras, strict enums, optionality) + amendments binding #9073/#9074 (comments 6839361945/6839362599). V2–V4 doc notes staging schema/code exist but aggregator/embedder, merge-on-close, sensitivity stores are outside implementation |
| Query-planner boosts | NONE. Depends on #5773/#5774 query-understanding; §A "kill the dead contract or implement it" is future work |
| Error-driven investigation recipe | NONE. C8's recipe (first-appearance → time range → issues/PRs/commits → queue attempts/models/worktrees → unpushed-commit check → session transcripts → rhyming class fixes) is entirely new; no part exists as tooling |

### 4b. #9060 hierarchy/genealogy + children — epic CLOSED, 12 sub-issues all OPEN

- #9060 CLOSED (design-phase epic), 12 sub-issues, 0 closed:
  9065, 9066, 9067, 9068, 9069, **9070** (genealogy read model — ancestors, spawned-by,
  sentence renderer, API), 9071, **9073** (fleet gh Metadata superset — create-path
  stamping + DB insert), **9074** (metadata DB + fleet Metadata query surface),
  **9075** (session state — session id in env + worktree pwd-file), 9076, 9062.
- Relevance to C7/C8 (per `note.md:126-127`): C7 design pages render the #9070 genealogy;
  metadata comes from #9073/#9074; session joins from #9075. All three are OPEN —
  C7/C8 build on unshipped foundations. Consume-don't-remodel boundary stands.

### 4c. #9106 federation triple/link constraint — CLOSED program, 6 children open

- #9106 CLOSED, 6 sub-issues (9107–9112), all OPEN. Part A = ID core (namespaces, ID
  shapes, **triples**, user attributes) — "the addressing and data model everything else
  stands on" (#9106 body §A).
- The C8-relevant constraint (`note.md:131-133`): "C8's metadata-link model should stay
  compatible with its triple/link model." The exact triple/link schema text was NOT
  located in this checkout beyond the program statement + proto scaffold dir
  (`doc/2026-11-13T00-41-00Z_design_id-atlas-primitives-federation-namespaces/`);
  identity-amendment inputs live on #9132/#9121/#9127 (not read in this pass).
  Treat the constraint as: real, but detail lives in #9106's children / Part A design.

### 4d. #1184's 100-sub-issue cap — VERIFIED

- `gh issue view 1184 --json subIssues` → `nodes` length **exactly 100** (81 CLOSED).
  GitHub rejects a 101st sub-issue, so the program rule (cross-link, don't parent;
  #9060 intermediate-grouping precedent, `note.md:118-122`) is load-bearing, not stylistic.

---

## 5. FDR convention (#9180) — ABSENT as a convention; seeds unrecorded

- `doc/decisions/` does **NOT exist** (confirmed `ls`; the FDR home is uncreated).
- #9180 OPEN, 0 sub-issues, 2 comments (a Kaneo link + a #9221 cross-ref note).
  Acceptance criteria (index + FDR-0001…FDR-0010 with status/owner/links) — none met.
- Seed records FDR-0001…FDR-0010: **zero files** (`find -iname 'FDR-0*'` → only the
  unrelated `FDR-PMO-001…008` set under
  `doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/decisions/` — a different
  program's convention, not the fleet convention #9180 specifies).
- Historical `*_decision_*.md` under `doc/`: ~25 scattered files (ID/Atlas decisions),
  unindexed for fleet purposes — exactly the gap #9180 names.
- Companion #9178 (queue semantics) is OPEN. Implication for this program: the C-program's
  own rulings (chat map, reset rule, edit windows) currently have **no home** — first
  FDRs will also be the convention's first use.

---

## 6. Collision-state table (consume / extend — never duplicate)

| Program / surface | State | This program's posture (per intake `note.md:116-143`) |
|---|---|---|
| #1184 PM-stack coherence | OPEN, 100/100 subs (81 closed) | C6 builds on its pages; cross-link, never parent |
| #9196 orchestration | (not audited in this pass) | C2/C5 consume dispatch/lifecycle; no second dispatcher |
| #9060 hierarchy/genealogy | CLOSED epic, 12 open children (#9070/#9073/#9074/#9075) | C7 renders #9070; metadata/session joins consumed as-is |
| #9091 Atlas v1 | OPEN starter, workstreams under #8983 (79 subs) | C8 is an input to its E/G threads, not a parallel search program |
| #9106 federation/namespaces | CLOSED program, 6 open children | C8 metadata-link model stays triple/link-compatible; identity material already routed to #9132/#9121/#9127 |
| #9178 queue semantics | OPEN | C2 assumes its claim surface |
| #9180 fleet decision records | OPEN, no records yet | This program's rulings recorded as early FDRs |
| Fleet Reports Space (17 streams) | Live, external impl | C6 builds on it; display defect is C6 input |
| page-health-push (5-min, main-chat urgency) | Live per audit | Precedent for high-urgency path; keep |
| Questions/decision machinery (ledger, decisions page, decision-events-worker) | Shipped + running | C6 generalizes; no second questions system |
| 6 space-owner chats + 3 refresh jobs | Live, external | C6 makes them freshness owners |
| Quota actions (`buildQuotaActions`) | Live derivation, read-only | C5/C6 extend toward confirmable actions; reset machinery rides pool steward's existing audibles |

---

## 7. Explicit UNKNOWNs (need a session with hatch-personal/OpenChamber access)

1. Fleet Reports Space: 17 stream names (~5 unaccounted), `recordReport` implementation path,
   JSONL fallback paths/retention, per-stream freshness observed values.
2. Display defect (job-run vs pool status): full description, repro, owner.
3. Refresh jobs (id-atlas-refresh, fleet-live-status-refresh, fleet-roadmap-stats-refresh):
   bodies, schedules beyond the one 6h sighting, write targets.
4. Dated per-space staleness instances (which space, when, how stale, caught by what).
5. #9106 Part A triple/link schema detail (beyond the program statement).
6. #9132/#9121/#9127 amendment contents (not read; identity material excluded from C6–C8 by intake).
7. #9178 acceptance state of FDR-0001…0004 (blocks #9178 completion; orthogonal to C6–C8 start).
8. Live counts: pending-questions ledger now (snapshot was 2026-11-11), Space stream freshness now.
9. Whether any daily/weekly prose rollup exists OUTSIDE this repo (chat summaries, Space streams
   with narrative content) — in-repo answer is none.
10. Mobile-device verification evidence for any PM-stack page (none found in-repo).
