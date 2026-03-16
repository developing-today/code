//! Interactive REPL (Read-Eval-Print Loop) module.
//!
//! This module provides the interactive command-line interface for the `id` tool.
//! It allows users to perform blob operations interactively with features like:
//!
//! - **Command history**: Previous commands are saved and can be recalled
//! - **Shell integration**: Execute shell commands with `!cmd` escape
//! - **Input preprocessing**: Support for `$()`, backticks, `|>`, `<<<`, and heredocs
//! - **Remote targeting**: Target specific nodes with `@NODE_ID` syntax
//!
//! # Module Structure
//!
//! - [`input`]: Input preprocessing (shell substitution, heredocs, pipes)
//! - [`runner`]: Main REPL loop and command dispatch
//!
//! # Usage
//!
//! The REPL is started with the `id repl` command:
//!
//! ```bash
//! # Start in auto-detect mode (local-serve if available, else local)
//! id repl
//!
//! # Connect to a specific remote node
//! id repl <NODE_ID>
//! ```
//!
//! # Available Commands
//!
//! | Command | Description |
//! |---------|-------------|
//! | `list` / `ls` | List all stored files |
//! | `put <FILE> [NAME]` | Store a file |
//! | `get <NAME> [OUTPUT]` | Retrieve a file |
//! | `cat <NAME>` | Print file to stdout |
//! | `gethash <HASH> <OUTPUT>` | Retrieve by hash |
//! | `delete` / `rm <NAME>` | Delete a file |
//! | `rename <FROM> <TO>` | Rename a file |
//! | `copy` / `cp <FROM> <TO>` | Copy a file |
//! | `find <QUERY>` | Find and output matching files |
//! | `search <QUERY>` | List matches |
//! | `!<cmd>` | Run shell command |
//! | `help` / `?` | Show help |
//! | `quit` / `exit` / `q` | Exit REPL |
//!
//! # Input Methods
//!
//! The REPL supports several input methods for the `put` command:
//!
//! ```text
//! put $(cmd) name        # Store output of command
//! put `cmd` name         # Store output of command (alt)
//! cmd |> put - name      # Pipe command output to put
//! put - name <<< 'text'  # Store literal text (here-string)
//! put - name <<EOF       # Start heredoc (end with EOF)
//! ```

pub mod input;
pub mod runner;

pub use input::{ReplInput, continue_heredoc, preprocess_repl_line, shell_capture};
pub use runner::run_repl;
