## OC-SURVEY complete — OpenCode + OpenChamber forks programme

Survey file: `~/workspace/pm-book/survey/oc-programme.md` (3,837 lines) — structured as a standalone-subject section, liftable into its own book later. Raw material deposited alongside, per the standing rule, in `~/workspace/pm-book/sideline-material/oc-survey/` (full API dumps, fetch/generator scripts, laptop README captures; index in its `README.md`).

**Counts (from fully paginated `fleet gh api` reads, 2026-11-24T00:35–01:05Z; computed from the responses):**

- **developing-today-forks/opencode**: 1,258 merged PRs (of 1,438 closed; 9 open), grouped by theme in the survey §1.5 — largest themes by conventional-commit scope: core (423), ai (181), ci (125), cli (49), docs (47), roadmap (47), server (42) (full per-theme counts + every PR enumerated with title/author/merged date in the file). **30 open issues** (§3.1).
- **developing-today-forks/openchamber**: 1,186 merged PRs (of 1,290 closed; 2 open), grouped the same way in §1.5 — largest themes: web (475), ui (108), vscode (89), ci (89), quota (48), routing (48). **30 open issues** (§3.2).
- **developing-today/code programme issues**: **91 open issues** in the survey's §3.3 table (scope rule stated in the file: title names opencode/openchamber/the opencode claimant; union of the three title searches). Context: the broad mention search `repo:developing-today/code opencode` totals 4,147 items — the title-search dumps (681 / 137 / 148 items) and the 480-item open-mention set are all in the deposit.

Also in the survey: goals & intentions with issue links (§2, incl. the code#1344/#1357/#1363 attach family, the claimant/session-contract work, and the OpenCode 2 migration), the PM-stack integration surface (§4: opencode-claimant, opencode-scheduled-jobs, opencode-usage-export, one section each, read in the laptop clone), dependency notes (§5, only edges actually seen in titles/bodies/READMEs), and a numbered disagreements list (§6, 9 items — headline: both fork READMEs still claim the latest upstream merge is Oct 16 2026, while upstream sync PRs merged Nov 22–23 on both forks).

Ordering rule for the issue maps is stated in §3: the repos *do* carry explicit `priority/*` labels, so order is priority tier → `user/escalated` → blocked → oldest; no open issue on either fork (or in the code set) carries a milestone.
