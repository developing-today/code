# Agent Instructions for `id` Codebase

Guidelines for AI coding agents working on the `id` peer-to-peer file sharing CLI built with Rust and Iroh.

## Critical: Toolchain Files

**NEVER delete `rust-toolchain.toml`** - it is required for Nix builds. The flake.nix uses rust-overlay which reads this file. Deleting it breaks `nix develop` and `nix build`.

## Critical: Nix and Justfile Synchronization

**When adding or modifying justfile commands, ALWAYS add a corresponding `nix run .#<command>` app in `flake.nix`.**

The flake.nix provides Nix-native equivalents for all just commands. This enables:
- Running commands without entering a dev shell: `nix run .#check-all`
- CI/CD pipelines that use pure Nix evaluation
- Reproducible command execution across systems

### Adding a New Just Command

1. Add the recipe to `justfile`
2. Add a corresponding app to `flake.nix` in the `apps` section:
   ```nix
   my-command = mkApp (mkScript "my-command" "just my-command");
   ```
3. If the command should be verifiable in CI, also add a check in the `checks` section:
   ```nix
   my-command = mkCheck "my-command" "my-command";
   ```

### Shared Packages (nix-common.nix)

Package definitions are shared between `shell.nix` and `flake.nix` via `nix-common.nix`. When adding new development dependencies:

1. Add the package to `nix-common.nix` (in `buildInputs` or `nativeBuildInputs`)
2. Both shell.nix and flake.nix will automatically include it
3. **Never** add packages directly to shell.nix or flake.nix—use nix-common.nix

The `shell.nix` reads `flake.lock` to use the exact same nixpkgs and rust-overlay versions as the flake, ensuring reproducibility without requiring flakes support.

### Nix File Architecture

```
flake.lock              # Pins exact versions (nixpkgs, rust-overlay hashes)
    │
    ├── flake.nix       # Reads inputs, defines rustToolchain, imports nix-common.nix
    │
    └── shell.nix       # Reads flake.lock for same versions, defines rustToolchain,
                        # imports nix-common.nix
            │
            └── nix-common.nix  # Shared: buildInputs, nativeBuildInputs,
                                # opensslEnv, shellHook
```

**Key alignment points:**

1. **Version pinning**: `shell.nix` parses `flake.lock` to get exact `narHash` values for nixpkgs and rust-overlay, ensuring identical versions to `flake.nix`

2. **Rust toolchain**: Defined separately in both `flake.nix` and `shell.nix` using:
   ```nix
   rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
   ```
   This requires rust-overlay to be applied to pkgs first, which happens before importing nix-common.nix. The toolchain is then prepended to `nativeBuildInputs`.

3. **Shared packages**: `nix-common.nix` contains `buildInputs`, `nativeBuildInputs` (excluding rustToolchain), `opensslEnv`, and `shellHook`

4. **Shell hook**: Defined once in `nix-common.nix`, used by both shells

5. **OpenSSL env vars**: Defined in `nix-common.nix.opensslEnv`, applied in both shells

## Environment Setup

**When in doubt, use the Nix dev shell** - it provides all tools with correct versions:

```bash
nix develop              # Preferred: Enter flake-based dev shell
nix-shell                # Alternative: Legacy shell.nix
```

The dev shell includes: Rust 1.89.0, clippy, rustfmt, cargo-llvm-cov, cargo-audit, cargo-outdated, cargo-machete, just, and more.

**Note:** Ignore Nix log messages about disk space, symlinks, or "cannot link" errors - these are harmless warnings.

## Build, Test, and Lint Commands

Use `just` for common tasks. Run `just` with no arguments to see all recipes.

```bash
# Primary quality check - RUN THIS BEFORE COMPLETING WORK
just check-all           # Runs: fmt, lint, test, doc

# Individual checks
just fmt-check           # Check formatting (no changes)
just fmt                 # Auto-format code
just lint                # Run clippy with all targets/features
just lint-fix            # Auto-fix clippy issues
just test                # Run all tests
just test-lib            # Run only unit tests (fast)
just test-int            # Run only integration tests
just doc                 # Build documentation

# Run a single test by name
just test-one test_name
cargo test --lib test_cli_parse_show
cargo test --test cli_integration test_peek_basic

# Code coverage
just coverage            # Generate HTML coverage report
just coverage-summary    # Print coverage summary
```

## Project Structure

```
src/
├── main.rs              # CLI entry point, command dispatch
├── lib.rs               # Library exports, constants, utilities
├── cli.rs               # Clap argument parsing definitions
├── protocol.rs          # Network protocol types (MetaRequest/Response)
├── store.rs             # Storage layer (FsStore/MemStore)
├── helpers.rs           # Parsing and formatting utilities
├── commands/            # Command implementations
│   ├── mod.rs, put.rs, get.rs, find.rs, list.rs, serve.rs, id.rs, client.rs, repl.rs
└── repl/
    ├── runner.rs        # REPL command execution
    └── input.rs         # Input preprocessing (heredocs, substitution)
tests/
└── cli_integration.rs   # Integration tests using built binary
```

## Code Style

### Imports

Order imports in groups separated by blank lines:
1. Standard library (`std::`)
2. External crates (alphabetically)
3. Internal crate imports (`crate::`, `super::`)

### Naming

- **Functions**: `snake_case`, prefix commands with `cmd_` (e.g., `cmd_find`)
- **Types/Structs**: `PascalCase` (e.g., `SearchOptions`, `MetaRequest`)
- **Constants**: `SCREAMING_SNAKE_CASE` (e.g., `META_ALPN`, `KEY_FILE`)
- **Tests**: `test_` prefix (e.g., `test_search_options_first`)

### Error Handling

- Use `anyhow::Result<T>` for fallible functions
- Use `bail!()` for early error returns
- Use `?` operator for propagation
- Add `.context()` for helpful error messages

```rust
pub async fn cmd_example(path: &str) -> Result<()> {
    if path.is_empty() { bail!("path cannot be empty"); }
    let content = std::fs::read(path).context("failed to read file")?;
    Ok(())
}
```

### Strict Lint Rules (Cargo.toml)

- **Denied**: `unwrap_used`, `expect_used`, `panic`, `todo`, `dbg_macro`
- **Enabled**: `clippy::all`, `clippy::pedantic`, `clippy::nursery`, `clippy::cargo`
- **Test modules**: Add `#[allow(clippy::unwrap_used, clippy::expect_used)]`

### Test Organization

Place tests in `#[cfg(test)] mod tests` at the bottom of each file:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    #[test]
    fn test_feature_basic() { /* ... */ }
}
```

## Adding Features

### Requirements

1. **Documentation**: Add docstrings (`///` for items, `//!` for modules)
2. **Unit tests**: In `#[cfg(test)] mod tests` at file bottom
3. **Integration tests**: In `tests/cli_integration.rs` for CLI behavior
4. **Quality**: Run `just check-all` before completing

### Handling Test Failures

- Ensure failure is related to your change
- Make tests *correct*, not just passing
- If behavior changed intentionally, update tests to match

## Dependency Management

```bash
just outdated            # Check for outdated dependencies
just audit               # Security vulnerability audit
just machete             # Find unused dependencies
just update              # Update Cargo.lock
```

Ask the user before updating dependencies, especially major versions.

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

### Command Flow

Commands: parse args → check local/remote → open store/connect → execute → cleanup

### Store Access

```rust
let store = open_store(ephemeral).await?;
let api = store.as_store();
// Use api.blobs() and api.tags()
store.shutdown().await?;
```

### Remote Operations

Check if first argument is a 64-char hex node ID to determine local vs remote mode.

### Type Definitions

Define options structs for commands with multiple parameters:

```rust
#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    pub first: Option<usize>,
    pub last: Option<usize>,
    pub count: bool,
    pub exclude: Vec<String>,
}
```
