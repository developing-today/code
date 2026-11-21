---
name: next-work
description: "Choose the next work in developing-today/code from the results-first plan, and decide whether a task in hand produces a result or only motion. Use at the start of a lane shift or session when picking id or fleet work, when a claimed task points at an effort/l or effort/m issue, before filing an issue, design record or follow-up, before posting a triage, park or status note, and when a track's exit test changes state and the plan's Progress needs a line."
---

# Next work

The plan of record is [`planning/plans/2026-11-21_results-first.md`](../../../planning/plans/2026-11-21_results-first.md). It names the few tracks that are active, the test that says each one is finished, and what waits. This skill is how a worker applies it. It adds no queue rule: claiming, holds and parks stay as [`AGENTS.md`](../../../AGENTS.md) and [`doc/agents/one-queue-claiming-work.md`](../../../doc/agents/one-queue-claiming-work.md) say.

A result is one of three things: a merged change that makes an unchecked acceptance criterion pass, a measured number that moves toward an exit test, or a recorded decision that unblocks one. Everything else is motion, however careful.

## Choosing work

Read the plan's **Now** table first. Then take the first of these that exists:

1. **Breakage on main in an active track's code.** A red `id` suite outranks every feature slice, because every later verdict reads that signal.
2. **The next slice of an active track.** Follow [`feature-slice`](../feature-slice/SKILL.md). The track's row names its exit test and its open issues.
3. **Work that was parked on the owner.** An issue carrying `user/escalated`, or blocked "for Drew", is ready work: decide it and continue ([`decide-and-proceed`](../decide-and-proceed/SKILL.md)).
4. **Anything else the queue hands you.** Do it as the packet says.

Work outside the active tracks is not forbidden. It is not chosen while an active track has an unclaimed slice.

## Before you post, file or park

Run these checks in order. Each one names what to do instead.

| About to                                                     | Check                                                                                                                      | Instead                                                                                                                                                                        |
| ------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Post a triage or "fresh check" note                          | Does the newest agent note on the issue already give the same verdict, with no commit, label, body or human comment since? | Post nothing. Complete the task and put the verdict in the queue checkpoint. A repeated verdict is not progress.                                                               |
| Leave an `effort/l` or `effort/m` issue "triaged, left open" | Is there an unchecked acceptance criterion a single PR could close or measurably move?                                     | Land that slice ([`feature-slice`](../feature-slice/SKILL.md)). If a choice stands in the way, make it ([`decide-and-proceed`](../decide-and-proceed/SKILL.md)).               |
| Return a packet as `packet_defect` because the issue is big  | Is the defect only that the packet says `small_fix` and the issue is large?                                                | Re-scope the packet yourself to the next slice and record that in the checkpoint. A slice is a bounded fix.                                                                    |
| Apply `user/escalated` or park for the owner                 | Is it a hands item: a physical action, an interactive login, a payment or a signature?                                     | If not, it is a decision and it is yours: make it, record it, continue ([`decide-and-proceed`](../decide-and-proceed/SKILL.md)). A hands item blocks one step, never the lane. |
| File a new issue for something you found                     | Can you fix it in this PR or the next one?                                                                                 | Fix it. File only what is real, reproduced and out of reach, and enqueue it in the same pass as the rulebook requires.                                                         |
| Write a design record                                        | Does an active track need this decision to land its next slice?                                                            | If not, add one row to the plan's **Later** table with the trigger that would start it. A design nobody will build this month goes stale before it is read.                    |
| Add a tracker, register, census or board                     | Does a test or a scheduled job regenerate it?                                                                              | If not, do not add it. [`doc/platform/work-strategies.md`](../../../doc/platform/work-strategies.md) records what hand-kept status documents cost here.                        |

## Updating the plan

The plan is edited in place, by PR, and only for events:

- A step's done check passes: tick the step and add one dated line to **Progress** with the PR.
- A track's exit test changes state (a criterion passes, a measurement moves, a release is tagged): update the track's row and add one Progress line with the number and the command that produced it.
- An item moves between Now, Next and Later: move its row and say why in the Progress line.

Do not append shift reports, triage verdicts or "still open" notes to the plan. Do not copy issue content into it: link the issue.

Re-measure before changing the **Scoreboard**: run [`scripts/fleet/flow-report`](../../../scripts/fleet/flow-report/README.md) and paste its numbers with the date. A scoreboard row that was not re-measured keeps its old date.

## Turn report

End a turn that used this skill with:

- Result: <merged PR, measurement or recorded decision, with link; or "none">
- Track and exit test: <which Now row this moved, and the test's state now>
- Not done, and why: <what stopped; "nothing" if the slice landed>
- Posted: <notes posted, or "none: verdict unchanged">
