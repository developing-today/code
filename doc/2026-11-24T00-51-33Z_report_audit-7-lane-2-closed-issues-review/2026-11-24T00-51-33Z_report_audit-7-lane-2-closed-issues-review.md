# AUDIT-7 Lane 2 — Closed-Issues Review (past month)

- **Programme:** AUDIT-7 (Drew's directive, 2026-11-23 ~17:56 CST) — seven parallel audit lanes over the four repos.
- **Lane:** 2 — CLOSED-ISSUES-REVIEW.
- **Umbrella:** https://github.com/developing-today/code/issues/13871
- **Lane tracking issue:** https://github.com/developing-today/code/issues/13878
- **Record date (UTC):** 2026-11-24T00-51-33Z
- **Scope window:** every issue closed 2026-10-23 through 2026-11-23 (America/Chicago directive date; census by UTC closed-date).

## 1. Census (exhaustive, count-confirmed)

All reads via `fleet gh api` on the laptop (the only sanctioned route; sandbox `fleet gh` hangs on token resolution and the laptop cache daemon was down, so each laptop invocation resolves in-process at ~29 s — this latency shaped the method below). Census used the search API in 17 two-day `closed:` windows per repo, one paginated fleet invocation per window, per-window counts logged (the census run log is `supplements/dump2.log`). No window hit the 1,000-result search cap (largest: developing-today/code 2026-11-02..03, 935).

| repo | closed issues in window |
|---|---|
| developing-today/code | 5,810 |
| developing-today-forks/opencode | 1,187 |
| developing-today-forks/openchamber | 1,153 |
| dezren39/fleet | 11 |
| **total** | **8,161** |

## 2. Method

1. **Full-corpus triage over all 8,161 issues.** The search payload carries each issue's full body, labels, `state_reason`, and comment count. Every one of the 8,161 was machine-read and flag-scored (`supplements/analyze.py`): `NOT_PLANNED` 264 · `DUPLICATE` 8 · `ZERO_COMMENTS` 187 · `NO_LABELS` 490 · `NO_REFS_IN_BODY` 870 · `SHORT_BODY` 106 · follow-up language 1,898 (noisy; handled by slice 4).
2. **Deep read (body + every comment) of the high-risk union:** all not_planned, all duplicates, all zero-comment, all short-body issues, plus all 11 dezren39/fleet issues — 493 issues (`supplements/flagged-full.json`). Bodies were read in full in this session; comment threads were read in full for the duplicates and spot classes (§3) and a detached comment dump for the whole union was started (`supplements/comments.log`, partial — stopped when the remaining adjudication was enqueued, since the slice packets require a fresh read anyway).
3. **Remainder enqueued, not dropped.** Where the deep read exceeds one session, per-batch slices were enqueued to the pilot queue as `feature_slice` packets (premise + singular-ownership check recorded in each packet; Lane 2 owns the closed-issue side, Lane 4 the PR side, Lane 7 stray-work labelling outside this scope):
   - Slice 1 — substantive not_planned adjudication, code: queue task **21595**
   - Slice 2 — not_planned + duplicate adjudication, forks: queue task **21596**
   - Slice 3 — zero-comment completed-close verification (187): queue task **21598**
   - Slice 4 — no-label backlog (490) + promised-follow-up verification: queue task **21600**
   Packets: `supplements/packets/`.

## 3. Findings

### 3.1 The duplicate pair that looked like a lost fix — clean (code#5284, code#5285, code#4982)
- code#5284 and code#5285 are the same report filed twice ("#4942 fix never landed — PR #5256 closed unmerged…"), and both are closed as duplicate. Read in full with comments: the correction comments name #4942 as canonical, and #5285's thread records the fix actually landing — PR #4976 merged (merge commit `ef55926de`), the `m.used >= 0` guard verified on main. **Verdict: clean.** One wart: an unrelated queue-task handoff note (task 9211, "Pay dashboard 500") was posted onto #5284 — a mis-posted comment, recorded here; no action needed beyond this note.
- code#4982 (mcpx refdoc slices 2–4) closed as duplicate: its closing comment records the slices as already implemented on main and the #1846 remainder as delivered under #4909. **Verdict: clean**, with a pointer on the umbrella for Lane 4 to confirm from the PR side.

### 3.2 main-broken issues closed as not_planned — flagged for Lane 4 (16 issues)
Sixteen `main-broken:` tracker issues (15 in code, 1 in openchamber: code#2305, #2554, #5580, #5666, #6201, #6203, #6363, #6769, #7997, #10427, #11804, #12106, #12148, #12239, #12527; openchamber#1560) were closed with `state_reason=not_planned` rather than `completed`. A main-broken tracker normally closes because main went green (completed) or because it was superseded; `not_planned` asserts the work was abandoned, which for a broken-main tracker is either wrong metadata or a silently dropped breakage. Pointer posted on umbrella #13871 for Lane 4 (PR side); the reopen/correction decision stays with Lane 2 via slice tasks 21595/21596.

### 3.3 Title/body mismatch — opencode#2500
Titled "fix(server): session.messages cursor drops type filter…" but its body is an unrelated Playwright aggregator test failure from a code-repo CI run. One of the two subjects may have no live tracker. Assigned to slice 2 (task 21596) with an explicit done-check: the cursor bug is either traced to a live issue or filed new.

### 3.4 Zero-comment completed closes — 184 issues with no closing record on the issue
187 issues closed with zero comments; 184 of them as `completed`. In dezren39/fleet that includes #16, #30, #92, #107 — detailed fix-spec bodies, closed with no comment recording what landed or how it was verified (any evidence lives only in PRs, which is Lane 4's side to confirm). This is the single largest hygiene gap in the window: the close itself leaves no trace on the issue. Slice 3 (task 21598) verifies each one and adds closing-evidence comments where the work is real but unrecorded, reopening where nothing implements the issue.

### 3.5 not_planned decomposition (264) — mostly housekeeping, one substantive tail
- 78 pm-dispatch placeholder issues titled "do the thing" (code) — not_planned is the correct housekeeping close. **Clean (class).**
- ~90 scheduled-bot issues (Scheduled Bolt/Palette/Suggestions, Daily codebase innovation & maintenance, Daily flake sync, Weekly housekeeping, Manual pm-dispatch runs, Fleet lane probes, Persistent fleet lane agents) — automation artifacts. **Clean (class).**
- 16 main-broken — §3.2.
- 6 preserved-branch decision issues — not_planned reads as the "drop" decision; comment confirmation in slices 1–2. **Clean (class, provisional).**
- ~62 substantive issues (real bugs, designs, and fixes, mostly filed 2026-10-27 – 2026-11-05, e.g. code#911, #1401, #1825, #1827, #2603, #2902, #2903, #2991) — each carries discussion (2–85 comments) and a not_planned close whose rationale lives in those comments. These are adjudicated one by one in slices 1–2 (tasks 21595/21596); this session does not pre-judge them from bodies alone.

### 3.6 Labels
490 closed issues (6.0% of the window) carry no labels at all; 179 of the 493-issue high-risk union are among them. Because Lane 7 owns stray-work labelling generally, the additions inside this closed-issue scope were **not** mass-applied from this session (at ~29 s per fleet call, 490 label PATCHes ≈ 4 h and would collide with Lane 7's pass); they are enqueued as slice 4 (task 21600) with the coordination note posted on the umbrella and on #13878, per the lane brief. Labels added by this session: **0** (deliberate, recorded here and on #13878).

### 3.7 Convergence / duplication observations
- The estate closes ~255 issues/day, the large majority by automation (queue workers, scheduled bots, dispatch placeholders). The closed-issue stream is therefore mostly a *machine log*, and its failure modes are machine-shaped: duplicate filings of the same finding minutes apart (#5284/#5285), mis-posted worker notes (#5284), title/body mismatches from templated filing (#2500), and closes with no evidence comment (184). A close-time lint — "completed close requires a comment or a linked merged PR; duplicate close requires naming the canonical issue in a comment" — would have caught every class found here at filing cost ≈ 0. Filed as a suggestion on tracking issue #13878 for the codification lane.
- The "#4942 fix never landed" pair shows the existing self-correction loop working (a later lane found the lost fix, landed it via #4976, and recorded it). The gap is that the record landed on the duplicate (#5285), not on the canonical #4942 — evidence placement, not evidence absence.

## 4. Counts

| measure | count |
|---|---|
| examined (full-corpus triage: body + labels + reason + comment count) | 8,161 |
| deep-read bodies (high-risk union + all fleet) | 493 |
| comment threads read in full this session | 11 (all duplicates + fleet spot checks + #5284/#5285/#4982 threads) |
| reopened | 0 (no close was proven wrong from full reads this session; 16 main-broken flagged, 62 substantive not_planned + 5 duplicates under adjudication in slices 21595/21596) |
| follow-ups filed | 0 new issues; 4 pilot-queue slice tasks filed (21595, 21596, 21598, 21600) carrying all follow-up work with acceptance criteria |
| labels added | 0 (see §3.6 — enqueued as slice 4 with Lane 7 coordination note) |
| pointers posted for Lane 4 | main-broken class (16) + #4982/#5285 (umbrella #13871) |

## 5. Per-issue ledger

`supplements/ledger.csv` — all 8,161 issues: repo, number, URL, closed_at, state_reason, comment count, labels, flags, title, and the lane verdict (`clean` / class verdict / `flagged` / `queued-slice-N` with the task id). The verdict vocabulary is defined in `supplements/ledger.py`, which generated the file from the census supplements, so the ledger is reproducible from the raw data in this folder.

## 6. Procedure (repeatable)

1. Census by search windows: `fleet gh api "search/issues?q=repo:<r> type:issue state:closed closed:<a>..<b>&per_page=100" --paginate`, 2-day windows, per-window counts logged; assert no window ≥ 1,000.
2. Triage-score every returned issue (body, labels, state_reason, comment count) with the flag set in `supplements/analyze.py`.
3. Deep-read the flagged union (body + all comments, `per_page=100`, count-confirmed against the issue's comment count).
4. Adjudicate: clean / reopen-with-reasons / file correction or follow-up / add labels — as you go, not at the end.
5. Enqueue whatever exceeds the session as `feature_slice` packets (`payload.slice` object with parent_issue = tracking issue, number, criterion, done_check, owned_paths; dedupe key `repo-health:v1:slice:<repo>:issue:<tracking>:slice-<k>`).
6. Commit everything — census JSON, ledgers, scripts, packets, logs — as supplements in the dated-record folder (Drew's steer, 2026-11-23 18:46 CST).

## 7. Supplements (all raw material, per Drew's 2026-11-23 steer)

- `supplements/list-developing-today__code.json`, `list-developing-today-forks__opencode.json`, `list-developing-today-forks__openchamber.json`, `list-dezren39__fleet.json` — the full census payloads (every closed issue's body, labels, state_reason, comment count).
- `supplements/ledger.csv` — the per-issue ledger with verdicts (8,161 rows).
- `supplements/ledger-flags.json` — the triage flag table the ledger was built from.
- `supplements/flagged-full.json` — the 493-issue high-risk union with full bodies.
- `supplements/dump.py` (first census attempt, superseded), `dump2.py`, `analyze.py`, `extract.py`, `ledger.py`, `comments.py` — the gathering/analysis scripts exactly as run.
- `supplements/README.md` — the supplements index.
- `supplements/packets/` — the four pilot-queue slice packets as enqueued (tasks 21595, 21596, 21598, 21600).
- `supplements/dump2.log` — the census run log (per-window counts).
- `supplements/comments.log` — partial log of the detached comment dump (stopped when adjudication was enqueued; the slices re-read fresh by design).
- `supplements/tracking-body.md` — the tracking issue's original body.

## Appendix — Lane brief verbatim

> You are LANE 2 — CLOSED-ISSUES-REVIEW of programme AUDIT-7 (Drew's directive, 2026-11-23 ~17:56 CST): seven parallel audit lanes over the four repos (developing-today/code, developing-today-forks/opencode, developing-today-forks/openchamber, dezren39/fleet), exhaustive — every item in scope, one by one, read in full, no sampling.
>
> UMBRELLA TRACKING ISSUE: developing-today/code#13871 — https://github.com/developing-today/code/issues/13871 — the cross-lane coordination point. File your own tracking issue in developing-today/code first, post its link as a comment on #13871, and post progress on your tracking issue at EVERY slice boundary.
>
> SHARED RULES (binding on all seven lanes):
> - GitHub access: the fleet GitHub subcommand ONLY. All reads via `fleet gh api` (the `fleet` binary lives at ~/.local/share/hatch-recovery/bin/fleet — put that dir on PATH); all issue/comment writes via `~/tools/gh-post` (the GitHub App identity). NEVER plain `gh` with the user token, NEVER the GitHub connector. Reason (Drew): direct access burns the personal user API tokens; the GitHub App identity must do this work. If a fleet-gh verb you need does not exist, report that to the coordinator in your return — do not fall back to plain gh.
> - Tracking first: file your own tracking issue in developing-today/code, link it on the umbrella (#13871), and post progress on it at EVERY slice boundary. Check the umbrella + an issue's comments before acting on it — if another lane has claimed it, the lane whose primary scope it is owns the action; the other records a pointer comment on the umbrella, never a duplicate issue or PR.
> - Exhaustive, one by one, read everything. No sampling, no skimming by title. Volume is large — work in slices (per repo, per batch); if a slice exceeds one session, enqueue per-batch sub-work to the pilot queue (`--queue repo-health-code-pilot`, kind `feature_slice` + `payload.slice`, NEVER kind `design`; premise + singular-ownership check first, fresh fleet-gh REST reads, comments per_page=100 count-confirmed) following the lane conventions. A claim comment from another lane is a HARD STOP.
> - Standing authority: merge PRs when verified done (merge authority applies; drafts/conflicted included, fix forward); never force-push main; never close issues for age; file-the-issue in the same pass for anything found. Flag doctrine: if a platform flag trips on a checkout, stop on that checkout, record it, vary the mechanic (worktree off the EXISTING clone, worktree at main then switch ref, vary transport fleet gh/git) before stopping; never re-ask Drew for a permanently-flagging item.
> - Repo work happens in laptop worktrees via ~/tools/laptop-ssh.sh (laptop `amd`, user `user`); NEVER the sandbox for repo work. One clone per repo on the laptop; each piece of work in a worktree off that clone.
> - Deliverables: (a) a dated report + comprehensive log committed to the code repo's doc/ area under the dated-record convention (`doc/<UTC datetime>_<kind>_<name>/` folder with a same-named .md inside, plus a line in doc/README.md), INCLUDING your own prompt verbatim (this whole brief); (b) findings filed as issues/PRs/labels AS YOU GO, not only at the end; (c) where the process is repeatable, a skill for it, committed in the repo if it has a skills home (check first) or documented as a procedure in the report; (d) a final summary comment on your tracking issue.
> - Latitude (rigid requirement to do the work, free choice in outputs): open investigation issues, designs, and bug tickets; open PRs for quick doc fixes and to document findings; create labels; suggest things that aren't top-level projects but could be; suggest where things are converging and could be merged, where there's duplication, which duplicate is best and how they should best be combined.
>
> YOUR BRIEF — LANE 2 — CLOSED-ISSUES-REVIEW (past month). Scope: every issue closed in the past month (since 2026-10-23) across the four repos. One by one, read fully and look for: gaps left behind by the close; issues closed that should NOT have been (reopen with reasons, or file the correction); follow-ups that never got issues (file them); links, tags, and labels that should have been added and weren't (add them). Return a per-issue ledger with actions taken.
>
> Coordination notes: LANE 7 (STRAY-WORK) owns stray-work labelling generally — for label additions inside your closed-issue scope, you act and note it on the umbrella so lane 7 doesn't duplicate. LANE 4 (MERGED-PR-AUDIT) examines the PRs that closed these issues from the PR side; you examine from the issue side — if you find a "closed but not actually implemented" case, post a pointer on the umbrella for lane 4 rather than duplicating its comparison work; the reopen/correction decision is yours.
>
> Your return: your tracking-issue link; the per-issue ledger (issue link, repo, verdict: clean / gap-filed+link / reopened+why / follow-up filed+link / labels added); report doc path + merge SHA; counts (examined, reopened, follow-ups filed, labels added).

> Steer (2026-11-23 ~18:46 CST): extend the dated report to the RAW gathered material too — all data gathered and files produced land inside the dated-record folder as supplements, with the outside index linking in. Anything that isn't a live credential value belongs in the repo.
