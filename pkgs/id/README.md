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
cargo run --features web -- serve --web 3000
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

# Start a server
id serve                          # CLI only
id serve --web 3000               # With web UI
id serve --web 3000 --ephemeral   # Ephemeral mode

# Interactive REPL
id repl

# Remote operations (prefix with node ID)
id put <node_id> myfile.txt
id list <node_id>

# Identity and peers
id id                   # Print local node ID
id peers                # List discovered peers
```

## CLI Commands

| Command        | Aliases                        | Description                                     |
| -------------- | ------------------------------ | ----------------------------------------------- |
| `serve`        |                                | Start server for peer requests                  |
| `repl`         | `shell`                        | Interactive REPL with pipes and subshells       |
| `put`          | `in`, `add`, `store`, `import` | Store files in blob store                       |
| `put-hash`     |                                | Store content by hash only (no named tag)       |
| `get`          |                                | Retrieve files by name or hash                  |
| `get-hash`     |                                | Retrieve file by hash with explicit output path |
| `cat`          | `output`, `out`                | Output file content to stdout                   |
| `show`         | `view`                         | Find file by pattern and output content         |
| `peek`         |                                | Preview with configurable head/tail lines       |
| `find`         |                                | Find files by query, optionally output content  |
| `search`       |                                | Search files and list all matches               |
| `list`         | `ls`                           | List all stored files                           |
| `tag`          | `label`, `link`                | Manage metadata tags on files                   |
| `migrate-tags` | `migrate`                      | Add name/file auto-tags to existing files       |
| `id`           |                                | Print local node public ID                      |
| `peers`        |                                | Discover and list known peers                   |

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

This provides: Rust 1.89.0, clippy, rustfmt, cargo-llvm-cov, cargo-audit, cargo-machete, just, Bun, TypeScript, chromium, firefox.

### Testing

```bash
just test              # All fast tests (Rust + TypeScript unit + typecheck)
just test-rust         # Rust tests only (~500 unit + ~85 integration)
just test-unit         # Unit tests only (fast, ~500 tests)
just test-integration  # Integration tests only (~85 tests)
just test-web-unit     # TypeScript unit tests (~116 assertions)
just test-e2e          # Playwright E2E (chromium + firefox, 146 tests)
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
├── src/                 # ProseMirror editor, collab, themes
└── dist/                # Built assets (embedded in binary)

e2e/                     # Playwright E2E tests (38 tests × 2 browsers = 146)
├── tests/basic.spec.ts      # 19 UI fundamental tests
├── tests/websocket.spec.ts  # 19 WS + collaboration tests
└── playwright.config.ts     # 3-mode config: local / nix sandbox / VM test

nix/tests/                   # NixOS VM integration tests
├── serve-test.nix           # HTTP API test (curl, ~15 assertions)
├── e2e-test.nix             # Chromium --dump-dom DOM test (~10 assertions)
├── integration-test.nix     # Full cli_integration suite including serve_tests
└── playwright-e2e-test.nix  # 4-VM full Playwright (146 interactive tests)

tests/
└── cli_integration.rs   # ~85 integration tests
```

## Documentation

- [ARCHITECTURE.md](ARCHITECTURE.md) — System architecture and design decisions
- [WEB.md](WEB.md) — Web interface documentation
- [web/README.md](web/README.md) — Wire protocol and frontend details
- [docs/](docs/) — Feature documentation (chronological)
- [AGENTS.md](AGENTS.md) — AI agent development guidelines

## License

MIT OR Apache-2.0
