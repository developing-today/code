# CI tier policy — fast PR tier, full main tier

Design: developing-today/code#1779 (ci-fast-pr-full-main). Phases:
#1780 (cancel-in-progress), #1781 (this policy), #1782 (main-breakage
chain, .github/workflows/main-broken.yml). Tracker: #1599.

Every workflow touched by the tier redesign carries the header
comment `CI tier policy: see .github/ci-policy.md`. Tuning the tiers
means editing THIS table by PR — never ad-hoc pipeline changes.

## Merge requirement

**Merge requirement: the basic-validation (PR) tier only. The full
suite runs on main and is NOT a merge gate.**

In this repo the PR tier is named **basic**: job display names in
the check list carry the tier — PR-tier jobs are prefixed
`basic: ` (`basic: proxy port fallbacks`, `basic: spec parity`,
`basic: nix flake check (<check>)`, `basic: go build`, ...), and
jobs that run only on main/nightly are prefixed
`full (main tier): `. One deliberate exception: the `id`
workflow's aggregator job keeps its historical status name
`nix flake check` — the required status context branch protection
has always used — while the matrix jobs under it carry the tier
prefix (the same exception opencode made for its branch-required
aggregator).

**Formatting is not a merge gate.** No treefmt/format context is
in the basic tier or the required checks. Formatting is
auto-fixed on main instead: the `auto-format` workflow
(`.github/workflows/auto-format.yml`) runs treefmt on every
push to main (plus a nightly sweep) and commits any fixes
directly to main as the github-actions bot
(`style(treefmt): auto-format on main`). The fix lands within
seconds of the merge that introduced the drift — well before
the main-tier full check would file a breakage for it.

Workflow names (`nix ci`, `id`, `mcpx`, `mcpx conformance`,
`mcpx nix build`, `go modules build`, `identity build`,
`map build`, `id bench`) are deliberately NOT renamed:
`main-broken.yml` (the Phase 3 watcher) and `jules.yml` reference
them by name in their `workflow_run` triggers. If branch rules
ever require checks by name, require the `basic`-prefixed names
(plus `nix flake check` for `id`) — full main-tier jobs must
never become merge requirements.

## Merge queue (pilot, 2026-10-29)

`main` runs a merge queue (ruleset `main: merge queue pilot`):
merge method MERGE (merge commits only), grouping HEADGREEN
(only the group's combined head must pass the required checks),
min entries to merge 1 with a 1-minute wait (a lone PR is never
held for a batch), max entries to merge 100, max entries to build
100, check response timeout 30 minutes. The queue enforces the
same required check as branch protection — `basic: proxy port
fallbacks` — which is why `nix ci` also runs on `merge_group`
events: the queue validates the combined merge-group head, never
the PR head alone. Operationally: `gh pr merge` now ENQUEUES a
PR instead of merging it; "in the merge queue" is merge-pending,
not a failure.

## Change classes (§3.3 of the design)

| class                        | trigger paths (examples)                                                                                                                       | PR tier                                                                         |
| ---------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------- |
| 0 — prose                    | `**.md`, docs/, comments-only changes                                                                                                          | nothing, or markdown lint only                                                  |
| 1 — config/data              | `**.json/yaml/toml` non-build, `.github/**` (non-workflow)                                                                                     | syntax parse + lint of changed files                                            |
| 2 — package source (default) | one package's src + tests                                                                                                                      | lint/format + typecheck/syntax of affected packages + affected unit tests       |
| 3 — elevated                 | lockfiles, `go.mod`/`go.sum` roots, `flake.nix`/`flake.lock`, build definitions (`package.nix`), `.github/workflows/**`, shared/core libraries | class 2 + compile check of affected packages                                    |
| 4 — nix expressions          | `**.nix`                                                                                                                                       | class 1 + parse/eval of the changed files only (never full flake check on a PR) |

Default when a change spans classes: the highest class wins. The
policy ships conservative (more class 3) and loosens as the
main-breakage rate (issues labelled `main-broken`) proves out.

## Tier assignment in this repo (after #1781)

| workflow                                                       | PR tier                                                                                             | main tier                                                                    | nightly                                                          |
| -------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| nix ci                                                         | proxy port fallbacks only                                                                           | full flake-check (formatting auto-fixed on main by the auto-format workflow) | schedule in nix-ci.yml                                           |
| id                                                             | guard + affected-crate matrix (`id-pr-checks.txt`)                                                  | + full `nix flake check ./pkgs/id` job                                       | schedule in id-check.yml + id-e2e-nightly.yml (full flake check) |
| mcpx conformance                                               | spec-parity leg only (measured 28s exec, run 40801660942, 2026-10-29 — under the 5m fast-leg limit) | all conformance legs + records + spec-parity                                 | — (main covers it)                                               |
| id bench                                                       | — (removed)                                                                                         | yes                                                                          | —                                                                |
| go modules build / identity build / map build / mcpx nix build | class-3 paths only (see each workflow)                                                              | always (push to main, no path filter)                                        | —                                                                |
| mcpx, lab-ops-hub, mod-tidy, research-workflow                 | unchanged path-gated PR checks (already PR-tier weight)                                             | push to main                                                                 | —                                                                |
| mcpx test (#1998)                                              | init + unit + lint + 6 static e2e shards + ci-ok (PR tier, non-draft PRs)                            | same full sharded suite on push to main                                      | — (main covers it)                                               |

## Coverage rule

Nothing removed from PRs may vanish: the union of main-tier +
nightly jobs must cover the old PR matrix. For pkgs/id the full
flake check runs on main (id-check full-flake-check job) and
nightly twice over (id-check schedule, id-e2e-nightly). For nix ci
the flake-check job runs on main and on the nix-ci nightly
schedule. mcpx conformance legs run on every mcpx-touching merge
to main.

## Dedup rule

A commit must not run the same suite twice via push +
pull_request. Standard shape: `pull_request` for PRs, `push`
restricted to `branches: [main]` (plus tags where already
applicable, e.g. goreleaser).

## Concurrency rule (#1780)

Standard block (design §4): group keyed by `run_id` for
push-to-main, by `github.ref` otherwise, `cancel-in-progress: true`
— PR/branch runs cancel on supersede; main runs are never
cancelled. Exceptions: `jules.yml` (load-bearing cip=false, #304)
and scheduled singleton/deploy workflows (cip=false) are NOT
touched. Issue/PR management is NOT event-triggered: `repo-sweeper.yml`
runs every 15 minutes (`*/15 * * * *`, one live run: group
`repo-sweeper`, `cancel-in-progress: true` — safe because the
sweeper is schedule/dispatch-only) and performs the management
duties in one deterministic pass. It replaced `auto-assign.yml`
(assign the maintainer to open issues/PRs). Kept event-based,
deliberately: `jules.yml` — a human applies the `jules` label
and waits for Jules to start, and its `workflow_run` leg is
the ci-failure-fix remediation chain (#304); a synchronous
human-initiated handoff, not management bookkeeping. Never
pair `queue: max` with a cancel-in-progress expression that
can evaluate true for `issues`/`pull_request_target` events
(a GitHub validation error, openchamber#84) — the sweeper
stays schedule-only for this reason.
