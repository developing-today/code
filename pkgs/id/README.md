# id

A peer-to-peer file sharing CLI built with [Iroh](https://iroh.computer). Store, retrieve, and share files across a decentralized network with optional collaborative web editing.

## Features

- **P2P File Sharing**: Store and retrieve files using content-addressed hashes over the Iroh network
- **Named Tags**: Reference files by human-readable names instead of hashes
- **Metadata Tags**: Attach arbitrary key-value metadata to files with binary-safe keys/values and structured search syntax
- **Remote Operations**: Run commands against remote peers by node ID
- **Collaborative Editor**: Real-time multi-user text editing via WebSocket (web UI)
- **Interactive REPL**: Shell-like interface with pipes, subshells, and heredocs
- **Peer Discovery**: Automatic local network peer discovery
- **Content Modes**: Smart rendering for text, markdown, images, video, audio, and PDF
- **Themes**: Terminal-inspired UI themes (sneak, arch, mech)

## Quick Start

### With Nix (recommended)

```bash
nix develop   # Enter dev shell with all tools

# Library-only build (no web dependencies)
just build-lib
just serve-lib

# Full build with web UI (requires Bun)
just build
just serve
```

### With Cargo

```bash
# Library-only (no web UI)
cargo build --no-default-features
cargo run --no-default-features -- serve

# With web UI (build assets first)
cd web && bun install && bun run build && cd ..
cargo build --features web
cargo run --features web -- serve --web
```

## Usage

```bash
# Store a file
id put myfile.txt

# Retrieve a file
id get myfile.txt

# List all files
id list

# Find files by pattern
id find "*.rs"

# Search files
id search "hello"

# View file content
id cat myfile.txt
id show myfile          # Find + cat in one step
id peek myfile          # Preview with head/tail

# Metadata tags (aliases: tag, label, link)
id tag set myfile.txt author "Jane Doe"
id tag set myfile.txt category notes
id tag list myfile.txt
id tag search author
id tag search author:Jane           # Key-value search
id tag search :important             # Value-only search
id tag search "literal:text"         # Quoted literal search
id tag del myfile.txt category

# Migrate existing files to have name/file auto-tags
id migrate-tags

# Client-only by default: the remote operations above and every
# `id world ...` subcommand are clients — they connect out to a host
# and never host or serve a world, however many world sessions they
# open. Hosting begins only through the explicit `id serve` (its SSH
# world serving is opt-in via `serve` flags). Reaching a node is not
# world authority: a valid world capability is still required for
# world operations. See "Client-only boundary" below.

# Start a server
id serve                          # CLI only
id serve --web                    # With web UI (default port 3000)
id serve --web --port 8080        # Custom port
id serve --web --ephemeral        # Ephemeral mode
id serve --web --data-dir /path   # Custom data directory
id serve --web --new              # Fresh node (new identity)

# Interactive REPL
id repl

# Remote operations (prefix with node ID)
id put <node_id> myfile.txt
id list <node_id>

# Identity and peers
id id                   # Print local node ID
id key export backup.idkey    # Export with a hidden passphrase prompt
id key import backup.idkey    # Import; existing identities require --replace
id peers                # List discovered peers
id peers --abuse        # Per-peer frame/byte abuse counters, by direction (world traffic only; empty when serving without --world)
```

## The #9106 walking skeleton, by hand

`id walk` drives the #9106 walking-skeleton path from the terminal:
one signed record at an `id://` address, held in a node's on-disk
store, synced to a second node over the federation exchange
protocol, a conflicting write, a conflict report on both nodes,
and a resolution. Each node is a directory; every command opens
its node fresh and shuts it down on the way out, so what a stage
leaves behind is read back from disk by the next one. One process
uses a node directory at a time — stop a node's `serve` before
running one-shot commands against it (an open that finds the
directory held says so, rather than waiting).

The writers are demonstration keys, derived from their names so
two terminals can play both sides without a key exchange: alice
and bob are the whole demo world, and production key management
(`id key`, `id keyset`) is a different path.

```bash
# Two nodes.
id walk init node-a --name node-a
id walk init node-b --name node-b

# 1. Write a signed record at an id:// address on A.
id walk write node-a --as alice --text "first draft"
# address: id://node-a/notes/v1/content/raw-blake3-256.82b41092…
# event:   82b41092…
# outcome: applied
ADDR=id://node-a/notes/v1/content/raw-blake3-256.82b41092…   # the full address, from the write

# B does not hold it yet — reading the address there is an error,
# not a fetch:
id walk read node-b "$ADDR"
# Error: this node does not hold the record at id://node-a/…

# 2. Serve B (it prints a peer token), and sync A to it.
id walk serve node-b
# WALK_NODE name=node-b peer=fe50e5b9…@127.0.0.1:45718
PEER=fe50e5b9…@127.0.0.1:45718                                # the full token, from the serve
id walk sync node-a --peer "$PEER"
# sent to the peer: 82b41092…
#   82b41092… -> applied

# Stop B's serve (Ctrl-C). The record B admitted while serving
# was persisted: it still reads from its address.
id walk read node-b "$ADDR"
# signer: alice
# text:   first draft

# 3. Conflicting writes: each writer follows the base record on
# their own node. (A write with no --parent follows the node's
# current heads.)
id walk write node-a --as alice --text "alice's edit"   # event: c56121b2…
id walk write node-b --as bob   --text "bob's edit"     # event: efe96df0…

# 4. Sync again; the conflict is reported on both nodes.
id walk serve node-b &   # take a fresh peer token, as above
id walk sync node-a --peer "$PEER"
id walk reports node-a
# head: c56121b2…
# head: efe96df0…
# report: 1cb9f5cd…
#   status: open
#   key:    e7bbe860…
#   base:   82b41092…
#   version: c56121b2… by alice
#   version: efe96df0… by bob
id walk reports node-b   # the same key, base and versions;
                         # the report id is B's own (7802d305…) —
                         # a report records how versions arrived
                         # at the reporting node, so ids are
                         # node-local and the key is the shared
                         # identity of the conflict

# 5. Resolve on B with a record that follows both edits, sync
# once more, and the report closes on A too.
id walk write node-b --as bob --text "both edits, merged by bob" \
    --parent c56121b2… --parent efe96df0…
# outcome: applied; resolved report 7802d305…
id walk sync node-a --peer "$PEER"
id walk reports node-a   # status: resolved by 8c2bdd30…
id walk read node-a id://node-b/notes/v1/content/raw-blake3-256.8c2bdd30…
# signer: bob
# text:   both edits, merged by bob
```

The same five steps run as suites in the nightly (registry entry
`id.rust.substrate-path`): in-process in
`crates/id-sync/tests/substrate_sync_path.rs`, between two
processes in `substrate_transport_path.rs`, and through this
command surface, against on-disk nodes with a restart between
stages, in `crates/id-cli/tests/walk_cli_path.rs`.

## Client-only boundary

Remote commands are clients: they never host a world. `id world
join`, `id world invite`, `id world records`, the other `id world ...`
subcommands and the remote `put` / `list` forms connect out to a host
node; none of them hosts a world or grants any authority on your node,
and opening additional world sessions from a client changes none of
that. There is no `--client-only` flag: no client command has a
hosting mode to switch off.

Two client subcommands do open local listeners without hosting a
world: `id world ssh-tunnel` without `--stdio` listens on a local
address (loopback by default) so an ordinary `ssh` client can reach
the host through it, and `id world mirror` runs a replica endpoint
that accepts sync connections for the replica it keeps. Neither makes
your node a world host and neither grants world authority.

Hosting starts only when you run `id serve` explicitly (SSH world
serving is a further opt-in among the `serve` flags). And connectivity
is never authority: being able to reach a node proves only which node
it is — a valid world capability, minted by the host's admin with
`id world invite`, is still required for every world operation.

## CLI Commands

| Command        | Aliases                        | Description                                             |
| -------------- | ------------------------------ | ------------------------------------------------------- |
| `serve`        |                                | Start server for peer requests                          |
| `publish`      |                                | Publish a program (world) on the running `id serve`     |
| `unpublish`    |                                | Remove a dynamic publication from the running serve     |
| `publications` |                                | List the running serve node's dynamic publications      |
| `repl`         | `shell`                        | Interactive REPL with pipes and subshells               |
| `put`          | `in`, `add`, `store`, `import` | Store files in blob store                               |
| `put-hash`     |                                | Store content by hash only (no named tag)               |
| `send`         |                                | Copy a file from one server to another                  |
| `get`          |                                | Retrieve files by name or hash                          |
| `get-hash`     |                                | Retrieve file by hash with explicit output path         |
| `cat`          | `output`, `out`                | Output file content to stdout                           |
| `show`         | `view`                         | Find file by pattern and output content                 |
| `peek`         |                                | Preview with configurable head/tail lines               |
| `find`         |                                | Find files by query, optionally output content          |
| `search`       |                                | Search files and list all matches                       |
| `list`         | `ls`                           | List all stored files                                   |
| `tag`          | `label`, `link`                | Manage metadata tags on files                           |
| `migrate-tags` | `migrate`                      | Add name/file auto-tags to existing files               |
| `id`           |                                | Print local node public ID                              |
| `key`          |                                | Export or import a passphrase-wrapped identity key      |
| `keyset`       |                                | Show or change the identity's admin and device keys    |
| `peers`        |                                | Discover and list known peers                           |
| `world`        |                                | Mint invites for, and join, multiplayer worlds          |
| `gallery`      |                                | Administer a program gallery directory                  |
| `match-audit`  |                                | Export and verify signed match-audit v0 logs            |
| `group`        |                                | Groups: create, policy, add, remove, roles, join        |
| `delegate`     |                                | Grant a delegate a scope in a group                     |
| `revoke`       |                                | Revoke a delegation by its grant event ID               |
| `doc`          |                                | Publish documents; list events/frontier/history         |
| `claim`        |                                | Tag claims on any subject (set, clear, search)          |
| `writer-sets`  |                                | Writer sets in the event store (show, expand, discover) |

### Tag Subcommands

| Subcommand | Aliases                                  | Description                                               |
| ---------- | ---------------------------------------- | --------------------------------------------------------- |
| `set`      | `add`                                    | Set a tag on a file                                       |
| `del`      | `rm`, `remove`, `rem`, `delete`, `unset` | Delete a tag from a file                                  |
| `list`     | `ls`                                     | List tags (supports `--hex`, `--binary`, `--no-truncate`) |
| `search`   | `find`                                   | Search tags with structured query syntax                  |

#### Search Syntax

Search uses structured query terms (space-separated, ANDed together):

| Syntax      | Meaning                                   | Example       |
| ----------- | ----------------------------------------- | ------------- |
| `key:`      | Filter by key name                        | `author:`     |
| `:value`    | Filter by value                           | `:Jane`       |
| `key:value` | Filter by exact key-value pair            | `author:Jane` |
| `"literal"` | Search all fields for literal text        | `"key:value"` |
| `bare`      | Case-insensitive search across all fields | `readme`      |

Quoted strings work in key:value position: `"key:":":value"` matches key `key:` with value `:value`.

## REPL

The interactive REPL supports shell-like features:

```bash
id repl

> put myfile.txt                  # Store a file
> list                            # List files
> tag set myfile.txt key value    # Set metadata
> tags myfile.txt                 # List tags (also: labels, links)
> label set myfile.txt key value  # Same as tag set
> link set myfile.txt key value   # Same as tag set
> tag search author:Jane          # Structured search syntax
> tag find readme                 # find = search alias
> migrate-tags                    # Add name/file tags to all files
> cat myfile.txt | head -5        # Pipe output to shell
> $(id id)                        # Subshell expansion
> help                            # Show all commands
```

## Build Variants

| Variant | Command          | Description              | Requires  |
| ------- | ---------------- | ------------------------ | --------- |
| `lib`   | `just build-lib` | Rust CLI only            | Rust      |
| `web`   | `just build`     | CLI with embedded web UI | Rust, Bun |

## Development

### Prerequisites

Enter the Nix dev shell for all required tools:

```bash
nix develop
```

This provides: Rust 1.99.1, clippy, rustfmt, cargo-llvm-cov, cargo-audit, cargo-machete, just, Bun, TypeScript, chromium, firefox.

### Testing

```bash
just test              # All fast tests (Rust + TypeScript unit + typecheck)
just test-rust         # Rust tests only (~408 unit + ~93 integration)
just test-unit         # Unit tests only (fast, ~408 tests)
just test-integration  # Integration tests only (~93 tests)
just test-web-unit     # TypeScript unit tests (~304 assertions)
just test-e2e          # Playwright E2E (chromium + firefox, 208 tests)
just test-nix          # nix flake check (27 checks — runs everything)
just ci                # Full CI check suite
just check             # Fix + CI (run before committing)
```

See [`doc/testing-architecture`](../../doc/2026-03-29T00-00-00Z_reference_testing_architecture/2026-03-29T00-00-00Z_reference_testing_architecture.md) for the complete testing reference: 6 test layers, browser coverage matrix, environment comparison, and when to add tests where.

### Nix

```bash
just build-nix                  # Build web variant (nix build)
just build-nix-lib              # Build lib variant (nix build .#id-lib)
just test-nix                   # All 27 CI checks in sandbox (nix flake check)
just test-nixos-playwright-e2e  # Full Playwright in 4 NixOS VMs
```

## Project Structure

See [ARCHITECTURE.md](ARCHITECTURE.md) for detailed system architecture.

```
src/
├── main.rs              # Entry point
├── lib.rs               # Library exports, node bootstrap
├── cli.rs               # Clap argument definitions
├── protocol.rs          # P2P request/response protocol
├── store.rs             # Iroh blob store operations
├── tags.rs              # Metadata tag system (alpha/omega dual-index)
├── tuple.rs             # Sort-preserving binary key encoding
├── helpers.rs           # Parsing and formatting utilities
├── discovery.rs         # Peer discovery
├── commands/            # Command implementations
│   ├── put.rs, get.rs, find.rs, list.rs, peers.rs, id.rs, tag.rs
│   ├── serve.rs         # Server with optional web UI
│   ├── repl.rs          # REPL entry point
│   └── client.rs        # Remote peer client
├── repl/                # REPL internals
│   ├── runner.rs        # Command dispatch and help
│   └── input.rs         # Shell preprocessing (pipes, subshells, heredocs)
└── web/                 # Web UI (feature-gated)
    ├── routes.rs        # Axum HTTP handlers
    ├── templates.rs     # HTML template rendering
    ├── collab.rs        # WebSocket collaboration server
    ├── tags_ws.rs       # WebSocket tag updates
    ├── assets.rs        # rust-embed static asset serving
    ├── content_mode.rs  # Content type detection
    └── markdown.rs      # Markdown rendering

web/                     # TypeScript frontend
├── src/                 # ProseMirror editor, collab, themes, editor features
└── dist/                # Built assets (embedded in binary)

e2e/                     # Playwright E2E tests (104 tests × 2 browsers = 208)
├── tests/basic.spec.ts           # 19 UI fundamental tests
├── tests/editor-features.spec.ts # 31 editor feature tests
├── tests/file-operations.spec.ts # 18 file/tag operation tests
├── tests/navigation.spec.ts      # 17 navigation + SPA tests
├── tests/websocket.spec.ts       # 19 WS + collaboration tests
└── playwright.config.ts          # 3-mode config: local / nix sandbox / VM test

nix/tests/                   # NixOS VM integration tests
├── serve-test.nix           # HTTP API test (curl, ~15 assertions)
├── e2e-test.nix             # Chromium --dump-dom DOM test (~10 assertions)
├── integration-test.nix     # Full cli_integration suite including serve_tests
└── playwright-e2e-test.nix  # 4-VM full Playwright (208 interactive tests)

tests/
└── cli_integration.rs   # ~93 integration tests
```

## Documentation

- [ARCHITECTURE.md](ARCHITECTURE.md) — System architecture and design decisions
- [WEB.md](WEB.md) — Web interface documentation
- [web/README.md](web/README.md) — Wire protocol and frontend details
- [docs/](docs/) — Feature documentation (chronological)
- [AGENTS.md](AGENTS.md) — AI agent development guidelines

## License

MIT OR Apache-2.0
