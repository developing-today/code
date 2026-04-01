# Agent Instructions for `id` Codebase

Guidelines for AI coding agents working on the `id` peer-to-peer file sharing CLI built with Rust and Iroh.

**Updating this file:** Keep prose tight and context-efficient. Prefer links to files over inline examples. Include only essential code samples.

## Critical: Preserving Unstaged Work

**NEVER use `git restore`, `git checkout -- <file>`, or any command that overwrites pre-existing unstaged changes.**

Only discard unstaged work if:

1. The user explicitly instructs you to discard it, OR
2. You ask and receive specific approval to do so

This applies to all files with uncommitted modifications—assume the user has intentional work in progress.

## Critical: Toolchain Files

**NEVER delete `rust-toolchain.toml`** - it is required for Nix builds. The flake.nix uses rust-overlay which reads this file. Deleting it breaks `nix develop` and `nix build`.

## Critical: Nix and Justfile Synchronization

**When adding or modifying justfile commands, ALWAYS add a corresponding `nix run .#<command>` app in `flake.nix`.**

This enables running commands without entering a dev shell (`nix run .#ci`), CI/CD pipelines with pure Nix evaluation, and reproducible execution across systems.

When adding a new just command:

1. Add the recipe to `justfile`
2. Add corresponding app in `flake.nix` `apps` section
3. For CI-verifiable commands, add a check in `flake.nix` `checks` section

**Package management:** Add new dev dependencies to `nix-common.nix` only (never directly to shell.nix or flake.nix). See `nix-common.nix` for the shared package architecture.

## OpenCode Plan Mode

When creating plans for this project:

1. **Write plans to `.opencode/plans/`** in the repo root (create directory if needed)
2. **After finalizing a plan**, add a first task to create a comprehensive docs file following the datetime documentation protocol in "Documenting Design & Architecture Decisions":
   - Create folder: `docs/<UTC_RFC_DATETIME>_<kind>_<name>/`
   - Create document explaining features, design, intent, and architecture in technical detail
   - Limit raw code listings; prefer explanations and file references
   - Near the top, add a relative markdown link to the source plan file (e.g., `See [original plan](../../.opencode/plans/<plan-file>.md)`)
   - Add a "References" section at the bottom that also links to the plan file
3. Plans in `.opencode/plans/` are working drafts; docs files are the comprehensive historical record

**Compaction priority:** During context compaction, active `docs/` and `.opencode/plans/` files should be kept near the top of context. If these files seem no longer relevant, ask the user before removing them from context or starting work without them.

## Environment Setup

Use the Nix dev shell for correct tool versions:

```bash
nix develop   # Preferred: flake-based
nix-shell     # Alternative: legacy
```

Includes: Rust (see `rust-toolchain.toml`), clippy, rustfmt, cargo-llvm-cov, cargo-audit, cargo-outdated, cargo-machete, just. Ignore Nix warnings about disk space or symlinks.

## Build Variants

The project has two build variants:

- **lib** - Rust CLI only (no web dependencies)
- **web** - Rust CLI with embedded web UI (requires Bun)

**Naming convention:** Simple command names default to web variant; use `-lib` suffix for library-only.

```bash
just build          # Build with web UI [requires bun]
just build-lib      # Build library only (no web)
just serve          # Serve with web UI [requires bun]
just serve-lib      # Serve without web UI
just release        # Release build with web UI [requires bun]
just release-lib    # Release build library only
```

**For non-just users (direct cargo/nix):**

```bash
# Library variant (no web) - works standalone
cargo build --no-default-features     # Debug build
cargo build --release --no-default-features  # Release build
nix build .#id-lib                    # Nix package (or: just build-nix-lib)

# Web variant - requires web assets built first
cd web && bun install && bun run build && cd ..  # Build web assets
cargo build                             # Debug build with embedded web UI
cargo build --release                   # Release build with embedded web UI
nix build                               # Nix package (or: just build-nix)
```

**Build variant tracking:** The build system tracks variants in `target/.build-variant` to detect when rebuild is needed due to variant change.

## Build, Test, and Lint Commands

See [`justfile`](justfile) for all recipes (`just` with no args lists them).

**Essential commands:**

```bash
just check      # Primary quality check - RUN BEFORE COMPLETING WORK
just ci         # CI-safe read-only checks (no modifications)
just fix        # Auto-fix formatting and lint issues
just serve      # Serve with web UI [requires bun]
just run        # Run CLI with arguments
just test       # All fast tests (Rust + TypeScript unit + typecheck)
just test-unit  # Unit tests only (fast)
just test-e2e   # Playwright E2E (146 tests, chromium + firefox)
just test-nix   # nix flake check (27 checks — runs everything)
```

**Testing architecture:** See [`doc/testing-architecture`](../../doc/2026-03-29T00-00-00Z_reference_testing_architecture/2026-03-29T00-00-00Z_reference_testing_architecture.md) for the complete 6-layer testing reference, browser coverage matrix, environment comparison, and "when to add tests where" decision tree.

Ask user before updating dependencies.

## CLI Commands

Run `id --help` for command list, `id <command> --help` for options. See `src/cli.rs` for full definitions.

## Project Structure

```
src/
├── main.rs, lib.rs, cli.rs    # Entry point, exports, Clap definitions
├── protocol.rs, store.rs      # Network protocol, storage layer
├── helpers.rs                 # Parsing/formatting utilities
├── commands/                  # Command implementations (put, get, find, list, serve, etc.)
├── repl/                      # REPL runner and input preprocessing
└── web/                       # Web UI: routes, assets, templates, collab (feature-gated)
tests/cli_integration.rs       # Integration tests
```

## Code Style

**Imports:** Group by std → external crates (alphabetical) → internal (`crate::`, `super::`), separated by blank lines.

**Naming:** Functions `snake_case` (commands prefixed `cmd_`), types `PascalCase`, constants `SCREAMING_SNAKE_CASE`, tests `test_` prefix.

**Error handling:** Use `anyhow::Result<T>`, `bail!()` for early returns, `?` with `.context()` for propagation.

**Lint rules (see `Cargo.toml`):** Denied: `unwrap_used`, `expect_used`, `panic`, `unimplemented`, `todo`, `dbg_macro`. Test modules: add `#[allow(clippy::unwrap_used, clippy::expect_used)]`.

**Tests:** Place in `#[cfg(test)] mod tests` at file bottom.

## Adding Features

1. Add docstrings (`///` for items, `//!` for modules)
2. Add unit tests in `#[cfg(test)] mod tests`
3. Add integration tests in `tests/cli_integration.rs` for CLI behavior
4. Run `just check` before completing

When tests fail: ensure failure relates to your change, make tests _correct_ not just passing, update tests if behavior changed intentionally.

## Documenting Design & Architecture Decisions

For significant changes, **document first, then implement**. See [`docs/DOCUMENTATION_PROTOCOL.md`](docs/DOCUMENTATION_PROTOCOL.md) for the full protocol.

**When to create docs** (load the protocol if any apply):

- New features affecting system behavior or adding new commands
- Architectural changes or major refactors
- Design decisions with non-obvious trade-offs
- Interface or API changes to existing components

**Quick reference:** Create `docs/<UTC_RFC_DATETIME>_<kind>_<name>/` folder with matching `.md` file before implementing.

## Key Patterns

**Command flow:** parse args → check local/remote → open store/connect → execute → cleanup

**Remote operations:** Check if first argument is a 64-char hex node ID to determine local vs remote mode.

**Type definitions:** Define options structs for commands with multiple parameters.
