# Repository layout

Source: `pkgs/id/Cargo.toml`, `pkgs/id/flake.nix`, `pkgs/id/book.toml` and `pkgs/id/docs.nix` in https://github.com/developing-today/code/blob/main/pkgs/id/Cargo.toml — provenance only; this chapter is complete without following it.

ID lives in the `developing-today/code` monorepo, in the directory `pkgs/id/`. That directory is a Cargo workspace, a Nix sub-flake, and the root of this book, all at once. Everything a contributor needs for ID is under `pkgs/id/`, except the design-record archive, which sits at the repository root (see "The design archive" below).

## The root crate

`pkgs/id/Cargo.toml` is two things: the package manifest for the root crate, named `id`, version `0.1.0`, edition 2024, and the workspace root manifest.

| Path                   | What it holds                                                                                                                                                                                                                                                                               |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/main.rs`          | The `id` binary entry point. It consumes the library through `id::` paths, the same surface an external consumer uses.                                                                                                                                                                      |
| `src/lib.rs`           | The facade crate: 26 modules of first-party code (among them `backup`, `currency`, `fileops`, `grants`, `hosted_binding`, `hosted_export`, `hosted_ingest` and `structured_logging`) plus the re-export surface over the sub-crates. The facade is not a re-export shim only; it owns code. |
| `src/` (rest)          | The facade-owned modules, one file or directory per module. Several files at historical paths are thin layers over the split crates that now own the code.                                                                                                                                  |
| `build.rs`             | The root package's build script. It handles the web asset directory `web/dist`: a release build with the `web` feature fails when that directory is missing or empty. See [Building and testing](building.md).                                                                              |
| `build.sh`, `scripts/` | The build and maintenance scripts the justfile recipes call, including `scripts/build.sh`, the Roc test and example rebuild scripts, and the test-sharding script.                                                                                                                          |

There is no `src/bin/` directory and no `[[bin]]` section in any workspace `Cargo.toml`. The two binaries exist by Cargo's implicit `src/main.rs` rule: `id` from the root package, and `id-native-shell` from `crates/id-native-shell/src/main.rs`.

## The 19 crates

`crates/` holds the 19 sub-crates. The workspace has 20 members in total: the root plus these 19. `default-members` lists 17 packages — the root plus 16 crates. `id-native-shell`, `id-hw-shell`, and `id-tui-shell` are members but not default members, so a plain `cargo build` in `pkgs/id/` never builds the shell crates.

| Crate                  | Role in one line                                                                                                                                                                                             |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `id-core`              | Foundation leaf modules: addressing, frames, discovery, membership, the graph record model. Depended on by every other split crate.                                                                          |
| `id-caps`              | Capability catalog, execution limits, the world manifest, and the MCP bridge translator and policy point.                                                                                                    |
| `id-config`            | The versioned configuration contract: the V2 `ConfigDocument`, its hard invariants, and the deterministic resolver. No production crate depends on it; the facade uses it in dev-dependencies for its tests. |
| `id-store`             | Blob store, chains, documents, tags and search.                                                                                                                                                              |
| `id-events`            | Event store, group projection, writer sets and sync gating.                                                                                                                                                  |
| `id-groups`            | Group policy and enforcement remainder over the model in `id-events`: blob gate, group encryption, group actions and log.                                                                                    |
| `id-sync`              | Log sync, the meta protocol, envelopes, signed artifacts and FileOps transfer.                                                                                                                               |
| `id-directory`         | Home-server directory, views, auth, mail, invites, keysets and seitan admission.                                                                                                                             |
| `id-worlds`            | World runtime, sessions, the wasmtime sandbox, the shell contract and the program gallery. The largest crate in the workspace.                                                                               |
| `id-web`               | The HTTP and WebSocket server: routes, templates and markdown for the web UI.                                                                                                                                |
| `id-cli`               | The application layer: the clap parser in `src/cli/`, the command handlers in `src/commands/`, the REPL, and the server-config loader.                                                                       |
| `id-currency-registry` | Currency control-plane registry: the ledger allocator, the account factory and the Postgres schema and migrations.                                                                                           |
| `id-render`            | Rendering pipeline compiler core: parse and vocabulary validation of semantic markup. No dependents.                                                                                                         |
| `id-import-notion`     | Notion importer manifest and inventory primitives. No dependents.                                                                                                                                            |
| `id-client`             | Client-only Rust facade for the id native Android client v1 (slice 1 of #7938, #13403).                                                                                                                     |
| `id-pg`                 | Postgres-over-HTTP adapter (#5805): the PostgREST query translator and collection allowlist (design §3), slice #13760. No dependents yet.                                                                   |
| `id-native-shell`      | The Bevy native reference shell. Not a default member.                                                                                                                                                       |
| `id-hw-shell`          | The ESP32-class hardware shell spike: one file, `src/lib.rs`, frame builders only, no binary. Not a default member.                                                                                          |
| `id-tui-shell`          | The terminal (TUI) shell: the text-native world-session client on the F13.1 shell contract (#12818). Not a default member.                                                                                  |

Dependency direction runs downward to `id-core`. Nothing in the split family depends on the facade in production code; the two original shells depend on the facade, and `id-store` dev-depends on it for tests. The full per-crate surface is [The crates](../library/crates.md).

Two directories sit outside the workspace: `vendor/p2panda-sync` and `vendor/p2panda-encryption` are listed in the workspace `exclude` array and are consumed as path dependencies only.

## Apps, examples and the guest surface

| Path            | What it holds                                                                                                                                                                                                                                                              |
| --------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `apps/`         | Packaged apps. It contains exactly one app: `apps/tic-tac-toe/`, with an `app.toml` declaring `[program] kind = "roc-world"` and the guest source in `program/main.roc`. It contains no Rust.                                                                              |
| `examples/`     | The Roc guest apps, the shared Roc platform package, the Rust guest SDK and its example, the import-policy fixture matrix, and one standalone Rust example file, `examples/session_checkpoint_poc.rs`. The guest apps are listed in [Add a Roc guest app](add-a-guest.md). |
| `contract/`     | The machine-readable shell contract, 4 files: `shell-contract.v1.json` (contract `id-shell`, version 1, revision 15), `shell-suite.v1.json`, and the two golden session transcripts `sessions/terminal.jsonl` and `sessions/native.jsonl`.                                 |
| `config/`       | The JSON Schema twin of the `id-config` document, `config/id-config-v1.schema.json`.                                                                                                                                                                                       |
| `test-vectors/` | Exactly 3 files: `schema.json` (the ID/Atlas shared conformance test-vector schema), `atlas-content-hash-v1.json` and `legacy-artifact-signatures.json`.                                                                                                                   |
| `benches/`      | The benchmark harness: `world_bench.rs` plus its baseline files. A separate `benchmarks/` directory holds benchmark notes and results.                                                                                                                                     |
| `tests/`        | The root package's integration tests, including `cli_integration.rs`, the `config_*` tests for `id-config`, and the MCP bridge and spike end-to-end tests. Each sub-crate also carries its own `tests/` directory.                                                         |
| `e2e/`          | The Playwright end-to-end suite for the web UI, with its own `package.json`, `bun.lock` and `playwright.config.ts`.                                                                                                                                                        |

## The web frontend and the hosted scaffold

| Path             | What it holds                                                                                                                                                                                                                                                                                                         |
| ---------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `web/`           | The web frontend: Bun plus TypeScript, package name `id-web`. Its build output lands in `web/dist`, which is embedded into the `id-web` crate at compile time with rust-embed. Only a tracked `.gitkeep` file is committed under `web/dist`; the assets are a build product. See [Building and testing](building.md). |
| `hosted-worker/` | The hosted edition's Cloudflare Worker scaffold: `wrangler.toml`, `src/` (front door, routing, the per-user Durable Object, provisioning, the currency gateway proof of concept) and `test/`. It is a local-only scaffold, not a deployed service. See [Release and rollout](release-and-rollout.md).                 |
| `worker-poc/`    | A proof of concept that runs one example world in a Worker isolate, plus its `d1-journal-poc/` subfolder. Not a production path.                                                                                                                                                                                      |

## Build and project files at the package root

| Path                                                 | What it is                                                                                                                                                                                                                                             |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `flake.nix`                                          | The sub-flake: dev shell, the checks set, the packages `id-web` and `id-lib`, and apps generated from `just-recipes.json`.                                                                                                                             |
| `justfile`                                           | The command runner recipes. Every build, test and serve command in [Building and testing](building.md) is a recipe here or a Cargo command the recipes wrap.                                                                                           |
| `just-recipes.json`                                  | A generated projection of the justfile, regenerated by the `lockfiles` recipe and checked for drift by a flake check. Do not edit it by hand.                                                                                                          |
| `rust-toolchain.toml`                                | The toolchain pin: channel `1.99.1`, with the components `rustfmt`, `clippy`, `rust-analyzer` and `llvm-tools`. The flake reads it through rust-overlay.                                                                                               |
| `Cargo.lock`                                         | The workspace lockfile, consumed by both Cargo and the Nix packages.                                                                                                                                                                                   |
| `default.nix`, `shell.nix`, `nix/`, `nix-common.nix` | The non-flake and shared Nix entry points, and the NixOS VM test definitions under `nix/tests/`.                                                                                                                                                       |
| `README.md`, `ARCHITECTURE.md`, `WEB.md`, `TODO.md`  | The package's older prose documents. They predate parts of the current tree and disagree with it in places; this book documents the source where they differ. `TODO.md` states in its own text that it is idle planning and not a basis for decisions. |

## Where this book lives

| Path                              | What it is                                                                                                                                                                                                                                                                                                                                                                                                                               |
| --------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `book/`                           | This book's chapters, as plain Markdown, organised by the sections in `book/SUMMARY.md`. The book is readable in the repository exactly as written.                                                                                                                                                                                                                                                                                      |
| `book.toml`                       | The mdBook configuration at the package root: title `id`, source directory `book`, build directory `rendered`, and `create-missing = false` under `[build]`. That last setting is one of the two build gates in [Keeping this book current](keeping-this-book-current.md).                                                                                                                                                               |
| `docs.nix`                        | The Nix derivation that renders the book: it takes only `book.toml` and the `book/` tree as its source, rewrites links that escape the book tree to their repository URLs in the copied pages only, and bakes in a `siteUrl` argument that defaults to `/`. Its header states that it is exposed as the `.#id-docs` flake output.                                                                                                        |
| Repository-root `lib/default.nix` | The root flake's wiring for every sub-flake, ID included: its `make-id` function imports `pkgs/id/flake.nix`, re-keys its inputs, and merges its outputs into the root flake — the packages are inherited as-is, and the apps are exposed with an `id-` name prefix, with the `id` app itself running the sub-flake's default app. This is the wiring that makes the sub-flake's packages and checks reachable from the repository root. |

## The design archive

The design records for ID — and for every other programme in the repository — live in the `doc/` directory at the repository root: 2,917 paths in total, of which the book's research classified 408 top-level entries as ID-programme records.

Inside `pkgs/id/`, two symlinks point at that archive:

- `pkgs/id/doc` → `../../doc`
- `pkgs/id/docs` → `../../doc`

Both resolve to the same repository-root directory. There is no ID-private docs tree: the archive reached through `pkgs/id/docs` is shared with the Atlas, Fleet, PM-stack and home-cluster programmes, and two of its entries are themselves symlinks out of the archive, to hardware documentation. A third symlink, `pkgs/id/thoughts` → `../../thoughts/`, points at a second shared repository-root directory holding session-continuity ledgers and the early collaborative-web working papers.

The mapping from design records to this book's design chapters is in [Roadmap, issue map and the design-record index](../design/roadmap-and-issue-map.md).
