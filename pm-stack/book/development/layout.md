# Repository layout

The PM stack lives in one repository, `developing-today/code`. Its application tree is `pm-stack/` at the repository root. Its Nix packages are four directories under `pkgs/`. This book lives inside the subject tree, at `pm-stack/book/` with its configuration at `pm-stack/book.toml`. A change to the stack and the change to this book land in the same repository, in the same pull request. See [Keeping this book current](keeping-this-book-current.md).

## The subject tree: `pm-stack/`

`pm-stack/` holds the services, the Workers, the operator scripts, the unit files, the design corpus for the stack, and this book. The deployment definition is `pm-stack/README.md`; the port registry is `pm-stack/PORTS.md` (audited 2026-11-05). One `nixos-rebuild switch` on laptop host `amd` deploys the stack; the NixOS module that declares the main user units is `nixos/services/pm-stack` in the same repository, outside `pm-stack/` itself.

The 36 components the survey counted in `pm-stack/` are the layout guide for the tree. The count is 36 sections (§1–§36 in the survey): every subdirectory named in the programme brief, plus the subdirectories and loose scripts present in the tree that the brief did not name.

### Core services

| Directory              | What lives there                                                                                                                                                                                                                                                               |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `pm-stack/aggregator/` | The dashboard. A Bun app: about 180 modules in `src/` (pages, health, quota, decisions, prompts, alerts, sources), producer and maintenance scripts in `scripts/`, `opencode-tools/`, unit files in `systemd/`, and its test suite in `test/`. Port 4870, unit `pm-aggregator` |
| `pm-stack/console/`    | The phone and owner app. A Bun server in `src/`, the single-page app in `public/`, the edge Worker in `edge/`, the Android wrapper in `android/`, and its tests in `test/`. Port 4872, unit `pm-console`. No Nix package exists for the console; see the packages table below  |
| `pm-stack/projector/`  | The projector and link registry. A Bun service: webhook intake, reconcile, the link registry, SQL schema in `schema.sql`, tests in `test/`. Port 7801. See [The projector and projections](../concepts/projector.md)                                                           |
| `pm-stack/dispatch/`   | pm-dispatch. A Bun intake service in `src/`, a worked caller example in `examples/`, tests in `test/`. Port 4879, public as `dispatch.security.cab`                                                                                                                            |

### Integrated external apps

Each integrated app keeps its own directory, with its own compose file, seed, and deploy script. The stack did not write these applications; it configures and reads them. See [The Kaneo sync and the other PM apps](../concepts/kaneo-sync.md).

| Directory                                                                 | App                                                                                                                                                                                                         | Port |
| ------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---- |
| `pm-stack/` root: `compose.yml`, `kaneo-projects.json`, `connect-repo.sh` | Kaneo, the execution board, with its Postgres 16 sidecar. The compose file pins the upstream image v2.47.0 by digest; the fork is not the deployed image (D1)                                               | 5173 |
| `pm-stack/grist/`                                                         | Grist, the roadmap tables: `compose.yml`, `seed/`, `seed.sh`, `deploy.sh`, choice-option sync tooling. Its compose file pins by digest with no tag; the README's v1.7.21 for that digest is UNVERIFIED (D2) | 8484 |
| `pm-stack/pad/`                                                           | Pad, the planning layer: `compose.yml`, seed plans in `seed/`. The aggregator reads no Pad planning data (D3)                                                                                               | 7777 |
| `pm-stack/ready-for-agent/`                                               | ready-for-agent, the dispatch UI layer: `config.json`, `install.sh`, `apply-config.sh`, unit files in `systemd/`                                                                                            | 6056 |

### Workers and the platform registry

| Directory                          | What lives there                                                                                                                                                                                                               |
| ---------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `pm-stack/decision-events-worker/` | The `decision-events` Worker: `src/`, `schema.sql`, `test/`, `wrangler.toml`. In service, deployed by hand                                                                                                                     |
| `pm-stack/voice-line-worker/`      | The `voice-line` Worker: `src/`, `test/`, `wrangler.toml`. In service                                                                                                                                                          |
| `pm-stack/notify-relay/`           | The `pm-notify` relay Worker: `src/`, `test/`, `wrangler.toml`. Recorded in the registry as not deployed, address null (D5)                                                                                                    |
| `pm-stack/console/edge/`           | The `pm-console-edge` Worker, inside the console tree rather than at the top level. Deployed from Actions                                                                                                                      |
| `pm-stack/platform/`               | Not a service. The Workers registry: `workers.json` (20 Workers, checked 2026-11-23), its README, and `workers.test.ts`, which holds the list to the repository's wrangler files. The README's count of eighteen is stale (D6) |

### Execution, guards, and monitors

| Directory or file                   | What lives there                                                                                                                                                                |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pm-stack/opencode-claimant/`       | The OpenCode claimant lane: `claimant.py`, `launch_one.py`, `poll.py`, migrations, and a large test suite. See [The claimant: keeping sessions honest](../concepts/claimant.md) |
| `pm-stack/opencode-scheduled-jobs/` | Scheduled OpenCode jobs: `jobs.json` and the one pilot job, `quota-freshness-summary`                                                                                           |
| `pm-stack/opencode-usage-export/`   | The usage exporter that reads the OpenCode session store                                                                                                                        |
| `pm-stack/lane-guard/`              | `lane_guard.py`: deterministic ownership and park-note checks for queue lanes, with tests                                                                                       |
| `pm-stack/health-hooks/`            | Failure-triggered, read-only troubleshoot-and-report hooks: `run.py` and its wrappers                                                                                           |
| `pm-stack/fleet-monitor/`           | Two report-only Python monitors, with tests                                                                                                                                     |
| `pm-stack/kaneo-sync-guard.py`      | Loose script at the `pm-stack/` root: asserts and heals the Kaneo GitHub integration's status transitions                                                                       |
| `pm-stack/closure-watchdog.py`      | Loose script: reopens automation closes that break the sub-issue invariant. Its scheduler is a Muse sandbox cron job, outside the laptop stack                                  |
| `pm-stack/invariant-sweeper.py`     | Loose script: the slow-cadence sweep for closed-with-open-children, open-with-done-status, and Kaneo–GitHub mismatch                                                            |
| `pm-stack/host-health-collector.py` | Loose script: writes the host-health snapshot the aggregator reads, on a 5-minute timer                                                                                         |

### Quota, usage, and exporters

| Directory                     | What lives there                                                                                                                           |
| ----------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `pm-stack/exporters/`         | The browser-dependent quota watchdog core. The only adapter present is a synthetic fixture; live per-provider production use is UNVERIFIED |
| `pm-stack/cloudflare-export/` | `cloudflare-quota-export.mjs` with service and timer units                                                                                 |
| `pm-stack/meta-exporter/`     | `meta-quota-export.mjs` with service and timer units                                                                                       |
| `pm-stack/provider-usage/`    | `check-usage.py` and `reconcile.py`: the provider and fleet snapshot a failover job consults                                               |
| `pm-stack/quota-check/`       | One helper, `turso-mirror-read.py`: per-database libsql HTTP reads of the `fleet-platform` and `fleet-mirror` stores                       |
| `pm-stack/scheduled-history/` | `collect.py` and `watchdog.py`: the reconciler for OpenChamber scheduled-task runs                                                         |

### Contracts, ledgers, and shared state

| Directory                  | What lives there                                                                                                                                                   |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `pm-stack/shared-schema/`  | The shared-store contract layer: `ORCHESTRATION.md`, the orchestration writer, `migrations/`, and its tests. See [The shared schema](../concepts/shared-schema.md) |
| `pm-stack/external-comms/` | The external-comms ledger and pusher: SQL migrations, the record CLI, the pusher unit files, and its quarantine test                                               |

### Operator scripts, units, and smoke tooling

| Directory or file                 | What lives there                                                                                                                                                                                                                                                                                                   |
| --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `pm-stack/scripts/`               | Operator scripts: `pm-git-sync.sh`, `kaneo-cutover.sh`, `fence-check.sh`, `restore-rehearsal.sh`, and their shell test suites                                                                                                                                                                                      |
| `pm-stack/systemd/`               | Unit files for the aggregator, console, dispatch, and tunnel, plus timers for host health, the invariant sweeper, the Kaneo sync guard, the quota watchdog, and the console expose, redeploy, and Android-release jobs. Which of these copies are installed as-is and which the NixOS module renders is UNVERIFIED |
| `pm-stack/private-laptop-bridge/` | Cloudflare-side bridge smoke tooling and its tests. The production surface it serves beyond smoke tests is UNVERIFIED                                                                                                                                                                                              |
| `pm-stack/tests/`                 | The stack-level Python suites: `test_kaneo_sync_guard.py`, `test_closure_watchdog.py`, `test_host_health_collector.py`, `test_invariant_sweeper_timeouts.py`                                                                                                                                                       |
| `pm-stack/docs/`                  | A directory the survey did not enumerate in detail; its last commit in the survey window is 2026-11-18, and its content is UNVERIFIED. The stack's design corpus lives at the repository root's `doc/` and as loose documents at the `pm-stack/` root, not here                                                    |

### Loose documents and configuration at the `pm-stack/` root

The root of `pm-stack/` carries the stack's own record: `README.md` (deployment definition), `PORTS.md` (port registry), `compose.yml`, `deploy.sh`, `connect-repo.sh`, `github-app-manifest.json`, `BOOTSTRAP-CREDENTIALS.md`, `COLD-RESTORE-RUNBOOK.md`, `KANEO-GITHUB-SYNC.md`, `kaneo-projects.json`, and the design documents the development chapters synthesise: `design-register-2026-11-11.md`, `design-tracking-manifest-2026-11-11.md`, `lineage-manifest-2026-11-12.md`, `design-reprioritization-2026-11-12.md`, `SHARED-SCHEMA-MAP.md`, `SHARED-SCHEMA-MIGRATION-PLAN.md`, `STORE-MIGRATION-PLAN-6816.md`, `decisions-ledger-system.md`, `needs-user-actions-design-2026-10-26.md`, and `project-tracking-evaluation-2026-10-23.md`. See [PM stack research and implementation roadmap](program-roadmap.md).

## The Nix packages: `pkgs/pm-*`

Four packages under `pkgs/` wrap `pm-stack/` sources as Nix derivations. Since the change recorded as #8171, every PM-stack app in this set is a Nix derivation; the units no longer run `bun run` out of the checkout.

| Package                          | Wraps                             | Notes                                                                                                                                                                                                                                                                                                                                                                   |
| -------------------------------- | --------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pkgs/pm-aggregator/package.nix` | `pm-stack/aggregator/`            | One package, four entry points: the server plus the quota gatherer, the digest generator, and the Turso-store exporter. The fleet-feed collector is not wrapped here; its unit runs it from the checkout because it derives repository-root paths from its own file location. The package bundles `pm-stack/kaneo-projects.json` and `pm-stack/grist/seed/roadmap.json` |
| `pkgs/pm-projector/package.nix`  | `pm-stack/projector/`             | The projector service                                                                                                                                                                                                                                                                                                                                                   |
| `pkgs/pm-dispatch/package.nix`   | `pm-stack/dispatch/`              | The dispatch service                                                                                                                                                                                                                                                                                                                                                    |
| `pkgs/pm-git-sync/package.nix`   | `pm-stack/scripts/pm-git-sync.sh` | A script wrapper, version 0.3.0 in the surveyed tree                                                                                                                                                                                                                                                                                                                    |

There is no `pkgs/pm-console`. The console deploys through `pm-stack/console/install.sh` and `redeploy.sh` and through its edge and Android release paths. Whether the NixOS module builds the console by another route is UNVERIFIED.

## This book: `pm-stack/book/`

The book lives with its subject. `pm-stack/book.toml` is the mdBook configuration; `pm-stack/book/` holds `SUMMARY.md` and the section folders — getting-started, concepts, reference, tutorials, library, development, operations — in the shape of the fleet book at `pkgs/fleet/book/` with `pkgs/fleet/book.toml`. This chapter is `pm-stack/book/development/layout.md` in the landed tree.

Two placement facts belong in this chapter because other records state them wrong. The programme issue #13900 as filed, and the coverage ledger's summary table, place this book at `pkgs/pm-stack/docs/`. The coordinator correction of 2026-11-23, verified against the exemplar books on main, places it at `pm-stack/book/`. The correction wins; the ledger table is stale on this point (D15).

Neighbouring books keep the same shape in their own subject trees: the fleet book at `pkgs/fleet/book/`, the Atlas book at `pkgs/atlas-search/book/`, the ID book at `pkgs/id/book/`. The mcpx book at `pkgs/mcpx/docs/` is the older exception.

## Where the rest of the stack's record lives

- `doc/` at the repository root: the dated design-record corpus, including the canonical PM-stack design of 2026-10-24 and the Turso-to-D1 migration audit of 2026-11-23. See [Project Management: prioritised issue map and dependency map](issue-map.md).
- `nixos/services/pm-stack`: the deploy-time declaration of the main units.
- `.github/workflows/`: the build and test workflows, including `pm-stack-tests.yml` and `pm-aggregator-tests.yml`. See [Building and testing](building-and-testing.md).
- GitHub Project #4, "Developing Today": the portfolio record the aggregator reads. It is data the stack consumes, not a directory in this repository.

Sources. Survey file `~/workspace/pm-book/survey/apps.md` (§1–§36 and the Nix packages summary); `~/workspace/pm-book/survey/disagreements.md` (D1, D2, D3, D5, D6); `~/workspace/pm-book/coverage-ledger.md`; `~/workspace/pm-book/placement-correction.md`; repo paths `pm-stack/`, `pm-stack/README.md`, `pm-stack/PORTS.md`, `pkgs/pm-aggregator/package.nix`, `pkgs/pm-dispatch/`, `pkgs/pm-projector/`, `pkgs/pm-git-sync/`, read on the laptop clone at `44aaf73` for the package list.
