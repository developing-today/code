# CI trigger policy — dispatch, schedule and merge-group only

Design history: developing-today/code#1779 (ci-fast-pr-full-main),
#1780 (cancel-in-progress), #1781 (the PR-tier policy this file used
to describe), #1782 (main-breakage chain, `.github/workflows/main-broken.yml`).
Tracker: #1599.

**Superseded 2026-11-04 by the owner ruling recorded in #5501: there is
no commit- or PR-triggered CI.** `push:` and `pull_request:` entries
were removed from every workflow. CI now runs via `workflow_dispatch`
(including placement through `org-run.yml` and the queue tooling),
`schedule`, `workflow_call`, `merge_group`, `workflow_run`, `release`
and `issues`. The #1779/#1781 model describes how this repo got here;
the sections below are the model that is actually in force.

Every workflow that carries the header comment
`CI tier policy: see .github/ci-policy.md` is governed by this file.
Changing triggers means editing THIS file in the same PR — never an
ad-hoc pipeline change.

## Merge requirement

**There is no merge gate produced by GitHub Actions.**

Verified on 2026-11-04 at `origin/main` `864ed8a4a`:

```console
$ gh api repos/developing-today/code/branches/main/protection \
    --jq '.required_status_checks'
null
$ gh api 'repos/developing-today/code/rulesets?includes_parents=true' --jq 'length'
0
```

Branch protection has no required status contexts and the repository
has no ruleset (the 2026-10-29 merge-queue pilot ruleset is gone), so
`gh pr merge` merges directly rather than enqueueing. Do not describe
any check as "required" until a ruleset or branch protection actually
names it.

Formatting is not a gate either: `auto-format.yml` fixes drift on its
schedule and commits to `main`; no treefmt context runs per change.

The `basic: ` and `full (main tier): ` job-name prefixes remain as
**labels** for the verification profile a job belongs to. One
historical exception remains: the `id` workflow's aggregator job keeps
the status name `nix flake check`.

## What runs, and when

Triggers on current `main`, by workflow:

| trigger                        | workflows                                                                                                                                    |
| ------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `workflow_dispatch` only       | `cf-container-hello`, `cold-store-reaper`, `go-modules-build`, `goreleaser`, `id-bench`, `identity-build`, `lab-ops-hub`, `map-build`, `mcpx-conformance`, `mcpx-docs`, `mcpx-fix-test-guard`, `mcpx-nixbuild`, `mcpx-release`, `mcpx-test`, `mcpx`, `mod-tidy`, `org-run`, `pm-aggregator-tests`, `pm-dispatch-smoke`, `pm-stack-tests`, `production-line-guard`, `research-workflow`, `scripts-test` |
| `schedule` + `workflow_dispatch` | `agents`, `auto-format`, `branch-cleanup`, `flake-inputs-update`, `id-check`, `id-e2e-nightly`, `jules-daily-codebase-ideation`, `jules-daily-flake-sync`, `mcpx-realsoftware-nightly`, `mcpx-tier2-containers`, `nix-ci`, `program-tending`, `repo-sweeper` |
| `merge_group` (+ `workflow_dispatch`) | `opencode-config-guard`                                                                                                                |
| `merge_group` only             | `proxy-port-fallbacks`                                                                                                                       |
| `workflow_run` + `schedule` + `workflow_dispatch` | `main-broken`                                                                                                          |
| `workflow_call` + `workflow_dispatch` | `pm-scheduled`                                                                                                                         |
| `issues` + `schedule` + `workflow_dispatch` | `jules`                                                                                                                          |

`merge_group` jobs run only when a merge queue exists; with no ruleset
today they are dormant but ready if a queue is re-enabled.

## Dispatch is how verification is requested

A change is verified by dispatching the relevant workflow (by hand, by
the queue tooling, or through `org-run.yml`) against the branch or SHA
being reviewed, then reading that run's result. A change with no run is
**not** evidence of a passing check; report "not run" rather than
treating an empty check list as green.

The design's change classes remain an advisory guide to which workflow
to dispatch. They trigger nothing:

| class                        | examples                                                                         | verification to dispatch                                                  |
| ---------------------------- | -------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| 0 — prose                    | `**.md`, docs/, comments-only                                                    | none, or markdown lint if the change touches generated docs                |
| 1 — config/data              | non-build `**.json/yaml/toml`, `.github/**` (non-workflow)                        | syntax parse + lint of the changed files                                   |
| 2 — package source (default) | one package's src + tests                                                        | `id-check`, `mcpx-test`, `mcpx-conformance`, `pm-*`, `scripts-test` as applicable |
| 3 — elevated                 | lockfiles, `go.mod`/`go.sum`, `flake.nix`/`flake.lock`, `package.nix`, workflows | class 2 plus a build of the affected package (`*-build`, `mcpx-nixbuild`)   |
| 4 — nix expressions          | `**.nix`                                                                         | `nix-ci` (flake evaluation)                                                |

Default when a change spans classes: the highest class wins.

## Workflow-specific notes

### mcpx fix-test guard (#1969)

`.github/workflows/mcpx-fix-test-guard.yml` keeps the failing-first
policy from `pkgs/mcpx/docs/testing.md` ("A test per bug"). On dispatch
it runs the guard's shell contract tests
(`.github/scripts/check-mcpx-fix-test.test.sh`); the PR-attestation step
(`fix` PR touching `pkgs/mcpx/**` must carry a test or a `No test:
<reason>` line) needs a `pull_request` context and is therefore inert
until PR-triggered CI returns. The shell contract tests remain the
authoritative check of the guard script.

### OpenCode metered-provider config guard

`.github/workflows/opencode-config-guard.yml` runs the static guard and
its fixtures on `merge_group` and on dispatch. The guard checks known
metered selections (`cloudflare-*` and the direct PAYG `deepseek`
provider) in model-route targets/fallbacks, provider-route selections,
top-level defaults, and agent model/fallback fields. Disabled providers
are still checked: disabling a seat today must not make a later
re-enable silently restore metered exposure.

An explicitly approved one-off use must carry a standalone comment
directly above that specific selection, with a non-empty reason:

```jsonc
"targets": [
  // payg-exception: approved for the named migration, 2026-11-02
  "deepseek/deepseek-chat"
]
```

The marker is per selection; stale, malformed, or misplaced markers
fail. This is a static check of the canonical file, not enforcement of
runtime, machine-local, or environment overrides, and it does not claim
to classify every provider as metered or unmetered.

### Inert PR-only code (follow-up cleanup, no behaviour change)

The trigger removal in #5501 left `if: github.event_name ==
'pull_request'` jobs and steps in a few workflows. They can never be
true now and are deliberately left in place in that PR so no check body
is deleted without review. They are listed here so a future cleanup can
remove or convert them knowingly:

- `mcpx.yml` — `plugin-sync`, `docs-history`, and `stress-pr` jobs are
  PR-only; `vendorhash-repair` is PR+bot-branch only; `test` is
  `event_name == 'push'` only. None can run on dispatch/schedule today.
- `mcpx-docs.yml` — the "Upload site artifact (PR builds only)" step.
- `mod-tidy.yml` — the "Verify tidy changes are committed" (`pull_request`)
  and "Commit tidy changes on main" (`push`) steps; a dispatch run only
  computes the tidy result and discards it.
- `mcpx-fix-test-guard.yml` — the PR-attestation step described above.
- `mcpx-test.yml`, `mcpx-conformance.yml`, `id-check.yml`, `nix-ci.yml`
  — `event_name != 'pull_request'` guards evaluate true on dispatch, so
  they are harmless but misleading comments remain.

Removing or converting these is a separate reviewed change; none of
them is a live check today.

## Concurrency

The #1780 standard block (group keyed by `run_id` for push-to-main, by
`github.ref` otherwise, `cancel-in-progress: true`) no longer applies to
push/PR runs because those events are gone. It remains in place and is
harmless; a dispatch run keys by ref and a later dispatch of the same
ref supersedes it. `jules.yml` keeps its load-bearing
`cancel-in-progress: false` (#304), and singleton/deploy workflows stay
serialised.

Issue/PR management is not event-triggered: `repo-sweeper.yml` runs the
management duties in one deterministic pass on its schedule
(`*/15 * * * *`, group `repo-sweeper`). `jules.yml` stays `issues`-
triggered deliberately: a human applies the `jules` label, and its
`workflow_run` leg is the ci-failure-fix remediation chain (#304).

## Nix cache backend reliability (#774)

Magic Nix Cache is a performance optimization, not a CI verdict. The
`DeterminateSystems/magic-nix-cache-action` steps in `id-check.yml`,
`id-bench.yml`, `id-e2e-nightly.yml`, `mcpx-docs.yml`,
`mcpx-nixbuild.yml`, and `mcpx-realsoftware-nightly.yml` are best-effort
(`continue-on-error: true`): a cache service throttle (including HTTP
418) or 5xx must fall through to a cold build, while the subsequent
build/check remains authoritative.

The `nix flake check` in `nix-ci.yml` intentionally has no shared cache.
The October 2026 #774 incidents on PRs #771 and #747 showed that
cache-backend failures can masquerade as code failures. This choice is
unchanged; evaluate Cachix or another backend separately.
