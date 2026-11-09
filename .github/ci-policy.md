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
`main-broken.yml` (the Phase 3 watcher) references them by name in
its `workflow_run` trigger; `jules.yml` has no `workflow_run`
trigger. If branch rules
ever require checks by name, require the `basic`-prefixed names
(plus `nix flake check` for `id`) — full main-tier jobs must
never become merge requirements.

## No merge queue (Drew ruling, 2026-11-08)

`main` has **no merge queue**: merges land as direct merge commits,
and no workflow declares a `merge_group` trigger. A "merge queue
pilot" section stood here from 2026-10-29 describing a ruleset
(`main: merge queue pilot`) that was never actually configured on
the repo — verified 2026-11-07 on #5149: no ruleset on `main`, no
merge-queue configuration in branch protection, and zero
`merge_group` runs had ever occurred. Drew ruled it closed on
2026-11-08 ("no merge queue"): rather than leave the triggers
configured-but-unexercised, the `merge_group` triggers were removed
from every workflow (`proxy-port-fallbacks`, `workflow-lint`,
`opencode-config-guard`, `fleet-search-parity-guard`) — the two
guards that ran *only* on `merge_group` now run on `pull_request`
instead — and this policy was updated to match. The PR-tier required
check stays `basic: proxy port fallbacks`, which the dedicated
`.github/workflows/proxy-port-fallbacks.yml` runs on `pull_request`
events. `gh pr merge` merges a PR directly; there is no
merge-pending queue state.

## mcpx fix-test guard (#1969)

`.github/workflows/mcpx-fix-test-guard.yml` is a **basic-tier** check
(job `basic: mcpx fix carries a test (#1969)`): on pull requests
touching `pkgs/mcpx/**` it fails a fix PR — title starting with `fix`,
or a body `Fixes`/`Closes`/`Resolves` link to a `bug`/`kind/bug`
issue — that changes no `*_test.go` / `*.test.ts` file under
`pkgs/mcpx/`, unless the body carries `No test: <reason>` (the reason
is echoed in the check output and job summary). This is the option-C
enforcement of the failing-first rule in
`pkgs/mcpx/docs/testing.md` ("A test per bug"); the match logic lives
in `.github/scripts/check-mcpx-fix-test.sh` with shell contract tests
in `.github/scripts/check-mcpx-fix-test.test.sh`, which also run on
pushes to main. PRs outside the workflow's `pkgs/mcpx/**` path filter
never fire the check.

## fleet-search / atlas-search writer parity guard (#5920, #5604)

`.github/workflows/fleet-search-parity-guard.yml` is a **basic-tier**
check (job `basic: fleet-search pyjson/render parity (#5920)`):
`pkgs/fleet/crates/fleet-search/src/{pyjson,render}.rs` are verbatim
copies of the atlas-search writers (fleet step 5, #5433), and the
guard in `.github/scripts/check-fleet-search-parity.sh` fails when
either pair diverges by anything beyond the pinned four-line parity
header at lines 2-5 of each file -- a changed or removed header fails
too. Shell contract tests live in
`.github/scripts/check-fleet-search-parity.test.sh` and run in the
same job. Since #5604 the guard also holds `server.rs` (the Atlas
service rehomed as `fleet serve search`) to the same rule, with its
own pinned headers. Intentional changes land in both copies in the same PR;
intentional header changes update the guard script in the same PR.

## Workflow and composite-action validation (#5743)

`.github/workflows/workflow-lint.yml` is a **basic-tier** check
(job `basic: workflow lint`): on pull requests touching
`.github/workflows/**` or `.github/actions/**`, and on merge
groups, it runs `.github/scripts/check-workflows.sh`, which lints
the complete workflow set with actionlint (shellcheck over `run:`
blocks at `-S warning` when shellcheck is available, matching the
mcpx leg) and shape-validates every `.github/actions/*/action.yml`
(actionlint parses every file it is given as a workflow, so action
metadata gets a dedicated validator in the same script). The mcpx
actionlint leg stays scoped to the mcpx workflows (#1996); this is
its generic counterpart. Shell contract tests live in
`.github/scripts/check-workflows.test.sh` and run in the same job
against valid and deliberately invalid workflow/action fixtures.
actionlint is a pinned release binary, checksum-verified in the
workflow before use.

## OpenCode metered-provider config guard

`.github/workflows/opencode-config-guard.yml` runs the static guard and
its fixtures on pull requests, merge groups, and pushes to `main`. The
guard checks known metered selections (`cloudflare-*` and the direct
PAYG `deepseek` provider) in model-route targets/fallbacks, provider-route
selections, top-level defaults, and agent model/fallback fields. Disabled
providers are still checked: disabling a seat today must not make a later
re-enable silently restore metered exposure.

It also runs the `opencode-go.*` seating-guard regression fixtures
(`bin/check-opencode-go-seating.test.mjs`) on pull requests using Node 24.
Those fixtures use a synthetic SQLite store and pin the exact-id credential
and post-config-change successful-attempt assertions; CI never attempts to
read the laptop's live OpenCode database. The live-store check is the hourly
`opencode-go-seating-guard.timer` on the laptop.

An explicitly approved one-off use must carry a standalone comment directly
above that specific selection, with a non-empty reason:

```jsonc
"targets": [
  // payg-exception: approved for the named migration, 2026-11-02
  "deepseek/deepseek-chat"
]
```

The marker is per selection; stale, malformed, or misplaced markers fail.
This is a static check of the canonical file, not enforcement of runtime,
machine-local, or environment overrides, and it does not claim to classify
every provider as metered or unmetered.

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

| workflow                                                       | PR tier                                                                                             | main tier                                                                                           | nightly                                                          |
| -------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| nix ci                                                         | — (PR proxy check is in the dedicated workflow below)                               | proxy port fallbacks + full flake-check (formatting auto-fixed on main by the auto-format workflow) | schedule in nix-ci.yml                                           |
| proxy-port-fallbacks                                           | `basic: proxy port fallbacks` on `pull_request`                                                          | — (main copy remains in nix-ci.yml)                                                                 | —                                                                |
| workflow-lint | `basic: workflow lint` on `pull_request` (paths `.github/workflows/**`, `.github/actions/**`) | — | — |
| id                                                             | guard + affected-crate matrix (`id-pr-checks.txt`)                                                  | + full `nix flake check ./pkgs/id` job                                                              | schedule in id-check.yml + id-e2e-nightly.yml (full flake check) |
| mcpx conformance                                               | spec-parity leg only (measured 28s exec, run 40801660942, 2026-10-29 — under the 5m fast-leg limit) | all conformance legs + records + spec-parity                                                        | — (main covers it)                                               |
| id bench                                                       | — (removed)                                                                                         | yes                                                                                                 | —                                                                |
| go modules build / identity build / map build / mcpx nix build | class-3 paths only (see each workflow)                                                              | always (push to main, no path filter)                                                               | —                                                                |
| mcpx, lab-ops-hub, mod-tidy, research-workflow                 | unchanged path-gated PR checks (already PR-tier weight)                                             | push to main                                                                                        | —                                                                |
| mcpx test (#1998)                                              | init + unit + lint + 6 static e2e shards + ci-ok (PR tier, non-draft PRs)                           | same full sharded suite on push to main                                                             | — (main covers it)                                               |
| mcpx fix-test guard (#1969)                                    | fix PRs touching `pkgs/mcpx/**` must carry a test or a `No test: <reason>` attestation (basic tier) | contract tests for the guard script only                                                            | —                                                                |
| production-line guard (#1411)                                  | `basic: production line limit (2500)` on `pull_request`                                             | —                                                                                                   | —                                                                |

## CI-policy agreement contract (#7089, Phase 1)

For every workflow in this contract's scope, three records must
agree: the tier this policy assigns, the triggers the workflow
declares, and the Actions enablement state. A disagreement in any
leg is a defect with exactly one recorded disposition — **re-arm**
(triggers restored to the mapping + venue enabled + a completed run
on main as proof), **fold** (the check moves into a named venue that
satisfies the contract), or **retire** (workflow, policy row, and
header promises removed in the same change) — never a state that
persists by default. "Dispatch-only and nobody dispatches" is not a
disposition.

**Tier → trigger mapping** (the checker below enforces exactly
this):

| tier             | satisfied by                                                                 |
| ---------------- | ---------------------------------------------------------------------------- |
| PR (basic) tier  | a `pull_request` trigger (with the workflow's path gate where one exists)    |
| main tier        | a `push` trigger on `branches: [main]` (path gates as before), or a named equivalent main-tier venue running the same check |
| nightly tier     | a `schedule` trigger                                                         |

A `workflow_dispatch:`-only block satisfies **no** tier, and
`merge_group` satisfies no tier either: there is no merge queue
(Drew ruling, 2026-11-08, above), which supersedes the #7089
design's original merge_group pin for basic-tier guards — the
guards that ran only on `merge_group` now run on `pull_request`,
and this contract's mapping follows that ruling.

**Scope.** The guard/build workflows this policy names outside the
test-suite estate (#6185's registry owns suite venues):
`workflow-lint`, `proxy-port-fallbacks`, and
`fleet-search-parity-guard` (the conforming basic-tier guards),
`opencode-config-guard`, `mcpx-fix-test-guard`,
`production-line-guard`, and the build set `go-modules-build`,
`identity-build`, `map-build`, `mcpx-nixbuild`. Phase 4 of #7089
inventories widening this scope to every workflow this policy
names.

**Checking.** `.github/scripts/check-ci-policy-agreement.sh`
compares each in-scope workflow's tier assignment against its
declared triggers in the tree and runs in `workflow-lint`; shell
contract tests live in
`.github/scripts/check-ci-policy-agreement.test.sh` (oracle: for
every in-scope workflow, tier, declared triggers, and the recorded
waiver set must agree). Enablement is not visible from the tree:
it is audited from the Actions API on the #6187 recurring cadence,
and a disabled in-scope venue is reported with its disposition.
At Phase 1 landing, the disabled in-scope venues are exactly the
waiver set below (opencode-config-guard and the #6981 build set
are `disabled_manually`).

**Waivers — the recorded Phase 2–3 disagreement set.** Each waiver
names its phase issue and awaits Drew's re-arm-vs-dispatch-only
ruling on #7089; a waiver is a record, not a disposition. When a
phase lands or is ruled, the waiver is deleted in the same PR that
changes the triggers or strikes the tier — the checker fails on a
stale waiver whose leg now agrees.

| workflow                                          | missing leg(s)                                          | phase issue |
| ------------------------------------------------- | ------------------------------------------------------- | ----------- |
| opencode-config-guard                             | main tier (`push`); venue `disabled_manually`           | Phase 2 — #6953 |
| mcpx-fix-test-guard                               | PR tier (`pull_request`) and main tier (`push`)         | Phase 2 — #6929 |
| production-line-guard                             | PR tier (`pull_request`)                                | Phase 3 — #6959 |
| go-modules-build, identity-build, map-build, mcpx-nixbuild | PR tier (`pull_request`) and main tier (`push`); venues `disabled_manually` | Phase 3 — #6981 |

## Coverage rule

Nothing removed from PRs may vanish: the union of main-tier +
nightly jobs must cover the old PR matrix. For pkgs/id the full
flake check runs on main (id-check full-flake-check job) and
nightly twice over (id-check schedule, id-e2e-nightly). For nix ci
the flake-check job runs on main and on the nix-ci nightly
schedule. mcpx conformance legs run on every mcpx-touching merge
to main.

## Test-suite coverage contract (#6185, Phase 1 #6186)

The machine-readable inventory is
[`.github/test-suite-registry.json`](./test-suite-registry.json). A
suite counts as covered only when all five conditions hold:

1. **Registered:** exactly one registry entry names the suite's files,
   venue, and invocation; adding a suite without an entry is incomplete.
2. **Real venue:** the venue is enabled and dispatchable in GitHub Actions,
   or is a flake check that actually runs. A disabled Actions workflow is
   not a venue that runs the suite.
3. **Proven run:** the venue names the suite files explicitly and asserts
   they exist first; a reported `SKIP` fails coverage. Build, vet, and
   typecheck steps do not count as running a test suite.
4. **Runnable and hermetic:** the suite passes from a clean checkout with
   networking disabled; tests use mock seams and do not rely on
   import-time side effects or a mismatched lockfile/runner.
5. **Ratcheted:** a check compares the on-disk suite inventory, registry,
   and venue contents. Workflow enablement is external state and must be
   audited through the Actions API by the recurring check, not inferred
   from files in this repository.

This phase does not add or change workflow triggers. Under the dispatch-only
coverage ruling in #6185, an enabled workflow is not evidence that a suite
ran: a suite without an observed, successful run is **not run**, not green.
The registry records current venues and API enablement state; it does not
infer execution from either. Phase 2 wiring, Phase 3 venue dispositions,
and the Phase 4 ratchet remain separate work.

## Dedup rule

A commit must not run the same suite twice via push +
pull_request. Standard shape: `pull_request` for PRs, `push`
restricted to `branches: [main]` (plus tags where already
applicable, e.g. goreleaser).

## Concurrency rule (#1780)

Standard block (design §4): group keyed by `run_id` for
push-to-main, by `github.ref` otherwise, `cancel-in-progress: true`
— PR/branch runs cancel on supersede; main runs are never
cancelled. Exceptions: scheduled singleton/deploy workflows
(cip=false) are NOT touched. Issue/PR management is NOT
event-triggered: `repo-sweeper.yml`
runs every 15 minutes (`*/15 * * * *`, one live run: group
`repo-sweeper`, `cancel-in-progress: true` — safe because the
sweeper is schedule/dispatch-only) and performs the management
duties in one deterministic pass. It replaced `auto-assign.yml`
(assign the maintainer to open issues/PRs). Kept event-based,
deliberately: `jules.yml` is retained because `main-broken.yml` and
`repo-sweeper.yml` reference the path. The retained `jules` label
starts remediation via `pm-dispatch`; Jules was decommissioned
(#5098), and the old `workflow_run` ci-failure-fix leg is gone. This
is a synchronous human-initiated handoff, not management
bookkeeping. Never
pair `queue: max` with a cancel-in-progress expression that
can evaluate true for `issues`/`pull_request_target` events
(a GitHub validation error, openchamber#84) — the sweeper
stays schedule-only for this reason.

## Nix cache backend reliability (#774)

Magic Nix Cache is a performance optimization, not a CI verdict. The eight `DeterminateSystems/magic-nix-cache-action` steps in `id-check.yml`, `id-bench.yml`, `id-e2e-nightly.yml`, `mcpx-docs.yml`, `mcpx-nixbuild.yml`, and `mcpx-realsoftware-nightly.yml` are best-effort (`continue-on-error: true`): a cache service throttle (including HTTP 418) or 5xx must fall through to a cold build, while the subsequent build/check remains authoritative.

The required `nix flake check` in `nix-ci.yml` intentionally has no shared cache. The October 2026 #774 incidents on PRs #771 and #747 showed that cache-backend failures can masquerade as code failures. This change keeps the current Magic Nix Cache / GitHub Actions cache choice and does not migrate to Cachix or another backend; evaluate that separately.
