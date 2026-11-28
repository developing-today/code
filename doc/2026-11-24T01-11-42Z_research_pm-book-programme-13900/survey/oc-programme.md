# OpenCode + OpenChamber forks programme — survey (OC-SURVEY)

Survey date: **2026-11-23 (America/Chicago)**; API data read 2026-11-24T00:35–01:05Z via `fleet gh api`, fully paginated, counts computed from the responses. Laptop file reads via `~/tools/laptop-ssh.sh` against `~/git/code`, `~/git/opencode` and `~/git/openchamber-fork` (no fresh clones). Raw material for this survey is deposited in `~/workspace/pm-book/sideline-material/oc-survey/` (see §7). This section is written to be liftable into a standalone OpenCode/OpenChamber book later.

**Counts at a glance** — opencode fork: **1258 merged PRs** (of 1438 closed, 9 open), **30 open issues**. openchamber fork: **1186 merged PRs** (of 1290 closed, 2 open), **30 open issues**. code repo programme set (open, title-matched, defined in §3): **91 open issues**. Search totals for context: `repo:developing-today/code opencode` = 4,147 items (all states, issues+PRs, body mentions included) — the programme's code footprint is far larger than the title-matched set; the raw title-search dumps (681 opencode / 137 openchamber / 148 claimant items) are in the deposit.

---

## 1. The two forks

### 1.1 Identity (from the GitHub API repo objects)

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
| Description                         | “The open source coding agent.”                                                       | “Agentic Development Environment based on OpenCode AI agent”                                |

So the chain is two-step: upstream → Drew's personal fork (dezren39) → the org fork (developing-today-forks). Both org forks were created two seconds apart on 2026-10-09. On the laptop, `~/git/opencode` carries remotes `origin`/`devtoday` = the org fork, `upstream` = dezren39/opencode, `upstream-anomalyco` = anomalyco/opencode; `~/git/openchamber-fork` carries `origin` = the org fork and `upstream` = openchamber/openchamber directly.

### 1.2 Why we fork (as stated in the repos)

- The fork READMEs (fork-notice blocks at the top of each README, read on the laptop) state the tracking policy: opencode tracks upstream's **v2 line on `dev` exclusively** (upstream promoted v2 to `dev` on 2026-10-10; legacy v1 is on `v1`); openchamber tracks upstream `main`.
- The programme's own epics state the purpose: the forks exist so the fleet can change the agent and its control surface itself — route pools, quota/plan handling, custom providers, session control — instead of waiting on upstream. See [opencode#2319 “epic: opencode fork — platform work”](https://github.com/developing-today-forks/opencode/issues/2319), [openchamber#2153 “Epic: OpenCode platform”](https://github.com/developing-today-forks/openchamber/issues/2153), and the migration epic [opencode#374 “Migrate the fleet to OpenCode 2”](https://github.com/developing-today-forks/opencode/issues/374).
- A consolidation ruling (code#5060, cited in opencode#2490/#2491) goes further: merge OpenChamber's server-side control into OpenCode and fix the quota credential in the server credential store — i.e. the two forks are intended to converge at the server layer, not merely coexist.

### 1.3 Claimed divergences (fork README fork-notices)

**opencode fork claims:** (1) Model route pooling & scheduled weights (`experimental.model_routes`, recurring time-based weight schedules, ordered/round-robin/weighted selection, soft budgets, burst rate-limit continuity, silent-target hedging); (2) indefinite route telemetry & cost accounting (per-request `cost_usd` from catalog pricing); (3) autonomous route review & tuning (`route_stats`/`route_usage` tools, `route_adjust`); (4) plan & quota tracking (`plans.json`, `plan_info` tool, cost tiers cheap/standard/frontier); (5) built-in skills/commands (`/routing-review`, `/plan-update`, `/routing-note`, `/routing-config`); (6) MCPX integration & daemon handover (native MCPX tool projection into Code Mode, detached daemon, JIT secret prompts, socket handover on `serve` restarts).

**openchamber fork claims:** (1) Model Pools UI & recurring schedules (Settings UI for `experimental.model_routes`); (2) Route Rules interpreter & timed overrides (“Route by sentence”); (3) Plans & Usage section (Settings → Routing → Plans and usage); (4) Review & Update action buttons for `/routing-review` and `/plan-update`; (5) control & workspace enhancements (worktree-by-default sessions, `projects.add`/`projects.remove`, fork-specific release update resolution); (6) MCPX secret management UI (JIT secrets on desktop/web/VS Code/mobile); (7) inter-session messaging (Messages surface) — explicitly “Needs an OpenCode build with messaging (developing-today-forks/opencode)” (a cross-fork dependency, §5).

### 1.4 Standing divergences evidenced independently of the READMEs

- **Merged-PR volume is the divergence.** 1258 merged PRs on the opencode fork and 1186 on the openchamber fork, almost all authored by `dezren39` (opencode: [('dezren39', 1211), ('developing-today[bot]', 42), ('dependabot[bot]', 5)]; openchamber: [('dezren39', 1091), ('developing-today[bot]', 86), ('github-actions[bot]', 8), ('dependabot[bot]', 1)]), the remainder by `developing-today[bot]`, `github-actions[bot]` and `dependabot[bot]`. Full enumeration in §1.5.
- **Long-lived / standing branches.** opencode fork (61 branches): `spike/2492-tursodb-backend` (the TursoDB spike, opencode#2492), `pin/darwin-nix-opencode-6c43550`, `feat/2490-session-cookie-credential`, `feat/2494-route-overrides-api`, plus a large population of `queue/<task>-…`, `lane-*`, `probe-*` and `ci-*` working branches. openchamber fork (32 branches): six `sync/*-20261122` branches from the 2026-11-22 upstream-sync batch, seven `automation/opencode-update-20261117…20261122` daily branches, `216b-goal-spec-jobs`, `investigation/2325-dependabot-regression`. Full branch lists are in the deposit.
- **Fleet config against the fork (code repo, laptop read).** `~/git/code/.opencode/opencode.jsonc` is written against fork-only surface: its header records the 2026-11-07 provider rename (opencode#1823) under which “providers the fleet custom-built in the fork” carry a leading underscore (`_claude`, `_antigravity`, `_vibe`, `_commandcode`, `_muse`), the fork-native antigravity provider (opencode#1457, replacing the :18751 proxy per opencode#1414), the fork-native claude-code provider (opencode#1749 / code#6117), and the provider-availability system (opencode#1828). None of that exists in upstream.
- **Upstream sync is a standing mechanism, not a one-off:** Jules daily upstream-sync workflows, sync-failure tracking issues, and dated sync merges (latest in §6, item 1).

### 1.5 Merged PRs, grouped by theme

Grouping rule: theme = the conventional-commit scope in the PR title (`feat(scope):`, `fix(scope):`, …); titles without a scope are grouped by the first matching keyword (route/quota/provider/session/tui/console/plugin/nix/ci/test/doc/upstream), else “other/no-scope”. “Merged” = `merged_at` non-null in the API response. Shipped = merged into `main` (opencode: 1,254 of 1,258 merged PRs target `main`; the 4 exceptions target `sweep/560-provider-models`, `spike/2492-tursodb-backend` and `pin/darwin-nix-opencode-6c43550`). Everything in this section is **shipped**; intentions live in §2/§3, not here.

#### developing-today-forks/opencode — 1258 merged PRs by theme

| Theme                  | Merged PRs |
| ---------------------- | ---------- |
| core                   | 423        |
| ai                     | 181        |
| ci                     | 125        |
| cli                    | 49         |
| docs                   | 47         |
| roadmap                | 47         |
| server                 | 42         |
| schema                 | 26         |
| script                 | 24         |
| tui                    | 19         |
| gui-extensions         | 17         |
| client                 | 15         |
| app                    | 14         |
| desktop                | 14         |
| console                | 14         |
| nix                    | 13         |
| util                   | 10         |
| plugins/mcp            | 9          |
| web                    | 8          |
| specs                  | 7          |
| other/no-scope         | 7          |
| deps                   | 7          |
| quota                  | 6          |
| facade                 | 6          |
| plugin-github          | 5          |
| function               | 5          |
| core,ai                | 5          |
| codemode               | 4          |
| providers              | 4          |
| routes                 | 4          |
| www                    | 4          |
| protocol               | 3          |
| plugin-browser         | 3          |
| config                 | 3          |
| flake                  | 3          |
| routes/pools           | 3          |
| nix/packaging          | 3          |
| console,stats          | 3          |
| deps-dev               | 3          |
| legacy                 | 2          |
| session                | 2          |
| session-ui             | 2          |
| core,server            | 2          |
| util,core              | 2          |
| ui,gui-extensions      | 2          |
| lint                   | 2          |
| core,schema            | 2          |
| http-recorder          | 2          |
| stats                  | 2          |
| changelog              | 2          |
| cli,tui                | 2          |
| client,httpapi-codegen | 2          |
| plugin                 | 2          |
| sdk                    | 2          |
| cli-rs                 | 1          |
| ai/quota               | 1          |
| tui,codemode,ci        | 1          |
| update                 | 1          |
| ui                     | 1          |
| a11y                   | 1          |
| resolver               | 1          |
| stats-app              | 1          |
| console-app            | 1          |
| astro                  | 1          |
| ci,core                | 1          |
| app,session-ui         | 1          |
| quota/plans            | 1          |
| core,server,tui        | 1          |
| cost                   | 1          |
| util,tui               | 1          |
| test                   | 1          |
| tool                   | 1          |
| vibe                   | 1          |
| pkg                    | 1          |
| app,tui                | 1          |
| files                  | 1          |
| tests                  | 1          |
| cli,server             | 1          |
| ai,tui,cli             | 1          |
| httpapi-codegen        | 1          |
| mcpx                   | 1          |
| github                 | 1          |
| storage                | 1          |
| auth                   | 1          |
| latex                  | 1          |
| cli,client             | 1          |
| client,app             | 1          |
| core,ai,tui,cli        | 1          |
| core,app               | 1          |
| tui,cli                | 1          |
| security               | 1          |
| provider               | 1          |
| cli,core               | 1          |
| services               | 1          |
| upstream sync          | 1          |
| desktop,core           | 1          |
| acp                    | 1          |

##### developing-today-forks/opencode — theme: core (423)

| PR                                                                    | Title                                                                                                                 | Author                | Merged     |
| --------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------- | --------------------- | ---------- |
| [#15](https://github.com/developing-today-forks/opencode/pull/15)     | feat(core): inter-session messaging, session chain and logs, local semantic search                                    | dezren39              | 2026-10-17 |
| [#25](https://github.com/developing-today-forks/opencode/pull/25)     | test(core): cover plugin recovery after an entrypoint load failure                                                    | dezren39              | 2026-10-17 |
| [#27](https://github.com/developing-today-forks/opencode/pull/27)     | fix(core): reload plugins when file policies change                                                                   | dezren39              | 2026-10-17 |
| [#28](https://github.com/developing-today-forks/opencode/pull/28)     | test(core): pin shell create.before env isolation                                                                     | dezren39              | 2026-10-17 |
| [#30](https://github.com/developing-today-forks/opencode/pull/30)     | fix(core): stop mcpx plugin leaking detached daemons from tests                                                       | dezren39              | 2026-10-17 |
| [#31](https://github.com/developing-today-forks/opencode/pull/31)     | fix(core): honour zone designator in route reset timestamps                                                           | dezren39              | 2026-10-18 |
| [#32](https://github.com/developing-today-forks/opencode/pull/32)     | feat(core): configurable shell timeout via experimental.shell_timeout                                                 | dezren39              | 2026-10-18 |
| [#39](https://github.com/developing-today-forks/opencode/pull/39)     | feat(core): let subagents launch nested subagents                                                                     | dezren39              | 2026-10-18 |
| [#42](https://github.com/developing-today-forks/opencode/pull/42)     | fix(core): discover Copilot plan endpoint for migrated credentials                                                    | dezren39              | 2026-10-18 |
| [#45](https://github.com/developing-today-forks/opencode/pull/45)     | fix(core): forward prompt cache key to Copilot models via the AI SDK bridge                                           | dezren39              | 2026-10-18 |
| [#56](https://github.com/developing-today-forks/opencode/pull/56)     | fix(core): use Effect.orElseSucceed for constant failure recovery                                                     | dezren39              | 2026-10-18 |
| [#66](https://github.com/developing-today-forks/opencode/pull/66)     | test(core): expect compact JSON in callable namespace test                                                            | dezren39              | 2026-10-18 |
| [#68](https://github.com/developing-today-forks/opencode/pull/68)     | fix(core): validate OPENCODE_MCPX_DAEMON_IDLE_EXIT before spawning the mcpx daemon                                    | dezren39              | 2026-10-18 |
| [#80](https://github.com/developing-today-forks/opencode/pull/80)     | fix(core): match Go ParseDuration exactly in the mcpx idle-exit validator                                             | dezren39              | 2026-10-18 |
| [#88](https://github.com/developing-today-forks/opencode/pull/88)     | fix(core): guard subagent depth walk against parent cycles                                                            | dezren39              | 2026-10-19 |
| [#91](https://github.com/developing-today-forks/opencode/pull/91)     | fix(core): restore subagent_depth 0 disabling subagents from top-level sessions                                       | dezren39              | 2026-10-19 |
| [#93](https://github.com/developing-today-forks/opencode/pull/93)     | test(core): skip mcpx ensureDaemon spawn test on Windows                                                              | dezren39              | 2026-10-19 |
| [#98](https://github.com/developing-today-forks/opencode/pull/98)     | fix(core): guard messaging graph walks against parent cycles                                                          | dezren39              | 2026-10-19 |
| [#126](https://github.com/developing-today-forks/opencode/pull/126)   | fix(core): provision ripgrep for unit tests instead of falling through to a network download                          | dezren39              | 2026-10-19 |
| [#131](https://github.com/developing-today-forks/opencode/pull/131)   | fix(core): keep shell timeout description current after config edits                                                  | dezren39              | 2026-10-19 |
| [#135](https://github.com/developing-today-forks/opencode/pull/135)   | fix(core): enforce subagent_depth for subtask commands                                                                | dezren39              | 2026-10-19 |
| [#140](https://github.com/developing-today-forks/opencode/pull/140)   | fix(core): honour mcpx daemon.autostart in ensureDaemon                                                               | dezren39              | 2026-10-19 |
| [#186](https://github.com/developing-today-forks/opencode/pull/186)   | test(core): cover mcpx daemon autostart fail-open branches                                                            | dezren39              | 2026-10-20 |
| [#205](https://github.com/developing-today-forks/opencode/pull/205)   | chore(core): bump @modelcontextprotocol/* to 2.3.0 and re-base the client patch                                       | dezren39              | 2026-10-21 |
| [#220](https://github.com/developing-today-forks/opencode/pull/220)   | fix(core): group a session tree's model requests under its root session                                               | dezren39              | 2026-10-21 |
| [#232](https://github.com/developing-today-forks/opencode/pull/232)   | feat(core): support {env:VAR:-fallback} defaults in config substitution                                               | dezren39              | 2026-10-21 |
| [#234](https://github.com/developing-today-forks/opencode/pull/234)   | fix(core): time route attempts and pool health to the end of the model's response                                     | dezren39              | 2026-10-21 |
| [#261](https://github.com/developing-today-forks/opencode/pull/261)   | fix(core): apply opencode-route provider settings to built pools                                                      | dezren39              | 2026-10-22 |
| [#266](https://github.com/developing-today-forks/opencode/pull/266)   | fix(core): keep pool variants when route provider settings apply                                                      | dezren39              | 2026-10-22 |
| [#373](https://github.com/developing-today-forks/opencode/pull/373)   | feat(core): use several accounts of one provider at once with instanceOf                                              | dezren39              | 2026-10-24 |
| [#377](https://github.com/developing-today-forks/opencode/pull/377)   | fix(core): keep a Console instance's policy refusal across failed reloads                                             | dezren39              | 2026-10-24 |
| [#390](https://github.com/developing-today-forks/opencode/pull/390)   | fix(core): expand bare model aliases in the session runner                                                            | dezren39              | 2026-10-24 |
| [#453](https://github.com/developing-today-forks/opencode/pull/453)   | test(core): model experimental.aliases relocation in ConfigNormalize property test                                    | dezren39              | 2026-10-25 |
| [#462](https://github.com/developing-today-forks/opencode/pull/462)   | fix(core): apply provider baseURL to all catalog models                                                               | dezren39              | 2026-10-25 |
| [#468](https://github.com/developing-today-forks/opencode/pull/468)   | feat(core): subsession model governance                                                                               | dezren39              | 2026-10-26 |
| [#482](https://github.com/developing-today-forks/opencode/pull/482)   | fix(core): ignore calendar-invalid release dates                                                                      | dezren39              | 2026-10-26 |
| [#486](https://github.com/developing-today-forks/opencode/pull/486)   | fix(core): run the governance check on switchAgent for child sessions                                                 | dezren39              | 2026-10-26 |
| [#487](https://github.com/developing-today-forks/opencode/pull/487)   | fix(core): enforce override subagent lists in model governance                                                        | dezren39              | 2026-10-26 |
| [#509](https://github.com/developing-today-forks/opencode/pull/509)   | fix(core): validate models-dev release dates with parseReleaseDate                                                    | dezren39              | 2026-10-26 |
| [#511](https://github.com/developing-today-forks/opencode/pull/511)   | fix(core): apply Selection segment rules in v1 model migration                                                        | dezren39              | 2026-10-26 |
| [#512](https://github.com/developing-today-forks/opencode/pull/512)   | fix(core): judge route targets at their effective variant                                                             | dezren39              | 2026-10-26 |
| [#513](https://github.com/developing-today-forks/opencode/pull/513)   | fix(core): apply provider baseURL across config documents                                                             | dezren39              | 2026-10-26 |
| [#514](https://github.com/developing-today-forks/opencode/pull/514)   | fix(core): keep unmentioned list groups under force overrides                                                         | dezren39              | 2026-10-26 |
| [#518](https://github.com/developing-today-forks/opencode/pull/518)   | fix(core): reject out-of-range seconds in route schedule times                                                        | dezren39              | 2026-10-26 |
| [#524](https://github.com/developing-today-forks/opencode/pull/524)   | fix(core): reject route schedule windows with unreadable day entries                                                  | dezren39              | 2026-10-26 |
| [#525](https://github.com/developing-today-forks/opencode/pull/525)   | fix(core): parse short-form relative quota resets (in 45s)                                                            | dezren39              | 2026-10-26 |
| [#537](https://github.com/developing-today-forks/opencode/pull/537)   | fix(core): average the two middles in ModelRouteTuning median                                                         | dezren39              | 2026-10-26 |
| [#539](https://github.com/developing-today-forks/opencode/pull/539)   | fix(core): drop expired scheduled overrides on save; drop parse entries with non-string scope lists                   | dezren39              | 2026-10-26 |
| [#545](https://github.com/developing-today-forks/opencode/pull/545)   | fix(core): honor variant remaps for config-pinned route variants                                                      | dezren39              | 2026-10-27 |
| [#544](https://github.com/developing-today-forks/opencode/pull/544)   | fix(core): preserve **proto** entries in config alias resolution                                                      | dezren39              | 2026-10-27 |
| [#554](https://github.com/developing-today-forks/opencode/pull/554)   | feat(core): let a model route cap its presented context limit                                                         | dezren39              | 2026-10-27 |
| [#572](https://github.com/developing-today-forks/opencode/pull/572)   | fix(core): extend redactBody to common credential formats                                                             | dezren39              | 2026-10-27 |
| [#569](https://github.com/developing-today-forks/opencode/pull/569)   | fix(core): judge route governance targets via shared catalog expansion (#530)                                         | dezren39              | 2026-10-27 |
| [#575](https://github.com/developing-today-forks/opencode/pull/575)   | fix(core): read decimal and compound relative quota resets                                                            | dezren39              | 2026-10-27 |
| [#573](https://github.com/developing-today-forks/opencode/pull/573)   | fix(core): allow single-digit seconds in ModelRouteSchedule.minutesOf (#528)                                          | dezren39              | 2026-10-27 |
| [#581](https://github.com/developing-today-forks/opencode/pull/581)   | fix(core): accept zero-padded weekday strings in route schedule parse (#531)                                          | dezren39              | 2026-10-27 |
| [#584](https://github.com/developing-today-forks/opencode/pull/584)   | test(core): pin concrete route-target variant filtering in governance (#519)                                          | dezren39              | 2026-10-27 |
| [#586](https://github.com/developing-today-forks/opencode/pull/586)   | fix(core): attribute wrapping schedule window tails by calendar date (#585)                                           | dezren39              | 2026-10-27 |
| [#593](https://github.com/developing-today-forks/opencode/pull/593)   | fix(core): unify absolute-time keywords across quota-reset parsers (#532)                                             | dezren39              | 2026-10-27 |
| [#552](https://github.com/developing-today-forks/opencode/pull/552)   | fix(core): stop env substitution chaining into {file:...} reads                                                       | dezren39              | 2026-10-27 |
| [#582](https://github.com/developing-today-forks/opencode/pull/582)   | fix(core): scan past an unparseable first quota reset date                                                            | dezren39              | 2026-10-27 |
| [#598](https://github.com/developing-today-forks/opencode/pull/598)   | fix(core): skip env substitution on // comment lines in config (#547)                                                 | dezren39              | 2026-10-27 |
| [#599](https://github.com/developing-today-forks/opencode/pull/599)   | fix(core): apply route context cap to presented input limit (#559)                                                    | dezren39              | 2026-10-27 |
| [#603](https://github.com/developing-today-forks/opencode/pull/603)   | refactor(core): extract shared governance catalog helper (#591)                                                       | dezren39              | 2026-10-27 |
| [#606](https://github.com/developing-today-forks/opencode/pull/606)   | fix(core): drop overrides whose schedule is present but malformed (#570)                                              | dezren39              | 2026-10-27 |
| [#602](https://github.com/developing-today-forks/opencode/pull/602)   | fix(core): observer retryable uses canonical isRetryable                                                              | dezren39              | 2026-10-27 |
| [#614](https://github.com/developing-today-forks/opencode/pull/614)   | fix(core): distinct cycle and depth errors in MessagingTemplate.render (#579)                                         | dezren39              | 2026-10-27 |
| [#610](https://github.com/developing-today-forks/opencode/pull/610)   | fix(core): check route target until before nested-route expansion (#580)                                              | dezren39              | 2026-10-27 |
| [#615](https://github.com/developing-today-forks/opencode/pull/615)   | feat(core): QuotaSnapshot derivation with a single staleness contract (#588)                                          | dezren39              | 2026-10-27 |
| [#612](https://github.com/developing-today-forks/opencode/pull/612)   | fix(core): count only completed attempts in route_stats speed median (#578)                                           | dezren39              | 2026-10-27 |
| [#616](https://github.com/developing-today-forks/opencode/pull/616)   | fix(core): reject equal start/end in ModelRouteSchedule.parse (#555)                                                  | dezren39              | 2026-10-27 |
| [#620](https://github.com/developing-today-forks/opencode/pull/620)   | fix(core): do not expand references in trailing // comments (#556)                                                    | dezren39              | 2026-10-27 |
| [#611](https://github.com/developing-today-forks/opencode/pull/611)   | fix(core): bound route_usage session targets to the requested window (#576)                                           | dezren39              | 2026-10-27 |
| [#613](https://github.com/developing-today-forks/opencode/pull/613)   | fix(core): make MessagingLog.clip respect its maximum at small limits (#577)                                          | dezren39              | 2026-10-27 |
| [#618](https://github.com/developing-today-forks/opencode/pull/618)   | feat(core): per-layer weight attribution in the route decision log (#589)                                             | dezren39              | 2026-10-27 |
| [#623](https://github.com/developing-today-forks/opencode/pull/623)   | fix(core): read bare epoch seconds as seconds in MessagingTemplate.parseTime                                          | dezren39              | 2026-10-27 |
| [#624](https://github.com/developing-today-forks/opencode/pull/624)   | fix(core): summarize non-Error thrown values in ErrorSummary.from (#600)                                              | dezren39              | 2026-10-27 |
| [#625](https://github.com/developing-today-forks/opencode/pull/625)   | fix(core): sort plan notes chronologically                                                                            | dezren39              | 2026-10-27 |
| [#627](https://github.com/developing-today-forks/opencode/pull/627)   | fix(core): reject calendar-invalid dates in quota-reset parsers (#609)                                                | dezren39              | 2026-10-27 |
| [#630](https://github.com/developing-today-forks/opencode/pull/630)   | fix(core): support external fields as when targets with boolean semantics (#601)                                      | dezren39              | 2026-10-27 |
| [#632](https://github.com/developing-today-forks/opencode/pull/632)   | fix(core): weightLayers reports the most recent selection, not the oldest (#628)                                      | dezren39              | 2026-10-27 |
| [#633](https://github.com/developing-today-forks/opencode/pull/633)   | fix(core): sort daysOfMonth to a canonical order in schedule parse (#527)                                             | dezren39              | 2026-10-27 |
| [#642](https://github.com/developing-today-forks/opencode/pull/642)   | fix(core): re-trim ModelPlan.slug after the 64-char slice                                                             | dezren39              | 2026-10-27 |
| [#635](https://github.com/developing-today-forks/opencode/pull/635)   | fix(core): pin slash-less behaviour for route-usage target split (#548)                                               | dezren39              | 2026-10-27 |
| [#638](https://github.com/developing-today-forks/opencode/pull/638)   | fix(core): report quota windows without windowHours in route_usage with usage unknown                                 | dezren39              | 2026-10-27 |
| [#640](https://github.com/developing-today-forks/opencode/pull/640)   | fix(core): yield null for out-of-range daily reset rules in QuotaSnapshot                                             | dezren39              | 2026-10-27 |
| [#643](https://github.com/developing-today-forks/opencode/pull/643)   | fix(core): throw on an unparseable readNotes since instead of silently returning no notes (#557)                      | dezren39              | 2026-10-28 |
| [#648](https://github.com/developing-today-forks/opencode/pull/648)   | fix(core): reject calendar-invalid dates in MessagingTemplate.parseTime (#647)                                        | dezren39              | 2026-10-28 |
| [#641](https://github.com/developing-today-forks/opencode/pull/641)   | fix(core): guard readNotes limit <= 0 — slice(-0) returned every note                                                 | dezren39              | 2026-10-28 |
| [#644](https://github.com/developing-today-forks/opencode/pull/644)   | fix(core): do not floor or accept non-finite strings in attemptParameterRepair                                        | dezren39              | 2026-10-28 |
| [#657](https://github.com/developing-today-forks/opencode/pull/657)   | fix(core): validate explicit quota reset dates                                                                        | dezren39              | 2026-10-28 |
| [#636](https://github.com/developing-today-forks/opencode/pull/636)   | fix(core): clamp daysSince at zero                                                                                    | dezren39              | 2026-10-28 |
| [#658](https://github.com/developing-today-forks/opencode/pull/658)   | fix(core): reject calendar-invalid date-time form values                                                              | dezren39              | 2026-10-28 |
| [#667](https://github.com/developing-today-forks/opencode/pull/667)   | fix(core): reject hour-24 ISO date-times in form and quota-reset validators                                           | dezren39              | 2026-10-28 |
| [#665](https://github.com/developing-today-forks/opencode/pull/665)   | fix(core): drop prefer overrides with invalid factor or weight                                                        | dezren39              | 2026-10-28 |
| [#670](https://github.com/developing-today-forks/opencode/pull/670)   | fix(core): drop overrides with a malformed until                                                                      | dezren39              | 2026-10-28 |
| [#674](https://github.com/developing-today-forks/opencode/pull/674)   | fix(core): treat blank rate-limit remaining as unparseable                                                            | dezren39              | 2026-10-28 |
| [#675](https://github.com/developing-today-forks/opencode/pull/675)   | fix(core): make dayReports provider accumulator prototype-safe                                                        | dezren39              | 2026-10-28 |
| [#680](https://github.com/developing-today-forks/opencode/pull/680)   | fix(core): reject blank and negative Modal pricing                                                                    | dezren39              | 2026-10-28 |
| [#683](https://github.com/developing-today-forks/opencode/pull/683)   | fix(core): reject unsatisfiable form field bounds and bad patterns                                                    | dezren39              | 2026-10-28 |
| [#689](https://github.com/developing-today-forks/opencode/pull/689)   | fix(core): restrict WebSearch retry-after cooldown to delay-seconds grammar                                           | dezren39              | 2026-10-28 |
| [#699](https://github.com/developing-today-forks/opencode/pull/699)   | fix(core): summarize thrown symbols in ErrorSummary                                                                   | dezren39              | 2026-10-28 |
| [#701](https://github.com/developing-today-forks/opencode/pull/701)   | fix(core): let a bare 4-digit ISO year parse in MessagingTemplate.parseTime                                           | dezren39              | 2026-10-28 |
| [#700](https://github.com/developing-today-forks/opencode/pull/700)   | fix(core): keep config references inside block comments literal                                                       | dezren39              | 2026-10-28 |
| [#704](https://github.com/developing-today-forks/opencode/pull/704)   | fix(core): reject unsatisfiable form field definitions in validateFields                                              | dezren39              | 2026-10-28 |
| [#709](https://github.com/developing-today-forks/opencode/pull/709)   | fix(core): reject duplicate multiselect selections in validateAnswer (#707)                                           | dezren39              | 2026-10-28 |
| [#714](https://github.com/developing-today-forks/opencode/pull/714)   | fix(core): reject required form fields no answer can satisfy (#712)                                                   | dezren39              | 2026-10-28 |
| [#649](https://github.com/developing-today-forks/opencode/pull/649)   | feat(core): named route windows and target eligibility gates (#639)                                                   | dezren39              | 2026-10-28 |
| [#721](https://github.com/developing-today-forks/opencode/pull/721)   | fix(core): reject when values a string target can never answer (#708)                                                 | dezren39              | 2026-10-28 |
| [#719](https://github.com/developing-today-forks/opencode/pull/719)   | fix(core): restrict ToolInputRepair number coercion to decimal grammar                                                | dezren39              | 2026-10-28 |
| [#722](https://github.com/developing-today-forks/opencode/pull/722)   | fix(core): calendar-check ModelPlan date parsing (#717)                                                               | dezren39              | 2026-10-28 |
| [#706](https://github.com/developing-today-forks/opencode/pull/706)   | fix(core): reject when values a numeric form target can never answer                                                  | dezren39              | 2026-10-28 |
| [#725](https://github.com/developing-today-forks/opencode/pull/725)   | fix(core): plain-decimal grammar for ModalModels pricing strings                                                      | dezren39              | 2026-10-28 |
| [#726](https://github.com/developing-today-forks/opencode/pull/726)   | fix(core): honour window gates inside nested route groups (#723)                                                      | dezren39              | 2026-10-28 |
| [#718](https://github.com/developing-today-forks/opencode/pull/718)   | fix(core): restrict ModelRouteLimits.windows counts to plain decimal grammar (#711)                                   | dezren39              | 2026-10-28 |
| [#739](https://github.com/developing-today-forks/opencode/pull/739)   | fix(core): exclude $0 from command template placeholders (#736)                                                       | dezren39              | 2026-10-28 |
| [#740](https://github.com/developing-today-forks/opencode/pull/740)   | fix(core): warn when a named route window is dropped (#734)                                                           | dezren39              | 2026-10-28 |
| [#745](https://github.com/developing-today-forks/opencode/pull/745)   | fix(core): parse ol start and li value with HTML integer grammar                                                      | dezren39              | 2026-10-28 |
| [#741](https://github.com/developing-today-forks/opencode/pull/741)   | fix(core): readNotes non-finite limit returns no notes                                                                | dezren39              | 2026-10-28 |
| [#738](https://github.com/developing-today-forks/opencode/pull/738)   | fix(core): reject unanswerable when values on external form targets                                                   | dezren39              | 2026-10-28 |
| [#748](https://github.com/developing-today-forks/opencode/pull/748)   | fix(core): warn on stdlib-named .py files in the shared tmp root (#132)                                               | dezren39              | 2026-10-28 |
| [#755](https://github.com/developing-today-forks/opencode/pull/755)   | fix(core): use own-property checks for messaging variable names (#747)                                                | dezren39              | 2026-10-28 |
| [#749](https://github.com/developing-today-forks/opencode/pull/749)   | feat(core): declare cache_format on model routes and targets                                                          | dezren39              | 2026-10-28 |
| [#763](https://github.com/developing-today-forks/opencode/pull/763)   | fix(core): TmpGuard.scan sees package directories and symlinked .py files                                             | dezren39              | 2026-10-28 |
| [#757](https://github.com/developing-today-forks/opencode/pull/757)   | test(core): keep the git packs loose objects test inside its timeout on windows (#652)                                | dezren39              | 2026-10-28 |
| [#764](https://github.com/developing-today-forks/opencode/pull/764)   | fix(core): TmpGuard stdlib list — add annotationlib, antigravity, cProfile                                            | dezren39              | 2026-10-28 |
| [#775](https://github.com/developing-today-forks/opencode/pull/775)   | fix(core): govern forks as children of their source session                                                           | dezren39              | 2026-10-28 |
| [#776](https://github.com/developing-today-forks/opencode/pull/776)   | fix(core): store the governance-checked model on a model-less child (#507)                                            | dezren39              | 2026-10-28 |
| [#778](https://github.com/developing-today-forks/opencode/pull/778)   | fix(core): plain-decimal grammar for OAuth expires_in and device interval (#771)                                      | dezren39              | 2026-10-28 |
| [#781](https://github.com/developing-today-forks/opencode/pull/781)   | fix(core): fall back to node fs.watch for directory watches when the Parcel binding is unavailable                    | dezren39              | 2026-10-29 |
| [#782](https://github.com/developing-today-forks/opencode/pull/782)   | fix(core): calendar-check the Azure CLI expiresOn token expiry                                                        | dezren39              | 2026-10-29 |
| [#787](https://github.com/developing-today-forks/opencode/pull/787)   | fix(core): use own-property lookups for AISDKNative package tables                                                    | dezren39              | 2026-10-29 |
| [#786](https://github.com/developing-today-forks/opencode/pull/786)   | fix(core): calendar-check Exa and Parallel websearch published dates                                                  | dezren39              | 2026-10-29 |
| [#768](https://github.com/developing-today-forks/opencode/pull/768)   | feat(core): per-launch stop policies for background subagents                                                         | dezren39              | 2026-10-29 |
| [#772](https://github.com/developing-today-forks/opencode/pull/772)   | test(core): give config reload polls a budget sized to the watcher chain                                              | dezren39              | 2026-10-29 |
| [#788](https://github.com/developing-today-forks/opencode/pull/788)   | test(core): await recorded request in first-output-deadline test (#488)                                               | dezren39              | 2026-10-29 |
| [#758](https://github.com/developing-today-forks/opencode/pull/758)   | fix(core): preserve a bare-alias target's variant in ModelResolver (#391)                                             | dezren39              | 2026-10-29 |
| [#790](https://github.com/developing-today-forks/opencode/pull/790)   | fix(core): restrict McpxDiagnose parameter repair to plain-decimal grammar (#769)                                     | dezren39              | 2026-10-29 |
| [#799](https://github.com/developing-today-forks/opencode/pull/799)   | fix(core): use own-property lookup for ShellParse ARITY table (#792)                                                  | dezren39              | 2026-10-29 |
| [#794](https://github.com/developing-today-forks/opencode/pull/794)   | fix(core): use own-property lookup for ModelRouteSchedule day names (#791)                                            | dezren39              | 2026-10-29 |
| [#796](https://github.com/developing-today-forks/opencode/pull/796)   | fix(core): own-property lookups in MessagingTemplate.constant (#793)                                                  | dezren39              | 2026-10-29 |
| [#806](https://github.com/developing-today-forks/opencode/pull/806)   | fix(core): own-property {env:} lookup in ConfigVariable.substitute                                                    | dezren39              | 2026-10-29 |
| [#817](https://github.com/developing-today-forks/opencode/pull/817)   | feat(core): per-session burn-rate estimation for quota windows (#568)                                                 | dezren39              | 2026-10-29 |
| [#808](https://github.com/developing-today-forks/opencode/pull/808)   | fix(core): own-property lookups for CodeMode fetch response headers (#805)                                            | dezren39              | 2026-10-29 |
| [#826](https://github.com/developing-today-forks/opencode/pull/826)   | fix(core): escalate route-health cooldowns on repeated failure                                                        | dezren39              | 2026-10-29 |
| [#814](https://github.com/developing-today-forks/opencode/pull/814)   | fix(core): let model pools reach transform-created models (#236)                                                      | dezren39              | 2026-10-29 |
| [#816](https://github.com/developing-today-forks/opencode/pull/816)   | fix(core): surface watcher subscribe failures — node fallback + degraded event (#797)                                 | dezren39              | 2026-10-29 |
| [#762](https://github.com/developing-today-forks/opencode/pull/762)   | feat(core): stitch continued route attempts into one turn                                                             | dezren39              | 2026-10-29 |
| [#810](https://github.com/developing-today-forks/opencode/pull/810)   | feat(core): door-set targets with exclusions for model routes (#637)                                                  | dezren39              | 2026-10-29 |
| [#835](https://github.com/developing-today-forks/opencode/pull/835)   | fix(core): resolve QuotaSnapshot per-plan max-age with own-property semantics (#824)                                  | dezren39              | 2026-10-29 |
| [#860](https://github.com/developing-today-forks/opencode/pull/860)   | fix(core): reject non-finite when values on number targets (#853)                                                     | dezren39              | 2026-10-29 |
| [#837](https://github.com/developing-today-forks/opencode/pull/837)   | fix(core): guard QuotaBurnRate priors lookup against prototype chain                                                  | dezren39              | 2026-10-29 |
| [#838](https://github.com/developing-today-forks/opencode/pull/838)   | fix(core): own-property route and variant lookups in ModelGovernance                                                  | dezren39              | 2026-10-29 |
| [#867](https://github.com/developing-today-forks/opencode/pull/867)   | fix(core): own-property reads in ModelPlan setPool and setModel                                                       | dezren39              | 2026-10-29 |
| [#868](https://github.com/developing-today-forks/opencode/pull/868)   | fix(core): resolve wellknown env substitution by own property                                                         | dezren39              | 2026-10-29 |
| [#811](https://github.com/developing-today-forks/opencode/pull/811)   | feat(core): per-target context limits for model routes                                                                | dezren39              | 2026-10-29 |
| [#888](https://github.com/developing-today-forks/opencode/pull/888)   | fix(core): stop Copilot release-date rollover via parseReleaseDate (#886)                                             | dezren39              | 2026-10-29 |
| [#887](https://github.com/developing-today-forks/opencode/pull/887)   | fix(core): validate MCP OAuth expires_in before storing it                                                            | dezren39              | 2026-10-29 |
| [#878](https://github.com/developing-today-forks/opencode/pull/878)   | fix(core): read Variant PROTOCOLS table with an own-property guard                                                    | dezren39              | 2026-10-29 |
| [#882](https://github.com/developing-today-forks/opencode/pull/882)   | fix(core): floor opencode device-code poll interval and pass expiresAt                                                | dezren39              | 2026-10-29 |
| [#884](https://github.com/developing-today-forks/opencode/pull/884)   | fix(core): validate OAuth expires_in in five providers                                                                | dezren39              | 2026-10-29 |
| [#883](https://github.com/developing-today-forks/opencode/pull/883)   | feat(core): shared own-property guard + prototype-chain read regression net                                           | dezren39              | 2026-10-29 |
| [#871](https://github.com/developing-today-forks/opencode/pull/871)   | fix(core): judge door-set variants through remaps and retarget across doors                                           | dezren39              | 2026-10-29 |
| [#894](https://github.com/developing-today-forks/opencode/pull/894)   | fix(core): bound oversized expires_in in xAI and Poe                                                                  | dezren39              | 2026-10-29 |
| [#901](https://github.com/developing-today-forks/opencode/pull/901)   | fix(core): bound oversized expires_in in DigitalOcean                                                                 | dezren39              | 2026-10-29 |
| [#854](https://github.com/developing-today-forks/opencode/pull/854)   | feat(core): credential failover on quota exhaustion via credential.activate (#6)                                      | dezren39              | 2026-10-29 |
| [#902](https://github.com/developing-today-forks/opencode/pull/902)   | fix(core): bound oversized device-code expires_in in opencode + xAI (#900)                                            | dezren39              | 2026-10-29 |
| [#898](https://github.com/developing-today-forks/opencode/pull/898)   | feat(core): thread requestTokens through the session runner so per-target ceilings filter live                        | dezren39              | 2026-10-29 |
| [#905](https://github.com/developing-today-forks/opencode/pull/905)   | fix(core): floor ModelPlan.readNotes limit so fractional limits cannot return every note (#904)                       | dezren39              | 2026-10-29 |
| [#914](https://github.com/developing-today-forks/opencode/pull/914)   | fix(core): clamp WebSearch Retry-After cooldown to 15 minutes                                                         | dezren39              | 2026-10-29 |
| [#918](https://github.com/developing-today-forks/opencode/pull/918)   | fix(core): bound Copilot device poll interval and keep slow_down monotonic                                            | dezren39              | 2026-10-29 |
| [#920](https://github.com/developing-today-forks/opencode/pull/920)   | fix(core): thread the credential dimension through restore, escalation, ranking, rate-limit holds and catalog headers | dezren39              | 2026-10-29 |
| [#925](https://github.com/developing-today-forks/opencode/pull/925)   | fix(core): write route-overrides.json atomically (#924)                                                               | dezren39              | 2026-10-29 |
| [#930](https://github.com/developing-today-forks/opencode/pull/930)   | fix(core): bound MessagingEmbedding.download in flight                                                                | dezren39              | 2026-10-29 |
| [#934](https://github.com/developing-today-forks/opencode/pull/934)   | fix(core): bound quota-reset predictions at a 30-day ceiling                                                          | dezren39              | 2026-10-30 |
| [#946](https://github.com/developing-today-forks/opencode/pull/946)   | fix(core): bound and normalise the plugin vcs.branch.list limit                                                       | dezren39              | 2026-10-30 |
| [#943](https://github.com/developing-today-forks/opencode/pull/943)   | fix(core): clamp OpenAI, OpenCode and xAI device poll intervals at 60s (#941)                                         | dezren39              | 2026-10-30 |
| [#949](https://github.com/developing-today-forks/opencode/pull/949)   | fix(core): floor and clamp Messaging read limits at the service boundary                                              | dezren39              | 2026-10-30 |
| [#950](https://github.com/developing-today-forks/opencode/pull/950)   | fix(core): normalise plugin vcs.diff context before the adapters                                                      | dezren39              | 2026-10-30 |
| [#956](https://github.com/developing-today-forks/opencode/pull/956)   | feat(core): report cache savings and hit ratio in route_usage (#952)                                                  | dezren39              | 2026-10-30 |
| [#954](https://github.com/developing-today-forks/opencode/pull/954)   | fix(core): normalise the plugin session.list limit before the store (#951)                                            | dezren39              | 2026-10-30 |
| [#955](https://github.com/developing-today-forks/opencode/pull/955)   | fix(core): normalise messaging offset at the service boundary                                                         | dezren39              | 2026-10-30 |
| [#967](https://github.com/developing-today-forks/opencode/pull/967)   | fix(core): normalise Shell.output paging at the service boundary                                                      | dezren39              | 2026-10-30 |
| [#974](https://github.com/developing-today-forks/opencode/pull/974)   | fix(core): clamp provider HTTP timeouts to the raw-timer bound                                                        | dezren39              | 2026-10-30 |
| [#973](https://github.com/developing-today-forks/opencode/pull/973)   | fix(core): respect detached policy on direct interrupt                                                                | dezren39              | 2026-10-30 |
| [#961](https://github.com/developing-today-forks/opencode/pull/961)   | fix(core): bound SessionStats range span                                                                              | dezren39              | 2026-10-30 |
| [#966](https://github.com/developing-today-forks/opencode/pull/966)   | fix(core): normalise Vcs.branches limit at the service boundary                                                       | dezren39              | 2026-10-30 |
| [#962](https://github.com/developing-today-forks/opencode/pull/962)   | fix(core): bound Copilot utilityTitle response body with a 64 KiB cap                                                 | dezren39              | 2026-10-30 |
| [#971](https://github.com/developing-today-forks/opencode/pull/971)   | test(core): pin node fs.watch fallback event delivery                                                                 | dezren39              | 2026-10-30 |
| [#982](https://github.com/developing-today-forks/opencode/pull/982)   | fix(core): normalise Session.diff context before git                                                                  | dezren39              | 2026-10-30 |
| [#983](https://github.com/developing-today-forks/opencode/pull/983)   | fix(core): normalise session message limits at the store boundary                                                     | dezren39              | 2026-10-30 |
| [#986](https://github.com/developing-today-forks/opencode/pull/986)   | fix(core): stop pruning pinned route leaves on suffixed-catalog providers (#985)                                      | dezren39              | 2026-10-30 |
| [#991](https://github.com/developing-today-forks/opencode/pull/991)   | feat(core): gate quota-exhausted pools until their stated reset (#984)                                                | dezren39              | 2026-10-30 |
| [#1012](https://github.com/developing-today-forks/opencode/pull/1012) | fix(core): normalise read page offset and limit at the boundary                                                       | dezren39              | 2026-10-30 |
| [#1020](https://github.com/developing-today-forks/opencode/pull/1020) | fix(core): bound Copilot and Modal /models response bodies                                                            | dezren39              | 2026-10-30 |
| [#1022](https://github.com/developing-today-forks/opencode/pull/1022) | fix(core): bound configured MCP OAuth discovery fetch                                                                 | dezren39              | 2026-10-30 |
| [#1024](https://github.com/developing-today-forks/opencode/pull/1024) | fix(core): bound mcpx daemon client timeout and response bodies                                                       | dezren39              | 2026-10-30 |
| [#1028](https://github.com/developing-today-forks/opencode/pull/1028) | fix(core): feed current quota reads into route candidacy gate (#984)                                                  | dezren39              | 2026-10-30 |
| [#1027](https://github.com/developing-today-forks/opencode/pull/1027) | fix(core): cap the remote instruction body read                                                                       | dezren39              | 2026-10-31 |
| [#1030](https://github.com/developing-today-forks/opencode/pull/1030) | fix(core): bound provider OAuth token/device/discovery response bodies                                                | dezren39              | 2026-10-31 |
| [#1042](https://github.com/developing-today-forks/opencode/pull/1042) | fix(core): scope route_usage burn-rate samples to one plan window and dedupe multi-pool models                        | dezren39              | 2026-10-31 |
| [#1041](https://github.com/developing-today-forks/opencode/pull/1041) | fix(core): reset consecutiveFailures in the networkSuspect cooldown undo                                              | dezren39              | 2026-10-31 |
| [#1058](https://github.com/developing-today-forks/opencode/pull/1058) | feat(core): add credential_id to route_health                                                                         | dezren39              | 2026-10-31 |
| [#1069](https://github.com/developing-today-forks/opencode/pull/1069) | test(core): add direct tests for tool/http-body bounded-body helpers (#1068)                                          | dezren39              | 2026-10-31 |
| [#1080](https://github.com/developing-today-forks/opencode/pull/1080) | fix(core): bound MessagingEmbedding.request response bodies (#1079)                                                   | dezren39              | 2026-10-31 |
| [#1072](https://github.com/developing-today-forks/opencode/pull/1072) | fix(core): bound custom embedding model downloads                                                                     | dezren39              | 2026-10-31 |
| [#1099](https://github.com/developing-today-forks/opencode/pull/1099) | fix(core): guard AISDK toolNames against prototype-chain reads and the **proto** write swallow                        | dezren39              | 2026-10-31 |
| [#1113](https://github.com/developing-today-forks/opencode/pull/1113) | fix(core): scope quota usage to the serving credential                                                                | dezren39              | 2026-10-31 |
| [#958](https://github.com/developing-today-forks/opencode/pull/958)   | fix(core): normalise messaging log chars and grep context at the service boundary                                     | dezren39              | 2026-11-01 |
| [#940](https://github.com/developing-today-forks/opencode/pull/940)   | test(core): plugin session.list defaults to newest 50 — regression test for #935                                      | dezren39              | 2026-11-01 |
| [#1013](https://github.com/developing-today-forks/opencode/pull/1013) | fix(core): normalise session list limits at the store boundary                                                        | dezren39              | 2026-11-01 |
| [#1124](https://github.com/developing-today-forks/opencode/pull/1124) | feat(core): round-robin weights are priority tiers, not shares                                                        | dezren39              | 2026-11-01 |
| [#1130](https://github.com/developing-today-forks/opencode/pull/1130) | fix(core): key restored attempt quota snapshots by credential_id                                                      | dezren39              | 2026-11-01 |
| [#1133](https://github.com/developing-today-forks/opencode/pull/1133) | fix(core): gate skill pull unversioned branch on download results                                                     | dezren39              | 2026-11-01 |
| [#1138](https://github.com/developing-today-forks/opencode/pull/1138) | fix(core): pin sha256 for the ripgrep archive download and verify before install                                      | dezren39              | 2026-11-01 |
| [#1134](https://github.com/developing-today-forks/opencode/pull/1134) | feat(core): provider routes — provider-level pools with ordered/round-robin/weighted selection                        | dezren39              | 2026-11-01 |
| [#1165](https://github.com/developing-today-forks/opencode/pull/1165) | fix(core): govern Markdown-defined agents through directory expansion (#1107)                                         | dezren39              | 2026-11-01 |
| [#1166](https://github.com/developing-today-forks/opencode/pull/1166) | fix(core): cap quota benches at actual resets                                                                         | dezren39              | 2026-11-01 |
| [#1180](https://github.com/developing-today-forks/opencode/pull/1180) | fix(core): bound the models-snapshot refresh fetch — 30s deadline + 32 MiB body cap                                   | dezren39              | 2026-11-01 |
| [#1191](https://github.com/developing-today-forks/opencode/pull/1191) | feat(core): shared numeric normalisation primitives + paging migration (#993)                                         | dezren39              | 2026-11-01 |
| [#1235](https://github.com/developing-today-forks/opencode/pull/1235) | ci(core): add check:numeric-boundaries regression net (#995)                                                          | dezren39              | 2026-11-01 |
| [#1293](https://github.com/developing-today-forks/opencode/pull/1293) | test(core): per-field normalization survival guard (#1285)                                                            | dezren39              | 2026-11-01 |
| [#1305](https://github.com/developing-today-forks/opencode/pull/1305) | fix(core): half-open probe at quota reset and active benches in route stats (#1123)                                   | dezren39              | 2026-11-01 |
| [#1306](https://github.com/developing-today-forks/opencode/pull/1306) | fix(core): expectedShare follows priority tiers in round-robin pools                                                  | dezren39              | 2026-11-01 |
| [#1190](https://github.com/developing-today-forks/opencode/pull/1190) | fix(core): gate balance-shaped quota pools by balance, not percent (#1014)                                            | dezren39              | 2026-11-01 |
| [#1351](https://github.com/developing-today-forks/opencode/pull/1351) | fix(core): gatingSnapshot fallback must not select an unknown snapshot over known evidence (#1341)                    | dezren39              | 2026-11-02 |
| [#1352](https://github.com/developing-today-forks/opencode/pull/1352) | fix(core): align fixed-reset quota usage to the current period (#1342)                                                | dezren39              | 2026-11-02 |
| [#1359](https://github.com/developing-today-forks/opencode/pull/1359) | fix(core): skip malformed V1 migration data URLs                                                                      | dezren39              | 2026-11-02 |
| [#1362](https://github.com/developing-today-forks/opencode/pull/1362) | fix(core): fail malformed base64 attachments closed in the v1 migration (#1360)                                       | dezren39              | 2026-11-02 |
| [#1364](https://github.com/developing-today-forks/opencode/pull/1364) | feat(core): cascade to the nesting parent route when a sub-route exhausts (#1014)                                     | dezren39              | 2026-11-02 |
| [#1377](https://github.com/developing-today-forks/opencode/pull/1377) | fix(core): bound the skill discovery index read with a byte cap and deadline (#1376)                                  | dezren39              | 2026-11-02 |
| [#1384](https://github.com/developing-today-forks/opencode/pull/1384) | fix(core): pool-shaped quota pull-out and any-error-class escalating bench (#1014 PR3)                                | dezren39              | 2026-11-02 |
| [#1353](https://github.com/developing-today-forks/opencode/pull/1353) | fix(core): exclude expired and cancelled plans from quota gate derivation                                             | dezren39              | 2026-11-02 |
| [#1393](https://github.com/developing-today-forks/opencode/pull/1393) | fix(core): stale balance holds + balance-exhaustion classification (#1380)                                            | dezren39              | 2026-11-02 |
| [#1392](https://github.com/developing-today-forks/opencode/pull/1392) | fix(core): ungate failover from the retry hook and walk credentials in proximity order (#1379)                        | dezren39              | 2026-11-02 |
| [#1387](https://github.com/developing-today-forks/opencode/pull/1387) | fix(core): bound OAuth callback page requests                                                                         | dezren39              | 2026-11-02 |
| [#1370](https://github.com/developing-today-forks/opencode/pull/1370) | test(core): assert body collection stops at size limit                                                                | dezren39              | 2026-11-02 |
| [#1429](https://github.com/developing-today-forks/opencode/pull/1429) | fix(core): write config patches into the explicit config file when configured                                         | dezren39              | 2026-11-02 |
| [#1428](https://github.com/developing-today-forks/opencode/pull/1428) | fix(core): expand provider routes in model governance                                                                 | dezren39              | 2026-11-02 |
| [#1437](https://github.com/developing-today-forks/opencode/pull/1437) | feat(core): demotion policy as config, Location-scoped (#1151)                                                        | dezren39              | 2026-11-02 |
| [#1440](https://github.com/developing-today-forks/opencode/pull/1440) | feat(core): wire tuning reset ceiling to quota-gate settings                                                          | dezren39              | 2026-11-02 |
| [#1427](https://github.com/developing-today-forks/opencode/pull/1427) | fix(core): let bench backoff grow until maxCooldownMs binds                                                           | dezren39              | 2026-11-02 |
| [#1446](https://github.com/developing-today-forks/opencode/pull/1446) | feat(core): native-shape pass-through fast path for the OpenAI façade (#1146)                                         | dezren39              | 2026-11-02 |
| [#1438](https://github.com/developing-today-forks/opencode/pull/1438) | fix(core): normalize round-robin tier keys against float noise                                                        | dezren39              | 2026-11-02 |
| [#1448](https://github.com/developing-today-forks/opencode/pull/1448) | fix(core): emit the full §5 Responses streaming grammar in the façade (#1442)                                         | dezren39              | 2026-11-02 |
| [#1458](https://github.com/developing-today-forks/opencode/pull/1458) | feat(core): reproduce muse attestation aliasing in the façade (#1443)                                                 | dezren39              | 2026-11-02 |
| [#1432](https://github.com/developing-today-forks/opencode/pull/1432) | fix(core): scope model_route_settings reader per Location                                                             | dezren39              | 2026-11-02 |
| [#1454](https://github.com/developing-today-forks/opencode/pull/1454) | fix(core): reject unsafe provider-route members                                                                       | dezren39              | 2026-11-02 |
| [#1468](https://github.com/developing-today-forks/opencode/pull/1468) | feat(core): import Codex CLI credentials in-fork with true expiry                                                     | dezren39              | 2026-11-03 |
| [#1466](https://github.com/developing-today-forks/opencode/pull/1466) | fix(core): pool quota exclusion counts distinct quota gates, not leaves sharing one gate                              | dezren39              | 2026-11-03 |
| [#1483](https://github.com/developing-today-forks/opencode/pull/1483) | fix(core): spell antigravity reasoning variants as thinking levels                                                    | dezren39              | 2026-11-03 |
| [#1479](https://github.com/developing-today-forks/opencode/pull/1479) | fix(core): settle bench probe claims spent by title generation (#1478)                                                | dezren39              | 2026-11-03 |
| [#1507](https://github.com/developing-today-forks/opencode/pull/1507) | refactor(core): collapse expiresInSeconds duplicates onto serverSeconds                                               | dezren39              | 2026-11-03 |
| [#1463](https://github.com/developing-today-forks/opencode/pull/1463) | fix(core): scope tuning quota ceiling and threshold balance holds per Location                                        | dezren39              | 2026-11-03 |
| [#1531](https://github.com/developing-today-forks/opencode/pull/1531) | fix(core): decode non-base64 V1 migration data URLs as bytes                                                          | dezren39              | 2026-11-03 |
| [#1509](https://github.com/developing-today-forks/opencode/pull/1509) | fix(core): prevent quota-feed starvation across routes                                                                | dezren39              | 2026-11-03 |
| [#1532](https://github.com/developing-today-forks/opencode/pull/1532) | fix(core): unify quota-reset relative units across both parsers                                                       | dezren39              | 2026-11-03 |
| [#1535](https://github.com/developing-today-forks/opencode/pull/1535) | fix(core): write Config.update patches atomically                                                                     | dezren39              | 2026-11-03 |
| [#1536](https://github.com/developing-today-forks/opencode/pull/1536) | fix(core): normalise Shell timeout durations at the service boundary                                                  | dezren39              | 2026-11-03 |
| [#1538](https://github.com/developing-today-forks/opencode/pull/1538) | fix(core): bound the ModelsDev catalog fetch body at 16 MiB                                                           | dezren39              | 2026-11-03 |
| [#1542](https://github.com/developing-today-forks/opencode/pull/1542) | fix(core): clamp the systemone data-path timeout through timeoutMs (#1477)                                            | dezren39              | 2026-11-03 |
| [#1544](https://github.com/developing-today-forks/opencode/pull/1544) | fix(core): settle bench probe claims on every resolution exit path (#1486)                                            | dezren39              | 2026-11-03 |
| [#1545](https://github.com/developing-today-forks/opencode/pull/1545) | fix(core): bound provider OAuth token and error body reads in gitlab, poe, snowflake-cortex (#1029)                   | dezren39              | 2026-11-03 |
| [#1547](https://github.com/developing-today-forks/opencode/pull/1547) | feat(core): hidden model state — picker-hidden but routable (#230)                                                    | dezren39              | 2026-11-03 |
| [#1559](https://github.com/developing-today-forks/opencode/pull/1559) | feat(core): native in-process embeddings backend behind the registry (#1554 step 2)                                   | dezren39              | 2026-11-03 |
| [#1563](https://github.com/developing-today-forks/opencode/pull/1563) | fix(core): build routes whose targets are all transform-created                                                       | dezren39              | 2026-11-03 |
| [#1571](https://github.com/developing-today-forks/opencode/pull/1571) | fix(core): validate the embeddings backend index set before pooling vectors                                           | dezren39              | 2026-11-03 |
| [#1579](https://github.com/developing-today-forks/opencode/pull/1579) | fix(core): an expired grant's balance hold survives a same-grant lower re-read (#1552)                                | dezren39              | 2026-11-04 |
| [#1591](https://github.com/developing-today-forks/opencode/pull/1591) | fix(core): add a named deadline to skill file downloads (#1381)                                                       | dezren39              | 2026-11-04 |
| [#1599](https://github.com/developing-today-forks/opencode/pull/1599) | fix(core): own-property alias read in muse-attestation; proto-reads gate to green (#1476)                             | dezren39              | 2026-11-04 |
| [#1605](https://github.com/developing-today-forks/opencode/pull/1605) | fix(core): judge demotion by the judging Location's settings (#1484)                                                  | dezren39              | 2026-11-04 |
| [#1603](https://github.com/developing-today-forks/opencode/pull/1603) | fix(core): bound the MCP OAuth and Snowflake error-body reads (#1049)                                                 | dezren39              | 2026-11-04 |
| [#1602](https://github.com/developing-today-forks/opencode/pull/1602) | fix(core): isolate content-policy rejections from route health (#1084)                                                | dezren39              | 2026-11-04 |
| [#1621](https://github.com/developing-today-forks/opencode/pull/1621) | fix(core): release probe claims on content-filter finishes                                                            | dezren39              | 2026-11-04 |
| [#1623](https://github.com/developing-today-forks/opencode/pull/1623) | fix(core): decode non-base64 data-URL attachments byte-wise                                                           | dezren39              | 2026-11-04 |
| [#1619](https://github.com/developing-today-forks/opencode/pull/1619) | fix(core): ledger-backed demotion counts from route_attempt (#1151)                                                   | dezren39              | 2026-11-04 |
| [#1626](https://github.com/developing-today-forks/opencode/pull/1626) | fix(core): keep the client timeout text out of failureHint's retry-after (#1551)                                      | dezren39              | 2026-11-04 |
| [#1636](https://github.com/developing-today-forks/opencode/pull/1636) | test(core): implement all() in the session-runner-model Model mock (#1584)                                            | dezren39              | 2026-11-04 |
| [#1631](https://github.com/developing-today-forks/opencode/pull/1631) | fix(core): reconcile persisted bound workspaces at boot (#1622)                                                       | dezren39              | 2026-11-04 |
| [#1662](https://github.com/developing-today-forks/opencode/pull/1662) | fix(core): diagnose unknown keys under experimental.routes and routes.settings (#1543)                                | dezren39              | 2026-11-04 |
| [#1674](https://github.com/developing-today-forks/opencode/pull/1674) | test(core): meta-validate built-in tool schemas                                                                       | dezren39              | 2026-11-05 |
| [#1645](https://github.com/developing-today-forks/opencode/pull/1645) | fix(core): evict failed native embeddings loads from the cache (#1567)                                                | dezren39              | 2026-11-05 |
| [#1632](https://github.com/developing-today-forks/opencode/pull/1632) | fix(core): validate the ONNX output width in the native embeddings backend (#1583)                                    | dezren39              | 2026-11-05 |
| [#1690](https://github.com/developing-today-forks/opencode/pull/1690) | feat(core): flip embeddings registration to native-onnx backend (#1554 step 3)                                        | dezren39              | 2026-11-05 |
| [#1693](https://github.com/developing-today-forks/opencode/pull/1693) | fix(core): spell vibe reasoning variants as reasoning effort (#1692)                                                  | dezren39              | 2026-11-05 |
| [#1748](https://github.com/developing-today-forks/opencode/pull/1748) | fix(core): load .opencode/tool custom tools in v2 sessions (#1703)                                                    | dezren39              | 2026-11-05 |
| [#1702](https://github.com/developing-today-forks/opencode/pull/1702) | feat(core): rotate workspace safety snapshots                                                                         | dezren39              | 2026-11-05 |
| [#1807](https://github.com/developing-today-forks/opencode/pull/1807) | test(core): mock provider harness for route failover scenarios (#1806)                                                | dezren39              | 2026-11-06 |
| [#1802](https://github.com/developing-today-forks/opencode/pull/1802) | fix(core): reject native embeddings in Node bundle                                                                    | dezren39              | 2026-11-06 |
| [#1814](https://github.com/developing-today-forks/opencode/pull/1814) | fix(core): retain live quota events for later-activating Locations                                                    | dezren39              | 2026-11-06 |
| [#1839](https://github.com/developing-today-forks/opencode/pull/1839) | fix(core): wire the real tryPromise interruption signal into v1 custom-tool legacyContext (#1784)                     | dezren39              | 2026-11-06 |
| [#1827](https://github.com/developing-today-forks/opencode/pull/1827) | fix(core): judge multi-credential leaves by an eligible credential (#1806)                                            | dezren39              | 2026-11-06 |
| [#1853](https://github.com/developing-today-forks/opencode/pull/1853) | fix(core): cascade on every target-indicting failure class (#1806)                                                    | dezren39              | 2026-11-06 |
| [#1864](https://github.com/developing-today-forks/opencode/pull/1864) | fix(core): resolve configured provider-route ids for direct provider/model session requests (#1720)                   | dezren39              | 2026-11-06 |
| [#1874](https://github.com/developing-today-forks/opencode/pull/1874) | fix(core): provider ownership hardening — package seating + unscoped baseURL rewrite warnings (#1858)                 | dezren39              | 2026-11-06 |
| [#1922](https://github.com/developing-today-forks/opencode/pull/1922) | docs(core): subagent description — inheritance, depth, background delivery, question stall                            | dezren39              | 2026-11-07 |
| [#1929](https://github.com/developing-today-forks/opencode/pull/1929) | feat(core): expose per-step serving leaf in route_usage session readout (#1891)                                       | dezren39              | 2026-11-07 |
| [#1925](https://github.com/developing-today-forks/opencode/pull/1925) | feat(core): make the built-in comms namespace discoverable from the mcpx tool surface                                 | dezren39              | 2026-11-07 |
| [#1955](https://github.com/developing-today-forks/opencode/pull/1955) | fix(core): cap the DigitalOcean OAuth callback token POST body at 64 KiB (#1954)                                      | dezren39              | 2026-11-07 |
| [#1968](https://github.com/developing-today-forks/opencode/pull/1968) | fix(core): messaging log clip() no longer splits surrogate pairs (#1966)                                              | dezren39              | 2026-11-08 |
| [#1969](https://github.com/developing-today-forks/opencode/pull/1969) | feat(core): in-session session_switch_model tool (#1886)                                                              | dezren39              | 2026-11-08 |
| [#1473](https://github.com/developing-today-forks/opencode/pull/1473) | fix(core): let due bench probes recover quota-exhausted pools                                                         | dezren39              | 2026-11-08 |
| [#1979](https://github.com/developing-today-forks/opencode/pull/1979) | fix(core): abolish pool-level quota verdicts                                                                          | dezren39              | 2026-11-08 |
| [#1978](https://github.com/developing-today-forks/opencode/pull/1978) | fix(core): quota-driven quota-skip re-admission + per-model reactive skip scoping (#1974, #1976)                      | dezren39              | 2026-11-08 |
| [#2114](https://github.com/developing-today-forks/opencode/pull/2114) | fix(core): retire the embeddings HTTP sidecar backend (#1689)                                                         | dezren39              | 2026-11-08 |
| [#2116](https://github.com/developing-today-forks/opencode/pull/2116) | fix(core): never split a surrogate pair in messaging truncations (#1970)                                              | dezren39              | 2026-11-08 |
| [#1906](https://github.com/developing-today-forks/opencode/pull/1906) | feat(core): add detached session mode                                                                                 | dezren39              | 2026-11-08 |
| [#2122](https://github.com/developing-today-forks/opencode/pull/2122) | fix(core): preserve config priority in custom-tool override order                                                     | dezren39              | 2026-11-08 |
| [#2135](https://github.com/developing-today-forks/opencode/pull/2135) | fix(core): guard truncations outside messaging against surrogate-pair splits                                          | developing-today[bot] | 2026-11-08 |
| [#2136](https://github.com/developing-today-forks/opencode/pull/2136) | fix(core): add week unit to shared quota-reset relative grammar                                                       | dezren39              | 2026-11-08 |
| [#2134](https://github.com/developing-today-forks/opencode/pull/2134) | fix(core): omit unset quota reset hints                                                                               | dezren39              | 2026-11-08 |
| [#2156](https://github.com/developing-today-forks/opencode/pull/2156) | fix(core): failureHint extracts stated resets from the error body too (#2108)                                         | developing-today[bot] | 2026-11-08 |
| [#2160](https://github.com/developing-today-forks/opencode/pull/2160) | fix(core): re-judge restored quota benches against current quota state (#1976)                                        | dezren39              | 2026-11-08 |
| [#2163](https://github.com/developing-today-forks/opencode/pull/2163) | test(core): cascade failover cases for UnsupportedOperation, NoRoute, payload-too-large                               | dezren39              | 2026-11-08 |
| [#2177](https://github.com/developing-today-forks/opencode/pull/2177) | fix(core): strengthen Mime.detect BMP/GIF signatures (#2149)                                                          | dezren39              | 2026-11-08 |
| [#2188](https://github.com/developing-today-forks/opencode/pull/2188) | fix(core): bound and dedupe question-tool form asks                                                                   | developing-today[bot] | 2026-11-08 |
| [#2194](https://github.com/developing-today-forks/opencode/pull/2194) | fix(core): align family prompt delegation stance; DeepSeek entry + unmatched-model fallback                           | developing-today[bot] | 2026-11-09 |
| [#2174](https://github.com/developing-today-forks/opencode/pull/2174) | fix(core): dispatch ftyp media brands                                                                                 | dezren39              | 2026-11-09 |
| [#2199](https://github.com/developing-today-forks/opencode/pull/2199) | fix(core): do not cache a synchronous throw in lazy as undefined                                                      | developing-today[bot] | 2026-11-09 |
| [#2210](https://github.com/developing-today-forks/opencode/pull/2210) | fix(core): ignore a conflicted document's provider identity and overlays                                              | dezren39              | 2026-11-09 |
| [#2219](https://github.com/developing-today-forks/opencode/pull/2219) | fix(core): normalize v1-dialect reasoning_effort in Provider.nativeSettings (#2203)                                   | dezren39              | 2026-11-09 |
| [#2220](https://github.com/developing-today-forks/opencode/pull/2220) | fix(core): redact SessionError payloads in sanitized session exports (#2207)                                          | dezren39              | 2026-11-09 |
| [#2201](https://github.com/developing-today-forks/opencode/pull/2201) | fix(core): add shared delegation guidance to all prompts                                                              | dezren39              | 2026-11-09 |
| [#2224](https://github.com/developing-today-forks/opencode/pull/2224) | fix(core): hedged race won by the hedge leg is not a failover (#2223)                                                 | dezren39              | 2026-11-09 |
| [#2216](https://github.com/developing-today-forks/opencode/pull/2216) | fix(core): cap an ordinary failure's bench at failureCapMs — a flaky leg keeps a short grace (#2212)                  | developing-today[bot] | 2026-11-09 |
| [#2237](https://github.com/developing-today-forks/opencode/pull/2237) | fix(core): Codex credential sync targets the account's instance door (code#7991)                                      | dezren39              | 2026-11-09 |
| [#2246](https://github.com/developing-today-forks/opencode/pull/2246) | fix(core): subagent depth line uses the effective cap enforcement enforces                                            | dezren39              | 2026-11-10 |
| [#2247](https://github.com/developing-today-forks/opencode/pull/2247) | fix(core): preserve delivery mode on delivered prompt retries                                                         | dezren39              | 2026-11-10 |
| [#2248](https://github.com/developing-today-forks/opencode/pull/2248) | fix(core): last resort de-prefers quota-dead targets in every scope (code#8086)                                       | dezren39              | 2026-11-10 |
| [#2250](https://github.com/developing-today-forks/opencode/pull/2250) | fix(core): Codex credential sync never targets the self-contained _openai door (code#8163)                            | dezren39              | 2026-11-10 |
| [#2252](https://github.com/developing-today-forks/opencode/pull/2252) | fix(core): gathered quota snapshot plants the wall before any attempt (code#8086, #2036)                              | dezren39              | 2026-11-10 |
| [#2254](https://github.com/developing-today-forks/opencode/pull/2254) | fix(core): gathered quota pass keeps its own feed stamps (code#8086)                                                  | dezren39              | 2026-11-10 |
| [#2259](https://github.com/developing-today-forks/opencode/pull/2259) | fix(core): fallback cascade ends with a non-quota-dead last-resort pass (code#7073)                                   | dezren39              | 2026-11-10 |
| [#2262](https://github.com/developing-today-forks/opencode/pull/2262) | fix(core): CodexSync never targets any _openai door (code#8163)                                                       | dezren39              | 2026-11-10 |
| [#2263](https://github.com/developing-today-forks/opencode/pull/2263) | fix(core): session outcomes record the provider/model that failed (code#7073 step 4)                                  | dezren39              | 2026-11-10 |
| [#2264](https://github.com/developing-today-forks/opencode/pull/2264) | feat(core): opendesign membership caps — meter, wait, fail over (#2253)                                               | dezren39              | 2026-11-11 |
| [#2267](https://github.com/developing-today-forks/opencode/pull/2267) | fix(core): overlay session environment on server env in Shell.create (#2266)                                          | dezren39              | 2026-11-11 |
| [#2278](https://github.com/developing-today-forks/opencode/pull/2278) | fix(core): preserve compaction-hook edits when rebuilding summary history (#2277)                                     | dezren39              | 2026-11-11 |
| [#2279](https://github.com/developing-today-forks/opencode/pull/2279) | fix(core): coordinate file writes with the edit/patch transaction lock (#2276)                                        | dezren39              | 2026-11-11 |
| [#2282](https://github.com/developing-today-forks/opencode/pull/2282) | fix(core): repair mcpx typo by exact call-name span, not a name-built regex                                           | dezren39              | 2026-11-11 |
| [#2285](https://github.com/developing-today-forks/opencode/pull/2285) | fix(core): check permission for each webfetch redirect destination (#2283)                                            | dezren39              | 2026-11-11 |
| [#2288](https://github.com/developing-today-forks/opencode/pull/2288) | fix(core): stop workspace provisioning when its last waiter is interrupted                                            | dezren39              | 2026-11-11 |
| [#2294](https://github.com/developing-today-forks/opencode/pull/2294) | fix(core): detect BMPs from truncated prefixes                                                                        | dezren39              | 2026-11-12 |
| [#2293](https://github.com/developing-today-forks/opencode/pull/2293) | fix(core): budget Code Mode namespace summaries                                                                       | dezren39              | 2026-11-12 |
| [#2301](https://github.com/developing-today-forks/opencode/pull/2301) | fix(core): revalidate remote instruction redirect destinations                                                        | dezren39              | 2026-11-12 |
| [#2306](https://github.com/developing-today-forks/opencode/pull/2306) | fix(core): validate MCP OAuth callback state first                                                                    | dezren39              | 2026-11-12 |
| [#2334](https://github.com/developing-today-forks/opencode/pull/2334) | fix(core): retain model-route events after persistence failures (#2331)                                               | dezren39              | 2026-11-13 |
| [#2318](https://github.com/developing-today-forks/opencode/pull/2318) | fix(core): preserve catalog baseURL pins on conflict                                                                  | dezren39              | 2026-11-13 |
| [#2411](https://github.com/developing-today-forks/opencode/pull/2411) | fix(core): republish authoritative usage after route_attempt writer persists (#2408)                                  | dezren39              | 2026-11-13 |
| [#233](https://github.com/developing-today-forks/opencode/pull/233)   | feat(core): pick pool members by pattern and control effort per level                                                 | dezren39              | 2026-11-14 |
| [#2418](https://github.com/developing-today-forks/opencode/pull/2418) | feat(core): register Deepgram, Google, AssemblyAI, ElevenLabs STT backends (#2386)                                    | dezren39              | 2026-11-14 |
| [#2419](https://github.com/developing-today-forks/opencode/pull/2419) | fix(core): bound per-leg buffering in session hedges (#2376)                                                          | dezren39              | 2026-11-14 |
| [#2421](https://github.com/developing-today-forks/opencode/pull/2421) | feat(core): #566 slice 1 — root-start quota gate predicate + rootStartMaxPct config                                   | dezren39              | 2026-11-14 |
| [#2424](https://github.com/developing-today-forks/opencode/pull/2424) | fix(core): gathered quota wall no longer overwrites attempt-reported skips (#2332)                                    | dezren39              | 2026-11-14 |
| [#2426](https://github.com/developing-today-forks/opencode/pull/2426) | fix(core): give the direct-model fallback the same last-resort pass as the routed fallback (#2382)                    | dezren39              | 2026-11-14 |
| [#2427](https://github.com/developing-today-forks/opencode/pull/2427) | fix(core): compose membership gate with hedging for capped primaries (#2383)                                          | dezren39              | 2026-11-14 |
| [#2410](https://github.com/developing-today-forks/opencode/pull/2410) | fix(core): stop Copilot stream tool-call emission after a stream error (#2407)                                        | dezren39              | 2026-11-14 |
| [#2429](https://github.com/developing-today-forks/opencode/pull/2429) | fix(core): treat a streak of no-message completions as a failed attempt (#1903)                                       | dezren39              | 2026-11-14 |
| [#2432](https://github.com/developing-today-forks/opencode/pull/2432) | fix(core): last-resort seat carries recheck time; Retry honours earliest re-admission (#1806)                         | dezren39              | 2026-11-14 |
| [#2438](https://github.com/developing-today-forks/opencode/pull/2438) | fix(core): remove value-import aliasing in transcription-facade (#2437)                                               | dezren39              | 2026-11-14 |
| [#2440](https://github.com/developing-today-forks/opencode/pull/2440) | fix(core): never retry a dead or walled leg, and bound a step's retries (code#9549)                                   | dezren39              | 2026-11-14 |
| [#2441](https://github.com/developing-today-forks/opencode/pull/2441) | fix(core): half-open probes for auth benches (code#9950)                                                              | dezren39              | 2026-11-14 |
| [#2444](https://github.com/developing-today-forks/opencode/pull/2444) | test(core): pin resolver-level auth bench probe admission (#9950)                                                     | dezren39              | 2026-11-14 |
| [#2449](https://github.com/developing-today-forks/opencode/pull/2449) | fix(core): effort-scoped traffic benches; long retry-after benches leave last-resort and probe at expiry              | dezren39              | 2026-11-14 |
| [#2459](https://github.com/developing-today-forks/opencode/pull/2459) | fix(core): probe-pending bench claims expire at a named bound (#2455)                                                 | dezren39              | 2026-11-15 |
| [#2464](https://github.com/developing-today-forks/opencode/pull/2464) | feat(core): #566 slice 2 — root-start placement wiring (root vs subsession)                                           | dezren39              | 2026-11-15 |
| [#2480](https://github.com/developing-today-forks/opencode/pull/2480) | fix(core): gathered provider-wide quota wall also gates the provider scope (#2479)                                    | dezren39              | 2026-11-15 |
| [#2506](https://github.com/developing-today-forks/opencode/pull/2506) | fix(core): apply opencode-route overrides to pools built in the later model transform (#239)                          | dezren39              | 2026-11-16 |
| [#2509](https://github.com/developing-today-forks/opencode/pull/2509) | fix(core): diagnose model pools whose matching keys select no models (#238)                                           | dezren39              | 2026-11-16 |
| [#2512](https://github.com/developing-today-forks/opencode/pull/2512) | fix(core): disposition render-card proto-read candidates (#2475)                                                      | dezren39              | 2026-11-16 |
| [#2514](https://github.com/developing-today-forks/opencode/pull/2514) | feat(core): suggest-first plan check-off for execution plans (#10)                                                    | dezren39              | 2026-11-16 |
| [#2516](https://github.com/developing-today-forks/opencode/pull/2516) | feat(core): wire ExecutionPlan suggestions into the post-turn path (#10)                                              | dezren39              | 2026-11-16 |
| [#2557](https://github.com/developing-today-forks/opencode/pull/2557) | fix(core): re-admit provider on provider-window headroom without a model reading                                      | dezren39              | 2026-11-16 |
| [#2559](https://github.com/developing-today-forks/opencode/pull/2559) | fix(core): auth-benched legs are dead for last-resort selection (#2555)                                               | dezren39              | 2026-11-16 |
| [#2570](https://github.com/developing-today-forks/opencode/pull/2570) | fix(core): bench permanent plan/tier refusals as plan walls (#10128)                                                  | dezren39              | 2026-11-17 |
| [#2572](https://github.com/developing-today-forks/opencode/pull/2572) | test(core): completeness-guard the tool-schema meta-validation inventory (#1902)                                      | dezren39              | 2026-11-17 |
| [#2573](https://github.com/developing-today-forks/opencode/pull/2573) | fix(core): make aisdk HOSTS meta rewrite coverage explicit (#2230)                                                    | dezren39              | 2026-11-17 |
| [#2571](https://github.com/developing-today-forks/opencode/pull/2571) | fix(core): type session-step test helpers so core typecheck passes                                                    | dezren39              | 2026-11-17 |
| [#2581](https://github.com/developing-today-forks/opencode/pull/2581) | fix(core): bound the model-route telemetry persistence backlog (#2335)                                                | dezren39              | 2026-11-17 |
| [#2580](https://github.com/developing-today-forks/opencode/pull/2580) | fix(core): reject non-positive quota window durations                                                                 | dezren39              | 2026-11-17 |
| [#2582](https://github.com/developing-today-forks/opencode/pull/2582) | fix(core): a joined Form.ask timeout no longer cancels the shared form (#2445)                                        | dezren39              | 2026-11-17 |
| [#2586](https://github.com/developing-today-forks/opencode/pull/2586) | test(core): pin prototype-member fallback for model route weights (#2472)                                             | dezren39              | 2026-11-17 |
| [#2588](https://github.com/developing-today-forks/opencode/pull/2588) | fix(core): disposition bare Number() on regex-captured exclusion instant in model-resolver                            | dezren39              | 2026-11-17 |
| [#2595](https://github.com/developing-today-forks/opencode/pull/2595) | fix(core): classify FileAccess targets by canonical location (#2286)                                                  | dezren39              | 2026-11-17 |
| [#2598](https://github.com/developing-today-forks/opencode/pull/2598) | feat(core): wall providers below 1% binding quota when quota failures are on record                                   | dezren39              | 2026-11-17 |
| [#2601](https://github.com/developing-today-forks/opencode/pull/2601) | fix(core): release hedge membership slots when the race is interrupted                                                | dezren39              | 2026-11-17 |
| [#2610](https://github.com/developing-today-forks/opencode/pull/2610) | fix(core): disposition render-wiring proto-read candidate (#2585)                                                     | dezren39              | 2026-11-17 |
| [#2612](https://github.com/developing-today-forks/opencode/pull/2612) | fix(core): resolve plan suggestions by content, not position (#2521)                                                  | dezren39              | 2026-11-17 |
| [#2624](https://github.com/developing-today-forks/opencode/pull/2624) | fix(core): default-model resolution carries session context to the #566 root-start gate (#2622)                       | dezren39              | 2026-11-17 |
| [#2643](https://github.com/developing-today-forks/opencode/pull/2643) | fix(core): parse render answers only treat chat as chat (#2620)                                                       | dezren39              | 2026-11-20 |
| [#2640](https://github.com/developing-today-forks/opencode/pull/2640) | fix(core): unwrap v1 variant options bags                                                                             | dezren39              | 2026-11-20 |
| [#2667](https://github.com/developing-today-forks/opencode/pull/2667) | fix(core): end no-candidate walls instead of sleeping in-lane; bench 429s until stated reset (#12228)                 | dezren39              | 2026-11-22 |
| [#2669](https://github.com/developing-today-forks/opencode/pull/2669) | test(core): cover 403 dead-door failover                                                                              | dezren39              | 2026-11-22 |
| [#2672](https://github.com/developing-today-forks/opencode/pull/2672) | feat(core): pin a preferred pool member per session                                                                   | dezren39              | 2026-11-22 |
| [#2674](https://github.com/developing-today-forks/opencode/pull/2674) | feat(core): model default effort, pool waits that follow nested pools, pools in config order                          | dezren39              | 2026-11-22 |
| [#2675](https://github.com/developing-today-forks/opencode/pull/2675) | fix(core): fail over on a content-filter finish before output; publish background shell timeouts                      | dezren39              | 2026-11-22 |
| [#2676](https://github.com/developing-today-forks/opencode/pull/2676) | feat(core): the last target a request can use is not cut off by the first-output wait                                 | dezren39              | 2026-11-22 |
| [#2679](https://github.com/developing-today-forks/opencode/pull/2679) | fix(core): a pool turn moves on to the next member when a failure after output is not retried                         | dezren39              | 2026-11-22 |
| [#2682](https://github.com/developing-today-forks/opencode/pull/2682) | test(core): five-door measurement for the walled-door failover criteria (#12228)                                      | dezren39              | 2026-11-22 |
| [#2685](https://github.com/developing-today-forks/opencode/pull/2685) | fix(core): keep a daemon command's output file on disk at the shell output cap                                        | dezren39              | 2026-11-22 |
| [#2686](https://github.com/developing-today-forks/opencode/pull/2686) | feat(core): archive a session at a carried-over time                                                                  | dezren39              | 2026-11-22 |
| [#2668](https://github.com/developing-today-forks/opencode/pull/2668) | test(core): cover quota-wall session failover                                                                         | dezren39              | 2026-11-23 |
| [#2692](https://github.com/developing-today-forks/opencode/pull/2692) | feat(core): cross-route failover within the same class (#12228)                                                       | dezren39              | 2026-11-23 |
| [#2699](https://github.com/developing-today-forks/opencode/pull/2699) | test(core): pin bench granularity seeded by failure class (code#10367 point 2)                                        | dezren39              | 2026-11-23 |
| [#2708](https://github.com/developing-today-forks/opencode/pull/2708) | fix(core): judge continuation members against the active history size                                                 | dezren39              | 2026-11-23 |
| [#2716](https://github.com/developing-today-forks/opencode/pull/2716) | fix(core): settle interrupted file mutations as possibly already applied                                              | dezren39              | 2026-11-23 |
| [#2717](https://github.com/developing-today-forks/opencode/pull/2717) | fix(core): key route-overrides cache on file identity, not mtime alone                                                | dezren39              | 2026-11-23 |
| [#2718](https://github.com/developing-today-forks/opencode/pull/2718) | fix(core): re-ensure a dead mcpx daemon once for projected tool calls                                                 | dezren39              | 2026-11-23 |
| [#2719](https://github.com/developing-today-forks/opencode/pull/2719) | fix(core): cap whole-file reads in the local filesystem driver                                                        | dezren39              | 2026-11-23 |
| [#2723](https://github.com/developing-today-forks/opencode/pull/2723) | fix(core): honor a joined Form.ask own timeout                                                                        | dezren39              | 2026-11-23 |
| [#2728](https://github.com/developing-today-forks/opencode/pull/2728) | fix(core): quota-fallback answers require complete context to classify                                                | dezren39              | 2026-11-23 |
| [#2739](https://github.com/developing-today-forks/opencode/pull/2739) | fix(core): shrink and retry when a compaction summary runs out of output room                                         | dezren39              | 2026-11-23 |
| [#2498](https://github.com/developing-today-forks/opencode/pull/2498) | feat(core): session-cookie credential kind, provider-only read posture (#2490)                                        | dezren39              | 2026-11-23 |
| [#2526](https://github.com/developing-today-forks/opencode/pull/2526) | fix(core): gate a provider after consecutive terminal auth failures (#2519)                                           | dezren39              | 2026-11-23 |
| [#2748](https://github.com/developing-today-forks/opencode/pull/2748) | fix(core): spend a granted after-output retry on the next pool member (#2681 item 1)                                  | dezren39              | 2026-11-23 |
| [#2750](https://github.com/developing-today-forks/opencode/pull/2750) | fix(core): bind fs.read streams to the checked file object                                                            | dezren39              | 2026-11-23 |
| [#2751](https://github.com/developing-today-forks/opencode/pull/2751) | fix(core): settle interrupted file mutation calls on restart                                                          | dezren39              | 2026-11-23 |
| [#2758](https://github.com/developing-today-forks/opencode/pull/2758) | fix(core): retain shell capture files while commands are running                                                      | dezren39              | 2026-11-23 |
| [#2766](https://github.com/developing-today-forks/opencode/pull/2766) | fix(core): purge and expire pending permission asks                                                                   | dezren39              | 2026-11-23 |
| [#2767](https://github.com/developing-today-forks/opencode/pull/2767) | fix(core): shell output reads surface a missing capture instead of an empty page                                      | dezren39              | 2026-11-23 |
| [#2780](https://github.com/developing-today-forks/opencode/pull/2780) | fix(core): refuse to overwrite an unreadable route-overrides store (#2771)                                            | dezren39              | 2026-11-23 |
| [#2742](https://github.com/developing-today-forks/opencode/pull/2742) | feat(core): learned bench granularity promotion and instant demotion                                                  | dezren39              | 2026-11-23 |
| [#2783](https://github.com/developing-today-forks/opencode/pull/2783) | fix(core): scope provider quota gates by sibling evidence + all-accounts-full aggregation (#13693 Slice F)            | dezren39              | 2026-11-23 |

##### developing-today-forks/opencode — theme: ai (181)

| PR                                                                    | Title                                                                                                                  | Author                | Merged     |
| --------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- | --------------------- | ---------- |
| [#679](https://github.com/developing-today-forks/opencode/pull/679)   | fix(ai): treat blank retry-after headers as absent                                                                     | dezren39              | 2026-10-28 |
| [#688](https://github.com/developing-today-forks/opencode/pull/688)   | fix(ai): make SpeechStream.headerUsage reject blank, negative and non-plain usage headers                              | dezren39              | 2026-10-28 |
| [#684](https://github.com/developing-today-forks/opencode/pull/684)   | fix(ai): restrict retry-after parsing to its decimal grammar                                                           | dezren39              | 2026-10-28 |
| [#691](https://github.com/developing-today-forks/opencode/pull/691)   | fix(ai): make ElevenLabs describeOutput reject non-plain and non-positive sample rates                                 | dezren39              | 2026-10-28 |
| [#702](https://github.com/developing-today-forks/opencode/pull/702)   | fix(ai): treat a non-positive SpeechStream.sampleRate as unknown                                                       | dezren39              | 2026-10-28 |
| [#783](https://github.com/developing-today-forks/opencode/pull/783)   | fix(ai): guard Replicate completed_at parsing against NaN and calendar rollover                                        | dezren39              | 2026-10-29 |
| [#858](https://github.com/developing-today-forks/opencode/pull/858)   | fix(ai): server-tool-result gate matches own keys only (#856)                                                          | dezren39              | 2026-10-29 |
| [#910](https://github.com/developing-today-forks/opencode/pull/910)   | fix(ai): read open-responses item-ID prefixes by own property (#908)                                                   | dezren39              | 2026-10-29 |
| [#911](https://github.com/developing-today-forks/opencode/pull/911)   | fix(ai): guard speech format tables against prototype-chain reads                                                      | dezren39              | 2026-10-29 |
| [#909](https://github.com/developing-today-forks/opencode/pull/909)   | fix(ai): guard media-type lookups against prototype-chain reads                                                        | dezren39              | 2026-10-29 |
| [#921](https://github.com/developing-today-forks/opencode/pull/921)   | fix(ai): guard bedrock-media MIME tables against prototype-chain reads                                                 | dezren39              | 2026-10-29 |
| [#922](https://github.com/developing-today-forks/opencode/pull/922)   | fix(ai): guard speech finish-path tables against prototype-chain reads                                                 | dezren39              | 2026-10-29 |
| [#928](https://github.com/developing-today-forks/opencode/pull/928)   | fix(ai): guard ResponsesHostedTools against prototype-named item types (#927)                                          | dezren39              | 2026-10-29 |
| [#1102](https://github.com/developing-today-forks/opencode/pull/1102) | fix(ai): bound provider error response bodies                                                                          | dezren39              | 2026-10-31 |
| [#1457](https://github.com/developing-today-forks/opencode/pull/1457) | feat(ai): antigravity v1internal provider — the antigravity-direct wire protocol in-fork (#1414)                       | dezren39              | 2026-11-02 |
| [#1474](https://github.com/developing-today-forks/opencode/pull/1474) | fix(ai): bound Antigravity token-refresh fetch (#1469)                                                                 | dezren39              | 2026-11-03 |
| [#1491](https://github.com/developing-today-forks/opencode/pull/1491) | chore(ai): disposition antigravity-auth content-length parse in numeric-boundaries net (#1487)                         | dezren39              | 2026-11-03 |
| [#1492](https://github.com/developing-today-forks/opencode/pull/1492) | fix(ai): bound recording-cost-report models.dev fetch with deadline and size cap (#1395)                               | dezren39              | 2026-11-03 |
| [#1549](https://github.com/developing-today-forks/opencode/pull/1549) | fix(ai): synthesize items for bare array schemas in antigravity conversion (#1548)                                     | dezren39              | 2026-11-03 |
| [#1555](https://github.com/developing-today-forks/opencode/pull/1555) | fix(ai): fold anyOf to oneOf for Anthropic targets in antigravity schemas (#1550)                                      | dezren39              | 2026-11-03 |
| [#1558](https://github.com/developing-today-forks/opencode/pull/1558) | fix(ai): carry tool-call ids on antigravity history parts for Claude targets (#1550)                                   | dezren39              | 2026-11-03 |
| [#1629](https://github.com/developing-today-forks/opencode/pull/1629) | fix(ai): emit one branch per member for multi-valued type unions in antigravity convertSchema (#1556)                  | dezren39              | 2026-11-04 |
| [#1642](https://github.com/developing-today-forks/opencode/pull/1642) | fix(ai): antigravity secret extraction tries all GOCSPX candidates (#1633)                                             | dezren39              | 2026-11-04 |
| [#1643](https://github.com/developing-today-forks/opencode/pull/1643) | fix(ai): cut multi-def $ref cycles in Gemini schema normalize (#1585)                                                  | dezren39              | 2026-11-04 |
| [#1663](https://github.com/developing-today-forks/opencode/pull/1663) | fix(ai): encode boolean false schemas as { not: {} } in antigravity convertSchema (#1573)                              | dezren39              | 2026-11-04 |
| [#1669](https://github.com/developing-today-forks/opencode/pull/1669) | fix(ai): stop forcing OBJECT onto allOf/not-only roots in antigravity convertSchema (#1667)                            | dezren39              | 2026-11-04 |
| [#1661](https://github.com/developing-today-forks/opencode/pull/1661) | feat(ai): vibe provider — native emulation of the Mistral Vibe binary (#1653)                                          | developing-today[bot] | 2026-11-05 |
| [#1647](https://github.com/developing-today-forks/opencode/pull/1647) | fix(ai): moonshot projection keeps prefixItems when items is also present (#1574)                                      | dezren39              | 2026-11-05 |
| [#1697](https://github.com/developing-today-forks/opencode/pull/1697) | fix(ai): let caller topP suppress Vibe temperature default                                                             | dezren39              | 2026-11-05 |
| [#1716](https://github.com/developing-today-forks/opencode/pull/1716) | fix(ai): send wire-complete antigravity model ids bare, not effort-suffixed (#1704)                                    | dezren39              | 2026-11-05 |
| [#1722](https://github.com/developing-today-forks/opencode/pull/1722) | feat(ai): provider quota functions and opencode quota subcommand                                                       | dezren39              | 2026-11-05 |
| [#1728](https://github.com/developing-today-forks/opencode/pull/1728) | fix(ai): retire anti-slop findings in quota code                                                                       | dezren39              | 2026-11-05 |
| [#1731](https://github.com/developing-today-forks/opencode/pull/1731) | fix(ai): detect OpenAI Chat compatibility by model family behind gateways                                              | dezren39              | 2026-11-05 |
| [#1723](https://github.com/developing-today-forks/opencode/pull/1723) | test(ai): disposition tables + fidelity corpus for the three tool-schema translators (#1685)                           | dezren39              | 2026-11-05 |
| [#1738](https://github.com/developing-today-forks/opencode/pull/1738) | fix(ai): detect DeepSeek model family behind multi-family gateways                                                     | dezren39              | 2026-11-05 |
| [#1742](https://github.com/developing-today-forks/opencode/pull/1742) | fix(ai): quota toNumber treats blank strings as absent, not zero (#1741)                                               | dezren39              | 2026-11-05 |
| [#1745](https://github.com/developing-today-forks/opencode/pull/1745) | fix(ai): publish Command Code monthly window when remaining exceeds included credits (#1744)                           | dezren39              | 2026-11-05 |
| [#1749](https://github.com/developing-today-forks/opencode/pull/1749) | feat(ai): native claude-code + commandcode direct providers — wire emulation, real client tool calls (code#6117)       | dezren39              | 2026-11-05 |
| [#1740](https://github.com/developing-today-forks/opencode/pull/1740) | fix(ai): preserve Gemini thought signatures across routes                                                              | dezren39              | 2026-11-05 |
| [#1761](https://github.com/developing-today-forks/opencode/pull/1761) | fix(ai): map commandcode wire reasoningTokens and provider totalTokens into Usage (#1760)                              | dezren39              | 2026-11-05 |
| [#1764](https://github.com/developing-today-forks/opencode/pull/1764) | fix(ai): treat blank credentials as absent in commandcode + claude-code auth bridges (#1763)                           | dezren39              | 2026-11-05 |
| [#1766](https://github.com/developing-today-forks/opencode/pull/1766) | test(ai): pin claude/meta/opendesign quota parsers and fix three divergences (#1750)                                   | dezren39              | 2026-11-05 |
| [#1767](https://github.com/developing-today-forks/opencode/pull/1767) | fix(ai): expired credentials-file token falls through to CLAUDE_CODE_OAUTH_TOKEN (#1752)                               | dezren39              | 2026-11-05 |
| [#1770](https://github.com/developing-today-forks/opencode/pull/1770) | fix(ai): preserve Codex quota windows on label collisions, drop phantom windows (#1762)                                | dezren39              | 2026-11-05 |
| [#1772](https://github.com/developing-today-forks/opencode/pull/1772) | fix(ai): fail loudly on nameless/id-less commandcode tool-calls (#1756)                                                | developing-today[bot] | 2026-11-05 |
| [#1773](https://github.com/developing-today-forks/opencode/pull/1773) | fix(ai): cline-pass quota keep-first on duplicate limit entries (#1769)                                                | developing-today[bot] | 2026-11-05 |
| [#1771](https://github.com/developing-today-forks/opencode/pull/1771) | fix(ai): remove purchased/free credit balance windows from Command Code quota                                          | dezren39              | 2026-11-05 |
| [#1787](https://github.com/developing-today-forks/opencode/pull/1787) | fix(ai): map commandcode content-filter/error/abort finishes honestly (#1755)                                          | developing-today[bot] | 2026-11-05 |
| [#1796](https://github.com/developing-today-forks/opencode/pull/1796) | test(ai): quota window contract — parser inventory + shared probe matrix (#1795)                                       | dezren39              | 2026-11-06 |
| [#1801](https://github.com/developing-today-forks/opencode/pull/1801) | fix(ai): openrouter zero-limit publishes no quota window (#1799)                                                       | dezren39              | 2026-11-06 |
| [#1810](https://github.com/developing-today-forks/opencode/pull/1810) | fix(ai): fail loudly on id-less or nameless commandcode tool-results (#1804)                                           | dezren39              | 2026-11-06 |
| [#1812](https://github.com/developing-today-forks/opencode/pull/1812) | fix(ai): strict groq quota header and reset parsing                                                                    | dezren39              | 2026-11-06 |
| [#1813](https://github.com/developing-today-forks/opencode/pull/1813) | fix(ai): Anthropic Messages default endpoint rendered /v1/v1/messages (claude-code 404)                                | dezren39              | 2026-11-06 |
| [#1815](https://github.com/developing-today-forks/opencode/pull/1815) | fix(ai): codex quota isConfigured counts only active credentials                                                       | dezren39              | 2026-11-06 |
| [#1826](https://github.com/developing-today-forks/opencode/pull/1826) | fix(ai): strip the git-repo env line on the claude-code route (#1819)                                                  | dezren39              | 2026-11-06 |
| [#1833](https://github.com/developing-today-forks/opencode/pull/1833) | fix(ai): quota providers treat blank credentials as not configured (#1831)                                             | dezren39              | 2026-11-06 |
| [#1835](https://github.com/developing-today-forks/opencode/pull/1835) | fix(ai): direct providers present their binary's User-Agent on the wire                                                | dezren39              | 2026-11-06 |
| [#1836](https://github.com/developing-today-forks/opencode/pull/1836) | fix(ai): keep Mistral budget fields consistent                                                                         | dezren39              | 2026-11-06 |
| [#1837](https://github.com/developing-today-forks/opencode/pull/1837) | fix(ai): antigravity quota buckets skip unrecognized windows and keep first on label collisions (#1768)                | dezren39              | 2026-11-06 |
| [#1829](https://github.com/developing-today-forks/opencode/pull/1829) | fix(ai): clamp command-code windowLimits windows whole and null non-positive resetAt (#1821)                           | dezren39              | 2026-11-06 |
| [#1830](https://github.com/developing-today-forks/opencode/pull/1830) | fix(ai): remove unreachable prefixItems else-if in antigravity union branches (#1820)                                  | dezren39              | 2026-11-06 |
| [#1838](https://github.com/developing-today-forks/opencode/pull/1838) | fix(ai): antigravity-auth applies the blank-credential rule (#1765)                                                    | dezren39              | 2026-11-06 |
| [#1840](https://github.com/developing-today-forks/opencode/pull/1840) | fix(ai): relocate model-family defaults to gateway-door compatibility entries (#1824)                                  | dezren39              | 2026-11-06 |
| [#1843](https://github.com/developing-today-forks/opencode/pull/1843) | fix(ai): GOCSPX scan recovers stable-boundary straddles whole (#1822)                                                  | dezren39              | 2026-11-06 |
| [#1844](https://github.com/developing-today-forks/opencode/pull/1844) | fix(ai): antigravity missing-binary errors name the binary and remedies (#1825)                                        | dezren39              | 2026-11-06 |
| [#1846](https://github.com/developing-today-forks/opencode/pull/1846) | fix(ai): bound CDP WebSocket requests — per-request timeout + close/error settlement (#1803)                           | dezren39              | 2026-11-06 |
| [#1847](https://github.com/developing-today-forks/opencode/pull/1847) | fix(ai): mistral quota budget label uses payload currency (#1805)                                                      | dezren39              | 2026-11-06 |
| [#1850](https://github.com/developing-today-forks/opencode/pull/1850) | fix(ai): codex quota treats non-positive limit_window_seconds as unknown duration (#1848)                              | dezren39              | 2026-11-06 |
| [#1851](https://github.com/developing-today-forks/opencode/pull/1851) | fix(ai): shared quota toTimestamp returns null for non-positive resets (#1845)                                         | dezren39              | 2026-11-06 |
| [#1856](https://github.com/developing-today-forks/opencode/pull/1856) | fix(ai): meta quota portal resets route through toTimestamp — non-positive resets publish null, not 1969 dates (#1855) | dezren39              | 2026-11-06 |
| [#1854](https://github.com/developing-today-forks/opencode/pull/1854) | fix(ai): vibe-auth shadow diagnostic when a non-keyring tier wins (#1809)                                              | dezren39              | 2026-11-06 |
| [#1859](https://github.com/developing-today-forks/opencode/pull/1859) | fix(ai): normalizeTimestamp returns null for non-positive expiries — no 1969/1970 credentialExpiresAt (#1857)          | developing-today[bot] | 2026-11-06 |
| [#1861](https://github.com/developing-today-forks/opencode/pull/1861) | feat(ai): in-provider OAuth sign-in initiation for claude-code (#1852)                                                 | dezren39              | 2026-11-06 |
| [#1816](https://github.com/developing-today-forks/opencode/pull/1816) | fix(ai): preserve positive DeepSeek quota balances                                                                     | dezren39              | 2026-11-06 |
| [#1863](https://github.com/developing-today-forks/opencode/pull/1863) | fix(ai): remove duplicate deepseekProvider import in quota tests                                                       | dezren39              | 2026-11-06 |
| [#1865](https://github.com/developing-today-forks/opencode/pull/1865) | fix(ai): bound Claude Code sign-in token exchange                                                                      | dezren39              | 2026-11-06 |
| [#1860](https://github.com/developing-today-forks/opencode/pull/1860) | fix(ai): meta subscription quota drops only the window with a non-positive limit (#1849)                               | dezren39              | 2026-11-06 |
| [#1869](https://github.com/developing-today-forks/opencode/pull/1869) | fix(ai): claude-code sign-in callback retries after a failed attempt (#1868)                                           | dezren39              | 2026-11-06 |
| [#1873](https://github.com/developing-today-forks/opencode/pull/1873) | fix(ai): vibe-direct presents the Mistral backend's real User-Agent                                                    | dezren39              | 2026-11-06 |
| [#1875](https://github.com/developing-today-forks/opencode/pull/1875) | fix(ai): commandcode provider must not inherit the catalog hosted-API baseURL                                          | dezren39              | 2026-11-06 |
| [#1878](https://github.com/developing-today-forks/opencode/pull/1878) | fix(ai): readQuotaJson enforces its 1 MiB cap in UTF-8 bytes, not UTF-16 code units (#1877)                            | dezren39              | 2026-11-06 |
| [#1894](https://github.com/developing-today-forks/opencode/pull/1894) | fix(ai): bound the muse-direct key-exchange response at 1 MiB (#1893)                                                  | dezren39              | 2026-11-06 |
| [#1896](https://github.com/developing-today-forks/opencode/pull/1896) | fix(ai): bound the CDP websocket connect phase (#1895)                                                                 | dezren39              | 2026-11-06 |
| [#1905](https://github.com/developing-today-forks/opencode/pull/1905) | fix(ai): cap toUsageWindow remainingPercent at 100 (#1904)                                                             | dezren39              | 2026-11-07 |
| [#1908](https://github.com/developing-today-forks/opencode/pull/1908) | fix(ai): meta quota key-endpoint drops figure-less subs_usage windows (#1907)                                          | dezren39              | 2026-11-07 |
| [#1916](https://github.com/developing-today-forks/opencode/pull/1916) | fix(ai): claude quota returns the no-data contract for a figure-less 200 payload (#1915)                               | developing-today[bot] | 2026-11-07 |
| [#1918](https://github.com/developing-today-forks/opencode/pull/1918) | fix(ai): codex spend-control label and percent derive from one clamped figure                                          | developing-today[bot] | 2026-11-07 |
| [#1930](https://github.com/developing-today-forks/opencode/pull/1930) | fix(ai): vibe-auth recovers from a 401 — invalidate, re-resolve, retry once (#1900)                                    | dezren39              | 2026-11-07 |
| [#1935](https://github.com/developing-today-forks/opencode/pull/1935) | fix(ai): stop forcing OBJECT onto enum/items roots in antigravity convertSchema (#1901)                                | dezren39              | 2026-11-07 |
| [#1936](https://github.com/developing-today-forks/opencode/pull/1936) | docs(ai): flip antigravity/mistral quota inventory rows to conforming (#1909)                                          | dezren39              | 2026-11-07 |
| [#1937](https://github.com/developing-today-forks/opencode/pull/1937) | fix(ai): antigravity transformModels fallback publishes no window without a quota figure (#1912)                       | dezren39              | 2026-11-07 |
| [#1938](https://github.com/developing-today-forks/opencode/pull/1938) | fix(ai): openrouter unlimited path reads a negative spend as absent (#1913)                                            | dezren39              | 2026-11-07 |
| [#1940](https://github.com/developing-today-forks/opencode/pull/1940) | fix(ai): antigravity summary path publishes no window without a quota figure (#1917)                                   | dezren39              | 2026-11-07 |
| [#1949](https://github.com/developing-today-forks/opencode/pull/1949) | fix(ai): read claude spend money with an out-of-range exponent as absent (#1920)                                       | dezren39              | 2026-11-07 |
| [#1951](https://github.com/developing-today-forks/opencode/pull/1951) | fix(ai): stop forcing OBJECT onto constraint-only roots in antigravity convertSchema (#1950)                           | dezren39              | 2026-11-07 |
| [#1953](https://github.com/developing-today-forks/opencode/pull/1953) | fix(ai): moonshot projection passes literal enum/default/const values through verbatim (#1952)                         | dezren39              | 2026-11-07 |
| [#1957](https://github.com/developing-today-forks/opencode/pull/1957) | fix(ai): claude quota spend reads negative amounts as absent                                                           | dezren39              | 2026-11-07 |
| [#1958](https://github.com/developing-today-forks/opencode/pull/1958) | fix(ai): honour disabled_providers for the built-in turso quota fetcher                                                | dezren39              | 2026-11-07 |
| [#1921](https://github.com/developing-today-forks/opencode/pull/1921) | fix(ai): clamp groq quota remaining to its limit                                                                       | dezren39              | 2026-11-08 |
| [#1960](https://github.com/developing-today-forks/opencode/pull/1960) | fix(ai): de-duplicate trailing /v1 in Endpoint.render (#1956)                                                          | dezren39              | 2026-11-08 |
| [#1963](https://github.com/developing-today-forks/opencode/pull/1963) | docs(ai): update quota inventory to current parser state                                                               | dezren39              | 2026-11-08 |
| [#1961](https://github.com/developing-today-forks/opencode/pull/1961) | fix(ai): scope Muse minted-key cache by credential source                                                              | dezren39              | 2026-11-08 |
| [#2037](https://github.com/developing-today-forks/opencode/pull/2037) | test(ai): pin antigravity model catalog classification against a live probe (#1719)                                    | dezren39              | 2026-11-08 |
| [#2043](https://github.com/developing-today-forks/opencode/pull/2043) | fix(ai): claude quota duplicates keep the first entry (#1781)                                                          | developing-today[bot] | 2026-11-08 |
| [#2101](https://github.com/developing-today-forks/opencode/pull/2101) | fix(ai): clamp opendesign remaining credits for percent and label (#1879)                                              | dezren39              | 2026-11-08 |
| [#1726](https://github.com/developing-today-forks/opencode/pull/1726) | test(ai): cover quota parsers and reset boundaries                                                                     | dezren39              | 2026-11-08 |
| [#2104](https://github.com/developing-today-forks/opencode/pull/2104) | fix(ai): meta rolling window key never collides with weekly                                                            | dezren39              | 2026-11-08 |
| [#2106](https://github.com/developing-today-forks/opencode/pull/2106) | fix(ai): keep multi-valued type conjunctive with anyOf/oneOf in antigravity convertSchema (#1926)                      | developing-today[bot] | 2026-11-08 |
| [#2109](https://github.com/developing-today-forks/opencode/pull/2109) | test(ai): reclassify moonshot prefixItems corpus row to approximated/widening (#1933)                                  | developing-today[bot] | 2026-11-08 |
| [#2107](https://github.com/developing-today-forks/opencode/pull/2107) | fix(ai): derive quota resetAfterFormatted from resetAfterSeconds (#2103)                                               | dezren39              | 2026-11-08 |
| [#2098](https://github.com/developing-today-forks/opencode/pull/2098) | test(ai): assert claude-code CLI User-Agent on the wire through HTTP transport (#1834)                                 | dezren39              | 2026-11-08 |
| [#2117](https://github.com/developing-today-forks/opencode/pull/2117) | test(ai): lift quota suites for #1744/#1746/#1821 out of the #1741 describe (#1934)                                    | developing-today[bot] | 2026-11-08 |
| [#1870](https://github.com/developing-today-forks/opencode/pull/1870) | fix(ai): strip comments from quoted dotenv values                                                                      | dezren39              | 2026-11-08 |
| [#2121](https://github.com/developing-today-forks/opencode/pull/2121) | fix(ai): read a negative codex credits balance as absent (#1924)                                                       | dezren39              | 2026-11-08 |
| [#2123](https://github.com/developing-today-forks/opencode/pull/2123) | fix(ai): publish float percents from antigravity remainingFraction (#1941)                                             | dezren39              | 2026-11-08 |
| [#2124](https://github.com/developing-today-forks/opencode/pull/2124) | fix(ai): reject negative DeepSeek balances                                                                             | dezren39              | 2026-11-08 |
| [#2125](https://github.com/developing-today-forks/opencode/pull/2125) | test(ai): quota figure contract — figure inventory + shared figure probe matrix (#1943)                                | dezren39              | 2026-11-08 |
| [#2126](https://github.com/developing-today-forks/opencode/pull/2126) | fix(ai): renew the claude-code OAuth credential with an atomic write-back (#1818)                                      | dezren39              | 2026-11-08 |
| [#2128](https://github.com/developing-today-forks/opencode/pull/2128) | fix(ai): reject non-UUID commandcode threadId and validate permissionMode                                              | dezren39              | 2026-11-08 |
| [#2130](https://github.com/developing-today-forks/opencode/pull/2130) | fix(ai): publish a zero deepseek balance in a third currency (#2129)                                                   | dezren39              | 2026-11-08 |
| [#2131](https://github.com/developing-today-forks/opencode/pull/2131) | docs(ai): disclose Command Code config sent to gateway                                                                 | dezren39              | 2026-11-08 |
| [#2137](https://github.com/developing-today-forks/opencode/pull/2137) | fix(ai): reject non-decimal spellings in the shared quota toNumber (#2102)                                             | dezren39              | 2026-11-08 |
| [#2138](https://github.com/developing-today-forks/opencode/pull/2138) | fix(ai): de-duplicate media route version segments                                                                     | developing-today[bot] | 2026-11-08 |
| [#2140](https://github.com/developing-today-forks/opencode/pull/2140) | fix(ai): recognize daily spelling in antigravity parseBucket                                                           | developing-today[bot] | 2026-11-08 |
| [#2141](https://github.com/developing-today-forks/opencode/pull/2141) | fix(ai): quota model maps keep a model named **proto** (#2110)                                                         | developing-today[bot] | 2026-11-08 |
| [#2143](https://github.com/developing-today-forks/opencode/pull/2143) | test(ai): pin single version segment for BFL/Replicate/Stability image routes (#1962)                                  | dezren39              | 2026-11-08 |
| [#2144](https://github.com/developing-today-forks/opencode/pull/2144) | fix(ai): dispatch media-type sniffer on ftyp major brand (#2127)                                                       | dezren39              | 2026-11-08 |
| [#2155](https://github.com/developing-today-forks/opencode/pull/2155) | fix(ai): align claude-code and commandcode bare routes to renamed provider ids (#1945)                                 | dezren39              | 2026-11-08 |
| [#2157](https://github.com/developing-today-forks/opencode/pull/2157) | fix(ai): meta quota portal read uses the portal page team (#1931)                                                      | developing-today[bot] | 2026-11-08 |
| [#2164](https://github.com/developing-today-forks/opencode/pull/2164) | fix(ai): remove binary credential dependencies                                                                         | dezren39              | 2026-11-08 |
| [#2173](https://github.com/developing-today-forks/opencode/pull/2173) | fix(ai): refuse to persist an empty claude-code credential at the write boundary (code#7098)                           | dezren39              | 2026-11-08 |
| [#2172](https://github.com/developing-today-forks/opencode/pull/2172) | fix(ai): clean up credential temp file when rename fails                                                               | dezren39              | 2026-11-08 |
| [#2176](https://github.com/developing-today-forks/opencode/pull/2176) | fix(ai): exclude UTF-16 BOMs from media frame-sync sniff (#2151)                                                       | dezren39              | 2026-11-08 |
| [#2170](https://github.com/developing-today-forks/opencode/pull/2170) | fix(ai): multiline-aware parseDotEnv in vibe-auth                                                                      | dezren39              | 2026-11-08 |
| [#2179](https://github.com/developing-today-forks/opencode/pull/2179) | test(ai): pin cdpPageFetch page-eval-exception passthrough (#2165)                                                     | dezren39              | 2026-11-08 |
| [#2169](https://github.com/developing-today-forks/opencode/pull/2169) | test(ai): pin vibe-auth shadow diagnostic never-break guarantee                                                        | dezren39              | 2026-11-08 |
| [#2181](https://github.com/developing-today-forks/opencode/pull/2181) | fix(ai): keep every record entry when sanitized keys collide                                                           | dezren39              | 2026-11-08 |
| [#2182](https://github.com/developing-today-forks/opencode/pull/2182) | fix(ai): reject claude-code callback without state                                                                     | dezren39              | 2026-11-08 |
| [#2183](https://github.com/developing-today-forks/opencode/pull/2183) | fix(ai): preserve **proto** keys in mergeJsonRecords                                                                   | dezren39              | 2026-11-08 |
| [#2184](https://github.com/developing-today-forks/opencode/pull/2184) | fix(ai): track pending tool calls per occurrence in normalizeToolHistory                                               | dezren39              | 2026-11-08 |
| [#2185](https://github.com/developing-today-forks/opencode/pull/2185) | fix(ai): trim Turso plan-label matching                                                                                | dezren39              | 2026-11-08 |
| [#2205](https://github.com/developing-today-forks/opencode/pull/2205) | fix(ai): getAuthEntry reads own properties only (#2189)                                                                | developing-today[bot] | 2026-11-09 |
| [#2215](https://github.com/developing-today-forks/opencode/pull/2215) | fix(ai): guard nested partial-json number trim with Allow.NUM and scope it to the token (#2213)                        | dezren39              | 2026-11-09 |
| [#2161](https://github.com/developing-today-forks/opencode/pull/2161) | fix(ai): prefer stored Muse inference key                                                                              | dezren39              | 2026-11-09 |
| [#2180](https://github.com/developing-today-forks/opencode/pull/2180) | fix(ai): scope session-expired URL matching to path segments                                                           | dezren39              | 2026-11-09 |
| [#2236](https://github.com/developing-today-forks/opencode/pull/2236) | fix(ai): leave format-only roots self-describing in antigravity convertSchema (#2235)                                  | dezren39              | 2026-11-09 |
| [#2243](https://github.com/developing-today-forks/opencode/pull/2243) | fix(ai): isRetryable ignores the wire x-should-retry header (#2225)                                                    | dezren39              | 2026-11-10 |
| [#2255](https://github.com/developing-today-forks/opencode/pull/2255) | fix(ai): claude-code renewal renews early and waits out rate limits (code#6120)                                        | dezren39              | 2026-11-10 |
| [#2257](https://github.com/developing-today-forks/opencode/pull/2257) | fix(ai): fold opencode-go instance rows into the quota answer (code#8260)                                              | dezren39              | 2026-11-10 |
| [#2260](https://github.com/developing-today-forks/opencode/pull/2260) | test(ai): opendesign gateway tool-call cassettes, opencode-go catalog pin, index-reuse parser fix (#1708)              | dezren39              | 2026-11-10 |
| [#2261](https://github.com/developing-today-forks/opencode/pull/2261) | fix(ai): rename the OpenAI quota provider id from codex to _openai (code#8163)                                         | dezren39              | 2026-11-10 |
| [#2270](https://github.com/developing-today-forks/opencode/pull/2270) | fix(ai): standing renewal retry for Claude Code credentials                                                            | dezren39              | 2026-11-11 |
| [#2312](https://github.com/developing-today-forks/opencode/pull/2312) | fix(ai): discard incomplete Copilot tool calls after stream errors (#2311)                                             | dezren39              | 2026-11-12 |
| [#2297](https://github.com/developing-today-forks/opencode/pull/2297) | fix(ai): parse standalone speech sample rate parameter                                                                 | dezren39              | 2026-11-12 |
| [#2378](https://github.com/developing-today-forks/opencode/pull/2378) | fix(ai): bound WebSocket inbound queue memory                                                                          | dezren39              | 2026-11-13 |
| [#2404](https://github.com/developing-today-forks/opencode/pull/2404) | fix(ai): clear post-#2403 main test failures — bounded-http dispositions + hermetic v1-dialect test (#2392)            | dezren39              | 2026-11-13 |
| [#2423](https://github.com/developing-today-forks/opencode/pull/2423) | fix(ai): antigravity convertSchema lowers const to a one-element enum (#1557)                                          | dezren39              | 2026-11-14 |
| [#2425](https://github.com/developing-today-forks/opencode/pull/2425) | fix(ai): restore the session-bus address synthesis in lookupSecretService (#2313)                                      | dezren39              | 2026-11-14 |
| [#2461](https://github.com/developing-today-forks/opencode/pull/2461) | fix(ai): restore codex as a legacy alias for the _openai quota provider (#2460)                                        | dezren39              | 2026-11-15 |
| [#2439](https://github.com/developing-today-forks/opencode/pull/2439) | test(ai): cline-pass gateway tool-call cassettes, glm-5.3-flash recorded (#1708)                                       | dezren39              | 2026-11-15 |
| [#2474](https://github.com/developing-today-forks/opencode/pull/2474) | fix(ai): synthesize the session-bus address from XDG_RUNTIME_DIR when set (#2473)                                      | dezren39              | 2026-11-15 |
| [#2466](https://github.com/developing-today-forks/opencode/pull/2466) | fix(ai): claude-code renewal adopts another writer's rotation instead of replaying it (code#9549)                      | dezren39              | 2026-11-15 |
| [#2510](https://github.com/developing-today-forks/opencode/pull/2510) | test(ai): land phase-4 constraint inventory and corpus ratchet (#1684)                                                 | dezren39              | 2026-11-16 |
| [#2531](https://github.com/developing-today-forks/opencode/pull/2531) | fix(ai): antigravity const narrowing for numeric/boolean enum members                                                  | dezren39              | 2026-11-16 |
| [#2538](https://github.com/developing-today-forks/opencode/pull/2538) | fix(ai): wire-identity stamps track latest upstream binaries (#1834, #1876)                                            | dezren39              | 2026-11-16 |
| [#2568](https://github.com/developing-today-forks/opencode/pull/2568) | fix(ai): widen claude-code renewal lead to half the token lifetime (code#9549)                                         | dezren39              | 2026-11-17 |
| [#2574](https://github.com/developing-today-forks/opencode/pull/2574) | test(ai): add DeepSeek tool-call fixtures (deepseek.test.ts)                                                           | dezren39              | 2026-11-17 |
| [#2575](https://github.com/developing-today-forks/opencode/pull/2575) | test(ai): add combined Vibe fixture — reasoningEffort high with a tool call (#2232)                                    | dezren39              | 2026-11-17 |
| [#2576](https://github.com/developing-today-forks/opencode/pull/2576) | test(ai): pin flattenTools namespace collision behavior (#2233)                                                        | dezren39              | 2026-11-17 |
| [#2579](https://github.com/developing-today-forks/opencode/pull/2579) | fix(ai): error explicitly on scalar Mistral tool arguments (#2229)                                                     | dezren39              | 2026-11-17 |
| [#2584](https://github.com/developing-today-forks/opencode/pull/2584) | test(ai): guard secret-service dbus tests on Linux (#2456)                                                             | dezren39              | 2026-11-17 |
| [#2594](https://github.com/developing-today-forks/opencode/pull/2594) | fix(ai): flag-gated bounded repair for truncated tool-call JSON                                                        | dezren39              | 2026-11-17 |
| [#2628](https://github.com/developing-today-forks/opencode/pull/2628) | fix(ai): single-flight ownership follows refresh-token adoption (#2623)                                                | dezren39              | 2026-11-17 |
| [#2661](https://github.com/developing-today-forks/opencode/pull/2661) | fix(ai): mark error tool results on the chat/responses/mistral/antigravity wires                                       | dezren39              | 2026-11-21 |
| [#2702](https://github.com/developing-today-forks/opencode/pull/2702) | fix(ai): preserve failed tool-result status in Cohere history                                                          | dezren39              | 2026-11-23 |
| [#2727](https://github.com/developing-today-forks/opencode/pull/2727) | fix(ai): trim truncated uppercase exponents in partial-json like lowercase                                             | dezren39              | 2026-11-23 |
| [#2747](https://github.com/developing-today-forks/opencode/pull/2747) | fix(ai): make the Mistral tool-id projection session-stable (#2228)                                                    | dezren39              | 2026-11-23 |
| [#2756](https://github.com/developing-today-forks/opencode/pull/2756) | fix(ai): guard new URL parse in isSessionExpiredStatus (#2657)                                                         | dezren39              | 2026-11-23 |
| [#2764](https://github.com/developing-today-forks/opencode/pull/2764) | fix(ai): honor the stated index when streamed tool-call IDs collide                                                    | dezren39              | 2026-11-23 |
| [#2772](https://github.com/developing-today-forks/opencode/pull/2772) | fix(ai): mark Command Code history errors                                                                              | dezren39              | 2026-11-23 |
| [#2776](https://github.com/developing-today-forks/opencode/pull/2776) | fix(ai): preserve failed tool-result status in Gemini history                                                          | dezren39              | 2026-11-23 |

##### developing-today-forks/opencode — theme: ci (125)

| PR                                                                    | Title                                                                                                 | Author                | Merged     |
| --------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- | --------------------- | ---------- |
| [#24](https://github.com/developing-today-forks/opencode/pull/24)     | fix(ci): point pr-standards and compliance-close at main, not the nonexistent dev branch              | dezren39              | 2026-10-17 |
| [#38](https://github.com/developing-today-forks/opencode/pull/38)     | fix(ci): pin jules-action to v1.0.0 — v1 tag does not exist (#36)                                     | dezren39              | 2026-10-18 |
| [#40](https://github.com/developing-today-forks/opencode/pull/40)     | fix(ci): point verification workflows at main, not nonexistent dev                                    | dezren39              | 2026-10-18 |
| [#44](https://github.com/developing-today-forks/opencode/pull/44)     | fix(ci): point close-issues and close-prs housekeeping at this fork, not upstream                     | dezren39              | 2026-10-18 |
| [#50](https://github.com/developing-today-forks/opencode/pull/50)     | ci: add concurrency groups to cancel superseded runs                                                  | dezren39              | 2026-10-18 |
| [#51](https://github.com/developing-today-forks/opencode/pull/51)     | ci: move workflows to GitHub-hosted runners (Blacksmith commented out for switchback)                 | dezren39              | 2026-10-18 |
| [#73](https://github.com/developing-today-forks/opencode/pull/73)     | fix(ci): run e2e for PRs whose head branch is named main                                              | dezren39              | 2026-10-18 |
| [#86](https://github.com/developing-today-forks/opencode/pull/86)     | fix(ci): key issue/issue_comment concurrency groups by target, queue all runs                         | dezren39              | 2026-10-18 |
| [#106](https://github.com/developing-today-forks/opencode/pull/106)   | fix(ci): use the filewatcher env name the server reads                                                | dezren39              | 2026-10-19 |
| [#119](https://github.com/developing-today-forks/opencode/pull/119)   | ci: declare least-privilege permissions in check, storybook and notify-discord workflows              | dezren39              | 2026-10-19 |
| [#121](https://github.com/developing-today-forks/opencode/pull/121)   | ci: add timeout-minutes caps to every workflow job                                                    | dezren39              | 2026-10-19 |
| [#130](https://github.com/developing-today-forks/opencode/pull/130)   | fix(ci): queue Jules upstream-sync dispatches with queue: max (code#359 set)                          | dezren39              | 2026-10-19 |
| [#148](https://github.com/developing-today-forks/opencode/pull/148)   | fix(ci): queue close-prs dispatches with queue: max                                                   | dezren39              | 2026-10-20 |
| [#150](https://github.com/developing-today-forks/opencode/pull/150)   | ci: pin Bun from package.json in close-issues and close-prs workflows                                 | dezren39              | 2026-10-20 |
| [#152](https://github.com/developing-today-forks/opencode/pull/152)   | ci: queue every close-issues and compliance-close run (#151)                                          | dezren39              | 2026-10-20 |
| [#157](https://github.com/developing-today-forks/opencode/pull/157)   | ci: add queue: max to models-snapshot, docs-update and stats workflows                                | dezren39              | 2026-10-20 |
| [#161](https://github.com/developing-today-forks/opencode/pull/161)   | ci: retire actions/checkout v3.6.0 in deploy/publish workflows                                        | dezren39              | 2026-10-20 |
| [#165](https://github.com/developing-today-forks/opencode/pull/165)   | ci: pin actions/github-script at v8.0.0 everywhere (#164)                                             | dezren39              | 2026-10-20 |
| [#170](https://github.com/developing-today-forks/opencode/pull/170)   | ci: align actions/checkout on the v4.3.1 standard pin                                                 | dezren39              | 2026-10-20 |
| [#180](https://github.com/developing-today-forks/opencode/pull/180)   | ci: add queue: max to the six deploy workflows                                                        | dezren39              | 2026-10-20 |
| [#185](https://github.com/developing-today-forks/opencode/pull/185)   | ci: pin jules-action to commit SHA (v1.0.0)                                                           | dezren39              | 2026-10-20 |
| [#183](https://github.com/developing-today-forks/opencode/pull/183)   | fix(ci): deduplicate turbo.json test tasks and guard against repeats                                  | dezren39              | 2026-10-20 |
| [#198](https://github.com/developing-today-forks/opencode/pull/198)   | fix(ci): configure Dependabot for bun and land the vite bump with a regenerated lockfile              | dezren39              | 2026-10-20 |
| [#207](https://github.com/developing-today-forks/opencode/pull/207)   | chore(ci): group @modelcontextprotocol/* Dependabot updates into one PR                               | dezren39              | 2026-10-21 |
| [#211](https://github.com/developing-today-forks/opencode/pull/211)   | chore(ci): group MCP security updates into one Dependabot PR                                          | dezren39              | 2026-10-21 |
| [#225](https://github.com/developing-today-forks/opencode/pull/225)   | fix(ci): gate docs-update to anomalyco/opencode, drop dead fork cron and stale sst action path        | dezren39              | 2026-10-21 |
| [#226](https://github.com/developing-today-forks/opencode/pull/226)   | fix(ci): build containers on main without pushing; GHCR push becomes a dispatch input (#172)          | dezren39              | 2026-10-21 |
| [#229](https://github.com/developing-today-forks/opencode/pull/229)   | fix(ci): make release-github-action a deliberate manual dispatch (#197)                               | dezren39              | 2026-10-21 |
| [#252](https://github.com/developing-today-forks/opencode/pull/252)   | fix(ci): run models-snapshot refresh on this fork against main                                        | dezren39              | 2026-10-21 |
| [#608](https://github.com/developing-today-forks/opencode/pull/608)   | ci: add stale-branch cleanup workflow (dry-run by default)                                            | dezren39              | 2026-10-27 |
| [#626](https://github.com/developing-today-forks/opencode/pull/626)   | ci: run branch cleanup daily                                                                          | dezren39              | 2026-10-27 |
| [#671](https://github.com/developing-today-forks/opencode/pull/671)   | fix(ci): protect open-PR bases in branch-cleanup (openchamber#629)                                    | dezren39              | 2026-10-28 |
| [#731](https://github.com/developing-today-forks/opencode/pull/731)   | feat(ci): record upstream-sync failures on a tracking issue for Muse tasks                            | dezren39              | 2026-10-28 |
| [#754](https://github.com/developing-today-forks/opencode/pull/754)   | fix(ci): close upstream-sync tracking issue on success (#742)                                         | dezren39              | 2026-10-28 |
| [#761](https://github.com/developing-today-forks/opencode/pull/761)   | fix(ci): distinguish a turbo crash from unit test failures                                            | dezren39              | 2026-10-28 |
| [#827](https://github.com/developing-today-forks/opencode/pull/827)   | ci: fast PR tier, full main tier — cancel-in-progress + PR slimming (Phases 1+2)                      | dezren39              | 2026-10-29 |
| [#828](https://github.com/developing-today-forks/opencode/pull/828)   | ci: main-broken watcher — Phase 3 filing leg (Jules leg blocked on missing JULES_API_KEY)             | dezren39              | 2026-10-29 |
| [#832](https://github.com/developing-today-forks/opencode/pull/832)   | ci: basic tier named as the merge requirement + event bots consolidated into repo-sweeper (every 15m) | dezren39              | 2026-10-29 |
| [#834](https://github.com/developing-today-forks/opencode/pull/834)   | ci: repo-sweeper daily closers are schedule-only (dispatch can never close)                           | dezren39              | 2026-10-29 |
| [#897](https://github.com/developing-today-forks/opencode/pull/897)   | fix(ci): refresh variant.ts proto-reads allowlist entry for #878 guard                                | dezren39              | 2026-10-29 |
| [#932](https://github.com/developing-today-forks/opencode/pull/932)   | fix(ci): scan every English docs page in check:doc-env-vars                                           | dezren39              | 2026-10-30 |
| [#963](https://github.com/developing-today-forks/opencode/pull/963)   | ci: main-only workflow triggers, cancel-in-progress everywhere                                        | dezren39              | 2026-10-30 |
| [#976](https://github.com/developing-today-forks/opencode/pull/976)   | fix(ci): restore cancel-in-progress: false on the seven queue: max workflows                          | dezren39              | 2026-10-30 |
| [#979](https://github.com/developing-today-forks/opencode/pull/979)   | ci: add dispatch agents workflow + ci-fix hook                                                        | dezren39              | 2026-10-30 |
| [#1044](https://github.com/developing-today-forks/opencode/pull/1044) | fix(ci): verify main-broken runs belong to this repo before filing                                    | dezren39              | 2026-10-31 |
| [#1046](https://github.com/developing-today-forks/opencode/pull/1046) | fix(ci): serialize main-broken filing with race-loser reconcile                                       | dezren39              | 2026-10-31 |
| [#1045](https://github.com/developing-today-forks/opencode/pull/1045) | fix(ci): serialize release-github-action manual dispatches                                            | dezren39              | 2026-10-31 |
| [#1092](https://github.com/developing-today-forks/opencode/pull/1092) | fix(ci): close check-proto-reads computed-key and duplicate-line gaps                                 | dezren39              | 2026-10-31 |
| [#1117](https://github.com/developing-today-forks/opencode/pull/1117) | fix(ci): branch-cleanup — one failed delete must not abort the run (code#1615 set)                    | dezren39              | 2026-11-01 |
| [#1164](https://github.com/developing-today-forks/opencode/pull/1164) | ci: restrict branch-deploy workflows to main                                                          | dezren39              | 2026-11-01 |
| [#1185](https://github.com/developing-today-forks/opencode/pull/1185) | fix(ci): add timeout-minutes to branch-cleanup job (#1184)                                            | dezren39              | 2026-11-01 |
| [#1189](https://github.com/developing-today-forks/opencode/pull/1189) | fix(ci): refresh bounded-http allowlist for post-#1161 api.ts (#1188)                                 | dezren39              | 2026-11-01 |
| [#1274](https://github.com/developing-today-forks/opencode/pull/1274) | fix(ci): scan services/ in the bounded-http gate                                                      | dezren39              | 2026-11-01 |
| [#1287](https://github.com/developing-today-forks/opencode/pull/1287) | fix(ci): disposition #1277 model-route reads in proto-reads allowlist (#1282)                         | dezren39              | 2026-11-01 |
| [#1288](https://github.com/developing-today-forks/opencode/pull/1288) | fix(ci): refresh install.astro bounded-http allowlist entry for the #1246 fix (#1283)                 | dezren39              | 2026-11-01 |
| [#1313](https://github.com/developing-today-forks/opencode/pull/1313) | fix(ci): scan services workers in numeric-boundaries gate                                             | dezren39              | 2026-11-01 |
| [#1321](https://github.com/developing-today-forks/opencode/pull/1321) | fix(ci): scan plugin-github in numeric-boundaries gate                                                | dezren39              | 2026-11-01 |
| [#1315](https://github.com/developing-today-forks/opencode/pull/1315) | fix(ci): preserve inherited run links in main-broken reconcile (#1314)                                | dezren39              | 2026-11-01 |
| [#1327](https://github.com/developing-today-forks/opencode/pull/1327) | fix(ci): cap test.yml ci-fix job at 10 minutes (#1326)                                                | dezren39              | 2026-11-01 |
| [#1372](https://github.com/developing-today-forks/opencode/pull/1372) | fix(ci): extend numeric-boundary scan to production trees                                             | dezren39              | 2026-11-02 |
| [#1397](https://github.com/developing-today-forks/opencode/pull/1397) | fix(ci): scan the script trees in check:bounded-http (#1394)                                          | dezren39              | 2026-11-02 |
| [#1400](https://github.com/developing-today-forks/opencode/pull/1400) | fix(ci): disposition desktop extension assets test read in bounded-http allowlist                     | dezren39              | 2026-11-02 |
| [#1401](https://github.com/developing-today-forks/opencode/pull/1401) | ci: add workflow_dispatch where missing (no-PR-trigger policy)                                        | dezren39              | 2026-11-02 |
| [#1415](https://github.com/developing-today-forks/opencode/pull/1415) | fix(ci): add check:workflows gate enforcing the workflow policies (#1406)                             | dezren39              | 2026-11-02 |
| [#1444](https://github.com/developing-today-forks/opencode/pull/1444) | fix(ci): scan nested script trees in check:bounded-http                                               | dezren39              | 2026-11-02 |
| [#1451](https://github.com/developing-today-forks/opencode/pull/1451) | fix(ci): gate publish workflows on a green main-gate for the release SHA                              | dezren39              | 2026-11-02 |
| [#1480](https://github.com/developing-today-forks/opencode/pull/1480) | ci: add timeout-minutes to jobs that lacked one                                                       | dezren39              | 2026-11-03 |
| [#1482](https://github.com/developing-today-forks/opencode/pull/1482) | fix(ci): validate reusable workflow caller keys                                                       | dezren39              | 2026-11-03 |
| [#1490](https://github.com/developing-today-forks/opencode/pull/1490) | fix(ci): install dependencies before models snapshot refresh                                          | dezren39              | 2026-11-03 |
| [#1501](https://github.com/developing-today-forks/opencode/pull/1501) | fix(ci): wire protocol check:generated into test workflow                                             | dezren39              | 2026-11-03 |
| [#919](https://github.com/developing-today-forks/opencode/pull/919)   | fix(ci): scan packages/ai in the check:proto-reads gate                                               | dezren39              | 2026-11-03 |
| [#1575](https://github.com/developing-today-forks/opencode/pull/1575) | fix(ci): add src-tree drift guards to bounded-http and numeric-boundaries gates                       | dezren39              | 2026-11-04 |
| [#1577](https://github.com/developing-today-forks/opencode/pull/1577) | fix(ci): scan packages/plugin-github in the check:proto-reads gate                                    | dezren39              | 2026-11-04 |
| [#1594](https://github.com/developing-today-forks/opencode/pull/1594) | feat(ci): add a permissions policy to check:workflows (#1503)                                         | dezren39              | 2026-11-04 |
| [#1586](https://github.com/developing-today-forks/opencode/pull/1586) | refactor(ci): one shared harness for the three allowlist gates (#1526)                                | dezren39              | 2026-11-04 |
| [#1612](https://github.com/developing-today-forks/opencode/pull/1612) | fix(ci): disposition build.ts bounded-http sites after Bun archive rewrite (#1606)                    | dezren39              | 2026-11-04 |
| [#1616](https://github.com/developing-today-forks/opencode/pull/1616) | fix(ci): reconcile the tier policy with the no-PR-trigger policy (#1533)                              | dezren39              | 2026-11-04 |
| [#1613](https://github.com/developing-today-forks/opencode/pull/1613) | chore(ci): disposition salesforce content-length parse in numeric-boundaries allowlist (#1607)        | dezren39              | 2026-11-04 |
| [#1630](https://github.com/developing-today-forks/opencode/pull/1630) | fix(ci): load pm-dispatch locally in agents.yml instead of the remote reusable workflow               | dezren39              | 2026-11-04 |
| [#1635](https://github.com/developing-today-forks/opencode/pull/1635) | ci: remove push/pull_request workflow triggers                                                        | developing-today[bot] | 2026-11-04 |
| [#1637](https://github.com/developing-today-forks/opencode/pull/1637) | fix(ci): re-sync numeric-boundaries allowlist for #1507/#1510 (#1514)                                 | dezren39              | 2026-11-04 |
| [#1644](https://github.com/developing-today-forks/opencode/pull/1644) | fix(ci): run lint:changed on check.yml's actual events (#1508)                                        | dezren39              | 2026-11-04 |
| [#1641](https://github.com/developing-today-forks/opencode/pull/1641) | fix(ci): disposition prompt.ts hex parse in numeric-boundaries gate (#1625)                           | dezren39              | 2026-11-04 |
| [#1650](https://github.com/developing-today-forks/opencode/pull/1650) | fix(ci): revive pr-standards payload-free via a pr_number dispatch input (#1534)                      | dezren39              | 2026-11-04 |
| [#1639](https://github.com/developing-today-forks/opencode/pull/1639) | fix(ci): extend check:proto-reads gate to every production src tree (#1369)                           | dezren39              | 2026-11-04 |
| [#1671](https://github.com/developing-today-forks/opencode/pull/1671) | fix(ci): load pm-dispatch actions from a local .pm-central checkout (#1519)                           | dezren39              | 2026-11-04 |
| [#1670](https://github.com/developing-today-forks/opencode/pull/1670) | feat(ci): gate registry + wiring meta-check (#1527)                                                   | dezren39              | 2026-11-05 |
| [#1676](https://github.com/developing-today-forks/opencode/pull/1676) | fix(ci): load pm-dispatch actions locally                                                             | dezren39              | 2026-11-05 |
| [#1691](https://github.com/developing-today-forks/opencode/pull/1691) | fix(ci): re-sync bounded-http and numeric gate allowlists (#1688)                                     | dezren39              | 2026-11-05 |
| [#1696](https://github.com/developing-today-forks/opencode/pull/1696) | fix(ci): wire packages/app vite.icons.test.ts into the package test script (#1695)                    | dezren39              | 2026-11-05 |
| [#1657](https://github.com/developing-today-forks/opencode/pull/1657) | fix(ci): scan the script trees in check:numeric-boundaries (#1517)                                    | dezren39              | 2026-11-05 |
| [#1701](https://github.com/developing-today-forks/opencode/pull/1701) | fix(ci): wire packages/app timeline-stability fixture.test.ts into the package test script (#1700)    | dezren39              | 2026-11-05 |
| [#1775](https://github.com/developing-today-forks/opencode/pull/1775) | fix(ci): restore the survivor number in the main-broken reconcile closing echo (#1582)                | developing-today[bot] | 2026-11-05 |
| [#1780](https://github.com/developing-today-forks/opencode/pull/1780) | fix(ci): stop nix-hashes self-cancellation, restore push trigger                                      | dezren39              | 2026-11-05 |
| [#1791](https://github.com/developing-today-forks/opencode/pull/1791) | fix(ci): disposition the #1727 sharp-stub build read in the bounded-http allowlist (#1790)            | dezren39              | 2026-11-06 |
| [#1842](https://github.com/developing-today-forks/opencode/pull/1842) | fix(ci): disposition #1828 infra-hold ladder read in check:proto-reads allowlist (#1841)              | dezren39              | 2026-11-06 |
| [#1914](https://github.com/developing-today-forks/opencode/pull/1914) | fix(ci): disposition the quota subcommand gate drift, bound turso + readQuotaJson (#1724)             | dezren39              | 2026-11-07 |
| [#1694](https://github.com/developing-today-forks/opencode/pull/1694) | test(ci): run gate harness in CI                                                                      | dezren39              | 2026-11-08 |
| [#2042](https://github.com/developing-today-forks/opencode/pull/2042) | fix(ci): one token convention for repo-sweeper closers (#1972)                                        | dezren39              | 2026-11-08 |
| [#2097](https://github.com/developing-today-forks/opencode/pull/2097) | fix(ci): gate drift guard walks the whole repo tree                                                   | dezren39              | 2026-11-08 |
| [#2105](https://github.com/developing-today-forks/opencode/pull/2105) | fix(ci): validate job-level permissions content in check:workflows (#1759)                            | developing-today[bot] | 2026-11-08 |
| [#2148](https://github.com/developing-today-forks/opencode/pull/2148) | fix(ci): retry Bun setup once after a transient download failure (#2112)                              | dezren39              | 2026-11-08 |
| [#2162](https://github.com/developing-today-forks/opencode/pull/2162) | fix(ci): collect stale issues before closing in close-issues.ts                                       | dezren39              | 2026-11-08 |
| [#2191](https://github.com/developing-today-forks/opencode/pull/2191) | docs(ci): reconcile ci-policy.md to dispatch/schedule-only triggers                                   | developing-today[bot] | 2026-11-09 |
| [#2197](https://github.com/developing-today-forks/opencode/pull/2197) | fix(ci): recover nix-hashes when the darwin runner never starts                                       | developing-today[bot] | 2026-11-09 |
| [#2198](https://github.com/developing-today-forks/opencode/pull/2198) | fix(ci): quote aarch64-darwin in nix-hashes continue-on-error                                         | developing-today[bot] | 2026-11-09 |
| [#2200](https://github.com/developing-today-forks/opencode/pull/2200) | fix(ci): nix-hashes darwin fallback on macos-15 (macos-14 retired)                                    | developing-today[bot] | 2026-11-09 |
| [#2242](https://github.com/developing-today-forks/opencode/pull/2242) | fix(ci): run the repo-sweeper token-contract guard in the test workflow (#1972)                       | dezren39              | 2026-11-10 |
| [#2241](https://github.com/developing-today-forks/opencode/pull/2241) | fix(ci): run native-arch tests in the test workflow (#2222)                                           | dezren39              | 2026-11-10 |
| [#2323](https://github.com/developing-today-forks/opencode/pull/2323) | docs(ci): backstop guard for the nightly test workflow (#2322)                                        | dezren39              | 2026-11-12 |
| [#2327](https://github.com/developing-today-forks/opencode/pull/2327) | fix(ci): repair test workflow failures                                                                | dezren39              | 2026-11-13 |
| [#2398](https://github.com/developing-today-forks/opencode/pull/2398) | Fix main CI: guard proto-reads hazards + platform-accurate writeAtomic restore test (#2392)           | dezren39              | 2026-11-13 |
| [#2406](https://github.com/developing-today-forks/opencode/pull/2406) | fix(ci): disposition numeric-boundaries conversions + clear session-ui unit failures (#2405)          | dezren39              | 2026-11-13 |
| [#2471](https://github.com/developing-today-forks/opencode/pull/2471) | fix: resolve main test CI failures (proto-reads, Windows @opencode/ai)                                | dezren39              | 2026-11-16 |
| [#2540](https://github.com/developing-today-forks/opencode/pull/2540) | fix(ci): engage darwin hash fallback when compute leg is cancelled                                    | dezren39              | 2026-11-16 |
| [#2545](https://github.com/developing-today-forks/opencode/pull/2545) | fix(ci): narrow darwin fallback gate so green runs don't depend on a no-op macos job                  | dezren39              | 2026-11-16 |
| [#2549](https://github.com/developing-today-forks/opencode/pull/2549) | fix(ci): engage darwin hash fallback whenever compute-hash ran (#2547)                                | dezren39              | 2026-11-16 |
| [#2552](https://github.com/developing-today-forks/opencode/pull/2552) | fix(ci): decide darwin fallback on ubuntu before allocating macos                                     | dezren39              | 2026-11-16 |
| [#2565](https://github.com/developing-today-forks/opencode/pull/2565) | fix(ci): ungate nix-hashes fallback Upload hash step (#2560)                                          | dezren39              | 2026-11-17 |
| [#2600](https://github.com/developing-today-forks/opencode/pull/2600) | docs(ci): extend the backstop guard to check, nix-eval, and storybook                                 | dezren39              | 2026-11-17 |
| [#2602](https://github.com/developing-today-forks/opencode/pull/2602) | ci: replicate auto-format workflow from code PoC (#2453)                                              | dezren39              | 2026-11-17 |
| [#2627](https://github.com/developing-today-forks/opencode/pull/2627) | fix(ci): resolve upload paths before containment and drop the PersistentPty import alias              | dezren39              | 2026-11-17 |
| [#2703](https://github.com/developing-today-forks/opencode/pull/2703) | cleanup(ci): remove orphaned .github/actions/main-gate                                                | dezren39              | 2026-11-23 |
| [#2731](https://github.com/developing-today-forks/opencode/pull/2731) | fix(ci): formatter-stable gate keys, auto-format gate guard, disposition current drift (#2630)        | dezren39              | 2026-11-23 |
| [#2608](https://github.com/developing-today-forks/opencode/pull/2608) | fix(ci): harden review workflow against untrusted PR title/body (#2478)                               | dezren39              | 2026-11-23 |

##### developing-today-forks/opencode — theme: cli (49)

| PR                                                                    | Title                                                                                                           | Author                | Merged     |
| --------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- | --------------------- | ---------- |
| [#35](https://github.com/developing-today-forks/opencode/pull/35)     | fix(cli): retry explicit --server probes during a handoff                                                       | dezren39              | 2026-10-18 |
| [#71](https://github.com/developing-today-forks/opencode/pull/71)     | fix(cli): report incompatible health when --server probe exhausts on 503                                        | dezren39              | 2026-10-18 |
| [#95](https://github.com/developing-today-forks/opencode/pull/95)     | test(cli): load systemd-socket fixture's libc lazily so the file loads on Windows                               | dezren39              | 2026-10-19 |
| [#101](https://github.com/developing-today-forks/opencode/pull/101)   | fix(cli): Node build smoke cleanup retries and warns on Windows EBUSY                                           | dezren39              | 2026-10-19 |
| [#154](https://github.com/developing-today-forks/opencode/pull/154)   | test(cli): cover ACP follow's stale-attach guard                                                                | dezren39              | 2026-10-20 |
| [#219](https://github.com/developing-today-forks/opencode/pull/219)   | docs(cli): document opencode run stdin EOF behaviour and workaround                                             | dezren39              | 2026-10-21 |
| [#242](https://github.com/developing-today-forks/opencode/pull/242)   | fix(cli): validate OC_PORT in restart-driver, fall back to 4096                                                 | dezren39              | 2026-10-21 |
| [#248](https://github.com/developing-today-forks/opencode/pull/248)   | test(cli): pin restart-driver OC_PORT validation with a pure parse helper                                       | dezren39              | 2026-10-21 |
| [#312](https://github.com/developing-today-forks/opencode/pull/312)   | fix(cli): honor OPENCODE_DISABLE_EMBEDDED_WEB_UI in the web UI handler                                          | dezren39              | 2026-10-23 |
| [#441](https://github.com/developing-today-forks/opencode/pull/441)   | fix(cli): read import files with node fs instead of Bun.file                                                    | dezren39              | 2026-10-25 |
| [#448](https://github.com/developing-today-forks/opencode/pull/448)   | fix(cli): remove Bun global from ACP and CPU-profile paths                                                      | dezren39              | 2026-10-25 |
| [#767](https://github.com/developing-today-forks/opencode/pull/767)   | fix(cli): log stdin reads before EOF                                                                            | dezren39              | 2026-10-28 |
| [#876](https://github.com/developing-today-forks/opencode/pull/876)   | fix(cli): accept only plain decimal spellings for service set port                                              | dezren39              | 2026-10-29 |
| [#938](https://github.com/developing-today-forks/opencode/pull/938)   | test(cli): cover the mini half of the stdin-read log                                                            | dezren39              | 2026-10-30 |
| [#978](https://github.com/developing-today-forks/opencode/pull/978)   | fix(cli): write mcp add config atomically with owner-only permissions                                           | dezren39              | 2026-10-30 |
| [#990](https://github.com/developing-today-forks/opencode/pull/990)   | fix(cli): rewrite shell config atomically on uninstall                                                          | dezren39              | 2026-10-30 |
| [#1032](https://github.com/developing-today-forks/opencode/pull/1032) | fix(cli): bound updater release-metadata response body                                                          | dezren39              | 2026-10-31 |
| [#1057](https://github.com/developing-today-forks/opencode/pull/1057) | fix(cli): harden plugin config writers for symlinks, concurrency, permissions                                   | dezren39              | 2026-10-31 |
| [#1070](https://github.com/developing-today-forks/opencode/pull/1070) | fix(cli): harden service-config and cli-config writers — symlink, unique temp, mode preservation (#1067)        | dezren39              | 2026-10-31 |
| [#1074](https://github.com/developing-today-forks/opencode/pull/1074) | fix(cli): preserve symlink and mode in config migration writer                                                  | dezren39              | 2026-10-31 |
| [#944](https://github.com/developing-today-forks/opencode/pull/944)   | fix(cli): bound session import remote reads at 256 MiB                                                          | dezren39              | 2026-11-01 |
| [#1121](https://github.com/developing-today-forks/opencode/pull/1121) | fix(cli): bound session-import remote fetch — 10-min deadline threaded into the bounded reader                  | dezren39              | 2026-11-01 |
| [#1161](https://github.com/developing-today-forks/opencode/pull/1161) | fix(cli): add named timeouts to opencode api fetches (#1144)                                                    | dezren39              | 2026-11-01 |
| [#1178](https://github.com/developing-today-forks/opencode/pull/1178) | fix(cli): bound session import server requests (#1167)                                                          | dezren39              | 2026-11-01 |
| [#1278](https://github.com/developing-today-forks/opencode/pull/1278) | fix(cli): add a download deadline to build-node fetches (#1276)                                                 | dezren39              | 2026-11-01 |
| [#1296](https://github.com/developing-today-forks/opencode/pull/1296) | fix(cli): create mcp add config temp file exclusively (wx)                                                      | dezren39              | 2026-11-01 |
| [#1299](https://github.com/developing-today-forks/opencode/pull/1299) | fix(cli): preserve rc file ownership in writeShellConfig                                                        | dezren39              | 2026-11-01 |
| [#1298](https://github.com/developing-today-forks/opencode/pull/1298) | fix(cli): create plugin config temp files exclusively (wx)                                                      | dezren39              | 2026-11-01 |
| [#1368](https://github.com/developing-today-forks/opencode/pull/1368) | refactor(cli): migrate config writers to atomic writes                                                          | dezren39              | 2026-11-02 |
| [#1399](https://github.com/developing-today-forks/opencode/pull/1399) | fix(cli): bound session import's local-file read at the transfer cap (#1398)                                    | dezren39              | 2026-11-02 |
| [#1391](https://github.com/developing-today-forks/opencode/pull/1391) | fix(cli): bound non-streaming run requests                                                                      | dezren39              | 2026-11-02 |
| [#1407](https://github.com/developing-today-forks/opencode/pull/1407) | fix(cli): bound auth import file and stdin reads at 10 MiB                                                      | dezren39              | 2026-11-02 |
| [#1460](https://github.com/developing-today-forks/opencode/pull/1460) | fix(cli): never hang run on an open, never-written stdin pipe                                                   | dezren39              | 2026-11-03 |
| [#1467](https://github.com/developing-today-forks/opencode/pull/1467) | fix(cli): match run fetch long-lived exemptions under path prefixes                                             | dezren39              | 2026-11-03 |
| [#1515](https://github.com/developing-today-forks/opencode/pull/1515) | test(cli): give the upgrade detection-failure test a 60s budget (#895)                                          | dezren39              | 2026-11-03 |
| [#989](https://github.com/developing-today-forks/opencode/pull/989)   | fix(cli): stream opencode api response body instead of buffering it whole                                       | dezren39              | 2026-11-03 |
| [#1564](https://github.com/developing-today-forks/opencode/pull/1564) | fix(cli): verify Bun compiler archive before use                                                                | dezren39              | 2026-11-03 |
| [#1617](https://github.com/developing-today-forks/opencode/pull/1617) | fix(cli): stub onnxruntime-node and sharp out of the Node bundle (#1615)                                        | dezren39              | 2026-11-04 |
| [#1638](https://github.com/developing-today-forks/opencode/pull/1638) | fix(cli): pass /v1 facade routes through the web UI transform (#1627)                                           | dezren39              | 2026-11-04 |
| [#1735](https://github.com/developing-today-forks/opencode/pull/1735) | fix(cli): quota unknown provider name no longer discards valid results                                          | dezren39              | 2026-11-05 |
| [#1736](https://github.com/developing-today-forks/opencode/pull/1736) | fix(cli): quota config facts walk ancestors and read the system directory                                       | dezren39              | 2026-11-05 |
| [#1789](https://github.com/developing-today-forks/opencode/pull/1789) | fix(cli): assert each sharp-stub build rewrite independently (#1788)                                            | dezren39              | 2026-11-05 |
| [#1798](https://github.com/developing-today-forks/opencode/pull/1798) | fix(cli): round quota percents in human-readable rendering (#1797)                                              | developing-today[bot] | 2026-11-06 |
| [#1732](https://github.com/developing-today-forks/opencode/pull/1732) | docs(cli): document quota command                                                                               | dezren39              | 2026-11-07 |
| [#822](https://github.com/developing-today-forks/opencode/pull/822)   | test(cli): add standalone Meta serve smoke                                                                      | dezren39              | 2026-11-08 |
| [#2258](https://github.com/developing-today-forks/opencode/pull/2258) | fix(cli): service-mode start takes over a different-version incumbent instead of silently deferring (code#8255) | dezren39              | 2026-11-10 |
| [#2401](https://github.com/developing-today-forks/opencode/pull/2401) | fix(cli): ACP session follow logs and re-sends state after a failed sessionUpdate                               | dezren39              | 2026-11-13 |
| [#2451](https://github.com/developing-today-forks/opencode/pull/2451) | fix(cli): wire Node SEA keyring addon for darwin/win32 targets (#2413)                                          | dezren39              | 2026-11-14 |
| [#2485](https://github.com/developing-today-forks/opencode/pull/2485) | fix(cli): re-point meta-serve-smoke Meta leg at the _muse door (#431)                                           | dezren39              | 2026-11-16 |

##### developing-today-forks/opencode — theme: docs (47)

| PR                                                                    | Title                                                                                | Author   | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------------------ | -------- | ---------- |
| [#83](https://github.com/developing-today-forks/opencode/pull/83)     | docs: add roadmap, planning folder and templates                                     | dezren39 | 2026-10-18 |
| [#112](https://github.com/developing-today-forks/opencode/pull/112)   | docs: move fixed #77, #54 and #74 ROADMAP rows to Done                               | dezren39 | 2026-10-19 |
| [#147](https://github.com/developing-today-forks/opencode/pull/147)   | docs: move fixed #67/#72 ROADMAP rows to Done, drop premise-false #59, add #127/#134 | dezren39 | 2026-10-20 |
| [#163](https://github.com/developing-today-forks/opencode/pull/163)   | docs: ROADMAP — move #62 to Done, add 2026-10-20 merges, Now points at #159          | dezren39 | 2026-10-20 |
| [#169](https://github.com/developing-today-forks/opencode/pull/169)   | docs: record the #159, #63 and #164 completions in ROADMAP                           | dezren39 | 2026-10-20 |
| [#177](https://github.com/developing-today-forks/opencode/pull/177)   | docs: record the #173, #141 and #153 completions in ROADMAP Done                     | dezren39 | 2026-10-20 |
| [#182](https://github.com/developing-today-forks/opencode/pull/182)   | docs: record the #143, #160 and #179 completions in ROADMAP Done                     | dezren39 | 2026-10-20 |
| [#188](https://github.com/developing-today-forks/opencode/pull/188)   | docs: record the #145 and #184 completions in ROADMAP Done                           | dezren39 | 2026-10-20 |
| [#203](https://github.com/developing-today-forks/opencode/pull/203)   | docs: record the #178 and #175 completions in ROADMAP Done                           | dezren39 | 2026-10-21 |
| [#209](https://github.com/developing-today-forks/opencode/pull/209)   | docs: record the #196 and #199 completions in ROADMAP Done                           | dezren39 | 2026-10-21 |
| [#213](https://github.com/developing-today-forks/opencode/pull/213)   | docs: record the #201 and #210 completions in ROADMAP Done                           | dezren39 | 2026-10-21 |
| [#216](https://github.com/developing-today-forks/opencode/pull/216)   | docs: record the #212 completion in ROADMAP Done                                     | dezren39 | 2026-10-21 |
| [#218](https://github.com/developing-today-forks/opencode/pull/218)   | docs: record the #204 completion in ROADMAP Done                                     | dezren39 | 2026-10-21 |
| [#222](https://github.com/developing-today-forks/opencode/pull/222)   | docs: record the #193 docs workaround in ROADMAP Done                                | dezren39 | 2026-10-21 |
| [#224](https://github.com/developing-today-forks/opencode/pull/224)   | docs: record the #129 completion in ROADMAP Done                                     | dezren39 | 2026-10-21 |
| [#228](https://github.com/developing-today-forks/opencode/pull/228)   | docs: record the #171 and #172 completions in ROADMAP Done                           | dezren39 | 2026-10-21 |
| [#249](https://github.com/developing-today-forks/opencode/pull/249)   | docs: record the #197 and #232 completions in ROADMAP Done (#235)                    | dezren39 | 2026-10-21 |
| [#251](https://github.com/developing-today-forks/opencode/pull/251)   | docs: record the #241, #243, #246 and #235 completions in ROADMAP Done               | dezren39 | 2026-10-21 |
| [#313](https://github.com/developing-today-forks/opencode/pull/313)   | docs: stop promising OPENCODE_SERVER_USERNAME — the v2 username is always opencode   | dezren39 | 2026-10-23 |
| [#336](https://github.com/developing-today-forks/opencode/pull/336)   | docs: remove OPENCODE_PORT passage from troubleshooting guide                        | dezren39 | 2026-10-23 |
| [#349](https://github.com/developing-today-forks/opencode/pull/349)   | docs: stop promising dead web-search, shell-timeout and LSP env flags (#105)         | dezren39 | 2026-10-23 |
| [#354](https://github.com/developing-today-forks/opencode/pull/354)   | docs: stop promising eleven dead OPENCODE_* env vars (#114)                          | dezren39 | 2026-10-23 |
| [#357](https://github.com/developing-today-forks/opencode/pull/357)   | docs: stop promising twelve dead OPENCODE_EXPERIMENTAL_* variables (#116)            | dezren39 | 2026-10-24 |
| [#396](https://github.com/developing-today-forks/opencode/pull/396)   | docs: record the #392 completion in ROADMAP Done                                     | dezren39 | 2026-10-24 |
| [#400](https://github.com/developing-today-forks/opencode/pull/400)   | docs: record the #395 completion in ROADMAP Done (#397)                              | dezren39 | 2026-10-24 |
| [#405](https://github.com/developing-today-forks/opencode/pull/405)   | docs: record the #397, #389 and #398/#399 completions in ROADMAP Done                | dezren39 | 2026-10-24 |
| [#409](https://github.com/developing-today-forks/opencode/pull/409)   | docs: record the #404 completion in ROADMAP Done                                     | dezren39 | 2026-10-25 |
| [#417](https://github.com/developing-today-forks/opencode/pull/417)   | docs: record the #408, #406/#407 and #411 completions in ROADMAP Done                | dezren39 | 2026-10-25 |
| [#419](https://github.com/developing-today-forks/opencode/pull/419)   | docs: record the #414/#412 and #416 completions in ROADMAP Done                      | dezren39 | 2026-10-25 |
| [#421](https://github.com/developing-today-forks/opencode/pull/421)   | docs: record the #418 completion in ROADMAP Done                                     | dezren39 | 2026-10-25 |
| [#424](https://github.com/developing-today-forks/opencode/pull/424)   | docs: record the #420 completion in ROADMAP Done                                     | dezren39 | 2026-10-25 |
| [#427](https://github.com/developing-today-forks/opencode/pull/427)   | docs: record the #423 and #422 completions in ROADMAP Done                           | dezren39 | 2026-10-25 |
| [#430](https://github.com/developing-today-forks/opencode/pull/430)   | docs: record the #426 completion in ROADMAP Done                                     | dezren39 | 2026-10-25 |
| [#433](https://github.com/developing-today-forks/opencode/pull/433)   | docs: record the #429 completion in ROADMAP Done                                     | dezren39 | 2026-10-25 |
| [#439](https://github.com/developing-today-forks/opencode/pull/439)   | docs: record the #432 completion in ROADMAP Done                                     | dezren39 | 2026-10-25 |
| [#446](https://github.com/developing-today-forks/opencode/pull/446)   | docs: record the #435, #436/#437 and #438 completions in ROADMAP Done                | dezren39 | 2026-10-25 |
| [#455](https://github.com/developing-today-forks/opencode/pull/455)   | docs: record the #442, #447 and #450 completions in ROADMAP Done                     | dezren39 | 2026-10-25 |
| [#458](https://github.com/developing-today-forks/opencode/pull/458)   | docs: record the #443 and #452 completions in ROADMAP Done                           | dezren39 | 2026-10-25 |
| [#461](https://github.com/developing-today-forks/opencode/pull/461)   | docs: record the #456 completion in ROADMAP Done                                     | dezren39 | 2026-10-25 |
| [#464](https://github.com/developing-today-forks/opencode/pull/464)   | docs: record the #434 completion in ROADMAP Done                                     | dezren39 | 2026-10-25 |
| [#485](https://github.com/developing-today-forks/opencode/pull/485)   | docs: record the #468, #474, #482 and #484 completions in ROADMAP Done               | dezren39 | 2026-10-26 |
| [#492](https://github.com/developing-today-forks/opencode/pull/492)   | docs: record the #478 and #479 completions in ROADMAP Done                           | dezren39 | 2026-10-26 |
| [#496](https://github.com/developing-today-forks/opencode/pull/496)   | docs: record the #489 completion in ROADMAP Done                                     | dezren39 | 2026-10-26 |
| [#473](https://github.com/developing-today-forks/opencode/pull/473)   | docs: record the #465/#467 completion in ROADMAP Done                                | dezren39 | 2026-10-26 |
| [#500](https://github.com/developing-today-forks/opencode/pull/500)   | docs: record the #480 set and #497/#498 completions in ROADMAP Done                  | dezren39 | 2026-10-26 |
| [#521](https://github.com/developing-today-forks/opencode/pull/521)   | docs: remove ROADMAP Done tables — GitHub is the record                              | dezren39 | 2026-10-26 |
| [#2034](https://github.com/developing-today-forks/opencode/pull/2034) | docs: design image and video generation behind the facade (#1679)                    | dezren39 | 2026-11-09 |

##### developing-today-forks/opencode — theme: roadmap (47)

| PR                                                                    | Title                                                                    | Author   | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------ | -------- | ---------- |
| [#245](https://github.com/developing-today-forks/opencode/pull/245)   | docs(roadmap): record the #231 completion in Done                        | dezren39 | 2026-10-21 |
| [#254](https://github.com/developing-today-forks/opencode/pull/254)   | docs(roadmap): record the #244 and #250 completions in Done              | dezren39 | 2026-10-21 |
| [#260](https://github.com/developing-today-forks/opencode/pull/260)   | docs(roadmap): record #41 closure and #109 completion in Done (#259)     | dezren39 | 2026-10-22 |
| [#263](https://github.com/developing-today-forks/opencode/pull/263)   | docs(roadmap): record the #257 and #259 completions in Done (#262)       | dezren39 | 2026-10-22 |
| [#268](https://github.com/developing-today-forks/opencode/pull/268)   | docs(roadmap): record the #264 completion in Done                        | dezren39 | 2026-10-22 |
| [#274](https://github.com/developing-today-forks/opencode/pull/274)   | docs(roadmap): record the #265 and #269 completions in Done              | dezren39 | 2026-10-22 |
| [#282](https://github.com/developing-today-forks/opencode/pull/282)   | docs(roadmap): record the #272, #273, #276 and #277 completions in Done  | dezren39 | 2026-10-22 |
| [#285](https://github.com/developing-today-forks/opencode/pull/285)   | docs(roadmap): record the #281 completion in Done                        | dezren39 | 2026-10-22 |
| [#289](https://github.com/developing-today-forks/opencode/pull/289)   | docs(roadmap): record the #280 and #284 completions in Done              | dezren39 | 2026-10-22 |
| [#292](https://github.com/developing-today-forks/opencode/pull/292)   | docs(roadmap): record the #286 and #288 completions in Done              | dezren39 | 2026-10-22 |
| [#296](https://github.com/developing-today-forks/opencode/pull/296)   | docs(roadmap): record the #290 completion in Done                        | dezren39 | 2026-10-22 |
| [#298](https://github.com/developing-today-forks/opencode/pull/298)   | docs(roadmap): record the #295 completion in Done                        | dezren39 | 2026-10-22 |
| [#300](https://github.com/developing-today-forks/opencode/pull/300)   | docs(roadmap): record the #297 completion in Done                        | dezren39 | 2026-10-22 |
| [#303](https://github.com/developing-today-forks/opencode/pull/303)   | docs(roadmap): record the #299 and #294 completions in Done              | dezren39 | 2026-10-22 |
| [#305](https://github.com/developing-today-forks/opencode/pull/305)   | docs(roadmap): record the #302 completion in Done (#304)                 | dezren39 | 2026-10-23 |
| [#307](https://github.com/developing-today-forks/opencode/pull/307)   | docs(roadmap): record the #304 completion in Done (#306)                 | dezren39 | 2026-10-23 |
| [#309](https://github.com/developing-today-forks/opencode/pull/309)   | docs(roadmap): record the #306 completion in Done (#308)                 | dezren39 | 2026-10-23 |
| [#311](https://github.com/developing-today-forks/opencode/pull/311)   | docs(roadmap): record the #308 completion in Done (#310)                 | dezren39 | 2026-10-23 |
| [#315](https://github.com/developing-today-forks/opencode/pull/315)   | docs(roadmap): record the #108 and #310 completions in Done              | dezren39 | 2026-10-23 |
| [#318](https://github.com/developing-today-forks/opencode/pull/318)   | docs(roadmap): record the #103 completion in Done                        | dezren39 | 2026-10-23 |
| [#320](https://github.com/developing-today-forks/opencode/pull/320)   | docs(roadmap): record the #314 and #317 completions in Done              | dezren39 | 2026-10-23 |
| [#323](https://github.com/developing-today-forks/opencode/pull/323)   | docs(roadmap): record the #115 and #319 completions in Done              | dezren39 | 2026-10-23 |
| [#325](https://github.com/developing-today-forks/opencode/pull/325)   | docs(roadmap): record the #322 completion in Done                        | dezren39 | 2026-10-23 |
| [#327](https://github.com/developing-today-forks/opencode/pull/327)   | docs(roadmap): record the #324 completion in Done (#326)                 | dezren39 | 2026-10-23 |
| [#329](https://github.com/developing-today-forks/opencode/pull/329)   | docs(roadmap): record the #326 completion in Done (#328)                 | dezren39 | 2026-10-23 |
| [#331](https://github.com/developing-today-forks/opencode/pull/331)   | docs(roadmap): record the #328 completion in Done (#330)                 | dezren39 | 2026-10-23 |
| [#333](https://github.com/developing-today-forks/opencode/pull/333)   | docs(roadmap): record the #330 completion in Done (#332)                 | dezren39 | 2026-10-23 |
| [#338](https://github.com/developing-today-forks/opencode/pull/338)   | docs(roadmap): record the #332, #334 and #117 completions in Done (#337) | dezren39 | 2026-10-23 |
| [#341](https://github.com/developing-today-forks/opencode/pull/341)   | docs(roadmap): record the #337 completion in Done (#339)                 | dezren39 | 2026-10-23 |
| [#344](https://github.com/developing-today-forks/opencode/pull/344)   | docs(roadmap): record the #339 and #340 completions in Done (#343)       | dezren39 | 2026-10-23 |
| [#348](https://github.com/developing-today-forks/opencode/pull/348)   | docs(roadmap): record the #343 and #345 completions in Done              | dezren39 | 2026-10-23 |
| [#351](https://github.com/developing-today-forks/opencode/pull/351)   | docs(roadmap): record the #347 and #105 completions in Done              | dezren39 | 2026-10-23 |
| [#353](https://github.com/developing-today-forks/opencode/pull/353)   | docs(roadmap): record the #350 completion in Done                        | dezren39 | 2026-10-23 |
| [#356](https://github.com/developing-today-forks/opencode/pull/356)   | docs(roadmap): record the #352 and #114 completions in Done              | dezren39 | 2026-10-23 |
| [#361](https://github.com/developing-today-forks/opencode/pull/361)   | docs(roadmap): record the #355 and #116 completions in Done              | dezren39 | 2026-10-24 |
| [#363](https://github.com/developing-today-forks/opencode/pull/363)   | docs(roadmap): record the #358 and #360 completions in Done (#362)       | dezren39 | 2026-10-24 |
| [#367](https://github.com/developing-today-forks/opencode/pull/367)   | docs(roadmap): record the #362 and #240 bulk completions in Done (#366)  | dezren39 | 2026-10-24 |
| [#369](https://github.com/developing-today-forks/opencode/pull/369)   | docs(roadmap): record the #366 completion in Done (#368)                 | dezren39 | 2026-10-24 |
| [#372](https://github.com/developing-today-forks/opencode/pull/372)   | docs(roadmap): record the #368 and #123 completions in Done              | dezren39 | 2026-10-24 |
| [#378](https://github.com/developing-today-forks/opencode/pull/378)   | docs(roadmap): record the #371 completion in Done                        | dezren39 | 2026-10-24 |
| [#381](https://github.com/developing-today-forks/opencode/pull/381)   | docs(roadmap): record the #375 completion in Done (#380)                 | dezren39 | 2026-10-24 |
| [#388](https://github.com/developing-today-forks/opencode/pull/388)   | docs(roadmap): record the #380 and #124 completions in Done              | dezren39 | 2026-10-24 |
| [#393](https://github.com/developing-today-forks/opencode/pull/393)   | docs(roadmap): record the #387 and #385 completions in Done              | dezren39 | 2026-10-24 |
| [#594](https://github.com/developing-today-forks/opencode/pull/594)   | docs(roadmap): drop the landed Nix-check row (#49)                       | dezren39 | 2026-10-27 |
| [#890](https://github.com/developing-today-forks/opencode/pull/890)   | docs(roadmap): program tending 2026-10-29 — drop landed rows             | dezren39 | 2026-10-30 |
| [#1095](https://github.com/developing-today-forks/opencode/pull/1095) | docs(roadmap): fix stale Areas index and dead plugin-api anchor          | dezren39 | 2026-10-31 |
| [#997](https://github.com/developing-today-forks/opencode/pull/997)   | docs(roadmap): program tending 2026-10-30 — drop landed failover row     | dezren39 | 2026-11-01 |

##### developing-today-forks/opencode — theme: server (42)

| PR                                                                    | Title                                                                                                      | Author   | Merged     |
| --------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#596](https://github.com/developing-today-forks/opencode/pull/596)   | fix(server): embed resolved models in /api/provider responses                                              | dezren39 | 2026-10-29 |
| [#1416](https://github.com/developing-today-forks/opencode/pull/1416) | feat(server): OpenAI-compatible completions door for external harnesses                                    | dezren39 | 2026-11-02 |
| [#1431](https://github.com/developing-today-forks/opencode/pull/1431) | feat(server): add GET /v1/models/{id} to the completions door                                              | dezren39 | 2026-11-02 |
| [#1434](https://github.com/developing-today-forks/opencode/pull/1434) | feat(server): embeddings door — POST /v1/embeddings backed by a local Snowflake model                      | dezren39 | 2026-11-02 |
| [#1435](https://github.com/developing-today-forks/opencode/pull/1435) | feat(server): Anthropic Messages door — POST /v1/messages for any model                                    | dezren39 | 2026-11-02 |
| [#1439](https://github.com/developing-today-forks/opencode/pull/1439) | feat(server): Responses item store + previous_response_id chains (#1149)                                   | dezren39 | 2026-11-02 |
| [#1450](https://github.com/developing-today-forks/opencode/pull/1450) | feat(server): systemone-standard endpoint for the clef family (#1410)                                      | dezren39 | 2026-11-02 |
| [#1587](https://github.com/developing-today-forks/opencode/pull/1587) | fix(server): honor session.create ?directory= with payload precedence (#1570)                              | dezren39 | 2026-11-04 |
| [#777](https://github.com/developing-today-forks/opencode/pull/777)   | fix(server): close the serve handover teardown race                                                        | dezren39 | 2026-11-08 |
| [#2238](https://github.com/developing-today-forks/opencode/pull/2238) | feat(server): let ancestors see and answer descendant session forms                                        | dezren39 | 2026-11-10 |
| [#2272](https://github.com/developing-today-forks/opencode/pull/2272) | fix(server): invalidate replay cursors on event encoding failure (#2271)                                   | dezren39 | 2026-11-11 |
| [#2299](https://github.com/developing-today-forks/opencode/pull/2299) | fix(server): omit IPv6 link-local pairing URLs                                                             | dezren39 | 2026-11-12 |
| [#2309](https://github.com/developing-today-forks/opencode/pull/2309) | fix(server): compare Origin scheme for session cookies                                                     | dezren39 | 2026-11-12 |
| [#2308](https://github.com/developing-today-forks/opencode/pull/2308) | fix(server): keep link previews from redeeming one-time pairing codes                                      | dezren39 | 2026-11-12 |
| [#2325](https://github.com/developing-today-forks/opencode/pull/2325) | fix(server): bound live PTY WebSocket output buffering                                                     | dezren39 | 2026-11-13 |
| [#2399](https://github.com/developing-today-forks/opencode/pull/2399) | test(server): add required cidr to server-info NetworkInterfaceInfo mocks (#2355)                          | dezren39 | 2026-11-13 |
| [#2430](https://github.com/developing-today-forks/opencode/pull/2430) | fix(server): drain PTY websockets during graceful shutdown (#2284)                                         | dezren39 | 2026-11-14 |
| [#2458](https://github.com/developing-today-forks/opencode/pull/2458) | fix(server): scope PTY shutdown registration to the connect handler (#2454)                                | dezren39 | 2026-11-15 |
| [#2515](https://github.com/developing-today-forks/opencode/pull/2515) | fix(server): bound STT transcriptions body while reading (#2433)                                           | dezren39 | 2026-11-16 |
| [#2578](https://github.com/developing-today-forks/opencode/pull/2578) | fix(server): reject weak validators for If-Range requests (#2268)                                          | dezren39 | 2026-11-17 |
| [#2583](https://github.com/developing-today-forks/opencode/pull/2583) | fix(server): bound and validate persistent-pty connect attachment_id (#2465)                               | dezren39 | 2026-11-17 |
| [#2589](https://github.com/developing-today-forks/opencode/pull/2589) | fix(server): bound session.list limit at 200 (#2483)                                                       | dezren39 | 2026-11-17 |
| [#2590](https://github.com/developing-today-forks/opencode/pull/2590) | fix(server): answer 404 for credential update/activate/remove on a missing credential                      | dezren39 | 2026-11-17 |
| [#2592](https://github.com/developing-today-forks/opencode/pull/2592) | fix(server): bind the type filter into session.messages cursors (#2501)                                    | dezren39 | 2026-11-17 |
| [#2591](https://github.com/developing-today-forks/opencode/pull/2591) | fix(server): shell.remove returns 404 for a missing shell command                                          | dezren39 | 2026-11-17 |
| [#2596](https://github.com/developing-today-forks/opencode/pull/2596) | fix(server): bound JSON request bodies before buffering                                                    | dezren39 | 2026-11-17 |
| [#2597](https://github.com/developing-today-forks/opencode/pull/2597) | fix(server): compare Basic-auth passwords in constant time                                                 | dezren39 | 2026-11-17 |
| [#2613](https://github.com/developing-today-forks/opencode/pull/2613) | fix(server): permission.saved.remove returns 404 for unknown ID (#2556)                                    | dezren39 | 2026-11-17 |
| [#2614](https://github.com/developing-today-forks/opencode/pull/2614) | fix(server): bound shell output capture and clamp shell.output page limit (#2486)                          | dezren39 | 2026-11-17 |
| [#2615](https://github.com/developing-today-forks/opencode/pull/2615) | fix(server): register persistent PTY connect sockets for shutdown drain (#2520)                            | dezren39 | 2026-11-17 |
| [#2616](https://github.com/developing-today-forks/opencode/pull/2616) | fix(server): bound experimental fs.write request body at 10 MiB (#2502)                                    | dezren39 | 2026-11-17 |
| [#2617](https://github.com/developing-today-forks/opencode/pull/2617) | fix(server): session inbox cancel returns 409 on InboxConflictError (#2553)                                | dezren39 | 2026-11-17 |
| [#2496](https://github.com/developing-today-forks/opencode/pull/2496) | feat(server): route-overrides API owned by the OpenCode server (#2494)                                     | dezren39 | 2026-11-20 |
| [#2656](https://github.com/developing-today-forks/opencode/pull/2656) | fix(server): validate session and entry existence in instructions-entry endpoints                          | dezren39 | 2026-11-21 |
| [#2729](https://github.com/developing-today-forks/opencode/pull/2729) | fix(server): disposition the transcriptions content-length conversion for check:numeric-boundaries (#2587) | dezren39 | 2026-11-23 |
| [#2735](https://github.com/developing-today-forks/opencode/pull/2735) | fix(server): integration cancel returns 404 for unknown attempts                                           | dezren39 | 2026-11-23 |
| [#2736](https://github.com/developing-today-forks/opencode/pull/2736) | fix(server): bound vcs.status results with a page limit and truncation marker                              | dezren39 | 2026-11-23 |
| [#2743](https://github.com/developing-today-forks/opencode/pull/2743) | fix(server): bound fs.find page limit at 200                                                               | dezren39 | 2026-11-23 |
| [#2746](https://github.com/developing-today-forks/opencode/pull/2746) | feat(server): read API over the route_attempt ledger (#2497 slice 1)                                       | dezren39 | 2026-11-23 |
| [#2633](https://github.com/developing-today-forks/opencode/pull/2633) | test(server): skip the /bin/sh PTY drain case on win32                                                     | dezren39 | 2026-11-23 |
| [#2765](https://github.com/developing-today-forks/opencode/pull/2765) | fix(server): job.cancel returns 404 for unknown job IDs                                                    | dezren39 | 2026-11-23 |
| [#2769](https://github.com/developing-today-forks/opencode/pull/2769) | fix(server): bound fs.list directory listing with limit/offset paging                                      | dezren39 | 2026-11-23 |

##### developing-today-forks/opencode — theme: schema (26)

| PR                                                                    | Title                                                                                                          | Author   | Merged     |
| --------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#57](https://github.com/developing-today-forks/opencode/pull/57)     | test(schema): use fileURLToPath for the source scan dir so tests pass on Windows                               | dezren39 | 2026-10-18 |
| [#364](https://github.com/developing-today-forks/opencode/pull/364)   | fix(schema): annotate model-route field descriptions before optional so they reach the OpenAPI document        | dezren39 | 2026-10-24 |
| [#403](https://github.com/developing-today-forks/opencode/pull/403)   | fix(schema): validate variants on the alias path and settle chains at the depth limit                          | dezren39 | 2026-10-24 |
| [#410](https://github.com/developing-today-forks/opencode/pull/410)   | fix(schema): validate provider and variant alias targets                                                       | dezren39 | 2026-10-25 |
| [#413](https://github.com/developing-today-forks/opencode/pull/413)   | fix(schema): reject empty model alias targets                                                                  | dezren39 | 2026-10-25 |
| [#415](https://github.com/developing-today-forks/opencode/pull/415)   | fix(schema): use own-property alias lookups and reject padded alias targets                                    | dezren39 | 2026-10-25 |
| [#425](https://github.com/developing-today-forks/opencode/pull/425)   | fix(schema): test scoped alias slash only before the variant separator                                         | dezren39 | 2026-10-25 |
| [#440](https://github.com/developing-today-forks/opencode/pull/440)   | fix(schema): reject padded and empty segments in model references                                              | dezren39 | 2026-10-25 |
| [#445](https://github.com/developing-today-forks/opencode/pull/445)   | fix(schema): reject model reference segments with internal whitespace                                          | dezren39 | 2026-10-25 |
| [#459](https://github.com/developing-today-forks/opencode/pull/459)   | fix(schema): reject padded, empty and whitespace segments in Model.Ref.parse                                   | dezren39 | 2026-10-25 |
| [#466](https://github.com/developing-today-forks/opencode/pull/466)   | fix(schema): allow slash-containing variants in ConfigModel.Selection string form                              | dezren39 | 2026-10-26 |
| [#474](https://github.com/developing-today-forks/opencode/pull/474)   | fix(schema): require segment-form Instruction.Key                                                              | dezren39 | 2026-10-26 |
| [#490](https://github.com/developing-today-forks/opencode/pull/490)   | fix(schema): reject unmatchable Model.Selector segments                                                        | dezren39 | 2026-10-26 |
| [#510](https://github.com/developing-today-forks/opencode/pull/510)   | fix(schema): reject trailing-dot InstructionEntry keys                                                         | dezren39 | 2026-10-26 |
| [#529](https://github.com/developing-today-forks/opencode/pull/529)   | fix(schema): reject #-in-segment and *-in-provider/variant model selectors                                     | dezren39 | 2026-10-26 |
| [#571](https://github.com/developing-today-forks/opencode/pull/571)   | fix(schema): widen Identifier sortable prefix to 14 hex chars (#561)                                           | dezren39 | 2026-10-27 |
| [#597](https://github.com/developing-today-forks/opencode/pull/597)   | fix(schema): annotate model-route field descriptions before optional so they reach the OpenAPI document (#240) | dezren39 | 2026-10-27 |
| [#651](https://github.com/developing-today-forks/opencode/pull/651)   | fix(schema): annotate remaining model-route descriptions before optional so they reach OpenAPI (#645)          | dezren39 | 2026-10-28 |
| [#795](https://github.com/developing-today-forks/opencode/pull/795)   | fix(schema): move field descriptions inside optional so they reach the encoded JSON schema (#650)              | dezren39 | 2026-10-29 |
| [#1578](https://github.com/developing-today-forks/opencode/pull/1578) | fix(schema): reject escalation afterGiveUps above outcomeWindow at decode (#1163)                              | dezren39 | 2026-11-04 |
| [#1565](https://github.com/developing-today-forks/opencode/pull/1565) | fix(schema): bound MCP timeouts at the setTimeout cliff                                                        | dezren39 | 2026-11-04 |
| [#1595](https://github.com/developing-today-forks/opencode/pull/1595) | fix(schema): enforce exactly one of factor and weight on route schedules (#1589)                               | dezren39 | 2026-11-04 |
| [#1601](https://github.com/developing-today-forks/opencode/pull/1601) | fix(schema): preserve optional array descriptions                                                              | dezren39 | 2026-11-04 |
| [#1687](https://github.com/developing-today-forks/opencode/pull/1687) | fix(schema): reject invalid route window bounds                                                                | dezren39 | 2026-11-05 |
| [#2035](https://github.com/developing-today-forks/opencode/pull/2035) | fix(schema): validate route Window string fields at decode time                                                | dezren39 | 2026-11-08 |
| [#2379](https://github.com/developing-today-forks/opencode/pull/2379) | fix(schema): reconcile route Window decode with the runtime parser for list/day/epoch fields (#2041)           | dezren39 | 2026-11-13 |

##### developing-today-forks/opencode — theme: script (24)

| PR                                                                    | Title                                                                                             | Author                | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- | --------------------- | ---------- |
| [#107](https://github.com/developing-today-forks/opencode/pull/107)   | fix(script): detect leaked mcpx daemons on macOS and Windows                                      | dezren39              | 2026-10-19 |
| [#113](https://github.com/developing-today-forks/opencode/pull/113)   | fix(script): gate mcpx-cleanup on native Windows instead of crashing on ps                        | dezren39              | 2026-10-19 |
| [#359](https://github.com/developing-today-forks/opencode/pull/359)   | fix(script): doc-env-var guard matches whole variable names (#358)                                | dezren39              | 2026-10-24 |
| [#1160](https://github.com/developing-today-forks/opencode/pull/1160) | feat(script): check:bounded-http regression net for unbounded outbound calls (#1129)              | dezren39              | 2026-11-01 |
| [#1279](https://github.com/developing-today-forks/opencode/pull/1279) | fix(script): bound the registry version-resolution fetch (#1275)                                  | dezren39              | 2026-11-01 |
| [#1289](https://github.com/developing-today-forks/opencode/pull/1289) | fix(script): normalize gate collector paths to POSIX                                              | dezren39              | 2026-11-01 |
| [#1297](https://github.com/developing-today-forks/opencode/pull/1297) | feat(script): scan packages/server in numeric-boundaries gate                                     | dezren39              | 2026-11-01 |
| [#1304](https://github.com/developing-today-forks/opencode/pull/1304) | feat(script): scan console and function packages in numeric-boundaries gate                       | dezren39              | 2026-11-01 |
| [#1303](https://github.com/developing-today-forks/opencode/pull/1303) | fix(script): add .js extension to ./registry import for NodeNext resolution                       | dezren39              | 2026-11-01 |
| [#1331](https://github.com/developing-today-forks/opencode/pull/1331) | fix(script): bound update-artifact publish requests                                               | dezren39              | 2026-11-01 |
| [#1335](https://github.com/developing-today-forks/opencode/pull/1335) | fix(script): bound publish-winget GitHub API requests                                             | dezren39              | 2026-11-01 |
| [#1334](https://github.com/developing-today-forks/opencode/pull/1334) | fix(script): cap update-artifact response bodies at 1 MiB                                         | dezren39              | 2026-11-02 |
| [#1338](https://github.com/developing-today-forks/opencode/pull/1338) | fix(script): cap publish-winget GitHub response bodies at 1 MiB                                   | dezren39              | 2026-11-02 |
| [#1343](https://github.com/developing-today-forks/opencode/pull/1343) | fix(script): add stats.ts fetch deadlines and bounded JSON reads (#1337)                          | dezren39              | 2026-11-02 |
| [#1357](https://github.com/developing-today-forks/opencode/pull/1357) | fix(script): bound GitHub closer requests                                                         | dezren39              | 2026-11-02 |
| [#1652](https://github.com/developing-today-forks/opencode/pull/1652) | fix(script): disposition bounded-http and numeric gate drift (#1540)                              | dezren39              | 2026-11-04 |
| [#1664](https://github.com/developing-today-forks/opencode/pull/1664) | fix(script): re-sync httpapi-codegen template allowlist entries to the post-#1640 text (#1655)    | dezren39              | 2026-11-04 |
| [#1665](https://github.com/developing-today-forks/opencode/pull/1665) | fix(script): disposition gemini minItems parse in numeric-boundaries gate (#1660)                 | dezren39              | 2026-11-04 |
| [#1774](https://github.com/developing-today-forks/opencode/pull/1774) | test(script): a stalled later releases page rejects instead of returning partial releases (#1530) | developing-today[bot] | 2026-11-05 |
| [#2175](https://github.com/developing-today-forks/opencode/pull/2175) | fix(script): skip pull-request items in issue sweeper (#2120)                                     | dezren39              | 2026-11-08 |
| [#2217](https://github.com/developing-today-forks/opencode/pull/2217) | fix(script): never split a surrogate pair in release-highlights truncate (#2214)                  | dezren39              | 2026-11-09 |
| [#2218](https://github.com/developing-today-forks/opencode/pull/2218) | fix(script): reject multi-arch universal Mach-O in NativeArch.verify (#2211)                      | dezren39              | 2026-11-09 |
| [#2221](https://github.com/developing-today-forks/opencode/pull/2221) | fix(script): classify raw-changelog by leading conventional type (#2206)                          | dezren39              | 2026-11-09 |
| [#2428](https://github.com/developing-today-forks/opencode/pull/2428) | fix(script): adapt raw-changelog to the fork layout and repo                                      | dezren39              | 2026-11-14 |

##### developing-today-forks/opencode — theme: tui (19)

| PR                                                                    | Title                                                                               | Author   | Merged     |
| --------------------------------------------------------------------- | ----------------------------------------------------------------------------------- | -------- | ---------- |
| [#64](https://github.com/developing-today-forks/opencode/pull/64)     | fix(tui): bubble nested subagent asks up to the root                                | dezren39 | 2026-10-18 |
| [#96](https://github.com/developing-today-forks/opencode/pull/96)     | fix(tui): retry writeJsonAtomic rename on transient Windows errors                  | dezren39 | 2026-10-19 |
| [#247](https://github.com/developing-today-forks/opencode/pull/247)   | fix(tui): parse editor ports strictly                                               | dezren39 | 2026-10-21 |
| [#401](https://github.com/developing-today-forks/opencode/pull/401)   | feat(tui): name the asking subagent in the permission prompt                        | dezren39 | 2026-10-24 |
| [#449](https://github.com/developing-today-forks/opencode/pull/449)   | fix(tui): give the footer form-draft fixture a label and description                | dezren39 | 2026-10-25 |
| [#607](https://github.com/developing-today-forks/opencode/pull/607)   | fix(tui): truncators respect lengths 0-2                                            | dezren39 | 2026-10-27 |
| [#746](https://github.com/developing-today-forks/opencode/pull/746)   | feat(tui): nest the mini Select subagent menu                                       | dezren39 | 2026-10-28 |
| [#789](https://github.com/developing-today-forks/opencode/pull/789)   | test(tui): provide Route context in permission input-scope tests                    | dezren39 | 2026-10-29 |
| [#1050](https://github.com/developing-today-forks/opencode/pull/1050) | test(tui): unskip direct-footer frame and leader tests (#1040)                      | dezren39 | 2026-10-31 |
| [#1675](https://github.com/developing-today-forks/opencode/pull/1675) | fix(tui): validate frecency JSONL entries                                           | dezren39 | 2026-11-05 |
| [#1721](https://github.com/developing-today-forks/opencode/pull/1721) | fix(tui): skip malformed custom themes during discovery                             | dezren39 | 2026-11-05 |
| [#2304](https://github.com/developing-today-forks/opencode/pull/2304) | docs(tui): explain HTML injection limitation                                        | dezren39 | 2026-11-12 |
| [#2420](https://github.com/developing-today-forks/opencode/pull/2420) | feat(tui): voice input — attach + mic capture + review + composer insertion (#2387) | dezren39 | 2026-11-14 |
| [#2435](https://github.com/developing-today-forks/opencode/pull/2435) | test(tui): assert parser config fields with Object.hasOwn                           | dezren39 | 2026-11-14 |
| [#2529](https://github.com/developing-today-forks/opencode/pull/2529) | fix(tui): /voice-send in-flight guard — a repeat beginSubmit grants nothing (#2446) | dezren39 | 2026-11-16 |
| [#2741](https://github.com/developing-today-forks/opencode/pull/2741) | fix(tui): lock and atomically rewrite the shared prompt JSONL stores                | dezren39 | 2026-11-23 |
| [#2745](https://github.com/developing-today-forks/opencode/pull/2745) | docs(tui): refresh the HTML injection limitation note for OpenTUI 0.9.4             | dezren39 | 2026-11-23 |
| [#2757](https://github.com/developing-today-forks/opencode/pull/2757) | fix(tui): do not treat JSONL read failures as an empty store                        | dezren39 | 2026-11-23 |
| [#2781](https://github.com/developing-today-forks/opencode/pull/2781) | fix(tui): refuse to overwrite a present-but-unreadable preference store (#2778)     | dezren39 | 2026-11-23 |

##### developing-today-forks/opencode — theme: gui-extensions (17)

| PR                                                                    | Title                                                                                           | Author   | Merged     |
| --------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#270](https://github.com/developing-today-forks/opencode/pull/270)   | fix(gui-extensions): read remote CLI version past login-shell banners                           | dezren39 | 2026-10-22 |
| [#271](https://github.com/developing-today-forks/opencode/pull/271)   | fix(gui-extensions): read WSL probe answer line instead of whole login-shell output             | dezren39 | 2026-10-22 |
| [#275](https://github.com/developing-today-forks/opencode/pull/275)   | fix(gui-extensions): read the CLI version from the last version-shaped line                     | dezren39 | 2026-10-22 |
| [#279](https://github.com/developing-today-forks/opencode/pull/279)   | fix(gui-extensions): frame remote CLI version output so a banner cannot invent a version        | dezren39 | 2026-10-22 |
| [#283](https://github.com/developing-today-forks/opencode/pull/283)   | fix(gui-extensions): put the CLI version frame END marker on its own line                       | dezren39 | 2026-10-22 |
| [#287](https://github.com/developing-today-forks/opencode/pull/287)   | fix(gui-extensions): put the CLI version frame BEGIN marker on its own line                     | dezren39 | 2026-10-22 |
| [#293](https://github.com/developing-today-forks/opencode/pull/293)   | fix(gui-extensions): put the probe, registration status and discovery prints on their own lines | dezren39 | 2026-10-22 |
| [#301](https://github.com/developing-today-forks/opencode/pull/301)   | fix(gui-extensions): put the WSL probeDistro answers on their own lines                         | dezren39 | 2026-10-22 |
| [#335](https://github.com/developing-today-forks/opencode/pull/335)   | fix(gui-extensions): WSL CLI discovery takes the last path-shaped line, not the first           | dezren39 | 2026-10-23 |
| [#342](https://github.com/developing-today-forks/opencode/pull/342)   | fix(gui-extensions): WSL discovery requires the managed CLI path suffix                         | dezren39 | 2026-10-23 |
| [#346](https://github.com/developing-today-forks/opencode/pull/346)   | fix(gui-extensions): frame WSL CLI discovery output                                             | dezren39 | 2026-10-23 |
| [#1064](https://github.com/developing-today-forks/opencode/pull/1064) | test(gui-extensions): cover whats-new changelog size cap                                        | dezren39 | 2026-10-31 |
| [#1011](https://github.com/developing-today-forks/opencode/pull/1011) | fix(gui-extensions): bound SSH bootstrap fallback download (size cap + timeout)                 | dezren39 | 2026-11-01 |
| [#1292](https://github.com/developing-today-forks/opencode/pull/1292) | fix(gui-extensions): cap SSH remote archive downloads                                           | dezren39 | 2026-11-01 |
| [#1310](https://github.com/developing-today-forks/opencode/pull/1310) | fix(gui-extensions): bound SSH bootstrap fallback download by one overall deadline              | dezren39 | 2026-11-01 |
| [#1330](https://github.com/developing-today-forks/opencode/pull/1330) | fix(gui-extensions): refuse malformed percent-encoded file URLs instead of throwing             | dezren39 | 2026-11-01 |
| [#2701](https://github.com/developing-today-forks/opencode/pull/2701) | fix(gui-extensions): pick the newest cached beta in SSH remote-CLI discovery                    | dezren39 | 2026-11-23 |

##### developing-today-forks/opencode — theme: client (15)

| PR                                                                    | Title                                                                                            | Author                | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ | --------------------- | ---------- |
| [#451](https://github.com/developing-today-forks/opencode/pull/451)   | chore(client): regenerate generated client for ConfigEntry aliases                               | dezren39              | 2026-10-25 |
| [#484](https://github.com/developing-today-forks/opencode/pull/484)   | chore(client): regenerate client for subsession governance schemas                               | dezren39              | 2026-10-26 |
| [#664](https://github.com/developing-today-forks/opencode/pull/664)   | fix(client): update ProviderInfo consumers to ProviderInfoWithModels                             | dezren39              | 2026-10-28 |
| [#685](https://github.com/developing-today-forks/opencode/pull/685)   | fix(client): regenerate client for ConfigModelRoutes.limit                                       | dezren39              | 2026-10-28 |
| [#815](https://github.com/developing-today-forks/opencode/pull/815)   | chore(client): regenerate generated client                                                       | dezren39              | 2026-10-29 |
| [#843](https://github.com/developing-today-forks/opencode/pull/843)   | fix(client): load session ancestors on sync so depth>=2 subagent views resolve their root (#631) | dezren39              | 2026-10-29 |
| [#1021](https://github.com/developing-today-forks/opencode/pull/1021) | fix(client): cap the service probe /api/info body read                                           | dezren39              | 2026-10-30 |
| [#1048](https://github.com/developing-today-forks/opencode/pull/1048) | fix(client): retry truncated ancestor walks instead of caching them                              | dezren39              | 2026-10-31 |
| [#1047](https://github.com/developing-today-forks/opencode/pull/1047) | fix(client): family sync publishes only ancestors the store still lacks                          | dezren39              | 2026-10-31 |
| [#1052](https://github.com/developing-today-forks/opencode/pull/1052) | fix(client): do not let a family sync overwrite a newer target snapshot                          | dezren39              | 2026-10-31 |
| [#1054](https://github.com/developing-today-forks/opencode/pull/1054) | fix(client): retry family sync target when covering plain sync fails (#1053)                     | dezren39              | 2026-10-31 |
| [#1366](https://github.com/developing-today-forks/opencode/pull/1366) | fix(client): regenerate client types for quotaGate.balanceDeadThreshold (#1365)                  | dezren39              | 2026-11-02 |
| [#1495](https://github.com/developing-today-forks/opencode/pull/1495) | fix(client): regenerate client for route system field and completions error statuses             | dezren39              | 2026-11-03 |
| [#1510](https://github.com/developing-today-forks/opencode/pull/1510) | fix(client): compose a default 120s deadline into generated promise transport                    | dezren39              | 2026-11-03 |
| [#2326](https://github.com/developing-today-forks/opencode/pull/2326) | fix(client): automatically reconnect Effect event subscriptions                                  | developing-today[bot] | 2026-11-13 |

##### developing-today-forks/opencode — theme: app (14)

| PR                                                                    | Title                                                                            | Author   | Merged     |
| --------------------------------------------------------------------- | -------------------------------------------------------------------------------- | -------- | ---------- |
| [#60](https://github.com/developing-today-forks/opencode/pull/60)     | fix(app): surface nested subagent asks at the root and nest running subagents    | dezren39 | 2026-10-18 |
| [#84](https://github.com/developing-today-forks/opencode/pull/84)     | fix(app): surface sessions in a parent cycle in the worktree list                | dezren39 | 2026-10-18 |
| [#370](https://github.com/developing-today-forks/opencode/pull/370)   | fix(app): root the running menu at the session tree root                         | dezren39 | 2026-10-24 |
| [#384](https://github.com/developing-today-forks/opencode/pull/384)   | feat(app): name the subagent that asked in permission and question docks         | dezren39 | 2026-10-24 |
| [#752](https://github.com/developing-today-forks/opencode/pull/752)   | fix(app): raise Playwright config spawn timeout for Windows cold start           | dezren39 | 2026-10-28 |
| [#1640](https://github.com/developing-today-forks/opencode/pull/1640) | fix(app): update signal tests for the composed client deadline (#1562)           | dezren39 | 2026-11-04 |
| [#2340](https://github.com/developing-today-forks/opencode/pull/2340) | fix(app): load root family for subagent running menu (#1280, lands rescue delta) | dezren39 | 2026-11-13 |
| [#2377](https://github.com/developing-today-forks/opencode/pull/2377) | fix(app): label recent-session search close action                               | dezren39 | 2026-11-13 |
| [#2463](https://github.com/developing-today-forks/opencode/pull/2463) | fix(app): reveal hover-only controls on keyboard focus                           | dezren39 | 2026-11-16 |
| [#2611](https://github.com/developing-today-forks/opencode/pull/2611) | fix(app): settle aborted-while-queued requests immediately                       | dezren39 | 2026-11-17 |
| [#2604](https://github.com/developing-today-forks/opencode/pull/2604) | fix(app): announce selected session tab                                          | dezren39 | 2026-11-17 |
| [#2684](https://github.com/developing-today-forks/opencode/pull/2684) | fix(app): indent running subagents under the one that launched them              | dezren39 | 2026-11-22 |
| [#2732](https://github.com/developing-today-forks/opencode/pull/2732) | fix(app): wire Home session archiving to the V2 session.update API               | dezren39 | 2026-11-23 |
| [#2777](https://github.com/developing-today-forks/opencode/pull/2777) | feat(app): restore custom provider setup in V2 (opencode#2652 criteria 1-2)      | dezren39 | 2026-11-23 |

##### developing-today-forks/opencode — theme: desktop (14)

| PR                                                                    | Title                                                                                               | Author                | Merged     |
| --------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- | --------------------- | ---------- |
| [#167](https://github.com/developing-today-forks/opencode/pull/167)   | test(desktop): run runScript tests with a cross-platform child                                      | dezren39              | 2026-10-20 |
| [#278](https://github.com/developing-today-forks/opencode/pull/278)   | fix(desktop): validate the CLI version line instead of trusting the last v token                    | dezren39              | 2026-10-22 |
| [#321](https://github.com/developing-today-forks/opencode/pull/321)   | fix(desktop): remove dead experimental env setters                                                  | dezren39              | 2026-10-23 |
| [#1350](https://github.com/developing-today-forks/opencode/pull/1350) | fix(desktop): return 404 for malformed renderer paths                                               | dezren39              | 2026-11-02 |
| [#1356](https://github.com/developing-today-forks/opencode/pull/1356) | fix(desktop): return 404 for malformed extension assets                                             | dezren39              | 2026-11-02 |
| [#1521](https://github.com/developing-today-forks/opencode/pull/1521) | fix(desktop): atomic saveFile via writeFileAtomic/renameWithRetry (#1005)                           | developing-today[bot] | 2026-11-03 |
| [#1528](https://github.com/developing-today-forks/opencode/pull/1528) | fix(desktop): renderer path guard no longer rejects in-root ..-prefixed names                       | dezren39              | 2026-11-03 |
| [#1568](https://github.com/developing-today-forks/opencode/pull/1568) | fix(desktop): bound finalize-latest-json GitHub fetches (deadline, bounded JSON, streamed download) | dezren39              | 2026-11-03 |
| [#2422](https://github.com/developing-today-forks/opencode/pull/2422) | fix(desktop): overall deadline for extension archive downloads (#1091)                              | dezren39              | 2026-11-14 |
| [#2607](https://github.com/developing-today-forks/opencode/pull/2607) | fix(desktop): reschedule a failed write-behind flush with backoff                                   | dezren39              | 2026-11-17 |
| [#2609](https://github.com/developing-today-forks/opencode/pull/2609) | fix(desktop): bound extension archive decompression by actual bytes                                 | dezren39              | 2026-11-17 |
| [#2706](https://github.com/developing-today-forks/opencode/pull/2706) | fix(desktop): harden window-bootstrap parsing in preload                                            | dezren39              | 2026-11-23 |
| [#2762](https://github.com/developing-today-forks/opencode/pull/2762) | fix(desktop): handle rejected renderer loadURL promises                                             | dezren39              | 2026-11-23 |
| [#2774](https://github.com/developing-today-forks/opencode/pull/2774) | feat(desktop): recover window bootstrap over IPC when the argv bootstrap is missing                 | dezren39              | 2026-11-23 |

##### developing-today-forks/opencode — theme: console (14)

| PR                                                                    | Title                                                                                      | Author   | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ | -------- | ---------- |
| [#1093](https://github.com/developing-today-forks/opencode/pull/1093) | fix(console): bound changelog fetch in time and size                                       | dezren39 | 2026-10-31 |
| [#1094](https://github.com/developing-today-forks/opencode/pull/1094) | fix(console): bound download route update-API fetch (8s timeout, 1MiB cap)                 | dezren39 | 2026-10-31 |
| [#1118](https://github.com/developing-today-forks/opencode/pull/1118) | fix(console): bound stats proxy upstream fetch — 8s timeout and 1 MiB HTML cap             | dezren39 | 2026-11-01 |
| [#1131](https://github.com/developing-today-forks/opencode/pull/1131) | fix(console): bound four proxy routes with an 8s upstream deadline                         | dezren39 | 2026-11-01 |
| [#1135](https://github.com/developing-today-forks/opencode/pull/1135) | fix(console): bound Honeycomb webhook Discord relay with a 5s deadline                     | dezren39 | 2026-11-01 |
| [#1157](https://github.com/developing-today-forks/opencode/pull/1157) | fix(console): bound the SES sendEmail fetch with a 5s timeout                              | dezren39 | 2026-11-01 |
| [#1162](https://github.com/developing-today-forks/opencode/pull/1162) | fix(console): cap the CRM lookup response body at 64 KiB (#1158)                           | dezren39 | 2026-11-01 |
| [#1187](https://github.com/developing-today-forks/opencode/pull/1187) | fix(console): bound inference proxy fetch with a headers deadline (#1186)                  | dezren39 | 2026-11-01 |
| [#1312](https://github.com/developing-today-forks/opencode/pull/1312) | fix(console): never serve an Accept-Language language refused with q=0 (#1309)             | dezren39 | 2026-11-01 |
| [#1324](https://github.com/developing-today-forks/opencode/pull/1324) | fix(console): guard locale lookups against prototype names and malformed oc_locale cookies | dezren39 | 2026-11-01 |
| [#1520](https://github.com/developing-today-forks/opencode/pull/1520) | fix(console): compose incoming request signal with headers deadline in inference proxy     | dezren39 | 2026-11-03 |
| [#1561](https://github.com/developing-today-forks/opencode/pull/1561) | fix(console): bound Salesforce login and lead requests                                     | dezren39 | 2026-11-03 |
| [#1658](https://github.com/developing-today-forks/opencode/pull/1658) | test(console): one shared console-resource preload fixture (#1522)                         | dezren39 | 2026-11-04 |
| [#1608](https://github.com/developing-today-forks/opencode/pull/1608) | fix(console): bound openapi.json upstream fetch                                            | dezren39 | 2026-11-05 |

##### developing-today-forks/opencode — theme: nix (13)

| PR                                                                    | Title                                                                            | Author   | Merged     |
| --------------------------------------------------------------------- | -------------------------------------------------------------------------------- | -------- | ---------- |
| [#189](https://github.com/developing-today-forks/opencode/pull/189)   | fix(nix): update stale node_modules hashes and fix the nix-hashes updater        | dezren39 | 2026-10-20 |
| [#386](https://github.com/developing-today-forks/opencode/pull/386)   | fix(nix): build on symlinked nix stores without relaxing purity elsewhere        | dezren39 | 2026-10-24 |
| [#770](https://github.com/developing-today-forks/opencode/pull/770)   | fix(nix): build opencode with mcpx from developing-today/code by default         | dezren39 | 2026-10-28 |
| [#1417](https://github.com/developing-today-forks/opencode/pull/1417) | fix(nix): refresh x86_64-linux nodeModules hash for current bun.lock             | dezren39 | 2026-11-02 |
| [#1430](https://github.com/developing-today-forks/opencode/pull/1430) | fix(nix): refresh x86_64-linux nodeModules hash (post-#1416 bun.lock)            | dezren39 | 2026-11-02 |
| [#1470](https://github.com/developing-today-forks/opencode/pull/1470) | chore(nix): follow fleet nixpkgs pin and add fleet cache                         | dezren39 | 2026-11-03 |
| [#1581](https://github.com/developing-today-forks/opencode/pull/1581) | fix(nix): restamp x86_64-linux nodeModules hash for fleet-pinned bun             | dezren39 | 2026-11-04 |
| [#1747](https://github.com/developing-today-forks/opencode/pull/1747) | fix(nix): refresh x86_64-linux nodeModules hash for b2459a47 lockfile            | dezren39 | 2026-11-05 |
| [#1754](https://github.com/developing-today-forks/opencode/pull/1754) | fix(nix): refresh x86_64-linux nodeModules hash for current bun.lock (24784651)  | dezren39 | 2026-11-05 |
| [#1739](https://github.com/developing-today-forks/opencode/pull/1739) | fix(nix): provide libstdc++/zlib for native-onnx embeddings in packaged opencode | dezren39 | 2026-11-05 |
| [#2269](https://github.com/developing-today-forks/opencode/pull/2269) | fix(nix): correct aarch64-darwin node_modules hash on the 6c43550 pin lineage    | dezren39 | 2026-11-11 |
| [#2649](https://github.com/developing-today-forks/opencode/pull/2649) | fix(nix): pull npm tarballs from the default registry, bound bun install         | dezren39 | 2026-11-19 |
| [#2712](https://github.com/developing-today-forks/opencode/pull/2712) | chore(nix): restamp x86_64-linux node_modules hash                               | dezren39 | 2026-11-23 |

##### developing-today-forks/opencode — theme: util (10)

| PR                                                                    | Title                                                                         | Author                | Merged     |
| --------------------------------------------------------------------- | ----------------------------------------------------------------------------- | --------------------- | ---------- |
| [#33](https://github.com/developing-today-forks/opencode/pull/33)     | feat(util): OPENCODE_TMPDIR durable scratch directory override                | dezren39              | 2026-10-18 |
| [#622](https://github.com/developing-today-forks/opencode/pull/622)   | fix(util): clamp retry attempts to one                                        | dezren39              | 2026-10-27 |
| [#634](https://github.com/developing-today-forks/opencode/pull/634)   | fix(util): scope activityCalendar levels to requested range                   | dezren39              | 2026-10-27 |
| [#654](https://github.com/developing-today-forks/opencode/pull/654)   | fix(util): guard getFilenameTruncated at maxLength <= 0 (#653)                | dezren39              | 2026-10-28 |
| [#698](https://github.com/developing-today-forks/opencode/pull/698)   | fix(util): normalize non-finite retry attempts                                | dezren39              | 2026-10-28 |
| [#1355](https://github.com/developing-today-forks/opencode/pull/1355) | feat(util): FSUtil.writeAtomic — atomic-replace primitive + conformance suite | dezren39              | 2026-11-02 |
| [#2190](https://github.com/developing-today-forks/opencode/pull/2190) | fix(util): return out-of-range line suffixes untouched in parsePathLineSuffix | developing-today[bot] | 2026-11-08 |
| [#2192](https://github.com/developing-today-forks/opencode/pull/2192) | fix(util): containsDirectory empty parent contains nothing                    | dezren39              | 2026-11-09 |
| [#2168](https://github.com/developing-today-forks/opencode/pull/2168) | fix(util): insert anchored pure-addition patch chunks at their @@ context     | dezren39              | 2026-11-09 |
| [#2333](https://github.com/developing-today-forks/opencode/pull/2333) | fix(util): preserve read-only mode across failed atomic replacement           | dezren39              | 2026-11-13 |

##### developing-today-forks/opencode — theme: plugins/mcp (9)

| PR                                                                    | Title                                                                             | Author                | Merged     |
| --------------------------------------------------------------------- | --------------------------------------------------------------------------------- | --------------------- | ---------- |
| [#11](https://github.com/developing-today-forks/opencode/pull/11)     | feat: built-in github tools plugin (review loops, PRs, issues, Actions, releases) | dezren39              | 2026-10-16 |
| [#48](https://github.com/developing-today-forks/opencode/pull/48)     | docs: document the OPENCODE_MCPX_* environment variables                          | dezren39              | 2026-10-18 |
| [#76](https://github.com/developing-today-forks/opencode/pull/76)     | docs: document mcpx-cleanup script for leaked daemons                             | dezren39              | 2026-10-18 |
| [#139](https://github.com/developing-today-forks/opencode/pull/139)   | ci: run script/mcpx-cleanup.test.ts in the unit job                               | dezren39              | 2026-10-19 |
| [#1505](https://github.com/developing-today-forks/opencode/pull/1505) | mcpx: re-export the daemon modules from the pinned mcpx source                    | dezren39              | 2026-11-03 |
| [#1553](https://github.com/developing-today-forks/opencode/pull/1553) | chore: bump mcpx-src (mcpx unit environment fix)                                  | developing-today[bot] | 2026-11-03 |
| [#1911](https://github.com/developing-today-forks/opencode/pull/1911) | chore: bump mcpx-src to code bcc09015 (readOnlyTools parser support)              | dezren39              | 2026-11-07 |
| [#2690](https://github.com/developing-today-forks/opencode/pull/2690) | chore: sync anomalyco/opencode dev (2026-11-23) and bump mcpx-src to code 0c9827b | dezren39              | 2026-11-23 |
| [#2694](https://github.com/developing-today-forks/opencode/pull/2694) | chore: merge anomalyco/opencode dev b705c446f0 and pin mcpx-src 2dc8cb8           | dezren39              | 2026-11-23 |

##### developing-today-forks/opencode — theme: web (8)

| PR                                                                    | Title                                                                     | Author   | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------- | -------- | ---------- |
| [#22](https://github.com/developing-today-forks/opencode/pull/22)     | docs(web): document the v2 plugin shape in plugins.mdx                    | dezren39 | 2026-10-17 |
| [#852](https://github.com/developing-today-forks/opencode/pull/852)   | fix(web): reject inherited names in docs locale lookups (#851)            | dezren39 | 2026-10-29 |
| [#903](https://github.com/developing-today-forks/opencode/pull/903)   | docs(web): document the experimental model_routes config family           | dezren39 | 2026-10-29 |
| [#1060](https://github.com/developing-today-forks/opencode/pull/1060) | docs(web): document route-window dates/excludeDates/excludeWindows        | dezren39 | 2026-10-31 |
| [#1183](https://github.com/developing-today-forks/opencode/pull/1183) | fix(web): bound the share page share_data fetch (8s deadline, 32 MiB cap) | dezren39 | 2026-11-01 |
| [#1182](https://github.com/developing-today-forks/opencode/pull/1182) | fix(web): guard empty share model parameter                               | dezren39 | 2026-11-01 |
| [#1317](https://github.com/developing-today-forks/opencode/pull/1317) | fix(web): honor Accept-Language q=0 refusals in docs locale middleware    | dezren39 | 2026-11-01 |
| [#1325](https://github.com/developing-today-forks/opencode/pull/1325) | docs(web): document canonical experimental.routes shape                   | dezren39 | 2026-11-01 |

##### developing-today-forks/opencode — theme: specs (7)

| PR                                                                    | Title                                                                                                | Author                | Merged     |
| --------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- | --------------------- | ---------- |
| [#1156](https://github.com/developing-today-forks/opencode/pull/1156) | docs(specs): design for generic OpenAI façades on the server                                         | dezren39              | 2026-11-01 |
| [#1472](https://github.com/developing-today-forks/opencode/pull/1472) | docs(specs): facade demotion section points at shipped settings, #1151 re-scoped to ledger remainder | dezren39              | 2026-11-03 |
| [#2118](https://github.com/developing-today-forks/opencode/pull/2118) | docs(specs): classify muse-direct as a rule-3 gap in the doctrine audit table (#2039)                | developing-today[bot] | 2026-11-08 |
| [#2032](https://github.com/developing-today-forks/opencode/pull/2032) | docs(specs): design voice input STT facade                                                           | dezren39              | 2026-11-09 |
| [#2033](https://github.com/developing-today-forks/opencode/pull/2033) | docs(specs): realtime audio facade inventory and design                                              | dezren39              | 2026-11-09 |
| [#2100](https://github.com/developing-today-forks/opencode/pull/2100) | docs(specs): design search providers behind the facade (#1677)                                       | dezren39              | 2026-11-09 |
| [#2784](https://github.com/developing-today-forks/opencode/pull/2784) | docs(specs): decision record for cli-rs tolerance of leaner payloads (#2770)                         | dezren39              | 2026-11-23 |

##### developing-today-forks/opencode — theme: other/no-scope (7)

| PR                                                                    | Title                                                                           | Author   | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------------- | -------- | ---------- |
| [#383](https://github.com/developing-today-forks/opencode/pull/383)   | chore: Jules sync prompt — merge only when reasonably certain                   | dezren39 | 2026-10-24 |
| [#1512](https://github.com/developing-today-forks/opencode/pull/1512) | opencode: services.opencode modules for the standing server                     | dezren39 | 2026-11-03 |
| [#1659](https://github.com/developing-today-forks/opencode/pull/1659) | chore: point functional references at developing-today-forks after org move     | dezren39 | 2026-11-05 |
| [#2275](https://github.com/developing-today-forks/opencode/pull/2275) | fix: materialize tool-result file URIs                                          | dezren39 | 2026-11-12 |
| [#2452](https://github.com/developing-today-forks/opencode/pull/2452) | core: wire the render-card seam into the live runner (#2384 slices 2-3)         | dezren39 | 2026-11-16 |
| [#2683](https://github.com/developing-today-forks/opencode/pull/2683) | chore: sync anomalyco/opencode dev (2026-11-22)                                 | dezren39 | 2026-11-22 |
| [#2691](https://github.com/developing-today-forks/opencode/pull/2691) | chore: merge anomalyco/opencode dev 04a10583c8 (video generation on by default) | dezren39 | 2026-11-23 |

##### developing-today-forks/opencode — theme: deps (7)

| PR                                                                    | Title                                                                                                              | Author                | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ | --------------------- | ---------- |
| [#214](https://github.com/developing-today-forks/opencode/pull/214)   | fix(deps): bump vite to 8.2.4 in app and desktop, rebase vite patch                                                | dezren39              | 2026-10-21 |
| [#258](https://github.com/developing-today-forks/opencode/pull/258)   | chore(deps): bump storybook family to 10.6.2 and group its dependabot updates                                      | dezren39              | 2026-10-22 |
| [#475](https://github.com/developing-today-forks/opencode/pull/475)   | chore(deps): bump dompurify from 3.4.14 to 3.4.17 in /packages/session-ui                                          | dependabot[bot]       | 2026-10-26 |
| [#476](https://github.com/developing-today-forks/opencode/pull/476)   | chore(deps): bump astro from 7.1.3 to 7.3.3 in /services/www                                                       | dependabot[bot]       | 2026-10-26 |
| [#1378](https://github.com/developing-today-forks/opencode/pull/1378) | fix(deps): consolidate glob 14 and minimatch 11 in packages/util                                                   | dezren39              | 2026-11-02 |
| [#1783](https://github.com/developing-today-forks/opencode/pull/1783) | fix(deps): regenerate the solid-js patch for Bun 1.4.4 — unbreaks install, nix build and nix-hashes (#1782, #1698) | developing-today[bot] | 2026-11-05 |
| [#2357](https://github.com/developing-today-forks/opencode/pull/2357) | build(deps): bump @astrojs/solid-js from 5.1.0 to 7.1.2 in /packages/web                                           | dezren39              | 2026-11-13 |

##### developing-today-forks/opencode — theme: quota (6)

| PR                                                                    | Title                                                                                                         | Author                | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- | --------------------- | ---------- |
| [#1743](https://github.com/developing-today-forks/opencode/pull/1743) | fix(quota): publish over-limit Turso windows above 100 percent (code#6227)                                    | developing-today[bot] | 2026-11-05 |
| [#1751](https://github.com/developing-today-forks/opencode/pull/1751) | fix(quota): meta portal parse must not require window_resets_at                                               | dezren39              | 2026-11-05 |
| [#2040](https://github.com/developing-today-forks/opencode/pull/2040) | fix(quota): Mistral CDP budget + probe-failure error rows (#1729)                                             | dezren39              | 2026-11-08 |
| [#2153](https://github.com/developing-today-forks/opencode/pull/2153) | fix(quota): resolve Turso plan limits dynamically from /plans                                                 | dezren39              | 2026-11-08 |
| [#2566](https://github.com/developing-today-forks/opencode/pull/2566) | feat(quota): google-free + exaone quota legs (R33)                                                            | dezren39              | 2026-11-17 |
| [#2737](https://github.com/developing-today-forks/opencode/pull/2737) | fix(quota): land per-leg deadline isolation for fetchAllQuota + end-to-end leg test (#2562, supersedes #2546) | dezren39              | 2026-11-23 |

##### developing-today-forks/opencode — theme: facade (6)

| PR                                                                    | Title                                                                                         | Author                | Merged     |
| --------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- | --------------------- | ---------- |
| [#1445](https://github.com/developing-today-forks/opencode/pull/1445) | test(facade): door vs façade parity suite (#1152)                                             | dezren39              | 2026-11-02 |
| [#1447](https://github.com/developing-today-forks/opencode/pull/1447) | fix(facade): surface quota failures as 429 with the reset hint (#1441)                        | dezren39              | 2026-11-02 |
| [#1455](https://github.com/developing-today-forks/opencode/pull/1455) | test(facade): claude-code door parity pairs — recorded replay for :18749 (#1412, #1152)       | dezren39              | 2026-11-02 |
| [#1456](https://github.com/developing-today-forks/opencode/pull/1456) | docs(facade): correct the claude-code continuity note — live attempt did not complete (#1412) | dezren39              | 2026-11-02 |
| [#1452](https://github.com/developing-today-forks/opencode/pull/1452) | test(facade): commandcode door parity pairs (#1411, #1152)                                    | dezren39              | 2026-11-02 |
| [#1465](https://github.com/developing-today-forks/opencode/pull/1465) | fix(facade): advertise configured model_routes at /v1/models, not a hardcoded list (#1413)    | developing-today[bot] | 2026-11-03 |

##### developing-today-forks/opencode — theme: plugin-github (5)

| PR                                                                    | Title                                                                               | Author   | Merged     |
| --------------------------------------------------------------------- | ----------------------------------------------------------------------------------- | -------- | ---------- |
| [#174](https://github.com/developing-today-forks/opencode/pull/174)   | test(plugin-github): make files and actions tests pass on Windows                   | dezren39 | 2026-10-20 |
| [#1320](https://github.com/developing-today-forks/opencode/pull/1320) | fix(plugin-github): guard comment mutation lookups                                  | dezren39 | 2026-11-01 |
| [#2715](https://github.com/developing-today-forks/opencode/pull/2715) | fix(plugin-github): enforce upload size limits while reading local files            | dezren39 | 2026-11-23 |
| [#2720](https://github.com/developing-today-forks/opencode/pull/2720) | fix(plugin-github): classify local paths by canonical location                      | dezren39 | 2026-11-23 |
| [#2726](https://github.com/developing-today-forks/opencode/pull/2726) | fix(plugin-github): authorizeRead returns the canonical path for an existing target | dezren39 | 2026-11-23 |

##### developing-today-forks/opencode — theme: function (5)

| PR                                                                    | Title                                                                                | Author                | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------------------ | --------------------- | ---------- |
| [#1120](https://github.com/developing-today-forks/opencode/pull/1120) | fix(function): bound Discord support-relay fetch — 5s timeout + 8 KiB error-body cap | dezren39              | 2026-11-01 |
| [#1609](https://github.com/developing-today-forks/opencode/pull/1609) | fix(function): share one remote JWK set across token exchanges (#1112)               | dezren39              | 2026-11-04 |
| [#2193](https://github.com/developing-today-forks/opencode/pull/2193) | fix(function): key the githubActionsJWKSet cache by options as well as URL           | developing-today[bot] | 2026-11-09 |
| [#2593](https://github.com/developing-today-forks/opencode/pull/2593) | fix(function): reject share_sync writes when the share secret is unset or missing    | dezren39              | 2026-11-17 |
| [#2599](https://github.com/developing-today-forks/opencode/pull/2599) | fix(function): verify /feishu webhook requests (#2564)                               | dezren39              | 2026-11-17 |

##### developing-today-forks/opencode — theme: core,ai (5)

| PR                                                                    | Title                                                                                   | Author   | Merged     |
| --------------------------------------------------------------------- | --------------------------------------------------------------------------------------- | -------- | ---------- |
| [#716](https://github.com/developing-today-forks/opencode/pull/716)   | fix(core,ai): calendar-check both retry-after date branches (#713)                      | dezren39 | 2026-10-28 |
| [#820](https://github.com/developing-today-forks/opencode/pull/820)   | fix(core,ai): handle prototype-named rate-limit windows                                 | dezren39 | 2026-10-29 |
| [#926](https://github.com/developing-today-forks/opencode/pull/926)   | fix(core,ai): cap unbounded download bodies in media, skill, ripgrep and CodeMode fetch | dezren39 | 2026-11-01 |
| [#2249](https://github.com/developing-today-forks/opencode/pull/2249) | fix(core,ai): apply well-formedness guard at remaining truncation sites (#1977)         | dezren39 | 2026-11-10 |
| [#2403](https://github.com/developing-today-forks/opencode/pull/2403) | fix(core,ai): clear post-#2398 main test failures (#2392)                               | dezren39 | 2026-11-13 |

##### developing-today-forks/opencode — theme: codemode (4)

| PR                                                                    | Title                                                                     | Author   | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------- | -------- | ---------- |
| [#812](https://github.com/developing-today-forks/opencode/pull/812)   | feat(codemode): versioned libraries and std v1 pure helpers               | dezren39 | 2026-10-29 |
| [#872](https://github.com/developing-today-forks/opencode/pull/872)   | feat(codemode): std async helpers on a capped sleep primitive             | dezren39 | 2026-10-29 |
| [#861](https://github.com/developing-today-forks/opencode/pull/861)   | fix(codemode): std.pick copies only own properties (#839)                 | dezren39 | 2026-10-29 |
| [#2381](https://github.com/developing-today-forks/opencode/pull/2381) | fix(codemode): block Code Mode fetches to private network targets (#2310) | dezren39 | 2026-11-13 |

##### developing-today-forks/opencode — theme: providers (4)

| PR                                                                    | Title                                                                     | Author   | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------- | -------- | ---------- |
| [#1832](https://github.com/developing-today-forks/opencode/pull/1832) | Provider availability: classification + stop/resume state machine (#1828) | dezren39 | 2026-11-06 |
| [#1872](https://github.com/developing-today-forks/opencode/pull/1872) | muse-direct: native Meta wire provider (#1825)                            | dezren39 | 2026-11-06 |
| [#1944](https://github.com/developing-today-forks/opencode/pull/1944) | Rename fleet-built providers to leading-underscore ids (#1823)            | dezren39 | 2026-11-07 |
| [#2038](https://github.com/developing-today-forks/opencode/pull/2038) | docs: record the *-direct provider emulation doctrine in specs (#1717)    | dezren39 | 2026-11-08 |

##### developing-today-forks/opencode — theme: routes (4)

| PR                                                                    | Title                                                                                                       | Author   | Merged     |
| --------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#1043](https://github.com/developing-today-forks/opencode/pull/1043) | feat(routes): date-aware route windows — dates/excludeDates + excludeWindows                                | dezren39 | 2026-10-31 |
| [#1142](https://github.com/developing-today-forks/opencode/pull/1142) | feat(routes): experimental.model_route_settings — route machinery constants as config (#1141)               | dezren39 | 2026-11-01 |
| [#1453](https://github.com/developing-today-forks/opencode/pull/1453) | feat(routes): routes carry a system prompt + façade route failover — persona routes for design-code (#1413) | dezren39 | 2026-11-02 |
| [#1971](https://github.com/developing-today-forks/opencode/pull/1971) | fix(routes): nested-route children weighed by bare route id (weights were silently 1)                       | dezren39 | 2026-11-08 |

##### developing-today-forks/opencode — theme: www (4)

| PR                                                                    | Title                                                                                                 | Author   | Merged     |
| --------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#836](https://github.com/developing-today-forks/opencode/pull/836)   | fix(www): regenerate services/www/openapi.json from protocol                                          | dezren39 | 2026-10-29 |
| [#1273](https://github.com/developing-today-forks/opencode/pull/1273) | fix(www): add deadline to /install proxy fetch (#1246)                                                | dezren39 | 2026-11-01 |
| [#1499](https://github.com/developing-today-forks/opencode/pull/1499) | fix(www): regenerate OpenAPI documents for completions façade and route-settings descriptions (#1498) | dezren39 | 2026-11-03 |
| [#1596](https://github.com/developing-today-forks/opencode/pull/1596) | fix(www): regenerate OpenAPI copies from protocol (#1593)                                             | dezren39 | 2026-11-04 |

##### developing-today-forks/opencode — theme: protocol (3)

| PR                                                                    | Title                                                                   | Author   | Merged     |
| --------------------------------------------------------------------- | ----------------------------------------------------------------------- | -------- | ---------- |
| [#663](https://github.com/developing-today-forks/opencode/pull/663)   | docs(protocol): regenerate OpenAPI documents for PR #596 provider shape | dezren39 | 2026-10-28 |
| [#1628](https://github.com/developing-today-forks/opencode/pull/1628) | fix(protocol): regenerate OpenAPI documents                             | dezren39 | 2026-11-04 |
| [#2761](https://github.com/developing-today-forks/opencode/pull/2761) | fix(protocol): regenerate openapi.json — drift gate green again         | dezren39 | 2026-11-23 |

##### developing-today-forks/opencode — theme: plugin-browser (3)

| PR                                                                    | Title                                                                                   | Author   | Merged     |
| --------------------------------------------------------------------- | --------------------------------------------------------------------------------------- | -------- | ---------- |
| [#2606](https://github.com/developing-today-forks/opencode/pull/2606) | fix(plugin-browser): authorize server-local file uploads by project containment (#2289) | dezren39 | 2026-11-17 |
| [#2744](https://github.com/developing-today-forks/opencode/pull/2744) | fix(plugin-browser): bind upload reads to the checked file object                       | dezren39 | 2026-11-23 |
| [#2753](https://github.com/developing-today-forks/opencode/pull/2753) | test(plugin-browser): prove upload byte limits hold during reads                        | dezren39 | 2026-11-23 |

##### developing-today-forks/opencode — theme: config (3)

| PR                                                                    | Title                                                                                                         | Author   | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#1108](https://github.com/developing-today-forks/opencode/pull/1108) | fix(config): auto-apply model and default_agent edits without reload                                          | dezren39 | 2026-10-31 |
| [#1277](https://github.com/developing-today-forks/opencode/pull/1277) | fix(config): provider_routes dropped by config normalization — provider-route groups never candidated (#1134) | dezren39 | 2026-11-01 |
| [#2730](https://github.com/developing-today-forks/opencode/pull/2730) | fix(config): name alias cycles in config diagnostics and runtime model errors                                 | dezren39 | 2026-11-23 |

##### developing-today-forks/opencode — theme: flake (3)

| PR                                                                    | Title                                                        | Author   | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------------ | -------- | ---------- |
| [#2637](https://github.com/developing-today-forks/opencode/pull/2637) | chore(flake): bump mcpx-src to developing-today/code d3a0db8 | dezren39 | 2026-11-18 |
| [#2678](https://github.com/developing-today-forks/opencode/pull/2678) | chore(flake): bump mcpx-src to developing-today/code 16ffdfd | dezren39 | 2026-11-22 |
| [#2688](https://github.com/developing-today-forks/opencode/pull/2688) | chore(flake): bump mcpx-src to developing-today/code 6f4696b | dezren39 | 2026-11-23 |

##### developing-today-forks/opencode — theme: routes/pools (3)

| PR                                                                    | Title                                                                                                       | Author   | Merged     |
| --------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#975](https://github.com/developing-today-forks/opencode/pull/975)   | ci: route workflow escalation through pm-dispatch                                                           | dezren39 | 2026-10-30 |
| [#1311](https://github.com/developing-today-forks/opencode/pull/1311) | config: nest routing config under experimental.routes with flat-key compat shim (#1140)                     | dezren39 | 2026-11-01 |
| [#2265](https://github.com/developing-today-forks/opencode/pull/2265) | Attach containing route to direct model resolutions so startup provider errors fail over (code#7073 step 2) | dezren39 | 2026-11-11 |

##### developing-today-forks/opencode — theme: nix/packaging (3)

| PR                                                                    | Title                                                                | Author   | Merged     |
| --------------------------------------------------------------------- | -------------------------------------------------------------------- | -------- | ---------- |
| [#1516](https://github.com/developing-today-forks/opencode/pull/1516) | nix: refresh node_modules patch set for the current lockfile         | dezren39 | 2026-11-03 |
| [#1523](https://github.com/developing-today-forks/opencode/pull/1523) | nix: point the R2 cache substituter at the renamed workers subdomain | dezren39 | 2026-11-03 |
| [#1880](https://github.com/developing-today-forks/opencode/pull/1880) | nix: regenerate x86_64-linux nodeModules hash                        | dezren39 | 2026-11-06 |

##### developing-today-forks/opencode — theme: console,stats (3)

| PR                                                                    | Title                                                                                        | Author   | Merged     |
| --------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#144](https://github.com/developing-today-forks/opencode/pull/144)   | test(console,stats): run console-app, stats-app and stats-core tests via turbo               | dezren39 | 2026-10-20 |
| [#1096](https://github.com/developing-today-forks/opencode/pull/1096) | fix(console,stats): bound GitHub metadata fetches (timeouts + body caps)                     | dezren39 | 2026-10-31 |
| [#1139](https://github.com/developing-today-forks/opencode/pull/1139) | fix(console,stats): bound EmailOctopus subscribe fetches — 5s timeout + 8 KiB error-body cap | dezren39 | 2026-11-01 |

##### developing-today-forks/opencode — theme: deps-dev (3)

| PR                                                                  | Title                                                                            | Author          | Merged     |
| ------------------------------------------------------------------- | -------------------------------------------------------------------------------- | --------------- | ---------- |
| [#477](https://github.com/developing-today-forks/opencode/pull/477) | chore(deps-dev): bump electron from 44.4.5 to 44.9.1 in /packages/gui-extensions | dependabot[bot] | 2026-10-26 |
| [#494](https://github.com/developing-today-forks/opencode/pull/494) | chore(deps-dev): bump electron from 44.4.5 to 44.9.1 in /packages/desktop        | dependabot[bot] | 2026-10-26 |
| [#493](https://github.com/developing-today-forks/opencode/pull/493) | chore(deps-dev): bump happy-dom from 20.11.1 to 21.0.4 in /packages/app          | dependabot[bot] | 2026-10-26 |

##### developing-today-forks/opencode — theme: legacy (2)

| PR                                                                    | Title                                                                             | Author   | Merged     |
| --------------------------------------------------------------------- | --------------------------------------------------------------------------------- | -------- | ---------- |
| [#2773](https://github.com/developing-today-forks/opencode/pull/2773) | test(legacy): rewrite share suite against the surviving shim, delete feishu suite | dezren39 | 2026-11-23 |
| [#2779](https://github.com/developing-today-forks/opencode/pull/2779) | fix(legacy): cap /api/share JSON body at 16 KiB                                   | dezren39 | 2026-11-23 |

##### developing-today-forks/opencode — theme: session (2)

| PR                                                                    | Title                                                                        | Author   | Merged     |
| --------------------------------------------------------------------- | ---------------------------------------------------------------------------- | -------- | ---------- |
| [#2754](https://github.com/developing-today-forks/opencode/pull/2754) | feat(session): surface route_usage.lastServing on the session record (#2696) | dezren39 | 2026-11-23 |
| [#2760](https://github.com/developing-today-forks/opencode/pull/2760) | feat(session): admit shell and command inputs durably through the inbox      | dezren39 | 2026-11-23 |

##### developing-today-forks/opencode — theme: session-ui (2)

| PR                                                                    | Title                                                                      | Author   | Merged     |
| --------------------------------------------------------------------- | -------------------------------------------------------------------------- | -------- | ---------- |
| [#2524](https://github.com/developing-today-forks/opencode/pull/2524) | test(session-ui): pin AttachmentCard button and div rendering (#2513)      | dezren39 | 2026-11-23 |
| [#2755](https://github.com/developing-today-forks/opencode/pull/2755) | fix(session-ui): name the line-comment textarea with its selection context | dezren39 | 2026-11-23 |

##### developing-today-forks/opencode — theme: core,server (2)

| PR                                                                    | Title                                                                                    | Author   | Merged     |
| --------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- | -------- | ---------- |
| [#2390](https://github.com/developing-today-forks/opencode/pull/2390) | feat(core,server): STT façade route + registry skeleton, OpenAI end-to-end (#2385)       | dezren39 | 2026-11-13 |
| [#2412](https://github.com/developing-today-forks/opencode/pull/2412) | fix(core,server): clear the masked unit layer behind the numeric-boundaries gate (#2405) | dezren39 | 2026-11-13 |

##### developing-today-forks/opencode — theme: util,core (2)

| PR                                                                    | Title                                                                                                    | Author   | Merged     |
| --------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#1136](https://github.com/developing-today-forks/opencode/pull/1136) | feat(util,core): shared bounded-call pair in @opencode/util (design #1127 phase 1)                       | dezren39 | 2026-11-01 |
| [#2239](https://github.com/developing-today-forks/opencode/pull/2239) | refactor(util,core): truncation well-formedness primitive in @opencode/util, core Text re-export (#2209) | dezren39 | 2026-11-10 |

##### developing-today-forks/opencode — theme: ui,gui-extensions (2)

| PR                                                                    | Title                                                                                   | Author   | Merged     |
| --------------------------------------------------------------------- | --------------------------------------------------------------------------------------- | -------- | ---------- |
| [#1062](https://github.com/developing-today-forks/opencode/pull/1062) | fix(ui,gui-extensions): bound remote JSON fetches in theme loader and whats-new (#1061) | dezren39 | 2026-10-31 |
| [#1598](https://github.com/developing-today-forks/opencode/pull/1598) | fix(ui,gui-extensions): add named deadlines to theme and whats-new fetches (#1081)      | dezren39 | 2026-11-04 |

##### developing-today-forks/opencode — theme: lint (2)

| PR                                                                    | Title                                                                                   | Author   | Merged     |
| --------------------------------------------------------------------- | --------------------------------------------------------------------------------------- | -------- | ---------- |
| [#155](https://github.com/developing-today-forks/opencode/pull/155)   | test(lint): run ast-grep lint rule tests in CI                                          | dezren39 | 2026-10-20 |
| [#1576](https://github.com/developing-today-forks/opencode/pull/1576) | fix(lint): disposition all 33 lint:effect-patterns violations and wire the scan into CI | dezren39 | 2026-11-04 |

##### developing-today-forks/opencode — theme: core,schema (2)

| PR                                                                    | Title                                                              | Author   | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------------------ | -------- | ---------- |
| [#379](https://github.com/developing-today-forks/opencode/pull/379)   | feat(core,schema): implement model, provider, and effort aliases   | dezren39 | 2026-10-24 |
| [#1537](https://github.com/developing-today-forks/opencode/pull/1537) | fix(core,schema): validate worktree names as a single path segment | dezren39 | 2026-11-03 |

##### developing-today-forks/opencode — theme: http-recorder (2)

| PR                                                                    | Title                                                                                | Author   | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------------------ | -------- | ---------- |
| [#1374](https://github.com/developing-today-forks/opencode/pull/1374) | fix(http-recorder): conform cassette store append to atomic-replace contract (#1373) | dezren39 | 2026-11-02 |
| [#1433](https://github.com/developing-today-forks/opencode/pull/1433) | fix(http-recorder): clean partial cassette temps                                     | dezren39 | 2026-11-02 |

##### developing-today-forks/opencode — theme: stats (2)

| PR                                                                    | Title                                              | Author   | Merged     |
| --------------------------------------------------------------------- | -------------------------------------------------- | -------- | ---------- |
| [#1087](https://github.com/developing-today-forks/opencode/pull/1087) | fix(stats): bound catalog fetches in time and size | dezren39 | 2026-10-31 |
| [#1383](https://github.com/developing-today-forks/opencode/pull/1383) | fix(stats): bound the R2 SQL response body read    | dezren39 | 2026-11-02 |

##### developing-today-forks/opencode — theme: changelog (2)

| PR                                                                    | Title                                                                       | Author   | Merged     |
| --------------------------------------------------------------------- | --------------------------------------------------------------------------- | -------- | ---------- |
| [#122](https://github.com/developing-today-forks/opencode/pull/122)   | chore(changelog): untrack Wrangler local state, gitignore .wrangler/ (#120) | dezren39 | 2026-10-19 |
| [#1272](https://github.com/developing-today-forks/opencode/pull/1272) | fix(changelog): read upstream changelog through a 1 MiB capped reader       | dezren39 | 2026-11-01 |

##### developing-today-forks/opencode — theme: cli,tui (2)

| PR                                                                    | Title                                                                              | Author   | Merged     |
| --------------------------------------------------------------------- | ---------------------------------------------------------------------------------- | -------- | ---------- |
| [#846](https://github.com/developing-today-forks/opencode/pull/846)   | fix(cli,tui): parse interactive form numeric answers with a strict decimal grammar | dezren39 | 2026-10-29 |
| [#1066](https://github.com/developing-today-forks/opencode/pull/1066) | fix(cli,tui): render numeric form defaults in plain decimal                        | dezren39 | 2026-10-31 |

##### developing-today-forks/opencode — theme: client,httpapi-codegen (2)

| PR                                                                    | Title                                                                                     | Author   | Merged     |
| --------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- | -------- | ---------- |
| [#1008](https://github.com/developing-today-forks/opencode/pull/1008) | fix(client,httpapi-codegen): bound generated client json() response body with a named cap | dezren39 | 2026-10-30 |
| [#1016](https://github.com/developing-today-forks/opencode/pull/1016) | fix(client,httpapi-codegen): cap the generated binary response body                       | dezren39 | 2026-10-30 |

##### developing-today-forks/opencode — theme: plugin (2)

| PR                                                                  | Title                                                                           | Author   | Merged     |
| ------------------------------------------------------------------- | ------------------------------------------------------------------------------- | -------- | ---------- |
| [#16](https://github.com/developing-today-forks/opencode/pull/16)   | fix(plugin): fall through on Bun ResolveMessage when probing plugin entrypoints | dezren39 | 2026-10-18 |
| [#766](https://github.com/developing-today-forks/opencode/pull/766) | feat(plugin): expose child session listing and server URL to v2 plugins         | dezren39 | 2026-10-29 |

##### developing-today-forks/opencode — theme: sdk (2)

| PR                                                                  | Title                                                                     | Author   | Merged     |
| ------------------------------------------------------------------- | ------------------------------------------------------------------------- | -------- | ---------- |
| [#158](https://github.com/developing-today-forks/opencode/pull/158) | test(sdk): expect the protocol SessionNotFoundError tag from sessions.get | dezren39 | 2026-10-20 |
| [#166](https://github.com/developing-today-forks/opencode/pull/166) | fix(sdk): pack plugin-github in verify-package pack list                  | dezren39 | 2026-10-20 |

##### developing-today-forks/opencode — theme: cli-rs (1)

| PR                                                                    | Title                                                                                                        | Author   | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ | -------- | ---------- |
| [#2768](https://github.com/developing-today-forks/opencode/pull/2768) | fix(cli-rs): api-types generator — optional-and-nullable numbers and keyword literal variants (#2733, #2738) | dezren39 | 2026-11-23 |

##### developing-today-forks/opencode — theme: ai/quota (1)

| PR                                                                    | Title                                                                          | Author   | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------------ | -------- | ---------- |
| [#2763](https://github.com/developing-today-forks/opencode/pull/2763) | fix(ai/quota): carry OAuth rejection facts through antigravity refresh (#2693) | dezren39 | 2026-11-23 |

##### developing-today-forks/opencode — theme: tui,codemode,ci (1)

| PR                                                                    | Title                                                                                 | Author   | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------------------- | -------- | ---------- |
| [#2740](https://github.com/developing-today-forks/opencode/pull/2740) | fix(tui,codemode,ci): consolidated voice-input bounds + fake-clock sleep test (#2537) | dezren39 | 2026-11-23 |

##### developing-today-forks/opencode — theme: update (1)

| PR                                                                    | Title                                                                       | Author   | Merged     |
| --------------------------------------------------------------------- | --------------------------------------------------------------------------- | -------- | ---------- |
| [#2734](https://github.com/developing-today-forks/opencode/pull/2734) | fix(update): bound publish/admin request bodies with a shared capped reader | dezren39 | 2026-11-23 |

##### developing-today-forks/opencode — theme: ui (1)

| PR                                                                    | Title                                                                        | Author   | Merged     |
| --------------------------------------------------------------------- | ---------------------------------------------------------------------------- | -------- | ---------- |
| [#2725](https://github.com/developing-today-forks/opencode/pull/2725) | test(ui): bind the transpiler's hashed jsx runtime names in the test preload | dezren39 | 2026-11-23 |

##### developing-today-forks/opencode — theme: a11y (1)

| PR                                                                    | Title                                                    | Author   | Merged     |
| --------------------------------------------------------------------- | -------------------------------------------------------- | -------- | ---------- |
| [#2714](https://github.com/developing-today-forks/opencode/pull/2714) | fix(a11y): expose line-comment mention list as a listbox | dezren39 | 2026-11-23 |

##### developing-today-forks/opencode — theme: resolver (1)

| PR                                                                    | Title                                                                    | Author   | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------ | -------- | ---------- |
| [#2697](https://github.com/developing-today-forks/opencode/pull/2697) | feat(resolver): key the within-call tried set by effort (#10367 point 1) | dezren39 | 2026-11-23 |

##### developing-today-forks/opencode — theme: stats-app (1)

| PR                                                                    | Title                                                          | Author   | Merged     |
| --------------------------------------------------------------------- | -------------------------------------------------------------- | -------- | ---------- |
| [#2569](https://github.com/developing-today-forks/opencode/pull/2569) | fix(stats-app): bump nitro off advisory-affected 3.0.1-alpha.1 | dezren39 | 2026-11-17 |

##### developing-today-forks/opencode — theme: console-app (1)

| PR                                                                    | Title                                                               | Author   | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------------------- | -------- | ---------- |
| [#2551](https://github.com/developing-today-forks/opencode/pull/2551) | fix(console-app): bump nitro to 3.0.260429-beta past advisory range | dezren39 | 2026-11-16 |

##### developing-today-forks/opencode — theme: astro (1)

| PR                                                                    | Title                                                     | Author   | Merged     |
| --------------------------------------------------------------------- | --------------------------------------------------------- | -------- | ---------- |
| [#2523](https://github.com/developing-today-forks/opencode/pull/2523) | chore(astro): lock-rescue Dependabot bumps to astro 7.4.7 | dezren39 | 2026-11-16 |

##### developing-today-forks/opencode — theme: ci,core (1)

| PR                                                                    | Title                                                                               | Author   | Merged     |
| --------------------------------------------------------------------- | ----------------------------------------------------------------------------------- | -------- | ---------- |
| [#2507](https://github.com/developing-today-forks/opencode/pull/2507) | fix(ci,core): TursoDB engine coverage in CI plus layout-aware sidecar modes (#2505) | dezren39 | 2026-11-23 |

##### developing-today-forks/opencode — theme: app,session-ui (1)

| PR                                                                    | Title                                                             | Author   | Merged     |
| --------------------------------------------------------------------- | ----------------------------------------------------------------- | -------- | ---------- |
| [#2489](https://github.com/developing-today-forks/opencode/pull/2489) | fix(app,session-ui): make attachment previews keyboard accessible | dezren39 | 2026-11-16 |

##### developing-today-forks/opencode — theme: quota/plans (1)

| PR                                                                    | Title                                                                                       | Author   | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#2450](https://github.com/developing-today-forks/opencode/pull/2450) | core: render-card seam for tier chooser + quota fallback cards (#2384 slices 2-3, design A) | dezren39 | 2026-11-15 |

##### developing-today-forks/opencode — theme: core,server,tui (1)

| PR                                                                    | Title                                                                                        | Author   | Merged     |
| --------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#2431](https://github.com/developing-today-forks/opencode/pull/2431) | feat(core,server,tui): STT façade streaming partials — SSE TranscriptionEvent frames (#2388) | dezren39 | 2026-11-14 |

##### developing-today-forks/opencode — theme: cost (1)

| PR                                                                    | Title                                                                              | Author   | Merged     |
| --------------------------------------------------------------------- | ---------------------------------------------------------------------------------- | -------- | ---------- |
| [#2316](https://github.com/developing-today-forks/opencode/pull/2316) | feat(cost): authoritative route_attempt cost on Session.Info + header display (#7) | dezren39 | 2026-11-12 |

##### developing-today-forks/opencode — theme: util,tui (1)

| PR                                                                    | Title                                                                        | Author   | Merged     |
| --------------------------------------------------------------------- | ---------------------------------------------------------------------------- | -------- | ---------- |
| [#2240](https://github.com/developing-today-forks/opencode/pull/2240) | fix(util,tui): display truncations compose the shared pair primitive (#2133) | dezren39 | 2026-11-10 |

##### developing-today-forks/opencode — theme: test (1)

| PR                                                                    | Title                                                                         | Author   | Merged     |
| --------------------------------------------------------------------- | ----------------------------------------------------------------------------- | -------- | ---------- |
| [#1965](https://github.com/developing-today-forks/opencode/pull/1965) | fix(test): compile Solid JSX in Bun test preload so suites run on current Bun | dezren39 | 2026-11-08 |

##### developing-today-forks/opencode — theme: tool (1)

| PR                                                                    | Title                                                                   | Author   | Merged     |
| --------------------------------------------------------------------- | ----------------------------------------------------------------------- | -------- | ---------- |
| [#1910](https://github.com/developing-today-forks/opencode/pull/1910) | docs(tool): question description warns against detached/subsessions use | dezren39 | 2026-11-08 |

##### developing-today-forks/opencode — theme: vibe (1)

| PR                                                                    | Title                                                       | Author   | Merged     |
| --------------------------------------------------------------------- | ----------------------------------------------------------- | -------- | ---------- |
| [#1808](https://github.com/developing-today-forks/opencode/pull/1808) | docs(vibe): billing follows the key, not the metadata stamp | dezren39 | 2026-11-06 |

##### developing-today-forks/opencode — theme: pkg (1)

| PR                                                                    | Title                                                                   | Author                | Merged     |
| --------------------------------------------------------------------- | ----------------------------------------------------------------------- | --------------------- | ---------- |
| [#1786](https://github.com/developing-today-forks/opencode/pull/1786) | fix(pkg): make the packaged binary load the native-onnx backend (#1727) | developing-today[bot] | 2026-11-05 |

##### developing-today-forks/opencode — theme: app,tui (1)

| PR                                                                    | Title                                                      | Author   | Merged     |
| --------------------------------------------------------------------- | ---------------------------------------------------------- | -------- | ---------- |
| [#1686](https://github.com/developing-today-forks/opencode/pull/1686) | feat(app,tui): wildcard search in the model pickers (#230) | dezren39 | 2026-11-05 |

##### developing-today-forks/opencode — theme: files (1)

| PR                                                                    | Title                                                                             | Author   | Merged     |
| --------------------------------------------------------------------- | --------------------------------------------------------------------------------- | -------- | ---------- |
| [#1649](https://github.com/developing-today-forks/opencode/pull/1649) | fix(files): ignore invalid Range sets per RFC 9110 instead of serving 416 (#1646) | dezren39 | 2026-11-05 |

##### developing-today-forks/opencode — theme: tests (1)

| PR                                                                    | Title                                                       | Author   | Merged     |
| --------------------------------------------------------------------- | ----------------------------------------------------------- | -------- | ---------- |
| [#1648](https://github.com/developing-today-forks/opencode/pull/1648) | docs: add testing doctrine (PBT, MBT, oracles) to AGENTS.md | dezren39 | 2026-11-04 |

##### developing-today-forks/opencode — theme: cli,server (1)

| PR                                                                    | Title                                                                                              | Author   | Merged     |
| --------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#1618](https://github.com/developing-today-forks/opencode/pull/1618) | fix(cli,server): cap stats --days at the service span and report over-wide ranges honestly (#1000) | dezren39 | 2026-11-04 |

##### developing-today-forks/opencode — theme: ai,tui,cli (1)

| PR                                                                    | Title                                                                                | Author   | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------------------ | -------- | ---------- |
| [#1614](https://github.com/developing-today-forks/opencode/pull/1614) | test(ai,tui,cli): pin the named-form calendar check in the ai/tui/cli copies (#1003) | dezren39 | 2026-11-04 |

##### developing-today-forks/opencode — theme: httpapi-codegen (1)

| PR                                                                    | Title                                                              | Author   | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------------------ | -------- | ---------- |
| [#1604](https://github.com/developing-today-forks/opencode/pull/1604) | fix(httpapi-codegen): support imported nonportable success schemas | dezren39 | 2026-11-05 |

##### developing-today-forks/opencode — theme: mcpx (1)

| PR                                                                    | Title                                       | Author                | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------- | --------------------- | ---------- |
| [#1546](https://github.com/developing-today-forks/opencode/pull/1546) | chore(mcpx): bump mcpx-src to code d51d7bef | developing-today[bot] | 2026-11-03 |

##### developing-today-forks/opencode — theme: github (1)

| PR                                                                    | Title                                                  | Author   | Merged     |
| --------------------------------------------------------------------- | ------------------------------------------------------ | -------- | ---------- |
| [#1459](https://github.com/developing-today-forks/opencode/pull/1459) | fix(github): bound action HTTP calls and scan the tree | dezren39 | 2026-11-08 |

##### developing-today-forks/opencode — theme: storage (1)

| PR                                                                    | Title                                                      | Author   | Merged     |
| --------------------------------------------------------------------- | ---------------------------------------------------------- | -------- | ---------- |
| [#1363](https://github.com/developing-today-forks/opencode/pull/1363) | refactor(storage): conform sync atomic writers to contract | dezren39 | 2026-11-02 |

##### developing-today-forks/opencode — theme: auth (1)

| PR                                                                    | Title                                                                      | Author   | Merged     |
| --------------------------------------------------------------------- | -------------------------------------------------------------------------- | -------- | ---------- |
| [#1145](https://github.com/developing-today-forks/opencode/pull/1145) | fix(auth): classify token-sharing scopes-401 as credential-method mismatch | dezren39 | 2026-11-01 |

##### developing-today-forks/opencode — theme: latex (1)

| PR                                                                  | Title                                                         | Author   | Merged     |
| ------------------------------------------------------------------- | ------------------------------------------------------------- | -------- | ---------- |
| [#939](https://github.com/developing-today-forks/opencode/pull/939) | fix(latex): read command dispatch tables by own property only | dezren39 | 2026-10-30 |

##### developing-today-forks/opencode — theme: cli,client (1)

| PR                                                                  | Title                                                            | Author   | Merged     |
| ------------------------------------------------------------------- | ---------------------------------------------------------------- | -------- | ---------- |
| [#877](https://github.com/developing-today-forks/opencode/pull/877) | fix(cli,client): treat blank OPENCODE_SHUTDOWN_GRACE_MS as unset | dezren39 | 2026-10-29 |

##### developing-today-forks/opencode — theme: client,app (1)

| PR                                                                  | Title                                                | Author   | Merged     |
| ------------------------------------------------------------------- | ---------------------------------------------------- | -------- | ---------- |
| [#807](https://github.com/developing-today-forks/opencode/pull/807) | fix(client,app): follow fork links for session roots | dezren39 | 2026-10-29 |

##### developing-today-forks/opencode — theme: core,ai,tui,cli (1)

| PR                                                                  | Title                                                                                    | Author   | Merged     |
| ------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- | -------- | ---------- |
| [#801](https://github.com/developing-today-forks/opencode/pull/801) | fix(core,ai,tui,cli): calendar-check non-ISO date strings in all six date parsers (#798) | dezren39 | 2026-10-29 |

##### developing-today-forks/opencode — theme: core,app (1)

| PR                                                                  | Title                                                         | Author   | Merged     |
| ------------------------------------------------------------------- | ------------------------------------------------------------- | -------- | ---------- |
| [#735](https://github.com/developing-today-forks/opencode/pull/735) | fix(core,app): plain-decimal grammar for file-URL line ranges | dezren39 | 2026-10-28 |

##### developing-today-forks/opencode — theme: tui,cli (1)

| PR                                                                  | Title                                                       | Author   | Merged     |
| ------------------------------------------------------------------- | ----------------------------------------------------------- | -------- | ---------- |
| [#730](https://github.com/developing-today-forks/opencode/pull/730) | fix(tui,cli): reject calendar-invalid date-time form values | dezren39 | 2026-10-28 |

##### developing-today-forks/opencode — theme: security (1)

| PR                                                                  | Title                                                    | Author   | Merged     |
| ------------------------------------------------------------------- | -------------------------------------------------------- | -------- | ---------- |
| [#668](https://github.com/developing-today-forks/opencode/pull/668) | fix(security): pin branch-cleanup checkout to v4.3.1 SHA | dezren39 | 2026-10-28 |

##### developing-today-forks/opencode — theme: provider (1)

| PR                                                                  | Title                                                                         | Author   | Merged     |
| ------------------------------------------------------------------- | ----------------------------------------------------------------------------- | -------- | ---------- |
| [#563](https://github.com/developing-today-forks/opencode/pull/563) | chore(provider): refresh built-in models catalog from models.dev (2026-10-27) | dezren39 | 2026-10-27 |

##### developing-today-forks/opencode — theme: cli,core (1)

| PR                                                                  | Title                                                             | Author   | Merged     |
| ------------------------------------------------------------------- | ----------------------------------------------------------------- | -------- | ---------- |
| [#382](https://github.com/developing-today-forks/opencode/pull/382) | fix(cli,core): resolve bare model aliases through the server path | dezren39 | 2026-10-27 |

##### developing-today-forks/opencode — theme: services (1)

| PR                                                                  | Title                                                    | Author   | Merged     |
| ------------------------------------------------------------------- | -------------------------------------------------------- | -------- | ---------- |
| [#142](https://github.com/developing-today-forks/opencode/pull/142) | test(services): run changelog and update tests via turbo | dezren39 | 2026-10-20 |

##### developing-today-forks/opencode — theme: upstream sync (1)

| PR                                                                  | Title                                                               | Author   | Merged     |
| ------------------------------------------------------------------- | ------------------------------------------------------------------- | -------- | ---------- |
| [#137](https://github.com/developing-today-forks/opencode/pull/137) | ci: pin actions/checkout to v4.3.1 SHA in jules-daily-upstream-sync | dezren39 | 2026-10-19 |

##### developing-today-forks/opencode — theme: desktop,core (1)

| PR                                                                  | Title                                                                                  | Author   | Merged     |
| ------------------------------------------------------------------- | -------------------------------------------------------------------------------------- | -------- | ---------- |
| [#102](https://github.com/developing-today-forks/opencode/pull/102) | fix(desktop,core): retry transient rename failures at the remaining atomic-write sites | dezren39 | 2026-10-19 |

##### developing-today-forks/opencode — theme: acp (1)

| PR                                                                | Title                                                                  | Author   | Merged     |
| ----------------------------------------------------------------- | ---------------------------------------------------------------------- | -------- | ---------- |
| [#58](https://github.com/developing-today-forks/opencode/pull/58) | feat(acp): follow every nested subsession and its asks between prompts | dezren39 | 2026-10-18 |

#### developing-today-forks/openchamber — 1186 merged PRs by theme

| Theme           | Merged PRs |
| --------------- | ---------- |
| web             | 475        |
| ui              | 108        |
| vscode          | 89         |
| ci              | 89         |
| quota           | 48         |
| routing         | 48         |
| electron        | 37         |
| roadmap         | 35         |
| other/no-scope  | 26         |
| web,vscode      | 22         |
| docs            | 20         |
| nix             | 14         |
| deps            | 11         |
| settings        | 8          |
| agent-tool      | 8          |
| chat            | 7          |
| mobile          | 7          |
| control         | 7          |
| flake           | 6          |
| git             | 5          |
| goal            | 5          |
| security        | 5          |
| release         | 5          |
| scripts         | 4          |
| tests           | 4          |
| quota/plans     | 4          |
| i18n            | 4          |
| environment     | 4          |
| web,ui          | 3          |
| sidebar         | 3          |
| browser         | 3          |
| fs              | 2          |
| upstream sync   | 2          |
| goal-writer     | 2          |
| sync            | 2          |
| cli             | 2          |
| ui,web          | 2          |
| model-choices   | 2          |
| routes/pools    | 2          |
| plugins/mcp     | 2          |
| sessions        | 2          |
| notes           | 2          |
| providers       | 2          |
| routes          | 2          |
| routing-ui      | 2          |
| proxy           | 2          |
| dictation       | 2          |
| lifecycle       | 2          |
| markdown        | 2          |
| audit           | 1          |
| client-auth     | 1          |
| tts             | 1          |
| models          | 1          |
| event-stream    | 1          |
| session-work    | 1          |
| perf            | 1          |
| oc-review       | 1          |
| opencode        | 1          |
| dependabot      | 1          |
| repo            | 1          |
| workflow        | 1          |
| notifications   | 1          |
| web,vscode,ui   | 1          |
| web,ui,sdk      | 1          |
| project-context | 1          |
| quota,web       | 1          |
| nix/packaging   | 1          |
| workflow-audit  | 1          |
| watcher         | 1          |
| themes          | 1          |
| github-routes   | 1          |
| context         | 1          |
| opencode-update | 1          |
| quota-ui        | 1          |
| web-cli         | 1          |
| deps-dev        | 1          |
| dev-servers     | 1          |
| github          | 1          |
| source-control  | 1          |
| stale           | 1          |
| sdk             | 1          |
| contributing    | 1          |
| messaging       | 1          |
| worktrees       | 1          |
| update          | 1          |

##### developing-today-forks/openchamber — theme: web (475)

| PR                                                                       | Title                                                                                                               | Author                | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------- | --------------------- | ---------- |
| [#132](https://github.com/developing-today-forks/openchamber/pull/132)   | test(web): self-handoff readiness timeout now reports server log, exit state and pid file                           | dezren39              | 2026-10-20 |
| [#187](https://github.com/developing-today-forks/openchamber/pull/187)   | fix(web): reject malformed versions in the upgrade check                                                            | dezren39              | 2026-10-21 |
| [#189](https://github.com/developing-today-forks/openchamber/pull/189)   | fix(web): reject malformed versions in package-manager update check                                                 | dezren39              | 2026-10-21 |
| [#229](https://github.com/developing-today-forks/openchamber/pull/229)   | fix(web): reject malformed and out-of-range serve --port values instead of coercing them                            | dezren39              | 2026-10-21 |
| [#234](https://github.com/developing-today-forks/openchamber/pull/234)   | fix(web): reject malformed dev-tunnel ports instead of coercing them with parseInt                                  | dezren39              | 2026-10-21 |
| [#238](https://github.com/developing-today-forks/openchamber/pull/238)   | fix(web): reject malformed and out-of-range env-config ports instead of coercing them                               | dezren39              | 2026-10-21 |
| [#508](https://github.com/developing-today-forks/openchamber/pull/508)   | test(web): isolate the folder-config suite from OPENCODE_CONFIG_DIR                                                 | dezren39              | 2026-10-26 |
| [#620](https://github.com/developing-today-forks/openchamber/pull/620)   | feat(web): session-activity v1 fields for external consumers (#524)                                                 | dezren39              | 2026-10-28 |
| [#619](https://github.com/developing-today-forks/openchamber/pull/619)   | feat(web): add GET /api/managed-servers registry read (#525)                                                        | dezren39              | 2026-10-28 |
| [#633](https://github.com/developing-today-forks/openchamber/pull/633)   | fix(web): scope managed-servers own-server sessionCount to host sessions (#623)                                     | dezren39              | 2026-10-28 |
| [#639](https://github.com/developing-today-forks/openchamber/pull/639)   | fix(web): verify recorded-pid identity in the managed-servers read (#638)                                           | dezren39              | 2026-10-28 |
| [#649](https://github.com/developing-today-forks/openchamber/pull/649)   | docs(web): document the observability contract v1 and pin it with a conformance fixture (#614)                      | dezren39              | 2026-10-28 |
| [#677](https://github.com/developing-today-forks/openchamber/pull/677)   | fix(web): surface git remote failures instead of reporting no remotes                                               | dezren39              | 2026-10-28 |
| [#670](https://github.com/developing-today-forks/openchamber/pull/670)   | test(web): surface child errors in the audit-storage multi-process test (#667)                                      | dezren39              | 2026-10-28 |
| [#671](https://github.com/developing-today-forks/openchamber/pull/671)   | fix(web): throw on ls-files failure in listUntrackedPaths                                                           | dezren39              | 2026-10-28 |
| [#683](https://github.com/developing-today-forks/openchamber/pull/683)   | test(web): make gatekeeper-program port selection race-free                                                         | dezren39              | 2026-10-28 |
| [#686](https://github.com/developing-today-forks/openchamber/pull/686)   | test(web): make the two env-gated suites' port selection race-free                                                  | dezren39              | 2026-10-28 |
| [#694](https://github.com/developing-today-forks/openchamber/pull/694)   | fix(web): strict limit parsing for model-choices and message-search routes                                          | dezren39              | 2026-10-28 |
| [#695](https://github.com/developing-today-forks/openchamber/pull/695)   | fix(web): strict env parsing for health-check and relay batch-window readers                                        | dezren39              | 2026-10-28 |
| [#698](https://github.com/developing-today-forks/openchamber/pull/698)   | fix(web): strict whole-text judging for the four fs env settings (#696)                                             | dezren39              | 2026-10-28 |
| [#700](https://github.com/developing-today-forks/openchamber/pull/700)   | fix(web): whole-text decimal judging for event-stream timeout env values                                            | dezren39              | 2026-10-28 |
| [#699](https://github.com/developing-today-forks/openchamber/pull/699)   | test(web): retry self-handoff server starts on a taken port (#676)                                                  | dezren39              | 2026-10-28 |
| [#712](https://github.com/developing-today-forks/openchamber/pull/712)   | fix(web): strict env parsing for guest upload cap (#704)                                                            | dezren39              | 2026-10-28 |
| [#706](https://github.com/developing-today-forks/openchamber/pull/706)   | fix(web): strict plain-decimal handoff timeout parsing                                                              | dezren39              | 2026-10-28 |
| [#719](https://github.com/developing-today-forks/openchamber/pull/719)   | fix(web): confine skill install copy to the clone and refuse symlinked selections                                   | dezren39              | 2026-10-28 |
| [#722](https://github.com/developing-today-forks/openchamber/pull/722)   | fix(web): resolve the real git common dir in ensureOpenCodeProjectId (#720)                                         | dezren39              | 2026-10-28 |
| [#728](https://github.com/developing-today-forks/openchamber/pull/728)   | fix(web): judge delta-coalesce window env value by whole-text plain decimal (#725)                                  | dezren39              | 2026-10-28 |
| [#740](https://github.com/developing-today-forks/openchamber/pull/740)   | fix(web): judge quota CLI numeric env values as plain decimal                                                       | dezren39              | 2026-10-29 |
| [#739](https://github.com/developing-today-forks/openchamber/pull/739)   | fix(web): cap /api/fs/exec command output per stream                                                                | dezren39              | 2026-10-29 |
| [#746](https://github.com/developing-today-forks/openchamber/pull/746)   | fix(web): cap guest runGit captured stdout at 4 MiB                                                                 | dezren39              | 2026-10-29 |
| [#748](https://github.com/developing-today-forks/openchamber/pull/748)   | feat(web): expose OpenCode Go plan status (product) on the quota result                                             | dezren39              | 2026-10-29 |
| [#762](https://github.com/developing-today-forks/openchamber/pull/762)   | fix(web): stream non-SSE answers on the event route instead of buffering them whole                                 | dezren39              | 2026-10-29 |
| [#757](https://github.com/developing-today-forks/openchamber/pull/757)   | fix(web): bound SSE partial-block buffers in upstream reader and cursor                                             | dezren39              | 2026-10-29 |
| [#771](https://github.com/developing-today-forks/openchamber/pull/771)   | feat(web): per-workspace OpenCode Go quota rows via per-credential dispatch (#768)                                  | dezren39              | 2026-10-29 |
| [#767](https://github.com/developing-today-forks/openchamber/pull/767)   | fix(web): read the Jev classifier response through a bounded reader                                                 | dezren39              | 2026-10-29 |
| [#772](https://github.com/developing-today-forks/openchamber/pull/772)   | fix(web): judge drain grace env as plain decimal only (#769)                                                        | dezren39              | 2026-10-29 |
| [#773](https://github.com/developing-today-forks/openchamber/pull/773)   | fix(web): bound guest surface service response reads during the read (#770)                                         | dezren39              | 2026-10-29 |
| [#777](https://github.com/developing-today-forks/openchamber/pull/777)   | fix(web): mcpx daemon client timeout + response cap (#774)                                                          | dezren39              | 2026-10-29 |
| [#778](https://github.com/developing-today-forks/openchamber/pull/778)   | fix(web): bound Linear GraphQL and OAuth fetches with deadlines                                                     | dezren39              | 2026-10-29 |
| [#779](https://github.com/developing-today-forks/openchamber/pull/779)   | fix(web): cap session payload body read at 16 MiB during the read                                                   | dezren39              | 2026-10-29 |
| [#783](https://github.com/developing-today-forks/openchamber/pull/783)   | fix(web): bound GitLab client requests with a per-attempt deadline                                                  | dezren39              | 2026-10-29 |
| [#785](https://github.com/developing-today-forks/openchamber/pull/785)   | fix(web): bound quota provider fetches with the 15s deadline                                                        | dezren39              | 2026-10-29 |
| [#801](https://github.com/developing-today-forks/openchamber/pull/801)   | fix(web): add deadlines to the APNs push runtime                                                                    | dezren39              | 2026-10-29 |
| [#792](https://github.com/developing-today-forks/openchamber/pull/792)   | fix(web): meta quota result carries failure class + last-success age (#787)                                         | dezren39              | 2026-10-29 |
| [#800](https://github.com/developing-today-forks/openchamber/pull/800)   | fix(web): give waitForAgentPresence a per-attempt deadline                                                          | dezren39              | 2026-10-29 |
| [#809](https://github.com/developing-today-forks/openchamber/pull/809)   | fix(web): bound message-search indexer reads with a deadline and abort on stop                                      | dezren39              | 2026-10-29 |
| [#811](https://github.com/developing-today-forks/openchamber/pull/811)   | fix(web): cancel a scheduled-task run when its watchdog times out                                                   | dezren39              | 2026-10-29 |
| [#799](https://github.com/developing-today-forks/openchamber/pull/799)   | fix(web): add deadline to /api/info version probes (#796)                                                           | dezren39              | 2026-10-29 |
| [#816](https://github.com/developing-today-forks/openchamber/pull/816)   | fix(web): bound local STT model downloads with a stall timeout                                                      | dezren39              | 2026-10-29 |
| [#806](https://github.com/developing-today-forks/openchamber/pull/806)   | feat(web): shared bounded-call primitives in lib/shared (design #802 phase 1)                                       | dezren39              | 2026-10-29 |
| [#821](https://github.com/developing-today-forks/openchamber/pull/821)   | fix(web): bound tunnel availability spawnSync probes with a timeout                                                 | dezren39              | 2026-10-29 |
| [#826](https://github.com/developing-today-forks/openchamber/pull/826)   | refactor(web): migrate parsers, GitLab fetch helper and bounded readers to lib/shared primitives (#804)             | dezren39              | 2026-10-29 |
| [#829](https://github.com/developing-today-forks/openchamber/pull/829)   | fix(web): stall deadline for routing local-model download (#828)                                                    | dezren39              | 2026-10-29 |
| [#831](https://github.com/developing-today-forks/openchamber/pull/831)   | fix(web): add timeout to NanoGPT quota provider fetch — #827                                                        | dezren39              | 2026-10-29 |
| [#838](https://github.com/developing-today-forks/openchamber/pull/838)   | fix(web): size caps on OpenCode v2 install downloads (#834)                                                         | dezren39              | 2026-10-29 |
| [#839](https://github.com/developing-today-forks/openchamber/pull/839)   | fix(web): deadline and no-retry policy for TTS/STT OpenAI SDK calls (#824)                                          | dezren39              | 2026-10-29 |
| [#840](https://github.com/developing-today-forks/openchamber/pull/840)   | test(web): keep sse-resume restart retries on the proxy's port (#830)                                               | dezren39              | 2026-10-29 |
| [#841](https://github.com/developing-today-forks/openchamber/pull/841)   | fix(web): timeouts on CLI spawnSync calls (startup wrapper + bun probe) (#835)                                      | dezren39              | 2026-10-29 |
| [#843](https://github.com/developing-today-forks/openchamber/pull/843)   | fix(web): bounded read for the login keeper issuer answer (#842)                                                    | dezren39              | 2026-10-29 |
| [#823](https://github.com/developing-today-forks/openchamber/pull/823)   | fix(web): deadline and in-flight guard for ngrok readiness poll (#819)                                              | dezren39              | 2026-10-29 |
| [#846](https://github.com/developing-today-forks/openchamber/pull/846)   | fix(web): add 15s deadline to Kimi quota primary fetch (#844)                                                       | dezren39              | 2026-10-29 |
| [#847](https://github.com/developing-today-forks/openchamber/pull/847)   | fix(web): bound OpenCode error-detail body reads (#845)                                                             | dezren39              | 2026-10-29 |
| [#850](https://github.com/developing-today-forks/openchamber/pull/850)   | fix(web): bound GitLab client error-body read (#849)                                                                | dezren39              | 2026-10-30 |
| [#852](https://github.com/developing-today-forks/openchamber/pull/852)   | fix(web): bound Octokit ETag cache by bytes (#848)                                                                  | dezren39              | 2026-10-30 |
| [#869](https://github.com/developing-today-forks/openchamber/pull/869)   | fix(web): bound the credential helper's delegate 'git credential fill' with a 10s deadline                          | dezren39              | 2026-10-30 |
| [#870](https://github.com/developing-today-forks/openchamber/pull/870)   | fix(web): add a deadline to the OpenCode CLI upgrade child process                                                  | dezren39              | 2026-10-30 |
| [#853](https://github.com/developing-today-forks/openchamber/pull/853)   | feat(web): Linear accounts from a 1Password API key reference (op://)                                               | dezren39              | 2026-10-30 |
| [#874](https://github.com/developing-today-forks/openchamber/pull/874)   | fix(web): carry legacy sidebarDetailedZoneRows into sidebarSessionRowDisplay                                        | dezren39              | 2026-10-30 |
| [#879](https://github.com/developing-today-forks/openchamber/pull/879)   | fix(web): bound notification runtime permission and cooldown retention                                              | dezren39              | 2026-10-30 |
| [#884](https://github.com/developing-today-forks/openchamber/pull/884)   | fix(web): bound the worktree start-command timeout at 2^31-1 ms                                                     | dezren39              | 2026-10-30 |
| [#878](https://github.com/developing-today-forks/openchamber/pull/878)   | fix(web): bound the quota CLI timeout at 2^31-1 ms                                                                  | dezren39              | 2026-10-30 |
| [#886](https://github.com/developing-today-forks/openchamber/pull/886)   | fix(web): strict spelling and 400-day bound for ui-auth session TTL readers                                         | dezren39              | 2026-10-30 |
| [#888](https://github.com/developing-today-forks/openchamber/pull/888)   | fix(web): atomic write + corrupt-file preservation for the magic-prompts store                                      | dezren39              | 2026-10-30 |
| [#900](https://github.com/developing-today-forks/openchamber/pull/900)   | fix(web): atomic 0600 ui-passkeys store with corrupt-read preservation (#897)                                       | dezren39              | 2026-10-30 |
| [#890](https://github.com/developing-today-forks/openchamber/pull/890)   | fix(web): atomic writes + corrupt-read preservation for client-auth and notification token stores                   | dezren39              | 2026-10-30 |
| [#896](https://github.com/developing-today-forks/openchamber/pull/896)   | fix(web): atomic writes and corrupt-file preservation for routing stores                                            | dezren39              | 2026-10-30 |
| [#902](https://github.com/developing-today-forks/openchamber/pull/902)   | fix(web): atomic migration writes + torn-destination preservation in settings-runtime                               | dezren39              | 2026-10-30 |
| [#906](https://github.com/developing-today-forks/openchamber/pull/906)   | fix(web): bound the notification template runtime session caches                                                    | dezren39              | 2026-10-30 |
| [#908](https://github.com/developing-today-forks/openchamber/pull/908)   | fix(web): preserve a malformed voice-keys file before an update can erase it                                        | dezren39              | 2026-10-30 |
| [#910](https://github.com/developing-today-forks/openchamber/pull/910)   | fix(web): Linear auth corrupt reads must not erase tokens or settings (#907)                                        | dezren39              | 2026-10-30 |
| [#930](https://github.com/developing-today-forks/openchamber/pull/930)   | fix(web): TTS speech body read has no byte cap                                                                      | dezren39              | 2026-10-30 |
| [#946](https://github.com/developing-today-forks/openchamber/pull/946)   | fix(web): bound dictation model tar extraction with a timeout                                                       | dezren39              | 2026-10-30 |
| [#945](https://github.com/developing-today-forks/openchamber/pull/945)   | fix(web): guard getProviderAuth against prototype-name provider ids                                                 | dezren39              | 2026-10-30 |
| [#958](https://github.com/developing-today-forks/openchamber/pull/958)   | fix(web): bound the project start-command read with an 8s deadline                                                  | dezren39              | 2026-10-30 |
| [#960](https://github.com/developing-today-forks/openchamber/pull/960)   | fix(web): add absolute deadline to mcpx daemon Unix-socket transport                                                | dezren39              | 2026-10-30 |
| [#957](https://github.com/developing-today-forks/openchamber/pull/957)   | fix(web): bound small-model catalog reads with a 15s deadline                                                       | dezren39              | 2026-10-30 |
| [#901](https://github.com/developing-today-forks/openchamber/pull/901)   | fix(web): atomic writes + corrupt-read preservation for the managed tunnel config store                             | dezren39              | 2026-10-30 |
| [#963](https://github.com/developing-today-forks/openchamber/pull/963)   | fix(web): add backpressure to dev-tunnel piping after connect                                                       | dezren39              | 2026-10-30 |
| [#964](https://github.com/developing-today-forks/openchamber/pull/964)   | fix(web): bound the realtime proxy WebSocket queue and handshake                                                    | dezren39              | 2026-10-30 |
| [#966](https://github.com/developing-today-forks/openchamber/pull/966)   | fix(web): use own-property lookups for prototype-named actions (#965)                                               | dezren39              | 2026-10-30 |
| [#970](https://github.com/developing-today-forks/openchamber/pull/970)   | fix(web): evict expired skills-catalog and github-meta cache entries                                                | dezren39              | 2026-10-31 |
| [#975](https://github.com/developing-today-forks/openchamber/pull/975)   | fix(web): bound pr-status caches with sweep and cap (#972)                                                          | dezren39              | 2026-10-31 |
| [#982](https://github.com/developing-today-forks/openchamber/pull/982)   | fix(web): bound notification sessionParentIdCache with cap and sweep (#974)                                         | dezren39              | 2026-10-31 |
| [#985](https://github.com/developing-today-forks/openchamber/pull/985)   | fix(web): evict expired git remote cache entries with sweep and bound (#979)                                        | dezren39              | 2026-10-31 |
| [#1002](https://github.com/developing-today-forks/openchamber/pull/1002) | fix(web): installed-sources store must not overwrite a malformed provenance file (#936)                             | dezren39              | 2026-10-31 |
| [#997](https://github.com/developing-today-forks/openchamber/pull/997)   | fix(web): report APNs relay body-read timeouts instead of swallowing them (#993)                                    | dezren39              | 2026-10-31 |
| [#981](https://github.com/developing-today-forks/openchamber/pull/981)   | feat(web): shared durable-store primitive in lib/shared (#977)                                                      | dezren39              | 2026-10-31 |
| [#989](https://github.com/developing-today-forks/openchamber/pull/989)   | fix(web): bound managed OpenCode log with pre-spawn rotation (#941)                                                 | dezren39              | 2026-10-31 |
| [#990](https://github.com/developing-today-forks/openchamber/pull/990)   | fix(web): gh-CLI setters use strict settings read (#940)                                                            | dezren39              | 2026-10-31 |
| [#996](https://github.com/developing-today-forks/openchamber/pull/996)   | fix(web): quota registries use own-property provider lookups (#939)                                                 | dezren39              | 2026-10-31 |
| [#1003](https://github.com/developing-today-forks/openchamber/pull/1003) | fix(web): settle direct APNs sends when APNs closes the stream (#992)                                               | dezren39              | 2026-10-31 |
| [#1000](https://github.com/developing-today-forks/openchamber/pull/1000) | fix(web): give managed-process claim files an end of life (#942)                                                    | dezren39              | 2026-10-31 |
| [#1004](https://github.com/developing-today-forks/openchamber/pull/1004) | fix(web): bound the session-warming activity map at 500 entries (#987)                                              | dezren39              | 2026-10-31 |
| [#1007](https://github.com/developing-today-forks/openchamber/pull/1007) | refactor(web): migrate voice-keys store to the durable-store primitive (#978 step 1)                                | dezren39              | 2026-10-31 |
| [#1012](https://github.com/developing-today-forks/openchamber/pull/1012) | refactor(web): export shared combineAbortSignals and migrate copies (#995)                                          | dezren39              | 2026-10-31 |
| [#1001](https://github.com/developing-today-forks/openchamber/pull/1001) | fix(web): dictation model spec lookup must not resolve Object.prototype names (#988)                                | dezren39              | 2026-10-31 |
| [#1009](https://github.com/developing-today-forks/openchamber/pull/1009) | fix(web): cap three public-internet JSON reads during streaming                                                     | dezren39              | 2026-10-31 |
| [#1016](https://github.com/developing-today-forks/openchamber/pull/1016) | fix(web): cap GitLab client success-path body read at 16 MiB                                                        | dezren39              | 2026-10-31 |
| [#1020](https://github.com/developing-today-forks/openchamber/pull/1020) | fix(web): import WebSocketServer by name in dev-tunnel backpressure test (#1018)                                    | dezren39              | 2026-10-31 |
| [#1022](https://github.com/developing-today-forks/openchamber/pull/1022) | fix(web): cap dictation reorder buffer at 8 MiB buffered bytes (#1017)                                              | dezren39              | 2026-10-31 |
| [#1036](https://github.com/developing-today-forks/openchamber/pull/1036) | test(web): seam and pin Kimi/NanoGPT quota deadlines via timeoutMs (#1033)                                          | dezren39              | 2026-10-31 |
| [#1038](https://github.com/developing-today-forks/openchamber/pull/1038) | docs(web): correct the self-handoff parser entry in shared DOCUMENTATION.md                                         | dezren39              | 2026-10-31 |
| [#1028](https://github.com/developing-today-forks/openchamber/pull/1028) | fix(web): cap /api/fs/read at 16 MiB with 413 before readFile (#1026)                                               | dezren39              | 2026-10-31 |
| [#1034](https://github.com/developing-today-forks/openchamber/pull/1034) | fix(web): race routing download write-side awaits against the stall deadline (#1031)                                | dezren39              | 2026-10-31 |
| [#1039](https://github.com/developing-today-forks/openchamber/pull/1039) | fix(web): bound OpenCode health timeout/interval readers at 2^31-1 ms (#1030)                                       | dezren39              | 2026-10-31 |
| [#1023](https://github.com/developing-today-forks/openchamber/pull/1023) | fix(web): cap GitLab auth-path JSON reads at 1 MiB (#1021)                                                          | dezren39              | 2026-10-31 |
| [#1035](https://github.com/developing-today-forks/openchamber/pull/1035) | fix(web): make the speech deadline one budget across create + body phases                                           | dezren39              | 2026-10-31 |
| [#1041](https://github.com/developing-today-forks/openchamber/pull/1041) | fix(web): cap upgrade-status registry /latest body read at 1 MiB (#1040)                                            | dezren39              | 2026-10-31 |
| [#1019](https://github.com/developing-today-forks/openchamber/pull/1019) | refactor(web): migrate pairing store to the durable-store primitive (#978 step 2)                                   | dezren39              | 2026-10-31 |
| [#1048](https://github.com/developing-today-forks/openchamber/pull/1048) | fix(web): bound package-manager update-check response bodies at 1 MiB                                               | dezren39              | 2026-10-31 |
| [#1055](https://github.com/developing-today-forks/openchamber/pull/1055) | fix(web): stat-gate project-icon discover before readFile (#1053)                                                   | dezren39              | 2026-10-31 |
| [#1058](https://github.com/developing-today-forks/openchamber/pull/1058) | fix(web): cap git file-diff working-tree reads at 50 MiB (#1054)                                                    | dezren39              | 2026-10-31 |
| [#882](https://github.com/developing-today-forks/openchamber/pull/882)   | fix(web): terminate the credential-helper delegate process tree on timeout                                          | dezren39              | 2026-10-31 |
| [#1072](https://github.com/developing-today-forks/openchamber/pull/1072) | fix(web): resolve the Antigravity local-store fallback database from the launch environment (#1063)                 | dezren39              | 2026-10-31 |
| [#1093](https://github.com/developing-today-forks/openchamber/pull/1093) | fix(web): make vault refresh:true re-read instead of serving a fresh cache entry                                    | dezren39              | 2026-11-01 |
| [#1088](https://github.com/developing-today-forks/openchamber/pull/1088) | fix(web): name the bindingIntent fields in the binding.set tool schema                                              | dezren39              | 2026-11-01 |
| [#1089](https://github.com/developing-today-forks/openchamber/pull/1089) | fix(web): match the plural authorizations in the SECRET_KEY guard                                                   | dezren39              | 2026-11-01 |
| [#1097](https://github.com/developing-today-forks/openchamber/pull/1097) | fix(web): write delta-coalescer KEY_SEPARATOR as an escape, not a raw NUL byte                                      | dezren39              | 2026-11-01 |
| [#1090](https://github.com/developing-today-forks/openchamber/pull/1090) | fix(web): guard parseDiffContextLines against non-finite and oversized context values                               | dezren39              | 2026-11-01 |
| [#1099](https://github.com/developing-today-forks/openchamber/pull/1099) | fix(web): bound dev-tunnel WebSocket frames at the held cap                                                         | dezren39              | 2026-11-01 |
| [#1117](https://github.com/developing-today-forks/openchamber/pull/1117) | fix(web): cap realtime-proxy and persistent-pty WebSocket maxPayload (#1109)                                        | dezren39              | 2026-11-01 |
| [#1101](https://github.com/developing-today-forks/openchamber/pull/1101) | fix(web): register worktree start commands with the live-tree shutdown cleanup                                      | dezren39              | 2026-11-01 |
| [#1110](https://github.com/developing-today-forks/openchamber/pull/1110) | fix(web): guard tunnel-auth cookie parsing against malformed values (#1095)                                         | dezren39              | 2026-11-01 |
| [#1118](https://github.com/developing-today-forks/openchamber/pull/1118) | fix(web): cap message-stream WebSocket maxPayload at 64 KiB (#1108)                                                 | dezren39              | 2026-11-01 |
| [#1111](https://github.com/developing-today-forks/openchamber/pull/1111) | fix(web): bound Linear broker/token JSON reads with a byte cap (#1102)                                              | dezren39              | 2026-11-01 |
| [#1106](https://github.com/developing-today-forks/openchamber/pull/1106) | fix(web): cap dev-server scanner stdout at 4 MiB with kill-on-overflow                                              | dezren39              | 2026-11-01 |
| [#1107](https://github.com/developing-today-forks/openchamber/pull/1107) | fix(web): stat-gate the post-checkout hook read (#1086)                                                             | dezren39              | 2026-11-01 |
| [#1140](https://github.com/developing-today-forks/openchamber/pull/1140) | fix(web): stat-gate the /api/fs/write identical-content pre-read (#1127)                                            | dezren39              | 2026-11-01 |
| [#1122](https://github.com/developing-today-forks/openchamber/pull/1122) | fix(web): stat-gate the guest asset serve read with a named byte cap (#1116)                                        | dezren39              | 2026-11-01 |
| [#1136](https://github.com/developing-today-forks/openchamber/pull/1136) | fix(web): gate project plan reads on a 1 MiB size cap (#1133)                                                       | dezren39              | 2026-11-01 |
| [#1124](https://github.com/developing-today-forks/openchamber/pull/1124) | fix(web): clamp git log maxCount                                                                                    | dezren39              | 2026-11-01 |
| [#1137](https://github.com/developing-today-forks/openchamber/pull/1137) | fix(web): stat-gate Behavior AGENTS.md reads at 1 MiB (#1132)                                                       | dezren39              | 2026-11-01 |
| [#1152](https://github.com/developing-today-forks/openchamber/pull/1152) | fix(web): stat-gate plugin file reads at 1 MiB (#1138)                                                              | dezren39              | 2026-11-01 |
| [#1147](https://github.com/developing-today-forks/openchamber/pull/1147) | fix(web): stat-gate skill supporting-file reads at 1 MiB (#1139)                                                    | dezren39              | 2026-11-01 |
| [#1148](https://github.com/developing-today-forks/openchamber/pull/1148) | fix(web): stat-gate snippet file reads at 4 MiB (#1144)                                                             | dezren39              | 2026-11-01 |
| [#1246](https://github.com/developing-today-forks/openchamber/pull/1246) | fix(web): join Express wildcard segments in skill-file PUT/DELETE handlers (#1146)                                  | dezren39              | 2026-11-01 |
| [#1153](https://github.com/developing-today-forks/openchamber/pull/1153) | fix(web): stat-gate guest package.json manifest read at the asset cap (#1149)                                       | dezren39              | 2026-11-01 |
| [#1145](https://github.com/developing-today-forks/openchamber/pull/1145) | fix(web): size-gate parseMdFile/parseMdFileAsync on a named 4 MiB cap (#1143)                                       | dezren39              | 2026-11-01 |
| [#1258](https://github.com/developing-today-forks/openchamber/pull/1258) | feat(web): add shared bounded file-read primitive (#1256)                                                           | dezren39              | 2026-11-01 |
| [#1253](https://github.com/developing-today-forks/openchamber/pull/1253) | fix(web): bound proxy upstream WebSocket frames with named maxPayload + 1009 handler checks (#1249)                 | dezren39              | 2026-11-01 |
| [#1155](https://github.com/developing-today-forks/openchamber/pull/1155) | fix(web): stat-gate config file reads at 4 MiB (#1150)                                                              | dezren39              | 2026-11-01 |
| [#1263](https://github.com/developing-today-forks/openchamber/pull/1263) | refactor(web): read SKILL.md through the shared bounded file-read primitive (#1255, scan module)                    | dezren39              | 2026-11-01 |
| [#1268](https://github.com/developing-today-forks/openchamber/pull/1268) | fix(web): bound generated agent-tool callback response read at 16 MiB (#1265)                                       | dezren39              | 2026-11-01 |
| [#1271](https://github.com/developing-today-forks/openchamber/pull/1271) | fix(web): cap walkthrough pull-request patch reads at 50 MiB (#1267)                                                | dezren39              | 2026-11-01 |
| [#1269](https://github.com/developing-today-forks/openchamber/pull/1269) | refactor(web): migrate remaining size gates to shared bounded file read (#1255)                                     | dezren39              | 2026-11-01 |
| [#1278](https://github.com/developing-today-forks/openchamber/pull/1278) | fix(web): stat-judge the remaining whole plan reads (#1277)                                                         | dezren39              | 2026-11-01 |
| [#1286](https://github.com/developing-today-forks/openchamber/pull/1286) | fix(web): bound quota provider response bodies at 1 MiB (#1282)                                                     | dezren39              | 2026-11-02 |
| [#1284](https://github.com/developing-today-forks/openchamber/pull/1284) | fix(web): cap content-search ripgrep line accumulator at 1 MiB, skip over-cap lines (#1283)                         | dezren39              | 2026-11-02 |
| [#1290](https://github.com/developing-today-forks/openchamber/pull/1290) | fix(web): isolate routes-upgrade bundled-binary test from machine npm registry config                               | dezren39              | 2026-11-02 |
| [#1287](https://github.com/developing-today-forks/openchamber/pull/1287) | fix(web): cap internal OpenCode JSON response bodies                                                                | dezren39              | 2026-11-02 |
| [#1296](https://github.com/developing-today-forks/openchamber/pull/1296) | fix(web): cap GitHub device-flow token body reads at 1 MiB (#1292)                                                  | dezren39              | 2026-11-02 |
| [#1294](https://github.com/developing-today-forks/openchamber/pull/1294) | fix(web): cap Linear GraphQL response bodies                                                                        | dezren39              | 2026-11-02 |
| [#1302](https://github.com/developing-today-forks/openchamber/pull/1302) | fix(web): cap local git diff output at 50 MiB (#1301)                                                               | dezren39              | 2026-11-02 |
| [#1306](https://github.com/developing-today-forks/openchamber/pull/1306) | fix(web): bound skills-catalog GitHub repo-meta response reads                                                      | dezren39              | 2026-11-02 |
| [#1307](https://github.com/developing-today-forks/openchamber/pull/1307) | fix(web): bound APNs relay send response read at 1 MiB                                                              | dezren39              | 2026-11-02 |
| [#1309](https://github.com/developing-today-forks/openchamber/pull/1309) | fix(web): read discovered project icon through a bounded handle (#1305)                                             | dezren39              | 2026-11-02 |
| [#1312](https://github.com/developing-today-forks/openchamber/pull/1312) | fix(web): cap direct-mode APNs response body at 1 MiB (#1311)                                                       | dezren39              | 2026-11-02 |
| [#1314](https://github.com/developing-today-forks/openchamber/pull/1314) | fix(web): cap GitHub branch pagination                                                                              | dezren39              | 2026-11-02 |
| [#1045](https://github.com/developing-today-forks/openchamber/pull/1045) | refactor(web): migrate remote-clients store to the durable-store primitive (#978 step 3)                            | dezren39              | 2026-11-02 |
| [#1112](https://github.com/developing-today-forks/openchamber/pull/1112) | fix(web): bound guest OAuth token and account response reads                                                        | dezren39              | 2026-11-02 |
| [#1319](https://github.com/developing-today-forks/openchamber/pull/1319) | fix(web): read the nested experimental.routes shape in model-routes                                                 | dezren39              | 2026-11-02 |
| [#1318](https://github.com/developing-today-forks/openchamber/pull/1318) | fix(web): cap openchamber-control cursor walks at 20 pages                                                          | dezren39              | 2026-11-02 |
| [#1329](https://github.com/developing-today-forks/openchamber/pull/1329) | fix(web): stream clone checkout digests instead of reading files whole                                              | dezren39              | 2026-11-02 |
| [#1325](https://github.com/developing-today-forks/openchamber/pull/1325) | fix(web): cap the three internal OpenCode fetchers missed by #1285 (#1291)                                          | dezren39              | 2026-11-02 |
| [#1316](https://github.com/developing-today-forks/openchamber/pull/1316) | fix(web): guard message-search indexer pagination against looping cursors                                           | dezren39              | 2026-11-02 |
| [#1066](https://github.com/developing-today-forks/openchamber/pull/1066) | fix(web): bound persistent-pty proxy pre-open queue, handshake, and slow peers (#1059)                              | dezren39              | 2026-11-02 |
| [#938](https://github.com/developing-today-forks/openchamber/pull/938)   | fix(web): surface shared OpenCode upstream-auth failure in quota results                                            | dezren39              | 2026-11-02 |
| [#731](https://github.com/developing-today-forks/openchamber/pull/731)   | feat(web): add openchamber rotate-credentials for the managed OpenCode pair                                         | dezren39              | 2026-11-02 |
| [#1334](https://github.com/developing-today-forks/openchamber/pull/1334) | refactor(web): migrate magic-prompts store to the durable-store primitive (#978)                                    | dezren39              | 2026-11-02 |
| [#1358](https://github.com/developing-today-forks/openchamber/pull/1358) | fix(web): decode streamed git output with StringDecoder so non-ASCII filenames survive chunk boundaries             | dezren39              | 2026-11-02 |
| [#1370](https://github.com/developing-today-forks/openchamber/pull/1370) | fix(web): bound the ngrok agent-API body read (#1359)                                                               | dezren39              | 2026-11-02 |
| [#1362](https://github.com/developing-today-forks/openchamber/pull/1362) | refactor(web): migrate route-settings store to the durable-store primitive (#978)                                   | dezren39              | 2026-11-02 |
| [#1376](https://github.com/developing-today-forks/openchamber/pull/1376) | fix(web): stream oversized Octokit ETag responses without whole-body buffering (#1373)                              | dezren39              | 2026-11-02 |
| [#1378](https://github.com/developing-today-forks/openchamber/pull/1378) | fix(web): paginate GitHub /pulls/context collections under a named cap (#1375)                                      | dezren39              | 2026-11-02 |
| [#1374](https://github.com/developing-today-forks/openchamber/pull/1374) | refactor(web): migrate UI passkeys store to durable-store primitive (#978)                                          | dezren39              | 2026-11-02 |
| [#1384](https://github.com/developing-today-forks/openchamber/pull/1384) | fix(web): paginate GitHub /issues/comments under the pull-context cap (#1383)                                       | dezren39              | 2026-11-03 |
| [#1401](https://github.com/developing-today-forks/openchamber/pull/1401) | fix(web): realtime-proxy bounds self-sabotage (#1394)                                                               | dezren39              | 2026-11-03 |
| [#1395](https://github.com/developing-today-forks/openchamber/pull/1395) | feat(web): shared traversal-budget primitive in lib/shared (#1392)                                                  | dezren39              | 2026-11-03 |
| [#1381](https://github.com/developing-today-forks/openchamber/pull/1381) | refactor(web): migrate apns-runtime token store to durable store (#978)                                             | dezren39              | 2026-11-03 |
| [#1385](https://github.com/developing-today-forks/openchamber/pull/1385) | refactor(web): migrate push subscriptions store to durable-store primitive (#978)                                   | dezren39              | 2026-11-03 |
| [#1390](https://github.com/developing-today-forks/openchamber/pull/1390) | refactor(web): migrate dispatch-results store to durable-store primitive (#978)                                     | dezren39              | 2026-11-03 |
| [#1407](https://github.com/developing-today-forks/openchamber/pull/1407) | fix(web): paginate GitHub check runs in the checks summary (#1405)                                                  | dezren39              | 2026-11-03 |
| [#1409](https://github.com/developing-today-forks/openchamber/pull/1409) | refactor(web): migrate message-queue store to durable-store primitive (#978)                                        | dezren39              | 2026-11-03 |
| [#1411](https://github.com/developing-today-forks/openchamber/pull/1411) | refactor(web): migrate session archive store to the durable-store primitive (#978)                                  | dezren39              | 2026-11-03 |
| [#1416](https://github.com/developing-today-forks/openchamber/pull/1416) | refactor(web): migrate session metadata legacy store to the durable-store primitive (#978)                          | dezren39              | 2026-11-03 |
| [#1379](https://github.com/developing-today-forks/openchamber/pull/1379) | fix(web): paginate board-picker label/reviewer reads under named caps                                               | dezren39              | 2026-11-03 |
| [#1422](https://github.com/developing-today-forks/openchamber/pull/1422) | refactor(web): migrate route-overrides store to the durable-store primitive (#978)                                  | dezren39              | 2026-11-03 |
| [#1415](https://github.com/developing-today-forks/openchamber/pull/1415) | fix(web): surface Linear comments truncation via commentsMore (#1414)                                               | dezren39              | 2026-11-03 |
| [#1423](https://github.com/developing-today-forks/openchamber/pull/1423) | fix(web): surface reference-detail truncation for reviews, review threads and thread replies (#1419)                | dezren39              | 2026-11-03 |
| [#1426](https://github.com/developing-today-forks/openchamber/pull/1426) | fix(web): paginate /pulls/context check-run annotations under a named cap, flag truncation (#1420)                  | dezren39              | 2026-11-03 |
| [#1424](https://github.com/developing-today-forks/openchamber/pull/1424) | fix(web): GraphQL check summaries flag truncated contexts (#1418)                                                   | dezren39              | 2026-11-03 |
| [#1428](https://github.com/developing-today-forks/openchamber/pull/1428) | refactor(web): migrate session-warming store to the durable-store primitive (#978)                                  | dezren39              | 2026-11-03 |
| [#1406](https://github.com/developing-today-forks/openchamber/pull/1406) | refactor(web): migrate file-search and git-dirs walks to the shared traversal budget (#1393)                        | dezren39              | 2026-11-03 |
| [#1434](https://github.com/developing-today-forks/openchamber/pull/1434) | fix(web): write gh CLI settings before reconciling bindings on account removal (#1274)                              | dezren39              | 2026-11-03 |
| [#1451](https://github.com/developing-today-forks/openchamber/pull/1451) | test(web): cover checks-summary dedupe for not-yet-started re-runs (#1408)                                          | dezren39              | 2026-11-03 |
| [#780](https://github.com/developing-today-forks/openchamber/pull/780)   | fix(web): make the scheduled-task run limit configurable, default effectively unbounded                             | dezren39              | 2026-11-03 |
| [#1463](https://github.com/developing-today-forks/openchamber/pull/1463) | fix(web): classify xAI credential-read failures as shared upstream-auth failures (#1131)                            | dezren39              | 2026-11-03 |
| [#1474](https://github.com/developing-today-forks/openchamber/pull/1474) | feat(web): effective provider off state — policies and enabled_providers report with reasons (#218)                 | dezren39              | 2026-11-03 |
| [#1469](https://github.com/developing-today-forks/openchamber/pull/1469) | refactor(web): migrate tunnels managed-config store to the durable-store primitive (#978)                           | dezren39              | 2026-11-03 |
| [#1481](https://github.com/developing-today-forks/openchamber/pull/1481) | chore(web): drop the last private corrupt- spellings from store comments (#978)                                     | dezren39              | 2026-11-03 |
| [#1478](https://github.com/developing-today-forks/openchamber/pull/1478) | feat(web): expose queue binding on session-activity entries (#1473)                                                 | dezren39              | 2026-11-03 |
| [#1485](https://github.com/developing-today-forks/openchamber/pull/1485) | fix(web): gate work-status row and PR header checks display on unknown/incomplete summary state                     | dezren39              | 2026-11-03 |
| [#1491](https://github.com/developing-today-forks/openchamber/pull/1491) | fix(web): enforce guest asset and file caps during the read, not from the pre-read stat                             | dezren39              | 2026-11-03 |
| [#1492](https://github.com/developing-today-forks/openchamber/pull/1492) | fix(web): settle quick tunnel startup on early exit                                                                 | dezren39              | 2026-11-03 |
| [#1505](https://github.com/developing-today-forks/openchamber/pull/1505) | fix(web): dispatch-results readFile fails closed when the read loop exhausts attempts (#1502)                       | dezren39              | 2026-11-03 |
| [#1495](https://github.com/developing-today-forks/openchamber/pull/1495) | fix(web): cap /pulls/context ?diff=1 at 50 MiB — 413, never served or cached whole                                  | dezren39              | 2026-11-03 |
| [#1508](https://github.com/developing-today-forks/openchamber/pull/1508) | fix(web): Antigravity marks the shared upstream-auth credential-read failure (#1128)                                | dezren39              | 2026-11-03 |
| [#1483](https://github.com/developing-today-forks/openchamber/pull/1483) | fix(web): classify shared upstream-auth by origin marker, not error text (#1129)                                    | dezren39              | 2026-11-03 |
| [#1501](https://github.com/developing-today-forks/openchamber/pull/1501) | fix(web): apply the walkthrough store size bound in the prune read (#1482)                                          | dezren39              | 2026-11-03 |
| [#1480](https://github.com/developing-today-forks/openchamber/pull/1480) | fix(web): surface label and review-request totals past the reference caps (#1427)                                   | dezren39              | 2026-11-04 |
| [#1511](https://github.com/developing-today-forks/openchamber/pull/1511) | fix(web): dispatch-results register rejects when its write is refused (#1503)                                       | dezren39              | 2026-11-04 |
| [#1515](https://github.com/developing-today-forks/openchamber/pull/1515) | fix(web): reap idle finished sessions out of the live set (#791)                                                    | dezren39              | 2026-11-04 |
| [#1520](https://github.com/developing-today-forks/openchamber/pull/1520) | fix(web): handler-side 1009 frame guards for dictation, terminal and guest-surface sockets (#1464)                  | dezren39              | 2026-11-04 |
| [#1522](https://github.com/developing-today-forks/openchamber/pull/1522) | perf(web): TTL cache and in-flight dedup for the plan metrics reader (#1476)                                        | dezren39              | 2026-11-04 |
| [#1526](https://github.com/developing-today-forks/openchamber/pull/1526) | fix(web): treat missing or unparseable pairing expiresAt as expired                                                 | dezren39              | 2026-11-04 |
| [#1523](https://github.com/developing-today-forks/openchamber/pull/1523) | fix(web): warn when OPENCODE_HOST carries URL credentials that will be ignored (#1504)                              | dezren39              | 2026-11-04 |
| [#1516](https://github.com/developing-today-forks/openchamber/pull/1516) | fix(web): track external OpenCode health checks                                                                     | dezren39              | 2026-11-04 |
| [#1536](https://github.com/developing-today-forks/openchamber/pull/1536) | fix(web): protect linked sessions from idle reaper (#1527)                                                          | dezren39              | 2026-11-04 |
| [#1535](https://github.com/developing-today-forks/openchamber/pull/1535) | fix(web): judge the walkthrough patch cap during the read, not after buffering (#1456)                              | dezren39              | 2026-11-04 |
| [#1543](https://github.com/developing-today-forks/openchamber/pull/1543) | fix(web): open fs save identical-content check with O_NONBLOCK (#1533)                                              | dezren39              | 2026-11-04 |
| [#1541](https://github.com/developing-today-forks/openchamber/pull/1541) | test(web): pin both check-ignore readers with chunk-boundary decode tests (#1540)                                   | dezren39              | 2026-11-04 |
| [#1531](https://github.com/developing-today-forks/openchamber/pull/1531) | fix(web): APNs relay send chunks tokens past 100 instead of dropping them (#1500)                                   | dezren39              | 2026-11-04 |
| [#1544](https://github.com/developing-today-forks/openchamber/pull/1544) | fix(web): bound the project-icon serve read at 5 MiB with a regular-file check (#1537)                              | dezren39              | 2026-11-04 |
| [#1545](https://github.com/developing-today-forks/openchamber/pull/1545) | fix(web): match cloudflared output over a bounded rolling buffer, not per pipe chunk                                | dezren39              | 2026-11-04 |
| [#1538](https://github.com/developing-today-forks/openchamber/pull/1538) | feat(web): extract GitHub collection readers into lib/github/collections.js (#1529)                                 | dezren39              | 2026-11-04 |
| [#1547](https://github.com/developing-today-forks/openchamber/pull/1547) | fix(web): judge guest-install SSRF guard by IPv6 range, not fe80: prefix (#1517)                                    | dezren39              | 2026-11-04 |
| [#1548](https://github.com/developing-today-forks/openchamber/pull/1548) | fix(web): extract the ngrok URL from complete lines, never a per-chunk fragment (#1546)                             | dezren39              | 2026-11-04 |
| [#1552](https://github.com/developing-today-forks/openchamber/pull/1552) | test(web): poll bootstrap status in #881 oversized-timeout test (#1550)                                             | dezren39              | 2026-11-04 |
| [#1568](https://github.com/developing-today-forks/openchamber/pull/1568) | fix(web): scope PR-status search fallback to the resolved target repos                                              | dezren39              | 2026-11-04 |
| [#1539](https://github.com/developing-today-forks/openchamber/pull/1539) | fix(web): let an fs save open failure fall through to the atomic write (#1534)                                      | dezren39              | 2026-11-04 |
| [#1521](https://github.com/developing-today-forks/openchamber/pull/1521) | fix(web): verify CRC-32 in both hand-rolled ZIP readers (#1472)                                                     | dezren39              | 2026-11-04 |
| [#1581](https://github.com/developing-today-forks/openchamber/pull/1581) | fix(web): judge loopback by parsed range test at all three web-server sites                                         | dezren39              | 2026-11-04 |
| [#1583](https://github.com/developing-today-forks/openchamber/pull/1583) | fix(web): decode fs/exec output incrementally so split UTF-8 survives (#1567)                                       | dezren39              | 2026-11-04 |
| [#1589](https://github.com/developing-today-forks/openchamber/pull/1589) | fix(web): bound the plan summary cache with LRU eviction (#1486)                                                    | dezren39              | 2026-11-04 |
| [#1602](https://github.com/developing-today-forks/openchamber/pull/1602) | fix(web): bound the session-goal objective read during the read (#1582)                                             | dezren39              | 2026-11-04 |
| [#1554](https://github.com/developing-today-forks/openchamber/pull/1554) | fix(web): decode environment command output with StringDecoder and count the output cap in bytes (#1553)            | dezren39              | 2026-11-05 |
| [#1617](https://github.com/developing-today-forks/openchamber/pull/1617) | fix(web): return teamsMore from listLinearTeams when its page cap is hit (#1598)                                    | dezren39              | 2026-11-05 |
| [#1618](https://github.com/developing-today-forks/openchamber/pull/1618) | fix(web): route agent-memory and project-context readJson through the shared bounded reader                         | dezren39              | 2026-11-05 |
| [#1625](https://github.com/developing-today-forks/openchamber/pull/1625) | fix(web): route guest extension/auth store reads through the shared bounded reader                                  | dezren39              | 2026-11-05 |
| [#1626](https://github.com/developing-today-forks/openchamber/pull/1626) | fix(web): bound session-folders store reads with the shared bounded reader                                          | dezren39              | 2026-11-05 |
| [#1629](https://github.com/developing-today-forks/openchamber/pull/1629) | fix(web): bound Linear auth and shared settings JSON reads (#1615)                                                  | dezren39              | 2026-11-05 |
| [#1640](https://github.com/developing-today-forks/openchamber/pull/1640) | fix(web): bound space-archive entry reads at startup                                                                | dezren39              | 2026-11-05 |
| [#1658](https://github.com/developing-today-forks/openchamber/pull/1658) | fix(web): bound environment store reads with the shared bounded reader                                              | dezren39              | 2026-11-05 |
| [#1646](https://github.com/developing-today-forks/openchamber/pull/1646) | fix(web): bound plan notes reads                                                                                    | dezren39              | 2026-11-05 |
| [#1666](https://github.com/developing-today-forks/openchamber/pull/1666) | fix(web): bound github/auth.js reads with per-file caps (#1651)                                                     | developing-today[bot] | 2026-11-05 |
| [#1667](https://github.com/developing-today-forks/openchamber/pull/1667) | fix(web): host-providers loopback judge reuses shared bind-host judge (#1657)                                       | dezren39              | 2026-11-05 |
| [#1668](https://github.com/developing-today-forks/openchamber/pull/1668) | fix(web): space-setting-file reads through the bounded handle reader (#1659)                                        | dezren39              | 2026-11-05 |
| [#1669](https://github.com/developing-today-forks/openchamber/pull/1669) | fix(web): size-gate the git identity store read on its open handle (#1635)                                          | dezren39              | 2026-11-05 |
| [#1670](https://github.com/developing-today-forks/openchamber/pull/1670) | fix(web): bound source-control store state reads via readBoundedFile (#1663)                                        | dezren39              | 2026-11-05 |
| [#1671](https://github.com/developing-today-forks/openchamber/pull/1671) | fix(web): quota credentials store bounded sync read (#1613)                                                         | dezren39              | 2026-11-05 |
| [#1673](https://github.com/developing-today-forks/openchamber/pull/1673) | fix(web): bound settings-runtime reads at 8 MiB (#1653)                                                             | dezren39              | 2026-11-05 |
| [#1676](https://github.com/developing-today-forks/openchamber/pull/1676) | fix(web): bound durable-json-store reads with cap and regular-file check (#1649)                                    | dezren39              | 2026-11-05 |
| [#1681](https://github.com/developing-today-forks/openchamber/pull/1681) | docs(web): state-file reader both-halves inventory (design #1655 phase 2, #1656)                                    | dezren39              | 2026-11-05 |
| [#1683](https://github.com/developing-today-forks/openchamber/pull/1683) | fix(web): state byte caps for git provenance and SSH credential store reads                                         | dezren39              | 2026-11-06 |
| [#1688](https://github.com/developing-today-forks/openchamber/pull/1688) | test(web): add grows-after-stat coverage for stored-icon GET route (#1686)                                          | dezren39              | 2026-11-06 |
| [#1698](https://github.com/developing-today-forks/openchamber/pull/1698) | fix(web): enforce the 2 MiB cap in flight in git network-operation and worktree-bootstrap stores                    | dezren39              | 2026-11-06 |
| [#1704](https://github.com/developing-today-forks/openchamber/pull/1704) | fix(web): bound Linear mapping and session-status store reads (#1621)                                               | dezren39              | 2026-11-06 |
| [#1708](https://github.com/developing-today-forks/openchamber/pull/1708) | fix(web): paginate the per-head pulls.list read in pr-status.js under a named cap (#1596)                           | dezren39              | 2026-11-06 |
| [#1712](https://github.com/developing-today-forks/openchamber/pull/1712) | fix(web): bound the post-checkout hook inspection read (#1701)                                                      | dezren39              | 2026-11-06 |
| [#1713](https://github.com/developing-today-forks/openchamber/pull/1713) | fix(web): judge the walkthrough cache cap during the read (#1685)                                                   | dezren39              | 2026-11-06 |
| [#1715](https://github.com/developing-today-forks/openchamber/pull/1715) | fix(web): flush cloudflare accumulator on stream end and exit so the final unterminated line is classified (#1699)  | dezren39              | 2026-11-06 |
| [#1700](https://github.com/developing-today-forks/openchamber/pull/1700) | fix(web): preserve UTF-8 dictation worker stderr                                                                    | dezren39              | 2026-11-06 |
| [#1707](https://github.com/developing-today-forks/openchamber/pull/1707) | fix(web): classify Antigravity credential-read failure by origin marker (#1675)                                     | dezren39              | 2026-11-06 |
| [#1716](https://github.com/developing-today-forks/openchamber/pull/1716) | fix(web): open the source-control lock file non-blocking and bound the reclaim read (#1714)                         | dezren39              | 2026-11-06 |
| [#1717](https://github.com/developing-today-forks/openchamber/pull/1717) | fix(web): judge /pulls/context diff cap during the read (#1664)                                                     | dezren39              | 2026-11-06 |
| [#1719](https://github.com/developing-today-forks/openchamber/pull/1719) | fix(web): decode dev-server scan output with StringDecoder and cap it in bytes (#1691)                              | dezren39              | 2026-11-06 |
| [#1720](https://github.com/developing-today-forks/openchamber/pull/1720) | fix(web): guest extension storage judges its byte cap on the opened handle in flight (#1680)                        | dezren39              | 2026-11-06 |
| [#1723](https://github.com/developing-today-forks/openchamber/pull/1723) | fix(web): theme runtime judges its 512 KiB cap on the opened handle in flight                                       | dezren39              | 2026-11-06 |
| [#1724](https://github.com/developing-today-forks/openchamber/pull/1724) | fix(web): route relay key-store readStore through the shared bounded reader                                         | dezren39              | 2026-11-06 |
| [#1725](https://github.com/developing-today-forks/openchamber/pull/1725) | fix(web): truncate APNs collapse-id to 64 UTF-8 bytes, not 64 characters                                            | dezren39              | 2026-11-06 |
| [#1748](https://github.com/developing-today-forks/openchamber/pull/1748) | fix(web): bound managed OpenCode state and draining reads                                                           | dezren39              | 2026-11-07 |
| [#1749](https://github.com/developing-today-forks/openchamber/pull/1749) | fix(web): bound routing plan, pool-link, and feedback reads                                                         | developing-today[bot] | 2026-11-07 |
| [#1750](https://github.com/developing-today-forks/openchamber/pull/1750) | test(web): cover ngrok stdout stream-end flush for an unterminated final URL line                                   | developing-today[bot] | 2026-11-07 |
| [#1752](https://github.com/developing-today-forks/openchamber/pull/1752) | refactor(web): extract shared bounded output accumulator (#1705)                                                    | developing-today[bot] | 2026-11-07 |
| [#1758](https://github.com/developing-today-forks/openchamber/pull/1758) | fix(web): read managed-process registry entries and claims through the bounded reader (#1736)                       | dezren39              | 2026-11-07 |
| [#1763](https://github.com/developing-today-forks/openchamber/pull/1763) | docs(web): stream-output inventory for design #1740 phase 2 (#1741)                                                 | dezren39              | 2026-11-07 |
| [#1769](https://github.com/developing-today-forks/openchamber/pull/1769) | fix(web): read relay host claim through the bounded reader (#1739)                                                  | dezren39              | 2026-11-07 |
| [#1771](https://github.com/developing-today-forks/openchamber/pull/1771) | fix(web): judge repository-credential query cap in bytes, not characters (#1742)                                    | dezren39              | 2026-11-07 |
| [#1775](https://github.com/developing-today-forks/openchamber/pull/1775) | test(web): pin git identity store in-flight cap with grows-after-stat case                                          | dezren39              | 2026-11-07 |
| [#1792](https://github.com/developing-today-forks/openchamber/pull/1792) | fix(web): bound the mcpx daemon discovery read (#1773)                                                              | dezren39              | 2026-11-08 |
| [#1709](https://github.com/developing-today-forks/openchamber/pull/1709) | fix(web): bounded reads for CLI state files and a bounded tail window for readTailLines (#1706)                     | dezren39              | 2026-11-08 |
| [#1795](https://github.com/developing-today-forks/openchamber/pull/1795) | fix(web): bound the legacy named-tunnels migration read (#1692)                                                     | dezren39              | 2026-11-08 |
| [#1797](https://github.com/developing-today-forks/openchamber/pull/1797) | fix(web): decode guest service output through StringDecoder (#1696)                                                 | dezren39              | 2026-11-08 |
| [#1728](https://github.com/developing-today-forks/openchamber/pull/1728) | fix(web): preserve UTF-8 tar extraction errors for dictation models                                                 | dezren39              | 2026-11-08 |
| [#1776](https://github.com/developing-today-forks/openchamber/pull/1776) | fix(web): route eight quota providers through a shared credential helper (#1751)                                    | dezren39              | 2026-11-08 |
| [#1802](https://github.com/developing-today-forks/openchamber/pull/1802) | fix(web): decode gatekeeper control body as one UTF-8 stream, cap judged in bytes                                   | dezren39              | 2026-11-08 |
| [#1721](https://github.com/developing-today-forks/openchamber/pull/1721) | fix(web): bound the Linear issue labels connection and report labelsMore (#1599)                                    | dezren39              | 2026-11-08 |
| [#1826](https://github.com/developing-today-forks/openchamber/pull/1826) | refactor(web): remove conditional spreads from session routes (#1437)                                               | dezren39              | 2026-11-08 |
| [#1703](https://github.com/developing-today-forks/openchamber/pull/1703) | fix(web): decode content-search ripgrep stderr through a StringDecoder and enforce its 64 KiB cap in bytes          | dezren39              | 2026-11-08 |
| [#1798](https://github.com/developing-today-forks/openchamber/pull/1798) | fix(web): bound the UI-auth JWT secret-file read (#1778)                                                            | developing-today[bot] | 2026-11-08 |
| [#1827](https://github.com/developing-today-forks/openchamber/pull/1827) | fix(web): bound quota auth-file readers with readBoundedFileSync (#1645)                                            | developing-today[bot] | 2026-11-08 |
| [#1864](https://github.com/developing-today-forks/openchamber/pull/1864) | fix(web): guest local-zip install reads through the bounded handle reader                                           | dezren39              | 2026-11-08 |
| [#1865](https://github.com/developing-today-forks/openchamber/pull/1865) | fix(web): pr-status fallback search survives an inaccessible network repo (422)                                     | dezren39              | 2026-11-08 |
| [#1800](https://github.com/developing-today-forks/openchamber/pull/1800) | fix(web): reject ordinary message-queue enqueue at the per-session cap instead of evicting the oldest item          | dezren39              | 2026-11-08 |
| [#1729](https://github.com/developing-today-forks/openchamber/pull/1729) | fix(web): prevent managed SSH key FIFO open hangs                                                                   | dezren39              | 2026-11-08 |
| [#1801](https://github.com/developing-today-forks/openchamber/pull/1801) | fix(web): plan reader must not block on a FIFO — O_NONBLOCK open and regular-file judged from the handle            | dezren39              | 2026-11-08 |
| [#1807](https://github.com/developing-today-forks/openchamber/pull/1807) | fix(web): read the cloudflared managed-local config through the shared bounded sync reader                          | developing-today[bot] | 2026-11-08 |
| [#1806](https://github.com/developing-today-forks/openchamber/pull/1806) | fix(web): bound the .npmrc read at 64 KiB via readBoundedFileSync                                                   | developing-today[bot] | 2026-11-08 |
| [#1837](https://github.com/developing-today-forks/openchamber/pull/1837) | fix(web): bound tracked-items persistence load() with 8 MiB cap (#1662)                                             | developing-today[bot] | 2026-11-08 |
| [#1702](https://github.com/developing-today-forks/openchamber/pull/1702) | fix(web): flush ngrok stderr accumulator in onExit so the final unterminated line reaches the startup error (#1693) | dezren39              | 2026-11-08 |
| [#1810](https://github.com/developing-today-forks/openchamber/pull/1810) | fix(web): bound Muse login-store read (#1774)                                                                       | dezren39              | 2026-11-08 |
| [#1863](https://github.com/developing-today-forks/openchamber/pull/1863) | fix(web): forward managed cloudflare stderr through a StringDecoder                                                 | dezren39              | 2026-11-08 |
| [#1915](https://github.com/developing-today-forks/openchamber/pull/1915) | fix(web): measure the merged session-folders snapshot before persisting it (#1761)                                  | developing-today[bot] | 2026-11-08 |
| [#1869](https://github.com/developing-today-forks/openchamber/pull/1869) | fix(web): flush ngrok stdout accumulator in onExit                                                                  | dezren39              | 2026-11-08 |
| [#1916](https://github.com/developing-today-forks/openchamber/pull/1916) | fix(web): non-blocking open + in-flight caps for provenance and SSH credential stores                               | dezren39              | 2026-11-08 |
| [#1934](https://github.com/developing-today-forks/openchamber/pull/1934) | fix(web): reap permanently unreadable managed-process claims by filename pid                                        | developing-today[bot] | 2026-11-08 |
| [#1937](https://github.com/developing-today-forks/openchamber/pull/1937) | fix(web): route feedback compaction preserves concurrent appends                                                    | developing-today[bot] | 2026-11-08 |
| [#1939](https://github.com/developing-today-forks/openchamber/pull/1939) | fix(web): relay writeClaim judges the opened handle, not the probe                                                  | developing-today[bot] | 2026-11-08 |
| [#1925](https://github.com/developing-today-forks/openchamber/pull/1925) | fix(web): add a total-bytes bound to the provider read cache (#1770)                                                | developing-today[bot] | 2026-11-08 |
| [#1941](https://github.com/developing-today-forks/openchamber/pull/1941) | fix(web): bounded-output accumulator validates maxChars                                                             | developing-today[bot] | 2026-11-08 |
| [#1928](https://github.com/developing-today-forks/openchamber/pull/1928) | fix(web): report unreadable plan stores as unreadable, not malformed JSON                                           | developing-today[bot] | 2026-11-08 |
| [#1942](https://github.com/developing-today-forks/openchamber/pull/1942) | fix(web): open theme files nonblocking so a FIFO cannot hang theme loading                                          | developing-today[bot] | 2026-11-08 |
| [#1943](https://github.com/developing-today-forks/openchamber/pull/1943) | fix(web): markdown image grant inspection opens nonblocking                                                         | developing-today[bot] | 2026-11-08 |
| [#1944](https://github.com/developing-today-forks/openchamber/pull/1944) | fix(web): guest file read opens nonblocking                                                                         | developing-today[bot] | 2026-11-08 |
| [#1945](https://github.com/developing-today-forks/openchamber/pull/1945) | fix(web): instance-dir owner read is bounded and handle-judged                                                      | developing-today[bot] | 2026-11-08 |
| [#1946](https://github.com/developing-today-forks/openchamber/pull/1946) | fix(web): project-icon discovery opens favicon nonblocking and refuses non-regular files                            | developing-today[bot] | 2026-11-08 |
| [#1954](https://github.com/developing-today-forks/openchamber/pull/1954) | fix(web): advance managed log cursor past invalid UTF-8 leads                                                       | dezren39              | 2026-11-08 |
| [#1950](https://github.com/developing-today-forks/openchamber/pull/1950) | fix(web): buildSnippet cuts never split a surrogate pair                                                            | developing-today[bot] | 2026-11-08 |
| [#1955](https://github.com/developing-today-forks/openchamber/pull/1955) | fix(web): keep session-goal auditor excerpts off surrogate-pair boundaries                                          | dezren39              | 2026-11-08 |
| [#1956](https://github.com/developing-today-forks/openchamber/pull/1956) | fix(web): keep normalizeGoalSpec clips off surrogate pairs (#1907)                                                  | dezren39              | 2026-11-08 |
| [#1952](https://github.com/developing-today-forks/openchamber/pull/1952) | fix(web): bound the guest storage read through the shared handle-judged reader (#1784)                              | dezren39              | 2026-11-08 |
| [#1963](https://github.com/developing-today-forks/openchamber/pull/1963) | fix(web): gate the managed log cursor open and cap its per-call read (#1794)                                        | developing-today[bot] | 2026-11-08 |
| [#1965](https://github.com/developing-today-forks/openchamber/pull/1965) | fix(web): make routing history excerpts surrogate-pair safe (#1910)                                                 | developing-today[bot] | 2026-11-08 |
| [#1966](https://github.com/developing-today-forks/openchamber/pull/1966) | fix(web): align the guest clone stderr tail to a UTF-8 boundary (#1811)                                             | developing-today[bot] | 2026-11-08 |
| [#1969](https://github.com/developing-today-forks/openchamber/pull/1969) | fix(web): judge the jwt-secret writes on the opened handle (#1940)                                                  | developing-today[bot] | 2026-11-08 |
| [#1970](https://github.com/developing-today-forks/openchamber/pull/1970) | fix(web): make fitObjective and the inline objective cut surrogate-pair safe (#1909)                                | developing-today[bot] | 2026-11-08 |
| [#1971](https://github.com/developing-today-forks/openchamber/pull/1971) | fix(web): make the session-assist excerpt surrogate-pair safe (#1911)                                               | developing-today[bot] | 2026-11-08 |
| [#1972](https://github.com/developing-today-forks/openchamber/pull/1972) | fix(web): redact header and bare-assignment credentials in redactSensitiveText (#1920)                              | developing-today[bot] | 2026-11-08 |
| [#1974](https://github.com/developing-today-forks/openchamber/pull/1974) | test(web): re-point the host-shutdown store spy at the bounded reader open (#1796)                                  | developing-today[bot] | 2026-11-08 |
| [#1978](https://github.com/developing-today-forks/openchamber/pull/1978) | fix(web): keep session-knowledge truncation pair-safe and inside its budget (#1921)                                 | developing-today[bot] | 2026-11-08 |
| [#1979](https://github.com/developing-today-forks/openchamber/pull/1979) | fix(web): keep notification text truncation surrogate-pair safe (#1922)                                             | developing-today[bot] | 2026-11-08 |
| [#1980](https://github.com/developing-today-forks/openchamber/pull/1980) | fix(web): keep the small-model prompt clamp pair-safe and inside its budget (#1931)                                 | developing-today[bot] | 2026-11-08 |
| [#1981](https://github.com/developing-today-forks/openchamber/pull/1981) | fix(web): keep summarization fallback clippers surrogate-pair safe (#1932)                                          | developing-today[bot] | 2026-11-08 |
| [#1977](https://github.com/developing-today-forks/openchamber/pull/1977) | fix(web): keep the dispatch-result 50,000-character cut surrogate-pair safe (#1908)                                 | developing-today[bot] | 2026-11-08 |
| [#1983](https://github.com/developing-today-forks/openchamber/pull/1983) | fix(web): keep notification last-message extraction surrogate-pair safe (#1948)                                     | developing-today[bot] | 2026-11-08 |
| [#1985](https://github.com/developing-today-forks/openchamber/pull/1985) | fix(web): keep the session-goal objective clamp surrogate-pair safe (#1949)                                         | developing-today[bot] | 2026-11-08 |
| [#1986](https://github.com/developing-today-forks/openchamber/pull/1986) | fix(web): keep the session-link title cut surrogate-pair safe (#1975)                                               | developing-today[bot] | 2026-11-08 |
| [#1995](https://github.com/developing-today-forks/openchamber/pull/1995) | fix(web): read every routing-store JSON file through the bounded reader (#1608)                                     | dezren39              | 2026-11-08 |
| [#1998](https://github.com/developing-today-forks/openchamber/pull/1998) | fix(web): keep client-auth device labels pair-safe at the 80-char clamp (#1993)                                     | dezren39              | 2026-11-08 |
| [#1997](https://github.com/developing-today-forks/openchamber/pull/1997) | fix(web): keep the agent-memory title clamp surrogate-pair safe (#1991)                                             | developing-today[bot] | 2026-11-08 |
| [#1989](https://github.com/developing-today-forks/openchamber/pull/1989) | fix(web): keep the scheduled-task length clamp surrogate-pair safe (#1976)                                          | developing-today[bot] | 2026-11-08 |
| [#1992](https://github.com/developing-today-forks/openchamber/pull/1992) | fix(web): keep bounded-output accumulator tail cuts surrogate-pair safe (#1803)                                     | developing-today[bot] | 2026-11-08 |
| [#2000](https://github.com/developing-today-forks/openchamber/pull/2000) | fix(web): keep the GitHub reference preview body cut surrogate-pair safe (#1957)                                    | developing-today[bot] | 2026-11-08 |
| [#2001](https://github.com/developing-today-forks/openchamber/pull/2001) | fix(web): keep queued-message context preview cuts surrogate-pair safe (#1959)                                      | developing-today[bot] | 2026-11-08 |
| [#2002](https://github.com/developing-today-forks/openchamber/pull/2002) | fix(web): stop GitLab branch-status and create-reconciliation reads treating page one as the whole set (#1597)      | developing-today[bot] | 2026-11-08 |
| [#1994](https://github.com/developing-today-forks/openchamber/pull/1994) | fix(web): read the APNs .p8 key file through the bounded reader (#1777)                                             | developing-today[bot] | 2026-11-08 |
| [#2005](https://github.com/developing-today-forks/openchamber/pull/2005) | test(web): pin settings-runtime grows-after-stat gate                                                               | dezren39              | 2026-11-08 |
| [#2007](https://github.com/developing-today-forks/openchamber/pull/2007) | test(web): pin environment runtime split two-byte, trailing incomplete UTF-8, and cumulative cap cases (#1819)      | developing-today[bot] | 2026-11-08 |
| [#2009](https://github.com/developing-today-forks/openchamber/pull/2009) | docs(web): document and pin recordFeedback 3 MiB per-entry limit (#1791)                                            | dezren39              | 2026-11-08 |
| [#2006](https://github.com/developing-today-forks/openchamber/pull/2006) | test(web): pin ngrok stderr stream-end flush                                                                        | dezren39              | 2026-11-08 |
| [#2004](https://github.com/developing-today-forks/openchamber/pull/2004) | test(web): pin durable-json-store in-flight cap with grows-after-stat tests (#1772)                                 | developing-today[bot] | 2026-11-08 |
| [#2011](https://github.com/developing-today-forks/openchamber/pull/2011) | fix(web): make project-context clampLength pair-safe (#2010)                                                        | dezren39              | 2026-11-08 |
| [#2020](https://github.com/developing-today-forks/openchamber/pull/2020) | fix(web): keep guest account clips surrogate-safe (#2013)                                                           | dezren39              | 2026-11-08 |
| [#2024](https://github.com/developing-today-forks/openchamber/pull/2024) | fix(web): keep walkthrough chapter-title clamp off surrogate pairs (#2016)                                          | dezren39              | 2026-11-08 |
| [#2025](https://github.com/developing-today-forks/openchamber/pull/2025) | fix(web): keep session-assist recap/suggestion clamps off surrogate pairs (#2017)                                   | dezren39              | 2026-11-08 |
| [#2026](https://github.com/developing-today-forks/openchamber/pull/2026) | fix(web): keep scheduled-task session-title cut off surrogate pairs (#2019)                                         | dezren39              | 2026-11-08 |
| [#2035](https://github.com/developing-today-forks/openchamber/pull/2035) | fix(web): make PWA manifest name clamps pair-safe                                                                   | developing-today[bot] | 2026-11-09 |
| [#2036](https://github.com/developing-today-forks/openchamber/pull/2036) | fix(web): make client PWA manifest clamps pair-safe                                                                 | developing-today[bot] | 2026-11-09 |
| [#2039](https://github.com/developing-today-forks/openchamber/pull/2039) | fix(web): make settings sanitizer clamps pair-safe                                                                  | developing-today[bot] | 2026-11-09 |
| [#2040](https://github.com/developing-today-forks/openchamber/pull/2040) | fix(web): align spaces runCommand byte tails to UTF-8 boundaries                                                    | developing-today[bot] | 2026-11-09 |
| [#2045](https://github.com/developing-today-forks/openchamber/pull/2045) | fix(web): bound skills-catalog store reads with the shared bounded reader                                           | developing-today[bot] | 2026-11-09 |
| [#2046](https://github.com/developing-today-forks/openchamber/pull/2046) | fix(web): align managed OpenCode log tails to UTF-8 boundaries                                                      | developing-today[bot] | 2026-11-09 |
| [#2047](https://github.com/developing-today-forks/openchamber/pull/2047) | fix(web): keep passkey label clamp from splitting a surrogate pair (#2012)                                          | dezren39              | 2026-11-09 |
| [#2048](https://github.com/developing-today-forks/openchamber/pull/2048) | fix(web): keep guest settings clamp from splitting a surrogate pair (#2014)                                         | dezren39              | 2026-11-09 |
| [#2034](https://github.com/developing-today-forks/openchamber/pull/2034) | fix(web): return Request timed out when the Cursor token refresh stalls (#2018)                                     | dezren39              | 2026-11-09 |
| [#2051](https://github.com/developing-today-forks/openchamber/pull/2051) | fix(web): session-folders PATCH guard measures the persisted pretty form                                            | developing-today[bot] | 2026-11-09 |
| [#2052](https://github.com/developing-today-forks/openchamber/pull/2052) | fix(web): spaces output tails never start inside a surrogate pair                                                   | developing-today[bot] | 2026-11-09 |
| [#2054](https://github.com/developing-today-forks/openchamber/pull/2054) | fix(web): tracked-items persistence residuals — bound docs, FIFO test, injected fs (#2015)                          | dezren39              | 2026-11-09 |
| [#2083](https://github.com/developing-today-forks/openchamber/pull/2083) | fix(web): redact scheme-headed and quoted-JSON credentials in both redactors (#2074)                                | dezren39              | 2026-11-11 |
| [#2084](https://github.com/developing-today-forks/openchamber/pull/2084) | test(web): pin the docker-tools fill-failure tail to a character boundary (#2080)                                   | dezren39              | 2026-11-11 |
| [#2090](https://github.com/developing-today-forks/openchamber/pull/2090) | fix(web): read model-choices through the bounded reader and bound decisions (#1610)                                 | dezren39              | 2026-11-11 |
| [#2100](https://github.com/developing-today-forks/openchamber/pull/2100) | fix(web): followFile carries undecoded bytes across polls (#2059)                                                   | dezren39              | 2026-11-11 |
| [#2087](https://github.com/developing-today-forks/openchamber/pull/2087) | fix(web): bound OpenCode service environment reads                                                                  | dezren39              | 2026-11-11 |
| [#2101](https://github.com/developing-today-forks/openchamber/pull/2101) | fix(web): source-control stores read state through the injected fs (#2063)                                          | dezren39              | 2026-11-11 |
| [#2088](https://github.com/developing-today-forks/openchamber/pull/2088) | fix(web): bound durable replay store loading                                                                        | dezren39              | 2026-11-11 |
| [#2108](https://github.com/developing-today-forks/openchamber/pull/2108) | fix(web): do not dispatch pre-cancelled browser-control requests                                                    | dezren39              | 2026-11-11 |
| [#2107](https://github.com/developing-today-forks/openchamber/pull/2107) | fix(web): enforce session folder ID uniqueness                                                                      | dezren39              | 2026-11-11 |
| [#2109](https://github.com/developing-today-forks/openchamber/pull/2109) | fix(web): skip tombstoned ids when remapping imported folder collisions                                             | dezren39              | 2026-11-11 |
| [#2119](https://github.com/developing-today-forks/openchamber/pull/2119) | fix(web): bound scheduled-task run history retention and startup load                                               | dezren39              | 2026-11-11 |
| [#2122](https://github.com/developing-today-forks/openchamber/pull/2122) | fix(web): reject repeated git directory query parameters                                                            | dezren39              | 2026-11-11 |
| [#2135](https://github.com/developing-today-forks/openchamber/pull/2135) | fix(web): preserve image grant fence state across text parts                                                        | dezren39              | 2026-11-12 |
| [#2139](https://github.com/developing-today-forks/openchamber/pull/2139) | fix(web): bound session linked-issue batches and metadata growth                                                    | dezren39              | 2026-11-12 |
| [#2129](https://github.com/developing-today-forks/openchamber/pull/2129) | fix(web): redact single-quoted credential shapes                                                                    | dezren39              | 2026-11-12 |
| [#2270](https://github.com/developing-today-forks/openchamber/pull/2270) | fix(web): mistral quota provider picks its key with asNonEmptyString (#1747)                                        | dezren39              | 2026-11-13 |
| [#2273](https://github.com/developing-today-forks/openchamber/pull/2273) | fix(web): zhipuai-coding-plan quota provider selects its key through asNonEmptyString (#2272)                       | dezren39              | 2026-11-13 |
| [#2276](https://github.com/developing-today-forks/openchamber/pull/2276) | fix(web): bounded read for readMergedSettingsSync (#1631)                                                           | dezren39              | 2026-11-14 |
| [#2277](https://github.com/developing-today-forks/openchamber/pull/2277) | fix(web): exhaust the three canonical mutation reconciliation reads under the collections cap (#1595)               | dezren39              | 2026-11-14 |
| [#2278](https://github.com/developing-today-forks/openchamber/pull/2278) | fix(web): replace literal NULs in run history source (#2115, replaces #2117)                                        | dezren39              | 2026-11-14 |
| [#2333](https://github.com/developing-today-forks/openchamber/pull/2333) | feat(web): watcherReady owned by connection state AND first payload (#1441)                                         | dezren39              | 2026-11-17 |
| [#2336](https://github.com/developing-today-forks/openchamber/pull/2336) | fix(web): make scheduled run-history compaction failure-atomic (#2120)                                              | dezren39              | 2026-11-17 |
| [#2337](https://github.com/developing-today-forks/openchamber/pull/2337) | fix(web): ignore multiline inline-code image references in grants                                                   | dezren39              | 2026-11-17 |
| [#2338](https://github.com/developing-today-forks/openchamber/pull/2338) | fix(web): reject repeated project directory query parameters                                                        | dezren39              | 2026-11-17 |
| [#2341](https://github.com/developing-today-forks/openchamber/pull/2341) | fix(web): do not advance durable replay state after an append failure (#2111)                                       | dezren39              | 2026-11-17 |
| [#2339](https://github.com/developing-today-forks/openchamber/pull/2339) | fix(web): ignore HTML-comment image references in grants (#2142)                                                    | dezren39              | 2026-11-17 |
| [#2342](https://github.com/developing-today-forks/openchamber/pull/2342) | fix(web): reject malformed scheduled-task run-history time filters                                                  | dezren39              | 2026-11-17 |
| [#2340](https://github.com/developing-today-forks/openchamber/pull/2340) | fix(web): ignore images inside indented code blocks in grants (#2151)                                               | dezren39              | 2026-11-17 |
| [#2344](https://github.com/developing-today-forks/openchamber/pull/2344) | fix(web): reject malformed scheduled-task run-history limits                                                        | dezren39              | 2026-11-17 |
| [#2343](https://github.com/developing-today-forks/openchamber/pull/2343) | fix(web): prevent split surrogate pairs in content-search previews (#2152)                                          | dezren39              | 2026-11-17 |
| [#2348](https://github.com/developing-today-forks/openchamber/pull/2348) | fix(web): announce collapse-only changes from legacy session-folder POSTs                                           | dezren39              | 2026-11-17 |
| [#2347](https://github.com/developing-today-forks/openchamber/pull/2347) | fix(web): do not resurrect evicted workspace file-name cache entries (#2161)                                        | dezren39              | 2026-11-17 |
| [#2350](https://github.com/developing-today-forks/openchamber/pull/2350) | fix(web): preserve file mode when overwriting uploads (#2249)                                                       | dezren39              | 2026-11-17 |
| [#2346](https://github.com/developing-today-forks/openchamber/pull/2346) | fix(web): ignore images inside raw HTML blocks in grants (#2158)                                                    | dezren39              | 2026-11-17 |
| [#2351](https://github.com/developing-today-forks/openchamber/pull/2351) | fix(web): keep a complete first row when the run-history window starts on a line boundary (#2267)                   | dezren39              | 2026-11-17 |
| [#2354](https://github.com/developing-today-forks/openchamber/pull/2354) | fix(web): reject repeated directory query params in source-control routes (#2292)                                   | dezren39              | 2026-11-17 |
| [#2357](https://github.com/developing-today-forks/openchamber/pull/2357) | fix(web): bounded FIFO-safe read for session-goal version history                                                   | dezren39              | 2026-11-17 |
| [#2356](https://github.com/developing-today-forks/openchamber/pull/2356) | fix(web): resolve tar and sync via PATH in the tools filler                                                         | dezren39              | 2026-11-17 |
| [#2360](https://github.com/developing-today-forks/openchamber/pull/2360) | fix(web): config-v2 section entry helpers resolve only own properties                                               | dezren39              | 2026-11-17 |
| [#2365](https://github.com/developing-today-forks/openchamber/pull/2365) | fix(web): bound session-knowledge pin ids and pin counts                                                            | dezren39              | 2026-11-17 |
| [#2367](https://github.com/developing-today-forks/openchamber/pull/2367) | fix(web): refuse repeated url params in realtime-proxy                                                              | dezren39              | 2026-11-17 |
| [#2364](https://github.com/developing-today-forks/openchamber/pull/2364) | fix(web): reject repeated query params on walkthrough pr-diff/pr-file (#2312)                                       | dezren39              | 2026-11-17 |
| [#2368](https://github.com/developing-today-forks/openchamber/pull/2368) | fix(web): atomic session-goal objective and version-history writes                                                  | dezren39              | 2026-11-17 |
| [#2370](https://github.com/developing-today-forks/openchamber/pull/2370) | fix(web): re-arm session-goal timer after a failed continuation (#2314)                                             | dezren39              | 2026-11-17 |
| [#2369](https://github.com/developing-today-forks/openchamber/pull/2369) | fix(web): refuse repeated directory/lastEventId on message-stream WS                                                | dezren39              | 2026-11-17 |
| [#2361](https://github.com/developing-today-forks/openchamber/pull/2361) | fix(web): prevent command injection in /api/tts/say/speak voice and rate handling                                   | dezren39              | 2026-11-17 |
| [#2358](https://github.com/developing-today-forks/openchamber/pull/2358) | fix(web): reject symlinked favicons escaping the project in icon discovery (#2294)                                  | dezren39              | 2026-11-17 |
| [#2372](https://github.com/developing-today-forks/openchamber/pull/2372) | fix(web): bound space record reads via readBoundedFileSync (#1628)                                                  | dezren39              | 2026-11-17 |
| [#2366](https://github.com/developing-today-forks/openchamber/pull/2366) | fix(web): give space archive staging a total byte budget (#2321)                                                    | dezren39              | 2026-11-17 |
| [#2373](https://github.com/developing-today-forks/openchamber/pull/2373) | fix(web): bound git status new-file line counting by the read, not the stat (#1730)                                 | dezren39              | 2026-11-17 |
| [#2381](https://github.com/developing-today-forks/openchamber/pull/2381) | fix(web): bound project-context todo IDs before writing context.json (#2113)                                        | dezren39              | 2026-11-17 |
| [#2380](https://github.com/developing-today-forks/openchamber/pull/2380) | fix(web): reserve in-flight login attempts against the per-IP failure budget                                        | dezren39              | 2026-11-17 |
| [#2383](https://github.com/developing-today-forks/openchamber/pull/2383) | fix(web): honor Markdown closing-fence rules in image grants (#2150)                                                | dezren39              | 2026-11-17 |
| [#2385](https://github.com/developing-today-forks/openchamber/pull/2385) | fix(web): follow message pages when resolving notification content (#2154)                                          | dezren39              | 2026-11-17 |
| [#2388](https://github.com/developing-today-forks/openchamber/pull/2388) | fix(web): reject repeated/invalid directory and remote query params in git routes                                   | dezren39              | 2026-11-17 |
| [#2394](https://github.com/developing-today-forks/openchamber/pull/2394) | test(web): isolate config-discovery suites from machine-global OpenCode state (#2309)                               | dezren39              | 2026-11-17 |
| [#2395](https://github.com/developing-today-forks/openchamber/pull/2395) | fix(web): cancel pending question push on form.settled / page-settled (#2313)                                       | dezren39              | 2026-11-17 |
| [#2389](https://github.com/developing-today-forks/openchamber/pull/2389) | fix(web): prevent blocking FIFO opens in /api/fs/raw                                                                | dezren39              | 2026-11-17 |
| [#2396](https://github.com/developing-today-forks/openchamber/pull/2396) | fix(web): guard active-session status lookups against Object.prototype members (#2311)                              | dezren39              | 2026-11-17 |
| [#2391](https://github.com/developing-today-forks/openchamber/pull/2391) | fix(web): prevent settings reads hanging when a regular file is swapped for a FIFO                                  | dezren39              | 2026-11-17 |
| [#2408](https://github.com/developing-today-forks/openchamber/pull/2408) | refactor(web): remove conditional spreads from control (#1437)                                                      | dezren39              | 2026-11-18 |
| [#2414](https://github.com/developing-today-forks/openchamber/pull/2414) | fix(web): skip model-choices FIFO test on win32 (#2376)                                                             | dezren39              | 2026-11-18 |
| [#2412](https://github.com/developing-today-forks/openchamber/pull/2412) | test(web): remove no-module-mocking findings from batch B                                                           | dezren39              | 2026-11-20 |
| [#2428](https://github.com/developing-today-forks/openchamber/pull/2428) | fix(web): reject repeated scope/source filters on skill delete (#2419)                                              | dezren39              | 2026-11-20 |
| [#2415](https://github.com/developing-today-forks/openchamber/pull/2415) | docs(web): clarify model-choice decision eviction                                                                   | dezren39              | 2026-11-20 |
| [#2443](https://github.com/developing-today-forks/openchamber/pull/2443) | fix(web): make AGENTS.md replacement atomic                                                                         | dezren39              | 2026-11-21 |
| [#2447](https://github.com/developing-today-forks/openchamber/pull/2447) | feat(web): resolve newer install for self-handoff (upgrade takeover stage 1)                                        | dezren39              | 2026-11-21 |
| [#2456](https://github.com/developing-today-forks/openchamber/pull/2456) | fix(web): contain recursive filesystem deletes                                                                      | dezren39              | 2026-11-22 |
| [#2451](https://github.com/developing-today-forks/openchamber/pull/2451) | fix(web): honor explicit session create --dir                                                                       | dezren39              | 2026-11-22 |
| [#2390](https://github.com/developing-today-forks/openchamber/pull/2390) | fix(web): bound pending skills-catalog scans with a typed busy result (#2269)                                       | dezren39              | 2026-11-22 |
| [#2382](https://github.com/developing-today-forks/openchamber/pull/2382) | fix(web): keep the passkey store empty-file recheck bounded                                                         | dezren39              | 2026-11-22 |
| [#2422](https://github.com/developing-today-forks/openchamber/pull/2422) | fix(web): reject repeated skill-delete scope/source filters (#2419)                                                 | dezren39              | 2026-11-22 |
| [#2470](https://github.com/developing-today-forks/openchamber/pull/2470) | fix(web): persist the headerless sidebar setting                                                                    | dezren39              | 2026-11-22 |
| [#2517](https://github.com/developing-today-forks/openchamber/pull/2517) | fix(web): count zombies as dead when polling process trees (#2495)                                                  | dezren39              | 2026-11-23 |
| [#2519](https://github.com/developing-today-forks/openchamber/pull/2519) | fix(web): redact mid-line sensitive key=value values in Git output                                                  | dezren39              | 2026-11-23 |
| [#2523](https://github.com/developing-today-forks/openchamber/pull/2523) | fix(web): tunnel start-route catch stops the just-started process and preserves the prior tunnel                    | dezren39              | 2026-11-23 |
| [#2524](https://github.com/developing-today-forks/openchamber/pull/2524) | test(web): pin mkdir containment for missing descendants below a link                                               | dezren39              | 2026-11-23 |
| [#2525](https://github.com/developing-today-forks/openchamber/pull/2525) | fix(web): claim handoff request by rename before consuming it                                                       | dezren39              | 2026-11-23 |
| [#2526](https://github.com/developing-today-forks/openchamber/pull/2526) | fix(web): prevent concurrent screenshots from overwriting the same capture                                          | dezren39              | 2026-11-23 |
| [#2527](https://github.com/developing-today-forks/openchamber/pull/2527) | fix(web): stage project icon uploads until settings persist                                                         | dezren39              | 2026-11-23 |
| [#2529](https://github.com/developing-today-forks/openchamber/pull/2529) | fix(web): keep login-shell path variables out of isolated launches                                                  | dezren39              | 2026-11-23 |
| [#2530](https://github.com/developing-today-forks/openchamber/pull/2530) | fix(web): write project plan bodies through temp-plus-rename                                                        | dezren39              | 2026-11-23 |
| [#2531](https://github.com/developing-today-forks/openchamber/pull/2531) | fix(web): honor durable appendTrim/appendReset/appendHead failures in the global hub                                | dezren39              | 2026-11-23 |
| [#2532](https://github.com/developing-today-forks/openchamber/pull/2532) | fix(web): guard live-session status lookups against Object.prototype members                                        | dezren39              | 2026-11-23 |
| [#2533](https://github.com/developing-today-forks/openchamber/pull/2533) | fix(web): save workflow scripts atomically via temp file and rename                                                 | dezren39              | 2026-11-23 |
| [#2542](https://github.com/developing-today-forks/openchamber/pull/2542) | fix(web): repeated reportUsage can no longer turn an opt-out into an opt-in                                         | dezren39              | 2026-11-23 |
| [#2543](https://github.com/developing-today-forks/openchamber/pull/2543) | fix(web): reject repeated run-history taskId instead of dropping the filter                                         | dezren39              | 2026-11-23 |
| [#2546](https://github.com/developing-today-forks/openchamber/pull/2546) | fix(web): save prompt, skill, plugin and snippet files atomically                                                   | dezren39              | 2026-11-23 |
| [#2549](https://github.com/developing-today-forks/openchamber/pull/2549) | fix(web): remove the verified dictation archive when the final rename fails                                         | dezren39              | 2026-11-23 |
| [#2559](https://github.com/developing-today-forks/openchamber/pull/2559) | fix(web): retire the v1 managed OpenCode state fallback (#2423)                                                     | dezren39              | 2026-11-23 |
| [#2565](https://github.com/developing-today-forks/openchamber/pull/2565) | fix(web): reject a repeated message-search session parameter instead of widening the search                         | dezren39              | 2026-11-23 |
| [#2566](https://github.com/developing-today-forks/openchamber/pull/2566) | fix(web): wire cloudflared spawn errors into the tunnel startup promises                                            | dezren39              | 2026-11-23 |
| [#2568](https://github.com/developing-today-forks/openchamber/pull/2568) | fix(web): strip terminal escape sequences from worktree command failure messages                                    | dezren39              | 2026-11-23 |
| [#2569](https://github.com/developing-today-forks/openchamber/pull/2569) | fix(web): preserve file mode on /api/fs/write editor saves                                                          | dezren39              | 2026-11-23 |
| [#2571](https://github.com/developing-today-forks/openchamber/pull/2571) | fix(web): make /api/fs/rename destination replacement race-safe                                                     | dezren39              | 2026-11-23 |
| [#2576](https://github.com/developing-today-forks/openchamber/pull/2576) | fix(web): scope POST /api/fs/clone destinationPath to the active workspace                                          | dezren39              | 2026-11-23 |
| [#2579](https://github.com/developing-today-forks/openchamber/pull/2579) | fix(web): preview-grant serve route re-checks canonical path vs grant root (openchamber#2564)                       | dezren39              | 2026-11-23 |

##### developing-today-forks/openchamber — theme: ui (108)

| PR                                                                       | Title                                                                                                  | Author                | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------ | --------------------- | ---------- |
| [#47](https://github.com/developing-today-forks/openchamber/pull/47)     | fix(ui): follow subsessions of subsessions                                                             | dezren39              | 2026-10-18 |
| [#53](https://github.com/developing-today-forks/openchamber/pull/53)     | test(ui): spy on toast in useWebNotificationStream test instead of mock.module (#46)                   | dezren39              | 2026-10-18 |
| [#57](https://github.com/developing-today-forks/openchamber/pull/57)     | test(ui): stop useTerminalSessionKeepalive leaking into later test files (#46)                         | dezren39              | 2026-10-18 |
| [#63](https://github.com/developing-today-forks/openchamber/pull/63)     | test(ui): stop useQueuedMessageAutoSend mocking the session UI store (#46)                             | dezren39              | 2026-10-18 |
| [#69](https://github.com/developing-today-forks/openchamber/pull/69)     | fix(ui): bubble nested subagent asks up to the root session                                            | dezren39              | 2026-10-18 |
| [#88](https://github.com/developing-today-forks/openchamber/pull/88)     | fix(ui): make computeRollup cycle-safe for the cost split and subagent count                           | dezren39              | 2026-10-18 |
| [#92](https://github.com/developing-today-forks/openchamber/pull/92)     | fix(ui): show parent-cycle sessions in the mobile sessions sheet                                       | dezren39              | 2026-10-19 |
| [#94](https://github.com/developing-today-forks/openchamber/pull/94)     | fix(ui): drop the phantom expand chevron on mobile cycle sessions                                      | dezren39              | 2026-10-19 |
| [#101](https://github.com/developing-today-forks/openchamber/pull/101)   | fix(ui): make mobile run-lane tree cycle-safe                                                          | dezren39              | 2026-10-19 |
| [#103](https://github.com/developing-today-forks/openchamber/pull/103)   | fix(ui): Header tree moves use the cycle-safe descendant walk (#102)                                   | dezren39              | 2026-10-19 |
| [#105](https://github.com/developing-today-forks/openchamber/pull/105)   | fix(ui): make restored-session relocation cycle-safe (#104)                                            | dezren39              | 2026-10-19 |
| [#108](https://github.com/developing-today-forks/openchamber/pull/108)   | test(ui): wait for the sonner toast pin instead of a fixed settle                                      | dezren39              | 2026-10-19 |
| [#181](https://github.com/developing-today-forks/openchamber/pull/181)   | fix(ui): reject malformed version parts in mobileReleaseUpdate                                         | dezren39              | 2026-10-20 |
| [#276](https://github.com/developing-today-forks/openchamber/pull/276)   | fix(ui): reject out-of-range and truncated ports in terminal preview URLs                              | dezren39              | 2026-10-22 |
| [#284](https://github.com/developing-today-forks/openchamber/pull/284)   | fix(ui): reject port 0 in terminal preview URLs                                                        | dezren39              | 2026-10-22 |
| [#309](https://github.com/developing-today-forks/openchamber/pull/309)   | fix(ui): skip port-0 candidates before per-line selection so a valid sibling is announced              | dezren39              | 2026-10-22 |
| [#318](https://github.com/developing-today-forks/openchamber/pull/318)   | fix(ui): a closing device-settings report stays delivered when an older send lands after it (#281)     | dezren39              | 2026-10-22 |
| [#333](https://github.com/developing-today-forks/openchamber/pull/333)   | test(ui): extract applyGoalChoice and pin its call sites                                               | dezren39              | 2026-10-23 |
| [#385](https://github.com/developing-today-forks/openchamber/pull/385)   | feat(ui): add prompt retry recovery controls and active-run model change preference                    | dezren39              | 2026-10-25 |
| [#468](https://github.com/developing-today-forks/openchamber/pull/468)   | feat(ui): subsession models picker and policy passthrough                                              | dezren39              | 2026-10-26 |
| [#473](https://github.com/developing-today-forks/openchamber/pull/473)   | fix(ui): pools section shows unavailable instead of Loading forever without /api/model-routes          | dezren39              | 2026-10-26 |
| [#306](https://github.com/developing-today-forks/openchamber/pull/306)   | feat(ui): say which tool and MCP switches cost a running session its prompt cache (#149)               | dezren39              | 2026-10-26 |
| [#680](https://github.com/developing-today-forks/openchamber/pull/680)   | fix(ui): order disabled-provider writes against in-flight reads                                        | dezren39              | 2026-10-28 |
| [#711](https://github.com/developing-today-forks/openchamber/pull/711)   | refactor(ui): move sendMessage into session-actions, drop ChatInput wrapper (#300)                     | dezren39              | 2026-10-28 |
| [#724](https://github.com/developing-today-forks/openchamber/pull/724)   | test(ui): restore module mocks between sidebar test files                                              | dezren39              | 2026-10-28 |
| [#726](https://github.com/developing-today-forks/openchamber/pull/726)   | fix(ui): unwrap the session-activity v1 envelope in getWebServerSessionActivity (#709)                 | dezren39              | 2026-10-29 |
| [#856](https://github.com/developing-today-forks/openchamber/pull/856)   | fix(ui): add a deadline to the gitmoji catalog fetch                                                   | dezren39              | 2026-10-30 |
| [#857](https://github.com/developing-today-forks/openchamber/pull/857)   | fix(ui): bound the URL-auth token mint with a deadline                                                 | dezren39              | 2026-10-30 |
| [#859](https://github.com/developing-today-forks/openchamber/pull/859)   | fix(ui): add a deadline to the mobile release update check                                             | dezren39              | 2026-10-30 |
| [#986](https://github.com/developing-today-forks/openchamber/pull/986)   | fix(ui): deadline and stale-response guard for McpxDashboard fetches (#980)                            | dezren39              | 2026-10-31 |
| [#1011](https://github.com/developing-today-forks/openchamber/pull/1011) | fix(ui): deadline and byte cap for Mermaid remote-URL load (#931)                                      | dezren39              | 2026-10-31 |
| [#1014](https://github.com/developing-today-forks/openchamber/pull/1014) | fix(ui): cap MessageFreshnessDetector seen ids, remove messageCreationTimes, wire clearSession (#1013) | dezren39              | 2026-10-31 |
| [#1044](https://github.com/developing-today-forks/openchamber/pull/1044) | fix(ui): mobile release comparator orders prereleases below releases and ignores +build (#1043)        | dezren39              | 2026-10-31 |
| [#1087](https://github.com/developing-today-forks/openchamber/pull/1087) | fix(ui): keep the held field value out of the pageActions mismatch error                               | dezren39              | 2026-11-01 |
| [#1094](https://github.com/developing-today-forks/openchamber/pull/1094) | fix(ui): stop treating accounting updatedAt bumps as conflicting goal edits                            | dezren39              | 2026-11-01 |
| [#1098](https://github.com/developing-today-forks/openchamber/pull/1098) | fix(ui): show sidebar row settings on the Appearance page                                              | dezren39              | 2026-11-01 |
| [#1266](https://github.com/developing-today-forks/openchamber/pull/1266) | fix(ui): bound models.dev and gitmoji catalog fetches with byte caps (#1264)                           | dezren39              | 2026-11-01 |
| [#1276](https://github.com/developing-today-forks/openchamber/pull/1276) | fix(ui): judge the guest binary editor cap before buffering the file (#1275)                           | dezren39              | 2026-11-01 |
| [#1310](https://github.com/developing-today-forks/openchamber/pull/1310) | fix(ui): cap local Mermaid file loads                                                                  | dezren39              | 2026-11-02 |
| [#1331](https://github.com/developing-today-forks/openchamber/pull/1331) | fix(ui): cap the four UI cursor walks at triage-pinned page caps (#1320)                               | dezren39              | 2026-11-02 |
| [#1333](https://github.com/developing-today-forks/openchamber/pull/1333) | fix(ui): cap the session-title history walk at 20 pages (#1322)                                        | dezren39              | 2026-11-02 |
| [#1459](https://github.com/developing-today-forks/openchamber/pull/1459) | refactor(ui): anti-slop phase 2 batch A — owner types for UI store modules (#1438)                     | dezren39              | 2026-11-03 |
| [#1471](https://github.com/developing-today-forks/openchamber/pull/1471) | feat(ui): toggle pin, done-hint and directory row markers (#22)                                        | dezren39              | 2026-11-03 |
| [#1475](https://github.com/developing-today-forks/openchamber/pull/1475) | feat(ui): per-kind sidebar row lines — List fold, Chats and Tree project lines (#22)                   | developing-today[bot] | 2026-11-03 |
| [#1479](https://github.com/developing-today-forks/openchamber/pull/1479) | fix(ui): restore objective file when a session-goal save aborts as changed (#1372)                     | dezren39              | 2026-11-03 |
| [#1512](https://github.com/developing-today-forks/openchamber/pull/1512) | test(ui): de-flake useChatTurnNavigation session-restore test (#1509)                                  | dezren39              | 2026-11-03 |
| [#1487](https://github.com/developing-today-forks/openchamber/pull/1487) | feat(ui): show providers off by policy or enabled_providers as read-only-off (#218)                    | dezren39              | 2026-11-04 |
| [#1571](https://github.com/developing-today-forks/openchamber/pull/1571) | fix(ui): settle layout animations on the timer when rAF is unavailable                                 | dezren39              | 2026-11-04 |
| [#1600](https://github.com/developing-today-forks/openchamber/pull/1600) | feat(ui): Answer again on questions (split 216a of #216)                                               | dezren39              | 2026-11-04 |
| [#1579](https://github.com/developing-today-forks/openchamber/pull/1579) | feat(ui): project visibility flags — split 216c of PR #216 (#1574)                                     | dezren39              | 2026-11-05 |
| [#1494](https://github.com/developing-today-forks/openchamber/pull/1494) | feat(ui): tier dispatch + cost badge for project knowledge plans (#16 stage 2)                         | dezren39              | 2026-11-05 |
| [#1338](https://github.com/developing-today-forks/openchamber/pull/1338) | test(ui): pin #721 footer-deferral non-blanking regression (#872)                                      | dezren39              | 2026-11-05 |
| [#1661](https://github.com/developing-today-forks/openchamber/pull/1661) | feat(ui): routing page tabs and settings-search wiring (split 216e of #216)                            | dezren39              | 2026-11-05 |
| [#1672](https://github.com/developing-today-forks/openchamber/pull/1672) | fix(ui): map routing.pools-schedules search target to the Pools tab (#1665)                            | dezren39              | 2026-11-05 |
| [#1674](https://github.com/developing-today-forks/openchamber/pull/1674) | test(ui): make the #872 footer-deferral regression test discriminating (#1616)                         | dezren39              | 2026-11-05 |
| [#1756](https://github.com/developing-today-forks/openchamber/pull/1756) | fix(ui): declare teamsMore on LinearMappingResult (#1732)                                              | dezren39              | 2026-11-07 |
| [#1789](https://github.com/developing-today-forks/openchamber/pull/1789) | fix(ui): reveal managed tunnel sections from settings search                                           | dezren39              | 2026-11-07 |
| [#1805](https://github.com/developing-today-forks/openchamber/pull/1805) | fix(ui): register Mistral in the quota UI and render its console-only status                           | developing-today[bot] | 2026-11-08 |
| [#1804](https://github.com/developing-today-forks/openchamber/pull/1804) | fix(ui): translate the five untranslated project-knowledge plan strings in all fourteen locales        | developing-today[bot] | 2026-11-08 |
| [#1912](https://github.com/developing-today-forks/openchamber/pull/1912) | feat(ui): show estimated token count beside the goal spec length counter (#1831)                       | developing-today[bot] | 2026-11-08 |
| [#1913](https://github.com/developing-today-forks/openchamber/pull/1913) | feat(ui): show a diff of old vs rewritten spec after a goal writer rewrite (#1828)                     | dezren39              | 2026-11-08 |
| [#1914](https://github.com/developing-today-forks/openchamber/pull/1914) | fix(ui): carry annotationsTruncated to the UI (#1743)                                                  | developing-today[bot] | 2026-11-08 |
| [#1868](https://github.com/developing-today-forks/openchamber/pull/1868) | fix(ui): point Settings search routing.token at Providers > Classification                             | dezren39              | 2026-11-08 |
| [#1917](https://github.com/developing-today-forks/openchamber/pull/1917) | feat(ui): goal editor spec version history with view and restore                                       | dezren39              | 2026-11-08 |
| [#1918](https://github.com/developing-today-forks/openchamber/pull/1918) | fix(ui): Linear preview renders the commentsMore truncation note                                       | developing-today[bot] | 2026-11-08 |
| [#1926](https://github.com/developing-today-forks/openchamber/pull/1926) | fix(ui): stop the debounced settings flush from leaking across test files (#46)                        | dezren39              | 2026-11-08 |
| [#1923](https://github.com/developing-today-forks/openchamber/pull/1923) | fix(ui): phone sessions sheet honours project visibility flags (#217)                                  | developing-today[bot] | 2026-11-08 |
| [#1930](https://github.com/developing-today-forks/openchamber/pull/1930) | feat(ui): phone sheet Chats and Tree rows follow chatsLines/projectLines                               | developing-today[bot] | 2026-11-08 |
| [#1924](https://github.com/developing-today-forks/openchamber/pull/1924) | fix(ui): settings search providers.custom enters custom-provider create mode (#1765)                   | developing-today[bot] | 2026-11-08 |
| [#1936](https://github.com/developing-today-forks/openchamber/pull/1936) | fix(ui): GitLab MR preview commentTotal counts the merged thread                                       | developing-today[bot] | 2026-11-08 |
| [#1960](https://github.com/developing-today-forks/openchamber/pull/1960) | feat(ui): render the Linear labelsMore truncation note (#1782)                                         | developing-today[bot] | 2026-11-08 |
| [#1973](https://github.com/developing-today-forks/openchamber/pull/1973) | fix(ui): restore i18n placeholder parity across dictionaries                                           | dezren39              | 2026-11-08 |
| [#1982](https://github.com/developing-today-forks/openchamber/pull/1982) | fix(ui): repair the settings search anchor test breaking main CI                                       | developing-today[bot] | 2026-11-08 |
| [#1999](https://github.com/developing-today-forks/openchamber/pull/1999) | fix(ui): use a logical side for the labelsMore link margin (main lint)                                 | developing-today[bot] | 2026-11-08 |
| [#2008](https://github.com/developing-today-forks/openchamber/pull/2008) | fix(ui): do not claim no teams for a truncated empty Linear list                                       | dezren39              | 2026-11-08 |
| [#2038](https://github.com/developing-today-forks/openchamber/pull/2038) | fix(ui): make GitLab reference body clamp pair-safe                                                    | developing-today[bot] | 2026-11-09 |
| [#2053](https://github.com/developing-today-forks/openchamber/pull/2053) | fix(ui): useUIStore persisted clamps never split a surrogate pair                                      | developing-today[bot] | 2026-11-09 |
| [#2058](https://github.com/developing-today-forks/openchamber/pull/2058) | fix(ui): excerptMarkdown never splits a surrogate pair (#2043)                                         | dezren39              | 2026-11-09 |
| [#2061](https://github.com/developing-today-forks/openchamber/pull/2061) | fix(ui): objective fitting never splits a surrogate pair (#2044)                                       | dezren39              | 2026-11-09 |
| [#2062](https://github.com/developing-today-forks/openchamber/pull/2062) | fix(ui): multi-run title clamps never split a surrogate pair (#2049)                                   | dezren39              | 2026-11-11 |
| [#2094](https://github.com/developing-today-forks/openchamber/pull/2094) | fix(ui): normalizeContextTabLabel pair-safe clips labels at or under the 120 limit                     | dezren39              | 2026-11-11 |
| [#2099](https://github.com/developing-today-forks/openchamber/pull/2099) | fix(ui): pair-safe browser history title clamp (#2050)                                                 | dezren39              | 2026-11-11 |
| [#2086](https://github.com/developing-today-forks/openchamber/pull/2086) | fix(ui): keep the extracted-text truncation cut off a surrogate pair (#2060)                           | dezren39              | 2026-11-11 |
| [#2089](https://github.com/developing-today-forks/openchamber/pull/2089) | fix(ui): Tree view honours hiddenFromAllSessions, header stays (#2065)                                 | dezren39              | 2026-11-11 |
| [#2112](https://github.com/developing-today-forks/openchamber/pull/2112) | test(ui): pin PWA pair-safe clamp boundaries                                                           | dezren39              | 2026-11-11 |
| [#2125](https://github.com/developing-today-forks/openchamber/pull/2125) | fix(ui): defer session Markdown blob cleanup                                                           | dezren39              | 2026-11-12 |
| [#2137](https://github.com/developing-today-forks/openchamber/pull/2137) | fix(ui): surface custom theme load failures                                                            | dezren39              | 2026-11-12 |
| [#2149](https://github.com/developing-today-forks/openchamber/pull/2149) | fix(ui): scope coalesced reads by Authorization                                                        | dezren39              | 2026-11-13 |
| [#1870](https://github.com/developing-today-forks/openchamber/pull/1870) | fix(ui): surface statesMore truncation in the Source Board Linear state picker (#1768)                 | dezren39              | 2026-11-14 |
| [#2280](https://github.com/developing-today-forks/openchamber/pull/2280) | feat(ui): voice capture + transcript insertion against the opencode STT facade (#2271)                 | dezren39              | 2026-11-14 |
| [#2285](https://github.com/developing-today-forks/openchamber/pull/2285) | fix(ui): replace animate-pulse recording dot with shared-clock blink (#2284)                           | dezren39              | 2026-11-14 |
| [#2352](https://github.com/developing-today-forks/openchamber/pull/2352) | fix(ui): prevent stale small-model availability responses after invalidation                           | dezren39              | 2026-11-17 |
| [#2353](https://github.com/developing-today-forks/openchamber/pull/2353) | fix(ui): scope session knowledge summary cache to the active runtime (#2160)                           | dezren39              | 2026-11-17 |
| [#2386](https://github.com/developing-today-forks/openchamber/pull/2386) | fix(ui): invalidate pending agent-memory loads on runtime switch (#2145)                               | dezren39              | 2026-11-17 |
| [#2393](https://github.com/developing-today-forks/openchamber/pull/2393) | fix(ui): client deadline + abort for voice-capture submit                                              | dezren39              | 2026-11-17 |
| [#2403](https://github.com/developing-today-forks/openchamber/pull/2403) | fix(ui): restore main's type-check and lint                                                            | dezren39              | 2026-11-17 |
| [#2460](https://github.com/developing-today-forks/openchamber/pull/2460) | fix(ui): stop the message recovery render loop that crashes the chat                                   | dezren39              | 2026-11-22 |
| [#2461](https://github.com/developing-today-forks/openchamber/pull/2461) | feat(ui): give the subsession models picker the main picker's list                                     | dezren39              | 2026-11-22 |
| [#2462](https://github.com/developing-today-forks/openchamber/pull/2462) | fix(ui): give multi-model runs their own worktrees and turn Auto-create worktree on by default         | dezren39              | 2026-11-22 |
| [#2464](https://github.com/developing-today-forks/openchamber/pull/2464) | feat(ui): fresh worktree per new chat, question timeout and answer later                               | dezren39              | 2026-11-22 |
| [#2468](https://github.com/developing-today-forks/openchamber/pull/2468) | feat(ui): session times, sorting, and chat entry start/end times                                       | dezren39              | 2026-11-22 |
| [#2472](https://github.com/developing-today-forks/openchamber/pull/2472) | fix(ui): Work Status panel sizing and readability                                                      | dezren39              | 2026-11-22 |
| [#2487](https://github.com/developing-today-forks/openchamber/pull/2487) | test(ui): Ctrl+B for Move to background in the real composer and in Settings recording                 | dezren39              | 2026-11-22 |
| [#2547](https://github.com/developing-today-forks/openchamber/pull/2547) | fix(ui): fail closed when saving a dirty file whose canvas editor is gone                              | dezren39              | 2026-11-23 |
| [#2551](https://github.com/developing-today-forks/openchamber/pull/2551) | test(ui): pin DictationClient concurrent connection attempts (#2400)                                   | dezren39              | 2026-11-23 |
| [#2552](https://github.com/developing-today-forks/openchamber/pull/2552) | fix(ui): null JSON auth responses are misclassified as network failures                                | dezren39              | 2026-11-23 |
| [#2557](https://github.com/developing-today-forks/openchamber/pull/2557) | fix(ui): warn that an integer-like pool id always sorts first (#2498 item 3)                           | dezren39              | 2026-11-23 |
| [#2570](https://github.com/developing-today-forks/openchamber/pull/2570) | fix(ui): restore the marked sticky-rule pin under Streamdown                                           | dezren39              | 2026-11-23 |

##### developing-today-forks/openchamber — theme: vscode (89)

| PR                                                                       | Title                                                                                                 | Author                | Merged     |
| ------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------- | --------------------- | ---------- |
| [#98](https://github.com/developing-today-forks/openchamber/pull/98)     | fix(vscode): retry project-setup rename on transient Windows errors                                   | dezren39              | 2026-10-19 |
| [#183](https://github.com/developing-today-forks/openchamber/pull/183)   | fix(vscode): reject malformed versions in the upgrade check                                           | dezren39              | 2026-10-21 |
| [#642](https://github.com/developing-today-forks/openchamber/pull/642)   | fix(vscode): normalize contextLines in gitService diff functions                                      | dezren39              | 2026-10-28 |
| [#643](https://github.com/developing-today-forks/openchamber/pull/643)   | fix(vscode): normalize getGitLog maxCount in gitService                                               | dezren39              | 2026-10-28 |
| [#646](https://github.com/developing-today-forks/openchamber/pull/646)   | fix(vscode): range functions throw on bad refs in gitService (#644)                                   | dezren39              | 2026-10-28 |
| [#647](https://github.com/developing-today-forks/openchamber/pull/647)   | fix(vscode): throw on bad hash in getCommitFiles (#645)                                               | dezren39              | 2026-10-28 |
| [#652](https://github.com/developing-today-forks/openchamber/pull/652)   | fix(vscode): throw on git status failure in getGitStatusRaw (#650)                                    | dezren39              | 2026-10-28 |
| [#651](https://github.com/developing-today-forks/openchamber/pull/651)   | fix(vscode): report merge/rebase in progress in linked worktrees                                      | dezren39              | 2026-10-28 |
| [#637](https://github.com/developing-today-forks/openchamber/pull/637)   | feat(vscode): serve the session-activity v1 envelope from the bridge parity route (#621)              | dezren39              | 2026-10-28 |
| [#654](https://github.com/developing-today-forks/openchamber/pull/654)   | fix(vscode): guard gitService branch/ref positional args against option-like values (#653)            | dezren39              | 2026-10-28 |
| [#658](https://github.com/developing-today-forks/openchamber/pull/658)   | fix(vscode): keep the merge message in conflict-details for linked worktrees                          | dezren39              | 2026-10-28 |
| [#657](https://github.com/developing-today-forks/openchamber/pull/657)   | fix(vscode): report rebase/cherry-pick/revert attention in linked worktrees                           | dezren39              | 2026-10-28 |
| [#679](https://github.com/developing-today-forks/openchamber/pull/679)   | fix(vscode): surface git remote failures instead of reporting no remotes                              | dezren39              | 2026-10-28 |
| [#674](https://github.com/developing-today-forks/openchamber/pull/674)   | fix(vscode): abort selective commit when a staging step fails                                         | dezren39              | 2026-10-28 |
| [#681](https://github.com/developing-today-forks/openchamber/pull/681)   | fix(vscode): fail closed when git status fails in the hard-reset guard                                | dezren39              | 2026-10-28 |
| [#673](https://github.com/developing-today-forks/openchamber/pull/673)   | fix(vscode): fail setGitIdentity when a git config write fails                                        | dezren39              | 2026-10-28 |
| [#684](https://github.com/developing-today-forks/openchamber/pull/684)   | fix(vscode): surface git read failures in conflict-details                                            | dezren39              | 2026-10-28 |
| [#672](https://github.com/developing-today-forks/openchamber/pull/672)   | fix(vscode): throw on git branch failure in getGitBranchesRaw                                         | dezren39              | 2026-10-28 |
| [#690](https://github.com/developing-today-forks/openchamber/pull/690)   | fix(vscode): recognise a .git directory in findWorktreeRootForSkills (#687)                           | dezren39              | 2026-10-28 |
| [#714](https://github.com/developing-today-forks/openchamber/pull/714)   | fix(vscode): judge bridge fs TTL/timeout env values by their whole text                               | dezren39              | 2026-10-28 |
| [#730](https://github.com/developing-today-forks/openchamber/pull/730)   | fix(vscode): resolve the real git common dir in ensureOpenCodeProjectId (#729)                        | dezren39              | 2026-10-29 |
| [#733](https://github.com/developing-today-forks/openchamber/pull/733)   | fix(vscode): launch fs exec commands with argv spawn, not a double-shell exec string                  | dezren39              | 2026-10-29 |
| [#735](https://github.com/developing-today-forks/openchamber/pull/735)   | fix(vscode): validate repo and untracked status before revertGitFile deletes                          | dezren39              | 2026-10-29 |
| [#743](https://github.com/developing-today-forks/openchamber/pull/743)   | fix(vscode): cap git executor output and add a default deadline                                       | dezren39              | 2026-10-29 |
| [#747](https://github.com/developing-today-forks/openchamber/pull/747)   | fix(vscode): cap bridge fs exec output per stream at 4 MiB                                            | dezren39              | 2026-10-29 |
| [#751](https://github.com/developing-today-forks/openchamber/pull/751)   | fix(vscode): surface non-repository git failures from worktree listing                                | dezren39              | 2026-10-29 |
| [#790](https://github.com/developing-today-forks/openchamber/pull/790)   | fix(vscode): stage fallback commit files after '--' separator (#786)                                  | dezren39              | 2026-10-29 |
| [#793](https://github.com/developing-today-forks/openchamber/pull/793)   | fix(vscode): add deadline to bridge api:proxy forwards (#788)                                         | dezren39              | 2026-10-29 |
| [#795](https://github.com/developing-today-forks/openchamber/pull/795)   | fix(vscode): add deadlines to /api/info version probes (#789)                                         | dezren39              | 2026-10-29 |
| [#815](https://github.com/developing-today-forks/openchamber/pull/815)   | fix(vscode): add 15s deadlines to the 11 unbounded quota fetches                                      | dezren39              | 2026-10-29 |
| [#893](https://github.com/developing-today-forks/openchamber/pull/893)   | fix(vscode): atomic writes + corrupt-read preservation for the magic-prompts store                    | dezren39              | 2026-10-30 |
| [#922](https://github.com/developing-today-forks/openchamber/pull/922)   | fix(vscode): OpenCode Go quota result never carries planLabel                                         | dezren39              | 2026-10-30 |
| [#928](https://github.com/developing-today-forks/openchamber/pull/928)   | fix(vscode): PR #735's revert guard rejects tracked symlinks pointing outside the repo                | dezren39              | 2026-10-30 |
| [#921](https://github.com/developing-today-forks/openchamber/pull/921)   | fix(vscode): Antigravity quota reads the credential store, not only the legacy accounts file          | dezren39              | 2026-10-30 |
| [#1008](https://github.com/developing-today-forks/openchamber/pull/1008) | fix(vscode): sweep expired skills-catalog cache entries and cap the cache (#998)                      | dezren39              | 2026-10-31 |
| [#1010](https://github.com/developing-today-forks/openchamber/pull/1010) | fix(vscode): prune deleted sessions from the session-activity watcher (#999)                          | dezren39              | 2026-10-31 |
| [#1029](https://github.com/developing-today-forks/openchamber/pull/1029) | fix(vscode): cap bridge local-fs proxy reads — 16 MiB text / 100 MiB raw, 413 before readFile (#1027) | dezren39              | 2026-10-31 |
| [#1052](https://github.com/developing-today-forks/openchamber/pull/1052) | fix(vscode): cap upgrade-status registry body at 1 MiB                                                | dezren39              | 2026-10-31 |
| [#1060](https://github.com/developing-today-forks/openchamber/pull/1060) | fix(vscode): cap git file-diff working-tree reads at 20 MiB (#1057)                                   | dezren39              | 2026-10-31 |
| [#1067](https://github.com/developing-today-forks/openchamber/pull/1067) | fix(vscode): bound the models.dev catalog read at 16 MiB (#1061)                                      | dezren39              | 2026-10-31 |
| [#1070](https://github.com/developing-today-forks/openchamber/pull/1070) | fix(vscode): clamp the worktree start-command timeout reader at 2^31-1 (#1064)                        | dezren39              | 2026-10-31 |
| [#1071](https://github.com/developing-today-forks/openchamber/pull/1071) | fix(vscode): bound bridge update-check reads at 1 MiB (#1062)                                         | dezren39              | 2026-10-31 |
| [#1074](https://github.com/developing-today-forks/openchamber/pull/1074) | fix(vscode): cap the bridge api:fs:read handler at 16 MiB (#1068)                                     | dezren39              | 2026-10-31 |
| [#1105](https://github.com/developing-today-forks/openchamber/pull/1105) | fix(vscode): run the getGitFileDiff size gate before repo reads                                       | dezren39              | 2026-11-01 |
| [#1123](https://github.com/developing-today-forks/openchamber/pull/1123) | fix(vscode): fall back to credential store when Antigravity file token parses to nothing (#1113)      | dezren39              | 2026-11-01 |
| [#1125](https://github.com/developing-today-forks/openchamber/pull/1125) | fix(vscode): clamp getGitLog maxCount to a 10,000 ceiling (#1121)                                     | dezren39              | 2026-11-01 |
| [#1119](https://github.com/developing-today-forks/openchamber/pull/1119) | test(vscode): isolate both Antigravity accounts-file paths in the store-quota tests (#1114)           | dezren39              | 2026-11-01 |
| [#1126](https://github.com/developing-today-forks/openchamber/pull/1126) | fix(vscode): cap Shiki theme JSON reads (#1104)                                                       | dezren39              | 2026-11-01 |
| [#1248](https://github.com/developing-today-forks/openchamber/pull/1248) | fix(vscode): stat-gate OpenCode config file reads at 4 MiB (#1154)                                    | dezren39              | 2026-11-01 |
| [#1159](https://github.com/developing-today-forks/openchamber/pull/1159) | fix(vscode): stat-gate bridge AGENTS.md get at the existing 1 MiB cap (#1151)                         | dezren39              | 2026-11-01 |
| [#1212](https://github.com/developing-today-forks/openchamber/pull/1212) | fix(vscode): size-gate parseMdFile at 4 MiB (#1157)                                                   | dezren39              | 2026-11-01 |
| [#1251](https://github.com/developing-today-forks/openchamber/pull/1251) | fix(vscode): stat-gate plugin file reads at 1 MiB (#1156)                                             | dezren39              | 2026-11-01 |
| [#1259](https://github.com/developing-today-forks/openchamber/pull/1259) | fix(vscode): cap plugin npm packument reads                                                           | dezren39              | 2026-11-01 |
| [#1160](https://github.com/developing-today-forks/openchamber/pull/1160) | fix(vscode): stat-gate skill supporting-file reads at 1 MiB (#1158)                                   | dezren39              | 2026-11-01 |
| [#1281](https://github.com/developing-today-forks/openchamber/pull/1281) | fix(vscode): cap bridge proxy response bodies                                                         | dezren39              | 2026-11-01 |
| [#1288](https://github.com/developing-today-forks/openchamber/pull/1288) | fix(vscode): let config reads fall back to the read when the size-gate stat fails (#1280)             | dezren39              | 2026-11-02 |
| [#1299](https://github.com/developing-today-forks/openchamber/pull/1299) | fix(vscode): cap quota module response bodies at 1 MiB (#1298)                                        | dezren39              | 2026-11-02 |
| [#1300](https://github.com/developing-today-forks/openchamber/pull/1300) | fix(vscode): cap OpenCode-server response bodies in extension host (#1297)                            | dezren39              | 2026-11-02 |
| [#1330](https://github.com/developing-today-forks/openchamber/pull/1330) | test(vscode): exercise the shikiThemes byte-length gate at its exact cap (#1328)                      | dezren39              | 2026-11-02 |
| [#1327](https://github.com/developing-today-forks/openchamber/pull/1327) | fix(vscode): bound quota provider response bodies at 1 MiB (#1282)                                    | dezren39              | 2026-11-02 |
| [#1435](https://github.com/developing-today-forks/openchamber/pull/1435) | fix(vscode): serve stubbed config through the capped reader in OpenRouter quota tests (#813)          | dezren39              | 2026-11-03 |
| [#1468](https://github.com/developing-today-forks/openchamber/pull/1468) | test(vscode): pin the web fs-search ceilings on the traversal-budget patterns (#1393)                 | dezren39              | 2026-11-04 |
| [#1593](https://github.com/developing-today-forks/openchamber/pull/1593) | fix(vscode): decode fs-exec bridge output with a per-stream StringDecoder (#1590)                     | dezren39              | 2026-11-05 |
| [#1637](https://github.com/developing-today-forks/openchamber/pull/1637) | fix(vscode): decode git exec output incrementally                                                     | dezren39              | 2026-11-05 |
| [#1650](https://github.com/developing-today-forks/openchamber/pull/1650) | fix(vscode): bound readOpenChamberSettings behind a 4 MiB capped read (#1644)                         | dezren39              | 2026-11-05 |
| [#1760](https://github.com/developing-today-forks/openchamber/pull/1760) | fix(vscode): re-judge attach cap on bytes actually read                                               | dezren39              | 2026-11-07 |
| [#1786](https://github.com/developing-today-forks/openchamber/pull/1786) | fix(vscode): route ten quota providers through shared credential selectors (#1759)                    | dezren39              | 2026-11-07 |
| [#1799](https://github.com/developing-today-forks/openchamber/pull/1799) | fix(vscode): bound session-state file reads                                                           | dezren39              | 2026-11-08 |
| [#1919](https://github.com/developing-today-forks/openchamber/pull/1919) | fix(vscode): declare bun:test types so vscode type-check passes                                       | developing-today[bot] | 2026-11-08 |
| [#1929](https://github.com/developing-today-forks/openchamber/pull/1929) | test(vscode): pin quota second-field fallthrough for all 11 providers                                 | developing-today[bot] | 2026-11-08 |
| [#1935](https://github.com/developing-today-forks/openchamber/pull/1935) | fix(vscode): port the Mistral quota fetcher to the VS Code backend                                    | dezren39              | 2026-11-08 |
| [#1968](https://github.com/developing-today-forks/openchamber/pull/1968) | test(vscode): keep the entity-v2 scratch tree out from under a config-carrying ancestor (#1814)       | developing-today[bot] | 2026-11-08 |
| [#2131](https://github.com/developing-today-forks/openchamber/pull/2131) | fix(vscode): bound selected text attachments                                                          | dezren39              | 2026-11-12 |
| [#2279](https://github.com/developing-today-forks/openchamber/pull/2279) | fix(vscode): add command-code and opendesign quota fetchers (#1744)                                   | dezren39              | 2026-11-14 |
| [#2283](https://github.com/developing-today-forks/openchamber/pull/2283) | fix(vscode): wire Open Design credential management into the VS Code surface (#2281)                  | dezren39              | 2026-11-14 |
| [#2287](https://github.com/developing-today-forks/openchamber/pull/2287) | fix(vscode): add MANAGED_PROVIDERS to quotaCredentials mock in bridge-system-runtime test             | dezren39              | 2026-11-15 |
| [#2302](https://github.com/developing-today-forks/openchamber/pull/2302) | test(vscode): pin skillsCatalog install-copy caps from PR #2297 (#2301)                               | dezren39              | 2026-11-16 |
| [#2332](https://github.com/developing-today-forks/openchamber/pull/2332) | fix(vscode): match shared settings.json cap at 8 MiB (#1746)                                          | dezren39              | 2026-11-17 |
| [#2362](https://github.com/developing-today-forks/openchamber/pull/2362) | fix(vscode): contain skill supporting-file paths within the skill dir (#2304)                         | dezren39              | 2026-11-17 |
| [#2359](https://github.com/developing-today-forks/openchamber/pull/2359) | test(vscode): pin npm userconfig for every opencode-upgrade-runtime test                              | dezren39              | 2026-11-17 |
| [#2355](https://github.com/developing-today-forks/openchamber/pull/2355) | fix(vscode): bound bridge settings runtime reads behind per-file caps (#1641)                         | dezren39              | 2026-11-17 |
| [#2413](https://github.com/developing-today-forks/openchamber/pull/2413) | fix(vscode): bound the quota local-file readers (#1643)                                               | dezren39              | 2026-11-20 |
| [#2518](https://github.com/developing-today-forks/openchamber/pull/2518) | fix(vscode): contain api:fs:list to the active workspace                                              | dezren39              | 2026-11-23 |
| [#2522](https://github.com/developing-today-forks/openchamber/pull/2522) | fix(vscode): contain api:fs:search to the active workspace                                            | dezren39              | 2026-11-23 |
| [#2528](https://github.com/developing-today-forks/openchamber/pull/2528) | fix(vscode): contain api:fs:mkdir targets to sanctioned roots                                         | dezren39              | 2026-11-23 |
| [#2538](https://github.com/developing-today-forks/openchamber/pull/2538) | fix(vscode): refuse allowOutsideWorkspace in the api:fs:mkdir bridge handler (#2521 remainder)        | dezren39              | 2026-11-23 |
| [#2555](https://github.com/developing-today-forks/openchamber/pull/2555) | fix(vscode): save AGENTS.md, prompt, plugin and skill files atomically                                | dezren39              | 2026-11-23 |
| [#2563](https://github.com/developing-today-forks/openchamber/pull/2563) | fix(vscode): bound save-image and save-markdown writes to the attachment cap                          | dezren39              | 2026-11-23 |
| [#2575](https://github.com/developing-today-forks/openchamber/pull/2575) | fix(vscode): reject fs read and stat paths escaping through symlinks                                  | dezren39              | 2026-11-23 |

##### developing-today-forks/openchamber — theme: ci (89)

| PR                                                                       | Title                                                                                                        | Author                | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------ | --------------------- | ---------- |
| [#4](https://github.com/developing-today-forks/openchamber/pull/4)       | ci: use GitHub-hosted runners instead of Blacksmith                                                          | dezren39              | 2026-10-12 |
| [#5](https://github.com/developing-today-forks/openchamber/pull/5)       | ci: make the iOS/macOS mobile builds opt-in                                                                  | dezren39              | 2026-10-12 |
| [#49](https://github.com/developing-today-forks/openchamber/pull/49)     | fix(ci): pin jules-action to v1.0.0 — @v1 does not exist (#48)                                               | dezren39              | 2026-10-18 |
| [#58](https://github.com/developing-today-forks/openchamber/pull/58)     | ci: add concurrency groups to cancel superseded runs                                                         | dezren39              | 2026-10-18 |
| [#62](https://github.com/developing-today-forks/openchamber/pull/62)     | fix(ci): skip pr-review automation when the review app is not configured                                     | dezren39              | 2026-10-18 |
| [#66](https://github.com/developing-today-forks/openchamber/pull/66)     | fix(ci): skip issue-intake when the bot app is not configured                                                | dezren39              | 2026-10-18 |
| [#74](https://github.com/developing-today-forks/openchamber/pull/74)     | fix(ci): skip bot-help, bot-summarize, oc-integration and opencode when the bot app is not configured (#73)  | dezren39              | 2026-10-18 |
| [#76](https://github.com/developing-today-forks/openchamber/pull/76)     | ci: key concurrency groups per PR/issue/input (fixes #72)                                                    | dezren39              | 2026-10-18 |
| [#78](https://github.com/developing-today-forks/openchamber/pull/78)     | ci: queue bot-summarize, issue-intake and pr-review comment runs (queue: max)                                | dezren39              | 2026-10-18 |
| [#85](https://github.com/developing-today-forks/openchamber/pull/85)     | fix(ci): issue-intake and pr-review fail at startup — split supersede events into per-run concurrency groups | dezren39              | 2026-10-18 |
| [#96](https://github.com/developing-today-forks/openchamber/pull/96)     | fix(ci): run pr checks on pushes to main                                                                     | dezren39              | 2026-10-19 |
| [#97](https://github.com/developing-today-forks/openchamber/pull/97)     | fix(ci): key sdk-preview and mobile-release concurrency by dispatch inputs                                   | dezren39              | 2026-10-19 |
| [#113](https://github.com/developing-today-forks/openchamber/pull/113)   | ci: declare least-privilege permissions in dmg, mobile-ci and oc-review workflows                            | dezren39              | 2026-10-19 |
| [#116](https://github.com/developing-today-forks/openchamber/pull/116)   | ci: add timeout-minutes caps to every workflow job                                                           | dezren39              | 2026-10-19 |
| [#119](https://github.com/developing-today-forks/openchamber/pull/119)   | fix(ci): remove timeout-minutes from the mobile-release caller job                                           | dezren39              | 2026-10-19 |
| [#124](https://github.com/developing-today-forks/openchamber/pull/124)   | fix(ci): queue Jules upstream-sync dispatches with queue: max (code#359 set)                                 | dezren39              | 2026-10-19 |
| [#128](https://github.com/developing-today-forks/openchamber/pull/128)   | ci: SHA-pin the six remaining floating @v4 action references                                                 | dezren39              | 2026-10-19 |
| [#144](https://github.com/developing-today-forks/openchamber/pull/144)   | ci: align checkout/upload/download actions to the repo's v7 pins                                             | dezren39              | 2026-10-20 |
| [#156](https://github.com/developing-today-forks/openchamber/pull/156)   | ci: bump actions/cache to v6.1.0 in the desktop release pipeline                                             | dezren39              | 2026-10-20 |
| [#158](https://github.com/developing-today-forks/openchamber/pull/158)   | ci: bump actions/github-script to v8.0.0 in pr-intake (#157)                                                 | dezren39              | 2026-10-20 |
| [#162](https://github.com/developing-today-forks/openchamber/pull/162)   | ci: pin jules-action to commit SHA (v1.0.0)                                                                  | dezren39              | 2026-10-20 |
| [#391](https://github.com/developing-today-forks/openchamber/pull/391)   | chore(ci): Jules sync prompt — merge only when reasonably certain                                            | dezren39              | 2026-10-24 |
| [#448](https://github.com/developing-today-forks/openchamber/pull/448)   | ci: scheduled opencode-update workflow (daily bump, build, test, auto-merge)                                 | dezren39              | 2026-10-25 |
| [#451](https://github.com/developing-today-forks/openchamber/pull/451)   | fix(ci): keep the end of the log tail in opencode-update failure excerpts                                    | dezren39              | 2026-10-25 |
| [#452](https://github.com/developing-today-forks/openchamber/pull/452)   | ci: opencode-update — merge handoff for token refusals, cleaner failure excerpts                             | dezren39              | 2026-10-25 |
| [#460](https://github.com/developing-today-forks/openchamber/pull/460)   | fix(ci): record opencode-update install failures on the tracking issue                                       | dezren39              | 2026-10-26 |
| [#462](https://github.com/developing-today-forks/openchamber/pull/462)   | fix(ci): run pr-intake on this fork as well as upstream                                                      | dezren39              | 2026-10-26 |
| [#466](https://github.com/developing-today-forks/openchamber/pull/466)   | fix(ci): let pr-intake label with the workflow token when the bot app is unconfigured                        | dezren39              | 2026-10-26 |
| [#479](https://github.com/developing-today-forks/openchamber/pull/479)   | fix(ci): keep a truncated single line instead of an empty opencode-update failure excerpt                    | dezren39              | 2026-10-26 |
| [#581](https://github.com/developing-today-forks/openchamber/pull/581)   | ci: add stale-branch cleanup workflow (dry-run by default)                                                   | dezren39              | 2026-10-27 |
| [#590](https://github.com/developing-today-forks/openchamber/pull/590)   | ci: run branch cleanup daily                                                                                 | dezren39              | 2026-10-27 |
| [#593](https://github.com/developing-today-forks/openchamber/pull/593)   | fix(ci): run label-merge-conflict on this fork with a workflow-token fallback (#87)                          | dezren39              | 2026-10-27 |
| [#600](https://github.com/developing-today-forks/openchamber/pull/600)   | fix(ci): run stale on this fork as label-only (never closes) (#87)                                           | dezren39              | 2026-10-27 |
| [#636](https://github.com/developing-today-forks/openchamber/pull/636)   | fix(ci): protect open-PR bases in branch-cleanup (#629)                                                      | dezren39              | 2026-10-28 |
| [#691](https://github.com/developing-today-forks/openchamber/pull/691)   | fix(ci): opencode-update tests on Node 24; failures escalate to Jules, then Muse tasks                       | dezren39              | 2026-10-28 |
| [#692](https://github.com/developing-today-forks/openchamber/pull/692)   | feat(ci): record upstream-sync failures on a tracking issue for Muse tasks                                   | dezren39              | 2026-10-28 |
| [#716](https://github.com/developing-today-forks/openchamber/pull/716)   | fix(ci): close upstream-sync tracking issue on success                                                       | dezren39              | 2026-10-28 |
| [#752](https://github.com/developing-today-forks/openchamber/pull/752)   | ci: cancel-in-progress gaps + PR-tier slimming (fleet Phases 1+2)                                            | dezren39              | 2026-10-29 |
| [#753](https://github.com/developing-today-forks/openchamber/pull/753)   | ci: main-broken watcher — Phase 3 remediation chain (Jules leg gated off: no JULES_API_KEY)                  | dezren39              | 2026-10-29 |
| [#754](https://github.com/developing-today-forks/openchamber/pull/754)   | ci: Drew expanded directives — basic-tier naming + repo-sweeper consolidation (follow-up to #752)            | dezren39              | 2026-10-29 |
| [#755](https://github.com/developing-today-forks/openchamber/pull/755)   | fix(ci): repo-sweeper duplicate needs/if keys (workflow startup failure)                                     | dezren39              | 2026-10-29 |
| [#760](https://github.com/developing-today-forks/openchamber/pull/760)   | fix(ci): repo-sweeper ticks can no longer cancel a running issue intake                                      | dezren39              | 2026-10-29 |
| [#895](https://github.com/developing-today-forks/openchamber/pull/895)   | ci: main-only workflow triggers, cancel-in-progress everywhere                                               | dezren39              | 2026-10-30 |
| [#904](https://github.com/developing-today-forks/openchamber/pull/904)   | ci: migrate workflow dispatches from Jules to pm-dispatch                                                    | dezren39              | 2026-10-30 |
| [#920](https://github.com/developing-today-forks/openchamber/pull/920)   | fix(ci): desktop smoke concurrency group ignores platform inputs                                             | dezren39              | 2026-10-30 |
| [#927](https://github.com/developing-today-forks/openchamber/pull/927)   | fix(ci): pm-dispatch smoke workflow has no concurrency group                                                 | dezren39              | 2026-10-30 |
| [#926](https://github.com/developing-today-forks/openchamber/pull/926)   | fix(ci): pr-review verdict verification can consume another concurrent run's verdict                         | dezren39              | 2026-10-30 |
| [#929](https://github.com/developing-today-forks/openchamber/pull/929)   | fix(ci): main-broken watcher lookup-and-create is not serialized                                             | dezren39              | 2026-10-30 |
| [#909](https://github.com/developing-today-forks/openchamber/pull/909)   | ci: add scheduled pm-dispatch agents (agents.yml) and ci-fix hook                                            | dezren39              | 2026-10-30 |
| [#924](https://github.com/developing-today-forks/openchamber/pull/924)   | fix(ci): main-broken watcher fires for failed fork PR runs                                                   | dezren39              | 2026-10-30 |
| [#953](https://github.com/developing-today-forks/openchamber/pull/953)   | fix(ci): sweeper intake re-check aborts on comment fetch failure                                             | dezren39              | 2026-10-30 |
| [#973](https://github.com/developing-today-forks/openchamber/pull/973)   | fix(ci): reconcile ci-policy and workflow headers with main-only triggers (#971)                             | dezren39              | 2026-10-31 |
| [#1092](https://github.com/developing-today-forks/openchamber/pull/1092) | fix(ci): branch-cleanup — one failed delete must not abort the run (code#1615 set)                           | dezren39              | 2026-11-01 |
| [#1100](https://github.com/developing-today-forks/openchamber/pull/1100) | fix(ci): persist the first-failure record on the pm-dispatch-created opencode-update tracker                 | dezren39              | 2026-11-01 |
| [#935](https://github.com/developing-today-forks/openchamber/pull/935)   | fix(ci): add concurrency group to agents.yml (#933)                                                          | dezren39              | 2026-11-02 |
| [#1335](https://github.com/developing-today-forks/openchamber/pull/1335) | ci: add workflow_dispatch where missing (no-PR-trigger policy)                                               | dezren39              | 2026-11-02 |
| [#1353](https://github.com/developing-today-forks/openchamber/pull/1353) | fix(ci): restore cancel-in-progress: false in opencode.yml (#1337)                                           | dezren39              | 2026-11-02 |
| [#1360](https://github.com/developing-today-forks/openchamber/pull/1360) | fix(ci): restore cancel-in-progress: false in mobile-release.yml                                             | dezren39              | 2026-11-02 |
| [#1350](https://github.com/developing-today-forks/openchamber/pull/1350) | fix(ci): restore valid bot-summarize concurrency                                                             | dezren39              | 2026-11-02 |
| [#1351](https://github.com/developing-today-forks/openchamber/pull/1351) | fix(ci): oc-integration allows queued bot runs again                                                         | dezren39              | 2026-11-02 |
| [#1361](https://github.com/developing-today-forks/openchamber/pull/1361) | fix(ci): restore cancel-in-progress: false in jules-daily-upstream-sync concurrency (#1346)                  | dezren39              | 2026-11-02 |
| [#1367](https://github.com/developing-today-forks/openchamber/pull/1367) | fix(ci): restore cancel-in-progress: false in stale.yml (#1342)                                              | dezren39              | 2026-11-02 |
| [#1364](https://github.com/developing-today-forks/openchamber/pull/1364) | fix(ci): restore cancel-in-progress: false in sdk-preview.yml                                                | dezren39              | 2026-11-02 |
| [#1355](https://github.com/developing-today-forks/openchamber/pull/1355) | fix(ci): restore cancel-in-progress: false in opencode-smoke.yml                                             | dezren39              | 2026-11-02 |
| [#1354](https://github.com/developing-today-forks/openchamber/pull/1354) | fix(ci): bot-help allows queued runs again                                                                   | dezren39              | 2026-11-02 |
| [#1356](https://github.com/developing-today-forks/openchamber/pull/1356) | fix(ci): restore valid build-macos-arm64-dmg concurrency (#1348)                                             | dezren39              | 2026-11-02 |
| [#1382](https://github.com/developing-today-forks/openchamber/pull/1382) | ci: gate releases on green pr-checks main-gate SHA                                                           | dezren39              | 2026-11-03 |
| [#1396](https://github.com/developing-today-forks/openchamber/pull/1396) | ci: add timeout-minutes to jobs that lacked one                                                              | dezren39              | 2026-11-03 |
| [#1400](https://github.com/developing-today-forks/openchamber/pull/1400) | fix(ci): remove timeout-minutes from reusable-workflow job in mobile-release.yml (#1398)                     | dezren39              | 2026-11-03 |
| [#1402](https://github.com/developing-today-forks/openchamber/pull/1402) | fix(ci): remove timeout-minutes from reusable-workflow caller jobs in agents.yml and release.yml (#1399)     | dezren39              | 2026-11-03 |
| [#1413](https://github.com/developing-today-forks/openchamber/pull/1413) | fix(ci): remove dead pull_request_target branches from pr-review.yml (#1412)                                 | dezren39              | 2026-11-03 |
| [#1417](https://github.com/developing-today-forks/openchamber/pull/1417) | ci: trim 180-minute job timeouts to 90                                                                       | dezren39              | 2026-11-03 |
| [#1425](https://github.com/developing-today-forks/openchamber/pull/1425) | ci: add caller-job audit as the #1399 recurrence guard                                                       | dezren39              | 2026-11-03 |
| [#1447](https://github.com/developing-today-forks/openchamber/pull/1447) | fix(ci): vendor pm-dispatch actions into this repo (#805)                                                    | dezren39              | 2026-11-03 |
| [#1455](https://github.com/developing-today-forks/openchamber/pull/1455) | ci: one-off arm workflow to measure the aarch64-linux node_modules hash (#275)                               | dezren39              | 2026-11-03 |
| [#1532](https://github.com/developing-today-forks/openchamber/pull/1532) | fix(ci): use vendored pm-dispatch actions in jules sync and oc-review ci-fix                                 | dezren39              | 2026-11-04 |
| [#1584](https://github.com/developing-today-forks/openchamber/pull/1584) | ci: remove push/pull_request workflow triggers                                                               | developing-today[bot] | 2026-11-04 |
| [#1588](https://github.com/developing-today-forks/openchamber/pull/1588) | docs(ci): reconcile ci-policy and oc-review comments to nightly+dispatch main-gate (#1585)                   | dezren39              | 2026-11-04 |
| [#1594](https://github.com/developing-today-forks/openchamber/pull/1594) | fix(ci): remove repo-sweeper workflow-level concurrency block (#1592)                                        | dezren39              | 2026-11-04 |
| [#1611](https://github.com/developing-today-forks/openchamber/pull/1611) | ci: run workflow audit nightly                                                                               | dezren39              | 2026-11-05 |
| [#1612](https://github.com/developing-today-forks/openchamber/pull/1612) | ci: fix workflow guards after repository move                                                                | dezren39              | 2026-11-05 |
| [#1603](https://github.com/developing-today-forks/openchamber/pull/1603) | fix(ci): reconcile release workflows to the dispatch-only reality (#1601)                                    | dezren39              | 2026-11-05 |
| [#1634](https://github.com/developing-today-forks/openchamber/pull/1634) | ci: wire the release-gate test into CI (#1565)                                                               | dezren39              | 2026-11-05 |
| [#1933](https://github.com/developing-today-forks/openchamber/pull/1933) | ci: evaluate this repo's own flake in the pr checks main tier                                                | developing-today[bot] | 2026-11-08 |
| [#2397](https://github.com/developing-today-forks/openchamber/pull/2397) | feat(ci): enable Knip classMembers in dead-code check; disposition 55 findings                               | dezren39              | 2026-11-17 |
| [#2503](https://github.com/developing-today-forks/openchamber/pull/2503) | test: pin git's big-file threshold in diff-cap tests; stop the table browser test racing Vite's reload       | dezren39              | 2026-11-23 |
| [#2537](https://github.com/developing-today-forks/openchamber/pull/2537) | fix(ci): fall back to github.token in stale workflow label step                                              | dezren39              | 2026-11-23 |
| [#2548](https://github.com/developing-today-forks/openchamber/pull/2548) | fix(ci): fail release entitlement checks when codesign fails                                                 | dezren39              | 2026-11-23 |
| [#2558](https://github.com/developing-today-forks/openchamber/pull/2558) | fix(ci): grant actions: read to canary.yml mobile-release caller jobs (#2554)                                | dezren39              | 2026-11-23 |

##### developing-today-forks/openchamber — theme: quota (48)

| PR                                                                       | Title                                                                                                   | Author                | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------- | --------------------- | ---------- |
| [#486](https://github.com/developing-today-forks/openchamber/pull/486)   | test(quota): pin minimax-shared and wafer provider behavior                                             | dezren39              | 2026-10-26 |
| [#496](https://github.com/developing-today-forks/openchamber/pull/496)   | fix(quota): VS Code quota port NaN resetAt in google/antigravity paths (#492)                           | dezren39              | 2026-10-26 |
| [#501](https://github.com/developing-today-forks/openchamber/pull/501)   | fix(quota): shared toTimestamp/normalizeTimestamp leak Infinity and NaN instead of null                 | dezren39              | 2026-10-26 |
| [#520](https://github.com/developing-today-forks/openchamber/pull/520)   | feat(quota): wire per-credential results through the web Usage page                                     | dezren39              | 2026-10-27 |
| [#528](https://github.com/developing-today-forks/openchamber/pull/528)   | feat(quota): restore the Command Code usage provider                                                    | dezren39              | 2026-10-27 |
| [#529](https://github.com/developing-today-forks/openchamber/pull/529)   | fix(quota): fall back to the local credential store and bound provider fetches                          | dezren39              | 2026-10-27 |
| [#537](https://github.com/developing-today-forks/openchamber/pull/537)   | fix(quota): Cursor windowSeconds is the billing-cycle duration, not time until reset (#495)             | dezren39              | 2026-10-27 |
| [#538](https://github.com/developing-today-forks/openchamber/pull/538)   | fix(quota): VS Code Cursor enterprise windowSeconds is the cycle duration, not time until reset (#506)  | dezren39              | 2026-10-27 |
| [#541](https://github.com/developing-today-forks/openchamber/pull/541)   | fix(quota): clamp usedPercent to [0,100] in google and antigravity transforms (#507)                    | dezren39              | 2026-10-27 |
| [#548](https://github.com/developing-today-forks/openchamber/pull/548)   | fix(quota): clamp usedPercent to [0,100] in VS Code port zai and zhipu paths (#510)                     | dezren39              | 2026-10-27 |
| [#550](https://github.com/developing-today-forks/openchamber/pull/550)   | fix(quota): clamp usedPercent to [0,100] in ClinePass provider (#512)                                   | dezren39              | 2026-10-27 |
| [#552](https://github.com/developing-today-forks/openchamber/pull/552)   | fix(quota): clamp usedPercent to [0,100] in Claude transforms (#515)                                    | dezren39              | 2026-10-27 |
| [#555](https://github.com/developing-today-forks/openchamber/pull/555)   | fix(quota): clamp Cursor usedPercent to [0,100] on the explicit-percent paths                           | dezren39              | 2026-10-27 |
| [#547](https://github.com/developing-today-forks/openchamber/pull/547)   | fix(quota): clamp usedPercent to [0,100] in zai and zhipu providers (#509)                              | dezren39              | 2026-10-27 |
| [#551](https://github.com/developing-today-forks/openchamber/pull/551)   | fix(quota): clamp Codex usage percentages                                                               | dezren39              | 2026-10-27 |
| [#556](https://github.com/developing-today-forks/openchamber/pull/556)   | fix(quota): clamp Claude usage in VS Code                                                               | dezren39              | 2026-10-27 |
| [#557](https://github.com/developing-today-forks/openchamber/pull/557)   | fix(quota): clamp Codex usedPercent to [0,100] in the VS Code provider                                  | dezren39              | 2026-10-27 |
| [#565](https://github.com/developing-today-forks/openchamber/pull/565)   | fix(quota): clamp Cursor plan usage in VS Code                                                          | dezren39              | 2026-10-27 |
| [#563](https://github.com/developing-today-forks/openchamber/pull/563)   | fix(quota): clamp VS Code google/antigravity usedPercent to [0,100] (#545)                              | dezren39              | 2026-10-27 |
| [#564](https://github.com/developing-today-forks/openchamber/pull/564)   | fix(quota): guard non-finite VS Code timestamps                                                         | dezren39              | 2026-10-27 |
| [#539](https://github.com/developing-today-forks/openchamber/pull/539)   | fix(quota): reuse Codex fetcher for OpenAI export                                                       | dezren39              | 2026-10-27 |
| [#540](https://github.com/developing-today-forks/openchamber/pull/540)   | feat(quota): Meta (Muse Code High Usage) provider for /api/quota                                        | dezren39              | 2026-10-27 |
| [#577](https://github.com/developing-today-forks/openchamber/pull/577)   | fix(quota): report non-finite VS Code z.ai/zhipu percentages as unknown                                 | dezren39              | 2026-10-27 |
| [#576](https://github.com/developing-today-forks/openchamber/pull/576)   | test(quota): pin OpenAI usage percent clamping                                                          | dezren39              | 2026-10-27 |
| [#575](https://github.com/developing-today-forks/openchamber/pull/575)   | fix(quota): google transformModelData uses toTimestamp for resetTime (#489)                             | dezren39              | 2026-10-27 |
| [#580](https://github.com/developing-today-forks/openchamber/pull/580)   | fix(quota): clamp Meta used_percent to [0,100] in web and VS Code providers                             | dezren39              | 2026-10-27 |
| [#583](https://github.com/developing-today-forks/openchamber/pull/583)   | fix(quota): clamp ClinePass usedPercent to [0,100] in VS Code port (#546)                               | dezren39              | 2026-10-27 |
| [#582](https://github.com/developing-today-forks/openchamber/pull/582)   | fix(quota): read Cursor enterprise cycle start from planInfo first (#574)                               | dezren39              | 2026-10-27 |
| [#597](https://github.com/developing-today-forks/openchamber/pull/597)   | fix(quota): align VS Code Meta exhausted message with the web provider (#585)                           | dezren39              | 2026-10-27 |
| [#609](https://github.com/developing-today-forks/openchamber/pull/609)   | fix(quota): stop reporting MiniMax remains_time as windowSeconds (#606)                                 | dezren39              | 2026-10-28 |
| [#611](https://github.com/developing-today-forks/openchamber/pull/611)   | fix(quota): stop Wafer provider fabricating usedPercent 0 when percent is absent (#608)                 | dezren39              | 2026-10-28 |
| [#634](https://github.com/developing-today-forks/openchamber/pull/634)   | fix(quota): stop Cursor on-demand fabricating exhaustion when remaining is absent (#622)                | dezren39              | 2026-10-28 |
| [#618](https://github.com/developing-today-forks/openchamber/pull/618)   | fix(quota): stop the VS Code Wafer provider fabricating usedPercent 0 (#616)                            | dezren39              | 2026-10-28 |
| [#675](https://github.com/developing-today-forks/openchamber/pull/675)   | fix(quota): report post-roll Meta empty window as an empty reading                                      | dezren39              | 2026-10-28 |
| [#764](https://github.com/developing-today-forks/openchamber/pull/764)   | fix(quota): resolve Antigravity auth from the OpenCode credential store                                 | dezren39              | 2026-10-29 |
| [#860](https://github.com/developing-today-forks/openchamber/pull/860)   | fix(quota): surface antigravity credential-store read failures as their own error state                 | dezren39              | 2026-10-30 |
| [#952](https://github.com/developing-today-forks/openchamber/pull/952)   | test(quota): pin pendingFetches release after a timed-out provider settles                              | dezren39              | 2026-10-30 |
| [#956](https://github.com/developing-today-forks/openchamber/pull/956)   | fix(quota): a store login confirmed wrong-client reads as Antigravity not configured                    | dezren39              | 2026-10-30 |
| [#959](https://github.com/developing-today-forks/openchamber/pull/959)   | fix(quota): Console rows precede the API-key row for OpenCode Go                                        | dezren39              | 2026-10-30 |
| [#1142](https://github.com/developing-today-forks/openchamber/pull/1142) | fix(quota): forward usesLocalStore through the CLI combined credential source (#1141)                   | dezren39              | 2026-11-02 |
| [#1404](https://github.com/developing-today-forks/openchamber/pull/1404) | fix(quota): stop the CLI combined source passing its own env as the server's launch environment (#1403) | dezren39              | 2026-11-03 |
| [#1461](https://github.com/developing-today-forks/openchamber/pull/1461) | fix(quota): classify instance credential read failures in listing as shared upstream-auth (#1130)       | dezren39              | 2026-11-03 |
| [#1556](https://github.com/developing-today-forks/openchamber/pull/1556) | fix(quota): read Codex quota from provider instance credentials                                         | developing-today[bot] | 2026-11-04 |
| [#1867](https://github.com/developing-today-forks/openchamber/pull/1867) | fix(quota): unify minimax + wafer key selection (web + VS Code)                                         | dezren39              | 2026-11-08 |
| [#2003](https://github.com/developing-today-forks/openchamber/pull/2003) | fix(quota): retire the duplicate github-copilot-addon provider (#1738)                                  | developing-today[bot] | 2026-11-08 |
| [#2075](https://github.com/developing-today-forks/openchamber/pull/2075) | fix(quota): fold opencode-go instance rows into the base answer (code#8260)                             | dezren39              | 2026-11-10 |
| [#2318](https://github.com/developing-today-forks/openchamber/pull/2318) | fix(quota): unify CLI + server endpoint onto the direct credential-store read (#2282)                   | dezren39              | 2026-11-16 |
| [#2513](https://github.com/developing-today-forks/openchamber/pull/2513) | feat(quota): read the agy keyring login in the Antigravity provider                                     | dezren39              | 2026-11-23 |

##### developing-today-forks/openchamber — theme: routing (48)

| PR                                                                       | Title                                                                                       | Author   | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#303](https://github.com/developing-today-forks/openchamber/pull/303)   | feat(routing): pool health panel with session counts and plan headroom                      | dezren39 | 2026-10-23 |
| [#340](https://github.com/developing-today-forks/openchamber/pull/340)   | fix(routing): accept route schedules in the model pool validator (#298)                     | dezren39 | 2026-10-23 |
| [#343](https://github.com/developing-today-forks/openchamber/pull/343)   | fix(routing): refuse pool schedule windows with equal start and end (#341)                  | dezren39 | 2026-10-23 |
| [#345](https://github.com/developing-today-forks/openchamber/pull/345)   | fix(routing): refuse non-finite pool weights and health rates (#344)                        | dezren39 | 2026-10-23 |
| [#348](https://github.com/developing-today-forks/openchamber/pull/348)   | fix(routing): refuse a pool budget keyed to a target the route does not have (#347)         | dezren39 | 2026-10-23 |
| [#350](https://github.com/developing-today-forks/openchamber/pull/350)   | fix(routing): refuse schedule targets that match no route target (#349)                     | dezren39 | 2026-10-23 |
| [#355](https://github.com/developing-today-forks/openchamber/pull/355)   | fix(routing): refuse a pattern budgets key in the model pool validator (#354)               | dezren39 | 2026-10-23 |
| [#357](https://github.com/developing-today-forks/openchamber/pull/357)   | fix(routing): accept a schedule pattern that partly overlaps a route target pattern (#353)  | dezren39 | 2026-10-23 |
| [#361](https://github.com/developing-today-forks/openchamber/pull/361)   | fix(routing): refuse route references OpenCode would silently drop (#356)                   | dezren39 | 2026-10-23 |
| [#362](https://github.com/developing-today-forks/openchamber/pull/362)   | fix(routing): judge schedule targets the way OpenCode applies them                          | dezren39 | 2026-10-23 |
| [#360](https://github.com/developing-today-forks/openchamber/pull/360)   | fix(routing): refuse weights and budgets on a fixed model route                             | dezren39 | 2026-10-23 |
| [#370](https://github.com/developing-today-forks/openchamber/pull/370)   | fix(routing): refuse empty variant settings on pool targets                                 | dezren39 | 2026-10-24 |
| [#372](https://github.com/developing-today-forks/openchamber/pull/372)   | fix(routing): refuse pattern weights keys in model pool validator                           | dezren39 | 2026-10-24 |
| [#376](https://github.com/developing-today-forks/openchamber/pull/376)   | fix(routing): judge pool budgets through route references                                   | dezren39 | 2026-10-24 |
| [#375](https://github.com/developing-today-forks/openchamber/pull/375)   | fix(routing): refuse a schedule window that has already ended (#374)                        | dezren39 | 2026-10-24 |
| [#378](https://github.com/developing-today-forks/openchamber/pull/378)   | fix(routing): refuse duplicate targets in a model pool route (#377)                         | dezren39 | 2026-10-24 |
| [#380](https://github.com/developing-today-forks/openchamber/pull/380)   | fix(routing): refuse a model pool target that references its own route (#379)               | dezren39 | 2026-10-24 |
| [#381](https://github.com/developing-today-forks/openchamber/pull/381)   | fix(routing): judge pool schedule targets through route references (#364)                   | dezren39 | 2026-10-24 |
| [#387](https://github.com/developing-today-forks/openchamber/pull/387)   | fix(routing): refuse a target until that can never apply (#373)                             | dezren39 | 2026-10-24 |
| [#392](https://github.com/developing-today-forks/openchamber/pull/392)   | fix(routing): refuse a route-reference target that is trail-dead (#382)                     | dezren39 | 2026-10-24 |
| [#396](https://github.com/developing-today-forks/openchamber/pull/396)   | fix(routing): refuse variant settings on a route-reference target                           | dezren39 | 2026-10-24 |
| [#398](https://github.com/developing-today-forks/openchamber/pull/398)   | fix(routing): refuse a concrete target shadowed by an earlier pattern target                | dezren39 | 2026-10-24 |
| [#407](https://github.com/developing-today-forks/openchamber/pull/407)   | fix(routing): refuse a pattern target fully shadowed by an earlier pattern target           | dezren39 | 2026-10-24 |
| [#413](https://github.com/developing-today-forks/openchamber/pull/413)   | fix(routing): abort local model download once it passes the pinned size                     | dezren39 | 2026-10-24 |
| [#437](https://github.com/developing-today-forks/openchamber/pull/437)   | fix(routing): judge schedule targets by their trimmed spelling                              | dezren39 | 2026-10-25 |
| [#444](https://github.com/developing-today-forks/openchamber/pull/444)   | fix(routing): refuse padded and empty-segment target references and padded variant settings | dezren39 | 2026-10-25 |
| [#453](https://github.com/developing-today-forks/openchamber/pull/453)   | fix(routing): reject target references with whitespace inside a segment or variant          | dezren39 | 2026-10-25 |
| [#305](https://github.com/developing-today-forks/openchamber/pull/305)   | feat(routing): pool description and details, given to Jev                                   | dezren39 | 2026-10-26 |
| [#513](https://github.com/developing-today-forks/openchamber/pull/513)   | test(routing): pin same-plan pattern overlap counting in pool health                        | dezren39 | 2026-10-27 |
| [#527](https://github.com/developing-today-forks/openchamber/pull/527)   | fix(routing): never suggest lower-weight for a pattern target                               | dezren39 | 2026-10-27 |
| [#532](https://github.com/developing-today-forks/openchamber/pull/532)   | fix(routing): stats and turns routes read no local DB when OpenCode is remote               | dezren39 | 2026-10-27 |
| [#561](https://github.com/developing-today-forks/openchamber/pull/561)   | fix(routing): read schedule window seconds the way OpenCode does                            | dezren39 | 2026-10-27 |
| [#568](https://github.com/developing-today-forks/openchamber/pull/568)   | fix(routing): save model pools to the user file only (#324)                                 | dezren39 | 2026-10-27 |
| [#613](https://github.com/developing-today-forks/openchamber/pull/613)   | fix(routing): judge pool-health hours/activeMinutes strictly (#604)                         | dezren39 | 2026-10-28 |
| [#635](https://github.com/developing-today-forks/openchamber/pull/635)   | fix(routing): parse stats-route hours strictly (#632)                                       | dezren39 | 2026-10-28 |
| [#710](https://github.com/developing-today-forks/openchamber/pull/710)   | fix(routing): label pool-health projection source and explain unprojected windows           | dezren39 | 2026-10-28 |
| [#717](https://github.com/developing-today-forks/openchamber/pull/717)   | feat(routing): tier chooser and quota fallback card helper (slice 1, #14)                   | dezren39 | 2026-10-28 |
| [#1470](https://github.com/developing-today-forks/openchamber/pull/1470) | feat(routing): staggered rotation planner for pool targets (#295 stage 1)                   | dezren39 | 2026-11-03 |
| [#2320](https://github.com/developing-today-forks/openchamber/pull/2320) | feat(routing): wire the tier-chooser card into a production action (#14 slice 2)            | dezren39 | 2026-11-16 |
| [#2323](https://github.com/developing-today-forks/openchamber/pull/2323) | feat(routing): wire the quota-fallback card into a production action (#14 slice 3)          | dezren39 | 2026-11-16 |
| [#2406](https://github.com/developing-today-forks/openchamber/pull/2406) | test(routing): keep the folder-config fixture out of a home that has ~/.opencode            | dezren39 | 2026-11-17 |
| [#2405](https://github.com/developing-today-forks/openchamber/pull/2405) | feat(routing): move sessions off a target one at a time before its plan runs out            | dezren39 | 2026-11-17 |
| [#2469](https://github.com/developing-today-forks/openchamber/pull/2469) | feat(routing): rebuild model pool settings around one page per pool                         | dezren39 | 2026-11-22 |
| [#2482](https://github.com/developing-today-forks/openchamber/pull/2482) | feat(routing): pin a pool member instead of leaving the pool                                | dezren39 | 2026-11-22 |
| [#2489](https://github.com/developing-today-forks/openchamber/pull/2489) | fix(routing): fixes found driving the pool settings in a real browser                       | dezren39 | 2026-11-22 |
| [#2492](https://github.com/developing-today-forks/openchamber/pull/2492) | feat(routing): accept OpenCode's last-target wait and explain it in Settings                | dezren39 | 2026-11-22 |
| [#2493](https://github.com/developing-today-forks/openchamber/pull/2493) | fix(routing): offer only the effort fixes that change something for a pool of pools         | dezren39 | 2026-11-22 |
| [#2497](https://github.com/developing-today-forks/openchamber/pull/2497) | docs(routing): a pool moves on after a failure that follows output                          | dezren39 | 2026-11-22 |

##### developing-today-forks/openchamber — theme: electron (37)

| PR                                                                       | Title                                                                                                                                 | Author   | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#6](https://github.com/developing-today-forks/openchamber/pull/6)       | build(electron): point the desktop updater at the fork's releases                                                                     | dezren39 | 2026-10-13 |
| [#8](https://github.com/developing-today-forks/openchamber/pull/8)       | fix(electron): keep other-architecture llama addons out of the Linux AppImage                                                         | dezren39 | 2026-10-14 |
| [#10](https://github.com/developing-today-forks/openchamber/pull/10)     | fix(electron): prune other-architecture llama store entries before packaging                                                          | dezren39 | 2026-10-14 |
| [#138](https://github.com/developing-today-forks/openchamber/pull/138)   | ci(electron): run test:linux-desktop in pr checks                                                                                     | dezren39 | 2026-10-20 |
| [#136](https://github.com/developing-today-forks/openchamber/pull/136)   | fix(electron): replace no-op lint stub with the shared oxlint command                                                                 | dezren39 | 2026-10-20 |
| [#159](https://github.com/developing-today-forks/openchamber/pull/159)   | fix(electron): make the build script bundle the main process                                                                          | dezren39 | 2026-10-20 |
| [#190](https://github.com/developing-today-forks/openchamber/pull/190)   | fix(electron): reject malformed versions in compareSemver                                                                             | dezren39 | 2026-10-21 |
| [#227](https://github.com/developing-today-forks/openchamber/pull/227)   | fix(electron): reject malformed ports instead of coercing them with parseInt                                                          | dezren39 | 2026-10-21 |
| [#242](https://github.com/developing-today-forks/openchamber/pull/242)   | fix(electron): read managed remote server port from serve --quiet output (#241)                                                       | dezren39 | 2026-10-22 |
| [#248](https://github.com/developing-today-forks/openchamber/pull/248)   | fix(electron): read remote probe statuses by prefix, not by line position (#246)                                                      | dezren39 | 2026-10-22 |
| [#254](https://github.com/developing-today-forks/openchamber/pull/254)   | fix(electron): read ssh remote answers past login-shell banner output (#253)                                                          | dezren39 | 2026-10-22 |
| [#257](https://github.com/developing-today-forks/openchamber/pull/257)   | fix(electron): do not return a banner line as the resolved remote tool (#255)                                                         | dezren39 | 2026-10-22 |
| [#287](https://github.com/developing-today-forks/openchamber/pull/287)   | fix(electron): judge SSH probe statuses whole so trailing junk cannot fake a healthy server                                           | dezren39 | 2026-10-22 |
| [#290](https://github.com/developing-today-forks/openchamber/pull/290)   | fix(electron): SSH probe statuses must use the canonical shape — leading-zero spellings no longer fake health or an accepted password | dezren39 | 2026-10-22 |
| [#312](https://github.com/developing-today-forks/openchamber/pull/312)   | fix(electron): reject leading-zero spellings in the SSH served-port parser (#311)                                                     | dezren39 | 2026-10-22 |
| [#314](https://github.com/developing-today-forks/openchamber/pull/314)   | fix(electron): drop out-of-range ports from the SSH server registry (#313)                                                            | dezren39 | 2026-10-22 |
| [#315](https://github.com/developing-today-forks/openchamber/pull/315)   | fix(electron): reject whitespace-padded SSH probe statuses (#292)                                                                     | dezren39 | 2026-10-22 |
| [#317](https://github.com/developing-today-forks/openchamber/pull/317)   | fix(electron): range-check ports in SSH instance sanitizers (#316)                                                                    | dezren39 | 2026-10-22 |
| [#420](https://github.com/developing-today-forks/openchamber/pull/420)   | refactor(electron): use validPort in listRemoteServers filter                                                                         | dezren39 | 2026-10-24 |
| [#749](https://github.com/developing-today-forks/openchamber/pull/749)   | fix(electron): cap SSH command output and add an execution deadline                                                                   | dezren39 | 2026-10-29 |
| [#818](https://github.com/developing-today-forks/openchamber/pull/818)   | fix(electron): deadline and bounded body read for favicon fetch (#817)                                                                | dezren39 | 2026-10-29 |
| [#837](https://github.com/developing-today-forks/openchamber/pull/837)   | fix(electron): bound the four production spawnSync calls with timeouts (#833)                                                         | dezren39 | 2026-10-29 |
| [#1260](https://github.com/developing-today-forks/openchamber/pull/1260) | fix(electron): stat-gate installed-app icon reads at 512 KiB (#1257)                                                                  | dezren39 | 2026-11-01 |
| [#1262](https://github.com/developing-today-forks/openchamber/pull/1262) | fix(electron): cap Linux .desktop entry scan reads at 512 KiB (#1261)                                                                 | dezren39 | 2026-11-01 |
| [#1578](https://github.com/developing-today-forks/openchamber/pull/1578) | fix(electron): decode SSH runSshOutput stdout incrementally                                                                           | dezren39 | 2026-11-04 |
| [#2085](https://github.com/developing-today-forks/openchamber/pull/2085) | fix(electron): updater E2E fixture returns HTTP errors instead of crashing when a served file disappears (#2078)                      | dezren39 | 2026-11-11 |
| [#2307](https://github.com/developing-today-forks/openchamber/pull/2307) | test(electron): web-server-imports guard covers the .then() destructure (#2306)                                                       | dezren39 | 2026-11-16 |
| [#2429](https://github.com/developing-today-forks/openchamber/pull/2429) | fix(electron): clean up window geometry debounce state                                                                                | dezren39 | 2026-11-20 |
| [#2439](https://github.com/developing-today-forks/openchamber/pull/2439) | fix(electron): handle installed-app refresh failures                                                                                  | dezren39 | 2026-11-22 |
| [#2392](https://github.com/developing-today-forks/openchamber/pull/2392) | fix(electron): strip URL-embedded credentials in remote hosts redaction (#2319)                                                       | dezren39 | 2026-11-22 |
| [#2446](https://github.com/developing-today-forks/openchamber/pull/2446) | fix(electron): preserve Linux autostart entry ownership                                                                               | dezren39 | 2026-11-22 |
| [#2539](https://github.com/developing-today-forks/openchamber/pull/2539) | fix(electron): refuse to rewrite a malformed settings.json from SSH write paths                                                       | dezren39 | 2026-11-23 |
| [#2541](https://github.com/developing-today-forks/openchamber/pull/2541) | fix(electron): handle rejected host deep-link activation                                                                              | dezren39 | 2026-11-23 |
| [#2544](https://github.com/developing-today-forks/openchamber/pull/2544) | fix(electron): handle rejected window-geometry persistence                                                                            | dezren39 | 2026-11-23 |
| [#2545](https://github.com/developing-today-forks/openchamber/pull/2545) | fix(electron): release isolated-space browser preview state after space removal                                                       | dezren39 | 2026-11-23 |
| [#2562](https://github.com/developing-today-forks/openchamber/pull/2562) | fix(electron): observe initial window navigation rejections                                                                           | dezren39 | 2026-11-23 |
| [#2567](https://github.com/developing-today-forks/openchamber/pull/2567) | fix(electron): handle rejected splash-color settings writes                                                                           | dezren39 | 2026-11-23 |

##### developing-today-forks/openchamber — theme: roadmap (35)

| PR                                                                     | Title                                                                                | Author   | Merged     |
| ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------ | -------- | ---------- |
| [#231](https://github.com/developing-today-forks/openchamber/pull/231) | docs(roadmap): record the #226 completion in Done                                    | dezren39 | 2026-10-21 |
| [#240](https://github.com/developing-today-forks/openchamber/pull/240) | docs(roadmap): record the #228, #230, #233 and #235 completions in Done (#239)       | dezren39 | 2026-10-21 |
| [#244](https://github.com/developing-today-forks/openchamber/pull/244) | docs(roadmap): record the #236 and #239 completions in Done (#243)                   | dezren39 | 2026-10-22 |
| [#251](https://github.com/developing-today-forks/openchamber/pull/251) | docs(roadmap): record the #241, #243, #174 and #246 completions in Done (#250)       | dezren39 | 2026-10-22 |
| [#258](https://github.com/developing-today-forks/openchamber/pull/258) | docs(roadmap): record the #247, #250 and #253 completions in Done (#252, #256)       | dezren39 | 2026-10-22 |
| [#260](https://github.com/developing-today-forks/openchamber/pull/260) | docs(roadmap): record the #255 and #252/#256 completions in Done (#259)              | dezren39 | 2026-10-22 |
| [#263](https://github.com/developing-today-forks/openchamber/pull/263) | docs(roadmap): record the #259 completion in Done (#262)                             | dezren39 | 2026-10-22 |
| [#267](https://github.com/developing-today-forks/openchamber/pull/267) | docs(roadmap): record the #178, #207 and #264 completions in Done                    | dezren39 | 2026-10-22 |
| [#278](https://github.com/developing-today-forks/openchamber/pull/278) | docs(roadmap): record the #266 and #268 completions in Done                          | dezren39 | 2026-10-22 |
| [#346](https://github.com/developing-today-forks/openchamber/pull/346) | docs(roadmap): record the #274 completion in Done (#279)                             | dezren39 | 2026-10-23 |
| [#352](https://github.com/developing-today-forks/openchamber/pull/352) | docs(roadmap): record the #279, #341, #344, #347 and #349 completions in Done (#351) | dezren39 | 2026-10-23 |
| [#366](https://github.com/developing-today-forks/openchamber/pull/366) | docs(roadmap): record the #351, #353, #354, #356 and #358 completions in Done (#365) | dezren39 | 2026-10-24 |
| [#368](https://github.com/developing-today-forks/openchamber/pull/368) | docs(roadmap): record the #359 completion in Done (#367)                             | dezren39 | 2026-10-24 |
| [#397](https://github.com/developing-today-forks/openchamber/pull/397) | docs(roadmap): record the #277, #283 and #286 completions in Done                    | dezren39 | 2026-10-24 |
| [#399](https://github.com/developing-today-forks/openchamber/pull/399) | docs(roadmap): record the #288 completion in Done                                    | dezren39 | 2026-10-24 |
| [#400](https://github.com/developing-today-forks/openchamber/pull/400) | docs(roadmap): record the #285 completion in Done                                    | dezren39 | 2026-10-24 |
| [#408](https://github.com/developing-today-forks/openchamber/pull/408) | docs(roadmap): record the #311, #313, #292, #316 and #281 completions in Done        | dezren39 | 2026-10-24 |
| [#409](https://github.com/developing-today-forks/openchamber/pull/409) | docs(roadmap): record the #299 and #294 completions in Done                          | dezren39 | 2026-10-24 |
| [#338](https://github.com/developing-today-forks/openchamber/pull/338) | docs(roadmap): record the #282 completion in Done (#337)                             | dezren39 | 2026-10-24 |
| [#411](https://github.com/developing-today-forks/openchamber/pull/411) | docs(roadmap): record the #298 and #335 completions in Done (#342)                   | dezren39 | 2026-10-24 |
| [#394](https://github.com/developing-today-forks/openchamber/pull/394) | docs(roadmap): record the #363–#379 completions in Done                              | dezren39 | 2026-10-24 |
| [#414](https://github.com/developing-today-forks/openchamber/pull/414) | docs(roadmap): record the #382, #395, #383, #321, #406 and #334 completions in Done  | dezren39 | 2026-10-24 |
| [#416](https://github.com/developing-today-forks/openchamber/pull/416) | docs(roadmap): record the #412 and #410 completions in Done                          | dezren39 | 2026-10-24 |
| [#421](https://github.com/developing-today-forks/openchamber/pull/421) | docs(roadmap): record the #415 completion in Done                                    | dezren39 | 2026-10-24 |
| [#426](https://github.com/developing-today-forks/openchamber/pull/426) | docs(roadmap): record the #418 and #419 completions in Done (#425)                   | dezren39 | 2026-10-25 |
| [#431](https://github.com/developing-today-forks/openchamber/pull/431) | docs(roadmap): record the #417, #425 and #422 completions in Done                    | dezren39 | 2026-10-25 |
| [#436](https://github.com/developing-today-forks/openchamber/pull/436) | docs(roadmap): record the #428 and #430 completions in Done                          | dezren39 | 2026-10-25 |
| [#442](https://github.com/developing-today-forks/openchamber/pull/442) | docs(roadmap): record the #432 completion in Done                                    | dezren39 | 2026-10-25 |
| [#454](https://github.com/developing-today-forks/openchamber/pull/454) | docs(roadmap): record the #441, #438, #439/#440, #446 and #453 completions in Done   | dezren39 | 2026-10-25 |
| [#455](https://github.com/developing-today-forks/openchamber/pull/455) | docs(roadmap): record the #450 completion in Done                                    | dezren39 | 2026-10-25 |
| [#461](https://github.com/developing-today-forks/openchamber/pull/461) | docs(roadmap): record the #448, #452, #456, #459 and #460 completions in Done        | dezren39 | 2026-10-26 |
| [#470](https://github.com/developing-today-forks/openchamber/pull/470) | docs(roadmap): record the #462 and #466 completions in Done                          | dezren39 | 2026-10-26 |
| [#472](https://github.com/developing-today-forks/openchamber/pull/472) | docs(roadmap): record the #469 and #468 completions in Done                          | dezren39 | 2026-10-26 |
| [#485](https://github.com/developing-today-forks/openchamber/pull/485) | docs(roadmap): record the #474, #475 and #478 completions in Done                    | dezren39 | 2026-10-26 |
| [#810](https://github.com/developing-today-forks/openchamber/pull/810) | docs(roadmap): program tending 2026-10-29 — drop landed rows                         | dezren39 | 2026-10-29 |

##### developing-today-forks/openchamber — theme: other/no-scope (26)

| PR                                                                       | Title                                                                                                                          | Author                | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------ | --------------------- | ---------- |
| [#24](https://github.com/developing-today-forks/openchamber/pull/24)     | fix: make check:changed pass on main                                                                                           | dezren39              | 2026-10-16 |
| [#106](https://github.com/developing-today-forks/openchamber/pull/106)   | fix: retry atomic-write renames on transient Windows errors in the remaining stores (#99)                                      | dezren39              | 2026-10-19 |
| [#121](https://github.com/developing-today-forks/openchamber/pull/121)   | chore: untrack root .env (covered by .gitignore 'Local env')                                                                   | dezren39              | 2026-10-19 |
| [#126](https://github.com/developing-today-forks/openchamber/pull/126)   | fix: count subagent chains at any depth in ancestor walks (#123)                                                               | dezren39              | 2026-10-19 |
| [#166](https://github.com/developing-today-forks/openchamber/pull/166)   | chore: remove tracked .create-pr-proof screenshots and gitignore the directory                                                 | dezren39              | 2026-10-20 |
| [#168](https://github.com/developing-today-forks/openchamber/pull/168)   | Unify the app id on today.developing.openchamber                                                                               | dezren39              | 2026-10-20 |
| [#172](https://github.com/developing-today-forks/openchamber/pull/172)   | Android: check GitHub releases directly and offer the release page                                                             | dezren39              | 2026-10-20 |
| [#177](https://github.com/developing-today-forks/openchamber/pull/177)   | fix: dead-code check failed in any checkout under a .local folder                                                              | dezren39              | 2026-10-21 |
| [#179](https://github.com/developing-today-forks/openchamber/pull/179)   | feat: agents can read a project's setup (projects.setup)                                                                       | dezren39              | 2026-10-21 |
| [#713](https://github.com/developing-today-forks/openchamber/pull/713)   | Add durable OpenChamber cursors for local event frames                                                                         | dezren39              | 2026-10-28 |
| [#1442](https://github.com/developing-today-forks/openchamber/pull/1442) | openchamber: services.openchamber modules (cascade top, attach by default)                                                     | dezren39              | 2026-11-03 |
| [#1448](https://github.com/developing-today-forks/openchamber/pull/1448) | fix: remove dead outside-file grant machinery (#934)                                                                           | developing-today[bot] | 2026-11-03 |
| [#1484](https://github.com/developing-today-forks/openchamber/pull/1484) | chore: bump opencode pin to 12acd9f2                                                                                           | developing-today[bot] | 2026-11-03 |
| [#1604](https://github.com/developing-today-forks/openchamber/pull/1604) | chore: point functional references at developing-today-forks after org move                                                    | dezren39              | 2026-11-05 |
| [#2303](https://github.com/developing-today-forks/openchamber/pull/2303) | Restore provideLoginShellEnvSnapshot export, guard Electron web-server imports                                                 | dezren39              | 2026-11-16 |
| [#2322](https://github.com/developing-today-forks/openchamber/pull/2322) | Versioned v2 managed-OpenCode state store, write-ahead handoff, v1 migration, store-only reaper (#82)                          | dezren39              | 2026-11-16 |
| [#2416](https://github.com/developing-today-forks/openchamber/pull/2416) | Reject repeated run-history time filters                                                                                       | dezren39              | 2026-11-20 |
| [#2317](https://github.com/developing-today-forks/openchamber/pull/2317) | Sidebar view mode without headers that keeps the section order                                                                 | dezren39              | 2026-11-22 |
| [#2473](https://github.com/developing-today-forks/openchamber/pull/2473) | Add reviewed fusion passes and generated prompt variants                                                                       | dezren39              | 2026-11-22 |
| [#2479](https://github.com/developing-today-forks/openchamber/pull/2479) | Explain unavailable server-wide models during prompt generation                                                                | dezren39              | 2026-11-22 |
| [#2481](https://github.com/developing-today-forks/openchamber/pull/2481) | Merge dezren39/openchamber main into the fork (content already here)                                                           | dezren39              | 2026-11-22 |
| [#2485](https://github.com/developing-today-forks/openchamber/pull/2485) | Retain project prompt generation and complete legacy run controls                                                              | dezren39              | 2026-11-22 |
| [#2484](https://github.com/developing-today-forks/openchamber/pull/2484) | Goal loop: wait on background work, check on stalled jobs, recover from errors and refusals; Ctrl+B background in the composer | dezren39              | 2026-11-22 |
| [#2491](https://github.com/developing-today-forks/openchamber/pull/2491) | Upgrade takeover for self-handoff: handoff protocol 2 (#81)                                                                    | dezren39              | 2026-11-22 |
| [#2496](https://github.com/developing-today-forks/openchamber/pull/2496) | Probe successor gives up when its predecessor dies mid-exchange (#81 follow-up)                                                | dezren39              | 2026-11-22 |
| [#2540](https://github.com/developing-today-forks/openchamber/pull/2540) | Name the resolved database path in the managed launch log (#2501)                                                              | dezren39              | 2026-11-23 |

##### developing-today-forks/openchamber — theme: web,vscode (22)

| PR                                                                       | Title                                                                                                  | Author                | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------ | --------------------- | ---------- |
| [#825](https://github.com/developing-today-forks/openchamber/pull/825)   | fix(web,vscode): timeout on Cursor credential import sqlite3 reads (#822)                              | dezren39              | 2026-10-29 |
| [#836](https://github.com/developing-today-forks/openchamber/pull/836)   | fix(web,vscode): bound timer-feeding env readers at 2^31-1 ms (#832)                                   | dezren39              | 2026-10-29 |
| [#863](https://github.com/developing-today-forks/openchamber/pull/863)   | fix(web,vscode): bound the gpgconf probes in resolveSshAuthSock with a 5s timeout                      | dezren39              | 2026-10-30 |
| [#864](https://github.com/developing-today-forks/openchamber/pull/864)   | fix(web,vscode): bound the worktree start command with a 10-minute timeout                             | dezren39              | 2026-10-30 |
| [#880](https://github.com/developing-today-forks/openchamber/pull/880)   | fix(web,vscode): terminate the worktree start-command process tree on timeout                          | dezren39              | 2026-10-30 |
| [#969](https://github.com/developing-today-forks/openchamber/pull/969)   | fix(web,vscode): bound npm-registry metadata caches with sweep and cap (#967)                          | dezren39              | 2026-10-30 |
| [#1046](https://github.com/developing-today-forks/openchamber/pull/1046) | fix(web,vscode): google quota is the Code Assist per-model view — no fabricated daily window (#1042)   | dezren39              | 2026-10-31 |
| [#1115](https://github.com/developing-today-forks/openchamber/pull/1115) | fix(web,vscode): stat-gate skills-catalog SKILL.md disk reads at 4 MiB (#1103)                         | dezren39              | 2026-11-01 |
| [#1252](https://github.com/developing-today-forks/openchamber/pull/1252) | fix(web,vscode): cap shared project config reads at 1 MiB (#1247)                                      | dezren39              | 2026-11-01 |
| [#1323](https://github.com/developing-today-forks/openchamber/pull/1323) | fix(web,vscode): cap file-diff committed/index blob reads (#1272)                                      | dezren39              | 2026-11-02 |
| [#1363](https://github.com/developing-today-forks/openchamber/pull/1363) | fix(web,vscode): bound file-search directory visits (#1332)                                            | dezren39              | 2026-11-02 |
| [#1380](https://github.com/developing-today-forks/openchamber/pull/1380) | fix(web,vscode): cap skill-discovery walk at 5000 directories per root                                 | dezren39              | 2026-11-03 |
| [#1421](https://github.com/developing-today-forks/openchamber/pull/1421) | fix(web,vscode): cap skill supporting-files listing at 5000 directories and 5000 files (#1365)         | dezren39              | 2026-11-03 |
| [#1477](https://github.com/developing-today-forks/openchamber/pull/1477) | fix(web,vscode): cap skills-catalog install copies by entries and bytes (#1366)                        | dezren39              | 2026-11-03 |
| [#1493](https://github.com/developing-today-forks/openchamber/pull/1493) | test(web,vscode): land the agent-directory walk cap regression tests                                   | dezren39              | 2026-11-03 |
| [#1519](https://github.com/developing-today-forks/openchamber/pull/1519) | refactor(web,vscode): skill walks consult the shared traversal budget (#1465)                          | dezren39              | 2026-11-04 |
| [#1498](https://github.com/developing-today-forks/openchamber/pull/1498) | fix(web,vscode): cap plugin, skill supporting-file and markdown writes at their read caps (#1444)      | dezren39              | 2026-11-04 |
| [#1718](https://github.com/developing-today-forks/openchamber/pull/1718) | fix(web,vscode): decode managed OpenCode startup output without corrupting split multi-byte characters | dezren39              | 2026-11-06 |
| [#1996](https://github.com/developing-today-forks/openchamber/pull/1996) | fix(web,vscode): keep the project-setup clamp surrogate-pair safe (#1990)                              | developing-today[bot] | 2026-11-08 |
| [#2275](https://github.com/developing-today-forks/openchamber/pull/2275) | fix(web,vscode): bound personal project-config reads at the shared 1 MiB cap (#1490)                   | dezren39              | 2026-11-14 |
| [#2297](https://github.com/developing-today-forks/openchamber/pull/2297) | fix(web,vscode): judge SKILL.md against its 4 MiB limit during install copy                            | dezren39              | 2026-11-16 |
| [#2374](https://github.com/developing-today-forks/openchamber/pull/2374) | fix(web,vscode): enforce fs byte caps during the read, not only on the stat (#1605)                    | dezren39              | 2026-11-17 |

##### developing-today-forks/openchamber — theme: docs (20)

| PR                                                                     | Title                                                                                                | Author   | Merged     |
| ---------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#83](https://github.com/developing-today-forks/openchamber/pull/83)   | docs: add roadmap, planning folder and templates                                                     | dezren39 | 2026-10-18 |
| [#90](https://github.com/developing-today-forks/openchamber/pull/90)   | docs: move the fixed #79 cost bug and the #84/#85 follow-up into ROADMAP's Done table (#89)          | dezren39 | 2026-10-19 |
| [#110](https://github.com/developing-today-forks/openchamber/pull/110) | docs: remove stale WSL distro env var sections (#109)                                                | dezren39 | 2026-10-19 |
| [#115](https://github.com/developing-today-forks/openchamber/pull/115) | docs: record 2026-10-19 merged work in ROADMAP Done table                                            | dezren39 | 2026-10-19 |
| [#130](https://github.com/developing-today-forks/openchamber/pull/130) | docs: record the later 2026-10-19 merges in ROADMAP Done (#129)                                      | dezren39 | 2026-10-19 |
| [#141](https://github.com/developing-today-forks/openchamber/pull/141) | docs: record the post-#124 merges in ROADMAP Done                                                    | dezren39 | 2026-10-20 |
| [#154](https://github.com/developing-today-forks/openchamber/pull/154) | docs: record the #143 v7 action-pin alignment in ROADMAP Done                                        | dezren39 | 2026-10-20 |
| [#161](https://github.com/developing-today-forks/openchamber/pull/161) | docs: record #155, #157 and #142 in ROADMAP Done table                                               | dezren39 | 2026-10-20 |
| [#164](https://github.com/developing-today-forks/openchamber/pull/164) | docs: record #162 in ROADMAP Done table                                                              | dezren39 | 2026-10-20 |
| [#171](https://github.com/developing-today-forks/openchamber/pull/171) | docs: record #165 .create-pr-proof cleanup in ROADMAP Done                                           | dezren39 | 2026-10-21 |
| [#192](https://github.com/developing-today-forks/openchamber/pull/192) | docs: record the five version-parse completions in ROADMAP Done                                      | dezren39 | 2026-10-21 |
| [#213](https://github.com/developing-today-forks/openchamber/pull/213) | docs: record the #208/#175 completion in ROADMAP Done                                                | dezren39 | 2026-10-21 |
| [#221](https://github.com/developing-today-forks/openchamber/pull/221) | docs: record #209 and #214/#215 completions in ROADMAP Done                                          | dezren39 | 2026-10-21 |
| [#225](https://github.com/developing-today-forks/openchamber/pull/225) | docs: ROADMAP Done rows for #191/#186/#173/#220/#199 (supersedes #202)                               | dezren39 | 2026-10-21 |
| [#237](https://github.com/developing-today-forks/openchamber/pull/237) | docs: record the #222 completion in ROADMAP Done                                                     | dezren39 | 2026-10-21 |
| [#443](https://github.com/developing-today-forks/openchamber/pull/443) | docs: ROADMAP Done table omits the #433 and #435 completions                                         | dezren39 | 2026-10-25 |
| [#487](https://github.com/developing-today-forks/openchamber/pull/487) | docs: remove ROADMAP Done tables — GitHub is the record                                              | dezren39 | 2026-10-26 |
| [#599](https://github.com/developing-today-forks/openchamber/pull/599) | docs: bring the auto-model-routing mirrors up to the English page (#332)                             | dezren39 | 2026-10-27 |
| [#605](https://github.com/developing-today-forks/openchamber/pull/605) | docs: add the missing sections to the install, notes-todos-plans, and desktop-browser mirrors (#339) | dezren39 | 2026-10-28 |
| [#610](https://github.com/developing-today-forks/openchamber/pull/610) | docs: add the missing sections to the projects, scheduled-tasks, and walkthrough mirrors (#601)      | dezren39 | 2026-10-28 |

##### developing-today-forks/openchamber — theme: nix (14)

| PR                                                                       | Title                                                                                  | Author   | Merged     |
| ------------------------------------------------------------------------ | -------------------------------------------------------------------------------------- | -------- | ---------- |
| [#3](https://github.com/developing-today-forks/openchamber/pull/3)       | build(nix): add Android SDK and JDK 21 to the dev shells                               | dezren39 | 2026-10-12 |
| [#269](https://github.com/developing-today-forks/openchamber/pull/269)   | fix(nix): update the aarch64-darwin node_modules hash for the synced bun.lock          | dezren39 | 2026-10-22 |
| [#280](https://github.com/developing-today-forks/openchamber/pull/280)   | fix(nix): update stale x86_64-linux node_modules hash (#275)                           | dezren39 | 2026-10-22 |
| [#423](https://github.com/developing-today-forks/openchamber/pull/423)   | fix(nix): update stale x86_64-linux node_modules hash                                  | dezren39 | 2026-10-25 |
| [#488](https://github.com/developing-today-forks/openchamber/pull/488)   | fix(nix): make the node_modules FOD tree deterministic across builders                 | dezren39 | 2026-10-26 |
| [#876](https://github.com/developing-today-forks/openchamber/pull/876)   | fix(nix): re-bake the x86_64-linux node_modules hash                                   | dezren39 | 2026-10-30 |
| [#1386](https://github.com/developing-today-forks/openchamber/pull/1386) | chore(nix): follow fleet nixpkgs pin, add fleet cache and lean fleet shell             | dezren39 | 2026-11-03 |
| [#1389](https://github.com/developing-today-forks/openchamber/pull/1389) | fix(nix): accept extra inputs in outputs formals                                       | dezren39 | 2026-11-03 |
| [#1433](https://github.com/developing-today-forks/openchamber/pull/1433) | fix(nix): re-bake aarch64-darwin node_modules hash (#499)                              | dezren39 | 2026-11-03 |
| [#1445](https://github.com/developing-today-forks/openchamber/pull/1445) | chore(nix): bump opencode input to 2d912e02f                                           | dezren39 | 2026-11-03 |
| [#1453](https://github.com/developing-today-forks/openchamber/pull/1453) | fix(nix): re-bake the x86_64-linux node_modules hash after the fleet nixpkgs pin move  | dezren39 | 2026-11-03 |
| [#1467](https://github.com/developing-today-forks/openchamber/pull/1467) | fix(nix): re-bake the aarch64-linux node_modules hash measured on an arm runner (#275) | dezren39 | 2026-11-03 |
| [#1518](https://github.com/developing-today-forks/openchamber/pull/1518) | test(nix): pin the start wrapper guard tr line and leftover reaping (code#2562)        | dezren39 | 2026-11-04 |
| [#2582](https://github.com/developing-today-forks/openchamber/pull/2582) | fix(nix): re-bake x86_64-linux node_modules hash; add darwin hash-measure leg          | dezren39 | 2026-11-23 |

##### developing-today-forks/openchamber — theme: deps (11)

| PR                                                                       | Title                                                                  | Author                | Merged     |
| ------------------------------------------------------------------------ | ---------------------------------------------------------------------- | --------------------- | ---------- |
| [#456](https://github.com/developing-today-forks/openchamber/pull/456)   | chore(deps): update opencode to d7d6466f8342                           | dezren39              | 2026-10-25 |
| [#459](https://github.com/developing-today-forks/openchamber/pull/459)   | chore(deps): keep Renovate off the workflow-owned opencode flake input | dezren39              | 2026-10-25 |
| [#701](https://github.com/developing-today-forks/openchamber/pull/701)   | chore(deps): update opencode to d4eabe55249e                           | github-actions[bot]   | 2026-10-28 |
| [#1834](https://github.com/developing-today-forks/openchamber/pull/1834) | chore(deps): update opencode to 6c43550b12e0                           | developing-today[bot] | 2026-11-08 |
| [#2316](https://github.com/developing-today-forks/openchamber/pull/2316) | chore(deps): update opencode to e7d5bb735405                           | github-actions[bot]   | 2026-11-16 |
| [#2401](https://github.com/developing-today-forks/openchamber/pull/2401) | chore(deps): update opencode to 2c7a113cc602                           | github-actions[bot]   | 2026-11-17 |
| [#2417](https://github.com/developing-today-forks/openchamber/pull/2417) | chore(deps): update opencode to f8522013c590                           | github-actions[bot]   | 2026-11-18 |
| [#2424](https://github.com/developing-today-forks/openchamber/pull/2424) | chore(deps): update opencode to d025633fe33b                           | github-actions[bot]   | 2026-11-19 |
| [#2436](https://github.com/developing-today-forks/openchamber/pull/2436) | chore(deps): update opencode to acc0e5a24fd1                           | github-actions[bot]   | 2026-11-20 |
| [#2454](https://github.com/developing-today-forks/openchamber/pull/2454) | chore(deps): update opencode to b70580bbee83                           | github-actions[bot]   | 2026-11-21 |
| [#2490](https://github.com/developing-today-forks/openchamber/pull/2490) | chore(deps): update opencode to e1d1ee2154f9                           | github-actions[bot]   | 2026-11-22 |

##### developing-today-forks/openchamber — theme: settings (8)

| PR                                                                     | Title                                                                                                      | Author   | Merged     |
| ---------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#45](https://github.com/developing-today-forks/openchamber/pull/45)   | fix(settings): respect discovered config overrides                                                         | dezren39 | 2026-10-18 |
| [#194](https://github.com/developing-today-forks/openchamber/pull/194) | fix(settings): one failed save no longer fails every later save; project actions write under the save lock | dezren39 | 2026-10-21 |
| [#223](https://github.com/developing-today-forks/openchamber/pull/223) | fix(settings): feed updateSettings the migrated settings                                                   | dezren39 | 2026-10-21 |
| [#336](https://github.com/developing-today-forks/openchamber/pull/336) | test(settings): pin refused closing send in device settings reporter (#335)                                | dezren39 | 2026-10-23 |
| [#502](https://github.com/developing-today-forks/openchamber/pull/502) | feat(settings): edit pool schedules and schedules across pools                                             | dezren39 | 2026-10-26 |
| [#522](https://github.com/developing-today-forks/openchamber/pull/522) | fix(settings): give every routing search entry a matching anchor                                           | dezren39 | 2026-10-27 |
| [#526](https://github.com/developing-today-forks/openchamber/pull/526) | fix(settings): show pool weight fields for round-robin pools                                               | dezren39 | 2026-10-27 |
| [#571](https://github.com/developing-today-forks/openchamber/pull/571) | test(settings): assert anchors for every settings-search entry id                                          | dezren39 | 2026-10-27 |

##### developing-today-forks/openchamber — theme: agent-tool (8)

| PR                                                                     | Title                                                                                                       | Author   | Merged     |
| ---------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#37](https://github.com/developing-today-forks/openchamber/pull/37)   | fix(agent-tool): ship server.js re-export in the generated plugin (fixes #35)                               | dezren39 | 2026-10-17 |
| [#39](https://github.com/developing-today-forks/openchamber/pull/39)   | test(agent-tool): update session.messages description expectation                                           | dezren39 | 2026-10-17 |
| [#169](https://github.com/developing-today-forks/openchamber/pull/169) | feat(agent-tool): read and change GitHub, GitLab, Linear, bindings and extensions from the openchamber tool | dezren39 | 2026-10-21 |
| [#151](https://github.com/developing-today-forks/openchamber/pull/151) | fix(agent-tool): session.messages limit of 50 or more fails with OpenCode's page-size error                 | dezren39 | 2026-10-21 |
| [#145](https://github.com/developing-today-forks/openchamber/pull/145) | feat(agent-tool): read and write per-surface settings and project settings                                  | dezren39 | 2026-10-21 |
| [#198](https://github.com/developing-today-forks/openchamber/pull/198) | feat(agent-tool): opencode.config.set can write the custom OpenCode config file                             | dezren39 | 2026-10-21 |
| [#204](https://github.com/developing-today-forks/openchamber/pull/204) | docs(agent-tool): say new sessions get a worktree by default and projects can be added                      | dezren39 | 2026-10-22 |
| [#272](https://github.com/developing-today-forks/openchamber/pull/272) | feat(agent-tool): agents can read each window's device-only settings (settings.devices)                     | dezren39 | 2026-10-22 |

##### developing-today-forks/openchamber — theme: chat (7)

| PR                                                                       | Title                                                                     | Author   | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------- | -------- | ---------- |
| [#27](https://github.com/developing-today-forks/openchamber/pull/27)     | fix(chat): expand ~ in clicked file references to the home directory      | dezren39 | 2026-10-16 |
| [#43](https://github.com/developing-today-forks/openchamber/pull/43)     | feat(chat): deny a permission with feedback to the agent                  | dezren39 | 2026-10-17 |
| [#319](https://github.com/developing-today-forks/openchamber/pull/319)   | fix(chat): type the composer's send call against the session store        | dezren39 | 2026-10-22 |
| [#721](https://github.com/developing-today-forks/openchamber/pull/721)   | fix(chat): keep the transcript visible when the list footer resizes (#52) | dezren39 | 2026-10-28 |
| [#2466](https://github.com/developing-today-forks/openchamber/pull/2466) | feat(chat): show model switches in full and steer pool picks              | dezren39 | 2026-11-22 |
| [#2474](https://github.com/developing-today-forks/openchamber/pull/2474) | feat(chat): continue a session with another model                         | dezren39 | 2026-11-22 |
| [#2475](https://github.com/developing-today-forks/openchamber/pull/2475) | fix(chat): background commands and agent terminals say how they ended     | dezren39 | 2026-11-22 |

##### developing-today-forks/openchamber — theme: mobile (7)

| PR                                                                       | Title                                                                                      | Author   | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------ | -------- | ---------- |
| [#12](https://github.com/developing-today-forks/openchamber/pull/12)     | fix(mobile): check the fork's GitHub releases for Android updates                          | dezren39 | 2026-10-14 |
| [#200](https://github.com/developing-today-forks/openchamber/pull/200)   | fix(mobile): launch the real MainActivity class after the app-id change                    | dezren39 | 2026-10-21 |
| [#427](https://github.com/developing-today-forks/openchamber/pull/427)   | fix(mobile): point android-device.mjs at the flavorless debug APK                          | dezren39 | 2026-10-25 |
| [#386](https://github.com/developing-today-forks/openchamber/pull/386)   | feat(mobile): fork release flavor, native push key registration, and mobile session sheets | dezren39 | 2026-10-25 |
| [#718](https://github.com/developing-today-forks/openchamber/pull/718)   | feat(mobile): phone sheet attention zones, section order and list layout                   | dezren39 | 2026-10-29 |
| [#2349](https://github.com/developing-today-forks/openchamber/pull/2349) | fix(mobile): prevent native push registration after notifications are disabled             | dezren39 | 2026-11-17 |
| [#2387](https://github.com/developing-today-forks/openchamber/pull/2387) | fix(mobile): abort relay requests when the connection timeout expires (#2229)              | dezren39 | 2026-11-17 |

##### developing-today-forks/openchamber — theme: control (7)

| PR                                                                     | Title                                                                               | Author   | Merged     |
| ---------------------------------------------------------------------- | ----------------------------------------------------------------------------------- | -------- | ---------- |
| [#17](https://github.com/developing-today-forks/openchamber/pull/17)   | fix(control): restore the brace the upstream merge dropped                          | dezren39 | 2026-10-16 |
| [#38](https://github.com/developing-today-forks/openchamber/pull/38)   | fix(control): report the target session's directory for status and messages         | dezren39 | 2026-10-17 |
| [#40](https://github.com/developing-today-forks/openchamber/pull/40)   | feat(control): session.archive, session.unarchive and CLI-only session.delete (#36) | dezren39 | 2026-10-17 |
| [#211](https://github.com/developing-today-forks/openchamber/pull/211) | fix(control): mask and refuse bare-key secret names                                 | dezren39 | 2026-10-21 |
| [#219](https://github.com/developing-today-forks/openchamber/pull/219) | fix(control): mask secret subtrees and plural secret names (#214, #215)             | dezren39 | 2026-10-21 |
| [#210](https://github.com/developing-today-forks/openchamber/pull/210) | fix(control): reject malformed weekly weekdays instead of coercing them (#209)      | dezren39 | 2026-10-21 |
| [#232](https://github.com/developing-today-forks/openchamber/pull/232) | fix(control): mask passphrase values in opencode.config answers                     | dezren39 | 2026-10-21 |

##### developing-today-forks/openchamber — theme: flake (6)

| PR                                                                       | Title                                                                                 | Author                | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------- | --------------------- | ---------- |
| [#75](https://github.com/developing-today-forks/openchamber/pull/75)     | chore(flake): pin opencode to 005672f9b3 (nested asks reach the root)                 | dezren39              | 2026-10-18 |
| [#402](https://github.com/developing-today-forks/openchamber/pull/402)   | chore(flake): update opencode to 9b2651e8d5                                           | dezren39              | 2026-10-24 |
| [#1580](https://github.com/developing-today-forks/openchamber/pull/1580) | chore(flake): repoint opencode input to developing-today-forks/opencode               | developing-today[bot] | 2026-11-04 |
| [#2409](https://github.com/developing-today-forks/openchamber/pull/2409) | chore(flake): bump opencode to 0a8dbad                                                | dezren39              | 2026-11-18 |
| [#2507](https://github.com/developing-today-forks/openchamber/pull/2507) | chore(flake): pin opencode ce61597 (mcpx 6f4696b); bake the Linux node_modules hashes | dezren39              | 2026-11-23 |
| [#2514](https://github.com/developing-today-forks/openchamber/pull/2514) | chore(flake): pin opencode dfa989e (mcpx 2dc8cb8); bake #2513 node_modules hashes     | dezren39              | 2026-11-23 |

##### developing-today-forks/openchamber — theme: git (5)

| PR                                                                       | Title                                                                                   | Author   | Merged     |
| ------------------------------------------------------------------------ | --------------------------------------------------------------------------------------- | -------- | ---------- |
| [#56](https://github.com/developing-today-forks/openchamber/pull/56)     | test(git): set remote HEAD explicitly in option-like remote branch test                 | dezren39 | 2026-10-18 |
| [#195](https://github.com/developing-today-forks/openchamber/pull/195)   | fix(git): staging hunks failed for anyone with diff.mnemonicPrefix or diff.noprefix set | dezren39 | 2026-10-21 |
| [#469](https://github.com/developing-today-forks/openchamber/pull/469)   | fix(git): judge log maxCount and diff context strictly on every endpoint                | dezren39 | 2026-10-26 |
| [#579](https://github.com/developing-today-forks/openchamber/pull/579)   | fix(git): normalize contextLines in the git service diff functions (#578)               | dezren39 | 2026-10-27 |
| [#2434](https://github.com/developing-today-forks/openchamber/pull/2434) | test(git): make confirmed fetch timeout deterministic                                   | dezren39 | 2026-11-20 |

##### developing-today-forks/openchamber — theme: goal (5)

| PR                                                                       | Title                                                                                                              | Author                | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------ | --------------------- | ---------- |
| [#26](https://github.com/developing-today-forks/openchamber/pull/26)     | feat(goal): write goal specs with a model and start goals from the next message                                    | dezren39              | 2026-10-17 |
| [#150](https://github.com/developing-today-forks/openchamber/pull/150)   | feat(goal): background goal saves, message with the change, per-run writer model and effort, settable length limit | dezren39              | 2026-10-21 |
| [#205](https://github.com/developing-today-forks/openchamber/pull/205)   | fix(goal): a queued message keeps the composer's goal flags                                                        | dezren39              | 2026-10-22 |
| [#270](https://github.com/developing-today-forks/openchamber/pull/270)   | fix(goal): queued goals keep their flags through edit, retry and a forced queue; cap the inline objective again    | dezren39              | 2026-10-22 |
| [#1984](https://github.com/developing-today-forks/openchamber/pull/1984) | feat(goal): rewrite a single spec section instead of the whole document (#1829)                                    | developing-today[bot] | 2026-11-08 |

##### developing-today-forks/openchamber — theme: security (5)

| PR                                                                     | Title                                                                            | Author   | Merged     |
| ---------------------------------------------------------------------- | -------------------------------------------------------------------------------- | -------- | ---------- |
| [#544](https://github.com/developing-today-forks/openchamber/pull/544) | fix(security): bracket-aware Host parsing so the [::1] origin alias fires (#500) | dezren39 | 2026-10-27 |
| [#591](https://github.com/developing-today-forks/openchamber/pull/591) | fix(security): complete the loopback origin alias set (#589)                     | dezren39 | 2026-10-27 |
| [#595](https://github.com/developing-today-forks/openchamber/pull/595) | fix(security): add [::1] to the dev-mode loopback helpers (#594)                 | dezren39 | 2026-10-27 |
| [#625](https://github.com/developing-today-forks/openchamber/pull/625) | fix(security): validate the login rate-limit env vars (#624)                     | dezren39 | 2026-10-28 |
| [#630](https://github.com/developing-today-forks/openchamber/pull/630) | fix(security): pin branch-cleanup checkout to v7.0.1 SHA                         | dezren39 | 2026-10-28 |

##### developing-today-forks/openchamber — theme: release (5)

| PR                                                                     | Title                                                                                            | Author   | Merged     |
| ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ | -------- | ---------- |
| [#7](https://github.com/developing-today-forks/openchamber/pull/7)     | ci(release): make the macOS and Windows desktop builds manual opt-in                             | dezren39 | 2026-10-14 |
| [#9](https://github.com/developing-today-forks/openchamber/pull/9)     | fix(release): keep release uploads as drafts; prune other-arch llama addons from the Linux build | dezren39 | 2026-10-14 |
| [#11](https://github.com/developing-today-forks/openchamber/pull/11)   | fix(release): find the draft release when checking Linux assets                                  | dezren39 | 2026-10-14 |
| [#193](https://github.com/developing-today-forks/openchamber/pull/193) | fix(release): let finalize-latest-yml complete releases with no macOS build                      | dezren39 | 2026-10-21 |
| [#197](https://github.com/developing-today-forks/openchamber/pull/197) | fix(release): refuse a version bump with a bun older than the pinned one                         | dezren39 | 2026-10-21 |

##### developing-today-forks/openchamber — theme: scripts (4)

| PR                                                                       | Title                                                                                             | Author   | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#2334](https://github.com/developing-today-forks/openchamber/pull/2334) | fix(scripts): reject options a fixture command does not read (#2057)                              | dezren39 | 2026-11-17 |
| [#2379](https://github.com/developing-today-forks/openchamber/pull/2379) | chore(scripts): rename mobile workspace resize check to .manual.mjs and document it               | dezren39 | 2026-11-17 |
| [#2384](https://github.com/developing-today-forks/openchamber/pull/2384) | fix(scripts): retarget release build test from removed Tauri package to Electron                  | dezren39 | 2026-11-17 |
| [#2505](https://github.com/developing-today-forks/openchamber/pull/2505) | test(scripts): exit a node:test file once its tests finish, so a failure is not read as a timeout | dezren39 | 2026-11-23 |

##### developing-today-forks/openchamber — theme: tests (4)

| PR                                                                       | Title                                                                         | Author   | Merged     |
| ------------------------------------------------------------------------ | ----------------------------------------------------------------------------- | -------- | ---------- |
| [#1591](https://github.com/developing-today-forks/openchamber/pull/1591) | docs: add testing doctrine (PBT, MBT, oracles) to AGENTS.md                   | dezren39 | 2026-11-04 |
| [#2453](https://github.com/developing-today-forks/openchamber/pull/2453) | test: run real self-handoff under Node                                        | dezren39 | 2026-11-22 |
| [#2477](https://github.com/developing-today-forks/openchamber/pull/2477) | test: repair main's suites after today's merges                               | dezren39 | 2026-11-22 |
| [#2504](https://github.com/developing-today-forks/openchamber/pull/2504) | test: give the browser suites their own Vite port and chat-quote time to load | dezren39 | 2026-11-23 |

##### developing-today-forks/openchamber — theme: quota/plans (4)

| PR                                                                       | Title                                                                                    | Author   | Merged     |
| ------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------- | -------- | ---------- |
| [#474](https://github.com/developing-today-forks/openchamber/pull/474)   | Quota: per-credential and instance quota, disabled filter, and the openchamber quota CLI | dezren39 | 2026-10-26 |
| [#475](https://github.com/developing-today-forks/openchamber/pull/475)   | Quota: add the Open Design (AMR) provider on a managed session cookie                    | dezren39 | 2026-10-26 |
| [#1321](https://github.com/developing-today-forks/openchamber/pull/1321) | Quota: Command Code monthly plan window from included credits                            | dezren39 | 2026-11-02 |
| [#1633](https://github.com/developing-today-forks/openchamber/pull/1633) | quota: add mistral provider (key-validity class) (#1619)                                 | dezren39 | 2026-11-05 |

##### developing-today-forks/openchamber — theme: i18n (4)

| PR                                                                     | Title                                                                                | Author   | Merged     |
| ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------ | -------- | ---------- |
| [#112](https://github.com/developing-today-forks/openchamber/pull/112) | fix(i18n): French onboarding no longer directs Windows users to WSL                  | dezren39 | 2026-10-19 |
| [#320](https://github.com/developing-today-forks/openchamber/pull/320) | fix(i18n): translate the Arabic and Hebrew routing strings                           | dezren39 | 2026-10-23 |
| [#533](https://github.com/developing-today-forks/openchamber/pull/533) | fix(i18n): translate remaining routing chat strings in Arabic and Hebrew             | dezren39 | 2026-10-27 |
| [#549](https://github.com/developing-today-forks/openchamber/pull/549) | fix(i18n): plan tier description names the real tiers, tier labels translated (#326) | dezren39 | 2026-10-27 |

##### developing-today-forks/openchamber — theme: environment (4)

| PR                                                                     | Title                                                                                                    | Author   | Merged     |
| ---------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#167](https://github.com/developing-today-forks/openchamber/pull/167) | feat(environment): op:// vault references in environment variables, for OpenCode, MCP servers and agents | dezren39 | 2026-10-21 |
| [#245](https://github.com/developing-today-forks/openchamber/pull/245) | fix(environment): resolve vault references for the project environment command (#174)                    | dezren39 | 2026-10-22 |
| [#249](https://github.com/developing-today-forks/openchamber/pull/249) | fix(environment): a reload never joins a run started before the invalidation                             | dezren39 | 2026-10-22 |
| [#271](https://github.com/developing-today-forks/openchamber/pull/271) | fix(environment): tie vault read status to the reference it came from                                    | dezren39 | 2026-10-22 |

##### developing-today-forks/openchamber — theme: web,ui (3)

| PR                                                                       | Title                                                                                        | Author                | Merged     |
| ------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------- | --------------------- | ---------- |
| [#1025](https://github.com/developing-today-forks/openchamber/pull/1025) | fix(web,ui): bound relay fragment reassembler in aggregate + reject orphan fragments (#1024) | dezren39              | 2026-10-31 |
| [#2027](https://github.com/developing-today-forks/openchamber/pull/2027) | fix(web,ui): keep PWA app-name clamps off surrogate pairs (#2021)                            | developing-today[bot] | 2026-11-08 |
| [#2030](https://github.com/developing-today-forks/openchamber/pull/2030) | fix(web,ui): keep render-page title clamps off surrogate pairs (#2022)                       | dezren39              | 2026-11-08 |

##### developing-today-forks/openchamber — theme: sidebar (3)

| PR                                                                   | Title                                                                              | Author   | Merged     |
| -------------------------------------------------------------------- | ---------------------------------------------------------------------------------- | -------- | ---------- |
| [#20](https://github.com/developing-today-forks/openchamber/pull/20) | feat(sidebar): attention sections, movable sections and a held open session        | dezren39 | 2026-10-16 |
| [#44](https://github.com/developing-today-forks/openchamber/pull/44) | feat(sidebar): shortcut to open the next session needing attention                 | dezren39 | 2026-10-18 |
| [#71](https://github.com/developing-today-forks/openchamber/pull/71) | fix(sidebar): next-attention shortcut no longer skips sessions drawn project-first | dezren39 | 2026-10-18 |

##### developing-today-forks/openchamber — theme: browser (3)

| PR                                                                   | Title                                                                                   | Author   | Merged     |
| -------------------------------------------------------------------- | --------------------------------------------------------------------------------------- | -------- | ---------- |
| [#41](https://github.com/developing-today-forks/openchamber/pull/41) | fix(browser): refuse browser.type on non-text inputs (#32)                              | dezren39 | 2026-10-17 |
| [#42](https://github.com/developing-today-forks/openchamber/pull/42) | fix(browser): retry capturePage while Chromium has no frame yet                         | dezren39 | 2026-10-17 |
| [#70](https://github.com/developing-today-forks/openchamber/pull/70) | fix(browser): browser.type verifies the value landed instead of reporting false success | dezren39 | 2026-10-18 |

##### developing-today-forks/openchamber — theme: fs (2)

| PR                                                                       | Title                                                             | Author   | Merged     |
| ------------------------------------------------------------------------ | ----------------------------------------------------------------- | -------- | ---------- |
| [#932](https://github.com/developing-today-forks/openchamber/pull/932)   | fix(fs): kill the whole process tree on fs/exec cap and timeout   | dezren39 | 2026-10-30 |
| [#2536](https://github.com/developing-today-forks/openchamber/pull/2536) | fix(fs): contain reveal targets to the workspace or managed roots | dezren39 | 2026-11-23 |

##### developing-today-forks/openchamber — theme: upstream sync (2)

| PR                                                                       | Title                                                                                | Author   | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------ | -------- | ---------- |
| [#2502](https://github.com/developing-today-forks/openchamber/pull/2502) | Merge upstream openchamber main (v2.8.0) into the fork                               | dezren39 | 2026-11-22 |
| [#2510](https://github.com/developing-today-forks/openchamber/pull/2510) | Merge upstream openchamber main (3a0dbeafb4) and pin opencode a6c1e21 / mcpx 0c9827b | dezren39 | 2026-11-23 |

##### developing-today-forks/openchamber — theme: goal-writer (2)

| PR                                                                       | Title                                                                          | Author   | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------ | -------- | ---------- |
| [#2467](https://github.com/developing-today-forks/openchamber/pull/2467) | feat(goal-writer): send when ready, with the goal written in the background    | dezren39 | 2026-11-22 |
| [#2483](https://github.com/developing-today-forks/openchamber/pull/2483) | feat(goal-writer): the server owns "Send when ready" until the message is sent | dezren39 | 2026-11-22 |

##### developing-today-forks/openchamber — theme: sync (2)

| PR                                                                       | Title                                                    | Author   | Merged     |
| ------------------------------------------------------------------------ | -------------------------------------------------------- | -------- | ---------- |
| [#203](https://github.com/developing-today-forks/openchamber/pull/203)   | chore(sync): merge upstream main (218 commits) into main | dezren39 | 2026-10-21 |
| [#2465](https://github.com/developing-today-forks/openchamber/pull/2465) | fix(sync): clear running tasks that already ended        | dezren39 | 2026-11-22 |

##### developing-today-forks/openchamber — theme: cli (2)

| PR                                                                       | Title                                                          | Author   | Merged     |
| ------------------------------------------------------------------------ | -------------------------------------------------------------- | -------- | ---------- |
| [#447](https://github.com/developing-today-forks/openchamber/pull/447)   | fix(cli): judge --timeout whole at parse time (fixes #446)     | dezren39 | 2026-10-25 |
| [#2450](https://github.com/developing-today-forks/openchamber/pull/2450) | fix(cli): give session create the worktree provisioning budget | dezren39 | 2026-11-22 |

##### developing-today-forks/openchamber — theme: ui,web (2)

| PR                                                                       | Title                                                                     | Author   | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------- | -------- | ---------- |
| [#2118](https://github.com/developing-today-forks/openchamber/pull/2118) | fix(ui,web): truncate PWA shortcut names on grapheme boundaries           | dezren39 | 2026-11-11 |
| [#2148](https://github.com/developing-today-forks/openchamber/pull/2148) | fix(ui,web): make PWA app-name and fallback-manifest clamps grapheme-safe | dezren39 | 2026-11-12 |

##### developing-today-forks/openchamber — theme: model-choices (2)

| PR                                                                       | Title                                                                                                 | Author   | Merged     |
| ------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#505](https://github.com/developing-today-forks/openchamber/pull/505)   | feat(model-choices): learn from manual model and effort picks (#147)                                  | dezren39 | 2026-10-27 |
| [#2096](https://github.com/developing-today-forks/openchamber/pull/2096) | fix(model-choices): distinguish default effort from a literal default variant in suggestion decisions | dezren39 | 2026-11-11 |

##### developing-today-forks/openchamber — theme: routes/pools (2)

| PR                                                                       | Title                                                                           | Author   | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------- | -------- | ---------- |
| [#125](https://github.com/developing-today-forks/openchamber/pull/125)   | fix: route the remaining atomic-write stores through the rename retry helpers   | dezren39 | 2026-10-19 |
| [#2082](https://github.com/developing-today-forks/openchamber/pull/2082) | scheduled-tasks: durable per-run history + run-history read route (#2174 Gap 1) | dezren39 | 2026-11-11 |

##### developing-today-forks/openchamber — theme: plugins/mcp (2)

| PR                                                                       | Title                                                 | Author                | Merged     |
| ------------------------------------------------------------------------ | ----------------------------------------------------- | --------------------- | ---------- |
| [#1431](https://github.com/developing-today-forks/openchamber/pull/1431) | mcpx: pass the opencode fork's daemon modules through | dezren39              | 2026-11-03 |
| [#1488](https://github.com/developing-today-forks/openchamber/pull/1488) | chore: bump opencode (mcpx unit environment fix)      | developing-today[bot] | 2026-11-03 |

##### developing-today-forks/openchamber — theme: sessions (2)

| PR                                                                     | Title                                                                      | Author   | Merged     |
| ---------------------------------------------------------------------- | -------------------------------------------------------------------------- | -------- | ---------- |
| [#498](https://github.com/developing-today-forks/openchamber/pull/498) | fix(sessions): refuse a non-object subagentPolicy on session create (#477) | dezren39 | 2026-10-26 |
| [#723](https://github.com/developing-today-forks/openchamber/pull/723) | Session row lines and elements setting (#22)                               | dezren39 | 2026-10-28 |

##### developing-today-forks/openchamber — theme: notes (2)

| PR                                                                     | Title                                                                               | Author   | Merged     |
| ---------------------------------------------------------------------- | ----------------------------------------------------------------------------------- | -------- | ---------- |
| [#553](https://github.com/developing-today-forks/openchamber/pull/553) | fix(notes): format the whole note on first render                                   | dezren39 | 2026-10-27 |
| [#631](https://github.com/developing-today-forks/openchamber/pull/631) | test(notes): deterministic partial-tree regression test for NoteEditor first render | dezren39 | 2026-10-28 |

##### developing-today-forks/openchamber — theme: providers (2)

| PR                                                                     | Title                                                                  | Author   | Merged     |
| ---------------------------------------------------------------------- | ---------------------------------------------------------------------- | -------- | ---------- |
| [#588](https://github.com/developing-today-forks/openchamber/pull/588) | fix(providers): filter turned-off providers from every location picker | dezren39 | 2026-10-27 |
| [#617](https://github.com/developing-today-forks/openchamber/pull/617) | docs: add Custom / Other providers subsection to 9 mirror pages        | dezren39 | 2026-10-28 |

##### developing-today-forks/openchamber — theme: routes (2)

| PR                                                                     | Title                                                                               | Author   | Merged     |
| ---------------------------------------------------------------------- | ----------------------------------------------------------------------------------- | -------- | ---------- |
| [#558](https://github.com/developing-today-forks/openchamber/pull/558) | fix(routes): parse numeric query params strictly                                    | dezren39 | 2026-10-27 |
| [#598](https://github.com/developing-today-forks/openchamber/pull/598) | fix(routes): parse bindingRevision strictly in GitHub and GitLab read routes (#592) | dezren39 | 2026-10-27 |

##### developing-today-forks/openchamber — theme: routing-ui (2)

| PR                                                                     | Title                                                                      | Author   | Merged     |
| ---------------------------------------------------------------------- | -------------------------------------------------------------------------- | -------- | ---------- |
| [#562](https://github.com/developing-today-forks/openchamber/pull/562) | fix(routing-ui): preserve stored weekdays in schedule editor               | dezren39 | 2026-10-27 |
| [#596](https://github.com/developing-today-forks/openchamber/pull/596) | fix(routing-ui): read single-digit seconds in stored schedule times (#587) | dezren39 | 2026-10-27 |

##### developing-today-forks/openchamber — theme: proxy (2)

| PR                                                                     | Title                                                      | Author   | Merged     |
| ---------------------------------------------------------------------- | ---------------------------------------------------------- | -------- | ---------- |
| [#542](https://github.com/developing-today-forks/openchamber/pull/542) | fix(proxy): treat empty uppercase proxy variables as unset | dezren39 | 2026-10-27 |
| [#543](https://github.com/developing-today-forks/openchamber/pull/543) | fix(proxy): preserve repeated Set-Cookie headers           | dezren39 | 2026-10-27 |

##### developing-today-forks/openchamber — theme: dictation (2)

| PR                                                                     | Title                                                                             | Author   | Merged     |
| ---------------------------------------------------------------------- | --------------------------------------------------------------------------------- | -------- | ---------- |
| [#424](https://github.com/developing-today-forks/openchamber/pull/424) | fix(dictation): pin speech model archives, enforce size and checksum on download  | dezren39 | 2026-10-25 |
| [#429](https://github.com/developing-today-forks/openchamber/pull/429) | test(dictation): cover cached-archive pin re-verification in the model downloader | dezren39 | 2026-10-25 |

##### developing-today-forks/openchamber — theme: lifecycle (2)

| PR                                                                     | Title                                                                                          | Author   | Merged     |
| ---------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#206](https://github.com/developing-today-forks/openchamber/pull/206) | test(lifecycle): wait long enough for two real processes to start under load                   | dezren39 | 2026-10-22 |
| [#293](https://github.com/developing-today-forks/openchamber/pull/293) | fix(lifecycle): two OpenChamber servers on one data dir no longer retire each other's OpenCode | dezren39 | 2026-10-23 |

##### developing-today-forks/openchamber — theme: markdown (2)

| PR                                                                     | Title                                                                                               | Author   | Merged     |
| ---------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#196](https://github.com/developing-today-forks/openchamber/pull/196) | fix(markdown): long messages froze Safari and the iOS app for up to a minute                        | dezren39 | 2026-10-21 |
| [#273](https://github.com/developing-today-forks/openchamber/pull/273) | test(markdown): drop a timing test that cannot fail on CI's engine; correct which engines were slow | dezren39 | 2026-10-22 |

##### developing-today-forks/openchamber — theme: audit (1)

| PR                                                                       | Title                                                                          | Author   | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------ | -------- | ---------- |
| [#2561](https://github.com/developing-today-forks/openchamber/pull/2561) | ci(audit): fail caller jobs that under-grant the called workflow's permissions | dezren39 | 2026-11-23 |

##### developing-today-forks/openchamber — theme: client-auth (1)

| PR                                                                       | Title                                                       | Author   | Merged     |
| ------------------------------------------------------------------------ | ----------------------------------------------------------- | -------- | ---------- |
| [#2556](https://github.com/developing-today-forks/openchamber/pull/2556) | feat(client-auth): admit a programmatic pairing client kind | dezren39 | 2026-11-23 |

##### developing-today-forks/openchamber — theme: tts (1)

| PR                                                                       | Title                                                                     | Author   | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------- | -------- | ---------- |
| [#2550](https://github.com/developing-today-forks/openchamber/pull/2550) | fix(tts): per-request mkdtemp directory for say output, always cleaned up | dezren39 | 2026-11-23 |

##### developing-today-forks/openchamber — theme: models (1)

| PR                                                                       | Title                                                                                   | Author   | Merged     |
| ------------------------------------------------------------------------ | --------------------------------------------------------------------------------------- | -------- | ---------- |
| [#2486](https://github.com/developing-today-forks/openchamber/pull/2486) | feat(models): lock a model's default effort, edit OpenCode's model keys, own pool waits | dezren39 | 2026-11-22 |

##### developing-today-forks/openchamber — theme: event-stream (1)

| PR                                                                       | Title                                                                          | Author   | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------ | -------- | ---------- |
| [#2480](https://github.com/developing-today-forks/openchamber/pull/2480) | fix(event-stream): withhold OpenCode's greeting after a lossless cursor resume | dezren39 | 2026-11-22 |

##### developing-today-forks/openchamber — theme: session-work (1)

| PR                                                                       | Title                                                                                     | Author   | Merged     |
| ------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------- | -------- | ---------- |
| [#2471](https://github.com/developing-today-forks/openchamber/pull/2471) | feat(session-work): choose how sessions enter In work: off, every message, or Jev decides | dezren39 | 2026-11-22 |

##### developing-today-forks/openchamber — theme: perf (1)

| PR                                                                       | Title                                                            | Author   | Merged     |
| ------------------------------------------------------------------------ | ---------------------------------------------------------------- | -------- | ---------- |
| [#2345](https://github.com/developing-today-forks/openchamber/pull/2345) | fix(perf): verify seed replies belong to the current run (#2157) | dezren39 | 2026-11-17 |

##### developing-today-forks/openchamber — theme: oc-review (1)

| PR                                                                       | Title                                                                    | Author   | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------ | -------- | ---------- |
| [#2335](https://github.com/developing-today-forks/openchamber/pull/2335) | ci(oc-review): build the start-wrapper guard check in flake-eval (#2056) | dezren39 | 2026-11-17 |

##### developing-today-forks/openchamber — theme: opencode (1)

| PR                                                                       | Title                                                                              | Author   | Merged     |
| ------------------------------------------------------------------------ | ---------------------------------------------------------------------------------- | -------- | ---------- |
| [#2329](https://github.com/developing-today-forks/openchamber/pull/2329) | docs(opencode): state the v2 store dual-write reality in known gaps + store header | dezren39 | 2026-11-16 |

##### developing-today-forks/openchamber — theme: dependabot (1)

| PR                                                                       | Title                                                           | Author   | Merged     |
| ------------------------------------------------------------------------ | --------------------------------------------------------------- | -------- | ---------- |
| [#2327](https://github.com/developing-today-forks/openchamber/pull/2327) | ci(dependabot): route Dependabot through the bun ecosystem at / | dezren39 | 2026-11-16 |

##### developing-today-forks/openchamber — theme: repo (1)

| PR                                                                       | Title                                                                      | Author   | Merged     |
| ------------------------------------------------------------------------ | -------------------------------------------------------------------------- | -------- | ---------- |
| [#2296](https://github.com/developing-today-forks/openchamber/pull/2296) | fix(repo): remove unused exports so bun run dead-code passes on clean main | dezren39 | 2026-11-16 |

##### developing-today-forks/openchamber — theme: workflow (1)

| PR                                                                       | Title                                                      | Author   | Merged     |
| ------------------------------------------------------------------------ | ---------------------------------------------------------- | -------- | ---------- |
| [#2288](https://github.com/developing-today-forks/openchamber/pull/2288) | fix(workflow): handle opencode-update PR creation failures | dezren39 | 2026-11-16 |

##### developing-today-forks/openchamber — theme: notifications (1)

| PR                                                                       | Title                                                   | Author   | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------- | -------- | ---------- |
| [#2143](https://github.com/developing-today-forks/openchamber/pull/2143) | fix(notifications): require requested assistant message | dezren39 | 2026-11-12 |

##### developing-today-forks/openchamber — theme: web,vscode,ui (1)

| PR                                                                       | Title                                                                                  | Author                | Merged     |
| ------------------------------------------------------------------------ | -------------------------------------------------------------------------------------- | --------------------- | ---------- |
| [#1833](https://github.com/developing-today-forks/openchamber/pull/1833) | test(web,vscode,ui): quota provider manifest + cross-surface conformance tests (#1809) | developing-today[bot] | 2026-11-08 |

##### developing-today-forks/openchamber — theme: web,ui,sdk (1)

| PR                                                                       | Title                                                                                                           | Author   | Merged     |
| ------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#1499](https://github.com/developing-today-forks/openchamber/pull/1499) | refactor(web,ui,sdk): anti-slop phase 1 batch S — shape-in-names renames + object-parameter owner types (#1437) | dezren39 | 2026-11-03 |

##### developing-today-forks/openchamber — theme: project-context (1)

| PR                                                                       | Title                                                                     | Author   | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------- | -------- | ---------- |
| [#1462](https://github.com/developing-today-forks/openchamber/pull/1462) | feat(project-context): plan metrics from the linked session (#16 stage 1) | dezren39 | 2026-11-03 |

##### developing-today-forks/openchamber — theme: quota,web (1)

| PR                                                                       | Title                                                                                       | Author   | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#1460](https://github.com/developing-today-forks/openchamber/pull/1460) | refactor(quota,web): injected seams replace module mocks (anti-slop phase 3 batch A, #1439) | dezren39 | 2026-11-03 |

##### developing-today-forks/openchamber — theme: nix/packaging (1)

| PR                                                                       | Title                                                                | Author   | Merged     |
| ------------------------------------------------------------------------ | -------------------------------------------------------------------- | -------- | ---------- |
| [#1458](https://github.com/developing-today-forks/openchamber/pull/1458) | nix: point the R2 cache substituter at the renamed workers subdomain | dezren39 | 2026-11-03 |

##### developing-today-forks/openchamber — theme: workflow-audit (1)

| PR                                                                       | Title                                                  | Author   | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------ | -------- | ---------- |
| [#1452](https://github.com/developing-today-forks/openchamber/pull/1452) | ci(workflow-audit): pin GITHUB_TOKEN to contents: read | dezren39 | 2026-11-03 |

##### developing-today-forks/openchamber — theme: watcher (1)

| PR                                                                       | Title                                                               | Author   | Merged     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------- | -------- | ---------- |
| [#1349](https://github.com/developing-today-forks/openchamber/pull/1349) | ci(watcher): inline pm-dispatch POST, drop external action download | dezren39 | 2026-11-02 |

##### developing-today-forks/openchamber — theme: themes (1)

| PR                                                                     | Title                                                                                                                 | Author   | Merged     |
| ---------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#607](https://github.com/developing-today-forks/openchamber/pull/607) | test(themes): run the port-opencode-theme integration suite in the root test run with an explicit sibling skip (#139) | dezren39 | 2026-10-28 |

##### developing-today-forks/openchamber — theme: github-routes (1)

| PR                                                                     | Title                                                                                    | Author   | Merged     |
| ---------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- | -------- | ---------- |
| [#603](https://github.com/developing-today-forks/openchamber/pull/603) | refactor(github-routes): remove always-true canonical flag / dead legacy branches (#602) | dezren39 | 2026-10-28 |

##### developing-today-forks/openchamber — theme: context (1)

| PR                                                                     | Title                                                                        | Author   | Merged     |
| ---------------------------------------------------------------------- | ---------------------------------------------------------------------------- | -------- | ---------- |
| [#586](https://github.com/developing-today-forks/openchamber/pull/586) | fix(context): guard context-obligatory createdAt in the server reader (#584) | dezren39 | 2026-10-27 |

##### developing-today-forks/openchamber — theme: opencode-update (1)

| PR                                                                     | Title                                                                   | Author   | Merged     |
| ---------------------------------------------------------------------- | ----------------------------------------------------------------------- | -------- | ---------- |
| [#511](https://github.com/developing-today-forks/openchamber/pull/511) | ci(opencode-update): find the failure issue by server-side title search | dezren39 | 2026-10-26 |

##### developing-today-forks/openchamber — theme: quota-ui (1)

| PR                                                                     | Title                                                                | Author   | Merged     |
| ---------------------------------------------------------------------- | -------------------------------------------------------------------- | -------- | ---------- |
| [#497](https://github.com/developing-today-forks/openchamber/pull/497) | fix(quota-ui): make Open Design reachable from the Usage page (#480) | dezren39 | 2026-10-26 |

##### developing-today-forks/openchamber — theme: web-cli (1)

| PR                                                                     | Title                                                                                    | Author   | Merged     |
| ---------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- | -------- | ---------- |
| [#434](https://github.com/developing-today-forks/openchamber/pull/434) | fix(web-cli): reject malformed --lines and --limit values instead of reinterpreting them | dezren39 | 2026-10-25 |

##### developing-today-forks/openchamber — theme: deps-dev (1)

| PR                                                                     | Title                                                                 | Author          | Merged     |
| ---------------------------------------------------------------------- | --------------------------------------------------------------------- | --------------- | ---------- |
| [#401](https://github.com/developing-today-forks/openchamber/pull/401) | chore(deps-dev): bump happy-dom from 18.0.1 to 21.0.4 in /packages/ui | dependabot[bot] | 2026-10-28 |

##### developing-today-forks/openchamber — theme: dev-servers (1)

| PR                                                                     | Title                                                                                          | Author   | Merged     |
| ---------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#265](https://github.com/developing-today-forks/openchamber/pull/265) | fix(dev-servers): reject listener ports and pids with trailing junk instead of truncating them | dezren39 | 2026-10-22 |

##### developing-today-forks/openchamber — theme: github (1)

| PR                                                                     | Title                                                                            | Author   | Merged     |
| ---------------------------------------------------------------------- | -------------------------------------------------------------------------------- | -------- | ---------- |
| [#261](https://github.com/developing-today-forks/openchamber/pull/261) | test(github): cover the vault-add route (POST /auth/vault) and its client branch | dezren39 | 2026-10-22 |

##### developing-today-forks/openchamber — theme: source-control (1)

| PR                                                                     | Title                                                                           | Author   | Merged     |
| ---------------------------------------------------------------------- | ------------------------------------------------------------------------------- | -------- | ---------- |
| [#176](https://github.com/developing-today-forks/openchamber/pull/176) | feat(source-control): GitHub and GitLab accounts whose token stays in 1Password | dezren39 | 2026-10-21 |

##### developing-today-forks/openchamber — theme: stale (1)

| PR                                                                     | Title                                                                   | Author   | Merged     |
| ---------------------------------------------------------------------- | ----------------------------------------------------------------------- | -------- | ---------- |
| [#134](https://github.com/developing-today-forks/openchamber/pull/134) | ci(stale): add queue: max — the last non-cancelling workflow without it | dezren39 | 2026-10-20 |

##### developing-today-forks/openchamber — theme: sdk (1)

| PR                                                                   | Title                                                                | Author   | Merged     |
| -------------------------------------------------------------------- | -------------------------------------------------------------------- | -------- | ---------- |
| [#60](https://github.com/developing-today-forks/openchamber/pull/60) | fix(sdk): rebuild the stale browser-provider-stub panel bundle (#59) | dezren39 | 2026-10-18 |

##### developing-today-forks/openchamber — theme: contributing (1)

| PR                                                                   | Title                                                                                                                | Author   | Merged     |
| -------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#50](https://github.com/developing-today-forks/openchamber/pull/50) | docs(contributing): bare bun test over UI suites shows phantom failures — use the isolated runner or --isolate (#46) | dezren39 | 2026-10-18 |

##### developing-today-forks/openchamber — theme: messaging (1)

| PR                                                                   | Title                                                                                                     | Author   | Merged     |
| -------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- | -------- | ---------- |
| [#30](https://github.com/developing-today-forks/openchamber/pull/30) | feat(messaging): Messages surface, chat notices, toasts and Settings for OpenCode inter-session messaging | dezren39 | 2026-10-17 |

##### developing-today-forks/openchamber — theme: worktrees (1)

| PR                                                                   | Title                                                                                                  | Author   | Merged     |
| -------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ | -------- | ---------- |
| [#23](https://github.com/developing-today-forks/openchamber/pull/23) | feat(worktrees): choose where new worktrees start, branch off pull requests, and template branch names | dezren39 | 2026-10-16 |

##### developing-today-forks/openchamber — theme: update (1)

| PR                                                                   | Title                                                                     | Author   | Merged     |
| -------------------------------------------------------------------- | ------------------------------------------------------------------------- | -------- | ---------- |
| [#13](https://github.com/developing-today-forks/openchamber/pull/13) | fix(update): take every app's updates from the fork's releases, never npm | dezren39 | 2026-10-14 |

---

## 2. Goals & intentions (code repo + design records)

What the programme is trying to make true, each with its issue link. Landmark/closed items are marked (closed); open items appear again in the §3 map.

- [Attach, don't SSH: research which is more efficient for OpenCode work — SSH-on-laptop vs attaching to the laptop's running server](https://github.com/developing-today/code/issues/1344) (closed).
- [Attach-first OpenCode client helper — forward lifecycle, service discovery, stdin-safe run wrapper (the #1344 follow-through)](https://github.com/developing-today/code/issues/1357) (closed).
- [Evaluate a tailnet-bound `opencode serve` option on the laptop for attach clients (the #1344/#1357 family)](https://github.com/developing-today/code/issues/1363) (closed).
- **A claimant owns dispatch.** The OpenCode claimant/executor (code#3026, slice 1) should discover unbound opencode-lane queue tasks, create/verify/bind sessions on the fork server, deliver prompts, and recover stuck sessions — under the session contract of code#4912 (phases #4757/#4870/#4758, closing audit #4916) and the seam contract of code#7770 (phases #7325/#7375/#7706, #7367/#7760, #7356/#7366/#7732; audit #7772). Those numbers are cited as they appear in the claimant README itself (the programme's own record); #1344/#1357/#1363 above were additionally verified individually against the API in this survey.
- **OpenCode 2 migration of the whole fleet** — [opencode#374](https://github.com/developing-today-forks/opencode/issues/374) (priority/high), its core-migration tracking home [opencode#2295](https://github.com/developing-today-forks/opencode/issues/2295), plugin follow-through [opencode#2290](https://github.com/developing-today-forks/opencode/issues/2290), the embedded-OpenCode upgrade on the OpenChamber side [openchamber#384](https://github.com/developing-today-forks/openchamber/issues/384), and the deployed-plugin gap list at `doc/2026-11-23T03-45-00Z_opencode-v2-deployed-plugin-gap-list/` in the code repo (three v1 constructs with no v2 equivalent, each with a chosen workaround).
- **Routes, pools and quota as fork-native machinery** — model-route follow-ups [opencode#2681](https://github.com/developing-today-forks/opencode/issues/2681), the config-projection gap [opencode#2782](https://github.com/developing-today-forks/opencode/issues/2782) (“needed by code#13675 Part 2”), OpenChamber's pool-settings follow-ups [openchamber#2498](https://github.com/developing-today-forks/openchamber/issues/2498), retry-recovery controls [openchamber#2126](https://github.com/developing-today-forks/openchamber/issues/2126), and the provider doctrine [opencode#1717](https://github.com/developing-today-forks/opencode/issues/1717) (“*-direct providers EMULATE the first-party wire”).
- **Server façades and consolidation** — generic OpenAI façades on the fork server [opencode#1146](https://github.com/developing-today-forks/opencode/issues/1146) (priority/high; “proxy doors retire”), OpenChamber server-control consolidation + quota credential [opencode#2491](https://github.com/developing-today-forks/opencode/issues/2491) / [opencode#2490](https://github.com/developing-today-forks/opencode/issues/2490) (both citing the code#5060 ruling).
- **Console work** — the PM console (code repo `pm-stack/console`, §4 context) consumes the forks' server/session surface; on the fork itself the console scope is one of the largest PR themes in §1.5, and OpenChamber is the operator console for sessions (its epic: [openchamber#2153](https://github.com/developing-today-forks/openchamber/issues/2153); session dispatch + babysitting epic in the code title set, `code-title-openchamber.json` in the deposit).
- **Observability without instrumentation** — usage export by pull over the fork's own DB (code#2072 slice 1, §4.3); slices 2–7 are stated follow-ups, i.e. intentions, not shipped work.

---

## 3. Prioritised issue maps

**Ordering rule (stated, as required).** Both fork repos _do_ carry explicit priority labels (`priority/high`, `priority/normal`, `priority/low`) on most open issues, and the code repo uses the same family on part of its set. Order is therefore: (1) `priority/high` → `priority/normal` → `priority/low` → no priority label; (2) within a tier, `user/escalated` first; (3) then `status/blocked`/`queue/blocked`; (4) then oldest first. Milestones: **none** of the open issues on either fork, and none in the code set below, carries a milestone (the API `milestone` field is null throughout) — milestones play no part in the ordering. Cross-references from code issues are noted where the title/body itself states them (e.g. opencode#2782). Age = whole days from creation to 2026-11-24.

### 3.1 developing-today-forks/opencode — every open issue (30)

| #                                                                      | Issue                                                                                                                                        | Labels                                                                                                                                                                                         | Age (d) | Created    |
| ---------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------- | ---------- |
| [374](https://github.com/developing-today-forks/opencode/issues/374)   | Migrate the fleet to OpenCode 2                                                                                                              | kind/feature, source/creator, priority/high, effort/l, meta/enriched, meta/epic, epic/opencode-platform, design/opencode-2-migration                                                           | 30      | 2026-10-24 |
| [1146](https://github.com/developing-today-forks/opencode/issues/1146) | Design: generic OpenAI façades on the server — /v1/chat/completions + /v1/responses, one translation layer, proxy doors retire               | task/triage, user/review, source/creator, priority/high, effort/l, meta/enriched, meta/protocol-gap, kind/enhancement, design                                                                  | 22      | 2026-11-01 |
| [2785](https://github.com/developing-today-forks/opencode/issues/2785) | CI run silence: developing-today-forks/opencode — no completed Actions run within threshold                                                  | kind/bug, priority/high, source/ci-watch                                                                                                                                                       | 0       | 2026-11-23 |
| [2502](https://github.com/developing-today-forks/opencode/issues/2502) | fix(server): bound experimental fs.write body — unbounded raw payload plus unconfined absolute-path writes                                   | kind/security, status/needs-triage, status/blocked, priority/normal, app/server, user/escalated                                                                                                | 7       | 2026-11-16 |
| [1708](https://github.com/developing-today-forks/opencode/issues/1708) | Tool-call support across custom providers — audit + tracking (2026-11-05)                                                                    | task/small-fix, status/blocked, source/queue-worker, priority/normal, effort/m, queue/blocked, meta/enriched, kind/enhancement, epic/opencode-platform                                         | 18      | 2026-11-05 |
| [1680](https://github.com/developing-today-forks/opencode/issues/1680) | Design: custom + live audio through the façade — custom formats and realtime bidirectional (duplex) audio                                    | task/triage, user/review, source/creator, priority/normal, effort/m, meta/enriched, kind/enhancement, design                                                                                   | 19      | 2026-11-04 |
| [1684](https://github.com/developing-today-forks/opencode/issues/1684) | design(ai): translate the constraint, or say so — one fidelity contract for provider tool-schema projections                                 | task/triage, status/parked, priority/normal, queue/parked, app/ai, work/assigned, effort/m, meta/enriched, kind/design, source/design-architect, design                                        | 18      | 2026-11-05 |
| [1707](https://github.com/developing-today-forks/opencode/issues/1707) | Tool-call hardening — small items from the 2026-11-05 audit (checklist)                                                                      | kind/bug, task/triage, source/human, priority/normal, effort/m, meta/enriched, epic/opencode-platform                                                                                          | 18      | 2026-11-05 |
| [1717](https://github.com/developing-today-forks/opencode/issues/1717) | Doctrine: *-direct providers EMULATE the first-party wire — vendor binaries for login/bootstrap only, auth must auto-renew                   | task/triage, source/human, priority/normal, effort/l, meta/enriched, kind/design, design, epic/design-register, epic/opencode-platform, design/opencode-2-migration                            | 18      | 2026-11-05 |
| [1876](https://github.com/developing-today-forks/opencode/issues/1876) | Wire-identity conformance program: per-provider User-Agent verification, binary drift watch, and tool-call probes                            | priority/normal, effort/l, meta/enriched, kind/infrastructure, epic/opencode-platform                                                                                                          | 17      | 2026-11-06 |
| [2290](https://github.com/developing-today-forks/opencode/issues/2290) | OpenCode 2 migration follow-through: port the v1 plugins (mcpx bridge, mcpx-session, subtask2) and rebase the fork                           | kind/feature, task/triage, source/human, priority/normal, meta/follow-up, app/plugin, effort/l, mode/online, design, epic/design-register, epic/opencode-platform, design/opencode-2-migration | 12      | 2026-11-12 |
| [2490](https://github.com/developing-today-forks/opencode/issues/2490) | quota credential: provider-only session-cookie credential in the server credential store (code#5060 fix)                                     | priority/normal                                                                                                                                                                                | 7       | 2026-11-16 |
| [2491](https://github.com/developing-today-forks/opencode/issues/2491) | consolidation: merge OpenChamber's server-side control into OpenCode (Drew ruling, code#5060)                                                | priority/normal                                                                                                                                                                                | 7       | 2026-11-16 |
| [2492](https://github.com/developing-today-forks/opencode/issues/2492) | spike: evaluate TursoDB (Rust SQLite rewrite, not libsql) as the database engine behind the sqlite seam                                      | priority/normal                                                                                                                                                                                | 7       | 2026-11-16 |
| [2638](https://github.com/developing-today-forks/opencode/issues/2638) | main CI failure: developing-today-forks/opencode — agents workflow (run 44247391571)                                                         | kind/ci, task/small-fix, status/in-progress, priority/normal, queue/claimed, source/ci-watch                                                                                                   | 5       | 2026-11-18 |
| [2652](https://github.com/developing-today-forks/opencode/issues/2652) | feat(app): restore custom provider setup in V2                                                                                               | task/triage, status/needs-triage, source/creator, priority/normal, queue/queued, app/app, kind/enhancement, epic/opencode-platform                                                             | 4       | 2026-11-19 |
| [2775](https://github.com/developing-today-forks/opencode/issues/2775) | fix(core): surface ambiguous outcomes when command recovery may replay side effects                                                          | kind/bug, task/triage, source/creator, priority/normal, app/core, effort/m                                                                                                                     | 0       | 2026-11-23 |
| [2778](https://github.com/developing-today-forks/opencode/issues/2778) | fix(tui): an unparseable model.json is overwritten with defaults on the next preference update — favorites and recent models silently erased | kind/bug, task/triage, source/creator, priority/normal, app/tui, effort/s                                                                                                                      | 0       | 2026-11-23 |
| [2786](https://github.com/developing-today-forks/opencode/issues/2786) | fix(files): public-file cache hits ignore If-None-Match — a revalidating client gets 200 and the full body instead of 304                    | kind/bug, task/triage, source/creator, priority/normal                                                                                                                                         | 0       | 2026-11-23 |
| [1677](https://github.com/developing-today-forks/opencode/issues/1677) | Design: search providers behind the façade — one common search endpoint vs per-provider adapters per tool                                    | task/triage, user/review, source/creator, priority/low, effort/m, meta/enriched, kind/enhancement, design                                                                                      | 19      | 2026-11-04 |
| [2302](https://github.com/developing-today-forks/opencode/issues/2302) | feat(tui): highlight embedded JavaScript and CSS in HTML                                                                                     | kind/feature, task/small-fix, source/creator, priority/low, app/tui, effort/xs                                                                                                                 | 11      | 2026-11-12 |
| [2505](https://github.com/developing-today-forks/opencode/issues/2505) | test(core): no CI coverage runs the TursoDB backend — PR #2499 adds zero tests and OPENCODE_DB_ENGINE=turso is never exercised before merge  | kind/test, task/triage, source/pr-diff-review, priority/low, app/core, work/assigned, status/done                                                                                              | 7       | 2026-11-16 |
| [2295](https://github.com/developing-today-forks/opencode/issues/2295) | OpenCode 2 fleet migration — core migration (shipped — tracking home)                                                                        | design, epic/design-register, epic/opencode-platform, design/opencode-2-migration                                                                                                              | 11      | 2026-11-12 |
| [2319](https://github.com/developing-today-forks/opencode/issues/2319) | epic: opencode fork — platform work                                                                                                          | epic, epic/opencode-platform                                                                                                                                                                   | 11      | 2026-11-12 |
| [2518](https://github.com/developing-today-forks/opencode/issues/2518) | main CI failure: developing-today-forks/opencode — test workflow (run 43849921062)                                                           | kind/ci, status/parked, status/awaiting-ci, queue/parked, work/assigned, source/ci-watch                                                                                                       | 7       | 2026-11-16 |
| [2677](https://github.com/developing-today-forks/opencode/issues/2677) | Follow-ups from the 2026-11-22 upstream sync                                                                                                 | —                                                                                                                                                                                              | 1       | 2026-11-22 |
| [2681](https://github.com/developing-today-forks/opencode/issues/2681) | Model routes: follow-ups after #2674, #2676 and #2679                                                                                        | —                                                                                                                                                                                              | 1       | 2026-11-22 |
| [2770](https://github.com/developing-today-forks/opencode/issues/2770) | cli-rs: regenerated types require sections/models that leaner wire payloads omit — decide the tolerance policy                               | —                                                                                                                                                                                              | 0       | 2026-11-23 |
| [2782](https://github.com/developing-today-forks/opencode/issues/2782) | config projection strips model_routes "purpose" and top-level model_designations (needed by code#13675 Part 2)                               | —                                                                                                                                                                                              | 0       | 2026-11-23 |
| [2787](https://github.com/developing-today-forks/opencode/issues/2787) | fix(tui): truncated session.json is overwritten with in-memory pins on the next pin change — pinned sessions silently erased                 | —                                                                                                                                                                                              | 0       | 2026-11-23 |

### 3.2 developing-today-forks/openchamber — every open issue (30)

| #                                                                         | Issue                                                                                                                                     | Labels                                                                                                                                            | Age (d) | Created    |
| ------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- | ------- | ---------- |
| [25](https://github.com/developing-today-forks/openchamber/issues/25)     | Handoff: release and live-validate the openchamber fork's work since v2.2.0 as an orchestrator using subagents                            | needs-validation, source/human, priority/high, user/escalated, effort/l, meta/enriched, kind/infrastructure, epic/opencode-platform               | 38      | 2026-10-16 |
| [2274](https://github.com/developing-today-forks/openchamber/issues/2274) | ci: GitHub Actions is disabled for this repo again — no workflow has run since 2026-11-11 (recurrence of #54)                             | kind/ci, source/creator, priority/high, queue/queued, area/infra, user/escalated, effort/s                                                        | 10      | 2026-11-13 |
| [1642](https://github.com/developing-today-forks/openchamber/issues/1642) | main CI failure: .github/workflows/agents.yml (run 41997940196)                                                                           | kind/bug, task/small-fix, status/blocked, priority/high, effort/s, queue/blocked, meta/enriched, source/ci-watch                                  | 18      | 2026-11-05 |
| [1450](https://github.com/developing-today-forks/openchamber/issues/1450) | main CI failure: .github/workflows/jules-daily-upstream-sync.yml (run 41633212191)                                                        | kind/bug, task/small-fix, priority/high, meta/enriched, source/ci-watch, status/triaged                                                           | 20      | 2026-11-03 |
| [2564](https://github.com/developing-today-forks/openchamber/issues/2564) | fix(web): preview-grant serve route serves symlink-escaped files outside the grant root                                                   | kind/bug, task/triage, status/in-progress, source/creator, priority/high, queue/claimed, app/web, work/assigned, effort/xs                        | 0       | 2026-11-23 |
| [2581](https://github.com/developing-today-forks/openchamber/issues/2581) | fix(electron): cancelling the macOS quit confirmation leaves quitRequested set, so the next window close destroys instead of hiding       | kind/bug, task/triage, source/creator, priority/high, app/electron, effort/s                                                                      | 0       | 2026-11-23 |
| [2583](https://github.com/developing-today-forks/openchamber/issues/2583) | fix(web): /api/fs/exec has no bound on commands per request or retained jobs — unbounded server memory for 30 minutes                     | kind/bug, task/triage, source/creator, priority/high, app/web, effort/s                                                                           | 0       | 2026-11-23 |
| [2585](https://github.com/developing-today-forks/openchamber/issues/2585) | fix(web): repeated message-search role/lifecycle params silently widen to an unscoped search                                              | kind/bug, task/triage, source/creator, priority/high, app/web, effort/s                                                                           | 0       | 2026-11-23 |
| [21](https://github.com/developing-today-forks/openchamber/issues/21)     | Sidebar view mode without headers that keeps the section order                                                                            | kind/feature, source/human, priority/normal, app/ui, effort/m, meta/enriched, epic/opencode-platform                                              | 38      | 2026-10-16 |
| [81](https://github.com/developing-today-forks/openchamber/issues/81)     | Upgrade without dropped sessions: supervisor that owns the listener and the managed OpenCode children                                     | kind/feature, status/parked, source/human, priority/normal, queue/parked, effort/l, meta/enriched, epic/opencode-platform                         | 36      | 2026-10-18 |
| [322](https://github.com/developing-today-forks/openchamber/issues/322)   | Goal writer card says "The message was not sent." above reasons that say it may have been sent                                            | kind/bug, task/triage, status/parked, source/human, priority/normal, queue/parked, effort/s, meta/enriched                                        | 32      | 2026-10-23 |
| [384](https://github.com/developing-today-forks/openchamber/issues/384)   | Update the embedded OpenCode to a v2-based build (fleet OpenCode 2 migration)                                                             | kind/feature, task/triage, source/creator, priority/normal, effort/l, meta/enriched, epic/opencode-platform, design/opencode-2-migration          | 30      | 2026-10-24 |
| [937](https://github.com/developing-today-forks/openchamber/issues/937)   | stale-pr: resolve openchamber#216 — feat: goal writer card, model pool checks, Answer again on questions, project visibility and UI fixes | task/triage, status/parked, user/review, source/creator, priority/normal, queue/parked, work/assigned, effort/s, kind/question                    | 24      | 2026-10-30 |
| [1573](https://github.com/developing-today-forks/openchamber/issues/1573) | Split 216b of PR #216: land goal spec jobs (draft-review-send pipeline) against current main                                              | kind/feature, status/parked, priority/normal, queue/parked, app/web, work/assigned, effort/l, meta/enriched, source/agent, epic/opencode-platform | 19      | 2026-11-04 |
| [2290](https://github.com/developing-today-forks/openchamber/issues/2290) | Paired clients have no route scopes and no programmatic client kind (babysitter enrollment gap, code#1271)                                | kind/feature, priority/normal, app/web, effort/l                                                                                                  | 8       | 2026-11-15 |
| [2572](https://github.com/developing-today-forks/openchamber/issues/2572) | fix(web): preserve a concurrently created empty directory in /api/fs/rename                                                               | kind/bug, task/triage, source/creator, priority/normal, app/web, effort/xs                                                                        | 0       | 2026-11-23 |
| [80](https://github.com/developing-today-forks/openchamber/issues/80)     | Anti-slop lint backlog: 9,498 findings in 1,086 files, tracked by phase                                                                   | kind/debt, task/triage, source/human, priority/low, effort/l, meta/enriched, meta/epic                                                            | 36      | 2026-10-18 |
| [147](https://github.com/developing-today-forks/openchamber/issues/147)   | Learn from manual model and effort choices: notes for the classifier and Settings suggestions                                             | kind/feature, task/small-fix, source/human, priority/low, app/web, effort/l, meta/enriched                                                        | 34      | 2026-10-20 |
| [224](https://github.com/developing-today-forks/openchamber/issues/224)   | Back to edit on a new chat can replace the session the user moved to with a new draft                                                     | kind/bug, task/triage, status/parked, source/human, priority/low, queue/parked, effort/s, meta/enriched                                           | 33      | 2026-10-21 |
| [1437](https://github.com/developing-today-forks/openchamber/issues/1437) | Anti-slop phase 1: ~499 mechanical findings (separate statements, renames, named owner types)                                             | kind/debt, task/triage, source/queue-worker, priority/low, effort/m, meta/enriched                                                                | 20      | 2026-11-03 |
| [1438](https://github.com/developing-today-forks/openchamber/issues/1438) | Anti-slop phase 2: ~1,799 owner-type findings (unknowns, dictionaries, chained assertions, widening)                                      | kind/debt, source/queue-worker, priority/low, effort/l, meta/enriched                                                                             | 20      | 2026-11-03 |
| [1439](https://github.com/developing-today-forks/openchamber/issues/1439) | Anti-slop phase 3: ~7,091 design-pass findings (runtime typeof, safety comments, module mocking)                                          | kind/debt, status/parked, source/queue-worker, priority/low, queue/parked, work/assigned, effort/l, meta/enriched                                 | 20      | 2026-11-03 |
| [2081](https://github.com/developing-today-forks/openchamber/issues/2081) | One-queue lane designations for OpenChamber scheduled tasks (Drew 2026-11-10, code#8097)                                                  | status/done, queue/done                                                                                                                           | 12      | 2026-11-11 |
| [2126](https://github.com/developing-today-forks/openchamber/issues/2126) | OpenChamber retry-recovery controls — restart-with-new-model + force-next-pool-option (shipped — tracking home)                           | design, epic/design-register, epic/opencode-platform                                                                                              | 11      | 2026-11-12 |
| [2153](https://github.com/developing-today-forks/openchamber/issues/2153) | Epic: OpenCode platform                                                                                                                   | epic, epic/opencode-platform                                                                                                                      | 11      | 2026-11-12 |
| [2325](https://github.com/developing-today-forks/openchamber/issues/2325) | main CI failure: developing-today-forks/openchamber — Dependabot Updates workflow (run 43909247394)                                       | —                                                                                                                                                 | 7       | 2026-11-16 |
| [2498](https://github.com/developing-today-forks/openchamber/issues/2498) | Model pool settings: follow-ups after the rebuild                                                                                         | —                                                                                                                                                 | 1       | 2026-11-22 |
| [2573](https://github.com/developing-today-forks/openchamber/issues/2573) | Add atomic no-replace directory renames across supported platforms                                                                        | status/parked, queue/parked, work/assigned                                                                                                        | 0       | 2026-11-23 |
| [2580](https://github.com/developing-today-forks/openchamber/issues/2580) | feat(agent-tool): allow agents to write project setup commands and project actions (decision on #152)                                     | —                                                                                                                                                 | 0       | 2026-11-23 |
| [2586](https://github.com/developing-today-forks/openchamber/issues/2586) | main-broken: developing-today-forks/openchamber stale workflow (run 45147203731) — Label team pull requests step missing github-token     | —                                                                                                                                                 | 0       | 2026-11-23 |

### 3.3 developing-today/code — programme issues, open (91)

Scope rule for this table: open code issues whose **title** names opencode, openchamber or the (opencode) claimant — the union of the three title searches in the deposit. This is deliberately narrower than “mentions opencode anywhere” (480 open code issues do that); those 480, and all closed title-matched items, are in the deposit for the book's full-record landing.

| #                                                              | Issue                                                                                                                                                                                                                      | Labels                                                                                                                                                                                                                 | Age (d) | Created    |
| -------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------- | ---------- |
| [1862](https://github.com/developing-today/code/issues/1862)   | Files under $TMPDIR/opencode are being deleted (9 repos lost HEAD, config and loose refs)                                                                                                                                  | kind/bug, source/human, priority/high, user/escalated, effort/m, meta/enriched, epic/opencode-platform                                                                                                                 | 26      | 2026-10-28 |
| [1658](https://github.com/developing-today/code/issues/1658)   | rollout(fleet): OpenCode-half pilot on the code queue + extension to opencode/openchamber/nix (OpenCode-half step 5)                                                                                                       | kind/feature, task/triage, status/blocked, source/human, priority/high, queue/queued, meta/follow-up, area/infra, queue/blocked, effort/l, meta/enriched, status/triaged, design/opencode-half, epic/opencode-platform | 26      | 2026-10-28 |
| [5060](https://github.com/developing-today/code/issues/5060)   | quota(opendesign): billing quota ask cannot terminate at the OpenCode server — session-cookie credential the server cannot hold or use                                                                                     | task/triage, status/blocked, priority/high, area/infra, queue/blocked, effort/m, meta/enriched, kind/design, epic/opencode-platform                                                                                    | 20      | 2026-11-03 |
| [1187](https://github.com/developing-today/code/issues/1187)   | OpenChamber dispatch v1.1: admission control, hand-backs, stall nudges, harvesting, Workhorse-first routing                                                                                                                | status:in-review, kind/feature, source/creator, priority/high, queue/queued, area/infra, effort/l, mode/online, status/triaged, epic/opencode-platform                                                                 | 27      | 2026-10-27 |
| [1234](https://github.com/developing-today/code/issues/1234)   | epic(openchamber): session dispatch + babysitting — OpenChamber as the fleet's workhorse executor                                                                                                                          | status:in-review, source/creator, priority/high, queue/queued, area/infra, epic, effort/l, meta/enriched, mode/online, status/triaged, epic/opencode-platform                                                          | 27      | 2026-10-27 |
| [2174](https://github.com/developing-today/code/issues/2174)   | OpenCode scheduled tasks: migrate the Muse→OpenCode cron wrappers into OpenChamber scheduled tasks (waves, monitoring gates)                                                                                               | task/triage, status/parked, source/human, priority/high, queue/parked, work/assigned, effort/l, meta/enriched, kind/infrastructure, status/triaged, design/executor-expansion, epic/opencode-platform                  | 25      | 2026-10-29 |
| [10389](https://github.com/developing-today/code/issues/10389) | Design mill: one fleet surface for OpenCode, OpenChamber, ID, and mcpx                                                                                                                                                     | priority/high, effort/l, status/triaged, epic/design-register, primary-parent/design-register, stray-work                                                                                                              | 9       | 2026-11-14 |
| [10639](https://github.com/developing-today/code/issues/10639) | opencode handover not in effect: socket unit disabled, same-build switch darkened port ~16s and cut drain at 40ms (2026-11-16)                                                                                             | status:in-review, task/small-fix, status/parked, priority/high, queue/parked, work/assigned, effort/l, kind/infrastructure                                                                                             | 7       | 2026-11-16 |
| [1362](https://github.com/developing-today/code/issues/1362)   | chore(fleet): align opencode build revs across laptop service, laptop CLI, sandbox CLI                                                                                                                                     | status/blocked, priority/normal, meta/follow-up, queue/blocked, effort/s, meta/enriched, kind/infrastructure, mode/online, epic/opencode-platform                                                                      | 27      | 2026-10-27 |
| [1377](https://github.com/developing-today/code/issues/1377)   | chore(fleet): scope tailnet policy for the laptop opencode serve endpoint (amd-6:443)                                                                                                                                      | task/triage, status/blocked, source/creator, priority/normal, meta/follow-up, queue/blocked, work/assigned, effort/s, meta/enriched, kind/infrastructure, mode/online, epic/opencode-platform                          | 27      | 2026-10-27 |
| [1657](https://github.com/developing-today/code/issues/1657)   | feat(fleet): daily lane-share measurement, publishing + below-floor alert (OpenCode-half step 4)                                                                                                                           | kind/feature, task/triage, status/blocked, source/human, priority/normal, meta/follow-up, area/infra, queue/blocked, work/assigned, effort/m, meta/enriched, design/opencode-half, epic/opencode-platform              | 26      | 2026-10-28 |
| [2841](https://github.com/developing-today/code/issues/2841)   | mcpx opencode v2 plugin: implement the #1858 SET-REMAINDER items (1)-(5)                                                                                                                                                   | task/triage, status/blocked, source/queue-worker, priority/normal, app/mcpx, queue/blocked, effort/l, meta/enriched, kind/enhancement, epic/mcpx-platform                                                              | 24      | 2026-10-30 |
| [9097](https://github.com/developing-today/code/issues/9097)   | impl(atlas): continuous ingestion — queue/PM/GitHub/Muse/OpenCode/ID corpora + sources inventory                                                                                                                           | kind/feature, task/small-fix, status/blocked, priority/normal, queue/blocked, work/assigned, effort/l, status/triaged, epic/atlas-program                                                                              | 11      | 2026-11-12 |
| [730](https://github.com/developing-today/code/issues/730)     | OpenCode 2: validate .opencode config against the v2 schema and port the v1 plugins                                                                                                                                        | status:in-review, kind/feature, priority/normal, effort/l, meta/enriched, status/triaged, epic/opencode-platform                                                                                                       | 30      | 2026-10-24 |
| [1940](https://github.com/developing-today/code/issues/1940)   | mcpx and opencode code mode: nested code mode, ?codemode=false, and what a harness expects of mcpx_exec                                                                                                                    | enhancement, task/triage, source/human, priority/normal, app/mcpx, effort/m, meta/enriched, kind/enhancement, status/triaged, epic/mcpx-platform                                                                       | 26      | 2026-10-28 |
| [3286](https://github.com/developing-today/code/issues/3286)   | OpenCode 2 (#730 WS2 remainder): plugin-array v2 verdict pass + subtask2 decision                                                                                                                                          | kind/feature, task/triage, source/creator, priority/normal, effort/l, meta/enriched, epic/opencode-platform                                                                                                            | 22      | 2026-11-01 |
| [6533](https://github.com/developing-today/code/issues/6533)   | opencode-claimant: handed-off tasks are never launched — executed_lane/session_id stay null across repeated claim→handed_off→park cycles                                                                                   | kind/bug, task/triage, status/parked, priority/normal, queue/parked, effort/m, meta/enriched, app/fleet, epic/opencode-platform                                                                                        | 17      | 2026-11-06 |
| [9094](https://github.com/developing-today/code/issues/9094)   | research(atlas): OpenCode DB backup economics — copy vs zstd vs restic-delta cost + growth model                                                                                                                           | task/small-fix, status/parked, priority/normal, queue/parked, work/assigned, effort/l, kind/design, epic/atlas-program                                                                                                 | 11      | 2026-11-12 |
| [1241](https://github.com/developing-today/code/issues/1241)   | T3 Code provider warnings: close the model-list/usage gap (opencode models empty via API; antigravity account unchecked)                                                                                                   | kind/bug, task/triage, status/blocked, source/creator, priority/low, queue/blocked, work/assigned, effort/s, meta/enriched, epic/opencode-platform                                                                     | 27      | 2026-10-27 |
| [9551](https://github.com/developing-today/code/issues/9551)   | OpenClaw picker: pools listed plainly (no "(OpenCode Route pool)" suffix, no grouping), add Small pool, hide Copilot + Claude CLI                                                                                          | task/small-fix, status/blocked, user/escalated, queue/blocked, work/assigned                                                                                                                                           | 10      | 2026-11-13 |
| [1409](https://github.com/developing-today/code/issues/1409)   | pool flag: add Kimi K3.1 Pro via opencode-go to summit — AA 55, plan door found (models.dev Go catalog + 53/57 measured probe successes), $2.11/task                                                                       | kind/feature, task/triage, status/blocked, source/creator, area/infra, queue/blocked, meta/enriched, epic/opencode-platform                                                                                            | 27      | 2026-10-27 |
| [6254](https://github.com/developing-today/code/issues/6254)   | pool flag: add Tencent Hy4 to the Flash pool family via opencode-go — AA 34.6 vs Flash #high 28.6 (+6.0), Go catalog door $0.834/$2.501, AA 93.5 t/s                                                                       | kind/feature, status/blocked, area/infra, queue/blocked, work/assigned, meta/enriched, epic/opencode-platform                                                                                                          | 18      | 2026-11-05 |
| [9801](https://github.com/developing-today/code/issues/9801)   | OpenCode: Whisper STT + TTS as routed pool models — failover, proxying, silence auto-trim                                                                                                                                  | status/blocked, queue/blocked, work/assigned                                                                                                                                                                           | 9       | 2026-11-14 |
| [1858](https://github.com/developing-today/code/issues/1858)   | mcpx: port the opencode plugin to opencode v2 (a rewrite, and several things get better)                                                                                                                                   | enhancement, task/triage, source/creator, kind/enhancement, status/triaged, epic/mcpx-platform                                                                                                                         | 26      | 2026-10-28 |
| [2171](https://github.com/developing-today/code/issues/2171)   | OpenClaw: finish setup, begin passing jobs, and review results against OpenCode                                                                                                                                            | task/small-fix, status/parked, source/creator, queue/parked, work/assigned, design/executor-expansion, epic/opencode-platform                                                                                          | 25      | 2026-10-29 |
| [8821](https://github.com/developing-today/code/issues/8821)   | AI Gateway "opencode-gateway" (shipped — tracking home)                                                                                                                                                                    | kind/design, design, epic/design-register, epic/turso-cloudflare                                                                                                                                                       | 11      | 2026-11-12 |
| [9399](https://github.com/developing-today/code/issues/9399)   | Program: pool & effort system — effort tiers per pool, OpenChamber category pools, granular stats store (design mill)                                                                                                      | status/parked, queue/parked, work/assigned, design                                                                                                                                                                     | 10      | 2026-11-13 |
| [9886](https://github.com/developing-today/code/issues/9886)   | Retroactive home for PR #5081: chore(opencode): retire dead provider definitions (12 removed)                                                                                                                              | status:in-review                                                                                                                                                                                                       | 9       | 2026-11-14 |
| [10332](https://github.com/developing-today/code/issues/10332) | claimant mode file: guard the silent-deletion single point of failure                                                                                                                                                      | —                                                                                                                                                                                                                      | 9       | 2026-11-14 |
| [10395](https://github.com/developing-today/code/issues/10395) | Design mill pass 6: fork upkeep — upstream merges + feature-gap analysis (OpenCode, OpenChamber)                                                                                                                           | —                                                                                                                                                                                                                      | 9       | 2026-11-14 |
| [10416](https://github.com/developing-today/code/issues/10416) | replicate: format auto-fix workflow to dezren39/opencode-muse (from code PoC #2015; target repo has issues disabled)                                                                                                       | —                                                                                                                                                                                                                      | 8       | 2026-11-15 |
| [10973](https://github.com/developing-today/code/issues/10973) | fleet gh step 2.2: Queue/GitHub sync verbs for scripted flows (dispatch, claimant, job bodies)                                                                                                                             | status:in-review, task/small-fix                                                                                                                                                                                       | 6       | 2026-11-17 |
| [11149](https://github.com/developing-today/code/issues/11149) | [developing-today-forks/openchamber#2376] fix(web): model-choices FIFO test calls mkfifo unguarded — fails on native Windows (PR #2090 review finding)                                                                     | source/universe, universe/owned                                                                                                                                                                                        | 6       | 2026-11-17 |
| [11150](https://github.com/developing-today/code/issues/11150) | [developing-today-forks/openchamber#2377] docs(web): model-choices docs promise a declined suggestion is never re-offered, but decisions are bounded to the newest 5,000 (PR #2090 review finding)                         | source/universe, universe/owned                                                                                                                                                                                        | 6       | 2026-11-17 |
| [11151](https://github.com/developing-today/code/issues/11151) | [developing-today-forks/openchamber#2378] fix(web): prompt, skill-supporting, plugin and snippet file saves write in place — a save can leave OpenCode-watched files truncated or read half-written                        | source/universe, universe/owned                                                                                                                                                                                        | 6       | 2026-11-17 |
| [11154](https://github.com/developing-today/code/issues/11154) | [developing-today-forks/opencode#2605] fix(core): route-overrides cache keyed on mtime alone serves a stale list after a same-tick external rewrite                                                                        | source/universe, universe/owned                                                                                                                                                                                        | 6       | 2026-11-17 |
| [11692](https://github.com/developing-today/code/issues/11692) | [developing-today-forks/openchamber#2398] fix(web): two live-session status readers still resolve Object.prototype members — warming skips prototype-named sessions and the idle reaper never reaps them                   | source/universe, universe/owned                                                                                                                                                                                        | 5       | 2026-11-18 |
| [11693](https://github.com/developing-today/code/issues/11693) | [developing-today-forks/openchamber#2399] Fix main CI: type-check and lint failures in run 44107506860                                                                                                                     | source/universe, universe/owned                                                                                                                                                                                        | 5       | 2026-11-18 |
| [11694](https://github.com/developing-today/code/issues/11694) | [developing-today-forks/openchamber#2400] fix(ui): concurrent first dictation starts can create competing WebSockets                                                                                                       | source/universe, universe/owned                                                                                                                                                                                        | 5       | 2026-11-18 |
| [11695](https://github.com/developing-today/code/issues/11695) | [developing-today-forks/openchamber#2402] fix(ci): release workflows swallow codesign failure via \|\| true                                                                                                                | source/universe, universe/owned                                                                                                                                                                                        | 5       | 2026-11-18 |
| [11696](https://github.com/developing-today/code/issues/11696) | [developing-today-forks/openchamber#2404] fix(tts): say speak writes to a predictable shared temp file and leaks it on failure — concurrent requests can collide and the file is never cleaned up                          | source/universe, universe/owned                                                                                                                                                                                        | 5       | 2026-11-18 |
| [11697](https://github.com/developing-today/code/issues/11697) | [developing-today-forks/openchamber#2407] fix(ui): null JSON auth responses are misclassified as network failures                                                                                                          | source/universe, universe/owned                                                                                                                                                                                        | 5       | 2026-11-18 |
| [11698](https://github.com/developing-today/code/issues/11698) | [developing-today-forks/opencode#2618] compaction fails on short sessions: 'Compaction produced no summary' / 'Compaction summary reached the output token limit' (9 fleet sessions, 2026-11-17)                           | source/universe, universe/owned                                                                                                                                                                                        | 5       | 2026-11-18 |
| [11699](https://github.com/developing-today/code/issues/11699) | [developing-today-forks/opencode#2619] fix(core): joined Form.ask silently ignores its own timeout — a deduped ask can pend past its deadline, indefinitely when the owner set none                                        | source/universe, universe/owned                                                                                                                                                                                        | 5       | 2026-11-18 |
| [11700](https://github.com/developing-today/code/issues/11700) | [developing-today-forks/opencode#2620] fix(core): parseRenderAnswer returns chat for any answered:false result — timeout/absent reason must be none (PR #2450 review finding)                                              | source/universe, universe/owned                                                                                                                                                                                        | 5       | 2026-11-18 |
| [11701](https://github.com/developing-today/code/issues/11701) | [developing-today-forks/opencode#2621] fix(core): parseRenderAnswer classifies fallback answers without current provider/tier context — a wait for the exhausted provider becomes a switch to it (PR #2450 review finding) | source/universe, universe/owned                                                                                                                                                                                        | 5       | 2026-11-18 |
| [11702](https://github.com/developing-today/code/issues/11702) | [developing-today-forks/opencode#2622] fix(core): default-model resolution bypasses the #566 root-start gate — resolver.resolve passes no session context (PR #2464 review finding)                                        | source/universe, universe/owned                                                                                                                                                                                        | 5       | 2026-11-18 |
| [11703](https://github.com/developing-today/code/issues/11703) | [developing-today-forks/opencode#2623] fix(ai): claude-code renewal adoption does not transfer single-flight ownership — a concurrent load can exchange the adopted refresh token twice (PR #2466 review finding)          | source/universe, universe/owned                                                                                                                                                                                        | 5       | 2026-11-18 |
| [11704](https://github.com/developing-today/code/issues/11704) | [developing-today-forks/opencode#2626] fix(function): cap /share_sync request bodies before parsing or checking the secret                                                                                                 | source/universe, universe/owned                                                                                                                                                                                        | 5       | 2026-11-18 |
| [11705](https://github.com/developing-today/code/issues/11705) | [developing-today-forks/opencode#2630] fix(ci): auto-format rewrites gate-allowlisted lines — a Prettier pass turns check:numeric-boundaries red on main with a stale entry and an unlisted candidate                      | source/universe, universe/owned                                                                                                                                                                                        | 5       | 2026-11-18 |
| [11706](https://github.com/developing-today/code/issues/11706) | [developing-today-forks/opencode#2631] fix(function): share_delete leaves persisted R2 session objects behind                                                                                                              | source/universe, universe/owned                                                                                                                                                                                        | 5       | 2026-11-18 |
| [11707](https://github.com/developing-today/code/issues/11707) | [developing-today-forks/opencode#2632] fix(services/update): publish/admin request bodies have no size cap before JSON/form parse and D1 write                                                                             | source/universe, universe/owned                                                                                                                                                                                        | 5       | 2026-11-18 |
| [11708](https://github.com/developing-today/code/issues/11708) | [developing-today-forks/opencode#2634] fix(stats): newsletter subscribe reads its public multipart body unbounded before any size check                                                                                    | source/universe, universe/owned                                                                                                                                                                                        | 5       | 2026-11-18 |
| [11709](https://github.com/developing-today/code/issues/11709) | [developing-today-forks/opencode#2635] fix(console): enterprise contact form reads its public JSON body unbounded before any size check                                                                                    | source/universe, universe/owned                                                                                                                                                                                        | 5       | 2026-11-18 |
| [11710](https://github.com/developing-today/code/issues/11710) | [developing-today-forks/opencode#2636] fix(function): compare share and admin secrets in constant time                                                                                                                     | source/universe, universe/owned                                                                                                                                                                                        | 5       | 2026-11-18 |
| [11711](https://github.com/developing-today/code/issues/11711) | [developing-today-forks/opencode#2638] main CI failure: developing-today-forks/opencode — agents workflow (run 44247391571)                                                                                                | source/universe, universe/owned                                                                                                                                                                                        | 5       | 2026-11-18 |
| [11764](https://github.com/developing-today/code/issues/11764) | openchamber session list fails (rc=1, ~4s) and stops OpenChamber scheduled queue lanes before claiming                                                                                                                     | —                                                                                                                                                                                                                      | 5       | 2026-11-18 |
| [12173](https://github.com/developing-today/code/issues/12173) | [developing-today-forks/openchamber#2430] fix(ui): prevent saving stale drafts after a dirty guest editor disappears                                                                                                       | source/universe, universe/owned                                                                                                                                                                                        | 2       | 2026-11-21 |
| [12174](https://github.com/developing-today/code/issues/12174) | [developing-today-forks/openchamber#2431] fix(electron): release isolated-space browser preview state after space removal                                                                                                  | source/universe, universe/owned                                                                                                                                                                                        | 2       | 2026-11-21 |
| [12175](https://github.com/developing-today/code/issues/12175) | [developing-today-forks/openchamber#2432] fix(electron): handle rejected window-geometry persistence without crashing the app                                                                                              | source/universe, universe/owned                                                                                                                                                                                        | 2       | 2026-11-21 |
| [12176](https://github.com/developing-today/code/issues/12176) | [developing-today-forks/openchamber#2435] fix(electron): handle rejected installed-app cache refreshes                                                                                                                     | source/universe, universe/owned                                                                                                                                                                                        | 2       | 2026-11-21 |
| [12177](https://github.com/developing-today/code/issues/12177) | [developing-today-forks/openchamber#2437] fix(electron): catch rejected host deep-link activation                                                                                                                          | source/universe, universe/owned                                                                                                                                                                                        | 2       | 2026-11-21 |
| [12178](https://github.com/developing-today/code/issues/12178) | [developing-today-forks/openchamber#2438] fix(electron): handle rejected splash-color settings writes                                                                                                                      | source/universe, universe/owned                                                                                                                                                                                        | 2       | 2026-11-21 |
| [12179](https://github.com/developing-today/code/issues/12179) | [developing-today-forks/openchamber#2440] fix(electron): handle rejected initial window navigation promises                                                                                                                | source/universe, universe/owned                                                                                                                                                                                        | 2       | 2026-11-21 |
| [12180](https://github.com/developing-today/code/issues/12180) | [developing-today-forks/openchamber#2442] fix(web): project plan saves write the plan body in place — a save can leave it truncated or read half-written                                                                   | source/universe, universe/owned                                                                                                                                                                                        | 2       | 2026-11-21 |
| [12181](https://github.com/developing-today/code/issues/12181) | [developing-today-forks/openchamber#2444] fix(web): prevent concurrent screenshots from overwriting the same capture                                                                                                       | source/universe, universe/owned                                                                                                                                                                                        | 2       | 2026-11-21 |
| [12182](https://github.com/developing-today/code/issues/12182) | [developing-today-forks/opencode#2657] fix(ai): guard unguarded `new URL(result.url)` in `isSessionExpiredStatus` — a non-empty malformed final URL throws (inconsistent with guarded `teamIdOf`)                          | source/universe, universe/owned                                                                                                                                                                                        | 2       | 2026-11-21 |
| [12183](https://github.com/developing-today/code/issues/12183) | [developing-today-forks/opencode#2658] fix(ai): honor tool indexes when streamed IDs collide                                                                                                                               | source/universe, universe/owned                                                                                                                                                                                        | 2       | 2026-11-21 |
| [12190](https://github.com/developing-today/code/issues/12190) | [dezren39/darwin-nix#2032] fix(opencode-db-backup): a failed final re-read exits nonzero after the backup is published and success is printed                                                                              | source/universe, universe/owned                                                                                                                                                                                        | 2       | 2026-11-21 |
| [12430](https://github.com/developing-today/code/issues/12430) | [developing-today-forks/openchamber#2445] fix(electron): Linux autostart reconcile rewrites and deletes openchamber.desktop without ownership — user edits revert at every launch, foreign entries get rm-ed               | source/universe, universe/owned                                                                                                                                                                                        | 1       | 2026-11-22 |
| [12431](https://github.com/developing-today/code/issues/12431) | [developing-today-forks/openchamber#2448] fleet-stalling: session create times out at the 4s control deadline for 5+ consecutive conductor cycles — dispatch fully blocked                                                 | source/universe, universe/owned                                                                                                                                                                                        | 1       | 2026-11-22 |
| [12432](https://github.com/developing-today/code/issues/12432) | [developing-today-forks/openchamber#2449] session create ignores --dir: worker launched into auto-generated worktree instead of the conductor's prepared worktree                                                          | source/universe, universe/owned                                                                                                                                                                                        | 1       | 2026-11-22 |
| [12433](https://github.com/developing-today/code/issues/12433) | [developing-today-forks/openchamber#2452] fix(web): avoid deleting a newer self-handoff request during consume                                                                                                             | source/universe, universe/owned                                                                                                                                                                                        | 1       | 2026-11-22 |
| [12434](https://github.com/developing-today/code/issues/12434) | [developing-today-forks/openchamber#2455] fix(web): contain recursive filesystem deletes within workspace                                                                                                                  | source/universe, universe/owned                                                                                                                                                                                        | 1       | 2026-11-22 |
| [12435](https://github.com/developing-today/code/issues/12435) | [developing-today-forks/openchamber#2457] fix(web): contain filesystem mkdir targets after resolving symlinks                                                                                                              | source/universe, universe/owned                                                                                                                                                                                        | 1       | 2026-11-22 |
| [12436](https://github.com/developing-today/code/issues/12436) | [developing-today-forks/opencode#2659] fix(desktop): handle rejected renderer loadURL promises                                                                                                                             | source/universe, universe/owned                                                                                                                                                                                        | 1       | 2026-11-22 |
| [12437](https://github.com/developing-today/code/issues/12437) | [developing-today-forks/opencode#2660] fix(console): escape enterprise form fields before rendering the sales email as HTML                                                                                                | source/universe, universe/owned                                                                                                                                                                                        | 1       | 2026-11-22 |
| [12438](https://github.com/developing-today/code/issues/12438) | [developing-today-forks/opencode#2662] fix(protocol): openapi.json stale on main — route-overrides API, instruction-entry 404 and inbox-cancel 409 missing; check:generated red                                            | source/universe, universe/owned                                                                                                                                                                                        | 1       | 2026-11-22 |
| [12439](https://github.com/developing-today/code/issues/12439) | [developing-today-forks/opencode#2663] fix(ai): preserve failed tool-result status in Cohere history                                                                                                                       | source/universe, universe/owned                                                                                                                                                                                        | 1       | 2026-11-22 |
| [12440](https://github.com/developing-today/code/issues/12440) | [developing-today-forks/opencode#2664] fix(ai): preserve failed tool-result status in Gemini history                                                                                                                       | source/universe, universe/owned                                                                                                                                                                                        | 1       | 2026-11-22 |
| [12441](https://github.com/developing-today/code/issues/12441) | [developing-today-forks/opencode#2665] fix(ai): preserve failed tool-result status in Command Code history                                                                                                                 | source/universe, universe/owned                                                                                                                                                                                        | 1       | 2026-11-22 |
| [12445](https://github.com/developing-today/code/issues/12445) | [dezren39/darwin-nix#2042] opencode-tmp-guard: fail visibly when repo discovery or metadata refresh is incomplete                                                                                                          | source/universe, universe/owned                                                                                                                                                                                        | 1       | 2026-11-22 |
| [12734](https://github.com/developing-today/code/issues/12734) | OpenChamber constant queue lanes: 8 oc-code lanes verified live; WORK PRIORITIES prompt block added (2026-11-22)                                                                                                           | —                                                                                                                                                                                                                      | 1       | 2026-11-23 |
| [12766](https://github.com/developing-today/code/issues/12766) | Antigravity ACP: OpenClaw agy harness points at unauthenticated standalone server (fix: native agy acp); OpenCode door barred by 2026-11-14 ruling                                                                         | —                                                                                                                                                                                                                      | 0       | 2026-11-23 |
| [12792](https://github.com/developing-today/code/issues/12792) | Rotate the OpenChamber Android signing-keystore password (plaintext in a session transcript; found by the #10412 review)                                                                                                   | —                                                                                                                                                                                                                      | 0       | 2026-11-23 |
| [12977](https://github.com/developing-today/code/issues/12977) | [developing-today-forks/openchamber#2458] fix(vscode): reject fs read and stat paths escaping through symlinks                                                                                                             | source/universe, universe/owned                                                                                                                                                                                        | 0       | 2026-11-23 |
| [12979](https://github.com/developing-today/code/issues/12979) | [developing-today-forks/openchamber#2463] fix(web): redact mid-line sensitive key=value values in Git output                                                                                                               | source/universe, universe/owned                                                                                                                                                                                        | 0       | 2026-11-23 |
| [13135](https://github.com/developing-today/code/issues/13135) | perf(opencode-claimant): cut poll.py end-to-end latency (parallelize + bound its sequential external reads)                                                                                                                | status:in-review                                                                                                                                                                                                       | 0       | 2026-11-23 |
| [13462](https://github.com/developing-today/code/issues/13462) | impl(backups): mesh leg — OpenCode session DB backup (gated on research #9094)                                                                                                                                             | kind/feature, task/small-fix, epic/atlas-program                                                                                                                                                                       | 0       | 2026-11-23 |
| [13665](https://github.com/developing-today/code/issues/13665) | claimant: opencode-lane task 20708 unclaimable after muse platform_limit repair (claim->refuse loop)                                                                                                                       | app/pm-stack                                                                                                                                                                                                           | 0       | 2026-11-23 |
| [13733](https://github.com/developing-today/code/issues/13733) | opencode claimant: task 20708 stuck handed_off with platform_limit openchamber                                                                                                                                             | —                                                                                                                                                                                                                      | 0       | 2026-11-23 |

---

## 4. PM-stack integration surface (code repo, `pm-stack/`, read on the laptop)

### 4.1 `pm-stack/opencode-claimant` — the dispatcher

A Python claimant/executor (main script `claimant.py`, ~2,250 lines, plus `poll.py`, `launch_one.py`, `dispatch_batch.py`, `send_to_opencode.py`, `babysitter.py`, `nudges.py`; README 546 lines; specified in code#3026). Each fire: (1) supervises bound sessions and recovers zero-message/gone sessions (code#4758 rules: verify via the fork server's per-id endpoints, DELETE the verified-empty session, atomically reclaim via `repo-queue bind`); (2) discovers unbound `lane: "opencode"` tasks across the queue broker; (3) enforces a 36-root-session cap counted from the fork server's `GET /api/session/active` (claimant roots only, ruling session-cap-5016/code#5016); (4) creates sessions with the v2 create shape (`POST /api/session` with `location.directory`, verified against the repo primary before bind — a top-level `directory` is stripped by v2, code#4870), binds via `repo-queue bind`, and delivers the prompt with verified delivery; (5) logs the fire to `~/.local/state/opencode-claimant/fires.jsonl`. Mode gate: `shadow` (default, mutates nothing) vs `live`, from a local mode file. On cap saturation it hands one candidate back per fire, parked with reason `executor-saturated` and a capacity ready-condition (code#1187). Every fire also runs a babysitter round (code#1338): a verdict (progressing/stalled/done-unreported/blocked/walled/idle) for every root session, stall = ≥20 min silent with no live CI on the bound PR, confirmed on a second round. It drives the **fork** server specifically: the README's session contract (code#4912) and seam contract (code#7770) are written against the fork's v2 API behaviour (singular `/message`, `/session/active`, location-verified create) that upstream v1 did not have.

### 4.2 `pm-stack/opencode-scheduled-jobs` — fire-and-forget fork jobs

Source for small scheduled OpenCode jobs on the laptop: each job is a systemd user service/timer pair with its route visible in the unit, a read-only prompt, and a stable per-job state dir. The registry (`jobs.json`) currently holds exactly **one** job: `quota-freshness-summary` — hourly, route `opencode-route/workhorse` (a fork route pool, §1.3), reading a sanitised projection of `~/.local/state/pm-aggregator/quota-current.json` (only provider names, status, timestamps, window names/states reach the model; credentials, raw usage, limits and balances are stripped by `summarize.py` before the fork session sees them). Enabled 2026-11-21 after an explicit pilot decision; first hourly firing observed exit 0. Logs at `~/.local/state/opencode-scheduled/quota-freshness-summary/{latest.log,history.jsonl}`. The README is explicit that the timer provides **no** stall supervision — that remains the claimant babysitter's job (§4.1), and that a monitor must correlate via directory-scoped server API reads because `opencode session list` “is not a reliable discovery surface” (against the fork server).

### 4.3 `pm-stack/opencode-usage-export` — the usage producer

A read-only **pull** exporter (Node, `node:sqlite`, store opened read-only with `query_only`; 15-minute systemd timer on the laptop) over the fork's own history database — code#2072 slice 1, “the same pull pattern as the Muse runtime-db export. No push instrumentation is added to OpenCode/OpenChamber.” It probes candidate stores in order (`~/.local/share/opencode/opencode.db`, `~/.local/share/openchamber/opencode.db`, `~/.local/state/opencode/opencode.db`), picks the first that is _live_ (has the v2 tables `session_v2`/`session_message` and a newest `time_updated` within 24 h), surfaces a store move in the tick document, and otherwise fails loudly (`store-missing`/`store-stale`/`schema-drift`/`read-failed`, exits 3–6, failure documents carry no sessions). Output: `~/atlas/opencode-usage-export/{latest.json,history.jsonl,state.json}` — per-session cost/token totals with parentage, feeding the PM aggregator's usage views. The README warns that the legacy `session`/`message`/`part` tables in the same file froze on 2026-10-06; the exporter reads only the v2 tables the fork writes.

---

## 5. Dependency notes (only edges seen in titles/bodies/READMEs actually read)

1. code#1357 and code#1363 both follow the code#1344 research (attach-first family, titles/bodies of the three issues).
2. opencode#2782 (config projection strips `model_routes` purpose / `model_designations`) states in its title that it is “needed by code#13675 Part 2” — the fork fix blocks that code work.
3. opencode#2681 is explicitly “follow-ups after #2674, #2676 and #2679” (same-repo opencode numbers).
4. opencode#2505 (no CI coverage for the TursoDB backend) is about PR opencode#2499's zero tests; the TursoDB evaluation itself is the open spike opencode#2492, which also owns the standing branch `spike/2492-tursodb-backend`.
5. opencode#2490 and opencode#2491 both cite the code#5060 ruling; #2491 (consolidation of OpenChamber's server control into OpenCode) is the parent intention, #2490 (quota credential in the server credential store) is named “(code#5060 fix)”.
6. openchamber#384 (upgrade the embedded OpenCode to a v2 build) depends on the opencode fork's v2 line — the same migration as opencode#374/#2295/#2290; the openchamber README likewise states its messaging surface “needs an OpenCode build with messaging (developing-today-forks/opencode)”.
7. Claimant chain (claimant README): babysitter rounds (code#1338) produce the confirmed stalls that the nudges slice (code#1187's next slice) consumes; the claimant's session contract is code#4912 phase 4 (audit code#4916) over phases code#4757/#4870/#4758, and its seam contract is code#7770 over code#7325/#7375/#7706, #7367/#7760, #7356/#7366/#7732.
8. opencode-usage-export is code#2072 slice 1; the README states slices 2–7 (webhooks, views, rollups, projections) are separate follow-ups — they depend on this exporter existing, not vice versa.
9. opencode-scheduled-jobs depends on the claimant babysitter for stall supervision (its README: “that remains the babysitter's responsibility”) and on the aggregator's quota snapshot file as its only input.
10. openchamber#1573 is a split of openchamber PR #216 (“Split 216b of PR #216”); openchamber#937 is the stale-PR resolution for the same PR #216 — the two open issues share that parent PR.
11. No other blocking edges are asserted: no `blocked-by` label or explicit dependency field exists on the fork repos beyond the `status/blocked`/`queue/blocked` labels shown in §3.

---

## 6. Disagreements — where the docs/READMEs and the repos' actual state differ

1. **Upstream-sync date (both forks).** Both fork READMEs state the latest upstream commit merged is from **Fri Oct 16 2026** (opencode: `00e7472f…`; openchamber: `2b94db84…` via `8f9956331`). The repos show later upstream syncs merged: opencode PR #2683 “sync anomalyco/opencode dev (2026-11-22)” and #2690 “sync anomalyco/opencode dev (2026-11-23)”; openchamber PR #2502 “Merge upstream openchamber main (v2.8.0)” (2026-11-22) and #2510 “Merge upstream openchamber main (3a0dbeafb4)…” (2026-11-23). opencode#2677 “Follow-ups from the 2026-11-22 upstream sync” is itself open. The README notices are ~5 weeks stale.
2. **Fork parentage (both forks).** Both READMEs describe the repo as a fork of the upstream project (anomalyco/opencode; openchamber/openchamber). The API says each org fork's direct `parent` is Drew's personal fork (dezren39/opencode; dezren39/openchamber); upstream is the `source`, one step further up.
3. **openchamber#2564 vs its merged fix.** Issue #2564 (preview-grant symlink escape) is still **open**, labelled `status/in-progress` / `queue/claimed`, yet its fix PR #2579 (“preview-grant serve route re-checks canonical path vs grant root (openchamber#2564)”) and the underlying commit both merged on 2026-11-23 (also visible at the head of the laptop's openchamber clone). The issue record does not reflect the landed fix.
4. **“Done” labels on open issues.** opencode#2505 carries `status/done` and openchamber#2081 carries `status/done` + `queue/done`, yet both issues are open. Either the labels or the issue states are wrong.
5. **“Shipped” titles on open migration issues.** opencode#2295 is titled “…core migration (**shipped** — tracking home)” and openchamber#2126 “…(**shi**pped…)”, while the parent migration opencode#374 “Migrate the fleet to OpenCode 2” (priority/high), opencode#2290 (port the v1 plugins) and openchamber#384 (embedded v2 upgrade) all remain open — the migration is simultaneously recorded as shipped and as outstanding, on different surfaces.
6. **openchamber Actions: disabled vs running.** openchamber#2274 (2026-11-13) states “GitHub Actions is disabled for this repo again — no workflow has run since 2026-11-11”. The repo's own later record contradicts the standing state: openchamber#2586 (2026-11-23) reports a stale-workflow Actions run (run 45147203731), and PRs continued merging through 2026-11-23 — Actions did run after 2026-11-11, whatever was true on the 13th.
7. **Claimant README, internally and vs current operation.** (a) The README's queue section says the claimant “monitors four queues” (the four `repo-health-*-pilot` queues), while its own `launch_one.py` section describes the current one-queue form (“For the one-queue `code` stream, use `code:<task_id>`…”). (b) Its Zero-Message Recovery step 1 writes the endpoint as `GET /api/session/{id}/messages` (plural) in the same sentence that warns the plural endpoint 404s and the singular is correct; the Session Contract table later gives the singular `/message` form.
8. **Fork default branch vs tracking claim.** The opencode README says the fork tracks upstream `dev` exclusively, but the fork's own default branch in the API is `main` (all 1,254 home-grown merged PRs target `main`); `dev` is upstream's branch name, not the fork's — a reader following the README literally would look for the fork's work on a branch the fork does not default to.
9. **opencode CI state, two ways at once.** opencode#2785 (2026-11-23) reports “CI run silence — no completed Actions run within threshold”, while opencode#2638 and #2518 are open CI-_failure_ issues about specific completed runs (44247391571, 43849921062). Both can be true for different workflows, but as recorded they describe the same repo's CI as simultaneously silent and failing, with no surface reconciling the two.

---

## 7. Raw material deposit (per the 2026-11-23 standing rule)

Deposited in `~/workspace/pm-book/sideline-material/oc-survey/`: the full API dumps behind every count in this survey — merged/closed/open PR enumerations and open/closed issue lists for both forks, branch and label lists, the code-repo title-search sets (opencode/openchamber/claimant) and open-mention sets, the fetch scripts, and laptop README captures (fork notices, the full pm-stack READMEs for §4, and the claimant README). File list is in the deposit's `README.md`.
