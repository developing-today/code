# Subagent-First Workflow

**Your default approach should be to use subagents.** Delegate work to subagents with the `subagent` tool unless you have a specific reason not to. Subagents run autonomously with their own context windows, enabling parallel execution and keeping the main conversation focused. Reaching for a subagent should be your first instinct — only skip it when the task is truly trivial.

Mechanics in this file are production-verified (workshop test, 2026-11-06, developing-today-forks/opencode#1881). Where older text disagreed with the test, the test wins.

## The `subagent` tool

Launch shape: `subagent` with `agent` (type), `description`, `prompt`, optional `background`, optional `model`, optional `sessionID` (continue an existing subsession instead of launching cold).

- **Foreground (default):** you wait; the child's final output arrives inline as the tool result.
- **Background (`background: true`):** you are notified by an automatic turn when the child finishes — do not sleep, poll, or duplicate its work meanwhile.
- **Errors are results:** a child that errors reports the error verbatim as its output. Read it, re-spec or reroute, and continue — never let one failed child stall the work.
- **Continuation:** pass a finished child's `sessionID` to follow up in its existing context instead of relaunching cold.

## Inheritance — state it anyway

- A child lands in **your directory** and, when no `model` is passed, **inherits your session's model/route ref**. State the intended route/head and directory explicitly in every launch even though inheritance is automatic, so the record shows intent.
- Your route ref is your identity; the **serving leaf varies per step** under one route (a pool is a pool — workhorse steps in the workshop test were served by different leaves within single turns, and prompt policy differs by leaf). Judge subsessions by their output, never by which leaf you assume served them.

## Subagent types

Only agents whose mode permits it can run as subagents (mode `all` or `subagent`). A `primary`-mode agent is rejected by the tool ("cannot run as a subagent") — `plan` is such an agent: use it as a session head, never as a subagent type.

| Type | Use for |
|---|---|
| `explore` | Fast codebase search: find files by pattern, search code for keywords, answer structural questions. Specify thoroughness: "quick", "medium", or "very thorough". |
| `general` | General-purpose agent for researching complex questions and executing multi-step tasks in parallel. Can use all tools. Use for implementation, bug fixing, refactoring, running commands. |
| `build` | Full development agent with all tools enabled. Use for substantial implementation work, file operations, and system commands. |
| `bootstrapper` | Analyzes a request and creates exploration branches with scopes. Use when planning or decomposing a complex task into parallel exploration paths. |
| `probe` | Evaluates branch Q&A and decides whether to ask more or complete. Use for iterative refinement of answers and deep-dive investigations. |

Other types may be available depending on the session and plugins. Check what's offered before assuming only these exist.

## Nesting

A subagent may itself launch subagents. Depth cap: `experimental.subagent_depth`, **default 5**. Use nesting for sub-decomposition (a child splitting its spec into grandchildren), not as a default shape.

## Cross-session messaging

Running sessions can message each other through mcpx **code mode**, namespace `comms`: discover it (`mcpx_discover`, namespace `comms`), then `tools.comms.msg` — send with `deliver: "wake"` to wake the recipient, receive by blocking on `mode: "unread"` with a `wait`. That is a complete request/response loop between two live sessions. Messaging is not a plain tool call; if discovery fails once, retry it.

## Model switching — there is none in-session

A session **cannot change its own model**. No tool exposes it (an improvised attempt reaches for the wrong API and fails). The launcher can switch a session from outside via the API, but a switch invalidates the prompt cache for the new model — so launch the right head in the first place, and if part of the work needs a different head, launch a subsession on that head.

## ⚠️ Detached operation — the `question` tool stalls the tree

In detached work (no human watching the session live), **never call the `question` tool** — and never instruct a subsession to. Verified failure: a child's `question` call blocked the child *and* its foreground parent for 10+ minutes with **no notification anywhere** (the form waits on the child session for an external actor; the parent is not shown it). When the call eventually errors, the retry machinery can resume both sessions and **re-ask the identical question in a loop**.

Instead: record every question in the designated place (the task's progress, the issue, the session's notes) **with the default you chose**, and keep working. A watcher or the parent answers recorded questions asynchronously.

## When to use subagents

**Use subagents whenever it makes sense — which is most of the time:**

- **Codebase exploration** (`explore`): Finding files, searching for patterns, understanding code structure, answering questions about the codebase. Use before making changes to gather context.
- **Multi-step research or implementation** (`general`): Anything requiring multiple tool calls that can run independently — refactoring across files, running tests, investigating bugs, building features.
- **Parallel workstreams**: When multiple independent tasks exist, launch them all as subagents simultaneously rather than doing them sequentially.
- **Any task you're unsure about**: If you're debating whether to use a subagent, use one.

## Guidelines

1. **Subagent first.** Your default action for any non-trivial task should be launching a subagent. Only do work inline if it's a single read, grep, or edit you already have full context for.
2. **Explore before editing.** Before modifying code, launch an `explore` subagent to understand the current state — related files, callers, dependencies, conventions.
3. **Parallelize aggressively.** If you need to search multiple things or do independent tasks, launch multiple subagents in a single message.
4. **Be specific in prompts.** Tell the subagent exactly what to do, what to return, and whether it should write code or just research. Include file paths and function names you already know. State the route/head and the directory explicitly.
5. **Trust but verify.** Read what a subagent actually *did* — files, diffs, records — not just its final message, for anything that writes or anything surprising. A final message is enough for pure lookups and marker results.
6. **Summarize results.** When a subagent returns, relay the key findings to the user concisely — don't just silently consume the output.
7. **Chain when needed.** Use subagent results to inform the next step. An explore subagent's findings should feed into a general subagent's implementation task.

## Only skip subagents for

- Reading a single known file (use Read directly)
- Simple glob/grep on a known path (use Glob/Grep directly)
- Trivial one-shot edits where you already have full context
