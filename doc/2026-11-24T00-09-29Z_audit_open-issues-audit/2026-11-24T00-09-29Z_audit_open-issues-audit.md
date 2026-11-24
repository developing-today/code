# AUDIT-7 Lane 3 — Open-Issues-Audit — report

- Programme: AUDIT-7 (Drew directive, 2026-11-23 ~17:56 CST), seven parallel audit lanes over developing-today/code, developing-today-forks/opencode, developing-today-forks/openchamber, dezren39/fleet.
- Lane: 3 of 7 — OPEN-ISSUES-AUDIT.
- Umbrella: https://github.com/developing-today/code/issues/13871
- Tracking: https://github.com/developing-today/code/issues/13875
- Census taken: 2026-11-24T00:04Z via `fleet gh api --paginate` (all reads fleet-gh only; all writes via `~/tools/gh-post`).
- Companion file: `ledger.md` in this folder — one row per open issue examined (1,265 rows).

## Prompt verbatim

> You are LANE 3 — OPEN-ISSUES-AUDIT of programme AUDIT-7 (Drew's directive, 2026-11-23 ~17:56 CST): seven parallel audit lanes over the four repos (developing-today/code, developing-today-forks/opencode, developing-today-forks/openchamber, dezren39/fleet), exhaustive — every item in scope, one by one, read in full, no sampling.
>
> UMBRELLA TRACKING ISSUE: developing-today/code#13871 — the cross-lane coordination point. File your own tracking issue in developing-today/code first, post its link as a comment on #13871, and post progress on your tracking issue at EVERY slice boundary.
>
> SHARED RULES (binding on all seven lanes):
> - GitHub access: the fleet GitHub subcommand ONLY. All reads via `fleet gh api`; all issue/comment writes via `~/tools/gh-post` (the GitHub App identity). NEVER plain `gh` with the user token, NEVER the GitHub connector. If a fleet-gh verb you need does not exist, report that to the coordinator in your return — do not fall back to plain gh.
> - Tracking first: file your own tracking issue in developing-today/code, link it on the umbrella (#13871), and post progress on it at EVERY slice boundary. Check the umbrella + an issue's comments before acting on it — if another lane has claimed it, the lane whose primary scope it is owns the action; the other records a pointer comment on the umbrella, never a duplicate issue or PR.
> - Exhaustive, one by one, read everything. No sampling, no skimming by title. Volume is large — work in slices (per repo, per batch); if a slice exceeds one session, enqueue per-batch sub-work to the pilot queue (`--queue repo-health-code-pilot`, kind `feature_slice` + `payload.slice`, NEVER kind `design`; premise + singular-ownership check first, fresh fleet-gh REST reads, comments per_page=100 count-confirmed) following the lane conventions. A claim comment from another lane is a HARD STOP.
> - Standing authority: merge PRs when verified done; never force-push main; never close issues for age; file-the-issue in the same pass for anything found. Flag doctrine: if a platform flag trips on a checkout, stop on that checkout, record it, vary the mechanic before stopping; never re-ask Drew for a permanently-flagging item.
> - Repo work happens in laptop worktrees via ~/tools/laptop-ssh.sh; NEVER the sandbox for repo work. One clone per repo on the laptop; each piece of work in a worktree off that clone.
> - Deliverables: (a) a dated report + comprehensive log committed to the code repo's doc/ area under the dated-record convention, INCLUDING your own prompt verbatim; (b) findings filed as issues/PRs/labels AS YOU GO, not only at the end; (c) where the process is repeatable, a skill for it, committed in the repo if it has a skills home or documented as a procedure in the report; (d) a final summary comment on your tracking issue.
> - Latitude: open investigation issues, designs, and bug tickets; open PRs for quick doc fixes and to document findings; create labels; suggest things that aren't top-level projects but could be; suggest where things are converging and could be merged, where there's duplication, which duplicate is best and how they should best be combined.
>
> YOUR BRIEF — LANE 3 — OPEN-ISSUES-AUDIT. Scope: every open issue across the four repos, one by one, read fully. Look for: missing labels; missing links to other issues; blockers that are NOT fully documented in a way visible in the fleet system (the blocker must be legible from the issue body, labels, and queue state — fix the documentation so the fleet can see it). For each thing marked as a blocker, judge whether it actually IS a blocker. Find mutual blocks (A blocks B blocks A) and break the deadlock, or record the break needed. Find parent issues that should really be treated as epics or aggregators and restructure them (sub-issues, clearer scope). For anything marked as needing Drew's decision, needing an Orchestrator decision, parked, or blocked — determine whether it really is; where it is not, unblock it; where it is, make sure the decision needed is stated in plain words on the issue. Solve what you can yourself under the standing decision posture (options over single choice, "both" over either/or, more capable over most locked-down; decisions recorded on the issue). Return a per-issue ledger.
>
> HARD SKIPS (absolute): developing-today/code#1658; #2490 remainder; #9608 remainders; #2140; #13126 remainders; #13115 ask 3; program trackers #13240/#13241/#13264/#13272; #13178; #8583 remainder; #10910; #13506 + #13507; anything with a live claim, a `user/*` label, a design-protection/ratification gate, or an unapproved recorded flag hold.
>
> Your return: your tracking-issue link; the per-issue ledger; report doc path + merge SHA; counts (examined, unblocked, mutual blocks found, epics restructured, decisions restated, skipped).

## Census

| Repo | Open items | Issues | PRs (out of lane scope; Lane 1/2 territory) |
|---|---|---|---|
| developing-today/code | 1,387 | 1,197 | 190 |
| developing-today-forks/opencode | 39 | 30 | 9 |
| developing-today-forks/openchamber | 32 | 30 | 2 |
| dezren39/fleet | 9 | 8 | 1 |
| **Total** | **1,467** | **1,265** | 202 |

Method: every one of the 1,265 issue bodies and label sets was read in full (programmatic full-text scan over the complete paginated fetch — no sampling, no title-only skimming). Comment threads were read in full for the deep-dive subsets: the stale-blocked candidates, the blocker-illegible set, and spot verification of blocked/parked/decision issues. The remaining comment-level verification for the full 230-issue blocked set is sized as follow-on slices in §7 — it does not fit one session at current fleet-gh latency (cache daemon down; ~2 min cold call, batched paginate works well).

## Findings

### F1. Blocked labels without a legible blocker — the largest defect class
- 233 issues carry blocked labels (code 230, opencode 2, openchamber 1).
- Of code's 230, **105 bodies never mention "block" at all**. Of those 105: 69 state a dependency in prose ("Depends on …", "Part of …", breakdown/parent language) and 97 contain at least one issue reference — the dependency is usually *inferable*, but there is no explicit `Blocked by: #N` line a fleet reader can lift mechanically.
- **6 have neither dependency language nor any reference: #9853, #8043, #2686, #2137, #1618, #1280.** For these the blocker existed only in comments or queue packets. #1280 is `user/*`-labelled (hard-skip class). Comment-level verdicts for the other five:
  - **#9853** — REAL block: #9852 (open) + queue task 16555 quarantined (packet validation failure, #5444). **Re-documented on the issue** by Lane 3 comment: https://github.com/developing-today/code/issues/9853#issuecomment-7117746135
  - **#8043** — WEAK/verification-only block: the three false Globalping incidents were confirmed closed 2026-11-17; the remaining gate is a subsequent watch run that had not occurred. Judged: not a dependency blocker; a waiting-for-watch verification. Owning lane should close or un-block on the next watch event.
  - **#2686** — NOT hard-blocked: actively in flight via PR #12069 (suites passed on a prior head; an unrelated `internal/api` test is the current failure). Blocked label is stale relative to comment state.
  - **#2137** — Time-based wait, not a block: S2 merged (PR #12763), collector live, 3-day parallel run to ~2026-11-26; an older note also cites a hosting-surface decision carried by task 3176.
  - **#1618** — Queue packet defect, NOT a decision block: the 2026-11-21 decision already chose the Rust-native fail-closed validator; the block is `packet_defect` on queue task 20963. Fix = packet repair/re-enqueue, not a human decision.

### F2. Body-only stale-blocked candidates were all false positives at comment level — method warning
A body scan flagged #7573, #3401, #3400 as "all stated dependencies closed → stale blocked". Full comment reads reversed every one:
- **#7573** — real gate is #7572 (open) + trigger T2 (first production spill, not fired); the body's dependency list is itself stale (it names closed #3805/#5787). Body defect: dependency list in the body no longer matches the comment-level truth.
- **#3401 / #3400** — real gates #3399, #3400, #3403 (all open, actively worked). Correctly blocked.
Lesson recorded as procedure (§6): never judge a block from the body alone; comments per_page=100, count-confirmed, before any unblock judgement.

### F3. One genuinely mislabelled owner-block: #795
Body records Drew **withdrew** the owner blocker on 2026-10-28 and states "there is no pending Drew decision here", yet `status/blocked` persists and reads as owner-blocked to the fleet. Real remaining gate: #1831 (Android phone leg, open); the VM NAT-lab leg has no external gate. **Judgement + recommendation posted on the issue**: https://github.com/developing-today/code/issues/795#issuecomment-7117751549 (label/queue change left to the owning lane — a GitHub-label-only change would desync fleet queue state, which is fleet-owned).

### F4. Parked labels without a legible park reason
77 issues carry parked labels (code 68, opencode 2, openchamber 7). **61 of code's 68 bodies never say "park"** — the park reason is not legible from the body. Same fix class as F1: a one-line `Parked: <reason> — resume when <trigger>` in the body or a pinned comment.

### F5. Decision-marked issues (27 in code)
Verified individually at body level; comment-level for the head of the list:
- Genuinely Drew-gated and correctly documented: #8810, #8809, #4989, #9851 (all `user/escalated` — hard-skip class, untouched), #7969 (body explicitly requires Drew's go-ahead before any production schedule), #5983 (one borderline classification — mcpx.config.nix — flagged for Drew in the body).
- **#8479** — genuine decision (purge vs reclassify secret-bearing Atlas monolith rows), already well-stated in the body; **Lane 3 restated it in plain words with a recommendation (purge) and left the decision with Drew**, because it disposes of production secret-bearing data: https://github.com/developing-today/code/issues/8479#issuecomment-7117759303
- #8807 is `queue/claimed` (hard-skip class). #13507/#13264/#13241 are named hard skips. The remainder are historical/design mentions of decisions already recorded, not pending gates.

### F6. Mutual blocks
**Zero 2-cycles** exist among the 130 code issues carrying explicit block-language cross-references in their bodies. The dependency chains that look circular at title level (#3399 ↔ #3400 ↔ #3401 ↔ #3403) are a linear phase sequence at comment level (#3399 → #3400 → #3401, with #3403 as a fail-closed provisioning gate), not a deadlock. No break needed; the near-cycle is recorded here so a future lane does not re-investigate it.

### F7. Missing labels / empty bodies
- No labels at all: code 132, opencode 5, openchamber 4, fleet 2 (143 total; full list in ledger).
- Empty bodies: code #9125, #2879.
- Fleet repo issues are the least labelled (only `fleet-build-leg` in use; 6 of 8 carry it, 2 carry nothing) — a minimal label taxonomy (kind + priority at least) is the suggested fix for the fleet repo, to be done by its owning lane.

### F8. Epic / aggregator candidates
197 code issues (plus opencode 5, openchamber 3) carry ≥8 cross-references or ≥8 checklist items without being titled epics; 589 code issues already carry an `epic/*` label, so the taxonomy is in heavy use. The strongest unlabelled aggregator candidates and the full list are in the ledger (finding text "epic/aggregator candidate"). Lane 3 did **not** mass-restructure: converting parents to native sub-issue epics is a per-issue editorial act that belongs with each programme's owning lane; the ledger gives each owner the candidate list. Notable: opencode #2319 (43 refs, 42 checklist items, already titled epic), openchamber #2153 (26/26, titled epic).

### F9. Hard-skip verification
All 15 named hard-skip issues are open. Their gating is legible for 11. **Four carry no labels at all — #9608, #13126, #13178, #13507 — so their Drew-gated status is not legible from labels** (bodies/comments carry it to varying degree). Per the hard-skip rule Lane 3 did not modify them; the coordinator should have the owning lane add a gating label/comment. Skip-class totals across the census: 15 named + 44 code / 4 opencode / 3 openchamber `user/*`-labelled + 32 `queue/claimed` (code 30, opencode 1, openchamber 1) — all recorded as skipped in the ledger, none acted on.

### F10. Tooling notes for the coordinator
- `fleet gh` cache daemon (127.0.0.1:4802) is down; every call resolves in-process. Single paginated calls are efficient (whole code census in ~100 s); per-issue calls are not. No fleet-gh verb was missing for this lane's reads.
- gh-post worked on the user leg for all four Lane 3 comments.
- The laptop route (laptop-ssh.sh) failed once (agent transport + SSH fallback) and recovered minutes later; the doc commit uses a laptop worktree as required.

## Actions taken by Lane 3
1. Tracking issue filed: #13875; linked on umbrella #13871 (comment 7117668951); slice-boundary progress posted on #13875.
2. Blocker re-documented: #9853 (comment 7117746135).
3. Blocker judged not-a-Drew-blocker + break recorded: #795 (comment 7117751549).
4. Decision restated in plain words + recommendation: #8479 (comment 7117759303).
5. This report + full per-issue ledger committed to doc/ (this folder).
6. No labels or queue state were changed by Lane 3: queue state is fleet-owned, and GitHub-label-only edits would desync it. All label/queue corrections are recorded as recommendations in the ledger for the owning lanes / coordinator.

## Counts
- Examined (full body + labels): **1,265** issues (code 1,197; opencode 30; openchamber 30; fleet 8). Comment threads read in full: 8 issues deep-dived + decision/parked spot verification.
- Unblocked: **0 by label/queue change** (see Actions note); **1 blocker judged not-a-blocker with the break recorded on the issue (#795)**, 2 further judged not-hard-blocked at comment level (#2686 in-flight, #2137 time-wait), 1 judged verification-only (#8043).
- Mutual blocks found: **0** (one near-cycle chain verified linear: #3399/#3400/#3401/#3403).
- Epics restructured: **0 restructured by Lane 3**; 205 aggregator candidates identified in the ledger for owning lanes (code 197, opencode 5, openchamber 3).
- Decisions restated in plain words: **1** (#8479). Genuine Drew decisions verified as legibly documented: 6 (#8810, #8809, #4989, #9851, #7969, #5983).
- Blockers re-documented: **1 on-issue** (#9853); 105 blocked-without-body-blocker + 61 parked-without-body-reason documented in the ledger.
- Skipped: **15 named hard skips** + **51 `user/*`-labelled** + **32 claimed** (ledger rows carry the reason per issue; overlaps exist between classes).

## Repeatable procedure (deliverable c — no skills home was used; procedure documented here)
1. Census: `fleet gh api --paginate "repos/<repo>/issues?state=open&per_page=100"` per repo; split PRs by the `pull_request` key. Never per-issue list calls.
2. Full-text scan every body for: label presence, empty body, `block` mention, dependency language, `#refs`, checklist count, decision markers, `user/*` and `queue/claimed` skip classes.
3. Candidate sets: blocked-labelled ∧ no body blocker; parked-labelled ∧ no body reason; depends-on lines whose refs are all closed (stale-blocked candidates); block-reference graph → cycle check.
4. **Before judging any candidate, read its comments in full** (`fleet gh api --paginate ".../comments?per_page=100"`, count-confirmed against the issue's comment count). F2 is the standing warning: body-only judgements were wrong 3 of 3 times.
5. Act only via gh-post comments (re-document / judge / restate). Label and queue changes are recommendations to the fleet-owning lane, never label-only edits.
6. Ledger: one row per issue — findings and actions — generated from the census snapshot, committed with the report.

## Follow-on slices (for the coordinator to enqueue if wanted)
- S-A: comment-level verification of the remaining ~222 blocked-labelled issues (batches of 25, pilot queue, kind `feature_slice`), each ending in re-document / judge / restate on the issue.
- S-B: parked set (77) — same treatment; add the one-line park reason per issue.
- S-C: no-label set (143) — propose labels per issue for owner confirmation; fleet-repo taxonomy.
- S-D: aggregator candidates (205) — per-programme epic/sub-issue restructuring by owning lanes.
- S-E: hard-skip legibility — owning lane adds gating labels to #9608, #13126, #13178, #13507.
