# Research and incidental findings

```
created:      2026-11-01T01:30:00-05:00
last-updated: 2026-11-01T01:30:00-05:00
increment:    1
status:       draft
tags:         area:docs, area:compendium, topic:research
description:  what was learned while building mcpx, and while doing anything
              else in this repository, collected by topic and tagged with its
              provenance so it is not lost in session transcripts. First slice
              of #1871: the section outline, the seed findings that already
              exist in the repository, and the harvesting process.
```

This chapter is the compendium's research chapter (#1871). It holds research
and incidental findings — things learned while building mcpx, and while doing
anything else in this repository — filed by topic so they survive the sessions
that produced them. Findings that live inside feature entries stay there as
the source of truth; this chapter excerpts them with a citation (the answer
to open question 5 on #1871, applied: excerpt, never copy-and-fork).

Every finding here carries a provenance tag, per the quality bar below:

- **measured** — a number or behaviour produced by running something, with
  the environment named in the source.
- **read-from-source** — read in code, docs, or a specification in this
  repository.
- **reported** — someone else's claim, cited: an external article, another
  project's documentation, or a statement in an issue. Reported findings are
  not verified here and must not read like measurements.

Sections with no seed yet are outline only: the heading exists so the next
harvest has somewhere to put things, and says so rather than padding.

## AI and LLMs

- **The cost model of zero-context discovery.** Listing servers costs ~171
  tokens, a search ~61, and `types` 675–4,402 per namespace — the numbers
  behind progressive discovery: a client should not pay for every tool
  definition up front. *(measured — `docs/story.md`, "Zero-context
  discovery")*
- **Switch to progressive discovery at 1–5% of the context window**, and do
  not mutate the `tools` array mid-conversation: it invalidates the
  provider's prompt cache. *(reported — modelcontextprotocol.io, MCP client
  best practices, as cited on #1871)*
- **Models write code better than they call tools**, Cloudflare argues,
  because tool-call tokens are synthetic training data while code is what
  models were trained on. This is the argument behind Code Mode. *(reported —
  blog.cloudflare.com/code-mode, as cited on #1871)*

## Harnesses

- **What is opencode's and what is general.** `docs/opencode-plugin.md`
  separates the opencode plugin surface (v1 vs v2) from the parts that apply
  to any harness; `OPENCODE-V2.md` corrects the record where the first draft
  blurred them — opencode v1 ships code mode behind
  `OPENCODE_EXPERIMENTAL_CODE_MODE`. *(read-from-source —
  `docs/opencode-plugin.md`, `OPENCODE-V2.md`)*
- **The harness knows what the agent does not.** Session state, statistics,
  and affordances live in the harness; an agent that assumes them instead of
  asking gets them wrong. The opencode session statistics are the worked
  example. *(read-from-source — `docs/story.md`, "The harness knows what the
  agent does not" and "Statistics from opencode")*
- **`AGENTS.md` is the cross-harness instruction file**: the same file is
  read by the harnesses this repository is used from, which is why fleet
  rules live there and not in one tool's config. *(read-from-source —
  `AGENTS.md` at the repository root and in `pkgs/mcpx`)*
- **A harness-free core is a design constraint, not an accident** (#48):
  codex, claude code, pi, and manus must all be able to drive the same core.
  *(read-from-source — `docs/story.md`; issue #48)*

## Plugins

<!-- id: research-plugins since: unreleased -->

- **A plugin can connect without the binary.** The connection ladder and
  `GET /v1/resolve` let the opencode plugin find or start a daemon instead
  of assuming one is installed (#44, fixed in #53). *(read-from-source —
  `docs/plugin-discovery.md`)*
- **The plugin logs over the socket** (#36), so plugin diagnostics land
  where the daemon's do instead of in a file nobody opens.
  *(read-from-source — `docs/plugin-discovery.md`; issue #36)*
- **Plugin install is a README, not a package step.** *(read-from-source —
  `plugin/opencode/README.md`)*

## Token providers

The outline assumes both readings of the term (open question 1 on #1871):
model providers, and the credential/OAuth providers that hand out tokens.

- **Model providers are maintained as patches.** This repository patches
  opencode for provider behaviour (Copilot business support, Copilot
  compaction, OpenAI response-id caching) and keeps skills for maintaining
  those patches across opencode updates. *(read-from-source — #1871 seed
  list. Harvest note, 2026-11-01: the three patch files the issue names are
  no longer at the repository root on `main`; the patches' current home
  needs naming here by whoever next touches the opencode patch set.)*
- **Credential placeholders keep secrets out of config files and descriptions, but
  stdio servers receive them through the environment. Generated scripts inherit
  that environment unless variables are unset or the run uses `-i`; do not
  treat this as isolation from code you execute. *(read-from-source —
  `docs/story.md`, "Credentials"; `internal/runner/dispatch.go:693–705`)*

## Cloudflare

- **Code Mode mechanics.** The Worker Loader API runs generated code in
  isolates; `fetch()`/`connect()` throw inside it; bindings hide API keys
  from the generated code. Shared with #153 — the design treatment lives
  with the design, this is the finding. *(read-from-source — `docs/story.md`;
  issue #153)*

## The TypeScript ecosystem

- **Node runs TypeScript by stripping types, and only erasable syntax
  survives.** Node v24.20.0 type stripping rejects a file with an `enum`;
  constructor parameter properties failed too, which is why the generated
  client uses plain fields. *(read-from-source — `ASSESSMENT.md` bug 3;
  `docs/story.md`, "Runtime portability")*
- **Runtime detection order is deno, then bun, then node** (`script.runtime`,
  `--runtime`, `MCPX_SCRIPT_RUNTIME`). Versions at the time of the seed:
  Deno 2.9.6, Bun 1.3.11. *(read-from-source — `docs/story.md`, "Runtime
  portability"; the runtime comparison itself belongs to #101 and
  `docs/runtimes.md`)*
- **`--typecheck` is `deno check`, Deno only**: ~80 ms on repeat, ~400 ms
  for a new program (#79). *(measured — as recorded on #1871; `docs/story.md`)*
- **Constructing a TypeScript compiler per request was lootbox's memory
  problem** — the `ts-morph` lesson behind the current design.
  *(read-from-source — `ASSESSMENT.md`)*

## SSE

- **Server-sent events, not WebSockets.** SSE is plain HTTP, `EventSource`
  reconnects on its own, and WebSocket upgrades are mishandled by
  middleboxes — the reasoning behind mcpx's remote transport.
  *(read-from-source — `docs/story.md`, "Server-sent events, not
  WebSockets")*
- **Lossless reconnection** is a protocol property, not a client hope: the
  stream carries what a reconnecting client needs to resume without a gap.
  *(read-from-source — `docs/story.md`, "Lossless reconnection")*

## Sockets and TCP

- **What each transport costs.** Spawning `mcpx status` took 23.12 ms; a
  unix-socket keep-alive round trip 0.17 ms (135× cheaper); TCP loopback
  keep-alive 0.65 ms. The daemon-and-socket design is priced by these
  numbers. *(measured — `docs/story.md`, "What each transport costs")*
- **macOS `sun_path` is 104 bytes.** Deep state directories produced socket
  paths that failed with `bind: invalid argument`. *(read-from-source —
  `ASSESSMENT.md` bug 4)*
- **`localhost` is not `127.0.0.1`.** It resolved to IPv6 only while the
  compiled build bound `127.0.0.1`, so the client dialled an address
  nothing listened on. *(read-from-source — `ASSESSMENT.md`)*

## RPC and WebSockets

- **JSON-RPC framing needs no dependency** — it is standard library, which
  is why it appears in the dependency audit as a deliberate non-dependency.
  *(read-from-source — `docs/dependencies.md`)*
- **Lootbox's WebSocket.** The predecessor system used a WebSocket RPC
  channel (`lootbox.config.json`: `serverUrl: ws://127.0.0.1:9420/ws`) —
  the contrast case for the SSE choice above. *(read-from-source — #1871
  seed list)*

## MCP

<!-- id: research-mcp since: unreleased -->

- **Two protocol eras.** The protocol has a before and an after, and a
  client has to speak to both; "What 2026-07-28 actually asks of a client"
  is the checklist. *(read-from-source — `docs/story.md`, "Two protocol
  eras")*
- **The revision matrix** — every revision in both directions, plus what is
  still missing — lives in `docs/protocol.md` and is excerpted, not copied,
  here. *(read-from-source — `docs/protocol.md`)*
- **The sampling bug: it did not work, and nothing said so.** Capabilities
  are declared in the handshake, but the handler was installed after it,
  and every test called the passthrough directly — so sampling was
  advertised, broken, and green in CI at the same time.
  *(read-from-source — `docs/story.md`, "Sampling")*
- **The elicitation bug came first** — the same shape, earlier.
  *(read-from-source — `docs/story.md`, "Elicitation"; `docs/elicitation.md`)*
- **`mcpx doctor` found a bug immediately** — a diagnostics command earns
  its keep the first time it runs against a real installation.
  *(read-from-source — `docs/story.md`, "mcpx doctor")*
- **Remote servers without a shim**: Streamable HTTP makes `mcp-remote`
  unnecessary for remote servers. *(read-from-source — `docs/story.md`,
  "Remote servers without a shim")*
- **Which registry.** The official registry publishes an OpenAPI spec;
  HAPI is not open source and returns 404. Registry choice is a licensing
  and availability decision before it is a technical one.
  *(read-from-source — `docs/story.md`, "Which registry")*

## OpenAPI and REST

- **Specifications as tools: two things easy to get wrong.** A relative
  server URL must resolve against the document's location, and only read
  methods are exposed by default. *(read-from-source — `docs/story.md`,
  "Specifications as tools")*
- **The specification described the wrong thing**: the first spec described
  mcpx's own ten tools, not the hundreds it fronts. *(read-from-source —
  `docs/story.md`, "Specifications as tools")*
- **One ops table generates the routes, the MCP tools, and the OpenAPI
  document** (`internal/api/ops*.go`), so the three cannot drift apart.
  *(read-from-source — `internal/api/`)*

## OAuth

- **OAuth for remote servers is `status: proposed`**, not built; the
  credential story that exists today is in "Credentials".
  *(read-from-source — `docs/story.md`, "OAuth for remote servers" and
  "Credentials")*

## Command line

- **Command-line programs as servers**: a CLI with a daemon behind it is a
  server with a command-line client, and should be designed like one.
  *(read-from-source — `docs/story.md`, "Command-line programs as servers")*
- **Derived flag names surprise people**: `daemon.leaseTTL` becomes
  `--daemon-lease-ttl`. *(read-from-source — `docs/cli.md:142–146`)*
- **Accepted-and-ignored flags are bugs**, even when accepting them is
  deliberate-looking (#74, fixed by #75). *(read-from-source — issues #74,
  #75)*
- **Help coverage drifts from the dispatch table**: `internal/cli/manual.go`
  `Commands()` lists 32 commands, but `cmd/mcpx/main.go` also routes `man`,
  `completion`, `serve`, `openapi`, `adapter`, `registry`, and `api`, which
  help and man therefore do not list. *(read-from-source —
  `internal/cli/manual.go`, `cmd/mcpx/main.go`)*

## Containers and CI

Outline only in this slice: the container and CI seeds (static Go builds,
image-size anatomy, `actions/setup-go` in a monorepo, stopping test
daemons) are harvested with the Nix slice below, because several were
recorded alongside it (#182) and splitting them twice would fork them.
The seeds themselves are listed on #1871 and in `docs/container.md`
sections 4–5.

## Nix

**Later slice, named rather than dropped.** The standalone Nix section —
a section that does not need to mention mcpx — is the second slice of
#1871, not part of this one. Its seeds are already recorded on #1871:
`vendorHash` repair in the mcpx workflow, the sandbox-test package list in
`package.nix`, pure-Go SQLite, `wrapProgram` PATH suffixes, generated man
pages and completions, the flake layout and patch maintenance for this
repository, and the macOS findings. Open question 2 on #1871 (does this
section live in this book, or in a book for the whole repository that this
one links to?) is still open and should be answered before the slice is
written, not after.

## Harvesting: how findings get here

Approach C from #1871, in force for this chapter:

1. **Intake.** Anything learned that is not about the change at hand — a
   verified surprise, a measured cost, a behaviour read in someone else's
   source — is filed as an issue (or a comment with a link) under the
   `research` label, not buried in a session transcript or a PR body. A
   finding stays an issue until it is written up here, so nothing is lost
   in between. `docs/ideas.md` states the same discipline for design
   questions: an answer that only exists in a conversation is lost.
2. **Entry format.** A finding written up here is an excerpt plus a
   citation, in the entry style the runbook uses (created / last-updated /
   status / tags / sources / checked), with the finding-specific statuses
   `verified`, `measured`, `read-in-source`, `reported`, `hearsay`, and
   `superseded`. The tags in this file's key block are the chapter's; a
   finding's own topics go in its section placement and its citation.
3. **Harvest.** A per-release step moves labelled items into this chapter
   as entries and closes the issues with a link to the section that took
   them. Excerpt, do not move: a finding inside a feature entry keeps that
   entry as its source of truth and is cited from here.
4. **Supersede, never silently edit.** When a finding turns out wrong, it
   is struck through or marked `superseded` with a link to what replaced
   it. The history is the audit trail (#150).

## Quality bar for an entry

From #1871, applied to every seed above:

- **Sources.** Every claim has at least one: a file with a section or line,
  a commit, an issue, or an external URL with the date it was read.
- **Measurements.** A measurement carries its environment (machine, OS,
  runtime and tool versions) and the command that produced it — re-run the
  command, paste the result.
- **Provenance.** Say how it is known: reproduced, read in source, read in
  docs, or someone said so. Never let hearsay read like a measurement.
- **Dates.** Every entry is dated; a re-check updates the date. The seed
  harvest above was checked against `main` on 2026-11-01.
- **Standalone.** Where possible an entry is readable without mcpx
  context. That is the point of the Nix section.
