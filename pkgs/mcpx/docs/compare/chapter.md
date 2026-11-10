# The comparison chapter

```
created:      2026-10-29T19:20:00-05:00
last-updated: 2026-10-29T19:20:00-05:00
increment:    1
status:       standard
tags:         area:compare
description:  mcpx next to lootbox, opencode code mode, Cloudflare Code Mode,
              Anthropic programmatic tool calling, mcpac, mcpc and
              code-mode-mcp as one table plus prose, every row stamped with
              what it was checked against, including where the others are
              better.
```

This is the presentation the compendium asks for: one table, then one section
per neighbour, honest about where they are better. The details live in the
[register](register/code-mode.md) and the [matrices](matrix-code-mode.md); this
page is the reading copy, and every row says what it was checked against so a
stale row is visible as a stale row.

**How to read a row.** The mcpx cell carries `mcpx @ 5a34fa70` — checked
against the code at that commit on 2026-10-29. The other cells carry the date of
the last check recorded in the register, `2026-09-30`, or the date the upstream
documentation was fetched. A row whose stamp is older than your copy of the
other project may be out of date: re-check it, do not assume it.

## The table

| | mcpx `@ 5a34fa70` | lootbox (deployed `587a5a1`+4 patches) | opencode code mode (v1 1.18.31 / v2 2.0.3) | Cloudflare Code Mode | Anthropic programmatic tool calling | `sh1nduu/mcpac` | `apify/mcpc` | `tmustier/code-mode-mcp` |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| **Shape** | CLI + daemon, generated TypeScript client, MCP server mode | Deno daemon + CLI, launchd singleton on `:9420` | a feature inside opencode's agent loop (v1 behind `OPENCODE_EXPERIMENTAL_CODE_MODE`, v2 on by default) | a feature of the Agents SDK on Workers | a hosted API feature of Claude's tool use | CLI: generate TS types, execute | CLI MCP client | stdio MCP server exposing one `exec` tool |
| **Who invokes** | shell: any agent, script, human, CI | shell / HTTP | the model, via `execute` | the model, via one tool | Claude, via `code_execution` | shell | shell | the model |
| **Runtime** | real deno / bun / node subprocess | Deno subprocess | purpose-built JS interpreter in-process (v1: no classes or generators; v2: generators, no classes) | V8 isolate via the Worker Loader API | Python in Anthropic's container | Bun | none (shell + `--json`) | Node |
| **Confinement** | *not* "none": `script.permissions` defaults to `all` and has profiles (`net`, `read`, `read-net`, `strict`), taking effect under Deno — an audited, opt-in boundary | Deno permission flags; filesystem and subprocess off, network open by default | strong: only implemented language features exist; no filesystem, no network | real isolation: isolate, `fetch()`/`connect()` throw, bindings hide credentials | managed sandbox, zero infrastructure | declared + granted capability lists, checked against the script's declaration | n/a | none: "same authority as the Node.js process" |
| **Discovery** | `ls` / `search` / `types` outside the context, plus in-script `search()` / `describe()` | recomputed on every request — the bug that started all this | budgeted catalog + in-program `search()`, diff updates | whole generated API in context; the post calls search a "future improvement" | tool search is a separate platform feature | CLI exploration | `grep`, `tools-list`, `tools-get` | `search()` / `describe()` in the program |
| **Stateful server isolation** | `shared` / `pooled` / `session` pools, one process per run | none: one shared client, one Chrome | one connection per server per workspace; `_meta` sessionID hint | per agent | n/a | not described | persistent named sessions | in-memory JSON session state |
| **Outside a model turn** | yes | yes | no | no | no | yes | yes | only via an MCP client |
| **Where it is better** | — | nothing we still need | confinement by construction; the catalog budgeting mcpx copied | real isolation; credentials never reach the code | zero infrastructure; `allowed_callers` | capability grants checked against a declaration | the widest protocol coverage on a CLI, including OAuth 2.1 | OAuth, sampling and elicitation as a drop-in MCP server |

Stamps: mcpx column and the confinement correction at `5a34fa70` (2026-10-29),
from [`register/code-mode.md`](register/code-mode.md) and
[`matrix-code-mode.md`](matrix-code-mode.md); the opencode, lootbox and
Cloudflare columns as recorded in the same matrix (checked 2026-09-30 against
opencode 1.18.31 and 2.0.3, lootbox `587a5a1`+4 patches, and Cloudflare's
documentation); the three search-found projects from their READMEs as fetched
that day; Anthropic from
<https://www.anthropic.com/engineering/code-execution-with-mcp> (2025-11-04)
and <https://platform.claude.com/docs/en/agents-and-tools/tool-use/programmatic-tool-calling>.

The earlier seed table in #1869 said mcpx's confinement is "none, by choice".
That was wrong when it was written: `script.permissions` exists with four
profiles and is enforced under Deno
(`internal/settings/registry.go:183`, [`../container.md`](../container.md) §2).
The default is still `all`, which is a stance, not an accident — see the
divergence below.

## The pattern, and where mcpx diverges from it

The MCP client best-practices guide
(<https://modelcontextprotocol.io/docs/2026-07-28/develop/clients/client-best-practices>)
is not a product here: it is the pattern's specification — progressive
discovery (catalog, inspect, execute) plus programmatic tool calling — and it
says the pattern "requires clients to implement a sandbox environment" with no
direct network access. mcpx deliberately has none; `ASSESSMENT.md` records the
instruction as "I dropped the sandbox, as you asked". That divergence is stated
here rather than buried: mcpx is a tool for an owner who wants a script to have
the machine's authority, and every neighbour above with real confinement is
making a different trade.

## The neighbours, one section each

### lootbox

The predecessor. Same shape (daemon + CLI + generated types), same purpose, and
the reason mcpx exists. Its discovery recomputed every request, it was killed by
the OOM reaper under load, its artifact was 158 MB, and one Chrome served every
agent — the last is architectural, not a bug to fix. Nothing it did is still
missed. Checked 2026-09-30; it has had no upstream commit since 2026-02-27.

### opencode code mode

The one that nearly made mcpx pointless, and the one mcpx learned from. Where it
is better: confinement comes free, because a purpose-built interpreter simply
does not implement `fetch`, `Deno.run` or a filesystem; and the catalog budget
with in-program `search()` was the model mcpx copied. Where it is not: it only
exists inside a model turn, and it keeps one MCP connection per server per
workspace, so concurrent Chrome work is still shared. The 2026-09-29 correction
means v1 has code mode too — the earlier claim that v1 users had nothing is
wrong. Checked against opencode 1.18.31 and 2.0.3 (2026-09-30).

### Cloudflare Code Mode

Where it is better, plainly: real isolation with a per-pass V8 isolate, network
blocked or routed through a host `Fetcher`, and credentials that never reach the
code. It also has CPU, subrequest, output and durable-value limits that mcpx
does not have any of. Where it is not: it is a hosted product feature, not
something a human or a CI job can call from a shell. Checked against
Cloudflare's Code Mode documentation (2026-09-30); the original argument is the
2025-09-26 post, <https://blog.cloudflare.com/code-mode/>.

### Anthropic programmatic tool calling

The pattern with zero infrastructure: the model writes Python in a managed
sandbox and calls tools as functions. Better at exactly that. Not comparable as
a tool — there is no CLI, no daemon and nothing to run outside a model turn, and
its own documentation says tools "provided by an MCP connector" cannot be called
programmatically, which is the gap mcpx fills. Checked 2026-09-30.

### `sh1nduu/mcpac`

A Bun CLI that generates TypeScript types and executes. Better at capability
grants: the script declares what it needs and the grant list is checked against
that declaration, which is a more interesting model than mcpx's all-or-nothing
default. Checked against its README 2026-09-30.

### `apify/mcpc`

A CLI MCP client with the widest protocol coverage of the group on a command
line, including OAuth 2.1, plus persistent named sessions. Better at protocol
coverage. It does not run code, so it is a complement rather than a rival: it is
also where to look for the longest maintained table of MCP CLI clients, which is
why this page links it instead of copying it. Checked against its README
2026-09-30.

### `tmustier/code-mode-mcp`

The inverse shape: a stdio MCP server that exposes one `exec` tool, so the model
gets code mode through the protocol rather than through a harness. Better at
being a drop-in — OAuth, sampling and elicitation come with being a server.
Worse at isolation: it says the script has "same authority as the Node.js
process". Checked against its README 2026-09-30.

## Re-checking

Rows are re-checked when the compared project moves or when mcpx does, and a
re-check is stamped with the commit or date it used. The mcpx column has the
same problem the register's `mcpx @` column has: it is a snapshot, and the code
moves faster than this page. When a stamp here is older than the register's, the
register is right.

## Related

- [`history.md`](history.md): how mcpx got here, including the correction.
- [`README.md`](README.md): the register, the matrices, the citation forms.
- [`../../ASSESSMENT.md`](../../ASSESSMENT.md),
  [`../../OPENCODE-V2.md`](../../OPENCODE-V2.md): the two frozen records.
