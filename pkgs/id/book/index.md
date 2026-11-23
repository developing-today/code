# id

ID is Drew's identity and graph platform. It started as a peer-to-peer file
sharing tool and is now a local-first, federated, capability-based
application platform built on the Iroh networking stack. The parts of the
system are:

- **Content-addressed objects.** A file is stored under the hash of its
  bytes in an Iroh blob store. A human-readable name points at the hash, so
  the same content is the same object on every node that holds it.
- **P2P file sharing over Iroh.** Nodes store, retrieve, and send files by
  name or by hash, locally or against a remote node named by its 64-hex
  node ID. Peer discovery runs over gossip, mDNS, and an optional DHT
  lookup.
- **Worlds with Roc guests.** A world is a multiplayer program written in
  Roc, compiled to an import-free Wasm module, and run by a host under the
  Plaza ABI v1. Guests have no direct access to the filesystem, the
  network, the clock, or randomness; every effect goes through the
  capability protocol.
- **Capabilities.** World access is granted as capabilities: an invite
  minted by a host's admin, a per-world ceiling that bounds what any
  program in the world may request, and provider grants enforced on the
  host side. Reaching a node grants no authority on that node.
- **The directory.** A home-server directory holds accounts, groups,
  invites, and keysets, recorded in an append-only journal and replayed
  through the same code path as live changes.
- **The hosted edition.** A per-user edition of the same system runs as a
  Cloudflare Worker with one Durable Object per user. In the source tree
  today it is a local-only scaffold: it is not a deployed service, and
  this book says so wherever it describes it.

The code lives in `pkgs/id/` in the `developing-today/code` repository: a
Rust workspace of 17 packages — the root `id` crate (the facade) plus 16
crates under `pkgs/id/crates/` — surveyed at commit `1ccb4a19457` on
2026-11-23.

## Why ID exists

Before ID, each of Drew's tools kept its own store in its own shape, and
each program that needed identity, files, or shared state built its own
version of them. ID replaces those separate stores with one platform:

- **One object store for files and records.** Files, documents, signed
  events, and world state all live in content-addressed or signed,
  journalled stores with one set of rules for naming, tagging, and sync,
  instead of one ad-hoc store per application.
- **Identity and authority in one place.** Accounts, groups, delegation,
  key custody, and recovery are directory functions. World access is a
  capability issued against that identity, not a password per program.
- **Local-first, hostable by anyone.** A node runs entirely on one
  machine from a data directory. Hosting is an explicit act — `id serve`
  — and remote commands are clients only: they connect out to a host and
  never host a world themselves. The hosted edition is a per-user
  backstop built from the same stores, not a requirement.
- **Programs that cannot exceed their grant.** A Roc guest is a pure
  function over its model plus a list of capability requests. The host
  answers each request only if the world's ceiling and the participant's
  grant allow it; a known but ungranted request returns a refusal, never
  silent execution.

## Who it serves

- **Drew, at the command line.** The `id` binary is the owner's tool:
  file commands (`put`, `get`, `find`, `list`, `cat`, `show`, `peek`),
  the `tag` family, `serve`, the `world` client commands, `group`,
  `doc`, `claim`, `key`, and `keyset`. Running `id` with no subcommand
  starts the interactive REPL. The second binary, `id-native-shell`, is
  the Bevy desktop shell.
- **Agents in worlds.** An agent joins a world with a capability and acts
  through the world session protocol. The MCP bridge in the `id-caps`
  crate translates MCP tool calls into world session frames under a
  policy point that checks a capability ceiling, charges quota before
  execution, and writes one signed audit event per action. The bridge is
  a library translator: there is no standalone MCP server binary in
  `pkgs/id`. See [Agents in worlds: the MCP bridge and policy](concepts/mcp-and-agents.md)
  and [The MCP bridge](reference/mcp.md).
- **Hosted per-user edition users.** A bound hosted user gets one Durable
  Object holding that user's key-value data, blob references, and a
  signed-object bridge store that acts as a sync peer. The binding
  ceremony is native code (`pkgs/id/src/hosted_binding.rs`); the Worker
  consumes the resolved binding and jurisdiction headers. This edition is
  a scaffold in the tree, gated on issues #2285 and #3403 for any real
  deployment. See [The hosted edition](concepts/hosted.md) and
  [The hosted Worker API](reference/hosted-worker.md).

## The surface map

ID is one platform with several surfaces. The map below is the whole
system on one page; [The shape of the system](getting-started/shape.md)
explains how the pieces fit together.

| Surface | What it is | Where it is covered |
| --- | --- | --- |
| The crates | 17 packages: the `id` facade plus `id-core`, `id-caps`, `id-store`, `id-events`, `id-groups`, `id-sync`, `id-directory`, `id-worlds`, `id-web`, `id-cli`, `id-config`, `id-currency-registry`, `id-render`, `id-import-notion`, `id-native-shell`, and `id-hw-shell` | [The crates](library/crates.md), [The id facade crate](library/facade.md) |
| The `id` binary | The CLI: 32 top-level commands and 113 leaf commands (surveyed 2026-11-23), including `serve`, the only command that hosts files, a world, or the web UI | [The id command line](reference/cli.md) |
| The `id-native-shell` binary | A Bevy desktop client for the world shell contract; a workspace member but not a default member, so a normal `id` build never builds it | [id-native-shell](reference/native-shell.md) |
| The REPL | The interactive shell started by `id repl` or by bare `id`, with its own parser, its own command set, and shell features (command substitution, the `|>` pipe-in operator, here-strings, and heredocs) | [The interactive REPL](reference/repl.md) |
| The web UI | The `id-web` crate served by `id serve --web` (bind default `127.0.0.1`, port default `3000`): file browser, collaborative editor, world pages, program gallery, and the HTTP and WebSocket APIs — 54 distinct paths and 58 method bindings in the main router (surveyed 2026-11-23) | [The web UI: pages and HTTP API](reference/web-ui.md), [WebSocket protocols](reference/websockets.md) |
| The hosted Worker scaffold | `pkgs/id/hosted-worker`: a Cloudflare Worker front door forwarding to one `UserDO` Durable Object per bound user, with 15 method routes; local-only, not deployed | [The hosted Worker API](reference/hosted-worker.md) |
| The MCP bridge | `id_caps::mcp_bridge` and `id_caps::mcp_policy`: a library-level translator from 8 pinned MCP v1 tools to world session frames, plus the host-side policy point; no server binary and no transport of its own | [The MCP bridge](reference/mcp.md) |

## How to read this book

- **First run:** [Installation and provisioning](getting-started/installation.md),
  then [Quickstart](getting-started/quickstart.md). You will have a built
  `id` binary, a verified version string, and one complete store, fetch,
  tag, and find run.
- **Understanding the system:** [The shape of the system](getting-started/shape.md),
  then [Architecture](concepts/architecture.md), then the concept chapter
  for the part you are working in — the file core, worlds, capabilities,
  identity and the directory, sync, events, residency, the hosted
  edition, shells, agents, and currency.
- **Looking up an exact surface:** the reference part gives, for every
  command, page, endpoint, and protocol, its invocation shape, its flags
  with their literal defaults, its behaviour, and its failure shapes.
- **Building on the library:** the library part covers the crates as a
  Rust API, starting from the facade; the tutorials walk through complete
  tasks end to end.
- **Design work:** the design part is explicitly design. Those chapters
  describe the recorded design programme — 408 ID design records in the
  repository's `doc/` archive (counted 2026-11-23) — and each one states
  what exists today before it describes what is designed.

## Conventions used in this book

- **Defaults are stated literally.** When a flag, parameter, or setting
  has a default, the default is given as the value itself — for example
  "web port (default `3000`)", "world name (default `lobby`)",
  "checkpoint interval (default `1000`)".
- **Source wins over help.** Where a binary's help text, a README, or a
  package document disagrees with the source, this book states the source
  behaviour and names the disagreement. The surveys behind this book
  recorded 56 numbered disagreements across the CLI, library, web and
  configuration, and design-record surfaces (surveyed 2026-11-23); the
  chapters note the ones that affect their surface instead of silently
  picking a side.
- **Secrets are named, never valued.** This book gives secret names,
  token file paths, flag names such as `--web-token` and
  `--world-admin-token`, and environment variable names such as
  `ID_WEB_TOKEN` and `ID_WORLD_ADMIN_TOKEN`. It never prints a secret
  value. Passphrases for `id key` and `id keyset` are hidden prompts in
  the tools themselves, never flags.
- **Design is marked as design.** A chapter in the design part, and any
  designed-but-unshipped behaviour mentioned elsewhere, is labelled as a
  design with its tracking issues named. Designed behaviour is never
  written as if it works today.
- **Numbers are dated.** Command, flag, route, crate, and record counts
  carry their survey date (2026-11-23, at commit `1ccb4a19457`). A number
  without a date in this book is a constant of the code — a default, a
  limit, a version — not a measurement.
- **Chapters are self-contained.** Each chapter defines its terms where
  it first uses them and cross-links instead of assuming you have read
  the chapters before it.
