# Delegation: subagents and independent sessions

Delegate to keep the coordinating session focused and run independent work in
parallel. This repository's [routing policy](routing-policy.md) owns model
selection: dispatch normal subsessions on `opencode-route/workhorse`, and launch
an escalation as a fresh session with its reason recorded. The personal pool
examples in [work-strategies.md](../../doc/platform/work-strategies.md#delegation-models-subagents-and-sessions)
are guidance for the Mac, not a replacement for that repository policy.

## Pick the size and vehicle

- **Inline:** work already understood and cheaper to finish than to brief.
  Small tasks are not automatically inline: a fast helper can beat a slow model
  doing a mechanical lookup or edit itself.
- **Subagent:** a bounded task whose result belongs to this task. Name one
  outcome, its read-only scope or owned files, the model/route, the directory,
  what it may decide, verification and the evidence to return.
- **Background subagent:** the same task while the parent does non-overlapping
  work. Launch independent pieces in one message. Completion arrives as a new
  turn; do not sleep, poll or duplicate it.
- **Separate session:** a substantial workstream with its own branch, worktree,
  context, lifetime and possibly its own fan-out. Check the available session
  tools first. The OpenChamber CLI is a fallback when its tool is absent.

A strong lead can supervise strong area leads that use cheaper implementation
and lookup helpers. A cheaper coordinator can instead supervise strong sessions
when it only tracks progress and routes decisions: the sessions own their
technical decisions and an appropriately capable agent integrates the result.
Neither shape is universally faster. Measure actual latency, rework and cost;
model rank alone does not determine the hierarchy.

## Tool mechanics

Read the current `subagent` tool schema; it is the API for this build. The
installed fork on 2026-11-24 accepts `agent`, `description`, `prompt`, `model`,
`background`, `stopPolicy` and `sessionID` (continue an existing child). Some
other builds always launch in the background. Use the behavior the tool reports,
not an older workflow's assumed foreground wait.

- A child starts in the parent's directory. Give a writing child its own
  worktree and state the directory in the brief; the directory in prose alone
  does not move it. See [parallel writers](../../doc/agents/parallel-writers-and-subsessions.md).
- A launch's explicit model wins over the child agent's configured model; only
  when neither exists does the parent model supply it. Name the route in the
  launch so intent is visible. A routed session's serving model may vary by
  step: verify its output rather than guessing which model answered.
- Only agent modes `subagent` or `all` are launchable through `subagent`.
  `general` and `explore` are common; use the actual offered list. Do not invent
  `plan`, `build`, `bootstrapper` or `probe` as subagent types from old docs.
- An error is a result: inspect it and correct the brief or execution path.
  Never let one failed child silently stall a whole workstream.

## Nesting and coordination

A subagent can launch its own children when its exposed tools, inherited policy
and remaining depth permit it. The limit is `experimental.subagent_depth`; the
Mac currently configures 10, and per-agent/session limits can be lower. Do not
use a new session to evade a permission or explicit no-delegation instruction.

Fan out for breadth or depth when ownership is partitioned. One writer owns
each branch and file set, shared interfaces are agreed before writing, and one
named integrator verifies the combined result. Readers may investigate competing
hypotheses about the same bug; competing writers fixing it in one checkout are
not parallelism. Every level checks its children's diffs and tests before
passing a claim upward.

## Cross-session messaging

Use OpenCode Code Mode (`execute`), not `mcpx_exec`:

```js
await tools.comms.msg({ to: "ses_<recipient>", text: "<a concrete question or interface change>", deliver: "wake" });
```

`tools.comms.topics` manages shared topics and subscriptions;
`tools.comms.sessions` reads session metadata/logs. Parent/child relationships do
not limit useful coordination: siblings can settle an interface directly. Topic
messages are read on demand unless subscribed; direct messages wake recipients,
so avoid acknowledgement loops.

The `mcpx_exec` description/discovery mistakenly advertises `comms` as available
there. The daemon has no such namespace; [mcpx.md](mcpx.md) gives the tested
runtime boundary and reproduction.

## Supervision and finishing

On a result, verify changed files and re-run the checks that support any claim
you will repeat. Continue an existing child by `sessionID` when its context is
still useful. If it is idle before its deliverable is done, inspect the last
answer and give a specific next action; if blocked, route the actual question to
its owner rather than sending an empty "continue" loop.

End a turn while notified background work runs; do not call the whole objective
finished with uncollected workers. Before publishing, name remaining work and
who owns it, and stop any development server or watcher you started unless an
explicit handoff keeps it running.

Detached or unattended sessions must not call `question`: no one may see the
form, so it can block the whole tree indefinitely. Record a reasonable default
and proceed inside the authorized scope, escalating only an action that truly
requires a person's credentials or hands. A session cannot change its own model;
launch a new session for work requiring a different one.
