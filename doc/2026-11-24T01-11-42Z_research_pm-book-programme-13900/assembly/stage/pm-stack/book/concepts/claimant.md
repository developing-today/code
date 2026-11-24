# The claimant: keeping sessions honest

## What the claimant is

The claimant is the OpenCode execution lane (`pm-stack/opencode-claimant/`, issue #3026). It takes queue tasks, launches OpenCode sessions for them on the laptop, binds each session to its task, and supervises the sessions it launched. Its name in this book is its job: a session that claims a task is held to that claim — supervised while it runs, recovered when it sticks, and capped so claims never exceed the concurrency the laptop carries.

## The fire: six steps

`claimant.py` fires in six steps:

1. **Supervision and recovery.** Bound sessions that are stuck are supervised and recovered first, before new work launches.
2. **Candidate gate.** `poll.py` polls across four queues and gates candidates for launch.
3. **Concurrency cap.** Launches stop at the cap: 36 claimant roots, in force since 2026-11-05.
4. **Launch.** A session is launched for the task.
5. **Bind and prompt.** The session is bound to its task and given its prompt (`prompt.md` in the claimant directory).
6. **Logging.** The fire is logged to `~/.local/state/opencode-claimant/fires.jsonl`.

The survey names the six-step shape and the step contents at this grain; the internal order of steps 4 and 5 inside a single launch is stated here as launch, then bind and prompt, following the survey's step list.

## The supporting tools

- `launch_one.py` launches one task immediately, outside the polling cycle (#8187).
- `dispatch_batch.py` dispatches a batch of tasks.
- `send_to_opencode.py` sends input to a running OpenCode session.
- `babysitter.py` and `nudges.py` carry the supervision side: watching bound sessions and nudging them. The survey names both files and their place in the directory; their internal rules are beyond the depth the survey supports, and this chapter claims no more than their presence and role by name.
- Migrations `migration-0021.sql` through `migration-0026.sql` carry the claimant's store changes.
- A static page, `index.html`, sits in the claimant directory. What it displays is UNVERIFIED: the survey records its existence and did not read its contents.

## How honesty is enforced

Three mechanisms in the surveyed code do the work this chapter's title names:

- **Recovery before novelty.** Step 1 runs before the candidate gate. A stuck bound session is recovered before its slot and its task are treated as free, so a task is not silently claimed twice by a replacement session while the first session still holds it.
- **The cap is a count of roots.** The concurrency cap counts claimant roots — 36 since 2026-11-05 — and the gate in step 2 admits candidates only under it. The cap is the bound that keeps claimed work equal to supervised work.
- **Every fire leaves a record.** Step 6 appends to `fires.jsonl`. A launch with no log line is outside the design; the log is the audit surface for what the claimant started.

Two adjacent guards sit outside the claimant and protect the same invariant from the GitHub side. The lane guard (`pm-stack/lane-guard/lane_guard.py`) makes deterministic ownership and park-note checks for queue lanes: a lane counts as held only on explicit markers (`held`, `user/owned`, `do-not-auto-edit`); assignees and `work/assigned` never count as held. Its `note` check suppresses repeat park notes. The guard was built after repo-sweeper's blanket assignment produced more than 50 duplicate park notes in two days (#7616). The claimant's cap step is also a named secondary caller of the health hooks: a cap-step failure fires a read-only health hook (see [Health, quota and usage surfaces](health-surfaces.md)).

## Place in the stack

The claimant is one consumer of the queue among several. The dispatch service claims its own launched task immediately for `dispatch-opencode` with a 6-hour lease; the claimant polls and claims under its cap. Both flows, and the queue they share, are described in [Queues and dispatch methods, from the PM side](dispatch-methods.md). Scheduled OpenCode sessions are a separate mechanism and are treated in [Scheduled jobs and the history they keep](scheduled-jobs.md).

The claimant directory carries a large test suite and a set of historical slice reports (`SLICE1_*.md` and `.txt`). The slice reports are history, not current specification. The component's last commit in the surveyed clone is 2026-11-23, and the survey marks it actively developed.

Sources. Survey `apps.md` §§18–19; survey `website-pages.md` §G; code at laptop clone `~/git/code` at `4d937b3` as cited inline.
