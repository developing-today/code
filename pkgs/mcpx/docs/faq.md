# FAQ

> The five questions a skeptical first-time reader asks first, in the order
> they ask them, answered in the same voice.

---

Short answers here; the long ones live in the pages each answer links to. Every
question keeps the wording it was written in.

<a id="do-you-like-mcp"></a>

## "so why'd you get rid of all the mcp and then make one, do you like mcp or not"

We like MCP fine. What we got rid of is tool schemas sitting in the model's
context, not MCP itself: mcpx is an MCP client of every server you configure,
across four protocol revisions, reading the same `mcpServers` config every
other host reads, and "mcpx puts every MCP server behind one command so their
schemas stay out of the model's context until something asks for them". The
server we made is deliberately tiny. "Exposing three hundred tools over MCP
would rebuild the problem mcpx was built to solve, with extra steps", so it is
`/mcp` on the daemon, `mcpx serve --transport stdio`, and `mcpx_exec` as the
one tool that matters.

More: [the story](./story.md).

<a id="opencode-code-mode"></a>

## "i mean, opencodemode is right there"

It is, and not only in v2. opencode v1 ships code mode behind
`OPENCODE_EXPERIMENTAL_CODE_MODE`, v2 has it on by default, and both accept
TypeScript. Its version is better confined: a purpose-built interpreter that
runs only the language features upstream implemented, plus OAuth and
elicitation for remote servers. What it does not do is what mcpx is for. It
only runs inside a model turn — there is no `opencode2 exec` — one daemon here
serves every harness with a real process per session for stateful servers, and
our scripts run on a real runtime with imports and a filesystem. Our own
recommendation still stands: let code mode own ordinary MCP inside opencode's
loop, and use mcpx for stateful servers and everything outside a turn.

More: [the opencode plugin](./opencode-plugin.md).

<a id="is-it-safe"></a>

## "is it safe?", no.

No. Scripts run on a real runtime with your authority — filesystem, network,
subprocesses. `script.permissions` defaults to `all`. What a narrower profile
does depends on the runtime, and the defaults say so exactly
(`internal/defaults/defaults.json`): on Deno the profiles become Deno
permission flags, and on node `readnet` / `read-net` are enforced with
`--permission --allow-fs-read=*`. `net`, `read`, and `strict` are defined for
Deno only. Ask for a named profile that has no entry for the runtime you
chose and mcpx refuses to run the script at all — "permission profile %q
defines no flags for %s, so %s cannot enforce it"
(`internal/runner/runtimes.go`) — it never runs the script unenforced and
pretends otherwise. Bun has no permission model, so no narrower profile is
defined for it and every narrower ask there is refused the same way. The
profiles that are defined for bun and node — `all`, `none`, `off`,
`unsandboxed` — carry an empty flag list by design: defined, and
deliberately unrestricted, not enforcement. The daemon API is
unauthenticated. It listens on loopback and on a socket only you can open, and
binding it wider prints a warning. `mcpx prompt` hands you a generated script
instead of running it, because generated code runs with your credentials. So
treat a script like a shell command you are about to paste: configure only
servers you would run by hand, and do not expose the port. That is the whole
answer.

More: [the script contract](../AGENTS.md).

<a id="why"></a>

## "but why would you.."

Three reasons, then the history. Context cost: tool schemas sit in every
request whether or not the tools get used, so mcpx's always-on cost is zero —
"there are 27 Chrome tools live on this machine and not one byte of their
schemas is in the agent's context". Concurrency: two agents sharing one browser
corrupt each other, so each session gets its own process. Reach: the same tools
from a shell, CI or any harness. The history is that this replaced lootbox, a
fork 51 commits ahead of a quiet upstream, where discovery was uncached (about
650 ms per request) and the architecture "cannot express the one thing you
actually need (more than one Chrome)".

More: [the assessment](../ASSESSMENT.md).

<a id="examples"></a>

## "but can i get some examples?"

Yes. `examples/` holds short scripts that CI runs on every change, against a
deterministic fixture server, paired with the output they print today — so an
example cannot quietly rot, and one of them is a real recipe (`mcpx recipes`).
The gallery page pairs each script with its command and its expected output;
copy one, change the server name, run it. They are deliberately small, and none
of them needs Chrome or the network. If one is wrong, that is a bug worth
reporting, because the only reason these are trustworthy is that they fail the
build when they stop being true.

More: [the examples gallery](./examples.md).
