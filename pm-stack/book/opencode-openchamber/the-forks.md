# The forks: what they are and why we carry them

The programme carries two forks. The opencode fork is the agent: the program that runs sessions, walks model routes, and holds the quota machinery. The openchamber fork is the control surface: the agentic development environment that presents sessions, settings, and routing controls to the operator. Both sit in the `developing-today-forks` organisation. Both were created on 2026-10-09, two seconds apart.

## Identity

The repository record, from the GitHub API repo objects read in the survey (2026-11-24T00:35–01:05Z):

|                                     | opencode fork                                                                         | openchamber fork                                                                            |
| ----------------------------------- | ------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| Repo                                | [developing-today-forks/opencode](https://github.com/developing-today-forks/opencode) | [developing-today-forks/openchamber](https://github.com/developing-today-forks/openchamber) |
| API `parent`                        | dezren39/opencode                                                                     | dezren39/openchamber                                                                        |
| API `source` (root of fork network) | anomalyco/opencode                                                                    | openchamber/openchamber                                                                     |
| Default branch                      | `main`                                                                                | `main`                                                                                      |
| Org fork created                    | 2026-10-09T04:04:23Z                                                                  | 2026-10-09T04:04:25Z                                                                        |
| Last push at read time              | 2026-11-23T23:54:52Z                                                                  | 2026-11-23T23:16:56Z                                                                        |
| Branches (API)                      | 61                                                                                    | 32                                                                                          |
| Labels defined                      | 168                                                                                   | 150                                                                                         |
| Description                         | "The open source coding agent."                                                       | "Agentic Development Environment based on OpenCode AI agent"                                |

The chain is two-step: upstream → Drew's personal fork (dezren39) → the org fork (developing-today-forks). Both fork READMEs describe the repo as a fork of the upstream project; the API places the personal fork between the org fork and upstream (D51). On the laptop, `~/git/opencode` carries remotes `origin`/`devtoday` = the org fork, `upstream` = dezren39/opencode, `upstream-anomalyco` = anomalyco/opencode; `~/git/openchamber-fork` carries `origin` = the org fork and `upstream` = openchamber/openchamber directly.

## Tracking policy

The fork READMEs (fork-notice blocks at the top of each README, read on the laptop) state the tracking policy:

- The opencode fork tracks upstream's **v2 line on `dev` exclusively**. Upstream promoted v2 to `dev` on 2026-10-10; legacy v1 is on `v1`.
- The openchamber fork tracks upstream `main`.

One reading trap stands in the record: the opencode README's `dev` is upstream's branch name. The fork's own default branch in the API is `main`, and 1,254 of its 1,258 merged pull requests target `main` (D57). A reader who follows the README literally looks for the fork's work on a branch the fork does not default to.

Upstream sync is a standing mechanism, not a one-off: Jules daily upstream-sync workflows, sync-failure tracking issues, and dated sync merges. The README notices do not keep up with it. Both READMEs state the latest upstream commit merged is from Fri Oct 16 2026; the repos show opencode PR #2683 "sync anomalyco/opencode dev (2026-11-22)" and #2690 "sync anomalyco/opencode dev (2026-11-23)", and openchamber PR #2502 "Merge upstream openchamber main (v2.8.0)" (2026-11-22) and #2510 "Merge upstream openchamber main (3a0dbeafb4)…" (2026-11-23). The README notices are five weeks stale at the survey read (D50). **Update (2026-11-25, #13963 slice 1):** both README notices are corrected — they now state the latest upstream commit actually merged (opencode `bab0d0413ff3` via sync PR #2690; openchamber `3a0dbeafb42c` via sync PR #2510, both 2026-11-23), name the personal fork (dezren39) as the direct parent (D51), and describe the opencode fork as syncing upstream `dev` into its `main` default branch (D57). D50, D51 and D57 are settled.

## Why we fork

The programme's own epics state the purpose: the forks exist so the fleet can change the agent and its control surface itself — route pools, quota/plan handling, custom providers, session control — instead of waiting on upstream. The stating issues are [opencode#2319 "epic: opencode fork — platform work"](https://github.com/developing-today-forks/opencode/issues/2319), [openchamber#2153 "Epic: OpenCode platform"](https://github.com/developing-today-forks/openchamber/issues/2153), and the migration epic [opencode#374 "Migrate the fleet to OpenCode 2"](https://github.com/developing-today-forks/opencode/issues/374).

A consolidation ruling (code#5060, cited in opencode#2490/#2491) goes further: merge OpenChamber's server-side control into OpenCode and fix the quota credential in the server credential store. The two forks are intended to converge at the server layer, not merely coexist. The ruling's two fork issues are open; the intention is stated in [Goals and intentions](goals-and-intentions.md) and the edge is drawn in [The forks programme dependency map](dependency-map.md).

## Claimed divergences

Each fork README's fork-notice claims a divergence set. These are the READMEs' claims, recorded as claims.

**The opencode fork claims:** (1) Model route pooling & scheduled weights (`experimental.model_routes`, recurring time-based weight schedules, ordered/round-robin/weighted selection, soft budgets, burst rate-limit continuity, silent-target hedging); (2) indefinite route telemetry & cost accounting (per-request `cost_usd` from catalog pricing); (3) autonomous route review & tuning (`route_stats`/`route_usage` tools, `route_adjust`); (4) plan & quota tracking (`plans.json`, `plan_info` tool, cost tiers cheap/standard/frontier); (5) built-in skills/commands (`/routing-review`, `/plan-update`, `/routing-note`, `/routing-config`); (6) MCPX integration & daemon handover (native MCPX tool projection into Code Mode, detached daemon, JIT secret prompts, socket handover on `serve` restarts).

**The openchamber fork claims:** (1) Model Pools UI & recurring schedules (Settings UI for `experimental.model_routes`); (2) Route Rules interpreter & timed overrides ("Route by sentence"); (3) Plans & Usage section (Settings → Routing → Plans and usage); (4) Review & Update action buttons for `/routing-review` and `/plan-update`; (5) control & workspace enhancements (worktree-by-default sessions, `projects.add`/`projects.remove`, fork-specific release update resolution); (6) MCPX secret management UI (JIT secrets on desktop/web/VS Code/mobile); (7) inter-session messaging (Messages surface) — explicitly "Needs an OpenCode build with messaging (developing-today-forks/opencode)". Item (7) is a cross-fork dependency; it is drawn as an edge in [The forks programme dependency map](dependency-map.md).

## Standing divergences the record shows on its own

- **Merged-PR volume is the divergence.** 1,258 merged PRs on the opencode fork and 1,186 on the openchamber fork at the survey read. Authorship: opencode — dezren39 1,211, developing-today[bot] 42, dependabot[bot] 5; openchamber — dezren39 1,091, developing-today[bot] 86, github-actions[bot] 8, dependabot[bot] 1. The grouped record is [Our changes, shipped and in flight](our-changes.md).
- **Long-lived and standing branches.** The opencode fork (61 branches) carries `spike/2492-tursodb-backend` (the TursoDB spike, opencode#2492), `pin/darwin-nix-opencode-6c43550`, `feat/2490-session-cookie-credential`, `feat/2494-route-overrides-api`, plus a large population of `queue/&lt;task&gt;-…`, `lane-*`, `probe-*` and `ci-*` working branches. The openchamber fork (32 branches) carries six `sync/*-20261122` branches from the 2026-11-22 upstream-sync batch, seven `automation/opencode-update-20261117…20261122` daily branches, `216b-goal-spec-jobs`, and `investigation/2325-dependabot-regression`. Full branch lists are in the survey's deposit.
- **Fleet config is written against the fork.** `~/git/code/.opencode/opencode.jsonc` (code repo, laptop read) is written against fork-only surface: its header records the 2026-11-07 provider rename (opencode#1823) under which "providers the fleet custom-built in the fork" carry a leading underscore (`_claude`, `_antigravity`, `_vibe`, `_commandcode`, `_muse`), the fork-native antigravity provider (opencode#1457, replacing the :18751 proxy per opencode#1414), the fork-native claude-code provider (opencode#1749 / code#6117), and the provider-availability system (opencode#1828). None of that exists in upstream. The deployed pools and routes that config names are recorded in [Pools, routes and the switchboard](../llm-plans/pools-and-routes.md); this chapter records only that the config's surface is the fork's.

Sources. Survey file `~/workspace/pm-book/survey/oc-programme.md` §1.1–§1.4 and §6 items 1, 2, 8 (cited in this part as D50, D51, D57); GitHub API repo objects and laptop README reads as recorded in the survey; deposit `~/workspace/pm-book/sideline-material/oc-survey/` (`repo-opencode.json`, `repo-openchamber.json`, `branches-oc.json`, `branches-och.json`, `labels-oc.json`, `labels-och.json`, `laptop-readme-excerpts.txt`).
