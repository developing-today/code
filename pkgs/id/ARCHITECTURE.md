# Architecture

System architecture for the `id` peer-to-peer platform and Rust facade.

## Overview

`id` is a peer-to-peer collaboration platform built on [Iroh](https://iroh.computer). It composes a workspace of crates through the root facade: an Iroh-based file core (blob storage, metadata documents and peer networking) plus signed events, groups, the directory, currency services and the world runtime. The CLI, REPL and optional web UI consume those layers. The facade retains historical `id::` paths by re-exporting the split modules. The diagram below shows the file-core path, not the entire platform; the crate table and store-port boundaries describe the wider workspace.

```
┌──────────────────────────────────────────────────────────────────┐
│                        User Interface                            │
│  ┌──────────┐  ┌──────────┐  ┌───────────────────────────────┐   │
│  │   CLI    │  │   REPL   │  │         Web UI (opt)          │   │
│  │  (clap)  │  │ (rustyline│  │  Datastar + ProseMirror      │   │
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
│  │ (P2P RPC) │  │ (blobs)   │  │ (metadata) │  │(gossip/mDNS) │  │
│  └─────┬─────┘  └─────┬─────┘  └─────┬─────┘  └──────┬───────┘  │
├────────┴──────────────┴──────────────┴───────────────┴───────────┤
│                     Iroh Runtime                                 │
│  Blobs (content-addressed) · Docs (CRDT documents) · Gossip     │
│  QUIC transport · Hole punching · Relay servers                  │
└──────────────────────────────────────────────────────────────────┘
```

## Public API and Crate Surface

Crates in `pkgs/id/crates/` and the root `id` package, classified by
dependents (checked against `Cargo.toml` and a repo-wide grep; no `id-*`
crate is consumed outside `pkgs/id/`).

| Crate                                                                                             | Classification            | Notes                                                                                                                                                                                                                                                   |
| ------------------------------------------------------------------------------------------------- | ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `id` (root, `src/`)                                                                               | Public facade and binary  | Re-exports the workspace API at historical `id::` paths; command parsing and dispatch are implemented in `id-cli`.                                                                                                                                      |
| `id-core`                                                                                         | Shared core               | Depended on by nine sibling crates and the root. The facade re-exports a selected subset; it also includes `admission_gate`, `migration_control`, `petname`, `world_presentation`, and feature-gated `qr`. See `src/lib.rs` for the authoritative list. |
| `id-caps`, `id-store`, `id-events`, `id-groups`, `id-sync`, `id-worlds`, `id-directory`, `id-web` | Internal                  | Used only by sibling crates and the root.                                                                                                                                                                                                               |
| `id-cli`                                                                                          | Internal (binary)         | Only the root depends on it.                                                                                                                                                                                                                            |
| `id-config`                                                                                       | Internal (dev only)       | The facade lists it as a dev-dependency for invariant tests (`tests/config_invariants.rs`); it has no production dependent.                                                                                                                             |
| `id-currency-registry`                                                                            | Internal                  | Only the root depends on it.                                                                                                                                                                                                                            |
| `id-native-shell`, `id-hw-shell`                                                                  | Internal (app)            | Workspace members, but not default members; both production-depend on the facade, and nothing depends on them. `id-native-shell` rejects `--help` and unknown flags; it has no `--version` option.                                                      |
| `id-render`                                                                                       | Internal compiler library | Rendering-pipeline compiler core: semantic-markup parsing and vocabulary validation, with composition/lowering/emission stages. Nothing depends on it.                                                                                                  |
| `id-import-notion`                                                                                | Internal importer library | Notion importer manifest and inventory primitives, with maintained integration tests. Nothing depends on it.                                                                                                                                            |

Standalone examples and benchmarks (`examples/`, `benchmarks`) are not
public API. Vendored crates (`vendor/p2panda-sync`, `vendor/p2panda-encryption`)
are third-party code.

Rustdoc coverage on public items is about 99% in `id-core` and the root
crate. `missing_docs = "warn"` is set in both `Cargo.toml` files. DES-3: this chapter's module map once followed the pre-split single-crate layout and pointed at paths that no longer exist; it now follows the workspace layout at `06450991e33` (`crates/id-cli/src/cli/`, `crates/id-sync/src/protocol.rs`, `crates/id-store/src/store.rs`, `crates/id-core/src/tuple.rs`, `crates/id-web/src/`, `web/src/`).

## Core Modules

### Entry Point (`src/main.rs`, `src/lib.rs`)

`main.rs` starts the facade binary; command parsing and dispatch are implemented in `id-cli` and re-exported at the historical facade paths. Node services are composed by the current command/server layers.

### CLI (`crates/id-cli/src/cli/`)

Defines the command-line interface using [clap](https://docs.rs/clap). All commands, subcommands, options, and aliases are declared here (`mod.rs` plus one module per command group, such as `world.rs`, `groups.rs` and `walk.rs`). The `tag` command accepts aliases `label` and `link`.

### Commands (`crates/id-cli/src/commands/`)

Implements the command handlers the parser dispatches to; see [Command Layer](#command-layer).

### Protocol (`crates/id-sync/src/protocol.rs`)

Defines the P2P request/response protocol between nodes. Uses Iroh's RPC mechanism with custom `MetaRequest`/`MetaResponse` types serialized as postcard-encoded bytes. The ALPN is `/iroh-meta/3` (`META_ALPN`).

Key message types:

- `Put`/`Get` — file transfer
- `List` — enumerate remote files
- `Find`/`Search` — pattern-based file lookup
- `Rename`/`Copy`/`Delete` — file management
- `SetTag`/`DelTag`/`GetTags`/`SearchTags` — metadata operations (SearchTags uses structured query syntax)
- `MigrateTags` — add name/file auto-tags to all existing files

### Store (`crates/id-store/src/store.rs`)

Wraps Iroh's blob store for named file operations. Maps human-readable names to content hashes via Iroh tags. Handles import from filesystem, export, and content retrieval.

### Tags (`crates/id-store/src/tags.rs`, `crates/id-core/src/tuple.rs`)

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

### Discovery (`crates/id-core/src/discovery.rs`)

Peer discovery with three mechanisms that coexist: gossip over a shared topic (the wide-area default), mDNS on the local network (on by default), and mainline DHT address lookup and publishing (off by default, enabled with `id serve --dht` behind the `dht` cargo feature). The module holds the announcement types and the in-memory peer table; the hardcoded fallbacks are `DEFAULT_TOPIC` (`id-peer-discovery-v1`) and `DEFAULT_TOPIC_SECRET` (`id-public-discovery-v1`), with `defaults.json` the operative source on a stock serve. It maintains a peer list and provides lookup by node ID prefix.

### Helpers (`crates/id-cli/src/helpers.rs`)

Utility functions for hash parsing, node ID validation, file size formatting, timestamp handling, and output formatting.

## Command Layer

Each command in `crates/id-cli/src/commands/` follows a consistent pattern:

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

## REPL (`crates/id-cli/src/repl/`)

The interactive REPL (`runner.rs`) provides a shell-like experience:

- **Command dispatch**: Maps input to command handlers with alias resolution
- **Shell features** (`input.rs`): Pipe to shell (`|>`), subshell capture (`$()`), backtick expansion, heredocs (`<<<`, `<<EOF`), input redirection
- **Alias groups**: `tag`/`label`/`link` are interchangeable prefixes; `tags`/`labels`/`links` for listing

## Web UI (`crates/id-web/src/`, `web/src/`)

Feature-gated behind `--features web`. Embeds a full browser UI in the binary.

### Backend (`crates/id-web/src/`)

| Module            | Purpose                                            |
| ----------------- | -------------------------------------------------- |
| `routes.rs`       | Axum HTTP handlers: file CRUD, rename, copy, tags  |
| `templates.rs`    | Server-side HTML rendering (no template engine)    |
| `collab.rs`       | WebSocket server for collaborative editing         |
| `tags_ws.rs`      | WebSocket broadcast for tag change events          |
| `assets.rs`       | Static asset serving via rust-embed                |
| `content_mode.rs` | Content type detection and rendering mode          |
| `markdown.rs`     | Markdown → HTML rendering with syntax highlighting |

### Frontend (`web/src/`)

TypeScript bundled with Bun, producing a single JS file and CSS file embedded in the binary.

| File              | Purpose                                         |
| ----------------- | ----------------------------------------------- |
| `main.ts`         | Entry point, SPA navigation, file operations    |
| `input.css`       | TailwindCSS v4 + DaisyUI entry (themes, styles) |
| `editor.ts`       | ProseMirror editor setup, schema, menu          |
| `collab.ts`       | WebSocket collaboration client, MessagePack     |
| `cursors.ts`      | Cursor/selection plugin with opacity fading     |
| `theme.ts`        | Theme switching (sneak/arch/mech)               |
| `search-panel.ts` | Find & replace panel (Ctrl+F / Ctrl+H)          |
| `highlight.ts`    | Syntax highlighting via Shiki (50+ languages)   |
| `cursor-utils.ts` | Cursor opacity, color, tooltip clustering       |
| `goto-line.ts`    | Go-to-line dialog (Ctrl+G)                      |
| `indent.ts`       | Tab indentation (2-space)                       |
| `active-line.ts`  | Active line highlight plugin                    |
| `wrap.ts`         | Word wrap toggle (Alt+Z)                        |

### Collaboration Protocol

Real-time collaborative editing uses ProseMirror's `prosemirror-collab` plugin over WebSocket with binary MessagePack encoding. See [web/README.md](web/README.md) for the wire protocol specification.

### Content Viewers

The web UI renders files differently based on content type:

| Content Type | Viewer        | Features                              |
| ------------ | ------------- | ------------------------------------- |
| Text/Code    | ProseMirror   | Collaborative editing, save, download |
| Markdown     | ProseMirror   | Rich editing with menu bar            |
| Images       | Media viewer  | Inline display, download              |
| Video/Audio  | Media viewer  | Native player, download               |
| PDF          | Media viewer  | Embedded viewer, download             |
| Binary       | Binary viewer | Download only                         |

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

## Store Port Boundaries

Slice 9110-01 (ID/Atlas V2 port contract) seam map. Per the settled
design (`doc/2026-11-13T03-40-00Z_design_id-atlas-summit/crosscut/versioned-design.md`,
**Port contract**), port types are separate — record store, blob store,
event log, world store, object bucket, message stream, and transport are
distinct ports — and a single "store abstraction" does not merge them:
existing blob, event, world, and sync authorities remain separately
owned until a mapping is reviewed. An adapter either satisfies a port's
declared capabilities or returns a structured unsupported result; it
must not silently emulate weaker semantics, and an adapter-specific API
stays behind its own port boundary.

### Seam-map classification

| Port  | Owning crate / seam (consumer-owned files)                                            | Concrete types                                                                       | Backing storage                                                                                                     | Authority                                                  |
| ----- | ------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| blob  | `id-store` — `crates/id-store/src/store.rs`                                           | `StoreType` (`Persistent(FsStore)` / `Ephemeral(MemStore)`), `StoreType::as_store()` | iroh-blobs file store: redb metadata plus blob data files under `.iroh-store/` (`STORE_PATH`); `MemStore` in memory | content-addressed blob bytes and name-to-hash tags         |
| event | `id-events` — `crates/id-events/src/event_store/mod.rs`                               | `EventStore`, behind the `Storage` / `FileStorage` file seam                         | one separate redb file per store directory                                                                          | signed event log: chains, heads, fork proofs, waiting pool |
| world | `id-worlds` — `crates/id-worlds/src/world_store.rs`                                   | `FileJournal`, `ModuleDir`                                                           | fsynced `journal.jsonl` plus content-addressed `modules/<blake3>.wasm`                                              | durable world state: journal replay and installed modules  |
| sync  | `id-sync` — `crates/id-sync/src/envelope_outbox.rs`, `crates/id-sync/src/log_sync.rs` | `Outbox`, `SyncProtocol`                                                             | append-only outbox journal file, replayed on open                                                                   | pending envelope delivery only; no stored-data authority   |

Non-authority is part of the boundary: the event store decides nothing
about blob bytes, the world journal decides nothing about event-log
heads, and sync owns no stored data at all — it moves envelopes and log
entries between the other ports and records only what is still pending
delivery. Treating any row's backing storage (redb, journal files) as
the identity of the port is the flattening error this map exists to
prevent: the blob and event ports both use redb today and remain
different ports with different authorities.

### Consumer-owned files and compatibility adapters

Consumer-owned files for this seam map (owner: 9110-01):
`pkgs/id/ARCHITECTURE.md` (this section) and
`pkgs/id/tests/store_port_boundary.rs` (the boundary fixture/test). The
consuming facade is the root `id` package: its `Cargo.toml` depends on
`id-store`, `id-events`, `id-worlds`, and `id-sync` as four separate
path dependencies, and no crate in that list re-exports another port's
store type as its own.

Compatibility adapters that exist today, each confined to one port:

- **Blob:** `StoreType::as_store()` is the compatibility adapter over
  iroh-blobs `FsStore`/`MemStore`, yielding one iroh-blobs `Store`
  handle. It unifies two backends of the _same_ blob port only.
- **Event:** `id-store::store_traits::EventStoreBackend`, implemented
  by `SharedEventStore<S>`, adapts the event store for in-fleet
  consumers. Event authority stays in `id-events`; the trait is a
  consumer seam, not a transfer of ownership.
- **Event (file seam):** the `Storage` trait in
  `id-events::event_store` adapts real directories (`FileStorage`) and
  the crash model's simulated directory behind one file interface.
- **Sync:** `Outbox::open` replays its journal to rebuild pending
  delivery state, adapting an append-only file to the delivery queue
  other sync code consumes.

A new adapter must declare the single port it serves. A trait or type
that claims two rows of the classification table — one kind owning two
authorities, or one authority answering for two kinds — is a
flattening, and `tests/store_port_boundary.rs` rejects it.

## Build System

### Nix Integration

The project uses Nix flakes for reproducible builds:

- `flake.nix` — packages (`id-web`, `id-lib`), checks (the CI set plus the NixOS VM checks; 50 attributes under `checks.x86_64-linux` when measured on 2026-11-23 at `06450991e33` with `nix eval '.#checks.x86_64-linux' --apply 'x: builtins.length (builtins.attrNames x)'` in `pkgs/id`), apps (all justfile commands)
- `nix-common.nix` — shared build inputs and dev shell packages
- `rust-toolchain.toml` — Rust version pinning (read by rust-overlay)
- `web/bun.nix` — offline npm dependency fetching via bun2nix

### Build Variants

| Variant | Feature Flag | Assets Required | Output       |
| ------- | ------------ | --------------- | ------------ |
| `lib`   | (none)       | No              | CLI binary   |
| `web`   | `web`        | `web/dist/`     | CLI + web UI |

The build script (`scripts/build.sh`) tracks the current variant beside its Cargo artifacts: under `CARGO_TARGET_DIR` when set (including a broker-leased shared target), or under `target/` otherwise.

## Testing

| Layer                   | Framework         | Command                          |
| ----------------------- | ----------------- | -------------------------------- |
| Unit                    | `cargo test`      | `just test-unit`                 |
| Integration             | `cargo test`      | `just test-int`                  |
| TypeScript              | vitest            | `just test-web-unit`             |
| E2E                     | Playwright        | `just test-e2e`                  |
| NixOS VM (API)          | curl/Python       | `just test-nixos-serve`          |
| NixOS VM (DOM)          | Chromium          | `just test-nixos-e2e`            |
| NixOS VM (Integration)  | cargo test binary | `just test-nixos-integration`    |
| NixOS 4-VM (Playwright) | Playwright        | `just test-nixos-playwright-e2e` |

Test totals are deliberately not stated: a count is a measurement of one run on one date, and this file records it with its date or not at all. To produce a current count for one layer, run its command and record the summary line with the date and the commit.

`just test-nix` (`nix flake check`, 50 checks on `x86_64-linux` when measured on 2026-11-23 by the method above) runs everything — all test layers above are included. See [`doc/testing-architecture`](../../doc/2026-03-29T00-00-00Z_reference_testing_architecture/2026-03-29T00-00-00Z_reference_testing_architecture.md) for the complete testing reference.
