//! Command-line interface argument parsing for the `id` CLI tool.
//!
//! This module defines the CLI structure using [clap](https://docs.rs/clap),
//! providing a declarative interface for parsing command-line arguments
//! into structured data.
//!
//! # CLI Structure
//!
//! ```text
//! id [COMMAND]
//!
//! Commands:
//!   serve      Start server (accepts put/get from peers)
//!   repl       Interactive REPL (alias: shell)
//!   put        Store files (aliases: in, add, store, import)
//!   put-hash   Store content by hash only
//!   get        Retrieve files by name or hash
//!   get-hash   Retrieve by hash (shortcut)
//!   cat        Output files to stdout (aliases: output, out)
//!   find       Find files and output content
//!   search     Search files and list matches
//!   show       Find and output file content (alias: view)
//!   peek       Preview files with head/tail display
//!   list       List all stored files
//!   id         Print node ID
//! ```
//!
//! # Search Filtering Flags
//!
//! The `find`, `search`, `show`, and `peek` commands support filtering:
//!
//! - `--first N`: Return only the first N matches (default 1 if no number)
//! - `--last N`: Return only the last N matches (default 1 if no number)
//! - `--count`: Print count of matches instead of the matches
//! - `--exclude PATTERN`: Exclude matches containing pattern (repeatable)
//!
//! # Usage Examples
//!
//! ```bash
//! # Start a persistent server
//! id serve
//!
//! # Store a file with a custom name
//! id put myfile.txt:config.json
//!
//! # Get from a remote node
//! id get abc123...def456 config.json
//!
//! # Interactive REPL connected to remote
//! id repl abc123...def456
//!
//! # Show content of first match for "config"
//! id show config
//!
//! # Preview file with head/tail
//! id peek readme
//!
//! # Search with filters
//! id search --first 5 --exclude .bak config
//! ```
//!
//! # Remote Operations
//!
//! Many commands support remote operations by specifying a 64-character
//! hex node ID as the first positional argument:
//!
//! ```bash
//! # Local put
//! id put file.txt
//!
//! # Remote put (NODE_ID is 64 hex chars)
//! id put abc123...def456 file.txt
//! ```
//!
//! # Input/Output Flexibility
//!
//! Commands support various input and output modes:
//!
//! - **Stdin input**: `--content` for direct content, `--stdin` for paths
//! - **Stdout output**: `-` as output path, `--stdout` flag, or `cat` command
//! - **Renaming**: Use `source:dest` syntax for any path argument

use clap::{Parser, Subcommand};

/// The main CLI structure for the `id` peer-to-peer file sharing tool.
///
/// When invoked without a subcommand, the CLI defaults to REPL mode.
///
/// # Example
///
/// ```rust
/// use id::cli::Cli;
/// use clap::Parser;
///
/// // Parse command line arguments
/// let cli = Cli::parse_from(["id", "serve", "--ephemeral"]);
/// ```
#[derive(Parser, Debug)]
#[command(
    name = "id",
    version,
    about = "An iroh-based peer-to-peer file sharing CLI",
    long_about = None
)]
pub struct Cli {
    /// The subcommand to execute.
    ///
    /// If `None`, the REPL is started.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Available CLI commands.
///
/// Each variant represents a distinct operation mode for the `id` tool.
/// Commands are organized by their primary function: storage, retrieval,
/// search, or system operations.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Start a server that accepts put/get requests from peers.
    ///
    /// The server runs indefinitely, hosting stored blobs and accepting
    /// new content from remote nodes.
    ///
    /// # Examples
    ///
    /// ```bash
    /// # Persistent storage (default)
    /// id serve
    ///
    /// # In-memory storage (lost on exit)
    /// id serve --ephemeral
    ///
    /// # Direct connections only (no relay)
    /// id serve --no-relay
    ///
    /// # Start with web interface on port 3000
    /// id serve --web 3000
    /// ```
    Serve {
        /// Use in-memory storage instead of persistent disk storage.
        ///
        /// Content is lost when the server stops. Useful for testing
        /// or temporary file sharing sessions.
        #[arg(long)]
        ephemeral: bool,
        /// Disable relay servers and use direct connections only.
        ///
        /// May prevent connections through NATs or firewalls.
        #[arg(long)]
        no_relay: bool,
        /// Start web interface on the specified port.
        ///
        /// Enables an HTTP server with a browser-based UI for
        /// file browsing and collaborative editing.
        /// Requires the `web` feature to be enabled at build time.
        #[arg(long)]
        web: Option<u16>,
    },
    /// Start an interactive REPL for issuing commands.
    ///
    /// The REPL provides a shell-like interface for executing multiple
    /// commands without restarting the tool.
    ///
    /// # Session Modes
    ///
    /// - **Local mode**: `id repl` - commands operate on local store
    /// - **Remote mode**: `id repl NODE_ID` - commands target remote node
    ///
    /// # Examples
    ///
    /// ```bash
    /// # Local REPL
    /// id repl
    ///
    /// # Remote REPL (all commands target this node)
    /// id repl abc123...def456
    /// ```
    #[command(alias = "shell")]
    Repl {
        /// Remote node ID for session-level remote targeting.
        ///
        /// When set, all commands in the REPL session target this
        /// remote node instead of the local store.
        #[arg(required = false)]
        node: Option<String>,
    },
    /// Store one or more files in the local or remote blob store.
    ///
    /// Files can be renamed during storage using the `path:name` syntax.
    ///
    /// # Remote Operations
    ///
    /// If the first argument is a 64-character hex node ID, remaining
    /// files are stored on that remote node.
    ///
    /// # Examples
    ///
    /// ```bash
    /// # Store a single file
    /// id put file.txt
    ///
    /// # Store multiple files
    /// id put file1.txt file2.txt
    ///
    /// # Rename during storage
    /// id put myfile.txt:config.json
    ///
    /// # Store on remote node
    /// id put NODE_ID file.txt
    ///
    /// # Store from stdin
    /// echo "content" | id put --content myname.txt
    /// ```
    #[command(aliases = ["in", "add", "store", "import"])]
    Put {
        /// File paths to store.
        ///
        /// Use `path:name` syntax to rename files during storage.
        /// If the first argument is a 64-char hex node ID, files
        /// are sent to that remote node.
        #[arg(required = false)]
        files: Vec<String>,
        /// Read content from stdin instead of file paths.
        ///
        /// Requires exactly one name argument for the stored content.
        #[arg(long, visible_alias = "data", conflicts_with = "stdin")]
        content: bool,
        /// Read additional file paths from stdin.
        ///
        /// Paths are split on newline, tab, or comma.
        #[arg(long, conflicts_with = "content")]
        stdin: bool,
        /// Store by hash only without creating a named tag.
        ///
        /// The content is stored but no human-readable name is assigned.
        /// Useful when you only need the content hash.
        #[arg(long)]
        hash_only: bool,
        /// Disable relay servers for remote operations.
        #[arg(long)]
        no_relay: bool,
    },
    /// Store content by hash only, without a named tag.
    ///
    /// Similar to `put --hash-only` but only accepts a single source.
    ///
    /// # Examples
    ///
    /// ```bash
    /// # Store file by hash
    /// id put-hash file.txt
    ///
    /// # Store stdin by hash
    /// echo "content" | id put-hash -
    /// ```
    #[command(name = "put-hash")]
    PutHash {
        /// File path to store, or "-" for stdin.
        source: String,
    },
    /// Retrieve one or more files by name or hash.
    ///
    /// Files can be written to different output paths using `source:output`.
    ///
    /// # Source Resolution
    ///
    /// 1. Try as exact tag name
    /// 2. Try as hash (if 64 hex characters)
    /// 3. Use `--hash` to force hash interpretation
    /// 4. Use `--name-only` to skip hash interpretation
    ///
    /// # Examples
    ///
    /// ```bash
    /// # Get by name (writes to same name)
    /// id get config.json
    ///
    /// # Get with custom output
    /// id get config.json:local.json
    ///
    /// # Get to stdout
    /// id get config.json:-
    ///
    /// # Get from remote
    /// id get NODE_ID config.json
    /// ```
    Get {
        /// Names or hashes to retrieve.
        ///
        /// Use `source:output` to specify output path (`-` for stdout).
        /// If first arg is a 64-char hex node ID, files are fetched
        /// from that remote node.
        #[arg(required = false)]
        sources: Vec<String>,
        /// Read additional sources from stdin.
        ///
        /// Sources are split on newline, tab, or comma.
        #[arg(long)]
        stdin: bool,
        /// Treat all sources as hashes.
        ///
        /// Fails if a source doesn't match a known hash.
        #[arg(long, conflicts_with = "name_only")]
        hash: bool,
        /// Treat all sources as names only.
        ///
        /// Skips hash interpretation even for 64-char hex strings.
        #[arg(long, conflicts_with = "hash")]
        name_only: bool,
        /// Output all files to stdout (concatenated).
        ///
        /// Overrides per-item output specifications.
        #[arg(long)]
        stdout: bool,
        /// Disable relay servers for remote operations.
        #[arg(long)]
        no_relay: bool,
    },
    /// Retrieve a file by hash with explicit output path.
    ///
    /// Shortcut for `get --hash HASH:OUTPUT`.
    ///
    /// # Examples
    ///
    /// ```bash
    /// # Get hash to file
    /// id get-hash abc123... output.txt
    ///
    /// # Get hash to stdout
    /// id get-hash abc123... -
    /// ```
    #[command(name = "get-hash")]
    GetHash {
        /// The blob hash (64 hex characters).
        hash: String,
        /// Output path, or "-" for stdout.
        output: String,
    },
    /// Output files to stdout (like `get` but defaults to stdout).
    ///
    /// Convenient for piping content to other commands.
    ///
    /// # Examples
    ///
    /// ```bash
    /// # Output to stdout
    /// id cat config.json
    ///
    /// # Pipe to another command
    /// id cat config.json | jq .
    /// ```
    #[command(aliases = ["output", "out"])]
    Cat {
        /// Names or hashes to output.
        ///
        /// If first arg is a 64-char hex node ID, content is fetched
        /// from that remote node.
        #[arg(required = false)]
        sources: Vec<String>,
        /// Read additional sources from stdin.
        #[arg(long)]
        stdin: bool,
        /// Treat all sources as hashes.
        #[arg(long, conflicts_with = "name_only")]
        hash: bool,
        /// Treat all sources as names only.
        #[arg(long, conflicts_with = "hash")]
        name_only: bool,
        /// Disable relay servers for remote operations.
        #[arg(long)]
        no_relay: bool,
    },
    /// Find a file by pattern and output its content (cat over find).
    ///
    /// Searches for files matching the query and outputs content to stdout.
    /// By default outputs the first (best) match. Use `--all` for all matches.
    ///
    /// Supports all find/search flags: `--first`, `--last`, `--exclude`, etc.
    ///
    /// # Examples
    ///
    /// ```bash
    /// # Show first match for "config"
    /// id show config
    ///
    /// # Show all matches
    /// id show --all config
    ///
    /// # Show first 3 matches
    /// id show --first 3 config
    ///
    /// # Exclude backup files
    /// id show --exclude .bak config
    ///
    /// # Write to file instead of stdout
    /// id show -o output.txt config
    /// ```
    #[command(alias = "view")]
    Show {
        /// Search queries (case-insensitive).
        #[arg(required = true)]
        queries: Vec<String>,
        /// Prefer name matches over hash matches in results.
        #[arg(long)]
        name: bool,
        /// Output all matches instead of just the first.
        #[arg(long)]
        all: bool,
        /// Output file (default: stdout).
        #[arg(short = 'o', long)]
        output: Option<String>,
        /// Return only the first N matches (default 1 if no number given).
        #[arg(long, num_args = 0..=1, default_missing_value = "1")]
        first: Option<usize>,
        /// Return only the last N matches (default 1 if no number given).
        #[arg(long, num_args = 0..=1, default_missing_value = "1")]
        last: Option<usize>,
        /// Exclude matches where name or hash contains this pattern (repeatable).
        #[arg(long, action = clap::ArgAction::Append)]
        exclude: Vec<String>,
        /// Remote node ID to search.
        #[arg(long)]
        node: Option<String>,
        /// Disable relay servers.
        #[arg(long)]
        no_relay: bool,
    },
    /// Preview file content with configurable head/tail lines.
    ///
    /// Shows a preview of matching files with head and tail lines.
    /// By default shows 5 head + 5 tail lines (or full content if ≤10 lines).
    ///
    /// # Display Modes
    ///
    /// - Default: shows header banner + head lines + ... + tail lines
    /// - `--quiet`: no header, just content
    /// - `--lines`: custom number of head/tail lines
    /// - `--head-only` / `--tail-only`: show only head or tail
    /// - `--chars` / `--words`: count by characters or words instead of lines
    ///
    /// # Examples
    ///
    /// ```bash
    /// # Preview readme (default 5 head + 5 tail)
    /// id peek readme
    ///
    /// # Preview with 10 head/tail lines
    /// id peek --lines 10 readme
    ///
    /// # Show only first 20 lines
    /// id peek --head-only --lines 20 readme
    ///
    /// # Preview multiple files
    /// id peek readme config.json package.json
    ///
    /// # Preview first 100 characters
    /// id peek --chars --lines 100 readme
    ///
    /// # Quiet mode (no header)
    /// id peek --quiet readme
    /// ```
    Peek {
        /// Search queries (case-insensitive).
        #[arg(required = true)]
        queries: Vec<String>,
        /// Prefer name matches over hash matches in results.
        #[arg(long)]
        name: bool,
        /// Number of lines to show from head and tail (default: 5).
        #[arg(short = 'n', long, default_value = "5")]
        lines: usize,
        /// Show only head lines (no tail).
        #[arg(long, conflicts_with = "tail_only")]
        head_only: bool,
        /// Show only tail lines (no head).
        #[arg(long, conflicts_with = "head_only")]
        tail_only: bool,
        /// Count by characters instead of lines.
        #[arg(long, conflicts_with = "words")]
        chars: bool,
        /// Count by words instead of lines.
        #[arg(long, conflicts_with = "chars")]
        words: bool,
        /// Quiet mode: no header banner, just content.
        #[arg(short = 'q', long)]
        quiet: bool,
        /// Output file (default: stdout).
        #[arg(short = 'o', long)]
        output: Option<String>,
        /// Peek all matches instead of just the first per query.
        #[arg(long)]
        all: bool,
        /// Return only the first N matches (default 1 if no number given).
        #[arg(long, num_args = 0..=1, default_missing_value = "1")]
        first: Option<usize>,
        /// Return only the last N matches (default 1 if no number given).
        #[arg(long, num_args = 0..=1, default_missing_value = "1")]
        last: Option<usize>,
        /// Exclude matches where name or hash contains this pattern (repeatable).
        #[arg(long, action = clap::ArgAction::Append)]
        exclude: Vec<String>,
        /// Remote node ID to search.
        #[arg(long)]
        node: Option<String>,
        /// Disable relay servers.
        #[arg(long)]
        no_relay: bool,
    },
    /// Find files by name/hash query and optionally output content.
    ///
    /// Searches return the best match (or all matches with `--all`).
    /// Match quality: exact > prefix > contains.
    ///
    /// # Output Modes
    ///
    /// - Default: write best match to file with its name
    /// - `--stdout`: write best match to stdout
    /// - `--all`: write all matches (to stdout or `--dir`)
    ///
    /// # Result Limiting
    ///
    /// - `--first`: Return only the first N matches (default 1 if no number)
    /// - `--last`: Return only the last N matches (default 1 if no number)
    /// - `--count`: Print count of matches instead of the matches themselves
    ///
    /// # Filtering
    ///
    /// - `--exclude`: Exclude matches containing the pattern (repeatable)
    ///
    /// # Examples
    ///
    /// ```bash
    /// # Find and save best match
    /// id find config
    ///
    /// # Find and output to stdout
    /// id find --stdout config
    ///
    /// # Find all matches and save to directory
    /// id find --all --dir ./output config
    ///
    /// # Get first 3 matches
    /// id find --first 3 config
    ///
    /// # Get last match
    /// id find --last config
    ///
    /// # Count matches
    /// id find --count config
    ///
    /// # Exclude backup files
    /// id find --exclude .bak --exclude .tmp config
    /// ```
    Find {
        /// Search queries (case-insensitive).
        ///
        /// Multiple queries find the best match for each.
        #[arg(required = true)]
        queries: Vec<String>,
        /// Prefer name matches over hash matches in results.
        #[arg(long)]
        name: bool,
        /// Output to stdout instead of writing to files.
        #[arg(long)]
        stdout: bool,
        /// Output all matches instead of just the best match.
        #[arg(long, visible_aliases = ["out", "export", "save", "full"])]
        all: bool,
        /// Output directory for `--all` (each file saved by name).
        #[arg(long)]
        dir: Option<String>,
        /// Output format: tag (default), group, or union.
        ///
        /// - `tag`: each match with its query
        /// - `group`: matches grouped by query
        /// - `union`: deduplicated by hash
        #[arg(long, default_value = "tag")]
        format: String,
        /// Return only the first N matches (default 1 if no number given).
        #[arg(long, num_args = 0..=1, default_missing_value = "1")]
        first: Option<usize>,
        /// Return only the last N matches (default 1 if no number given).
        #[arg(long, num_args = 0..=1, default_missing_value = "1")]
        last: Option<usize>,
        /// Print count of matches instead of the matches themselves.
        #[arg(long)]
        count: bool,
        /// Exclude matches where name or hash contains this pattern (repeatable).
        #[arg(long, action = clap::ArgAction::Append)]
        exclude: Vec<String>,
        /// Remote node ID to search.
        #[arg(long)]
        node: Option<String>,
        /// Disable relay servers.
        #[arg(long)]
        no_relay: bool,
    },
    /// Search files and list all matches (without outputting content).
    ///
    /// Like `find` but only lists matches, doesn't retrieve content.
    ///
    /// # Result Limiting
    ///
    /// - `--first`: Return only the first N matches (default 1 if no number)
    /// - `--last`: Return only the last N matches (default 1 if no number)
    /// - `--count`: Print count of matches instead of the matches themselves
    ///
    /// # Filtering
    ///
    /// - `--exclude`: Exclude matches containing the pattern (repeatable)
    ///
    /// # Examples
    ///
    /// ```bash
    /// # Search for matches
    /// id search config
    ///
    /// # Search with grouped output
    /// id search --format group config test
    ///
    /// # Get first 5 matches
    /// id search --first 5 config
    ///
    /// # Count matches
    /// id search --count config
    ///
    /// # Exclude backup files
    /// id search --exclude .bak config
    /// ```
    Search {
        /// Search queries (case-insensitive).
        #[arg(required = true)]
        queries: Vec<String>,
        /// Prefer name matches over hash matches.
        #[arg(long)]
        name: bool,
        /// Include all matches in output.
        #[arg(long, visible_aliases = ["out", "export", "save", "full"])]
        all: bool,
        /// Output directory for `--all`.
        #[arg(long)]
        dir: Option<String>,
        /// Output format: tag, group, or union.
        #[arg(long, default_value = "tag")]
        format: String,
        /// Return only the first N matches (default 1 if no number given).
        #[arg(long, num_args = 0..=1, default_missing_value = "1")]
        first: Option<usize>,
        /// Return only the last N matches (default 1 if no number given).
        #[arg(long, num_args = 0..=1, default_missing_value = "1")]
        last: Option<usize>,
        /// Print count of matches instead of the matches themselves.
        #[arg(long)]
        count: bool,
        /// Exclude matches where name or hash contains this pattern (repeatable).
        #[arg(long, action = clap::ArgAction::Append)]
        exclude: Vec<String>,
        /// Remote node ID to search.
        #[arg(long)]
        node: Option<String>,
        /// Disable relay servers.
        #[arg(long)]
        no_relay: bool,
    },
    /// List all stored files (tags) in local or remote store.
    ///
    /// # Examples
    ///
    /// ```bash
    /// # List local store
    /// id list
    ///
    /// # List remote store
    /// id list NODE_ID
    /// ```
    List {
        /// Remote node ID to list (omit for local).
        #[arg(required = false)]
        node: Option<String>,
        /// Disable relay servers for remote operations.
        #[arg(long)]
        no_relay: bool,
    },
    /// Print the local node's public ID.
    ///
    /// The node ID is derived from the keypair and is needed for
    /// remote nodes to connect.
    ///
    /// # Example
    ///
    /// ```bash
    /// id id
    /// # Output: abc123...def456
    /// ```
    Id,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn test_cli_parse_no_args() {
        let cli = Cli::parse_from(["id"]);
        assert!(cli.command.is_none());
    }

    #[test]
    fn test_cli_parse_serve() {
        let cli = Cli::parse_from(["id", "serve"]);
        match cli.command {
            Some(Command::Serve {
                ephemeral,
                no_relay,
                web,
            }) => {
                assert!(!ephemeral);
                assert!(!no_relay);
                assert!(web.is_none());
            }
            _ => panic!("Expected Serve command"),
        }
    }

    #[test]
    fn test_cli_parse_serve_with_flags() {
        let cli = Cli::parse_from(["id", "serve", "--ephemeral", "--no-relay"]);
        match cli.command {
            Some(Command::Serve {
                ephemeral,
                no_relay,
                web,
            }) => {
                assert!(ephemeral);
                assert!(no_relay);
                assert!(web.is_none());
            }
            _ => panic!("Expected Serve command"),
        }
    }

    #[test]
    fn test_cli_parse_put_single_file() {
        let cli = Cli::parse_from(["id", "put", "file.txt"]);
        match cli.command {
            Some(Command::Put {
                files,
                content,
                stdin,
                hash_only,
                no_relay,
            }) => {
                assert_eq!(files, vec!["file.txt"]);
                assert!(!content);
                assert!(!stdin);
                assert!(!hash_only);
                assert!(!no_relay);
            }
            _ => panic!("Expected Put command"),
        }
    }

    #[test]
    fn test_cli_parse_put_multiple_files() {
        let cli = Cli::parse_from(["id", "put", "file1.txt", "file2.txt", "file3.txt"]);
        match cli.command {
            Some(Command::Put { files, .. }) => {
                assert_eq!(files, vec!["file1.txt", "file2.txt", "file3.txt"]);
            }
            _ => panic!("Expected Put command"),
        }
    }

    #[test]
    fn test_cli_parse_put_with_rename() {
        let cli = Cli::parse_from(["id", "put", "local.txt:remote.txt"]);
        match cli.command {
            Some(Command::Put { files, .. }) => {
                assert_eq!(files, vec!["local.txt:remote.txt"]);
            }
            _ => panic!("Expected Put command"),
        }
    }

    #[test]
    fn test_cli_parse_put_content_flag() {
        let cli = Cli::parse_from(["id", "put", "--content", "name"]);
        match cli.command {
            Some(Command::Put { content, stdin, .. }) => {
                assert!(content);
                assert!(!stdin);
            }
            _ => panic!("Expected Put command"),
        }
    }

    #[test]
    fn test_cli_parse_put_aliases() {
        // Test all aliases work
        for alias in ["put", "in", "add", "store", "import"] {
            let cli = Cli::parse_from(["id", alias, "file.txt"]);
            assert!(matches!(cli.command, Some(Command::Put { .. })));
        }
    }

    #[test]
    fn test_cli_parse_get_single() {
        let cli = Cli::parse_from(["id", "get", "file.txt"]);
        match cli.command {
            Some(Command::Get {
                sources,
                hash,
                name_only,
                stdout,
                ..
            }) => {
                assert_eq!(sources, vec!["file.txt"]);
                assert!(!hash);
                assert!(!name_only);
                assert!(!stdout);
            }
            _ => panic!("Expected Get command"),
        }
    }

    #[test]
    fn test_cli_parse_get_with_output() {
        let cli = Cli::parse_from(["id", "get", "file.txt:output.txt"]);
        match cli.command {
            Some(Command::Get { sources, .. }) => {
                assert_eq!(sources, vec!["file.txt:output.txt"]);
            }
            _ => panic!("Expected Get command"),
        }
    }

    #[test]
    fn test_cli_parse_get_hash_flag() {
        let cli = Cli::parse_from(["id", "get", "--hash", "abc123"]);
        match cli.command {
            Some(Command::Get {
                hash, name_only, ..
            }) => {
                assert!(hash);
                assert!(!name_only);
            }
            _ => panic!("Expected Get command"),
        }
    }

    #[test]
    fn test_cli_parse_get_stdout_flag() {
        let cli = Cli::parse_from(["id", "get", "--stdout", "file.txt"]);
        match cli.command {
            Some(Command::Get { stdout, .. }) => {
                assert!(stdout);
            }
            _ => panic!("Expected Get command"),
        }
    }

    #[test]
    fn test_cli_parse_cat() {
        let cli = Cli::parse_from(["id", "cat", "file.txt"]);
        match cli.command {
            Some(Command::Cat { sources, .. }) => {
                assert_eq!(sources, vec!["file.txt"]);
            }
            _ => panic!("Expected Cat command"),
        }
    }

    #[test]
    fn test_cli_parse_cat_aliases() {
        for alias in ["cat", "output", "out"] {
            let cli = Cli::parse_from(["id", alias, "file.txt"]);
            assert!(matches!(cli.command, Some(Command::Cat { .. })));
        }
    }

    #[test]
    fn test_cli_parse_find() {
        let cli = Cli::parse_from(["id", "find", "query"]);
        match cli.command {
            Some(Command::Find {
                queries,
                name,
                stdout,
                all,
                format,
                ..
            }) => {
                assert_eq!(queries, vec!["query"]);
                assert!(!name);
                assert!(!stdout);
                assert!(!all);
                assert_eq!(format, "tag");
            }
            _ => panic!("Expected Find command"),
        }
    }

    #[test]
    fn test_cli_parse_find_multiple_queries() {
        let cli = Cli::parse_from(["id", "find", "query1", "query2"]);
        match cli.command {
            Some(Command::Find { queries, .. }) => {
                assert_eq!(queries, vec!["query1", "query2"]);
            }
            _ => panic!("Expected Find command"),
        }
    }

    #[test]
    fn test_cli_parse_find_with_format() {
        let cli = Cli::parse_from(["id", "find", "--format", "group", "query"]);
        match cli.command {
            Some(Command::Find { format, .. }) => {
                assert_eq!(format, "group");
            }
            _ => panic!("Expected Find command"),
        }
    }

    #[test]
    fn test_cli_parse_search() {
        let cli = Cli::parse_from(["id", "search", "query"]);
        match cli.command {
            Some(Command::Search { queries, .. }) => {
                assert_eq!(queries, vec!["query"]);
            }
            _ => panic!("Expected Search command"),
        }
    }

    #[test]
    fn test_cli_parse_list() {
        let cli = Cli::parse_from(["id", "list"]);
        match cli.command {
            Some(Command::List { node, no_relay }) => {
                assert!(node.is_none());
                assert!(!no_relay);
            }
            _ => panic!("Expected List command"),
        }
    }

    #[test]
    fn test_cli_parse_list_remote() {
        let node_id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let cli = Cli::parse_from(["id", "list", node_id]);
        match cli.command {
            Some(Command::List { node, .. }) => {
                assert_eq!(node, Some(node_id.to_owned()));
            }
            _ => panic!("Expected List command"),
        }
    }

    #[test]
    fn test_cli_parse_repl() {
        let cli = Cli::parse_from(["id", "repl"]);
        match cli.command {
            Some(Command::Repl { node }) => {
                assert!(node.is_none());
            }
            _ => panic!("Expected Repl command"),
        }
    }

    #[test]
    fn test_cli_parse_repl_alias() {
        let cli = Cli::parse_from(["id", "shell"]);
        assert!(matches!(cli.command, Some(Command::Repl { .. })));
    }

    #[test]
    fn test_cli_parse_id() {
        let cli = Cli::parse_from(["id", "id"]);
        assert!(matches!(cli.command, Some(Command::Id)));
    }

    #[test]
    fn test_cli_parse_get_hash() {
        let cli = Cli::parse_from(["id", "get-hash", "abc123", "output.txt"]);
        match cli.command {
            Some(Command::GetHash { hash, output }) => {
                assert_eq!(hash, "abc123");
                assert_eq!(output, "output.txt");
            }
            _ => panic!("Expected GetHash command"),
        }
    }

    #[test]
    fn test_cli_parse_put_hash() {
        let cli = Cli::parse_from(["id", "put-hash", "file.txt"]);
        match cli.command {
            Some(Command::PutHash { source }) => {
                assert_eq!(source, "file.txt");
            }
            _ => panic!("Expected PutHash command"),
        }
    }

    #[test]
    fn test_cli_verify() {
        // Verify CLI structure is valid
        Cli::command().debug_assert();
    }

    // Tests for Show command
    #[test]
    fn test_cli_parse_show() {
        let cli = Cli::parse_from(["id", "show", "query"]);
        match cli.command {
            Some(Command::Show {
                queries,
                name,
                all,
                output,
                first,
                last,
                exclude,
                ..
            }) => {
                assert_eq!(queries, vec!["query"]);
                assert!(!name);
                assert!(!all);
                assert!(output.is_none());
                assert!(first.is_none());
                assert!(last.is_none());
                assert!(exclude.is_empty());
            }
            _ => panic!("Expected Show command"),
        }
    }

    #[test]
    fn test_cli_parse_show_alias_view() {
        let cli = Cli::parse_from(["id", "view", "query"]);
        assert!(matches!(cli.command, Some(Command::Show { .. })));
    }

    #[test]
    fn test_cli_parse_show_with_all() {
        let cli = Cli::parse_from(["id", "show", "--all", "query"]);
        match cli.command {
            Some(Command::Show { all, .. }) => {
                assert!(all);
            }
            _ => panic!("Expected Show command"),
        }
    }

    #[test]
    fn test_cli_parse_show_with_output() {
        let cli = Cli::parse_from(["id", "show", "-o", "output.txt", "query"]);
        match cli.command {
            Some(Command::Show { output, .. }) => {
                assert_eq!(output, Some("output.txt".to_owned()));
            }
            _ => panic!("Expected Show command"),
        }
    }

    #[test]
    fn test_cli_parse_show_with_filters() {
        let cli = Cli::parse_from([
            "id",
            "show",
            "--first",
            "3",
            "--exclude",
            ".bak",
            "--exclude",
            ".tmp",
            "query",
        ]);
        match cli.command {
            Some(Command::Show { first, exclude, .. }) => {
                assert_eq!(first, Some(3));
                assert_eq!(exclude, vec![".bak", ".tmp"]);
            }
            _ => panic!("Expected Show command"),
        }
    }

    // Tests for Peek command
    #[test]
    fn test_cli_parse_peek() {
        let cli = Cli::parse_from(["id", "peek", "query"]);
        match cli.command {
            Some(Command::Peek {
                queries,
                lines,
                head_only,
                tail_only,
                chars,
                words,
                quiet,
                ..
            }) => {
                assert_eq!(queries, vec!["query"]);
                assert_eq!(lines, 5); // default
                assert!(!head_only);
                assert!(!tail_only);
                assert!(!chars);
                assert!(!words);
                assert!(!quiet);
            }
            _ => panic!("Expected Peek command"),
        }
    }

    #[test]
    fn test_cli_parse_peek_with_lines() {
        let cli = Cli::parse_from(["id", "peek", "-n", "10", "query"]);
        match cli.command {
            Some(Command::Peek { lines, .. }) => {
                assert_eq!(lines, 10);
            }
            _ => panic!("Expected Peek command"),
        }
    }

    #[test]
    fn test_cli_parse_peek_head_only() {
        let cli = Cli::parse_from(["id", "peek", "--head-only", "query"]);
        match cli.command {
            Some(Command::Peek {
                head_only,
                tail_only,
                ..
            }) => {
                assert!(head_only);
                assert!(!tail_only);
            }
            _ => panic!("Expected Peek command"),
        }
    }

    #[test]
    fn test_cli_parse_peek_tail_only() {
        let cli = Cli::parse_from(["id", "peek", "--tail-only", "query"]);
        match cli.command {
            Some(Command::Peek {
                head_only,
                tail_only,
                ..
            }) => {
                assert!(!head_only);
                assert!(tail_only);
            }
            _ => panic!("Expected Peek command"),
        }
    }

    #[test]
    fn test_cli_parse_peek_chars() {
        let cli = Cli::parse_from(["id", "peek", "--chars", "-n", "100", "query"]);
        match cli.command {
            Some(Command::Peek {
                chars,
                words,
                lines,
                ..
            }) => {
                assert!(chars);
                assert!(!words);
                assert_eq!(lines, 100);
            }
            _ => panic!("Expected Peek command"),
        }
    }

    #[test]
    fn test_cli_parse_peek_words() {
        let cli = Cli::parse_from(["id", "peek", "--words", "-n", "50", "query"]);
        match cli.command {
            Some(Command::Peek {
                chars,
                words,
                lines,
                ..
            }) => {
                assert!(!chars);
                assert!(words);
                assert_eq!(lines, 50);
            }
            _ => panic!("Expected Peek command"),
        }
    }

    #[test]
    fn test_cli_parse_peek_quiet() {
        let cli = Cli::parse_from(["id", "peek", "-q", "query"]);
        match cli.command {
            Some(Command::Peek { quiet, .. }) => {
                assert!(quiet);
            }
            _ => panic!("Expected Peek command"),
        }
    }

    #[test]
    fn test_cli_parse_peek_with_output() {
        let cli = Cli::parse_from(["id", "peek", "-o", "output.txt", "query"]);
        match cli.command {
            Some(Command::Peek { output, .. }) => {
                assert_eq!(output, Some("output.txt".to_owned()));
            }
            _ => panic!("Expected Peek command"),
        }
    }

    #[test]
    fn test_cli_parse_peek_all() {
        let cli = Cli::parse_from(["id", "peek", "--all", "query"]);
        match cli.command {
            Some(Command::Peek { all, .. }) => {
                assert!(all);
            }
            _ => panic!("Expected Peek command"),
        }
    }

    #[test]
    fn test_cli_parse_peek_with_filters() {
        let cli = Cli::parse_from(["id", "peek", "--first", "2", "--exclude", ".bak", "query"]);
        match cli.command {
            Some(Command::Peek { first, exclude, .. }) => {
                assert_eq!(first, Some(2));
                assert_eq!(exclude, vec![".bak"]);
            }
            _ => panic!("Expected Peek command"),
        }
    }

    // Tests for find/search with new flags
    #[test]
    fn test_cli_parse_find_with_first() {
        let cli = Cli::parse_from(["id", "find", "--first", "3", "query"]);
        match cli.command {
            Some(Command::Find { first, .. }) => {
                assert_eq!(first, Some(3));
            }
            _ => panic!("Expected Find command"),
        }
    }

    #[test]
    fn test_cli_parse_find_with_first_default() {
        // When --first is at the end, it uses the default value
        let cli = Cli::parse_from(["id", "find", "query", "--first"]);
        match cli.command {
            Some(Command::Find { first, .. }) => {
                assert_eq!(first, Some(1)); // default missing value
            }
            _ => panic!("Expected Find command"),
        }
    }

    #[test]
    fn test_cli_parse_find_with_last() {
        let cli = Cli::parse_from(["id", "find", "--last", "5", "query"]);
        match cli.command {
            Some(Command::Find { last, .. }) => {
                assert_eq!(last, Some(5));
            }
            _ => panic!("Expected Find command"),
        }
    }

    #[test]
    fn test_cli_parse_find_with_last_default() {
        // When --last is at the end, it uses the default value
        let cli = Cli::parse_from(["id", "find", "query", "--last"]);
        match cli.command {
            Some(Command::Find { last, .. }) => {
                assert_eq!(last, Some(1)); // default missing value
            }
            _ => panic!("Expected Find command"),
        }
    }

    #[test]
    fn test_cli_parse_find_with_count() {
        let cli = Cli::parse_from(["id", "find", "--count", "query"]);
        match cli.command {
            Some(Command::Find { count, .. }) => {
                assert!(count);
            }
            _ => panic!("Expected Find command"),
        }
    }

    #[test]
    fn test_cli_parse_find_with_exclude() {
        let cli = Cli::parse_from([
            "id",
            "find",
            "--exclude",
            ".bak",
            "--exclude",
            ".tmp",
            "query",
        ]);
        match cli.command {
            Some(Command::Find { exclude, .. }) => {
                assert_eq!(exclude, vec![".bak", ".tmp"]);
            }
            _ => panic!("Expected Find command"),
        }
    }

    #[test]
    fn test_cli_parse_search_with_first() {
        let cli = Cli::parse_from(["id", "search", "--first", "3", "query"]);
        match cli.command {
            Some(Command::Search { first, .. }) => {
                assert_eq!(first, Some(3));
            }
            _ => panic!("Expected Search command"),
        }
    }

    #[test]
    fn test_cli_parse_search_with_last() {
        let cli = Cli::parse_from(["id", "search", "--last", "5", "query"]);
        match cli.command {
            Some(Command::Search { last, .. }) => {
                assert_eq!(last, Some(5));
            }
            _ => panic!("Expected Search command"),
        }
    }

    #[test]
    fn test_cli_parse_search_with_count() {
        let cli = Cli::parse_from(["id", "search", "--count", "query"]);
        match cli.command {
            Some(Command::Search { count, .. }) => {
                assert!(count);
            }
            _ => panic!("Expected Search command"),
        }
    }

    #[test]
    fn test_cli_parse_search_with_exclude() {
        let cli = Cli::parse_from([
            "id",
            "search",
            "--exclude",
            ".bak",
            "--exclude",
            ".tmp",
            "query",
        ]);
        match cli.command {
            Some(Command::Search { exclude, .. }) => {
                assert_eq!(exclude, vec![".bak", ".tmp"]);
            }
            _ => panic!("Expected Search command"),
        }
    }

    #[test]
    fn test_cli_parse_find_combined_options() {
        let cli = Cli::parse_from([
            "id",
            "find",
            "--first",
            "10",
            "--exclude",
            ".bak",
            "--count",
            "query",
        ]);
        match cli.command {
            Some(Command::Find {
                first,
                count,
                exclude,
                ..
            }) => {
                assert_eq!(first, Some(10));
                assert!(count);
                assert_eq!(exclude, vec![".bak"]);
            }
            _ => panic!("Expected Find command"),
        }
    }
}
