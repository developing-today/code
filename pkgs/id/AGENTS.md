# Agent Instructions for `id` Codebase

Guidelines for AI coding agents working on the `id` peer-to-peer file sharing CLI built with Rust and Iroh.

## Build, Test, and Lint Commands

```bash
cargo build              # Debug build
cargo build --release    # Release build

# Run all tests
cargo test

# Run only library unit tests (fast, no binary needed)
cargo test --lib

# Run only integration tests (requires built binary)
cargo test --test cli_integration

# Run a single test by name
cargo test test_name
cargo test --lib test_cli_parse_show
cargo test --test cli_integration test_peek_basic

# Run tests matching a pattern
cargo test search_options

# Run with output shown
cargo test -- --nocapture

# Linting and formatting
cargo fmt                # Format code
cargo fmt -- --check     # Check formatting
cargo clippy             # Run linter
cargo clippy --fix       # Auto-fix lint issues

# Documentation
cargo doc --open         # Generate and view docs
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
├── commands/
│   ├── mod.rs           # Re-exports all command functions
│   ├── put.rs, get.rs   # Store/retrieve files
│   ├── find.rs          # Search/find/show/peek commands
│   ├── list.rs, serve.rs, id.rs, client.rs, repl.rs
└── repl/
    ├── runner.rs        # REPL command execution
    └── input.rs         # Input preprocessing (heredocs, substitution)
tests/
└── cli_integration.rs   # Integration tests using built binary
```

## Code Style Guidelines

### Import Organization
Order imports in groups separated by blank lines:
1. Standard library (`std::`)
2. External crates (alphabetically)
3. Internal crate imports (`crate::`, `super::`)

### Naming Conventions
- **Functions**: `snake_case`, prefix commands with `cmd_` (e.g., `cmd_find`)
- **Types/Structs**: `PascalCase` (e.g., `SearchOptions`, `MetaRequest`)
- **Constants**: `SCREAMING_SNAKE_CASE` (e.g., `META_ALPN`, `KEY_FILE`)
- **Test functions**: `test_` prefix (e.g., `test_search_options_first`)

### Error Handling
- Use `anyhow::Result<T>` for fallible functions
- Use `bail!()` for early error returns with messages
- Use `?` operator for propagating errors
- Provide context with `.context()` when helpful

```rust
pub async fn cmd_example(path: &str) -> Result<()> {
    if path.is_empty() { bail!("path cannot be empty"); }
    let content = std::fs::read(path).context("failed to read file")?;
    Ok(())
}
```

### Documentation
- Every module: `//!` doc comment explaining purpose
- Structs/functions: `///` doc comments with `# Arguments`, `# Returns`, `# Errors`, `# Example` sections

### Test Organization
Place tests in `#[cfg(test)] mod tests` at the bottom of each file:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_feature_basic() { /* ... */ }
}
```

### Async Patterns
- Use `tokio` runtime with `#[tokio::main]` or `#[tokio::test]`
- Use `futures_lite::StreamExt` for stream operations

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

### CLI Commands (clap)
- Use derive macros for argument parsing
- Provide short and long flag variants for common options
- Add aliases for user convenience

```rust
#[command(alias = "alt-name")]
MyCommand {
    #[arg(short, long)]
    verbose: bool,
}
```

## Key Patterns

### Command Flow
Commands: parse args -> check local/remote -> open store/connect -> execute -> cleanup

### Store Access
```rust
let store = open_store(ephemeral).await?;
let api = store.as_store();
// Use api.blobs() and api.tags()
store.shutdown().await?;
```

### Remote Operations
Check if first argument is a 64-char hex node ID to determine local vs remote mode.

## Testing Checklist
- Add unit tests in same file as code (`#[cfg(test)] mod tests`)
- Add integration tests in `tests/cli_integration.rs` for CLI behavior
- Test both success and error cases
- Use `tempfile::TempDir` for filesystem tests
