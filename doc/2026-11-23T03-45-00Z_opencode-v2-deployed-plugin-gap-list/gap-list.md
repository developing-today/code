# OpenCode v2 — deployed `.opencode/` plugin gap list (#730, criterion 2)

Basis: `developing-today/code` main `1d6d8f07d427fe919918b45f857ef57b894b542a`
(2026-11-22). The v2 contract cited is `@opencode/plugin` 2.3.0, whose packed
types were read in the #3286 survey (issue #3286, comment 6916287566): v2 is
`{ id, setup(context): Cleanup }` with domain APIs (`context.session`,
`.tool`, `.event`, …), no v1 hook map, and **no `tool()` helper**. The
v1→v2 mapping design is #1858; the landed v2 core it produced lives in
`pkgs/mcpx/plugin/opencode/v2/` (PR #2489) and is the source of the
"equivalent exists" verdicts below.

Scope: the **deployed** copy only — `.opencode/plugin/mcpx-session.ts`
(783 lines) plus its backing modules under `.opencode/plugin/core/` and
`.opencode/plugin/mcpx/`, and `.opencode/plugins/autoresearch-context.ts`.
The shipped package (`pkgs/mcpx/plugin/opencode/`) is #1858/#2841's set and
is out of scope here; #2841 assigns the deployed-copy port to #730.

## Deployed `mcpx-session.ts` — every v1 hook / SDK call

| v1 surface (line on main) | v2 status | Chosen workaround / equivalent |
| --- | --- | --- |
| `import { tool, type Plugin } from "@opencode-ai/plugin"` (line 1) | **No equivalent** — the v2 package is `@opencode/plugin`; there is no `tool()` helper and no `Plugin` hook-map type | Port to `define()` + domain APIs, as in `pkgs/mcpx/plugin/opencode/v2/index.ts` |
| 6 tools built with `tool({...})`: `mcpx_daemon_status` (436), `mcpx_daemon_select` (459), `mcpx_daemon_forget` (525), `mcpx_discover` (541), `mcpx_exec` (563), `mcpx_observe` (599), exposed via the returned `tool:` map (754) | **No `tool()` helper** in v2 | Register via `tool.transform(e => e.add(...))` under a namespace, as in `v2/tools.ts` + `v2/tool-filter.ts` |
| `"shell.env"` hook (635) — injects the session's mcpx environment into every shell command | **No equivalent shape** — v2 `shell.create.before` carries **no sessionID**, so per-command injection cannot know its session | Set the environment once per session: `PUT /api/experimental/session/:id/environment`, as in `v2/session-env.ts` (#1858 table) |
| `"experimental.chat.system.transform"` hook (723) — rewrites the system prompt per turn | **No direct equivalent** — v2 `Context` exposes no chat-transform hook (#3286 survey) | `session.hook("context")` → `event.system`, as in `v2/context.ts` (#1858 table) |
| `"tool.execute.after"` hook (765) — logs tool outcomes to mcpx's store | Equivalent, better: v2's tool-result surface is a union on `status` | Record both outcomes, not just success (#1858 table) |
| `client.session.get` (360) — fetch + cache session metadata once per session | Equivalent in domain form: `context.session` | Use the setup context's session domain instead of the v1 client |
| `client.tui.showToast` (106, 259) — user-visible daemon toasts | **No verified 1:1 equivalent** in the v2 server-plugin contract; the TUI surface moved to the `@opencode/plugin/tui` entry point with a different type surface (#3286 survey) | Drive user prompts through the v2 TUI surface (`v2/picker.ts` pattern); toasts degrade to log-only until that port lands |
| Remembered-daemon state file (via `.opencode/plugin/core/`) | Equivalent: `ctx.storage` | `v2/remember.ts` (#1858 table; lifetime rule is #87's) |

## `autoresearch-context.ts` — every v1 hook / SDK call

| v1 surface (line on main) | v2 status | Chosen workaround / equivalent |
| --- | --- | --- |
| `import type { Plugin } from "@opencode-ai/plugin"` (line 1) | **No equivalent** (as above) | `define()` under `@opencode/plugin` |
| `"experimental.chat.system.transform"` hook (26) — its only hook | **No direct equivalent** (as above) | `session.hook("context")` → `event.system` (`v2/context.ts`) |

## The gaps, named (criterion 2's ask)

Exactly three v1 constructs used by the deployed plugins have **no v2
equivalent**, each with its chosen workaround:

1. **The `tool()` helper / `Plugin` hook-map contract itself** — v2 replaces
   it wholesale with `{ id, setup }` + `define()` and domain APIs. Workaround:
   adopt the landed v2 core's shape (`v2/index.ts`, `v2/tools.ts`).
2. **`experimental.chat.system.transform`** — no chat-transform hook exists
   in the v2 `Context`. Workaround: `session.hook("context")` → `event.system`
   (`v2/context.ts`), for both deployed files that use it.
3. **Per-command `shell.env` injection keyed by session** — v2's
   `shell.create.before` has no sessionID. Workaround: one
   `PUT /api/experimental/session/:id/environment` per session
   (`v2/session-env.ts`).

One further call has no *verified* 1:1 equivalent and a degrade-path
workaround rather than a port target: `client.tui.showToast`
(TUI entry-point surface differs; toasts degrade to log-only).

Everything else the deployed plugins call has a v2 equivalent, most of them
already implemented in the shipped v2 core — the deployed-copy port is an
adoption of that core plus the three workarounds above, not new design.

## What this does not do

The port itself. Sequencing stays the #3286 survey's resume plan: decide
the transform replacement (done above: `session.hook("context")`), port
`mcpx-session.ts`, port the TUI surface, then move the
`.opencode/package.json` pin to the v2 package and regen the lock — which
is also what unblocks #730 criterion 5's v2-era pin.
