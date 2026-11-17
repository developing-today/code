# Research and incidental findings

```
created:      2026-11-01T01:30:00-05:00
last-updated: 2026-11-14T07:00:00-06:00
increment:    2
status:       draft
tags:         area:docs, area:compendium, topic:research
description:  what was learned while building mcpx, and while doing anything
              else in this repository, collected by topic and tagged with its
              provenance so it is not lost in session transcripts. First slice
              of #1871: the section outline, the seed findings that already
              exist in the repository, and the harvesting process. Second
              slice: the standalone Nix section and the Containers and CI
              harvest, with the placement answer recorded in the Nix
              section.
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
  definition up front. _(measured — `docs/story.md`, "Zero-context
  discovery")_
- **Switch to progressive discovery at 1–5% of the context window**, and do
  not mutate the `tools` array mid-conversation: it invalidates the
  provider's prompt cache. _(reported — modelcontextprotocol.io, MCP client
  best practices, as cited on #1871)_
- **Models write code better than they call tools**, Cloudflare argues,
  because tool-call tokens are synthetic training data while code is what
  models were trained on. This is the argument behind Code Mode. _(reported —
  blog.cloudflare.com/code-mode, as cited on #1871)_

## Harnesses

- **What is opencode's and what is general.** `docs/opencode-plugin.md`
  separates the opencode plugin surface (v1 vs v2) from the parts that apply
  to any harness; `OPENCODE-V2.md` corrects the record where the first draft
  blurred them — opencode v1 ships code mode behind
  `OPENCODE_EXPERIMENTAL_CODE_MODE`. _(read-from-source —
  `docs/opencode-plugin.md`, `OPENCODE-V2.md`)_
- **The harness knows what the agent does not.** Session state, statistics,
  and affordances live in the harness; an agent that assumes them instead of
  asking gets them wrong. The opencode session statistics are the worked
  example. _(read-from-source — `docs/story.md`, "The harness knows what the
  agent does not" and "Statistics from opencode")_
- **`AGENTS.md` is the cross-harness instruction file**: the same file is
  read by the harnesses this repository is used from, which is why fleet
  rules live there and not in one tool's config. _(read-from-source —
  `AGENTS.md` at the repository root and in `pkgs/mcpx`)_
- **A harness-free core is a design constraint, not an accident** (#48):
  codex, claude code, pi, and manus must all be able to drive the same core.
  _(read-from-source — `docs/story.md`; issue #48)_

## Plugins

<!-- id: research-plugins since: unreleased -->

- **A plugin can connect without the binary.** The connection ladder and
  `GET /v1/resolve` let the opencode plugin find or start a daemon instead
  of assuming one is installed (#44, fixed in #53). _(read-from-source —
  `docs/plugin-discovery.md`)_
- **The plugin logs over the socket** (#36), so plugin diagnostics land
  where the daemon's do instead of in a file nobody opens.
  _(read-from-source — `docs/plugin-discovery.md`; issue #36)_
- **Plugin install is a README, not a package step.** _(read-from-source —
  `plugin/opencode/README.md`)_

## Token providers

The outline assumes both readings of the term (open question 1 on #1871):
model providers, and the credential/OAuth providers that hand out tokens.

- **Model providers are maintained as patches.** This repository patches
  opencode for provider behaviour (Copilot business support, Copilot
  compaction, OpenAI response-id caching) and keeps skills for maintaining
  those patches across opencode updates. _(read-from-source — #1871 seed
  list. Harvest note, 2026-11-01: the three patch files the issue names are
  no longer at the repository root on `main`; the patches' current home
  needs naming here by whoever next touches the opencode patch set.)_
- **Credential placeholders keep secrets out of config files and descriptions, but
  stdio servers receive them through the environment. Generated scripts inherit
  that environment unless variables are unset or the run uses `-i`; do not
  treat this as isolation from code you execute. _(read-from-source —
  `docs/story.md`, "Credentials"; `internal/runner/dispatch.go:693–705`)_

## Cloudflare

- **Code Mode mechanics.** The Worker Loader API runs generated code in
  isolates; `fetch()`/`connect()` throw inside it; bindings hide API keys
  from the generated code. Shared with #153 — the design treatment lives
  with the design, this is the finding. _(read-from-source — `docs/story.md`;
  issue #153)_

## The TypeScript ecosystem

- **Node runs TypeScript by stripping types, and only erasable syntax
  survives.** Node v24.20.0 type stripping rejects a file with an `enum`;
  constructor parameter properties failed too, which is why the generated
  client uses plain fields. _(read-from-source — `ASSESSMENT.md` bug 3;
  `docs/story.md`, "Runtime portability")_
- **Runtime detection order is deno, then bun, then node** (`script.runtime`,
  `--runtime`, `MCPX_SCRIPT_RUNTIME`). Versions at the time of the seed:
  Deno 2.9.6, Bun 1.3.11. _(read-from-source — `docs/story.md`, "Runtime
  portability"; the runtime comparison itself belongs to #101 and
  `docs/runtimes.md`)_
- **`--typecheck` is `deno check`, Deno only**: ~80 ms on repeat, ~400 ms
  for a new program (#79). _(measured — as recorded on #1871; `docs/story.md`)_
- **Constructing a TypeScript compiler per request was lootbox's memory
  problem** — the `ts-morph` lesson behind the current design.
  _(read-from-source — `ASSESSMENT.md`)_

## SSE

- **Server-sent events, not WebSockets.** SSE is plain HTTP, `EventSource`
  reconnects on its own, and WebSocket upgrades are mishandled by
  middleboxes — the reasoning behind mcpx's remote transport.
  _(read-from-source — `docs/story.md`, "Server-sent events, not
  WebSockets")_
- **Lossless reconnection** is a protocol property, not a client hope: the
  stream carries what a reconnecting client needs to resume without a gap.
  _(read-from-source — `docs/story.md`, "Lossless reconnection")_

## Sockets and TCP

- **What each transport costs.** Spawning `mcpx status` took 23.12 ms; a
  unix-socket keep-alive round trip 0.17 ms (135× cheaper); TCP loopback
  keep-alive 0.65 ms. The daemon-and-socket design is priced by these
  numbers. _(measured — `docs/story.md`, "What each transport costs")_
- **macOS `sun_path` is 104 bytes.** Deep state directories produced socket
  paths that failed with `bind: invalid argument`. _(read-from-source —
  `ASSESSMENT.md` bug 4)_
- **`localhost` is not `127.0.0.1`.** It resolved to IPv6 only while the
  compiled build bound `127.0.0.1`, so the client dialled an address
  nothing listened on. _(read-from-source — `ASSESSMENT.md`)_

## RPC and WebSockets

- **JSON-RPC framing needs no dependency** — it is standard library, which
  is why it appears in the dependency audit as a deliberate non-dependency.
  _(read-from-source — `docs/dependencies.md`)_
- **Lootbox's WebSocket.** The predecessor system used a WebSocket RPC
  channel (`lootbox.config.json`: `serverUrl: ws://127.0.0.1:9420/ws`) —
  the contrast case for the SSE choice above. _(read-from-source — #1871
  seed list)_

## MCP

<!-- id: research-mcp since: unreleased -->

- **Two protocol eras.** The protocol has a before and an after, and a
  client has to speak to both; "What 2026-07-28 actually asks of a client"
  is the checklist. _(read-from-source — `docs/story.md`, "Two protocol
  eras")_
- **The revision matrix** — every revision in both directions, plus what is
  still missing — lives in `docs/protocol.md` and is excerpted, not copied,
  here. _(read-from-source — `docs/protocol.md`)_
- **The sampling bug: it did not work, and nothing said so.** Capabilities
  are declared in the handshake, but the handler was installed after it,
  and every test called the passthrough directly — so sampling was
  advertised, broken, and green in CI at the same time.
  _(read-from-source — `docs/story.md`, "Sampling")_
- **The elicitation bug came first** — the same shape, earlier.
  _(read-from-source — `docs/story.md`, "Elicitation"; `docs/elicitation.md`)_
- **`mcpx doctor` found a bug immediately** — a diagnostics command earns
  its keep the first time it runs against a real installation.
  _(read-from-source — `docs/story.md`, "mcpx doctor")_
- **Remote servers without a shim**: Streamable HTTP makes `mcp-remote`
  unnecessary for remote servers. _(read-from-source — `docs/story.md`,
  "Remote servers without a shim")_
- **Which registry.** The official registry publishes an OpenAPI spec;
  HAPI is not open source and returns 404. Registry choice is a licensing
  and availability decision before it is a technical one.
  _(read-from-source — `docs/story.md`, "Which registry")_

## OpenAPI and REST

- **Specifications as tools: two things easy to get wrong.** A relative
  server URL must resolve against the document's location, and only read
  methods are exposed by default. _(read-from-source — `docs/story.md`,
  "Specifications as tools")_
- **The specification described the wrong thing**: the first spec described
  mcpx's own ten tools, not the hundreds it fronts. _(read-from-source —
  `docs/story.md`, "Specifications as tools")_
- **One ops table generates the routes, the MCP tools, and the OpenAPI
  document** (`internal/api/ops*.go`), so the three cannot drift apart.
  _(read-from-source — `internal/api/`)_

## OAuth

- **OAuth for remote servers is `status: proposed`**, not built; the
  credential story that exists today is in "Credentials".
  _(read-from-source — `docs/story.md`, "OAuth for remote servers" and
  "Credentials")_

## Command line

- **Command-line programs as servers**: a CLI with a daemon behind it is a
  server with a command-line client, and should be designed like one.
  _(read-from-source — `docs/story.md`, "Command-line programs as servers")_
- **Derived flag names surprise people**: `daemon.leaseTTL` becomes
  `--daemon-lease-ttl`. _(read-from-source — `docs/cli.md:142–146`)_
- **Accepted-and-ignored flags are bugs**, even when accepting them is
  deliberate-looking (#74, fixed by #75). _(read-from-source — issues #74,
  #75)_
- **Help coverage once drifted from the dispatch table** (historical):
  `man`, `completion`, `serve`, `openapi`, `adapter`, `registry`, and
  `api` were once dispatched by `cmd/mcpx/main.go` but undeclared in
  `internal/cli/manual.go`, so help, the man page, and completions did
  not list them. All seven are declared now, and
  `TestEveryDispatchedCommandIsDeclared` holds the declaration and the
  dispatch table equal in both directions, so the gap cannot reopen
  silently. _(read-from-source — `docs/cli.md`, “once dispatched
  but undeclared”; `internal/cli/manual.go`)_

## Containers and CI

- **Static Go builds: `CGO_ENABLED=0` is a behaviour change on Linux, and
  nearly free.** `os/user` and `net` pull in libc only under cgo; with cgo
  off, pure-Go `os/user` reads `/etc/passwd` instead of NSS and the pure-Go
  resolver ignores `nsswitch.conf`. The measured cost was nothing: a fully
  static linux binary within ~100 bytes of the cgo build (14 811 298
  against 14 811 401) that still runs `exec`, verified `not a dynamic
executable` under `ldd`. The policy that came out of it (#1987): the
  artifacts that ship as a bare binary ship static, and `package.nix` sets
  `env.CGO_ENABLED = "0"`. On macOS, since Go 1.20, `net` and `os/user`
  use the system libraries without cgo, so the flag changes nothing there.
  _(measured — `docs/container.md` section 4)_
- **Where image size goes: `git` outweighs the runtime.** In the retired
  Debian prototype (historical measurements, linux/arm64, not the current
  Nix image), `debian:bookworm-slim` plus `git` with
  `--no-install-recommends` cost a 105 MB apt layer — more than bun at
  79.4 MB — because Debian's `git` depends on perl outright. Dropping git
  took the image from 413 MB to 288 MB of disk usage. _(measured —
  `docs/container.md` section 5)_
- **`actions/setup-go` in a monorepo needs `cache-dependency-path`.**
  Without it, setup-go looks for `go.sum` at the repository root, logs
  "Restore cache failed: Dependencies file is not found" on every run,
  and never restores a cache. Every setup-go step in the mcpx workflow now
  passes `cache-dependency-path: pkgs/mcpx/go.sum`, with the reason in the
  comment above it. _(read-from-source — `.github/workflows/mcpx.yml`
  setup-go steps; fixed in #182)_
- **Test daemons must be stopped the way they record themselves.** Plain
  `stop` recomputes the socket path from the environment, which a long
  state path moves elsewhere, so e2e daemons once survived for hours
  (#52). The fix stops daemons through the info files they write
  (`stop --all` builds its client from the daemon's own record), and the
  e2e harness sets a short idle exit as a backstop for anything that
  escapes cleanup entirely. _(read-from-source —
  `internal/cli/client.go`; `internal/cli/stopall_test.go`;
  `internal/e2e/e2e_test.go`; #52, harvested in #182)_

## Nix

This section is standalone by design: it does not need to mention the
project the book belongs to, and most entries below do not. **Placement,
recorded so the question does not reopen:** this section lives in this
book (open question 2 on #1871, answered). The note that commissioned
the chapter scoped it that way — the compendium "can have an entire
section on nix that is a hundred pages and doesnt mention mcpx at all" —
and the seeds are this repository's Nix findings, harvested alongside
the Containers and CI seeds they were recorded with. If a
repository-wide book ever exists, this book links to it then; there is
no pre-emptive split.

**Go on Nix.**

- **`vendorHash` repair is a workflow job, because the bot cannot do
  it.** Renovate can bump `go.mod` but cannot compute the vendored-tree
  hash, so every Go dependency bump used to break the Nix build and
  merge silently. The workflow's `vendorhash-repair` job runs only on
  bot branches: it blanks `vendorHash` in `package.nix`, reads the
  `got: sha256-...` line from the failed build, commits the repaired
  hash back to the bot branch, and fails the PR if the build still does
  not pass. A human changing dependencies deliberately gets no repair —
  they see the failure and fix it themselves. _(read-from-source —
  `.github/workflows/mcpx.yml`, `vendorhash-repair` job)_
- **The sandbox test list is computed now, not listed.** The seed form
  of this finding was an explicit package list in `package.nix`:
  `subPackages = [ "cmd/mcpx" ]` confines `go test` to a package with no
  tests, so `checkPhase` listed the internal packages by hand and a new
  package was untested in the sandbox until someone added it. The
  current form inverts the default: `checkPhase` computes every package
  with test files, minus a `checkExclude` list whose every entry needs
  a comment saying why it cannot run in the sandbox, with
  `internal/buildcheck` enforcing both halves. The lesson survives the
  inversion: a Nix sandbox runs only the tests the derivation says it
  runs, so say it in a way a new package cannot silently fall out of.
  _(read-from-source — `package.nix`, `checkPhase` and the comment
  above it; seed form on #1871)_
- **Pure-Go SQLite keeps the derivation free of a C toolchain.**
  `modernc.org/sqlite` is a pure-Go translation of SQLite rather than a
  binding; the usual cgo driver would require a C toolchain in the
  derivation, break cross-compilation (no Linux binary from a Mac), and
  fail `CGO_ENABLED=0` builds outright. _(read-from-source —
  `docs/dependencies.md`, "Why this driver"; `package.nix` comment at
  the dependency)_

**Wrapping and generated artefacts.**

- **`wrapProgram --suffix PATH`, so behaviour does not depend on the
  calling shell.** The installed binary is wrapped with git, deno,
  `bun-bin` and nodejs appended to PATH. Git is the load-bearing one:
  the `repo` and `worktree` scopes degrade to `cwd` — silently weaker —
  when git is missing, so the wrapper makes its presence a property of
  the package, not of the caller's environment. _(read-from-source —
  `package.nix`, `postInstall`; `docs/dependencies.md`, "External
  programs")_
- **The man page and shell completions are generated by the just-built
  binary in `postInstall`** — the man page, the Markdown reference
  tree, and the bash/zsh/fish completions all come from the binary's
  own declarations at install time. "A man page maintained separately
  is wrong within two releases" is the reason recorded in the source.
  _(read-from-source — `package.nix`, `postInstall`)_

**Beyond one package: this repository.**

- **One flake carries the whole estate.** This repository's
  `flake.nix` is a nix-darwin + NixOS + home-manager flake: the
  nix-darwin input, `machines/`, `nixos/`, `modules/`, `home/`, and a
  `pkgs/` tree of custom packages (dozens, from `bun-bin`-style runtime
  pins to whole tools) all build from the one flake. A finding about
  "the Nix setup" here is a finding about that monorepo flake, not
  about any single package in it. _(read-from-source — `flake.nix`;
  `pkgs/`)_
- **Homebrew is the flake's macOS escape hatch, recorded as a seed.**
  The seed names the nix-darwin Homebrew layer — brews, casks and
  `masApps` for what Nix does not package — as part of this section's
  territory. Harvest note, 2026-11-14: no Homebrew declaration was
  found in `home/`, `machines/`, `modules/` or `nixos/` on current
  `main`, so the layer's current home needs naming here by whoever
  next touches the darwin configuration; the seed stands as recorded.
  _(reported — #1871 seed list)_
- **Patching a fork from the flake, and keeping the patches alive.**
  This flake does not patch its opencode input with loose patch files:
  the `opencode` flake input is the maintained fork
  (`developing-today-forks/opencode`), the openchamber input follows
  the same opencode so there is one opencode source in the whole
  module chain, and the `flake-inputs-update` workflow bumps,
  validates and merges those inputs daily. The patch-maintenance
  skills the seed names (`update-opencode`, `fix-opencode-patches`)
  belong to that fork-and-update workflow; the three root-level patch
  files an earlier harvest named are no longer on `main` (see Token
  providers above). _(read-from-source — `flake.nix`, opencode input;
  `.github/workflows/flake-inputs-update.yml`; seed on #1871)_
- **Renovate on a flake: delegate what a workflow owns.** Renovate
  runs against this repository, but its `renovate.json` disables the
  nix manager for the opencode and openchamber flake inputs, because
  the dedicated `flake-inputs-update` workflow owns them. Two bots
  racing the same pin produce broken bumps; one owner per input is
  the rule the config records. _(read-from-source — `renovate.json`)_
- **The justfile is the human entry point.** The repository root
  `justfile` imports `root.just` and per-package recipe modules
  (`mod id 'pkgs/id/justfile'`), so the commands a person runs day to
  day are just recipes over the flake, not memorised `nix` invocations.
  _(read-from-source — `justfile` at the repository root)_

**macOS findings.**

- **The temp-dir sweep eats worktrees, silently.** The per-user temp
  directory `/var/folders/.../T/` is swept by the OS; a sweep destroyed
  ~3,000 tracked files across several active worktrees in a single
  session, while `git status` kept reporting a clean tree from a stale
  fsmonitor index. Worktrees belong on durable storage, never under
  `/var` — the "pre-approved" temp path is a permission, not a
  durability promise. _(read-from-source — `doc/platform/worktrees.md`;
  seed on #1871)_
- **Platform-only tests cut both ways.** Two tests once passed only on
  macOS (#35, as recorded on #1871). The current tree carries the
  inverse in the open: the Nix `checkPhase` names twenty Linux-only
  takeover/handoff tests that the darwin run skips by name, because
  they exercise `/proc` and systemd mechanics darwin does not have.
  A named skip list with the reason attached is how a platform-only
  test stays a fact instead of becoming a surprise. _(reported —
  #1871 seed list, #35; read-from-source — `package.nix`,
  `darwinSkip`)_

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
