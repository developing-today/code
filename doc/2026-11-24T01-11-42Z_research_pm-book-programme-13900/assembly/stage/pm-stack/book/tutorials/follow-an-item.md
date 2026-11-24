# Tutorial: follow an item from issue to done

This tutorial follows one work item through every surface that carries it: the GitHub issue, the portfolio pool, the aggregator, the Kaneo board, the claimant or dispatch session, and the closure guards. Read the item on each surface in order. Each surface names the item in its own form; the refs join them.

## Step 1: Start at the GitHub issue

The item starts as a GitHub issue or pull request in a fleet repo. Its aggregator subject form is `gh:…`, and other surfaces join to it by ref, in the form `&lt;owner&gt;/&lt;repo&gt;#&lt;number&gt;`.

Record three facts from the issue before moving on: its repo and number, its labels, and its sub-issue parent, if it has one. Hierarchy in this stack is the native GitHub sub-issue parent. Dependencies are the native `blockedBy` and `blocking` relations. Prose in the issue body is a substitute for neither.

## Step 2: Read the item in the portfolio pool

The item enters the portfolio through org Project "Developing Today" (#4). The aggregator reads the pool in `aggregator/src/projects.ts` and takes four single-select fields verbatim: Status, Area, Horizon, Kind.

Kind has exactly two values in code: `Task` and `Epic` (`aggregator/src/model.ts`). An item with Kind `Epic` gains an `epicRef`, resolved through the native sub-issue parent. Children group under that ref in the epic rollups in `aggregator/src/epics.ts`.

There is no aggregator level in this ladder (D10). The ladder is: program (document scheme, surfaced as an Area and as a registry project) → project (registry plus Project #4) → epic (Kind `Epic` plus sub-issue parent) → task or item. The word "aggregator" names the dashboard service.

The Project's single writer is the orchestrator. Agents and the dashboard do not edit Project fields directly. A dashboard status change travels as a `status-request` action; the orchestration pass performs the write and reports the outcome back.

## Step 3: Read the item in the aggregator

Open the item's detail page:

```text
/item/&lt;subject&gt;
```

Subject forms include `gh:`, `session:`, and `orch:` refs. The machine twin follows the pattern `/api/item/&lt;id&gt;.json`. An unresolved id is a 404.

Find the same item on the surfaces that carry it:

- `/` and `/today` — the front door and the day view.
- `/board` — the cross-repo work-in-progress board.
- `/queue` — in progress, blocked, recently completed, and next-up work across projects.
- `/program` and `/program/&lt;id&gt;` — the project hub and the item's project page.
- `/discuss/&lt;subject&gt;` — the discussion surface for the item. Comments post through `POST /api/comments`.

The aggregator joins its sources per item. The projector supplies links and the latest comments per subject, through `aggregator/src/sources/projector.ts`. The projector's own record of the item comes from its webhook intake and reconcile polls. See [Projector internals](../library/projector-internals.md).

Progress on an epic is weight, not counts (`aggregator/src/weights.ts`): `effort/xs` counts 1, `s` counts 4, `m` counts 12, `l` counts 40 as a floor. An unsized item contributes no weight and is counted visibly as unsized.

## Step 4: Read the item on the Kaneo board

The aggregator reads Kaneo read-only. It signs in through `/api/auth/sign-in/email`, reads `/api/workspace` and `/api/project`, and pages `/api/task/tasks/&lt;projectId&gt;` (`aggregator/src/sources/live.ts`, `kaneoBoard()`). A card becomes a board item with subject `kaneo:&lt;slug&gt;/&lt;taskId&gt;`. The deep link comes from `kaneoTaskUrl()` in `aggregator/src/service-links.ts`.

Kaneo columns map to canonical statuses in `aggregator/src/status-taxonomy.ts`: to-do, in-progress, in-flight, in-review, done. Board policy in `aggregator/src/board-policy.ts` filters retired "ROADMAP Done table" cards out of the board.

The board runs one repo per project, in the workspace "Developing Today": `code`, `opencode`, `openchamber`, `nix`, registered in `pm-stack/kaneo-projects.json`.

One correction stands over this step (D1): the deployed board is the upstream image `ghcr.io/usekaneo/kaneo:2.47.0`, pinned by digest in `pm-stack/compose.yml`. The fork at developing-today-forks/kaneo exists as a repo. Compose does not run it. Text in `aggregator/src/program.ts` that presents the fork as the deployed board is ahead of the deployment code.

## Step 5: Watch the sync move the item

Kaneo's own GitHub integration — the developing.today GitHub App — moves cards on status transitions. The transitions, per `pm-stack/KANEO-GITHUB-SYNC.md`:

- Branch push → task `in-progress`.
- Pull request open → task `in-review`.
- Pull request merge → task `in-review`.
- GitHub issue close → task `done`, through the built-in fallback.
- Reverse direction: a task reaching Done applies `status:done` labels and closes the GitHub issue.

The merge transition targets `in-review`, not `done`. That setting dates to 2026-11-14: the upstream default closed merely-referenced issues seconds after a merge.

The settings live only in Kaneo's Postgres, in `integration.config.statusTransitions` per project. No file in the repository sets them. The guard in Step 7 is the control that holds them.

No component in the repository carries the name "Kaneo PM bot". The bot-shaped actor in this step is the GitHub integration itself, acting on transitions, not on comments. A separate bot outside the repository is UNVERIFIED: no file under `pm-stack/` or `pkgs/pm-*` names one.

## Step 6: Follow the item into a session

Work on the item runs in a session. Two intake paths create sessions:

- Dispatch: `POST /v1/sessions` on the dispatch service creates the tracking GitHub issue with label `source/dispatch`, enqueues a queue task, creates an OpenCode session on the laptop server, and claims the task for `dispatch-opencode` with a 6-hour lease. A placement guard pins explicit model hints to the active account's quota pool before launch and fails before commit on a walled pool.
- The claimant: `pm-stack/opencode-claimant/claimant.py` fires in six steps — supervision and recovery of stuck bound sessions, the candidate gate through `poll.py` across four queues, the concurrency cap (36 claimant roots, from 2026-11-05), launch, bind, prompt. Fires log to `~/.local/state/opencode-claimant/fires.jsonl`. `launch_one.py` launches one task at once, per #8187.

Follow the session itself on `/sessions`, whose machine twin is `/api/registry.json`. The registry records who ran what, token counts, and task links. The console carries the same list at `#more/agents`.

Queue order is priority, then age: the board read in `console/src/work.ts` orders by priority ascending, nulls last, then creation time ascending. Priority bands and rank labels are separate records; the `/conflicts` page lists live disagreements between sources, with both values and the age of the disagreement.

## Step 7: Check the closure guards before calling the item done

Four guards stand between "looks done" and "done":

1. **Epic closure gate** — `evaluateEpicClosureGate` in `aggregator/src/epics.ts`, with `scripts/epic-closure-gate.ts`. An epic stays open while its complete subtree has open children or unmet conditions. The gate enumerates blockers by condition. `pm-stack/fleet-monitor/epic_close_check.py` monitors the gate report-only: a blocked epic is a finding, not a failure.
2. **Closure watchdog** — `pm-stack/closure-watchdog.py`. It polls issue-close events in the Kaneo-covered repos and reopens an automation close when the issue still has open sub-issues, or when the closing pull request carried no closing keyword. Keyword closes and human closes stay untouched. Its scheduler is a Muse sandbox cron job, outside the laptop stack, on a 15-minute cadence as recorded in `KANEO-GITHUB-SYNC.md`.
3. **Invariant sweeper** — `pm-stack/invariant-sweeper.py`. On a slow cadence it sweeps the four Kaneo-synced repos for: a closed issue with non-completed sub-issues; an open issue carrying `status:done`; a Kaneo↔GitHub state mismatch. Default mode reports only. `--act` repairs the first two classes under the watchdog's guards and still only reports the third.
4. **Kaneo sync guard** — `pm-stack/kaneo-sync-guard.py`. It asserts every GitHub integration's transitions (no target `done`; merge → in-review; open → in-review; push → in-progress), heals drift, and exits 1 loudly on a trip. `deploy.sh` runs it, and the `pm-kaneo-sync-guard` timer runs it hourly.

## You are done when

- The GitHub issue is closed by a keyword close or a human close, and no guard in Step 7 reopens it or reports it.
- The Kaneo card sits in the done column, and the aggregator board item carries the same state.
- An epic item passes the closure gate with no enumerated blockers.
- The session that did the work is in the registry on `/sessions`, linked to the item.

Sources. survey/apps.md §1, §7–§8, §19, §29, §31–§32; survey/roadmap.md §1–§3; survey/website-pages.md §A–§B; survey/disagreements.md D1, D10. Ground truth: laptop clone at 4d937b3, 2026-11-23.
