# Architecture

System architecture for the `id` peer-to-peer file sharing CLI.

## Overview

`id` is built on [Iroh](https://iroh.computer), a peer-to-peer networking library that provides content-addressed blob storage and document synchronization. The application layers a CLI interface, interactive REPL, and optional web UI on top of Iroh's primitives.

```
┌──────────────────────────────────────────────────────────────────┐
│                        User Interface                            │
│  ┌──────────┐  ┌──────────┐  ┌───────────────────────────────┐   │
│  │   CLI    │  │   REPL   │  │         Web UI (opt)          │   │
│  │  (clap)  │  │ (rustyline│  │  Axum + HTMX + ProseMirror   │   │
│  └────┬─────┘  │ + shell) │  └──────────────┬────────────────┘   │
│       │        └────┬─────┘                 │                    │
├───────┴─────────────┴─────────────────────┬─┘────────────────────┤
│                   Command Layer                                  │
│  put · get · find · list · cat · show · peek · search · tag      │
│  serve · repl · peers · id                                       │
├──────────────────────────────────────────────────────────────────┤
│                   Core Services                                  │
│  ┌───────────┐  ┌───────────┐  ┌───────────┐  ┌──────────────┐  │
│  │  Protocol │  │   Store   │  │  TagStore  │  │  Discovery   │  │
│  │ (P2P RPC) │  │ (blobs)   │  │ (metadata) │  │  (mDNS/DHT) │  │
│  └─────┬─────┘  └─────┬─────┘  └─────┬─────┘  └──────┬───────┘  │
├────────┴──────────────┴──────────────┴───────────────┴───────────┤
│                     Iroh Runtime                                 │
│  Blobs (content-addressed) · Docs (CRDT documents) · Gossip     │
│  QUIC transport · Hole punching · Relay servers                  │
└──────────────────────────────────────────────────────────────────┘
```

## Core Modules

### Entry Point (`main.rs`, `lib.rs`)

`main.rs` parses CLI args and dispatches to command handlers. `lib.rs` exports the library API and contains `start_node()` which bootstraps an Iroh node with blob storage, document sync, and peer discovery.

### CLI (`cli.rs`)

Defines the command-line interface using [clap](https://docs.rs/clap). All commands, subcommands, options, and aliases are declared here. The `tag` command accepts aliases `label` and `link`.

### Protocol (`protocol.rs`)

Defines the P2P request/response protocol between nodes. Uses Iroh's RPC mechanism with custom `MetaRequest`/`MetaResponse` types serialized as postcard-encoded bytes.

Key message types:
- `Put`/`Get` — file transfer
- `List` — enumerate remote files
- `Find`/`Search` — pattern-based file lookup
- `Rename`/`Copy`/`Delete` — file management
- `SetTag`/`DelTag`/`GetTags`/`SearchTags` — metadata operations (SearchTags uses structured query syntax)
- `MigrateTags` — add name/file auto-tags to all existing files

### Store (`store.rs`)

Wraps Iroh's blob store for named file operations. Maps human-readable names to content hashes via Iroh tags. Handles import from filesystem, export, and content retrieval.

### Tags (`tags.rs`, `tuple.rs`)

The metadata tag system uses a dual-index architecture for efficient queries:

```
Alpha Index: subject → key → value    (lookup tags for a file)
Omega Index: key → value → subject    (search files by tag)
```

Both indices are stored as Iroh documents using sort-preserving binary key encoding (`tuple.rs`). Each index is a `NamespacePair` — an alpha doc and omega doc that mirror each other for bidirectional queries.

Key concepts:
- **TagValue**: Binary-safe value type wrapping `Vec<u8>`, displays as UTF-8 when valid, `<binary N bytes>` otherwise. Supports arbitrary-length keys/values.
- **Tag**: `(subject, key, value?)` triple — e.g., `("readme.md", "author", "Jane")`
- **NamespacePair**: Two Iroh docs (alpha + omega) for dual-indexed queries
- **Registry**: JSON file mapping namespace names to Iroh document IDs
- **TagEvent**: Broadcast events (`Set`, `Del`, `DelAll`, `Transfer`) for WebSocket subscribers
- **TupleEncoder**: Sort-preserving binary encoding with smart type selection (UTF-8 → `string()` type 0x02 for backward compat, non-UTF-8 → `bytes()` type 0x01)
- **SearchQuery**: Structured search parser — `key:`, `:value`, `key:value`, `"literal"`, bare words. Multiple terms ANDed together.
- **Auto-tagging**: Files automatically get `name`, `file`, and optionally `path` metadata tags on upload via `auto_tag()`. Existing tags never overwritten (`set_if_absent`).
- **Migration**: `migrate_tags()` adds auto-tags to all existing files that lack them.

Operations: `set_tag`, `del_tag`, `get_tags`, `search_by_query` (structured syntax), `find_by_key`, `find_by_value`, `find_by_key_value`, `transfer_all_tags`, `copy_all_tags`, `auto_tag`, `migrate_tags`.

Display: Values truncated at 256 bytes by default. CLI supports `--hex`, `--binary`, `--no-truncate` flags. Web UI shows `name`/`file`/`path` tags as display names and deduplicates files with identical (hash, display_name, tags).

### Discovery (`discovery.rs`)

Peer discovery using mDNS for local network and Iroh's DHT for global discovery. Maintains a peer list and provides lookup by node ID prefix.

### Helpers (`helpers.rs`)

Utility functions for hash parsing, node ID validation, file size formatting, timestamp handling, and output formatting.

## Command Layer

Each command in `src/commands/` follows a consistent pattern:

1. Parse arguments (from CLI or REPL)
2. Determine local vs. remote operation (check if first arg is a 64-char hex node ID)
3. Open local store or connect to remote peer
4. Execute operation
5. Format and display results

### Local vs. Remote

Commands transparently work against local or remote nodes:

```bash
id list                    # Local
id list a1b2c3d4...        # Remote (64-char hex node ID)
```

Remote operations serialize the request via `protocol.rs` and send it over Iroh's QUIC transport.

## REPL (`repl/`)

The interactive REPL (`runner.rs`) provides a shell-like experience:

- **Command dispatch**: Maps input to command handlers with alias resolution
- **Shell features** (`input.rs`): Pipe to shell (`|>`), subshell capture (`$()`), backtick expansion, heredocs (`<<<`, `<<EOF`), input redirection
- **Alias groups**: `tag`/`label`/`link` are interchangeable prefixes; `tags`/`labels`/`links` for listing

## Web UI (`src/web/`, `web/`)

Feature-gated behind `--features web`. Embeds a full browser UI in the binary.

### Backend (`src/web/`)

| Module          | Purpose                                           |
|-----------------|---------------------------------------------------|
| `routes.rs`     | Axum HTTP handlers: file CRUD, rename, copy, tags |
| `templates.rs`  | Server-side HTML rendering (no template engine)    |
| `collab.rs`     | WebSocket server for collaborative editing         |
| `tags_ws.rs`    | WebSocket broadcast for tag change events          |
| `assets.rs`     | Static asset serving via rust-embed                |
| `content_mode.rs` | Content type detection and rendering mode        |
| `markdown.rs`   | Markdown → HTML rendering with syntax highlighting |

### Frontend (`web/`)

TypeScript bundled with Bun, producing a single JS file and CSS file embedded in the binary.

| File          | Purpose                                    |
|---------------|--------------------------------------------|
| `main.ts`     | Entry point, HTMX init, file operations    |
| `editor.ts`   | ProseMirror editor setup                   |
| `collab.ts`   | WebSocket collaboration client             |
| `cursors.ts`  | Cursor/selection plugin with opacity fading |
| `theme.ts`    | Theme switching (sneak/arch/mech)           |

### Collaboration Protocol

Real-time collaborative editing uses ProseMirror's `prosemirror-collab` plugin over WebSocket with binary MessagePack encoding. See [web/README.md](web/README.md) for the wire protocol specification.

### Content Viewers

The web UI renders files differently based on content type:

| Content Type | Viewer          | Features                              |
|-------------|-----------------|---------------------------------------|
| Text/Code    | ProseMirror     | Collaborative editing, save, download |
| Markdown     | ProseMirror     | Rich editing with menu bar            |
| Images       | Media viewer    | Inline display, download              |
| Video/Audio  | Media viewer    | Native player, download               |
| PDF          | Media viewer    | Embedded viewer, download             |
| Binary       | Binary viewer   | Download only                         |

All viewers include rename and copy buttons.

## Data Model

### Files

Files are stored as content-addressed blobs in Iroh's blob store. Named references (tags) map human-readable names to content hashes:

```
"readme.md" → Hash(abc123...)    (Iroh tag)
"photo.jpg" → Hash(def456...)    (Iroh tag)
```

### Metadata Tags

Arbitrary key-value pairs attached to file names:

```
Tag("readme.md", "author", "Jane")
Tag("readme.md", "category", "docs")
Tag("photo.jpg", "location", "Paris")
```

Stored in dual-indexed Iroh documents for O(1) lookup in both directions.

### Archive System

Deleted and renamed files are archived rather than destroyed:

```
"old-name.archive.1711234567" → Hash(...)     (archived original)
"target.archive.1711234567"   → Hash(...)     (archived replaced file)
```

Archive tags include metadata recording the operation type (`archive.rename`, `archive.replace`).

## Build System

### Nix Integration

The project uses Nix flakes for reproducible builds:

- `flake.nix` — packages (`id-web`, `id-lib`), checks (11 CI checks), apps (all justfile commands)
- `nix-common.nix` — shared build inputs and dev shell packages
- `rust-toolchain.toml` — Rust version pinning (read by rust-overlay)
- `web/bun.nix` — offline npm dependency fetching via bun2nix

### Build Variants

| Variant | Feature Flag | Assets Required | Output          |
|---------|-------------|-----------------|-----------------|
| `lib`   | (none)      | No              | CLI binary      |
| `web`   | `web`       | `web/dist/`     | CLI + web UI    |

The build script (`scripts/build.sh`) tracks the current variant in `target/.build-variant` to detect when a rebuild is needed due to variant change.

## Testing

| Layer         | Framework     | Command              | Count |
|---------------|---------------|----------------------|-------|
| Unit          | `cargo test`  | `just test-unit`     | 484   |
| Integration   | `cargo test`  | `just test-int`      | 64    |
| TypeScript    | `bun test`    | `just test-web-unit` | —     |
| E2E           | Playwright    | `just test-e2e`      | 15    |

Integration tests that require network (serve_tests) are skipped in sandbox environments. Playwright E2E tests run against both Chromium and Firefox.
