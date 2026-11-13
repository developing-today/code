# Statement inventory — reporting-architecture directive (2026-11-13 09:11 CST)

Source: `sources/source-brief.md` (verbatim text in the Reports chat
transcript). Every statement maps to exactly one component (C1–C8), the
starter epic (E), an existing issue (named), or the identity amendment
(A-id). Zero unmapped is the pass bar; the summit re-verifies atomically.

## Thread 0 — chat operating model → C1 (audit = deliverable A of the intake)

- S0.1 Audit every reporting/anecdote/status side chat one by one. → C1 (done
  at intake as the audit; C1 owns the rerouting implementation)
- S0.2 Each task is owned/updated by its owner/originator chat; that agent
  makes the change in the repo and pushes to the backend without reading
  from the backend. → C1
- S0.3 Each task names a category chat for relevant notes ("send relevant
  notes to Lanes"). → C1
- S0.4 Lanes chat receives all lane-effort notes. → C1
- S0.5 Issues & Designs covers all GitHub activity. → C1
- S0.6 One chat supports queues + PM-stack health + pending questions. → C1
- S0.7 Pinned category channels were abandoned when per-job chats
  proliferated; revive the pinned category model. → C1
- S0.8 A task ideally never reads its category chat: execute, insert update,
  end. Chats are write-sinks, not state stores. → C1
- S0.9 High-urgency: also post to main chat; message other tasks or the
  orchestrator; create queue items/follow-ups. → C1 (pattern), C2 (mechanism)

## Thread 1 — event-driven reporting → C2, C3

- S1.1 Explore Muse hooks so the PM stack directly triggers Muse to fill
  queue items. → C2
- S1.2 Consider removing much/all heavy repeated cron; leave certain reports
  and checks. → C2 (with C1: every surviving cron is named)
- S1.3 "PM stack reachable" / "OpenCode working" become hook scripts firing
  on failure-to-connect → immediately troubleshoot + report what occurred. → C3
- S1.4 Explore OpenClaw / OpenChamber / OpenCode / Cloudflare Workers /
  PM-stack integration options. → C2
- S1.5 GitHub org webhook + catchup-scan items of a kind are logged, inserted
  into appropriate DBs, AND gathered into dedicated queues. → C2
- S1.6 Hook scripts drain queues on thresholds (≥10 new items OR oldest
  ≥15 min) → triage new issues, review open PRs, ensure a task minds each,
  report current + projected quota. → C2 (quota report hook → C4)

## Thread 2 — quota forecasting & efficiency → C4, C5

- S2.1 Recommendations to improve consistency and reduce troughs. → C5
- S2.2 Model changes (recommendations). → C5
- S2.3 Predictions: when pools run out; when new models come in. → C4
- S2.4 Predictions: times we will be completely down. → C4
- S2.5 Track predictions; refine methodology until relatively accurate. → C4
- S2.6 Day-type adjustments: vs other days, this time of day, other Fridays,
  other weekends. → C4
- S2.7 Spike-day modeling (down 10% on the week → predict +20% spike). → C4
- S2.8 Document efficiency regressions (quota down, merged PRs down further). → C5
- S2.9 Rate-limit analysis; ordered vs round-robin vs weighted routing per
  provider (config). → C5
- S2.10 "Perhaps everything is perfect" is an allowed, evidence-tested
  outcome. → C5
- S2.11 Plan/model options: another OpenCode plan; CommandCode full; GLM
  plan; benchmark models that may outperform expectations. → C5
- S2.12 Push notifications with useful info. → C4 (forecast alerts), C6 (page actions)
- S2.13 Reset-duration predictions as ranges (X–Y hours; 2.5–3.7 days;
  "between tomorrow afternoon and Sunday 2 am"). → C4
- S2.14 Prediction bases: trailing-30d rates; rates-until-last-reset;
  since-config-change; past-day vs 30d. → C4
- S2.15 Provider/model policy: never serve model X from provider Y (e.g.
  Sonnet only from certain providers; DeepSeek/GLM paths). → C5
- S2.16 Free-provider inventory: limited context windows; wire into small /
  speed / scout paths; free decision models in front of workhorse. → C5
- S2.17 Track TESTED context windows vs official records (usually smaller,
  sometimes larger). → C5
- S2.18 Reliable wiring of free/cheap providers in front whenever available,
  with fall prevention on quota/rate-limit (work moves on, takes cache hit). → C5
  (mechanism overlaps #9196 dispatch/selection — consume, don't duplicate)

## Thread 3 — report pages, spaces freshness, actions → C6

- S3.1 The Space carries the most redundant detail and is updated routinely. → C6
- S3.2 Every update / lane change / PR merge updates the appropriate other
  spaces. → C6 (event source → C2)
- S3.3 Every space has routine updates AND task-driven updates on data
  arrival; never days/hours stale for current information. → C6
- S3.4 Push notifications open event-specific pages or generic queue/quota
  monitoring pages. → C6
- S3.5 Spaces may be driven by a hook-produced JSON blob; the space job only
  swaps the data object / file pointer. → C6
- S3.6 MOBILE-FIRST is a hard requirement for all spaces, reports, PM-stack
  pages (large font, narrow width); desktop = adaptation (two-column tiles,
  separate or degraded view), never the reverse. → C6 (applies to C7 pages too)
- S3.7 Dynamic actions on pages. → C6
- S3.8 The shared questions module embeds beside facts on multiple pages. → C6
- S3.9 Confirmation-bubble actions with real effects: queue an
  OpenAI/Anthropic/Fable reset when a pool reaches 100% (queueable from
  mobile). → C6
- S3.10 When all queues are down and Claude walls at 100%, trigger a cloud
  reset immediately. → C6 (policy gate: Drew confirms the standing rule)
- S3.11 Error question sets: fix it? hotpatch the service binary? NixOS
  reset? wait for downtime then reset? more feedback / comments /
  multi-selects. → C6
- S3.12 Answers spawn Muse tasks that update the page and notify (push /
  category chat / main chat). → C6 (task spawn → C2 mechanism)
- S3.13 Custom report pages per incident: own graphs; story (start,
  beginning, end); dynamic updates over time. → C6
- S3.14 Incident report contents: progress, fix attempts ("trials and
  tribulations"), findings, recurrence prevention, related prior incidents,
  final root cause, short/medium/long remediation. → C6
- S3.15 Not every reset/wall needs a page; useful page kinds: today's
  activity; predicted workhorse full-wall; predicted wall-until time;
  pool-availability/quota-change special report. → C6 (forecast data → C4)
- S3.16 Efficiency/distribution diagnosis is a report genre (tokens by
  surface vs PRs merged). → C6 (accounting → C5)
- S3.17 Reports are listed somewhere (like the question-sets list) and
  searchable. → C6 (index), C7 (timeline)

## Thread 4 — rollups, design pages, Atlas → C7, C8

- S4.1 Daily / weekly / monthly activity pages exist as a series. → C7
- S4.2 Period snapshots cloned into dated, unpinned, frozen library spaces
  (name + date / week number / year+quarter). → C7
- S4.3 Per-project spaces (Fleet, PM stack, Atlas, ID): exists / wanted /
  in-progress / designed / inspiration + end state / open research / report
  links; link to other spaces. → C7
- S4.4 PM stack answers "What did we do today?" in prose per project, with
  pivots and permanent remediations named. → C7
- S4.5 Weekly view Mon–Mon CT: biggest items/issues/designs changed; next
  week's plan; queue prioritization summary + custom report; spaces/links
  for next items, issues, PRs. → C7
- S4.6 Daily summaries roll into weekly backwards; done-this-week /
  planned-next / handled-last-week structure; forward days sketched. → C7
- S4.7 Context reach-back (e.g. through Friday of the week before) incl. how
  past work was used and issues it later caused. → C7
- S4.8 Daily window = midnight–midnight CT; soft edit window until early
  next morning (~2/5/8 am) for final details. → C7
- S4.9 Post-window outcomes are noted on the NEW day's report (confirmed /
  disproved hypothesis, invalidated benchmark — however old the setup). → C7
- S4.10 Soft edit windows: weekly ~1–2 days; monthly ~3–7 days; quarterly
  ~a week. → C7
- S4.11 After the window, edits restricted to: quote blocks, end appends,
  top banners, event-log injections, post-finalization feedback section. → C7
- S4.12 All rollups/pages feed Atlas embeddings and search. → C8
- S4.13 Per-design pages: subdesign trees with why-needed (epic/roadmap/
  origin), per-design status ladder (idea → planned → issue created →
  issue built out → fully designed → fully designed + implementation
  issues). → C7
- S4.14 Implementation rollups per design: counts done / in-progress /
  blocked / queued, rolling up child→parent→roadmap/epic pages with
  summaries, reasons, statuses, per-day/week updates. → C7
- S4.15 Very long-running items get a fresh custom page per month/quarter. → C7
- S4.16 Date pages link happenings to quota reports and timestamped generic
  pages; excerpts, graphs, pictures; hourly/timestamp event-log rollup. → C7
- S4.17 Reports discovery: tree/sitemap + search over page names and text
  fields; not top-nav only. → C7
- S4.18 Pages are backed by PM-stack stored data as permanent records;
  custom-pages/reports index; troubleshooting-reports grouping; reports
  timeline page. → C7
- S4.19 Timeline page: quarter → month → week → day unfolding; overall
  reports = first column with 1–3 big positives/negatives, folded detail,
  deeper unfold without click-through. → C7
- S4.20 Domain weeklies (ID; fleet/queues/PM stack; Atlas/Turso/Cloudflare
  incl. LLM + tertiary cloud quotas) as columns/subitems beside the overall
  weekly. → C7
- S4.21 Dated special reports (e.g. a queue overhaul) attach as a column/
  subitem to their day; long designs appear as links with phase-milestone
  dates (created, phase starts/ends, pauses for priority). → C7
- S4.22 Monthly/quarterly rollups: roadmap, new features, struggles,
  process improvements, cost/capability changes, traffic patterns,
  surprises, learnings. → C7
- S4.23 Monthly ideas/designs report: largest designs started / fully
  designed / closed out (implemented or canceled); automation-suggested
  ideas + ledger items; brainstorm kernels + rejected ideas; passed-over
  alternatives; design-mill and small-design details in prose; visual
  storytelling (graphs, flow diagrams: what it does, now vs change, why now
  blocks, fix horizons, why this design, extensibility picture);
  wireframes/prototypes/screenshots/SVG keyframes as UI work grows;
  inspiration sketches. → C7
- S4.24 Atlas ingestion many times daily; recency bias; dataset bias toward
  reports on issues + major happenings, with resource links (GitHub/DB). → C8
- S4.25 Page links become metadata links; embeddings cross-linked between an
  item and its linked items (1–2-step skips in search). → C8
- S4.26 Rich page metadata: URL decomposition (base, path, filename, last
  part, port, MIME/media type), HTML/video flags, page kind (report,
  comments+stream, live), graphs + libraries used, generating scripts
  (Python/JS), linked GitHub issues. → C8
- S4.27 Query-planner boosts: recency-sorted pulls for report/issue/PR
  kinds with labels and direct error strings. → C8
- S4.28 Error-driven investigation: error string's first appearance across
  embedded logs + event-log timing → time range → issues/PRs/commits
  touching the file/string; commit→PR→issue association. → C8
- S4.29 Per-issue investigation joins: queue attempts, models used,
  worktrees used; unpushed final commit check; session-transcript
  expectations (session ID) — was the error expected, were handling
  instructions recorded. → C8 (data from #9073/#9075 Metadata/session state)
- S4.30 Rhyming fixes: related fixes for the same issue class (label
  families) feed a permanent class fix; combined → architectural fix making
  the class impossible; residual occurrences become routine triage scripts /
  subtasks. → C8 (class-fix filing), C2 (triage subtasks)

## Thread 5 — identity material → A-id (amendment, NOT a component)

- S5.1 All kiosk / handle / cross-signing material amends the
  federation/namespaces program: structured comment on #9132, pointers on
  #9121 and #9127. No new issues. (Executed at intake.)
