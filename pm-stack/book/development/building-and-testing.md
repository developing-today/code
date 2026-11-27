# Building and testing

The PM stack has no single build command. Each Bun application builds and tests inside its own directory; the Nix packages wrap three applications and one script for deployment; and one workflow, `.github/workflows/pm-stack-tests.yml`, runs the stack's suites together. This chapter states each route as the repository records it. A route this chapter cannot ground in a package file, a package definition, or a workflow is marked UNVERIFIED.

## The Bun applications

The aggregator, the console, the projector, and pm-dispatch are Bun applications. Each carries a `package.json` in its own directory, and the test command is the same shape in each: install inside the directory, then run the suite there.

### Aggregator

`pm-stack/aggregator/package.json` (name `pm-aggregator-ui`, version 0.1.0) records the scripts:

| Command, run in `pm-stack/aggregator/` | What it does                                                                                                                                                                                                          |
| -------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `bun run dev`                          | Runs `src/server.ts` with `PM_AGG_DEV=1`                                                                                                                                                                              |
| `bun run start`                        | Runs `src/server.ts`                                                                                                                                                                                                  |
| `bun test`                             | The test suite: the `test/` directory's `*.test.ts` files (the `test` script is `bun test`)                                                                                                                           |
| `bun run typecheck`                    | `bunx tsc --noEmit` against `tsconfig.json`                                                                                                                                                                           |
| `bun run build:css`                    | Builds `src/assets/app.css` from `src/ui.css` with the Tailwind CLI. The built CSS is checked in; the Nix package treats the Tailwind and TypeScript dev-dependencies as build-time-only for that checked-in artifact |
| `bun run provenance:check`             | The provenance-matrix check, `scripts/check-provenance-matrix.ts`. CI runs this as its own job                                                                                                                        |
| `bun run contract:check`               | The contract-corpus check, `scripts/check-contract-corpus.ts`                                                                                                                                                         |

The remaining scripts are data jobs, not builds: the report generator, the quota-forecast backtest, the quota-anomaly replay, the reweight-proposal and weekly-review producers.

### Console

`pm-stack/console/package.json` (name `pm-console`, version 0.1.0) records two scripts: `bun run start` runs `src/server.ts`, and `bun test` runs the suite in `pm-stack/console/test/` — view, work, plan, platform, push, device, sign-in, outbox, and edge tests among them. The console's type-check tooling installs separately in the edge tree: the stack workflow runs `npm ci` in `pm-stack/console/edge/` for that step. The Android wrapper's sign-and-release script carries its own shell suite, `pm-stack/console/android/release-built.test.sh`, which the stack workflow runs as a named step.

### Projector and pm-dispatch

Both follow the aggregator's shape: `bun install --frozen-lockfile` inside the directory, then `bun test`. The projector's suite is in `pm-stack/projector/test/`; the dispatch suite is in `pm-stack/dispatch/test/` (16 files in the tree read for this chapter, including the launch-failure compensation pins). The stack workflow asserts the dispatch test directory exists and holds `*.test.ts` files before running it, so a green run cannot be a silently skipped suite. That assertion pattern — name the file, assert it exists, fail the job if it does not — repeats for every suite the workflow adopted after the suites were found running under no workflow at all.

## The Workers

Each Worker directory carries its own `package.json`, `tsconfig.json`, and `wrangler.toml`, and the surveyed Workers carry their own test directories: `pm-stack/decision-events-worker/test/`, `pm-stack/notify-relay/test/`, `pm-stack/voice-line-worker/test/`. The console edge Worker is the fourth: its tests sit in `pm-stack/console/test/` (the edge and direct tests). The exact per-Worker test command beyond the console edge's is UNVERIFIED in the sources this chapter grounds on; read the Worker's own `package.json` before running it.

One registry test stands apart. `pm-stack/platform/workers.test.ts` holds `pm-stack/platform/workers.json` to the repository's wrangler files: a Worker added to the repository without a registry entry, or a registry entry with no wrangler file behind it, fails that test. The stack workflow runs it in the `pm-stack/platform/` directory. The registry lists 20 Workers; its README's count of eighteen is stale (D6).

## The Python and shell suites

The stack's non-Bun components test in Python and shell, and the stack workflow names each suite explicitly:

| Suite                                                                                                                                     | Runner, as the workflow records it                                                                                                                                                                |
| ----------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pm-stack/tests/` — Kaneo sync guard, closure watchdog, host-health collector, invariant-sweeper timeouts                                 | Python `unittest` discovery over `pm-stack/tests/`, per named file                                                                                                                                |
| `pm-stack/opencode-claimant/` — claimant, poll, launch, handoff, batch dispatch, babysitter, nudges suites                                | Python `unittest`, per named file; the claimant install suite is a Bun suite (`test/install.test.mjs`, run with `bun test`)                                                                       |
| `pm-stack/grist/test_update_choice_options.py`                                                                                            | Python `unittest`, named explicitly                                                                                                                                                               |
| `pm-stack/lane-guard/test_lane_guard.py`                                                                                                  | Python `unittest`, named explicitly                                                                                                                                                               |
| `pm-stack/external-comms/test_push_quarantine.py`                                                                                         | Python `unittest`, named explicitly                                                                                                                                                               |
| `pm-stack/fleet-monitor/`, `pm-stack/scheduled-history/`, `pm-stack/provider-usage/`, `pm-stack/health-hooks/`, `pm-stack/shared-schema/` | Each directory carries its own Python tests in the tree. Which of these the stack workflow runs is UNVERIFIED from the workflow sections read for this chapter; run them in their own directories |
| `pm-stack/scripts/fence-check.test.sh` and `pm-stack/scripts/kaneo-cutover.test.sh`                                                       | Shell suites for the store-migration fence and cutover gate. See [PM stack research and implementation roadmap](program-roadmap.md)                                                               |
| `pm-stack/scripts/tailnet-bindings.test.sh`                                                                                               | Shell suite pinning the Pad and Grist tailnet bindings. The workflow fails the job if the suite reports SKIP, because a skip means the pin did not run                                            |
| Quota exporter suites: `pm-stack/opencode-usage-export/`, `pm-stack/cloudflare-export/`, `pm-stack/meta-exporter/`                        | Node test suites. The workflow records that the usage-export suite needs Node 24, because it imports `node:sqlite`                                                                                |

## The Nix build

Deployment builds the packages, not the checkouts. Since the change recorded as #8171, the PM-stack units no longer run `bun run` out of the working tree; `nixos-rebuild switch` on host `amd` builds and activates the derivations declared in `nixos/services/pm-stack`. See [Repository layout](layout.md) for the four packages and [Running the stack as services](../operations/services.md) for the running form.

`pkgs/pm-aggregator/package.nix` shows the build shape the other packages follow in substance: the derivation takes its version from `pm-stack/aggregator/package.json`; its source fileset is the package file, the lockfile, `src/`, and `scripts/`; a fixed-output derivation runs `bun install` once with network access and the application derivation copies the resulting `node_modules` in beside the source. One entry point is deliberately not packaged: the fleet-feed collector derives repository-root paths from its own file location, so its unit keeps running it from the checkout. The exact `nix build` attribute names for the four PM packages are UNVERIFIED in the sources read for this chapter; the flake is the authority.

The console has no Nix package. Its build and release route is its own scripts (`install.sh`, `redeploy.sh`, the edge deploy, and the Android release units), not a derivation. `nixos/services/pm-stack` declares no console unit: `pm-stack/console/install.sh` copies `pm-console.service` and the `pm-console-expose`, `pm-console-redeploy` and `pm-console-android-release` timers from `pm-stack/systemd/` into `~/.config/systemd/user/` and enables them, and `redeploy.sh` runs that installer for each release (D16, settled in source).

## The CI route

CI in this repository runs on `main` only; pull requests merge without CI and verification lands on the main-branch push. Two workflows carry the PM stack:

1. **`.github/workflows/pm-stack-tests.yml`** — the stack suite. It runs on `workflow_dispatch` and on a nightly schedule (`17 7 * * *`), not on every push, per the repository's nightly-main-CI policy recorded in the workflow header. Its jobs: a provenance job (the aggregator provenance matrix, 5-minute timeout) and a test job (20-minute timeout) that installs and tests the aggregator, the projector, and pm-dispatch with Bun; installs the console edge's type-check tools with Node 24; runs the platform registry test; and then runs the named Python and shell suites above, each asserted to exist before it runs.
2. **`.github/workflows/pm-aggregator-tests.yml`** — a second aggregator workflow exists in the tree. Its trigger and job shape are UNVERIFIED from the sources read for this chapter.

Worker deployment has its own workflows outside these two: the console edge deploys from Actions per the platform registry, and the message-bridge Worker deploys by `message-bridge-deploy.yml`. The decision-events Worker is deployed by hand, per the registry. The per-Worker deploy triggers are UNVERIFIED beyond those registry statements.

## What a green run proves, and what it does not

A green `pm-stack-tests` run proves the named suites passed on the runner at that commit. It does not prove a service is running: the survey's ground rule holds for this chapter — a last-commit date or a passing suite is evidence of repository activity, not of a live process, and runtime liveness is UNVERIFIED unless a dated record states it. The stack's live health evidence is the aggregator's own health page and its twin, not CI. See [The aggregator website, page by page](../reference/website-pages.md).

Sources. Repo files read on the laptop clone at `44aaf73`: `pm-stack/aggregator/package.json`, `pm-stack/console/package.json`, `.github/workflows/pm-stack-tests.yml`, `pkgs/pm-aggregator/package.nix`; survey files `~/workspace/pm-book/survey/apps.md` (§5–§7, §10, §11, §13, §26, §33, §36 and the Nix packages summary) and `~/workspace/pm-book/survey/disagreements.md` (D6).
