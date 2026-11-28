# Proto design — reporting architecture: event-driven reporting, quota forecasting, living report pages

Status: PROTO (design-mill intake scaffold, 2026-11-13). The summit
elaboration turns this into the full design set; component issues carry the
open questions. Nothing here narrows Drew's directive: current tooling is
cited as collision-check and substrate only.

## Source

Drew's directive, Reports side chat, 2026-11-13 09:11 CST: re-found fleet
reporting on (0) a category-chat operating model, (1) event-driven hooks
instead of heavy repeated crons, (2) quota forecasting + efficiency
accounting, (3) living, mobile-first, actionable report pages and spaces,
(4) daily→quarterly rollups + design pages, all ingested into Atlas. The
fifth thread (identity/kiosk material) is NOT part of this program — it was
filed at intake as amendment comments on #9132 / #9121 / #9127 of the
federation/namespaces program (#9106).

Statement inventory: ~75 statements (S0.1–S5.1), each mapped to exactly one
component below, this epic, or an existing issue. Zero unmapped at intake;
the summit re-verifies at atomic granularity. Full inventory ships in this
PR under `sources/`.

## Components (starter issues under this epic)

- **C1 — Chat operating model.** One category side chat per job family;
  jobs execute, update the backend/repo themselves, and insert their update
  into their category chat insert-only (never read it). Category map:
  Reports (exceptions/decisions), Lanes (all lane effort), Designs, Issues
  & PRs (all GitHub activity), Orchestration — queues, PM health &
  questions, Pools & Quota (detail). Per-task notes lines name the sink.
  Escalation: exceptions → Reports; high-urgency → main chat too (the
  page-health-push job is the working precedent); jobs may message tasks /
  the orchestrator / enqueue queue items. The intake audit (chat-by-chat
  fates + reroute list) is this component's input; rerouting ships only
  after Drew approves the map.
- **C2 — Event pipeline.** GitHub org webhook events + catchup scans are
  logged to DBs AND gathered into dedicated per-kind queues; Muse hook
  scripts drain a queue when it holds ≥10 new items or its oldest item is
  ≥15 min old → triage new issues, review open PRs, ensure a task minds
  each item, report quota current + projected. PM stack triggers Muse to
  fill queue items directly. Cron triage: every surviving cron is named
  with its reason; the rest retire. Integration options across OpenClaw,
  OpenChamber, OpenCode, Cloudflare Workers, PM stack are evaluated here.
- **C3 — Health hooks.** "PM stack reachable" and "OpenCode working" checks
  become hook scripts that fire on failure-to-connect, immediately
  troubleshoot, and report what occurred (category chat + Reports if it
  becomes an exception). Successor to the disabled session-minder's
  detection role — hooks, not a revived cron.
- **C4 — Quota forecasting.** Pool-exhaustion, new-model, and complete-
  downtime predictions; reset-duration ranges ("X–Y hours", "2.5–3.7
  days", "between tomorrow afternoon and Sunday 2 am") on recorded bases
  (trailing-30d rates, rates-until-last-reset, since-config-change,
  past-day vs 30d) with day-type adjustments (time-of-day, Fridays,
  weekends) and spike-day modeling. Every prediction is logged and scored;
  methodology refines until predictions are relatively accurate. Forecast
  alerts go out as push notifications.
- **C5 — Efficiency + provider/model policy.** Efficiency accounting
  (quota consumed vs PRs merged — a quota drop with a larger output drop is
  a regression and is documented as one); consistency/trough reduction;
  routing-mode analysis per provider (ordered vs round-robin vs weighted,
  against measured rate limits); never-serve rules (which providers must
  not serve which models); free-provider inventory with TESTED context
  windows vs official records, wired into small/speed/scout and
  pre-workhorse decision paths where reliable, with fall prevention on
  quota/rate-limit; plan & model-change recommendations (extra OpenCode
  plan, CommandCode full, GLM, benchmarks of dark-horse models). "No
  change needed" is an allowed conclusion only with evidence.
- **C6 — Report pages & spaces freshness.** Every space/report is updated
  routinely by its owning task as data arrives (staleness is a defect with
  an exception path). Spaces may be driven by a hook-produced JSON data
  blob the space job merely swaps. **Mobile-first is a hard requirement**
  for every space, report, and PM-stack page (large font, narrow width
  primary; desktop adapts — wider/two-column — never the reverse). The
  shared questions module embeds beside facts; confirmation-bubble actions
  carry real effects (queue an OpenAI/Anthropic/Fable reset at 100%;
  standing rule candidate: when all queues are down and Claude walls,
  trigger a cloud reset immediately — Drew confirms the rule in design).
  Error question sets (fix / hotpatch / NixOS reset / wait-for-downtime /
  feedback multi-select) spawn Muse tasks that update the page and
  notify. Custom incident report pages (graphs; story; progress; fix
  attempts; findings; related prior incidents; root cause; short/medium/
  long remediation) and period/predicted-wall pages; a searchable reports
  index (the question-sets list is the shape precedent).
- **C7 — Rollups, design pages, timeline.** Daily (midnight–midnight CT) /
  weekly (Mon–Mon CT) / monthly / quarterly activity pages answering "what
  did we do today?" per project, with pivots and permanent remediations.
  Soft edit windows (daily: until early next morning; weekly ~1–2 days;
  monthly ~3–7 days; quarterly ~a week), then restricted edit forms (quote
  blocks, end appends, top banners, event-log injections, post-finalization
  feedback); post-window outcomes are noted on the new period's page.
  Period snapshots cloned into dated frozen library spaces. Per-project
  spaces (Fleet, PM stack, Atlas, ID): exists / wanted / in-progress /
  designed / inspiration / open research / report links. Per-design pages
  with subdesign trees and implementation rollups (done / in-progress /
  blocked / queued counts) rolling up to roadmap/epic pages. Reports
  timeline page (quarter → month → week → day unfolding; overall reports
  first column with 1–3 big positives/negatives and fold-outs; domain
  weeklies — ID, fleet/queues/PM stack, Atlas/Turso/Cloudflare — as
  columns/subitems; dated special reports attached to their day; long
  designs linked with phase-milestone dates). Pages are PM-stack stored
  records; sitemap/tree + full search, not top-nav only.
- **C8 — Atlas ingestion & investigation boosts.** All pages/reports ingest
  into Atlas many times daily, recency- and dataset-biased (reports on
  issues, major happenings). Page links become metadata links; embeddings
  cross-link linked items (1–2-step skips). Rich page metadata (URL
  decomposition, media type, page kind, graph libraries, generating
  scripts, linked issues). Query-planner boosts for error-driven
  investigation: first-appearance of an error string across embedded logs
  → time range → issues/PRs/commits touching that file/string →
  commit→PR→issue association, joined with queue attempts / models /
  worktrees, unpushed-final-commit checks, and session-transcript
  expectations; rhyming fixes feed permanent class fixes, and residuals
  become routine triage scripts/subtasks.

## Collision map (consume / extend — never duplicate)

- **#1184 PM-stack coherence** (single-URL, fresh data, triage-able,
  hierarchical): this program is its reporting substrate made event-driven.
  #1184 is at GitHub's 100-sub-issue cap → cross-linked, not parented
  (the #9060 intermediate-grouping precedent). Parent: fleet-tool epic
  #8990, alongside orchestration starter #9196.
- **#9196 PM Stack & Fleet orchestration**: consumes its dispatch,
  selection, and lifecycle design — C2/C5 specify reporting triggers and
  policy inputs, not a second dispatcher.
- **#9060 hierarchy + genealogy** (closed, components open): C7's design
  pages and rollups render the genealogy #9070 specifies; metadata comes
  from #9073/#9074; session state from #9075. Consume, don't re-model.
- **#9091 Atlas v1**: C8 is an input to its ingestion/classification/query
  work (E/G threads), not a parallel search program.
- **#9106 federation/namespaces**: received the identity amendment (#9132,
  #9121, #9127 comments) at intake; C8's metadata-link model should stay
  compatible with its triple/link model.
- **#9178 queue semantics** (resume-by-session, claim-any, manual-only) and
  **#9180 fleet decision records**: C2 assumes #9178's claim surface; this
  program's own rulings (chat map, reset rule, edit windows) are recorded
  as FDRs per #9180.
- **Fleet Reports Space** (17 streams, recordReport + fallback JSONL) and
  **page-health-push**: existing surfaces C6 builds on; the display defect
  (job-run status vs pool status) is a C6 input.
- **Questions/decision machinery** (pending-questions-hourly,
  decision-set-activity-processor, the questions module in PM stack): C6
  generalizes it; no second questions system.

## Chat map (summary — full audit is a program input document)

Keep + pin: Reports (exceptions), Lanes (lane effort), Designs, Issues &
PRs (GitHub activity). Rename + pin: Orchestration pass chat → queues, PM
health & questions. Adopt: Pool intelligence chat → Pools & Quota detail.
Retire as delivery targets: 28 per-lane shift chats, the GitHub-activity
per-job chats (~15), the questions pair as separate surfaces, rebuilt jobs'
origin chats (Space is the record). Stay retired: OpenChamber
conductor/babysitter/dispatcher, session minder, top-issue launchers.
Space-owner chats (6) keep as artifact owners. **No rerouting ships before
Drew approves the map** (intake deliverable A).

## Version sketch

- V1: chat map approved + rerouted (C1); health hooks live (C3); first
  event queue (GitHub issues) draining on thresholds (C2); forecasting
  logger scoring predictions against the existing burn-plan data (C4).
- V2: questions-module actions on quota pages incl. confirmable resets
  (C6); efficiency accounting + routing-mode recommendations (C5);
  daily/weekly rollups generated from stored data (C7).
- V3: design pages with rollup trees; reports timeline; period snapshot
  spaces (C7); Atlas cross-linked ingestion of all report kinds (C8).
- V4: error-driven investigation planner (C8) + cron retirements completed
  (C2); forecasting methodology validated against a full quarter of scored
  predictions (C4).

## Open questions (recorded defaults — summit batches, doesn't stall)

1. Category #4 home: rename the Orchestration pass chat (default) vs the
   Pending Questions chat vs a fresh chat.
2. Four category chats vs five (Pools & Quota separate). Default: five.
3. Retired per-job chats: archive after one verified cycle (default) vs
   leave dormant.
4. Standing cloud-reset rule at total wall (C6/S3.10): default = propose
   the rule as an FDR for Drew's explicit ruling before any automation.
5. Which crons survive the event-driven pass: default = only checks whose
   failure is itself the event, plus daily/weekly report closers; the
   summit names each survivor with its reason.
6. Hook thresholds per event kind: default 10 items / 15 min oldest,
   tuned per kind in design (PR reviews may differ from issue triage).
