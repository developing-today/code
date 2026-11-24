# Our changes, shipped and in flight

This chapter records what the fleet has changed in the two forks. It has two halves and keeps them apart. Shipped work is merged pull requests. In-flight work is open pull requests and open issues. An open issue is an intention; it is recorded in this chapter as in flight and in the issue maps as ordered work, and nowhere in this book as a change.

## What counts as shipped

The survey's definitions, applied throughout this part:

- **Merged** = `merged_at` non-null in the API response.
- **Shipped** = merged into `main`. On the opencode fork, 1,254 of 1,258 merged PRs target `main`; the 4 exceptions target `sweep/560-provider-models`, `spike/2492-tursodb-backend` and `pin/darwin-nix-opencode-6c43550`.
- **Grouping rule:** theme = the conventional-commit scope in the PR title (`feat(scope):`, `fix(scope):`, …); titles without a scope are grouped by the first matching keyword (route/quota/provider/session/tui/console/plugin/nix/ci/test/doc/upstream), else "other/no-scope".

The headline counts at the survey read (2026-11-24T00:35–01:05Z): opencode fork — 1,258 merged PRs, of 1,438 closed, with 9 open. Openchamber fork — 1,186 merged PRs, of 1,290 closed, with 2 open. Authorship of the merged set: opencode — dezren39 1,211, developing-today[bot] 42, dependabot[bot] 5; openchamber — dezren39 1,091, developing-today[bot] 86, github-actions[bot] 8, dependabot[bot] 1.

## Shipped: opencode, 1,258 merged PRs by theme

| Theme | Merged PRs |
|---|---|
| core | 423 |
| ai | 181 |
| ci | 125 |
| cli | 49 |
| docs | 47 |
| roadmap | 47 |
| server | 42 |
| schema | 26 |
| script | 24 |
| tui | 19 |
| gui-extensions | 17 |
| client | 15 |
| app | 14 |
| desktop | 14 |
| console | 14 |
| nix | 13 |
| util | 10 |
| plugins/mcp | 9 |
| web | 8 |
| specs | 7 |
| other/no-scope | 7 |
| deps | 7 |
| quota | 6 |
| facade | 6 |
| plugin-github | 5 |
| function | 5 |
| core,ai | 5 |
| codemode | 4 |
| providers | 4 |
| routes | 4 |
| www | 4 |
| protocol | 3 |
| plugin-browser | 3 |
| config | 3 |
| flake | 3 |
| routes/pools | 3 |
| nix/packaging | 3 |
| console,stats | 3 |
| deps-dev | 3 |
| legacy | 2 |
| session | 2 |
| session-ui | 2 |
| core,server | 2 |
| util,core | 2 |
| ui,gui-extensions | 2 |
| lint | 2 |
| core,schema | 2 |
| http-recorder | 2 |
| stats | 2 |
| changelog | 2 |
| cli,tui | 2 |
| client,httpapi-codegen | 2 |
| plugin | 2 |
| sdk | 2 |
| cli-rs | 1 |
| ai/quota | 1 |
| tui,codemode,ci | 1 |
| update | 1 |
| ui | 1 |
| a11y | 1 |
| resolver | 1 |
| stats-app | 1 |
| console-app | 1 |
| astro | 1 |
| ci,core | 1 |
| app,session-ui | 1 |
| quota/plans | 1 |
| core,server,tui | 1 |
| cost | 1 |
| util,tui | 1 |
| test | 1 |
| tool | 1 |
| vibe | 1 |
| pkg | 1 |
| app,tui | 1 |
| files | 1 |
| tests | 1 |
| cli,server | 1 |
| ai,tui,cli | 1 |
| httpapi-codegen | 1 |
| mcpx | 1 |
| github | 1 |
| storage | 1 |
| auth | 1 |
| latex | 1 |
| cli,client | 1 |
| client,app | 1 |
| core,ai,tui,cli | 1 |
| core,app | 1 |
| tui,cli | 1 |
| security | 1 |
| provider | 1 |
| cli,core | 1 |
| services | 1 |
| upstream sync | 1 |
| desktop,core | 1 |
| acp | 1 |

The centre of the opencode fork's shipped work is the agent core and the AI layer: `core` 423 and `ai` 181 are 604 of the 1,258. `ci` at 125 is the third theme: the fork's own build and check machinery is a standing workstream in its own right, not overhead around the other work. The fork surface the READMEs claim in [The forks: what they are and why we carry them](the-forks.md) — routes, quota, plans — sits inside these scopes rather than under its own: the scope vocabulary gives `quota` 6, `routes` 4, `routes/pools` 3, `quota/plans` 1 and `ai/quota` 1 as standalone themes, while route and quota work also lands under `core` and `ai` titles. The theme table is a count by title scope. It is not a feature inventory, and this chapter does not read it as one.

## Shipped: openchamber, 1,186 merged PRs by theme

| Theme | Merged PRs |
|---|---|
| web | 475 |
| ui | 108 |
| vscode | 89 |
| ci | 89 |
| quota | 48 |
| routing | 48 |
| electron | 37 |
| roadmap | 35 |
| other/no-scope | 26 |
| web,vscode | 22 |
| docs | 20 |
| nix | 14 |
| deps | 11 |
| settings | 8 |
| agent-tool | 8 |
| chat | 7 |
| mobile | 7 |
| control | 7 |
| flake | 6 |
| git | 5 |
| goal | 5 |
| security | 5 |
| release | 5 |
| scripts | 4 |
| tests | 4 |
| quota/plans | 4 |
| i18n | 4 |
| environment | 4 |
| web,ui | 3 |
| sidebar | 3 |
| browser | 3 |
| fs | 2 |
| upstream sync | 2 |
| goal-writer | 2 |
| sync | 2 |
| cli | 2 |
| ui,web | 2 |
| model-choices | 2 |
| routes/pools | 2 |
| plugins/mcp | 2 |
| sessions | 2 |
| notes | 2 |
| providers | 2 |
| routes | 2 |
| routing-ui | 2 |
| proxy | 2 |
| dictation | 2 |
| lifecycle | 2 |
| markdown | 2 |
| audit | 1 |
| client-auth | 1 |
| tts | 1 |
| models | 1 |
| event-stream | 1 |
| session-work | 1 |
| perf | 1 |
| oc-review | 1 |
| opencode | 1 |
| dependabot | 1 |
| repo | 1 |
| workflow | 1 |
| notifications | 1 |
| web,vscode,ui | 1 |
| web,ui,sdk | 1 |
| project-context | 1 |
| quota,web | 1 |
| nix/packaging | 1 |
| workflow-audit | 1 |
| watcher | 1 |
| themes | 1 |
| github-routes | 1 |
| context | 1 |
| opencode-update | 1 |
| quota-ui | 1 |
| web-cli | 1 |
| deps-dev | 1 |
| dev-servers | 1 |
| github | 1 |
| source-control | 1 |
| stale | 1 |
| sdk | 1 |
| contributing | 1 |
| messaging | 1 |
| worktrees | 1 |
| update | 1 |

The centre of the openchamber fork's shipped work is the web surface: `web` alone is 475 of 1,186, and `web` with `ui` 108, `vscode` 89 and `electron` 37 makes the operator-facing surfaces the bulk of the fork. The routing and quota surface the fork claims is visible under its own scopes here: `quota` 48, `routing` 48, `quota/plans` 4, `routes/pools` 2, `routes` 2, `routing-ui` 2 and `quota-ui` 1. `ci` at 89 matches the opencode fork's pattern: the fork's own machinery is a workstream, and the open issue map shows its cost — four of the fork's eight priority/high open issues are CI items ([OpenChamber: prioritised issue map](openchamber-issue-map.md)).

The per-PR enumeration behind both tables — every merged PR with title, author and merge date — is in the survey, §1.5, and in the deposit. This chapter carries the groupings; the survey carries the rows.

## In flight

At the survey read the forks carry 11 open pull requests between them: 9 on the opencode fork, 2 on the openchamber fork. The survey counts the open PRs and does not enumerate them; their titles and authors sit in the deposit files `pulls-open-oc.json` and `pulls-open-och.json`, outside the survey text this part rests on, and this chapter states the counts without a per-PR list.

The in-flight intention set is the open issue set: 30 open issues on each fork, listed and ordered in [OpenCode: prioritised issue map](opencode-issue-map.md) and [OpenChamber: prioritised issue map](openchamber-issue-map.md), and the 91 open programme issues in the code repo defined in [Goals and intentions](goals-and-intentions.md). Standing branches carry in-flight work that has no merged form yet; the named ones are the TursoDB spike branch `spike/2492-tursodb-backend` (opencode#2492), `feat/2490-session-cookie-credential` and `feat/2494-route-overrides-api` on the opencode fork, and `216b-goal-spec-jobs` and `investigation/2325-dependabot-regression` on the openchamber fork ([The forks: what they are and why we carry them](the-forks.md)).

## The code-repo machinery that runs on the forks

Three components in the code repo's `pm-stack/` exist to drive the forks. They are code-repo work, not fork PRs, and they are part of the programme's shipped change because the forks' shipped surface is what they drive. The survey read all three on the laptop.

### The claimant: the dispatcher

`pm-stack/opencode-claimant` is a Python claimant/executor (main script `claimant.py`, ~2,250 lines, plus `poll.py`, `launch_one.py`, `dispatch_batch.py`, `send_to_opencode.py`, `babysitter.py`, `nudges.py`; README 546 lines; specified in code#3026). Each fire: (1) supervises bound sessions and recovers zero-message/gone sessions (code#4758 rules: verify via the fork server's per-id endpoints, DELETE the verified-empty session, atomically reclaim via `repo-queue bind`); (2) discovers unbound `lane: "opencode"` tasks across the queue broker; (3) enforces a 36-root-session cap counted from the fork server's `GET /api/session/active` (claimant roots only, ruling session-cap-5016/code#5016); (4) creates sessions with the v2 create shape (`POST /api/session` with `location.directory`, verified against the repo primary before bind — a top-level `directory` is stripped by v2, code#4870), binds via `repo-queue bind`, and delivers the prompt with verified delivery; (5) logs the fire to `~/.local/state/opencode-claimant/fires.jsonl`. Mode gate: `shadow` (default, mutates nothing) vs `live`, from a local mode file. On cap saturation it hands one candidate back per fire, parked with reason `executor-saturated` and a capacity ready-condition (code#1187). Every fire also runs a babysitter round (code#1338): a verdict (progressing/stalled/done-unreported/blocked/walled/idle) for every root session, stall = ≥20 min silent with no live CI on the bound PR, confirmed on a second round.

It drives the **fork** server specifically: the README's session contract (code#4912) and seam contract (code#7770) are written against the fork's v2 API behaviour (singular `/message`, `/session/active`, location-verified create) that upstream v1 did not have. The PM-side account of the same component is [The claimant: keeping sessions honest](../concepts/claimant.md). The claimant README disagrees with itself and with current operation at two points — four queues in its queue section vs the one-queue form its own `launch_one.py` section describes, and a plural `/messages` endpoint written in the same sentence that warns the plural 404s (D56).

### Scheduled jobs: fire-and-forget fork jobs

`pm-stack/opencode-scheduled-jobs` is the source for small scheduled OpenCode jobs on the laptop: each job is a systemd user service/timer pair with its route visible in the unit, a read-only prompt, and a stable per-job state dir. The registry (`jobs.json`) holds exactly **one** job: `quota-freshness-summary` — hourly, route `opencode-route/workhorse` (a fork route pool), reading a sanitised projection of `~/.local/state/pm-aggregator/quota-current.json` (only provider names, status, timestamps, window names/states reach the model; credentials, raw usage, limits and balances are stripped by `summarize.py` before the fork session sees them). Enabled 2026-11-21 after an explicit pilot decision; first hourly firing observed exit 0. Logs at `~/.local/state/opencode-scheduled/quota-freshness-summary/{latest.log,history.jsonl}`. The README is explicit that the timer provides **no** stall supervision — that remains the claimant babysitter's job — and that a monitor must correlate via directory-scoped server API reads because `opencode session list` "is not a reliable discovery surface" (against the fork server). The PM-side account is [Scheduled jobs and the history they keep](../concepts/scheduled-jobs.md).

### Usage export: the usage producer

`pm-stack/opencode-usage-export` is a read-only **pull** exporter (Node, `node:sqlite`, store opened read-only with `query_only`; 15-minute systemd timer on the laptop) over the fork's own history database — code#2072 slice 1, "the same pull pattern as the Muse runtime-db export. No push instrumentation is added to OpenCode/OpenChamber." It probes candidate stores in order (`~/.local/share/opencode/opencode.db`, `~/.local/share/openchamber/opencode.db`, `~/.local/state/opencode/opencode.db`), picks the first that is *live* (has the v2 tables `session_v2`/`session_message` and a newest `time_updated` within 24 h), surfaces a store move in the tick document, and otherwise fails loudly (`store-missing`/`store-stale`/`schema-drift`/`read-failed`, exits 3–6, failure documents carry no sessions). Output: `~/atlas/opencode-usage-export/{latest.json,history.jsonl,state.json}` — per-session cost/token totals with parentage, feeding the PM aggregator's usage views ([Health, quota and usage surfaces](../concepts/health-surfaces.md)). The README warns that the legacy `session`/`message`/`part` tables in the same file froze on 2026-10-06; the exporter reads only the v2 tables the fork writes. Slices 2–7 of code#2072 are stated follow-ups: intentions, recorded in [Goals and intentions](goals-and-intentions.md), not shipped work.

Sources. Survey file `~/workspace/pm-book/survey/oc-programme.md` §1.4, §1.5 (grouping rule, both theme tables, per-PR enumeration) and §4.1–§4.3; deposit `~/workspace/pm-book/sideline-material/oc-survey/` (`pulls-closed-oc.json`, `pulls-closed-och.json`, `pulls-open-oc.json`, `pulls-open-och.json`, `pm-stack-opencode-claimant-README.md`, `pm-stack-opencode-scheduled-jobs-README-and-jobs.md`, `pm-stack-opencode-usage-export-README.md`).
