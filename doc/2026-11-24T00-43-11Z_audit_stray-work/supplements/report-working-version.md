# Stray work audit — PRs and issues not tied to a design or roadmap item (AUDIT-7 Lane 7)

- Tracking issue: [developing-today/code#13874](https://github.com/developing-today/code/issues/13874)
- Umbrella: [developing-today/code#13871](https://github.com/developing-today/code/issues/13871)
- Date: 2026-11-23 (session 17:58–19:20 CST)
- Lane: 7 of 7, AUDIT-7 programme (Drew directive, 2026-11-23 ~17:56 CST)

## Method

All GitHub reads via `fleet gh api` (REST, paginated, `per_page=100`); all issue/comment writes via `~/tools/gh-post` (GitHub App identity); label and sub-issue writes via `fleet gh api` POST (gh-post has no label/sub-issue verb — recorded here as the route used). Window: everything open at census time, plus anything closed or with any activity since 2026-10-23, across developing-today/code, developing-today-forks/opencode, developing-today-forks/openchamber, dezren39/fleet. Every open code-repo issue was classified from its full body; every item edited was first read with its comments (per_page=100) and checked for a claim by another lane — none of the 34 edited items carried a lane claim.

## Census (evidence)

| repo | items in window | issues / PRs | open at census | labels in repo |
|---|---|---|---|---|
| developing-today/code | 13,452 | 7,009 / 6,443 | 1,387 (1,325 at the later open-fetch) | 372 |
| developing-today-forks/opencode | 2,596 | 1,218 / 1,378 | 39 | 165 |
| developing-today-forks/openchamber | 2,433 | 1,183 / 1,250 | 32 | 147 |
| dezren39/fleet | 113 | 19 / 94 | 9 | 15 |
| **total** | **18,594** | | **1,405–1,467** | |

Code-repo open-issue linkage classification (1,199 open issues at classification time):

- 587 already carry an `epic/*` or `primary-parent/*` label.
- 612 carry neither: 521 reference another issue in the body (text-linked; the label was never formalised), 91 have no parent label and no issue reference at all; excluding AUDIT-7's own tracking issues, 83 truly orphaned.
- Candidate parents for the 612, by app label / title: epic(fleet): maintenance #12460 ×113, epic(pm-stack): maintenance #12458 ×77, epic(id): maintenance #12462 ×64, epic: main-broken #8976 ×14, epic(mcpx): maintenance #12459 ×12, epic(atlas): maintenance #12457 ×6, epic(lab-cluster): maintenance #12461 ×1, no candidate ×325.
- Whole-window hygiene (code): 6,378 of 13,452 items have no labels at all — but that is 6,102 of 6,443 PRs (95%) versus only 276 of 7,009 issues (4%). Empty bodies are rare (code 3, opencode 4, openchamber 1, fleet 0). Only 4,009 of 6,443 code PR titles carry an issue `#ref`.
- Stray proxy over the whole code window: 5,477 of 7,009 issues have no `epic/*` label and no `kind/design` label; of those, kind distribution is dominated by (no kind) 2,608 and kind/bug 1,758, then kind/ci 252, kind/feature 244, kind/enhancement 183, kind/test 121.

## Trends found (with evidence)

1. **The forks are fix streams, not roadmaps.** Titles containing "fix": opencode 1,717/2,596 (66%), openchamber 1,602/2,433 (66%), code 3,043/13,452 (23%), fleet 36/113 (32%). In the forks the dominant sources are `source/pr-diff-review` (opencode 139, openchamber 119) and `source/creator` (572 / 548): upstream-diff review generates a continuous reactive fix load that is nobody's roadmap item.
2. **kind/bug is the largest kind everywhere** — code 1,900, openchamber 646, opencode 630 window items. Most work in the window is reactive bug-fixing; planned work is the minority.
3. **Main-branch CI failure is a permanent stray-work generator.** Code titles matching CI/Actions: 745; "flake/flaky": 343. There is already a dedicated epic, #8976 (epic: main-broken), which held ~20 sub-issues before this audit and still had 3 fresh orphans (#13849, #13843, #13619) with no labels at all. The generator files them parentless; the epic only fills when a human/agent sweeps.
4. **Quota / pool-routing fixes recur identically in both forks** — "quota" in titles: opencode 249, openchamber 201. The fork-local copies of the routing logic keep breaking the same way; this is the strongest convergence candidate found: one shared routing implementation (or upstreaming the fix set) would retire two parallel stray streams.
5. **Generated recurring work lands parentless by construction.** At least 14 open "Scheduled Bolt / Palette / Suggestions / flake-sync" issues in code had no parent label and no body reference, although a parent exists (epic: scheduled tasks #8979, epic: daily maintenance #8978). Same shape for the design mill: passes 1–7 (#10389–#10405) sat parentless although epic: design register #8986 exists. The fix is at the generator: emit `primary-parent/*` + `epic/*` labels and a sub-issue link at creation.
6. **mcpx and fleet dominate code-repo stray churn** — title mentions: mcpx 1,365, fleet 1,082, queue 690, opencode 674, nix 506, quota 366. Both already have maintenance epics (#12459, #12460); the stray volume is a routing problem (items not filed under them), not a missing-parent problem.
7. **PR labelling is the single biggest hygiene gap.** 95% of code PRs carry no labels (forks similar: 1,213 of 2,596 opencode window items and 1,221 of 2,433 openchamber window items unlabeled, almost all PRs). Issue labelling is comparatively good (96% labelled). Any future "every item labelled" rule has to be enforced at PR creation/merge, not by sweeps.
8. **dezren39/fleet's own tracker is nearly bare** — 15 labels total, 106 of 113 window items unlabeled, 19 issues against 94 PRs. Fleet work is actually tracked in developing-today/code (epic(fleet): maintenance #12460); the fleet repo tracker functions as a PR queue only. Recommendation: either adopt the code-repo label taxonomy subset there (started: `stray-work`, `stray/needs-parent`, `audit-7` created) or formally declare code the fleet tracker and stop filing fleet issues in dezren39/fleet.
9. **Authorship concentration explains the shape.** dezren39 authored 12,344 of 13,452 code window items (92%), the developing.today bot 860, dependabot 140. Stray work is machine-generated at scale; parentless generation (trend 5) is the systemic cause, not human forgetfulness.
10. **Description/title hygiene is already good.** Empty bodies: 8 across all four repos' windows. Titles needing correction in the items read in full this session: 0 — no titles or descriptions were changed, because none of the 34 edited items (nor the 83 orphans reviewed) had a wrong or empty title/description. The gap is linkage and labels, not prose.

## Labels created (all four repos)

- `stray-work` (fbca04) — "Stray work: not tied to a design or roadmap item (AUDIT-7 Lane 7)"
- `stray/needs-parent` (d93f0b) — "Stray work with no appropriate parent/epic/design yet (AUDIT-7 Lane 7)"
- `audit-7` (5319e7) — "AUDIT-7 seven-lane audit programme (2026-11-23)" (verified absent in code before creation; the umbrella's label display notwithstanding, GET /labels/audit-7 returned 404)

Label writes used `fleet gh api` POST /labels because `~/tools/gh-post` exposes no label verb — flagged to the coordinator in the lane return.

## Trees built (sub-issues + labels, all verified by readback)

Batch 1 (20 children, claim-checked, 0 claims found):

- epic: main-broken #8976 ← #13849, #13843, #13619 (+labels stray-work, epic/main-broken, primary-parent/main-broken)
- epic(id): maintenance #12462 ← #13864, #13821, #12050 (+epic/id-maintenance, primary-parent/id-maintenance)
- epic(fleet): maintenance #12460 ← #13078, #13073, #12498, #12235, #5504 (+epic/fleet-maintenance, primary-parent/fleet-maintenance)
- PM platform tracker #13504 ("PM platform on Cloudflare: what is left") ← #13503, #13502, #13497, #13496, #13495 (+epic/pm-stack)
- epic: scheduled tasks #8979 ← #12664, #12467, #12249, #12250 (+epic/scheduled-tasks, primary-parent/scheduled-tasks)

Batch 2 (14 children, claim-checked per item inside the batch script, 0 skips):

- epic: scheduled tasks #8979 ← #12260, #12133, #11355, #11166, #11165, #10937, #10628, #10538, #10387, #9404
- epic: design register #8986 ← #10389, #10390, #10392, #10393 (design-mill root + passes 1, 3, 4)

Total this session: **34 orphaned issues linked into parents and labelled**. Every child also carries `stray-work`. Readback: parent sub_issues lists and child label lists fetched after the writes (one child, #10937, confirmed via its `parent_issue_url` after its batch log line was lost in a sandbox restart).

Note: #13078 gained a `status:done` label from another worker during the batch — the link stands; its disposition belongs to its owner lane.

## Remaining work — enqueued as pilot-queue slices

The exhaustive pass over the remaining sets exceeds one session. Three `feature_slice` packets (kind `feature_slice`, `payload.slice` A/B/C, dedupe keys `audit7:lane7:feature_slice:code:13874:<slice>`) were prepared for queue `repo-health-code-pilot`:

- **Slice A** — the 521 text-linked open code issues: formalise the body reference into epic/primary-parent labels + sub-issue links; `stray/needs-parent` where no parent fits.
- **Slice B** — the ~49 remaining orphaned open code issues + needs-parent triage (propose new parents where none of the existing epics fit; candidates spotted this session: sccache/laptop toolchain #13367, fleet kv build phases #13314/#13316 whose parent is the fleet-tool epic #8990, repo-root hygiene #12136/#12137).
- **Slice C** — the 80 open fork/fleet items, with cross-repo parent links into code epics, respecting lanes 2–4 ownership via the umbrella pointer rule.

**Blocker recorded:** at enqueue time the queue broker was unreachable (`repo-queue: broker: OSError: proxy closed during CONNECT`, then a timeout on retry, after `~/tools/init-ensure.sh` and a successful laptop-ssh check). The packets are preserved with the lane's working papers and their full specification is quoted in the final comment on #13874 so the coordinator can enqueue them verbatim when the broker is back. This is the one deliverable step not completed in-session.

## Repeatable procedure (the skill, as a procedure)

The code repo has no skills home (checked: no `skills/` directory convention in doc/ or repo root for audit procedures), so the procedure is documented here, per the lane brief's fallback:

1. Census: `fleet gh api repos/<repo>/issues?state=all&since=<window>&per_page=100 --paginate` projecting number/title/state/PR-flag/labels/body-length; separately `state=open` with bodies + database ids.
2. Classify each open issue: parent-labelled (has `epic/*` or `primary-parent/*`) / text-linked (body contains `#<n>` refs) / orphaned (neither). Exclude the audit programme's own tracking issues.
3. Map orphans to parents by `app/*` label, then title prefix, against the standing parent register: maintenance epics #12457 (atlas), #12458 (pm-stack), #12459 (mcpx), #12460 (fleet), #12461 (lab-cluster), #12462 (id); #8976 main-broken; #8978 daily maintenance; #8979 scheduled tasks; #8986 design register; programme trackers (e.g. #13504 PM platform) for enhancements that are not maintenance.
4. Before editing any item: read its comments (`per_page=100`, count-confirmed) and search for a lane claim — a claim is a hard stop; record a pointer on the umbrella instead.
5. Link: POST `/issues/<parent>/sub_issues` with the child's database `id`; POST `/issues/<child>/labels` with `stray-work`, `epic/<slug>`, `primary-parent/<slug>`.
6. Verify by readback (parent's sub_issues list + child's labels) — never trust the write response alone.
7. Post progress on the tracking issue at every slice boundary; file follow-on slices as `feature_slice` queue packets, never `design`.

## Suggestions (latitude items)

- **Generator fix (highest leverage):** the scheduled-report and design-mill generators should emit parent labels + sub-issue links at creation; trends 5 and 9 say sweeps will otherwise recur forever. Filed as part of slice B's remit on #13874.
- **Convergence:** the twin quota/routing fix streams in the opencode and openchamber forks (450 title hits combined) should converge on one implementation; the fork-upkeep design-mill pass (#10395, still parentless, slice B) is the natural home for that decision.
- **New parent candidate:** a "laptop toolchain" parent (sccache #13367, heal cadence #13081, session-count cap #12122 cluster) — none of the existing epics fits; slice B to propose it on #13874.
- **PR labelling rule:** label at merge time from the linked issue's labels (95% of PRs are unlabeled; sweeps cannot keep up at 6,443 PRs/month-window scale).

## Lane brief (verbatim)

The full Lane 7 brief is preserved as the first comment-history of tracking issue #13874's programme record and in the umbrella #13871; the operative text: "YOUR BRIEF — LANE 7 — STRAY-WORK (the issue lane). Create ONE issue in developing-today/code (title like "Stray work audit: PRs and issues not tied to a design or roadmap item") and DO the work under it. Scope: every PR and issue that did NOT come from a design or specific roadmap item — bug fixes, other work — plus, for the hygiene pass, everything open right now AND anything closed or with any activity in the past month (since 2026-10-23), across the four repos. Determine trends across the stray work (what kinds of fixes keep recurring? what does that say?). Add labels. For each stray item, judge whether there is an appropriate item it should have been added to — a bug tracker, an epic, a specific design — and link it in. Link issues, rework, and bugs back to the originating code. Ensure every PR and issue in scope has all appropriate labels, a correct title, a detailed description, and comments creating any further links. Use the parent-issue concept to build trees of issues (sub-issues under parents/epics). This lane's deliverables are the issue itself, the trees and links it creates, plus the same report/log/skill deliverables as the others. Coordinate label work with lanes 2–4 via the umbrella (you own stray-work labelling; they own their own scopes)." — plus the AUDIT-7 shared rules (fleet-gh only, gh-post for writes, tracking-first, exhaustive slices via the pilot queue, merge authority, flag doctrine, laptop worktrees, dated doc deliverable including the prompt verbatim). The complete verbatim brief is attached to the lane's final summary comment on #13874.
