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
//!   list       List all stored files
//!   id         Print node ID
//! ```
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
#[derive(Parser)]
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
#[derive(Subcommand)]
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
    /// # Examples
    ///
    /// ```bash
    /// # Search for matches
    /// id search config
    ///
    /// # Search with grouped output
    /// id search --format group config test
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
            }) => {
                assert!(!ephemeral);
                assert!(!no_relay);
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
            }) => {
                assert!(ephemeral);
                assert!(no_relay);
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
                assert_eq!(node, Some(node_id.to_string()));
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
}
