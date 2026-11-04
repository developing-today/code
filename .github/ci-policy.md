# CI trigger policy — no push/PR CI

Design history: developing-today/code#1779 (ci-fast-pr-full-main),
#1780 (cancel-in-progress), #1781 (the former PR-tier policy), #1782
(main-breakage chain, `.github/workflows/main-broken.yml`). Tracker: #1599.

**Superseded 2026-11-04 by the owner ruling recorded in #5501: there is
no commit- or PR-triggered CI.** That PR removed `push:` and
`pull_request:` entries from the workflows. CI is requested through
`workflow_dispatch`/placement, schedules, and the remaining explicit
events (`workflow_call`, `merge_group`, `workflow_run`, `release`, and
`issues`). The #1779/#1781 model is history; this file describes the
trigger model now in force.

Every workflow carrying the header comment
`CI tier policy: see .github/ci-policy.md` is governed by this file.
Change triggers and this policy in the same PR; do not change a
workflow's event behavior ad hoc.

## Merge requirement

**There is no merge gate produced by GitHub Actions.**

Verified 2026-11-04 against current `main`: branch protection reports
`required_status_checks: null`, and
`gh api 'repos/developing-today/code/rulesets?includes_parents=true'`
returns no rulesets. The 2026-10-29 merge-queue pilot is not configured.
Do not call a check "required" until branch protection or a ruleset
actually names it.

The `basic: ` and `full (main tier): ` job-name prefixes remain labels
for a verification profile, not merge gates. The `id` workflow's
aggregator keeps its historical status name `nix flake check`.

Formatting is not a gate either. `auto-format.yml` runs on schedule or
dispatch; no treefmt context runs per change.

## Requesting verification

Dispatch the relevant workflow against the branch or SHA being reviewed
(by hand, through `org-run.yml`, or through the queue tooling), then
read that run's result. A change with no run is **not** evidence of a
passing check; report "not run" rather than treating an empty check
list as green.

Some workflows are `disabled_manually` in GitHub Actions. Before
dispatching, verify that the workflow is enabled: an `on:` declaration
alone does not mean GitHub will run it. A disabled workflow is not
verification.

Each workflow's `on:` block is the source of truth for its exact
events, filters and inputs. As of #5501, no workflow has a `push`,
`pull_request` or `pull_request_target` trigger. `proxy-port-fallbacks`
and `opencode-config-guard` retain `merge_group`; those checks run only
if a merge queue is enabled. Check the workflow file rather than
duplicating its trigger inventory here.

The design's change classes remain an advisory guide to what to
dispatch; they do not trigger anything:

| class                        | examples                                                                         | verification to dispatch                                                  |
| ---------------------------- | -------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| 0 — prose                    | `**.md`, docs/, comments-only                                                    | none, or markdown lint for generated docs                                  |
| 1 — config/data              | non-build `**.json/yaml/toml`, `.github/**` (non-workflow)                        | syntax parse + lint of changed files                                      |
| 2 — package source (default) | one package's src + tests                                                        | `id-check`, `mcpx-test`, `mcpx-conformance`, `pm-*`, `scripts-test`        |
| 3 — elevated                 | lockfiles, `go.mod`/`go.sum`, `flake.nix`/`flake.lock`, `package.nix`, workflows | class 2 plus affected-package build (`*-build`, `mcpx-nixbuild`)            |
| 4 — nix expressions          | `**.nix`                                                                         | `nix-ci` (flake evaluation)                                                |

Default when a change spans classes: use the highest class.

## Workflow-specific notes

### mcpx fix-test guard (#1969)

`.github/workflows/mcpx-fix-test-guard.yml` preserves the failing-first
policy in `pkgs/mcpx/docs/testing.md` ("A test per bug"). On dispatch,
it runs the guard's shell contract tests
(`.github/scripts/check-mcpx-fix-test.test.sh`). The PR-attestation
step (a `fix` PR touching `pkgs/mcpx/**` must carry a test or a
`No test: <reason>` line) requires a `pull_request` payload and is
therefore inert under the current triggers.

### OpenCode metered-provider config guard

`.github/workflows/opencode-config-guard.yml` runs its static guard and
fixtures on `merge_group` and dispatch. It checks known metered
selections (`cloudflare-*` and direct PAYG `deepseek`) in model-route
targets/fallbacks, provider-route selections, top-level defaults, and
agent model/fallback fields. Disabled providers are still checked: a
later re-enable must not silently restore metered exposure.

An explicitly approved one-off use must carry a standalone comment
directly above that selection, with a non-empty reason:

```jsonc
"targets": [
  // payg-exception: approved for the named migration, 2026-11-02
  "deepseek/deepseek-chat"
]
```

The marker is per selection; stale, malformed, or misplaced markers
fail. This is a static check of the canonical file, not enforcement of
runtime, machine-local, or environment overrides, and it does not
classify every provider as metered or unmetered.

### Event-specific job guards

Some job/step-level `if:` expressions still refer to `push` or
`pull_request`. They are not workflow triggers and do not make those
events run. Keep a guard when it prevents a dispatch from entering a
step that requires an event payload (for example PR number/title/body);
remove redundant guards only when behavior on every remaining event is
unchanged. A guard is not a substitute for dispatched verification.

## Concurrency

The #1780 standard block (group keyed by `run_id` for push-to-main, by
`github.ref` otherwise, `cancel-in-progress: true`) no longer applies
to push/PR runs because those events are gone. It remains in place:
dispatch runs key by ref, so a later dispatch of the same ref
supersedes the earlier one. `jules.yml` keeps its load-bearing
`cancel-in-progress: false` (#304), and singleton/deploy workflows stay
serialized.

Issue/PR management is not commit-triggered: `repo-sweeper.yml` runs
its management pass on schedule. `jules.yml` remains `issues`-triggered
deliberately: a human applies the `jules` label, and its `workflow_run`
leg is the ci-failure-fix remediation chain (#304).

## Nix cache backend reliability (#774)

Magic Nix Cache is a performance optimization, not a CI verdict. The
`DeterminateSystems/magic-nix-cache-action` steps in `id-check.yml`,
`id-bench.yml`, `id-e2e-nightly.yml`, `mcpx-docs.yml`,
`mcpx-nixbuild.yml`, and `mcpx-realsoftware-nightly.yml` are best-effort
(`continue-on-error: true`): a cache throttle or 5xx must fall through
to a cold build; the subsequent build/check remains authoritative.

The `nix flake check` in `nix-ci.yml` intentionally has no shared cache.
The October 2026 #774 incidents on PRs #771 and #747 showed that cache
backend failures can masquerade as code failures. This choice is
unchanged; evaluate another backend separately.
