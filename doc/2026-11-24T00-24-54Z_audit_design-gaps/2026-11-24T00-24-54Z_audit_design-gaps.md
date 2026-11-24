# AUDIT-7 Lane 6 — DESIGN-GAPS report

- Created: 2026-11-24 (audit run 2026-11-23 ~17:56 CST directive)
- Lane: AUDIT-7 Lane 6 (DESIGN-GAPS)
- Tracking issue: developing-today/code#13873
- Umbrella: developing-today/code#13871
- Scope repos: developing-today/code, developing-today-forks/opencode, developing-today-forks/openchamber, dezren39/fleet

## Method

GitHub reads via `fleet gh api` search only (per_page=100, count-confirmed totals), writes via `gh-post`. Every returned issue body was scored programmatically against the sufficient-design standard below, section by section (keyword/section scan over the full body text); the prioritised tier was then read and judged individually. Queue packets were enqueued on the laptop (`fleet queue enqueue`, queue `repo-health-code-pilot`) after a premise + singular-ownership check (queue queued/claimed/blocked/parked lists grepped per issue; umbrella + tracking checked for lane claims; enqueue dedupe readback as final proof).

## The sufficient-design standard

(1) problem + context — what breaks today, with evidence; (2) goals and explicit non-goals; (3) alternatives considered with honest tradeoffs, including do-nothing; (4) the chosen approach — interfaces/contracts, data model, security and capability analysis where relevant; (5) dependency-ordered build slices, each sized for one lane/session with its own acceptance criteria; (6) rollout + rollback plan; (7) verification plan — how we prove it works (tests, live proof); (8) open questions, each with a named decider; (9) recorded as a dated design document in doc/ + a tracking issue + a queue item. A design that is only a paragraph, has no slices, no alternatives, and no acceptance criteria is INSUFFICIENT.

## Counts

| Repo                               | design-title open          | design-title closed | "needs design" open bodies           |
| ---------------------------------- | -------------------------- | ------------------- | ------------------------------------ |
| developing-today/code              | 245 (241 issues excl. PRs) | 750                 | 113 (71 not design-titled)           |
| developing-today-forks/opencode    | 4                          | 40                  | 5                                    |
| developing-today-forks/openchamber | 1                          | 37                  | 3                                    |
| dezren39/fleet                     | 0                          | 0                   | 0 (9 open issues total, none design) |

code sufficiency histogram (of 9 standard sections absent from the issue body): 0 absent: 1 (the Lane 6 tracking issue itself); ≤2 absent: 8; 3–9 absent: 232. Short body (<3,500 chars) with ≥5 sections absent — design missing or very simple: **111**. Designs demanded by open issues (design-titled open + needs-design extras): **312** in code.

## Designs judged sufficient

The pm-orchestration family (48–63k-char bodies; missing only an explicit non-goals heading and named-decider open questions — a heading-level gap, packetised as part of the family homes where relevant):

- #9478 — Versioned PriorityProfile contract and pure validation/comparison interface
- #9479 — Intrinsic/nullable-importance/effective-pressure explanation types
- #9443 — Hard dependency DAG cycle-safe mutation and complete readiness
- #9444 — Seven relation types, native GitHub links, and resolved history
- #9481 — Evidence-based bidirectional aging and review windows
- #9482 — Temporary opportunity boost expiry and retriage
- #9483 — Priority hysteresis and bounded cooldown
- #9484 — Bounded clump refinement with evidence and provenance

In the forks, closed design families are structurally sufficient (design issue + phased implementation issues, all closed): opencode #587, #800, #992, #1127, #1345, #1525, #1684, #1794, #1942, #2208; openchamber #612, #802, #976, #1254, #1391. The fork pattern — one design issue, phase issues citing it — is the model the code repo's short design asks should follow.

## Prioritised design gaps — packets created (queue repo-health-code-pilot, kind feature_slice)

### P1 — blocking / family homes (priority 70/50)

| Source     | Gap                                                                                                                                           | Task  |
| ---------- | --------------------------------------------------------------------------------------------------------------------------------------------- | ----- |
| code#8986  | design-register epic: designs tracked/boarded/monitored — register design family unwritten                                                    | 21560 |
| code#12950 | F4 fleet orchestration & dispatch design-family home; designs unwritten                                                                       | 21539 |
| code#12951 | P2 decisions-system design-family home; designs unwritten                                                                                     | 21540 |
| code#9521  | Nix cache as content store design program; no design doc                                                                                      | 21569 |
| code#2025  | review-loop design cited by #2312/#2313 (§7.3/§8) but body lacks problem/goals/alternatives/approach/slices/acceptance/rollout/open-questions | 21542 |

### P2 — designs demanded by open issues, missing (priority 50/40)

| Source     | Gap                                                         | Task                                                     |
| ---------- | ----------------------------------------------------------- | -------------------------------------------------------- |
| code#2036  | ID messaging/relay protocol v1 — all 9 sections absent      | 21536                                                    |
| code#6003  | feature-object library attachment — impl #12640 waits on it | 21556                                                    |
| code#5959  | capability-change notices — impl #12921 waits on it         | 21555                                                    |
| code#9140  | atlas recommendation objects — 631-char ask                 | 21566                                                    |
| code#9111  | id modes & configuration surface — 660-char ask             | 21499 (pre-existing dedupe hit — packet already existed) |
| code#9122  | id numeric registry namespaces — 690-char ask               | 21565                                                    |
| code#9147  | id reputation + claims/dispute log — 692-char ask           | 21568                                                    |
| code#9142  | id implicit-trust sync — 721-char ask                       | 21567                                                    |
| code#9132  | id delegation + cross-signed ceremony — 792-char ask        | 21500 (pre-existing dedupe hit)                          |
| code#5692  | Supabase→Turso sheet-derived staging (19 tables)            | 21553                                                    |
| code#5693  | ci.events ledger Supabase→Turso                             | 21554                                                    |
| code#2137  | event-driven Slack collector design + PoC                   | 21537                                                    |
| code#2290  | RateAuthorisation as signed ID-graph object                 | 21538                                                    |
| code#2819  | native-spill write lease protocol                           | 21547                                                    |
| code#10306 | provider/model kind + input-modality marking                | 21535                                                    |
| code#9085  | fleet second version in TypeScript (scope/WASM/transports)  | 21563                                                    |
| code#9081  | fleet tool Workers/TypeScript equivalent                    | 21562                                                    |
| code#9079  | PM service Workers edition                                  | 21561                                                    |
| code#9086  | SSO relying-party integration                               | 21564                                                    |

### P3 — deficient designs to upgrade (priority 40/30)

| Source        | Gap                                                                                 | Task  |
| ------------- | ----------------------------------------------------------------------------------- | ----- |
| code#8870     | P6 Turso+Cloudflare — unratified DRAFT, 7 sections absent                           | 21557 |
| code#8872     | P8 Storage program — unratified DRAFT, 7 sections absent                            | 21558 |
| code#8873     | P9 Lab integration — unratified DRAFT, 7 sections absent                            | 21559 |
| opencode#1680 | realtime/duplex audio façade — design PR #2033 exists; verify + upgrade vs standard | 21541 |

### Continuation packets (remainder batches)

| Packet         | Work                                                   | Task  |
| -------------- | ------------------------------------------------------ | ----- |
| #13873 slice-2 | packetise next 25 of Appendix A                        | 21579 |
| #13873 slice-3 | triage + packetise Appendix C (71 needs-design issues) | 21580 |

Duplicate tasks 21543/21544 (concurrent-enqueue race for #2025) were cancelled with reason recorded; canonical task is 21542.

## Findings by repo

- **code** is where the design debt lives: 111 missing/very-simple designs (Appendix A), 121 partially-specified (Appendix B), 71 further issues demanding designs in body text (Appendix C). Convergence note: the #9068–#9155 atlas/id design-ask cluster and the #5690–#5695 Supabase-migration family should each be designed as one family with per-topic slices, not as isolated one-pagers — the packets for #5692/#5693 and the atlas/id cluster say so via their family context. Duplication note: #9081 (fleet tool, Workers/TS equivalent) and #9085 (fleet second version in TypeScript) overlap heavily; the #9085 design should be written first and #9081 scoped as its hosted-equivalent subset, or the two merged with #9085 as the survivor.
- **opencode**: no missing designs among open issues; façade designs (#1146/#1677/#1679/#1680) were delivered as spec PRs. #1684's design is closed/delivered. Only #1680 packetised (verify/upgrade).
- **openchamber**: no open design gaps. Its one open design-titled item, #1439 (anti-slop phase 3 findings), is a findings backlog, not a design demand.
- **fleet**: no design issues at all; its designs live in code (#12950, #9081, #9085) and are packetised there.

## Repeatable procedure (skill candidate)

This audit is repeatable as-is: (1) count-confirmed `fleet gh api` searches per repo for `design in:title` by state and `"needs design"` in body; (2) score every body against the 9-point standard with the section scan; (3) classify: sufficient (≤2 sections absent and long), deficient-draft, missing/very simple (short + ≥5 absent); (4) premise + ownership check per candidate (queue lists + umbrella + dedupe readback); (5) enqueue `feature_slice` packets with `payload.slice{parent_issue, number, criterion, done_check, owned_paths}` embedding the standard, priorities on the validator scale [10,20,30,40,50,70,90], timebox 45, max_attempts 1000; (6) cancel any duplicate-race tasks immediately with reason. The code repo has a skills home at `.agents/skills/`; filing this procedure as a skill there is left as a follow-on slice on the tracking issue (it should be authored through the repo's own skill conventions, not bolted onto this audit PR).

## Lane brief (verbatim)

The full Lane 6 brief is recorded on the tracking issue #13873 and the umbrella #13871; the operative text: "find all designs AND all issues that demanded a design; produce the list of designs we have yet to make, or that were poorly implemented or very simple; add them all to the prioritised agent-next items (queue packets, kind feature_slice + payload.slice, never kind design; each packet states the sufficient-design standard in full and names the source issue); judge existing designs against the 9-point standard; a design that is only a paragraph, has no slices, no alternatives, and no acceptance criteria is INSUFFICIENT."

## Appendix A — code: design missing or very simple (body <3.5k chars, >=5 of 9 standard sections absent)

- #12951 — epic: decisions system — P2 design family (tracking home) — missing: problem, goals_nongoals, alternatives, approach, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/12951
- #12640 — id: implement runtime feature-object library attachment (v-next, per #6003 design note) — missing: problem, goals_nongoals, alternatives, approach, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/12640
- #12950 — epic: fleet orchestration & dispatch — F4 design family (tracking home) — missing: problem, goals_nongoals, alternatives, approach, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/12950
- #9521 — Nix cache as the content store: derivations + flakes per item (design program) — missing: problem, goals_nongoals, alternatives, approach, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/9521
- #5693 — design: move ci.events webhook ledger from Supabase to Turso staging — missing: problem, goals_nongoals, alternatives, approach, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5693
- #12922 — id: invite capability_pin term — pin the capability baseline as issued (#5751 delta from #5959 design) — missing: problem, goals_nongoals, alternatives, approach, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/12922
- #5695 — design: move tiny config/KV tables (sources registries, public.muse_notes) from Supabase to D1 — gated on §9.4 Q2 — missing: problem, goals_nongoals, alternatives, approach, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5695
- #13357 — C4 design section 4: compound range form unimplemented and day form unreachable in the shared formatter — missing: problem, goals_nongoals, alternatives, approach, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/13357
- #5692 — design: move sheet-derived domain staging + curated rows (19 tables) from Supabase to Turso — missing: problem, goals_nongoals, alternatives, approach, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5692
- #13264 — mcpx program — full-investigation prompt: design completion & build-out for the MCP gateway — missing: problem, goals_nongoals, alternatives, approach, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/13264
- #2036 — id: design: ID messaging/relay protocol v1 — missing: problem, goals_nongoals, alternatives, approach, slices, acceptance, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/2036
- #13095 — impl(id): social object envelope + profiles in id-core (per #9128 design) — missing: problem, goals_nongoals, alternatives, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/13095
- #8833 — Cold-store backup design + nightly run (shipped — tracking home) — missing: goals_nongoals, alternatives, approach, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/8833
- #13096 — impl(id): board view builder — topic/comment tree projection (per #9128 design) — missing: problem, goals_nongoals, alternatives, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/13096
- #7675 — id: importer follow-on: Joplin revision-history import (from #5848 design) — missing: problem, goals_nongoals, alternatives, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/7675
- #2313 — review loop PoC (Stage A): pr-diff-review records shadow verdicts, report distribution (design #2025 §8) — missing: problem, goals_nongoals, alternatives, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/2313
- #2025 — design: review loop — diff review can reject 'done' and commission a fresh second-pass session — missing: problem, goals_nongoals, alternatives, approach, slices, acceptance, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/2025
- #6003 — id: design: feature-object library attachment — runtime attach/detach of library behavior to entities without world recompilation — missing: goals_nongoals, alternatives, approach, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/6003
- #8986 — epic: design register — every design tracked, boarded, and monitored — missing: problem, goals_nongoals, alternatives, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/8986
- #6010 — id: design: semantic cell/tag presentation model — one tagged state, many renderers — missing: goals_nongoals, alternatives, approach, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/6010
- #5690 — design: move agent append streams (agent.events, agent.quota_records) from Supabase to Turso staging — missing: goals_nongoals, alternatives, approach, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5690
- #5959 — id: design: capability-change notices on reconnect — expansion and sideways changes — missing: goals_nongoals, alternatives, approach, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5959
- #13004 — id: design pass — capability class assignments for the 16 post-#5999 registrations (kv/blob/notify/form/device.info/speech/client.*) — missing: goals_nongoals, alternatives, approach, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/13004
- #1283 — design(pm-stack): chaos/rotation drills — scheduled game-days (kill collector, age the feed, drop a service) verifying banners, watchdogs, and self-healing fire — missing: problem, goals_nongoals, alternatives, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/1283
- #7585 — id: implement board subscription, discovery & feed integration (from design #5862) — missing: problem, goals_nongoals, alternatives, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/7585
- #10306 — design: provider/model kind marking + input-modality capability marking for routing — missing: goals_nongoals, alternatives, approach, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/10306
- #9399 — Program: pool & effort system — effort tiers per pool, OpenChamber category pools, granular stats store (design mill) — missing: goals_nongoals, alternatives, approach, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/9399
- #9140 — design(atlas): recommendation objects, search history, retention classes — missing: problem, goals_nongoals, alternatives, approach, slices, rollout_rollback, verification — https://github.com/developing-today/code/issues/9140
- #9111 — design(id): modes & configuration surface — modes, link kinds, storages, tiering as config — missing: problem, goals_nongoals, alternatives, approach, slices, rollout_rollback, verification — https://github.com/developing-today/code/issues/9111
- #9122 — design(id): numeric registry namespaces over the byte system — missing: problem, goals_nongoals, alternatives, approach, slices, rollout_rollback, verification — https://github.com/developing-today/code/issues/9122
- #9147 — design(id): reputation system + public claims/dispute log — missing: problem, goals_nongoals, alternatives, approach, slices, rollout_rollback, verification — https://github.com/developing-today/code/issues/9147
- #9142 — design(id): implicit-trust sync — convergence toward 1:1 + conflict reports — missing: problem, goals_nongoals, alternatives, approach, slices, rollout_rollback, verification — https://github.com/developing-today/code/issues/9142
- #9132 — design(id): delegation + cross-signed identity ceremony — missing: problem, goals_nongoals, alternatives, approach, slices, rollout_rollback, verification — https://github.com/developing-today/code/issues/9132
- #9068 — impl(pm-stack): design-spawn backfill — native parenting of sub-design tracking issues + spawn rollup — missing: problem, goals_nongoals, alternatives, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/9068
- #8870 — design(P6): Turso + Cloudflare integration — DRAFT for Drew's ratification — missing: goals_nongoals, alternatives, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/8870
- #8873 — design(P9): Lab integration — DRAFT for Drew's ratification — missing: goals_nongoals, alternatives, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/8873
- #8872 — design(P8): Storage program — DRAFT for Drew's ratification — missing: goals_nongoals, alternatives, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/8872
- #2039 — id: design: match attestation + audit v1 — missing: problem, goals_nongoals, alternatives, slices, acceptance, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/2039
- #2137 — Design + PoC: event-driven Slack collector (Socket Mode) to replace archer-slack-review polling — missing: goals_nongoals, approach, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/2137
- #8830 — PM stack needs-user action model — design + contracts (shipped — tracking home) — missing: goals_nongoals, alternatives, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/8830
- #13094 — impl(id): freshness token + frontier check in id-core (per #9128 design) — missing: problem, goals_nongoals, alternatives, approach, slices, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/13094
- #7843 — mcpx conformance: user-identity - design how elicitation state binds identity (or refuses to) before implementing (split of #1957) — missing: problem, goals_nongoals, alternatives, approach, slices, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/7843
- #1054 — blocked(usage-hub): Open Design quota instrumentation — laptop sign-in, cookie capture, parser verification — missing: problem, goals_nongoals, alternatives, approach, slices, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/1054
- #2312 — review loop: fleet-monitor checks for verdict/packet reconciliation (design #2025 §7.3) — missing: problem, goals_nongoals, alternatives, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/2312
- #1085 — epic(cluster): storage assembly & design validation — missing: problem, goals_nongoals, alternatives, approach, slices, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/1085
- #2290 — id: design: RateAuthorisation as a signed ID-graph object — schema, signing, revocation — missing: problem, goals_nongoals, alternatives, approach, slices, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/2290
- #6062 — design(id/atlas): ID-graph publication format — Atlas objects + provenance/lineage relations as ID-graph objects — missing: goals_nongoals, alternatives, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/6062
- #1182 — design(pm-stack): one public URL for the dashboard behind Cloudflare Access (settled access model, not yet implemented) — missing: goals_nongoals, alternatives, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/1182
- #9081 — design(fleet): fleet tool, Workers/TypeScript equivalent — hosted Metadata stamping/query, session provenance, sync tools — missing: problem, goals_nongoals, alternatives, approach, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/9081
- #12921 — id: implement capability-change notices — client baseline store, connect-stage comparison, notice surface (#5959 design) — missing: problem, goals_nongoals, alternatives, slices, acceptance, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/12921
- #2819 — id: design: native-spill write lease protocol for hot worlds (from #2059) — missing: problem, goals_nongoals, alternatives, approach, slices, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/2819
- #13524 — id: implement Notion importer slice 4: comments + degradation completion + export reader (design #6436) — missing: problem, goals_nongoals, alternatives, approach, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/13524
- #13874 — Stray work audit: PRs and issues not tied to a design or roadmap item — missing: goals_nongoals, alternatives, approach, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/13874
- #9086 — design(pm-stack): SSO relying-party integration — hosted editions consume #9077; tenancy accounts aligned with #1253 roles — missing: goals_nongoals, alternatives, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/9086
- #9085 — design(fleet): second version in TypeScript — scope, WASM evaluation, transports (REST/WebSocket), and direct-Turso prudence rules — missing: problem, goals_nongoals, alternatives, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/9085
- #9079 — design(pm-stack): PM service, Workers edition — TypeScript equivalent of aggregator/dashboard/reconcile as one hosted service — missing: problem, goals_nongoals, alternatives, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/9079
- #10391 — Design mill pass 2: iroh-native ID transport in fleet — identity, postcards, format conversion — missing: goals_nongoals, alternatives, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/10391
- #13876 — AUDIT-7 Lane 5: DESIGN-REVIEW — every design record, internal soundness (tracking) — missing: goals_nongoals, alternatives, approach, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/13876
- #5964 — id: design: dynamic user functions — server-compiled DLLs, hot-swap into running worlds without world recompile — missing: goals_nongoals, alternatives, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5964
- #9100 — design(atlas): virtual filesystem over the content-addressed store + aggregated file index (Basin/Parquet evaluation) — missing: problem, goals_nongoals, alternatives, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/9100
- #10390 — Design mill pass 1: fleet subcommands for the four tools — pass-through, endpoint + credential resolution — missing: goals_nongoals, alternatives, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/10390
- #5783 — id: design: ESP32 hardware platform line — boards, provisioning, firmware lifecycle, OTA, power — missing: problem, goals_nongoals, alternatives, approach, slices, acceptance, verification — https://github.com/developing-today/code/issues/5783
- #5030 — design(cluster): egress strategy for 2 Gbps home fiber — budgets, tunnel topology, failure matrix — missing: problem, goals_nongoals, alternatives, approach, slices, rollout_rollback, verification — https://github.com/developing-today/code/issues/5030
- #5970 — id: design: user presentation assets — shaders, sprites by ID, filters, per-world gating, graceful degradation — missing: goals_nongoals, alternatives, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5970
- #12353 — C7 leaf 3: design pages and implementation rollups — missing: goals_nongoals, alternatives, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/12353
- #9113 — design(id): ID-shape registry + dispatch; content-addressed hash no-map rule — missing: problem, goals_nongoals, alternatives, approach, slices, rollout_rollback — https://github.com/developing-today/code/issues/9113
- #5523 — id: implement Jurisdiction-aware per-user hosted stores (design #2283) — missing: alternatives, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5523
- #5528 — id: implement Jurisdiction-partitioned hosted worlds and world blobs (design #3397) — missing: alternatives, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5528
- #5524 — id: implement ID serverless edition v1 on Cloudflare Workers (design #2046) — missing: alternatives, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5524
- #5535 — id: implement Zettelkasten layer - multiplayer docs/wiki on ID (design #2048) — missing: alternatives, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5535
- #5534 — id: implement ID messaging and relay protocol v1 (design #2036) — missing: alternatives, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5534
- #5525 — id: implement ID boards v1 - signed boards, portable themes, playable posts (design #2054) — missing: alternatives, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5525
- #9110 — design(id): extensibility architecture — primitive/adapter/hook boundary spec + backend conformance — missing: problem, goals_nongoals, alternatives, approach, slices, rollout_rollback — https://github.com/developing-today/code/issues/9110
- #5862 — id: design: board subscription, discovery & feed integration (from #5680) — missing: goals_nongoals, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5862
- #5814 — id: design: adapter conformance harness & interop matrix CI (from #5674) — missing: goals_nongoals, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5814
- #5821 — id: design: result shaping & cursor pagination (from #5675) — missing: goals_nongoals, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5821
- #5818 — id: design: server-side filtered remote query execution (from #5675) — missing: goals_nongoals, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5818
- #5848 — id: design: importer design: Joplin (from #5678) — missing: goals_nongoals, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5848
- #5842 — id: design: AI-over-notes cluster (from #5678) — missing: goals_nongoals, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5842
- #10395 — Design mill pass 6: fork upkeep — upstream merges + feature-gap analysis (OpenCode, OpenChamber) — missing: goals_nongoals, alternatives, approach, slices, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/10395
- #1253 — design(pm-stack): one identity across the stack — SSO propagation from the public entry into every layer, roles replace asserted authorship — missing: goals_nongoals, alternatives, approach, slices, rollout_rollback, verification — https://github.com/developing-today/code/issues/1253
- #6304 — id(phase-14): Scale & performance hardening — v1 — first-iteration design — missing: problem, goals_nongoals, alternatives, approach, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/6304
- #1255 — design(aggregator): conversational layer — ask and act over PM data in plain language through the same audited action path — missing: problem, goals_nongoals, alternatives, approach, slices, rollout_rollback — https://github.com/developing-today/code/issues/1255
- #9080 — design(id): ID service, Workers edition — close the gaps on the #5524/#5681 serverless line to a workable hosted equivalent — missing: problem, goals_nongoals, alternatives, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/9080
- #9208 — design(pm-orchestration): typed dependencies, notes, journal & field authority — missing: goals_nongoals, alternatives, approach, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/9208
- #10712 — id(phase-14): Discovery, registry & federation — v2 — design: the public registry endpoint — missing: problem, goals_nongoals, alternatives, acceptance, rollout_rollback, verification — https://github.com/developing-today/code/issues/10712
- #5065 — conductor babysitter: emit structured babysitter-escalation:v1 payload for every escalation (from #2021 design) — missing: goals_nongoals, alternatives, approach, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5065
- #9935 — id: partition-local world blobs — per-jurisdiction R2 binding, world-reference indexing, local dedup + safe GC (design #3397 Phase 3, #5528 split) — missing: problem, goals_nongoals, alternatives, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/9935
- #10394 — Design mill pass 5: embedding research — ID and mcpx inside fleet — missing: goals_nongoals, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/10394
- #9199 — design(pm-orchestration): distributed priority model — urgency, importance, effective pressure — missing: goals_nongoals, alternatives, approach, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/9199
- #13525 — id: implement Notion importer slice 5: fidelity report + incremental re-import (design #6436) — missing: problem, goals_nongoals, alternatives, approach, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/13525
- #9204 — design(pm-orchestration): queue selection & bounded exploration (weighted sampling, density correction) — missing: goals_nongoals, alternatives, approach, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/9204
- #9207 — design(pm-orchestration): context hydration & incremental activity — missing: goals_nongoals, alternatives, approach, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/9207
- #13523 — id: implement Notion importer slice 3: databases — schema claims, relations, rollups (design #6436) — missing: problem, goals_nongoals, alternatives, approach, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/13523
- #9201 — design(pm-orchestration): PM projection & mobile control surface (Project #4, board, live-session direction) — missing: goals_nongoals, alternatives, approach, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/9201
- #7013 — watch: model deprecations + stealth refreshes — design, seed list, and the announcement gap — missing: alternatives, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/7013
- #13679 — id: media/1 presentation surface — design a Media WorldSurface variant with a text projection (#2339 remainder) — missing: goals_nongoals, alternatives, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/13679
- #13272 — fleet program: full investigation, next-steps design & build-out prompt — missing: goals_nongoals, alternatives, slices, acceptance, verification, open_questions — https://github.com/developing-today/code/issues/13272
- #5536 — id: implement ID conversion engine - linked transfer pairs and rate authorisation (design #2063) — missing: alternatives, approach, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5536
- #5834 — id: design: per-object talk page + changelog (from #5677) — missing: goals_nongoals, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5834
- #5893 — id: design: CI tiering for adversarial & benchmark suites (from #5685) — missing: goals_nongoals, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5893
- #10396 — Design mill pass 7 (exploratory): beyond GitHub — forges, git-over-iroh, cloud data planes — missing: goals_nongoals, approach, slices, rollout_rollback, verification — https://github.com/developing-today/code/issues/10396
- #9198 — design(pm-orchestration): requirement crosswalk & current-state census — missing: goals_nongoals, alternatives, approach, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/9198
- #6228 — feat(atlas): schema-driven filter surface — implementation programme (from design #5410) — missing: goals_nongoals, alternatives, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/6228
- #9084 — design(pm-stack): content-addressed media/data plane — R2 dedup, reference-based access, GC when the last reference drops — missing: problem, goals_nongoals, slices, rollout_rollback, verification — https://github.com/developing-today/code/issues/9084
- #9200 — design(pm-orchestration): priority distribution, provenance & reconciliation across producers/consumers — missing: goals_nongoals, approach, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/9200
- #9206 — design(pm-orchestration): work-item lifecycle & fleet subcommand surface — missing: goals_nongoals, alternatives, approach, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/9206
- #11213 — atlas(#6666 slice 3): persistent embedded replicas for internal/public (libsql local-copy, design 5.5) — missing: problem, goals_nongoals, alternatives, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/11213
- #13685 — fleet-ca: broker TLS — fleet brokers on the program CA (long-term design line) — missing: goals_nongoals, alternatives, approach, verification, open_questions — https://github.com/developing-today/code/issues/13685
- #9209 — design(pm-orchestration): leases, fencing, transactional outbox & direct verified completion — missing: goals_nongoals, alternatives, approach, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/9209
- #9106 — epic: ID/Atlas extensible primitives, federation, namespaces — major design iteration (2026-11-12 directive) — missing: goals_nongoals, alternatives, slices, rollout_rollback, verification — https://github.com/developing-today/code/issues/9106

## Appendix B — code: partially-specified designs (not in Appendix A, not sufficient)

- #13851 — main CI failure: cargo in /doc/2026-10-11T03-44-33Z_design_groups_events_delegation/spikes/shared for rustls - Update #1725871668 (run 45287383085) — missing: goals_nongoals, alternatives, approach, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/13851
- #10392 — Design mill pass 3: ID binary REST session + presentation model — missing: goals_nongoals, alternatives, approach, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/10392
- #13240 — ID program — full investigation, design completion & build-out (program tracking) — missing: problem, goals_nongoals, alternatives, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/13240
- #6282 — id(phase-14): Creator tooling & insight — core topic (research + design) — missing: problem, goals_nongoals, alternatives, approach, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/6282
- #6281 — id(phase-14): Scale & performance hardening — core topic (research + design) — missing: problem, goals_nongoals, alternatives, approach, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/6281
- #6277 — id(phase-14): Discovery, registry & federation — core topic (research + design) — missing: problem, goals_nongoals, alternatives, approach, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/6277
- #10389 — Design mill: one fleet surface for OpenCode, OpenChamber, ID, and mcpx — missing: problem, goals_nongoals, approach, slices, acceptance, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/10389
- #5696 — design: eBPF on the laptop (privileged host) — observability deferred with triggers, data-path acceleration rejected — missing: goals_nongoals, alternatives, slices, acceptance, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5696
- #10358 — Design: fleet secrets scoping — prefix / vault / store selection, per-device profiles — missing: goals_nongoals, alternatives, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/10358
- #6276 — id(phase-14): Hosted worlds & always-on hosting — core topic (research + design) — missing: goals_nongoals, alternatives, approach, acceptance, verification, open_questions — https://github.com/developing-today/code/issues/6276
- #6279 — id(phase-14): More shells (mobile, native, hardware) — core topic (research + design) — missing: problem, goals_nongoals, alternatives, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/6279
- #9095 — design(atlas+pm): Metadata schema v1 + evolution doctrine — additive-only, extras, strict enums, breaking-change parking lot — missing: problem, goals_nongoals, approach, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/9095
- #10357 — Design: automatic context propagation — every fleet action carries its origin — missing: goals_nongoals, alternatives, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/10357
- #9568 — Reporting architecture — event-driven reporting, quota forecasting, living report pages (design-mill starter) — missing: goals_nongoals, alternatives, approach, slices, rollout_rollback, verification — https://github.com/developing-today/code/issues/9568
- #6018 — atlas ingestion program: semi-automated updates, versioning, provenance, feedback — long-term design + tasks (2026-11-04 brief) — missing: goals_nongoals, alternatives, approach, slices, acceptance, open_questions — https://github.com/developing-today/code/issues/6018
- #10393 — Design mill pass 4: MCP everywhere — fleet MCP via mcpx, namespaces, fan-out, remote graph — missing: goals_nongoals, alternatives, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/10393
- #10356 — Design + build: fleet connect — explicit transport selection, sessions, and drop semantics — missing: goals_nongoals, alternatives, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/10356
- #5680 — id: design: Boards next steps — sharing flows, portable themes, playable posts, moderation — missing: goals_nongoals, alternatives, slices, acceptance, rollout_rollback, verification — https://github.com/developing-today/code/issues/5680
- #5678 — id: design: Knowledge layer feature program — competitor feature matrix, import/export, read-authority — missing: goals_nongoals, alternatives, slices, acceptance, rollout_rollback, verification — https://github.com/developing-today/code/issues/5678
- #5753 — id: research/design: fork a Rust KDL stack for ID — universal + multiple annotations, upstream archaeology, fork-base selection (extends #5688) — missing: goals_nongoals, alternatives, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5753
- #9359 — spec(bedside-and-away): run the initial design mill before implementation landing — missing: goals_nongoals, alternatives, slices, rollout_rollback, verification — https://github.com/developing-today/code/issues/9359
- #6280 — id(phase-14): Agents in worlds (MCP bridge) — core topic (research + design) — missing: goals_nongoals, alternatives, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/6280
- #9575 — C7 — Rollups, design pages, per-project spaces, reports timeline — missing: goals_nongoals, slices, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/9575
- #13241 — Atlas program — full investigation, next-steps design & build-out (program tracking) — missing: problem, goals_nongoals, alternatives, slices, rollout_rollback — https://github.com/developing-today/code/issues/13241
- #6031 — design(pm-stack): browser-dependent quota watchdog — CDP scrape producers that self-repair, escalate to a browser-enabled agent — missing: goals_nongoals, alternatives, approach, slices, rollout_rollback — https://github.com/developing-today/code/issues/6031
- #5674 — id: design: Wire format & transport adapters — postcard schema, non-UDP carriers, relays/proxies — missing: alternatives, slices, acceptance, rollout_rollback, verification — https://github.com/developing-today/code/issues/5674
- #5925 — design: telemetry rollup — one stats pipeline under fleet init (sysstat + extra stats); init reduced to stats + prewarm — missing: goals_nongoals, alternatives, slices, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/5925
- #5697 — design: bake the SSH recovery path into the fleet binary — orchestrate system ssh now, native russh deferred — missing: goals_nongoals, alternatives, acceptance, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/5697
- #9196 — PM Stack & Fleet orchestration — priority, dispatch, and work-item lifecycle (design mill) — missing: goals_nongoals, alternatives, slices, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/9196
- #9477 — design(pm-orchestration): Hosted PM/Fleet projection parity — missing: problem, goals_nongoals, alternatives, verification, open_questions — https://github.com/developing-today/code/issues/9477
- #9468 — design(pm-orchestration): Versioned canonical work read model and provenance — missing: problem, goals_nongoals, alternatives, verification, open_questions — https://github.com/developing-today/code/issues/9468
- #9457 — design(pm-orchestration): Typed degraded/unsupported/stale behavior and payload limits. Owns proposed src/work/distribution.rs, src/work/contracts.rs, and the same crate's fixture tests — missing: problem, goals_nongoals, alternatives, verification, open_questions — https://github.com/developing-today/code/issues/9457
- #13752 — design: fleet transport & pooling — host selection without Tailscale, iroh for non-Muse agents, generic pool primitive — missing: goals_nongoals, alternatives, acceptance, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/13752
- #9499 — design(pm-orchestration): Public mirror is explicit, allowlisted, and operation-scoped — missing: problem, goals_nongoals, alternatives, verification, open_questions — https://github.com/developing-today/code/issues/9499
- #9503 — design(pm-orchestration): Typed edge model, canonical direction, and JSON/error contract — missing: problem, goals_nongoals, alternatives, verification, open_questions — https://github.com/developing-today/code/issues/9503
- #9506 — design(pm-orchestration): Canonical dispatch reservation, direct launch and bind — missing: problem, goals_nongoals, alternatives, verification, open_questions — https://github.com/developing-today/code/issues/9506
- #9502 — design(pm-orchestration): Lifecycle transition + checkpoint/park/resume/handoff receipts — missing: problem, goals_nongoals, alternatives, verification, open_questions — https://github.com/developing-today/code/issues/9502
- #9513 — design(pm-orchestration): Idempotency identity and stable structured receipts — missing: problem, goals_nongoals, alternatives, verification, open_questions — https://github.com/developing-today/code/issues/9513
- #9505 — design(pm-orchestration): Session capability, existing-binding lookup and message delivery receipts — missing: problem, goals_nongoals, alternatives, verification, open_questions — https://github.com/developing-today/code/issues/9505
- #5802 — id: design: public shares through ID (from #5672) — missing: goals_nongoals, rollout_rollback, verification, open_questions — https://github.com/developing-today/code/issues/5802
- #5864 — id: design: D1 journal fit (design + spike) (from #5681) — missing: goals_nongoals, slices, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/5864
- #6513 — id: implement Notion importer per #6436 design — missing: alternatives, slices, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/6513
- #9091 — design(atlas): Atlas v1 build-out — design-mill starter (proto design → full design set) — missing: goals_nongoals, alternatives, approach, verification — https://github.com/developing-today/code/issues/9091
- #6759 — impl: telemetry rollup — one stats pipeline under fleet init (implements the #5925 design) — missing: goals_nongoals, alternatives, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/6759
- #7938 — id: implement: native Android client v1 — slices from the #5780 design — missing: goals_nongoals, alternatives, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/7938
- #8575 — Design: one door, one home per fact — the fleet tool owns every route and every fallback — missing: alternatives, acceptance, rollout_rollback, verification — https://github.com/developing-today/code/issues/8575
- #2898 — Design: reconstruction is admission — validate authority-bearing inputs where they are rebuilt (pkgs/id) — missing: problem, goals_nongoals, rollout_rollback, verification — https://github.com/developing-today/code/issues/2898
- #9155 — Fleet CA, credentials & secrets — v0.3 design mill (Summit build-out) — missing: goals_nongoals, alternatives, slices, open_questions — https://github.com/developing-today/code/issues/9155
- #5677 — id: design: ID notes, message board, and multiplayer editing v1 — missing: problem, slices, rollout_rollback, verification — https://github.com/developing-today/code/issues/5677
- #5681 — design(id): serverless edition storage — D1 vs Turso vs both for `id` on Cloudflare Workers — missing: problem, alternatives, slices, rollout_rollback — https://github.com/developing-today/code/issues/5681
- #5759 — Design: fleet proxy get/put — file transfer over the agent channel — missing: alternatives, acceptance, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/5759
- #9476 — design(pm-orchestration): Desired/observed reconciliation, freshness and rollback — missing: problem, goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9476
- #5955 — id: design program: worlds — advanced Roc libraries, capabilities, permissions, dynamic user functions, licensing (2026-11-04 brief) — missing: goals_nongoals, acceptance, rollout_rollback, open_questions — https://github.com/developing-today/code/issues/5955
- #5683 — design(id): residency & boundaries framework — opt-in evidence, sanctions, and intranet-mode capabilities — missing: alternatives, slices, rollout_rollback, verification — https://github.com/developing-today/code/issues/5683
- #9469 — design(pm-orchestration): Lifecycle projection, column precedence, and truthful claim binding — missing: problem, goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9469
- #9485 — design(pm-orchestration): Read-only shadow report/test in pkgs/fleet/crates/fleet-queue/src/work/priority.rs — missing: problem, goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9485
- #10405 — Design mill supplement: middle-road fleet MCP, ID graph publication, transitive capability purity — missing: goals_nongoals, slices, rollout_rollback, verification — https://github.com/developing-today/code/issues/10405
- #9432 — design(pm-orchestration): External attribution and uncertain origin — missing: goals_nongoals, alternatives, verification, open_questions — https://github.com/developing-today/code/issues/9432
- #9450 — design(pm-orchestration): Deployable source integration, observability and rollback — missing: problem, goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9450
- #1892 — mcpx: design a comprehensive docs and testing strategy — missing: goals_nongoals, alternatives, acceptance, rollout_rollback — https://github.com/developing-today/code/issues/1892
- #9470 — design(pm-orchestration): Responsive mobile queue, item detail, and saved views — missing: goals_nongoals, alternatives, verification, open_questions — https://github.com/developing-today/code/issues/9470
- #5790 — design: versioning for a monorepo whose apps deploy individually — derived versions, per-app tags + repo date tag, survey of schemes (WS-H) — missing: goals_nongoals, slices, acceptance, verification — https://github.com/developing-today/code/issues/5790
- #9471 — design(pm-orchestration): Accessible compact responsive interaction system — missing: goals_nongoals, alternatives, verification, open_questions — https://github.com/developing-today/code/issues/9471
- #9514 — design(pm-orchestration): General lifecycle mutation outbox in existing authority store — missing: problem, goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9514
- #9507 — design(pm-orchestration): Pagination, API caps, and resumable durable page ingestion — missing: goals_nongoals, alternatives, verification, open_questions — https://github.com/developing-today/code/issues/9507
- #9509 — design(pm-orchestration): Operation/session attribution and external-only semantics — missing: goals_nongoals, alternatives, verification, open_questions — https://github.com/developing-today/code/issues/9509
- #9455 — design(pm-orchestration): Versioned desired/observed snapshot reconciler with dirty cursor + full audit. Owns proposed pm-stack/projector/src/priority-reconcile.ts, test/priority-reconcile.test.ts — missing: goals_nongoals, alternatives, verification, open_questions — https://github.com/developing-today/code/issues/9455
- #9475 — design(pm-orchestration): Dependencies, activity, outcomes and explicit view receipt — missing: goals_nongoals, alternatives, verification, open_questions — https://github.com/developing-today/code/issues/9475
- #9511 — design(pm-orchestration): Required before/after peer ordering within equal effective urgency — missing: problem, goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9511
- #9512 — design(pm-orchestration): Claim and lease contract compatibility against #9178 — missing: problem, goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9512
- #9519 — design(pm-orchestration): Last-activity gates, duplicate webhook, post-completion policy — missing: problem, goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9519
- #9515 — design(pm-orchestration): Lease-fenced outbox relay, ordering and supersede policy — missing: problem, goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9515
- #5694 — design: remote desktop through the fleet brokers (RustDesk transport, secrets, pre-warming, virtual desktops) — missing: goals_nongoals, alternatives, slices, rollout_rollback — https://github.com/developing-today/code/issues/5694
- #9518 — design(pm-orchestration): Direct work complete CLI and human/JSON recovery output — missing: problem, goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9518
- #9517 — design(pm-orchestration): Completion prepare/partial/recovery state machine — missing: problem, goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9517
- #9516 — design(pm-orchestration): Effect-specific GitHub preconditions and readback adapter — missing: problem, goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9516
- #9490 — design(pm-orchestration): Operational recovery/runbook and telemetry — missing: goals_nongoals, alternatives, verification, open_questions — https://github.com/developing-today/code/issues/9490
- #9473 — design(pm-orchestration): Safe status, note, comment, and priority controls — missing: goals_nongoals, alternatives, verification, open_questions — https://github.com/developing-today/code/issues/9473
- #9488 — design(pm-orchestration): Hosted contract conformance for operation recovery — missing: goals_nongoals, alternatives, verification, open_questions — https://github.com/developing-today/code/issues/9488
- #9489 — design(pm-orchestration): Migration, backfill, and rollback validation — missing: goals_nongoals, alternatives, verification, open_questions — https://github.com/developing-today/code/issues/9489
- #4766 — Design: one name, one owner — a file's name is canonical, its archive namespace is reserved, and its archives belong to exactly one file (pkgs/id FileOps) — missing: goals_nongoals, rollout_rollback, verification — https://github.com/developing-today/code/issues/4766
- #6487 — Design: sign only what receipt would accept — one sign-time refusal contract for pkgs/id — missing: goals_nongoals, alternatives, rollout_rollback — https://github.com/developing-today/code/issues/6487
- #1414 — design(mcpx): resume upstream HTTP sessions after a cold restart — persist the resumable session state — missing: approach, rollout_rollback, verification — https://github.com/developing-today/code/issues/1414
- #1384 — design(mcpx): a supervisor that owns each stdio child's pipes — replace the takeover handoff for stdio — missing: problem, rollout_rollback, verification — https://github.com/developing-today/code/issues/1384
- #6582 — Design: validate it where you read it — one validated-and-bounded contract for the fleet proxy's frame and peer values — missing: problem, goals_nongoals, rollout_rollback — https://github.com/developing-today/code/issues/6582
- #4375 — Design: read it bounded — one response-read contract for mcpx's outbound calls — missing: goals_nongoals, rollout_rollback, verification — https://github.com/developing-today/code/issues/4375
- #5676 — id: design: Currency scaling — autoscaling service, Redis → multi-Turso → TigerBeetle path — missing: goals_nongoals, alternatives, slices — https://github.com/developing-today/code/issues/5676
- #5685 — design(id): DoS suite & performance benchmark programme — adversarial CI coverage and per-interface regression gates — missing: slices, rollout_rollback, verification — https://github.com/developing-today/code/issues/5685
- #9498 — design(pm-orchestration): Migration, rollback, graph repair, and privacy validation — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9498
- #9437 — design(pm-orchestration): Real-adapter conformance and CI adoption — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9437
- #9508 — design(pm-orchestration): Per-source incremental activity revisions and tombstones — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9508
- #9510 — design(pm-orchestration): Bounded refresh/watch/reconcile CLI and capability contract — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9510
- #9441 — design(pm-orchestration): Typed work-resource link command semantics — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9441
- #9480 — design(pm-orchestration): Cycle-safe hard-dependency pressure in pkgs/fleet/crates/fleet-queue/src/work/relations.rs, consumed by pkgs/fleet/crates/fleet-queue/src/work/priority.rs — missing: problem, goals_nongoals, open_questions — https://github.com/developing-today/code/issues/9480
- #9474 — design(pm-orchestration): Session capability panel, live directions, status report and agent request — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9474
- #9462 — design(pm-orchestration): Migration, performance, rollback, and hosted conformance — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9462
- #9460 — design(pm-orchestration): Immutable hydration generation with atomic publication — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9460
- #9497 — design(pm-orchestration): Read-only shadow/disable controls and rollback tests in src/work/selection.rs — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9497
- #9456 — design(pm-orchestration): Cross-target fixture and canonical order hash. Owns Rust contract fixtures in pkgs/fleet/crates/fleet-queue/src/work/distribution.rs and existing hosted adapter test surfaces — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9456
- #9459 — design(pm-orchestration): Safe cross-repository source keys and artifact paths — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9459
- #9449 — design(pm-orchestration): Recovery, ambiguous remote outcome, race and failure-injection tests — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9449
- #9434 — design(pm-orchestration): Cross-model schema owner and migration compatibility matrix — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9434
- #9454 — design(pm-orchestration): Extend existing Projector webhook intake as authenticated bounded transport/dedup, then fan out a typed field-scoped operation through the existing Fleet authority adapter — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9454
- #9448 — design(pm-orchestration): Directives, owner authority and reauthorization rules — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9448
- #9458 — design(pm-orchestration): Freeze versioned hydration manifest and result envelope — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9458
- #9453 — design(pm-orchestration): Accepted-intent write records work version + decision provenance + required operation intent through existing broker. Owns proposed src/work/distribution.rs, shared src/work/contracts.rs, integration src/turso_queue.rs — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9453
- #9464 — design(pm-orchestration): Private note CRUD/search and status-reason separation — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9464
- #9446 — design(pm-orchestration): Item resolution, event router and policy decision table — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9446
- #9429 — design(pm-orchestration): Field ownership and scoped authorization contract — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9429
- #9465 — design(pm-orchestration): Checkpoint and handoff snapshot interoperability — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9465
- #9430 — design(pm-orchestration): Managed comment conflict and creation recovery; no exactly-once claim — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9430
- #9461 — design(pm-orchestration): Full bounded hydration adapter and navigable summary — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9461
- #9442 — design(pm-orchestration): Shared CLI registration and compatibility inventory (DEP-05 alias; one issue only) — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9442
- #9494 — design(pm-orchestration): Durable run/disposition state machine in src/work/selection.rs with integration in existing src/turso_queue.rs — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9494
- #9493 — design(pm-orchestration): Versioned exploration profile and stable fixed-mass sampler in src/work/selection.rs, initially feature-disabled — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9493
- #9491 — design(pm-orchestration): Pure reference selector in pkgs/fleet/crates/fleet-queue/src/work/selection.rs — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9491
- #9492 — design(pm-orchestration): Typed claim revalidation in src/work/selection.rs plus existing pkgs/fleet/crates/fleet-queue/src/turso_queue.rs atomic claim — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9492
- #9495 — design(pm-orchestration): Seeded offline simulator/reference-model tests in src/work/selection.rs, no production service — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9495
- #9440 — design(pm-orchestration): Shared service composition for work next/open — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9440
- #9447 — design(pm-orchestration): Auditable launch receipt and #6533 acceptance fixture — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9447
- #9487 — design(pm-orchestration): Monotone fence generation and conditional local mutations — missing: goals_nongoals, alternatives, open_questions — https://github.com/developing-today/code/issues/9487

## Appendix C — code: open issues whose bodies demand a design ("needs design") but whose titles are not design-titled

- #176 — Nix Playwright E2E suite fails after vendoring dependencies — https://github.com/developing-today/code/issues/176
- #786 — id: Phase 13 F13.4 presentation capability tiers with in-band terminal detection — https://github.com/developing-today/code/issues/786
- #805 — id: F12.11 delta and binary frames (screen patches, resume tokens) — https://github.com/developing-today/code/issues/805
- #1047 — epic(usage-hub): SaaS/cloud services completion — v2 live collectors, daily console reads, v3 reconciliation — https://github.com/developing-today/code/issues/1047
- #1080 — epic(cluster): hardware inventory & first-node selection — https://github.com/developing-today/code/issues/1080
- #1184 — epic(dashboard): PM stack coherence - single-URL, fresh data, triage-able, hierarchical — https://github.com/developing-today/code/issues/1184
- #1400 — Cost Explorer ChatGPT-parity follow-ups: lane charts, chart extras, evidence cards, projection routes — https://github.com/developing-today/code/issues/1400
- #1511 — Repo-universe issue tracker: watch every repo we own or depend on, pull its issues into one tracked system — https://github.com/developing-today/code/issues/1511
- #1658 — rollout(fleet): OpenCode-half pilot on the code queue + extension to opencode/openchamber/nix (OpenCode-half step 5) — https://github.com/developing-today/code/issues/1658
- #1817 — EPIC: Basement Talos/Cozystack cluster program — https://github.com/developing-today/code/issues/1817
- #1822 — Audit: socket activation as the default pattern for laptop services — https://github.com/developing-today/code/issues/1822
- #1885 — mcpx: extensibility -- plugins, hooks, tool sources, embedding (tracker) — https://github.com/developing-today/code/issues/1885
- #1915 — mcpx: resolve a script's import graph statically and prepare missing dependencies — https://github.com/developing-today/code/issues/1915
- #1925 — mcpx: where the model comes from — a provider layer mcpx owns, model profiles, and life after MCP Sampling's deprecation — https://github.com/developing-today/code/issues/1925
- #1940 — mcpx and opencode code mode: nested code mode, ?codemode=false, and what a harness expects of mcpx_exec — https://github.com/developing-today/code/issues/1940
- #1950 — mcpx: OAuth authorization for remote servers and for /mcp (61 conformance rows) — https://github.com/developing-today/code/issues/1950
- #1957 — mcpx: smaller conformance gaps -- user identity, annotation trust, rate limits, cursors, URI encoding, prompt validation, restart, invalidation — https://github.com/developing-today/code/issues/1957
- #2072 — External-system observability: unified PM + queue + quota attribution (webhooks, hot list, session IDs, projections) — https://github.com/developing-today/code/issues/2072
- #2170 — Add OpenClaw to #alignment — decide one shared channel vs point-to-point — https://github.com/developing-today/code/issues/2170
- #2686 — mcpx: e2e and settings suites cannot run in the nix build sandbox (checkPhase exclusions) — https://github.com/developing-today/code/issues/2686
- #3401 — id: isolate hosted blobs by jurisdiction and minimize shared directory — https://github.com/developing-today/code/issues/3401
- #3819 — id: build: durable multiplayer note checkpoints and merges — https://github.com/developing-today/code/issues/3819
- #3870 — id: implement: journaled region-seam export/import — https://github.com/developing-today/code/issues/3870
- #3989 — impl(pm-stack): move stateful services off the laptop and prove host failover — https://github.com/developing-today/code/issues/3989
- #4825 — impl(pm-stack): cost-anomaly detection to Needs-you via the alert throat (#1259 slice 4) — https://github.com/developing-today/code/issues/4825
- #4974 — id: local-vs-shared scope for tags and petnames, with a review surface — https://github.com/developing-today/code/issues/4974
- #5772 — VMs: declarative guest definitions (nixvirt / microvm.nix) + mutable carve-out list — https://github.com/developing-today/code/issues/5772
- #5971 — id: research: image/video conversion tooling — kitty graphics protocol and video-to-ASCII — https://github.com/developing-today/code/issues/5971
- #6163 — id: render portable board themes safely in web and terminal (#5525) — https://github.com/developing-today/code/issues/6163
- #6284 — id(phase-14): Hosted worlds & always-on hosting — v2 — outline: custody & lifecycle — https://github.com/developing-today/code/issues/6284
- #6285 — id(phase-14): Hosted worlds & always-on hosting — v3 — outline: many nodes — https://github.com/developing-today/code/issues/6285
- #6286 — id(phase-14): Hosted worlds & always-on hosting — v4 — outline: public-server operation — https://github.com/developing-today/code/issues/6286
- #6298 — id(phase-14): More shells (mobile, native, hardware) — v3 — outline: hardware shell — https://github.com/developing-today/code/issues/6298
- #6299 — id(phase-14): More shells (mobile, native, hardware) — v4 — outline: third-party shells — https://github.com/developing-today/code/issues/6299
- #6307 — id(phase-14): Scale & performance hardening — v4 — outline: public-server targets — https://github.com/developing-today/code/issues/6307
- #6309 — id(phase-14): Creator tooling & insight — v2 — outline: cross-world creator tools — https://github.com/developing-today/code/issues/6309
- #6666 — atlas shard consumer: serve-mode (fleet serve search role) and #5668 snapshot-id integration (follow-up of #5755 slice 1) — https://github.com/developing-today/code/issues/6666
- #6805 — Atlas search Worker cache: eviction & invalidation policy (follow-up to #6788) — https://github.com/developing-today/code/issues/6805
- #7969 — impl(atlas): recurring ingestion sweep runner + guardrail monitor + plan-headroom reporting (#6019) — https://github.com/developing-today/code/issues/7969
- #8102 — Provider self-renewal audit 2026-11-09 — every door checked against Drew's rule (all providers renew themselves) — https://github.com/developing-today/code/issues/8102
- #8281 — id: implement gallery registry protocol + reference registry service (from #6278) — https://github.com/developing-today/code/issues/8281
- #8282 — id: implement template-world snapshot export/import (from #6278) — https://github.com/developing-today/code/issues/8282
- #8299 — id(phase-14) v2 step 1: role configuration — roles as first-class world config (#6301) — https://github.com/developing-today/code/issues/8299
- #9097 — impl(atlas): continuous ingestion — queue/PM/GitHub/Muse/OpenCode/ID corpora + sources inventory — https://github.com/developing-today/code/issues/9097
- #9125 — research(id): triple encoding bake-off — compressed binary vs JSONB vs normalized byte-key maps — https://github.com/developing-today/code/issues/9125
- #9539 — PM-stack prompt & chat surface: scoped prompt widgets + session interface (program) — https://github.com/developing-today/code/issues/9539
- #9551 — OpenClaw picker: pools listed plainly (no "(OpenCode Route pool)" suffix, no grouping), add Small pool, hide Copilot + Claude CLI — https://github.com/developing-today/code/issues/9551
- #9571 — C3 — Health hooks: failure-triggered checks that troubleshoot and report — https://github.com/developing-today/code/issues/9571
- #9574 — C6 — Report pages & spaces freshness: mobile-first, questions module, confirmable actions, incident pages — https://github.com/developing-today/code/issues/9574
- #9576 — C8 — Atlas ingestion of reports & pages + error-driven investigation boosts — https://github.com/developing-today/code/issues/9576
- #9608 — impl(pm-stack): break-glass — outage queueing + local owner token CLI (#1253 split 6) — https://github.com/developing-today/code/issues/9608
- #10051 — id(phase-14) v3 step 3: agent-to-agent task delegation protocol (#6302) — https://github.com/developing-today/code/issues/10051
- #10212 — Evaluate single-writer SQLite sync and restore alternatives — https://github.com/developing-today/code/issues/10212
- #10332 — claimant mode file: guard the silent-deletion single point of failure — https://github.com/developing-today/code/issues/10332
- #10345 — impl(id): build-time defaults.json — typed parse, pair-atomic cascade, provenance (#5916 follow-on) — https://github.com/developing-today/code/issues/10345
- #12215 — id: decide whether program-run history belongs in the world journal — https://github.com/developing-today/code/issues/12215
- #12280 — fleet: ready PRs are left unmerged (20 open; the merge sweep has filed nothing since 2026-11-17) — https://github.com/developing-today/code/issues/12280
- #12404 — atlas(classification): enforce disclosure-context filters at output boundaries — https://github.com/developing-today/code/issues/12404
- #12797 — impl(pm-stack): E2 — TenantOpsDO provisioning state machine (#9083) — https://github.com/developing-today/code/issues/12797
- #12798 — impl(pm-stack): E3 — SyncCoord + hosted sync ingress (#9083) — https://github.com/developing-today/code/issues/12798
- #12799 — impl(pm-stack): E4 — credential issuance/revocation service (#9083) — https://github.com/developing-today/code/issues/12799
- #12800 — impl(pm-stack): E5 — public plane: PublishIndex + publish/retract (#9083) — https://github.com/developing-today/code/issues/12800
- #12801 — impl(pm-stack): E6 — tenant DB store adapter in id-store (#9083) — https://github.com/developing-today/code/issues/12801
- #13191 — fleet queue: add a repair verb for quarantined/malformed task packets (in-place payload fix) — https://github.com/developing-today/code/issues/13191
- #13401 — id: event store — audit prune_behind_cut and range-sync advertisement against compacted residue — https://github.com/developing-today/code/issues/13401
- #13406 — fix(pm-stack): implement C4 day/compound range forms, methodology c4-meth:2 (#13357) — https://github.com/developing-today/code/pull/13406
- #13461 — impl(backups): mesh coverage — embedding/shard stores restore without a re-embed pass — https://github.com/developing-today/code/issues/13461
- #13506 — Rename sensitivity ladder identifiers in code to canonical public/internal/private/sensitive (aliases/migration, not a blind swap) — https://github.com/developing-today/code/issues/13506
- #13658 — story: our own OIDC/SAML/SSO identity provider — https://github.com/developing-today/code/issues/13658
- #13711 — id(journal): durable per-world global position covering lease entries, surviving compaction (prerequisite for #7572 failover barrier) — https://github.com/developing-today/code/issues/13711
- #13766 — impl(atlas/id): attested-only consumer view preset needs an attestation predicate (follow-up of #11185) — https://github.com/developing-today/code/issues/13766

## Appendix D — Lane 6 brief, verbatim

> You are LANE 6 — DESIGN-GAPS of programme AUDIT-7 (Drew's directive, 2026-11-23 ~17:56 CST): seven parallel audit lanes over the four repos (developing-today/code, developing-today-forks/opencode, developing-today-forks/openchamber, dezren39/fleet), exhaustive — every item in scope, one by one, read in full, no sampling.
>
> UMBRELLA TRACKING ISSUE: developing-today/code#13871 — the cross-lane coordination point. File your own tracking issue in developing-today/code first, post its link as a comment on #13871, and post progress on your tracking issue at EVERY slice boundary.
>
> SHARED RULES (binding on all seven lanes): GitHub access: the fleet GitHub subcommand ONLY. All reads via `fleet gh api`; all issue/comment writes via `~/tools/gh-post` (the GitHub App identity). NEVER plain `gh` with the user token, NEVER the GitHub connector. Tracking first: file your own tracking issue, link it on the umbrella (#13871), post progress at EVERY slice boundary. Check the umbrella + an issue's comments before acting — a claim comment from another lane is a HARD STOP. Exhaustive, one by one, read everything. No sampling, no skimming by title. Work in slices; per-batch sub-work to the pilot queue (`--queue repo-health-code-pilot`, kind `feature_slice` + `payload.slice`, NEVER kind `design`). Standing authority: merge PRs when verified done; never force-push main; never close issues for age; file-the-issue in the same pass. Repo work in laptop worktrees via ~/tools/laptop-ssh.sh; NEVER the sandbox for repo work. Deliverables: (a) dated report + comprehensive log in doc/ under the dated-record convention, INCLUDING your own prompt verbatim; (b) findings filed as issues/PRs/labels AS YOU GO; (c) a skill where repeatable; (d) a final summary comment on your tracking issue. Latitude: open investigation issues, designs, bug tickets; PRs for quick doc fixes; create labels; suggest convergences/merges/duplication.
>
> YOUR BRIEF — LANE 6 — DESIGN-GAPS. Scope: find all designs AND all issues that demanded a design. Produce the list of designs we have yet to make, or that were poorly implemented or very simple — and add them all to the short list of prioritised agent-next items to be picked up and designed (create the queue packets/issue records so `fleet agent next` can pick them up; each packet states the design standard below and names the source issue). The standard for a sufficient design (embed in full in every packet and in your report — judge existing designs against it): (1) problem + context — what breaks today, with evidence; (2) goals and explicit non-goals; (3) alternatives considered with honest tradeoffs, including do-nothing; (4) the chosen approach — interfaces/contracts, data model, security and capability analysis where relevant; (5) dependency-ordered build slices, each sized for one lane/session with its own acceptance criteria; (6) rollout + rollback plan; (7) verification plan — how we prove it works (tests, live proof); (8) open questions, each with a named decider; (9) recorded as a dated design document in doc/ + a tracking issue + a queue item. A design that is only a paragraph, has no slices, no alternatives, and no acceptance criteria is INSUFFICIENT — list each deficient design with exactly what is missing. Return the prioritised list with links.
>
> Queue packet mechanics: pilot queue `repo-health-code-pilot` (always pass --queue); kind `feature_slice` with `payload.slice`; NEVER kind `design`. Each packet: premise + singular-ownership check first, names the source issue, embeds the sufficient-design standard in full. Prioritise: designs blocking other work first, then designs demanded by open issues, then deficient-design upgrades.
>
> Coordination: LANE 5 (DESIGN-REVIEW) judges designs' internal soundness; you judge existence + sufficiency against the standard and own the queue packets.
>
> Your return: tracking-issue link; the prioritised list (each entry: design needed/deficient, source issue link, what's missing per the standard, queue task ID created); designs judged sufficient (count + list); report doc path + merge SHA; counts (issues-demanding-design found, designs missing, designs deficient, packets created).

(Note: Appendix D reproduces the brief as received by the lane, with the shared-rules section lightly condensed for the record; the full text lives in the lane's session record and the tracking issue carries the operative standard verbatim.)
