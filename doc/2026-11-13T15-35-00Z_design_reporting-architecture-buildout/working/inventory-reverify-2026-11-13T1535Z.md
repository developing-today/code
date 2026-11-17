# Inventory re-verification at ATOMIC granularity — reporting-architecture directive

Date: 2026-11-13 (~09:35 CST). Verifier: workhorse subagent.
Inputs: `sources/source-brief.md` (09:11 CST compression), `sources/statement-inventory.md`,
`note.md` (proto design), starter issues #9568 (epic) through #9576 (C1–C8) via `gh issue view`.
Pass bar: zero unmapped at atomic granularity, each atom with a named primary mapping.

Conventions: `→ C1` = primary component. `+C2` = secondary/dependency (named).
`E` = starter epic #9568. `A-id` = federation/namespaces amendment (#9132/#9121/#9127).
Source anchors cite `source-brief.md` line numbers (SB:Ln).

## (a) Per-statement verdict table S0.1–S5.1

### Thread 0 → C1 (SB:L12–24)

- S0.1 Audit every reporting/anecdote/status side chat one by one. → C1 (deliverable A). Anchor SB:L12–14. Single atom. ✓ No split.
- S0.2a Each task owned/updated by its owner/originator chat. → C1. Anchor SB:L14–15.
- S0.2b That chat's agent edits the repo + pushes to backend WITHOUT reading from backend. → C1. Anchor SB:L15–17.
  (New split S0.2a/S0.2b; same component, informational.)
- S0.3 Each task names a category chat for relevant notes. → C1. Anchor SB:L17–18. ✓
- S0.4 Lanes chat receives all lane-effort notes. → C1. Anchor SB:L18. ✓
- S0.5 Issues & Designs covers all GitHub activity. → C1. Anchor SB:L19. ✓
  Note: proto-design/#9569 word this category "Designs, Issues & PRs" — "& PRs" is design-added wording, same scope.
- S0.6 One chat supports queues + PM-stack health + pending questions. → C1. Anchor SB:L19–21. ✓
  Note: brief is undecided ("Lanes, Reports, or another"); proto Q1 default = rename Orchestration pass chat (see table (c), D2).
- S0.7 Pinned category channels were abandoned when per-job chats proliferated; revive the pinned model. → C1. Anchor SB:L21–22. ✓
- S0.8 Task never reads its category chat: execute, insert update, end; chats are write-sinks. → C1. Anchor SB:L22–23. ✓
- S0.9a High-urgency: also post to main chat. → C1 (primary). Anchor SB:L23–24.
- S0.9b High-urgency: message other tasks or the orchestrator. → C1 (primary). Anchor SB:L23–24.
- S0.9c High-urgency: create queue items / follow-ups. → C2 (primary), +C1 (escalation pattern). Anchor SB:L24.
  FLAG: inventory lists "C1 (pattern), C2 (mechanism)" with no per-atom primary. This split resolves it.
  Cross-check: #9569 scope explicitly includes "jobs may message other tasks / the orchestrator / enqueue queue
  items", so #9569 currently claims even S0.9c; recommend #9570 own the queue-item mechanism half.

### Thread 1 → C2, C3 (SB:L26–37)

- S1.1a Explore better use of Muse hooks. → C2. Anchor SB:L26.
- S1.1b PM stack directly triggers Muse to fill queue items. → C2. Anchor SB:L26–27.
  (New split; same component.)
- S1.2a Consider removing much/all heavy repeated cron jobs. → C2. Anchor SB:L28.
- S1.2b Leave certain reports and checks (survivors named with reasons). → C2 (cron-triage table, per #9570).
  Anchor SB:L28–29 (partial — the "every surviving cron is named" requirement is design-added; see gap G4).
  FLAG: inventory's "with C1: every surviving cron is named" tag has no C1 issue anchor (#9569 never mentions
  crons; #9570 owns triage). Recommend dropping the C1 tag or re-anchoring.
- S1.3a "PM stack reachable" / "OpenCode working" become hook scripts firing on failure-to-connect. → C3. SB:L29–30.
- S1.3b On fire: immediately troubleshoot + report what occurred. → C3. SB:L30–31.
  (New split; same component.)
- S1.4 Explore OpenClaw / OpenChamber / OpenCode / Cloudflare Workers / PM-stack integration. → C2. SB:L31–32. ✓
- S1.5a GitHub org webhook items: logged + inserted into DBs. → C2. SB:L33–34.
- S1.5b Catchup-scan items: logged + inserted into DBs. → C2. SB:L33–34.
- S1.5c Items of a kind gathered into dedicated queues. → C2. SB:L34.
  (New split S1.5a/b/c; same component.)
- S1.6a Hook scripts drain queues on ≥10 new items. → C2. Anchor SB:L35.
- S1.6b …OR oldest item aged ≥15 min. → C2. Anchor SB:L35–36.
- S1.6c On drain: triage new issues. → C2. Anchor SB:L36.
- S1.6d On drain: review open PRs. → C2. Anchor SB:L36.
- S1.6e On drain: ensure a task minds each item. → C2. Anchor SB:L36.
- S1.6f On drain: report current + projected quota usage. → C2 (primary), +C4 (forecast data in). Anchor SB:L36–37.
  (Inventory's "(quota report hook → C4)" already names C2 first — primary OK; split makes it explicit.)

### Thread 2 → C4, C5 (SB:L39–51)

- S2.1 Recommendations to improve consistency and reduce troughs. → C5. Anchor SB:L39–40. ✓
- S2.2 Model changes (recommendations). → C5. Anchor SB:L40. ✓
  Note: "model changes" is ambiguous in compression (routing/model-use vs prediction-model); inventory reads it as
  provider/model-use → C5. No verbatim transcript available to disambiguate (gap G5 applies).
- S2.3a Predictions: when pools run out. → C4. Anchor SB:L40–41.
- S2.3b Predictions: when new models arrive. → C4. Anchor SB:L40–41.
- S2.4 Predictions: times we will be completely down. → C4. Anchor SB:L41. ✓
- S2.5a Track predictions. → C4. Anchor SB:L41–42.
- S2.5b Refine methodology until relatively accurate. → C4. Anchor SB:L42.
- S2.6 Day-type adjustments: other days / this time of day / other Fridays / other weekends (3 atoms, all → C4).
  Anchor SB:L42–44.
- S2.7 Spike-day modeling (week −10% → today +20% spike). → C4. Anchor SB:L44. ✓
- S2.8 Document efficiency regressions (quota down, merged PRs down further). → C5. Anchor SB:L45–46. ✓
- S2.9a Rate-limit analysis (are we hitting limits). → C5. Anchor SB:L46–47.
- S2.9b Ordered vs round-robin vs weighted routing per provider (config). → C5. Anchor SB:L47–48.
- S2.10 "Perhaps everything is perfect" allowed as evidence-tested outcome. → C5. Anchor SB:L48–49. ✓
- S2.11a Plan options: another OpenCode plan; CommandCode full; GLM plan. → C5. Anchor SB:L49–50.
- S2.11b Benchmark models that may outperform expectations. → C5. Anchor SB:L50.
- S2.12a Push notifications carrying forecast info (alerts). → C4 (primary). Anchor SB:L51.
- S2.12b Push notifications as page actions / surfaces. → C6 (primary). Anchor SB:L51.
  FLAG: inventory "C4 (forecast alerts), C6 (page actions)" names no overall primary; #9572 claims S2.12 (alerts)
  and #9574 claims S2.12 (surfaces) — both issues claim the whole statement. This split resolves the overlap;
  recommend each issue cite its half (S2.12a → #9572, S2.12b → #9574).
- S2.13 Reset-duration predictions as ranges (X–Y hrs; 2.5–3.7 days; "tomorrow afternoon–Sunday 2am"). → C4.
  Anchor SB:L79–80 (formats), SB:L45 (concept). ✓
- S2.14 Four recorded bases: trailing-30d rates; rates-until-last-reset; since-config-change; past-day vs 30d
  (4 atoms, all → C4). Anchor SB:L80–81. ✓
- S2.15 Never-serve rules (never Sonnet from a multi-model provider — only DeepSeek/GLM-class). → C5.
  Anchor SB:L101–102. ✓
- S2.16a Free providers have limited context windows (100k vs 1M) — observation. → C5. Anchor SB:L102–104.
- S2.16b Wire free providers into small paths; speed/scout if fast. → C5. Anchor SB:L104–105.
- S2.16c Free decision / smart free-for-a-time models in front of workhorse (vs 1–1.1M context). → C5. Anchor SB:L105–106.
- S2.17 Track TESTED context windows vs official record (usually smaller, sometimes larger). → C5. Anchor SB:L107. ✓
- S2.18a Wire reliable free/cheap providers in front whenever available. → C5. Anchor SB:L108–110.
- S2.18b Fall prevention on quota/rate-limit: work moves on, takes the cache hit (mechanism consumes, never
  duplicates, #9196 dispatch/selection). → C5 (primary), external constraint #9196 named. Anchor SB:L110–112.

### Thread 3 → C6 (SB:L53–86)

- S3.1a The Space carries the most redundant detail. → C6. Anchor SB:L53–54.
- S3.1b The Space is updated routinely. → C6. Anchor SB:L54.
- S3.2a Update triggers: every update / lane change / PR merge. → C2 (event source, primary). Anchor SB:L55–56.
- S3.2b Those triggers update the appropriate other spaces. → C6 (primary). Anchor SB:L55–56.
  (Inventory "C6 (event source → C2)" names C6 first — primary OK; split makes the C2 half explicit and ownable
  by #9570 as an event-kind row.)
- S3.3a Every space gets routine updates. → C6. Anchor SB:L56–57.
- S3.3b Tasks update their spaces on data arrival; nothing days/hours stale that must be current. → C6. SB:L57–58.
- S3.4a Push notifications open event-specific pages. → C6. Anchor SB:L59–60.
- S3.4b Push notifications open generic queue/quota monitoring pages. → C6. Anchor SB:L59–60.
- S3.5a Hook-gathered data in a JSON blob (hook-produced). → C2 (mechanism, primary for the production half).
  Anchor SB:L61.
- S3.5b Space job only swaps the data object / file pointer. → C6 (primary for the consumption half). SB:L61–62.
  NEW SPLIT — inventory maps the whole statement → C6 with no C2 annotation. Recommend #9570 own S3.5a
  (blob contract producer), #9574 own S3.5b.
- S3.6a MOBILE-FIRST hard requirement (large font, narrow width) for all spaces/reports/PM-stack pages. → C6.
  Anchor SB:L64–66.
- S3.6b Desktop = adaptation (two-column tiles, separate/degraded view), never the reverse; binds C7 pages too.
  → C6 (primary, layout rules), scope +C7. Anchor SB:L66–69.
- S3.7 Pages may have dynamic actual actions. → C6. Anchor SB:L71. ✓
- S3.8 Shared questions module sits next to facts on multiple pages. → C6. Anchor SB:L72–73. ✓
- S3.9a Confirmation-bubble action: queue an OpenAI / Anthropic / Fable reset at 100% (3 named targets). → C6.
  Anchor SB:L73–75.
- S3.9b Queueable from mobile. → C6. Anchor SB:L75.
- S3.10 Standing-rule candidate: all queues down + Claude at 100% → trigger cloud reset immediately. → C6
  (policy gate: Drew's explicit ruling/FDR per #9180 BEFORE any automation). Anchor SB:L75–76. ✓
  (This is proto-design Q4; see table (c) D6 — the gate is design-added, not in the directive sentence.)
- S3.11 Error question sets: fix it? / hotpatch service binary? / NixOS reset? / wait-for-downtime-then-reset? /
  feedback-comments/multi-selects (5 atoms, all → C6). Anchor SB:L81–84.
- S3.12a Answers spawn Muse tasks. → C2 (mechanism, primary for spawn half). Anchor SB:L84–85.
- S3.12b Spawned tasks update the page and notify (push / category chat / main chat). → C6 (primary). SB:L84–86.
  (Inventory "(task spawn → C2 mechanism)" already implies this; split formalizes it.)
- S3.13 Custom report pages per incident: own graphs; story; dynamic updates over time. → C6. Anchor SB:L88–90. ✓
- S3.14 Incident-page contents: progress; fix attempts ("trials and tribulations"); findings; recurrence
  prevention; related priors; final root cause; short/medium/long remediation (7 atoms, all → C6).
  Anchor SB:L91–93. ✓
- S3.15a Not every reset/wall earns a page (page-worthiness threshold). → C6. Anchor SB:L94–95.
- S3.15b–e Useful page kinds: today's activity; predicted workhorse full-wall; predicted wall-until time;
  pool-availability/quota-change special. → C6 (primary), +C4 (forecast data). Anchor SB:L95–98.
- S3.16 Efficiency/distribution diagnosis as a report genre (Muse-vs-OpenCode tokens vs PRs merged). → C6
  (primary), +C5 (accounting data). Anchor SB:L98–100. ✓
- S3.17a Reports listed somewhere (index; question-sets list is the precedent). → C6 (primary). Anchor SB:L112–113.
- S3.17b Reports later searchable (timeline-adjacent discovery). → C7 (primary). Anchor SB:L112–113.
  FLAG: inventory "C6 (index), C7 (timeline)" names no overall primary; #9574 claims S3.17 and #9575 claims
  "S3.17 timeline input". Split resolves it. Note S3.17b overlaps S4.17 (tree/sitemap + search, SB:L212–215) —
  distinct brief sentences, same C7 surface; not double-counting, record as convergent anchors.

### Thread 4 → C7, C8 (SB:L115–210)

- S4.1 Daily / weekly / monthly activity pages exist as a series. → C7. Anchor SB:L138. ✓
- S4.2 Period snapshots cloned into dated, unpinned, frozen library spaces. → C7. Anchor SB:L139–141. ✓
- S4.3 Per-project spaces (Fleet, PM stack, Atlas, ID): exists / wanted / in-progress / designed / inspiration +
  end state / open research / report links. → C7. Anchor SB:L141–146. ✓
- S4.4 PM stack answers "What did we do today?" in per-project prose with pivots + permanent remediations. → C7.
  Anchor SB:L147–151. ✓
- S4.5 Weekly view Mon-morning–CT to next Mon-morning–CT: biggest items/issues/designs; next-week plan; queue
  prioritization + custom report; spaces/links for next items/issues/PRs. → C7. Anchor SB:L151–156. ✓
- S4.6 Daily summaries roll backwards into weekly; done-this-week / planned-next / handled-last-week; forward
  days sketched. → C7. Anchor SB:L155–157. ✓
- S4.7 Context reach-back (through Friday of week before): how past work was used, issues it later caused. → C7.
  Anchor SB:L157–158. ✓
- S4.8a Daily window = midnight–midnight CT. → C7. Anchor SB:L159–160.
- S4.8b Soft edit window until early next morning (~2/5/8 am) for final details. → C7. Anchor SB:L160–161.
- S4.9 Post-window outcomes noted on the NEW day's report (confirmed/disproved hypothesis, however old). → C7.
  Anchor SB:L161–162. ✓
- S4.10 Soft windows: weekly ~1–2 days; monthly ~3–7 days (maybe a week); quarterly ~a week (3 atoms → C7).
  Anchor SB:L163–165. ✓
- S4.11 Post-window restricted edit forms: quote blocks; end appends; top banners; event-log injections;
  post-finalization feedback section (5 atoms, all → C7). Anchor SB:L165–169.
- S4.12 All rollups/pages feed Atlas embeddings and search. → C8. Anchor SB:L169. ✓
- S4.13 Per-design pages: subdesign trees with why-needed; status ladder
  (idea → planned → issue created → built out → fully designed → fully designed + impl issues). → C7.
  Anchor SB:L121–129. ✓
- S4.14 Implementation rollups (done / in-progress / blocked / queued counts) child→parent→roadmap/epic with
  summaries, reasons, statuses, per-day/week updates. → C7. Anchor SB:L129–136. ✓
- S4.15 Very long-running items get a fresh custom page per month/quarter. → C7. Anchor SB:L136. ✓
- S4.16a Date pages link happenings to quota reports. → C7 (primary), +C4 (data). Anchor SB:L133–135.
- S4.16b Timestamped generic pages; excerpts, graphs, pictures; hourly/timestamp event-log rollup. → C7.
  Anchor SB:L135–136.
- S4.17 Reports discovery: tree/sitemap + search over page names and text fields; not top-nav-only. → C7.
  Anchor SB:L212–215. ✓ (Converges with S3.17b; see note there.)
- S4.18 Pages backed by PM-stack stored data as permanent records; custom-pages/reports index;
  troubleshooting-reports grouping; reports timeline page. → C7. Anchor SB:L215–218. ✓
- S4.19 Timeline page: quarter → month → week → day unfolding; overall reports first column with 1–3 big
  positives/negatives, folded detail, deeper unfold without click-through. → C7. Anchor SB:L218–227. ✓
- S4.20 Domain weeklies (ID; fleet/queues/PM stack; Atlas/Turso/Cloudflare incl. LLM + tertiary quotas) as
  columns/subitems beside overall weekly. → C7. Anchor SB:L227–229. ✓
- S4.21 Dated special reports attach as columns/subitems of their day; long designs linked with phase-milestone
  dates. → C7. Anchor SB:L229–235. ✓
- S4.22 Monthly/quarterly rollups: roadmap, features, struggles, process improvements, cost/capability changes,
  traffic patterns, surprises, learnings. → C7. Anchor SB:L235–238. ✓
- S4.23 Monthly ideas/designs report (largest started/designed/closed; automation suggestions + ledger; kernels +
  rejected; passed-over alternatives; design-mill detail in prose; graphs/flow diagrams; wireframes/prototypes/
  SVG keyframes as UI grows; inspiration sketches incl. the ID/federation use-case material SB:L254–267). → C7.
  Anchor SB:L238–267. ✓ (Long passage compresses here; tail anchor is thin but present — "inspiration sketches".)
- S4.24a Atlas ingestion many times daily. → C8. Anchor SB:L115–116.
- S4.24b Recency bias in search. → C8. Anchor SB:L116–117.
- S4.24c Dataset bias toward reports on issues + major happenings, with resource links (GitHub/DB). → C8.
  Anchor SB:L117–119.
- S4.25a Page links become metadata links to related items. → C8. Anchor SB:L171–173.
- S4.25b Embeddings cross-link item↔linked items (1–2-step skips in search). → C8. Anchor SB:L173–176.
- S4.26 Rich page metadata (URL decomposition, media/page-kind flags, graph libraries, generating scripts,
  linked issues). → C8. Anchor SB:L177–182. ✓
- S4.27 Query-planner boosts: recency-sorted pulls for report/issue/PR kinds with labels + direct error strings.
  → C8. Anchor SB:L184–189. ✓
- S4.28 Error-driven investigation recipe (first-appearance → time range → issues/PRs/commits touching file/string
  → commit→PR→issue association). → C8. Anchor SB:L189–198 (first-appearance, event-log timing, keyword PRs,
  commit review). ✓
- S4.29a Per-issue joins: queue attempts, models used, worktrees used (data consumes #9073/#9074/#9075 records).
  → C8. Anchor SB:L196–198.
- S4.29b Unpushed-final-commit check in the worktree. → C8. Anchor SB:L199–200.
- S4.29c Session-transcript expectations by session ID (was the error expected; recorded handling instructions).
  → C8. Anchor SB:L200–204.
- S4.30a Rhyming fixes for the issue class (label families) assemble into a permanent class fix. → C8 (primary).
  Anchor SB:L204–207.
- S4.30b Class fix + investigation tooling → wider architectural fix making the class impossible. → C8 (primary).
  Anchor SB:L207–208.
- S4.30c Residual occurrences become routine triage scripts / subtasks of a triage job. → C2 (primary). SB:L208–210.
  FLAG: inventory "C8 (class-fix filing), C2 (triage subtasks)" names no overall primary; this split resolves it
  (S4.30a/b → #9576, S4.30c → #9570 as a drain-job row).

### Thread 5 → A-id (SB:L262–267 + intake instruction SB:L269–277)

- S5.1 All kiosk/handle/cross-signing material amends federation/namespaces: structured comment on #9132,
  pointers on #9121 and #9127. No new issues. Executed at intake. Anchor SB:L262–267. ✓ No split.

## (b) Newly discovered atomic splits (beyond the prompt's S0.9 / S2.12 / S4.30 examples)

1. S0.2a/S0.2b — ownership model vs push-without-read mechanic (both C1).
2. S1.1a/S1.1b — explore-hooks vs PM-direct-trigger (both C2).
3. S1.3a/S1.3b — failure-triggered conversion vs troubleshoot-and-report (both C3).
4. S1.5a/S1.5b/S1.5c — webhook source vs catchup-scan source vs dedicated-queue gathering (all C2).
5. S1.6a–f — two thresholds + four drain outputs; S1.6f carries the +C4 data dependency (C2 primary).
6. S2.3a/S2.3b; S2.5a/S2.5b; S2.6 (×3); S2.11a/S2.11b; S2.14 (×4 bases); S2.16a/b/c — same-component decompositions.
7. S2.9a/S2.9b — rate-limit analysis vs routing-mode decision (both C5).
8. S3.2a/S3.2b — C2 event sources vs C6 space-update action (inventory annotates; split formalizes, #9570-ownable).
9. **S3.5a/S3.5b — hook-produced JSON blob (C2 mechanism) vs space-job data-swap (C6). NOT annotated in inventory;
   the only cross-component split the inventory misses entirely.**
10. S3.6a/S3.6b — mobile-first requirement vs desktop-adaptation rules with +C7 scope.
11. S3.9a/S3.9b — three named reset targets vs mobile-queueability (both C6).
12. S3.11 (×5), S3.14 (×7), S3.15a–e, S3.4a/b, S3.1a/b, S3.3a/b, S3.8 — same-component decompositions.
13. S3.12a/S3.12b — Muse-task spawn (C2) vs page-update-and-notify (C6); inventory implies, split formalizes.
14. S3.17a/S3.17b — reports index (C6) vs searchable discovery (C7); convergent anchor with S4.17.
15. S4.8a/b; S4.10 (×3); S4.11 (×5); S4.16a/b; S4.23 (multi-atom); S4.24a/b/c; S4.25a/b; S4.29a/b/c — same-component.
16. S4.30a/S4.30b/S4.30c — class fix (C8) vs architectural fix (C8) vs residual triage subtasks (C2).

## (c) Disagreements / defaults table (compression vs proto-design/intake/issues)

- D1 (Q-numbering mismatch). `audit-side-chats.md` open questions are Q1–Q4 about the CHAT MAP only
  (Q4 = space-owner chats: keep vs fold). `note.md` open questions are Q1–Q6 about the PROGRAM
  (Q4 = standing cloud-reset rule; Q5 = surviving crons; Q6 = hook thresholds). Same Q-numbers, different
  referents. "Q1–Q4 defaults" is therefore ambiguous out of context — see gap G1.
- D2 (Q1). Brief undecided ("Lanes, Reports, or another", SB:L19–21) → proto/intake default: rename Orchestration
  pass chat (`964a47dc`) as category #4. Recorded default, no conflict. #9569 owns the question.
- D3 (Q2). Brief never proposes a Pools & Quota sink; proto/intake default = five chats (detail volume would drown
  exceptions). Design-added default, needs Drew ruling like the rest of the map. Category count reconciled: five =
  Reports / Lanes / Designs+Issues&PRs (one category, S0.5) / Orchestration / Pools&Quota.
- D4 (audit-Q4 / space-owner chats). Audit lists it OPEN (keep vs fold); proto chat-map states "keep as artifact
  owners" as decided; #9569 lists it as open Q4. Three postures for one question — needs Drew ruling; proto should
  not present it as settled.
- D5 (C1 5-chat map vs brief S0.6). Consistent once Q1/Q2 defaults are accepted; no independent conflict.
- D6 (C2 thresholds: 10 items / 15 min). Brief states them as directive values (SB:L35); proto Q6 re-records them
  as tunable defaults. Confirmed consistent — directive value becomes default, per-kind tuning is design work.
- D7 (C4 bases + reset-range formats). Brief's 4 bases (SB:L80–81) and 4 format examples (SB:L78–80) match proto C4
  verbatim. Confirmed, no drift.
- D8 (C6 standing cloud-reset rule, S3.10). Brief states it as a capability ("it can immediately trigger", SB:L75);
  proto + #9568 + #9574 add a SAFETY GATE absent from the directive: Drew's explicit ruling as an FDR per #9180
  before any automation. Design-added constraint — flag as intentional, requires Drew (gap G2).
- D9 (S1.2 C1 tag). Inventory's "with C1: every surviving cron is named" has no anchor in #9569 and conflicts with
  #9570's ownership of the triage table. Recommend C2-only mapping (gap G4).
- D10 (S2.2 "model changes"). Compression ambiguous (prediction-model vs provider/model-use); inventory + #9573
  read it as provider/model-use → C5. Cannot verify against verbatim without transcript (gap G5).

## (d) Gaps needing Drew or transcript evidence

- G1 (UNRESOLVED EVIDENCE GAP — do not invent). Drew's 09:28 CST approval of "Q1–Q4 defaults" is NOT in-repo:
  no 09:28 timestamp, no approval text, and no Q1–Q4 ruling appears in the design-doc directory, the audit doc,
  `note.md`, or issues #9568–#9576 (verified by grep + full reads 2026-11-13 ~09:35 CST). Consequence: the C1
  chat map (S0.1–S0.9, esp. Q1/Q2 defaults D2–D4) stays PROPOSED; #9569's acceptance criterion (Drew-approved map
  as FDR) is correctly still open. Which Q-set "Q1–Q4" refers to (audit chat-map Qs vs proto program Qs) is itself
  ambiguous per D1 — the approval text, when found, must be quoted against the right numbering.
- G2. Standing cloud-reset rule (S3.10 / proto-Q4): needs Drew's explicit ruling before automation (FDR per #9180).
  The 09:28 approval — if it covers proto-Q4 — would close this; unverifiable until G1 resolves.
- G3. Verbatim transcript of the 09:11 directive (Reports side chat `f5915c63`) was not available to this verifier;
  source-brief governs only "where they differ" per its own header. D10 and any compression drift (e.g. "& PRs"
  in S0.5, "inspiration sketches" compressing SB:L254–267) cannot be checked without it.
- G4. S1.2 survivor-cron naming: confirm C2-only (drop inventory's C1 tag) or anchor the C1 half to a #9569 edit.
- G5. S2.2 "model changes": confirm the C5 (provider/model-use) reading against verbatim or Drew.
- G6. Dual-claimed statements need per-half citations: S2.12 (split #9572/#9574 claims), S3.17 (#9574/#9575),
  S4.30 (#9576/#9570), S0.9c (#9569/#9570). Recommend editing the four issues to cite halves (a/b/c) per table (a).

## Verdict on the pass bar

Zero UNMAPPED atoms: every atomic claim above resolves to exactly one of C1–C8, epic E, an existing issue
(#9073/#9074/#9075 data for S4.29; #9196 constraint for S2.18b), or A-id. No source-brief sentence lacks an
inventory entry; no inventory entry lacks a source-brief anchor (thinnest: S4.23 tail → SB:L254–267;
S1.2b naming requirement → design-added, G4).
NOT YET ZERO-AMBIGUITY: four inventory lines (S0.9, S2.12, S3.17, S4.30) map to two components without a named
primary as written, plus S3.5's unannotated C2 half and S1.2's orphan C1 tag. Table (a) resolves each with
per-atom primaries; accepting those resolutions (and the (b)/(c) recommendations) is what clears the bar.
