# Queues and dispatch methods, from the PM side

## Why this chapter exists

The fleet book owns the queue system: the broker, the task store, leases, and placement. This chapter describes the same machinery from the PM side: the points where the PM stack puts work into queues, takes work out of them, and shows queue state to Drew. Overlap with the fleet book is intended. Where the fleet book has the full treatment, this chapter links to it and does not duplicate it:

- The queue itself: [The queue: tasks, lanes and leases](https://developing-today.github.io/code/fleet/concepts/queue.html).
- The bridge: [The bridge: fleet messaging](https://developing-today.github.io/code/fleet/concepts/bridge.html) and its reference, [fleet bridge](https://developing-today.github.io/code/fleet/reference/bridge.html).
- The queue client: [The queue client](https://developing-today.github.io/code/fleet/library/queue-client.html).
- Dispatch from the fleet side: [Dispatch and dispatch-test](https://developing-today.github.io/code/fleet/concepts/dispatch.html) and [fleet dispatch](https://developing-today.github.io/code/fleet/reference/dispatch.html).

All five fleet-book URLs above were fetched live at assembly (2026-11-23) and serve the chapters named; the fleet book lives at `pkgs/fleet/book/` in the repo, with no `src/` folder. Voice spot-check against `pkgs/fleet/book/concepts/queue.md`: the fleet chapter's claim rule — the claim pick is atomic, highest priority first (`priority ASC, created_at ASC`) — is the same rule this chapter states as priority-then-age order in §1 below. This chapter covers only the PM stack's own dispatch surfaces; the fleet book carries the dispatch treatment from the fleet side.

## The queues the PM stack sees

From the PM side, "the queue" is four concrete surfaces:

1. **The fleet queue.** Tasks carry a numeric priority and are claimed in priority-then-age order. The band scale and the program-to-band mapping are recorded in [Prioritisation: how order is decided and recorded](prioritisation.md). The PM stack reads this queue in the aggregator's `/queue` page — in progress, blocked, recently completed, and next-up work across projects, with twins `/api/queue.json` and `/api/orchestration.json` — and in the console's Now, Next, and Queue views.
2. **The claimant's four queues.** The claimant's candidate gate polls across four queues before launching a session (see [The claimant: keeping sessions honest](claimant.md)). The survey names the count, four, and the polling file, `poll.py`; it does not name the four queues individually, and this chapter does not name them either.
3. **The projector's queue view.** The projector serves an HTML queue view of its link and projection state (`pm-stack/projector/src/queue-view.ts`, `queue-view-html.ts`, port 7801). Its route path is UNVERIFIED: the survey records the file names only.
4. **Intake packets.** Console intake and console prompts enter the queue as packets: `triage.enqueue` for new intake, `prompt.enqueue` once an issue exists, and a top-band packet for an owner prompt from the console. An intake submission creates the GitHub issue and the queue packet in one action.

## Dispatch method 1: pm-dispatch, launch-or-enqueue

The PM stack's own dispatch service is pm-dispatch (`pm-stack/dispatch/`, `pkgs/pm-dispatch/`): a Bun intake service on port 4879, public as `dispatch.security.cab`. Its single entry point is `POST /v1/sessions`, and the service is API-only; no web UI exists in `dispatch/src/`.

One call performs the launch-or-enqueue sequence:

1. Create the tracking GitHub issue, labelled `source/dispatch`.
2. Enqueue a `repo-task/v1` queue task.
3. Create an OpenCode session on the laptop server (127.0.0.1:13420).
4. Claim the task immediately for `dispatch-opencode`, with a 6-hour lease.

The service is authenticated. A placement guard (`src/placement-guard.ts`) runs before launch: it pins explicit OpenCode Go and ClinePass model hints to the active account's quota pool, and fails pre-commit when that pool is walled. A walled pool therefore stops a dispatch before any issue, task, or session exists. An example caller workflow ships in `dispatch/examples/dispatch-workflow.yml`.

## Dispatch method 2: the claimant, poll-and-launch

The claimant is the polling method. Where pm-dispatch launches in response to a call, the claimant polls its queues, gates candidates, applies its concurrency cap (36 claimant roots), and launches, binds, and prompts sessions itself. Its six-step fire, its immediate single-task path (`launch_one.py`), and its batch path (`dispatch_batch.py`) are treated in [The claimant: keeping sessions honest](claimant.md).

The contrast that matters from the PM side: a pm-dispatch task is claimed at creation, by the service, for a named claimant identity. A claimant task is claimed at poll time, by the lane, under the cap. Both claims land on the same queue system the fleet book describes.

## Dispatch method 3: ready-for-agent

ready-for-agent is the third method: an upstream dispatch UI with a GraphQL API on port 6056, registering the same four repositories as the Kaneo board, with merging set to OFF in its config — a human merges, always. Its registration, defaults, and aggregator adapter are treated in [The Kaneo sync and the other PM apps](kaneo-sync.md). The split of real dispatch traffic between ready-for-agent and pm-dispatch is UNVERIFIED.

## How the PM surfaces steer the queue

Dispatch from the PM side is not only launch. The console and dashboard steer already-queued work:

- **Console queue actions.** `POST /console/api/queue/\<task\>/\<action\>` carries the queue verbs the console exposes, claim and block and the park family among them, executed through the queue broker and audit-logged before execution.
- **Console board order.** The console board reads the queue in the priority-then-age order of §1 and shows counts, claimed, queued, and blocked-with-reason groups.
- **Dashboard actions.** A `session` action (poke, answer, stop) is consumed by the fast session-action consumer for `opencode-laptop`; a `triage` or `status-request` action is consumed by the orchestration pass. The action flow is treated in [Question sets and the answer-back flow](questions-and-answers.md) and [Prioritisation: how order is decided and recorded](prioritisation.md).
- **Prompt packets.** A console `#ask` submission files its issue and enqueues its packet at the top band, so an owner's own words enter the same order as every other task, at the front of it.

## What this chapter does not cover

Broker internals, lease mechanics beyond the dispatch lease stated above, placement rules beyond the pm-dispatch guard, and the transport that carries queue traffic are fleet-book subjects. This chapter links to that book at the head of the chapter and stops at the PM stack's own surfaces: the queue as displayed, the packets as enqueued, and the three dispatch methods as the PM stack runs them.

Sources. Survey `apps.md` §§4, 7, 8, 19; survey `roadmap.md` §§3–4; survey `website-pages.md` §§A19, E, G; coverage ledger (fleet book chapter paths for the queue, bridge, and queue-client links); code at laptop clone `~/git/code` at `4d937b3` as cited inline.
