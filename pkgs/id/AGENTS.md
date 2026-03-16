# Agent Instructions for `id` Codebase

Guidelines for AI coding agents working on the `id` peer-to-peer file sharing CLI built with Rust and Iroh.

**Updating this file:** Keep prose tight and context-efficient. Prefer links to files over inline examples. Include only essential code samples.

## Critical: Toolchain Files

**NEVER delete `rust-toolchain.toml`** - it is required for Nix builds. The flake.nix uses rust-overlay which reads this file. Deleting it breaks `nix develop` and `nix build`.

## Critical: Nix and Justfile Synchronization

**When adding or modifying justfile commands, ALWAYS add a corresponding `nix run .#<command>` app in `flake.nix`.**

This enables running commands without entering a dev shell (`nix run .#check-all`), CI/CD pipelines with pure Nix evaluation, and reproducible execution across systems.

When adding a new just command:
1. Add the recipe to `justfile`
2. Add corresponding app in `flake.nix` `apps` section
3. For CI-verifiable commands, add a check in `flake.nix` `checks` section

**Package management:** Add new dev dependencies to `nix-common.nix` only (never directly to shell.nix or flake.nix). See `nix-common.nix` for the shared package architecture.

## Environment Setup

Use the Nix dev shell for correct tool versions:

```bash
nix develop   # Preferred: flake-based
nix-shell     # Alternative: legacy
```

Includes: Rust 1.89.0, clippy, rustfmt, cargo-llvm-cov, cargo-audit, cargo-outdated, cargo-machete, just. Ignore Nix warnings about disk space or symlinks.

## Build, Test, and Lint Commands

See [`justfile`](justfile) for all recipes (`just` with no args lists them).

**Essential commands:**
```bash
just check-all    # Primary quality check - RUN BEFORE COMPLETING WORK (fmt, lint, test, doc)
just fmt          # Format code
just lint         # Run clippy
just lint-fix     # Auto-fix clippy issues
just test         # All tests
just test-lib     # Unit tests only (fast)
just test-one X   # Run single test by name
```

**Dependency management:** `just outdated`, `just audit`, `just machete`, `just update`. Ask user before updating dependencies, especially major versions.

## CLI Commands

```
id serve     Start server accepting put/get requests from peers
id repl      Interactive REPL for commands
id put       Store files in local/remote blob store
id get       Retrieve files by name or hash
id cat       Output files to stdout
id find      Find files by name/hash query
id list      List all stored files (tags)
id id        Print local node's public ID
```

Run `id --help` or `id <command> --help` for full options.

## Project Structure

```
src/
├── main.rs, lib.rs, cli.rs    # Entry point, exports, Clap definitions
├── protocol.rs, store.rs      # Network protocol, storage layer
├── helpers.rs                 # Parsing/formatting utilities
├── commands/                  # Command implementations (put, get, find, list, serve, etc.)
└── repl/                      # REPL runner and input preprocessing
tests/cli_integration.rs       # Integration tests
```

## Code Style

**Imports:** Group by std → external crates (alphabetical) → internal (`crate::`, `super::`), separated by blank lines.

**Naming:** Functions `snake_case` (commands prefixed `cmd_`), types `PascalCase`, constants `SCREAMING_SNAKE_CASE`, tests `test_` prefix.

**Error handling:** Use `anyhow::Result<T>`, `bail!()` for early returns, `?` with `.context()` for propagation.

**Lint rules (see `Cargo.toml`):** Denied: `unwrap_used`, `expect_used`, `panic`, `todo`, `dbg_macro`. Test modules: add `#[allow(clippy::unwrap_used, clippy::expect_used)]`.

**Tests:** Place in `#[cfg(test)] mod tests` at file bottom.

## Adding Features

1. Add docstrings (`///` for items, `//!` for modules)
2. Add unit tests in `#[cfg(test)] mod tests`
3. Add integration tests in `tests/cli_integration.rs` for CLI behavior
4. Run `just check-all` before completing

When tests fail: ensure failure relates to your change, make tests *correct* not just passing, update tests if behavior changed intentionally.

## Documenting Design & Architecture Decisions

When making a significant design or architecture decision—whether modifying an existing component or introducing a new feature that changes how things operate—**document first, then implement**.

### When to Document

- New features or components that affect system behavior
- Architectural changes or refactors
- Design decisions with trade-offs worth recording
- Changes to existing components that alter their interface or semantics

### Initial Documentation

1. **Create a docs folder** for the change:
   ```
   docs/<UTC_RFC_DATETIME>_<kind>_<name>/
   ```
   - `<UTC_RFC_DATETIME>`: e.g., `2026-03-16T14-30-00Z`
   - `<kind>`: `feature`, `architecture`, `refactor`, `design`, `component`, etc.
   - `<name>`: descriptive snake_case name

2. **Create the initial document** with the same naming:
   ```
   docs/2026-03-16T14-30-00Z_feature_blob_streaming/2026-03-16T14-30-00Z_feature_blob_streaming.md
   ```

3. **Document the request/intent and initial plan** before implementing:
   - What was requested or identified as needed
   - Initial design approach
   - Key decisions and their rationale

4. **Append updates during rollout** as new sections:
   - Modifications discovered during implementation
   - Clarifications and edge cases
   - Deviations from the original plan

### Post-Rollout Updates

After initial rollout is complete:

- **If significantly different or many updates**: Create a new file in the same folder with a new datetime and clarifying suffix:
  ```
  2026-03-18T09-00-00Z_feature_blob_streaming_final_design.md
  ```

- **Returning in a new session with major changes planned**: Create a new datetime document with a suffix explaining the revision type:
  ```
  2026-03-25T11-00-00Z_feature_blob_streaming_v2_proposal.md
  ```

- **Short updates**: Files can be brief notes, updates to specific parts, or complete re-summarization of current/proposed design

### File Immutability

- **Do not modify files** after initial creation (except appending during active rollout)
- **After some time**, stop appending—create new files instead
- **Preserve historical record**: Files represent the state of understanding at that point in time

### Handling Superseded Features

If a new feature replaces or subsumes an old documented feature:

1. Create a new folder based on the new feature's creation date
2. Reference the old feature's folder in the new documentation
3. Add a note file in the old feature's folder backlinking to the new one
   - This may not be a 1:1 replacement—could indicate a shift in direction
   - Old feature may be deprioritized over time, or not

### Noticed Discrepancies

If the latest summary + subsequent notes are out of date with the actual implementation:

- Create a TODO to provide an update datetime file
- Cover at minimum: noticed differences, understanding of intent/implications, timeline if known

### Example Structure

```
docs/
├── 2026-03-10T08-00-00Z_feature_meta_protocol/
│   ├── 2026-03-10T08-00-00Z_feature_meta_protocol.md          # Initial design
│   ├── 2026-03-12T14-00-00Z_feature_meta_protocol_revised.md  # Post-rollout summary
│   └── 2026-03-20T10-00-00Z_note_superseded_by_v2.md          # Backlink to replacement
├── 2026-03-20T09-00-00Z_feature_meta_protocol_v2/
│   └── 2026-03-20T09-00-00Z_feature_meta_protocol_v2.md       # References old folder
```

## Key Patterns

**Command flow:** parse args → check local/remote → open store/connect → execute → cleanup

**Remote operations:** Check if first argument is a 64-char hex node ID to determine local vs remote mode.

**Type definitions:** Define options structs for commands with multiple parameters (see `SearchOptions` in `helpers.rs`).
