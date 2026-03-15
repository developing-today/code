//! CLI argument parsing

use clap::{Parser, Subcommand};

/// iroh-based peer-to-peer file sharing
#[derive(Parser)]
#[command(name = "id", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Start server (accepts put/get from peers)
    Serve {
        /// Use in-memory storage (default: persistent .iroh-store)
        #[arg(long)]
        ephemeral: bool,
        /// Disable relay servers (direct connections only)
        #[arg(long)]
        no_relay: bool,
    },
    /// Interactive REPL - use 'id repl <NODE_ID>' for remote session, or @NODE_ID prefix in commands
    #[command(alias = "shell")]
    Repl {
        /// Remote node ID for session-level remote targeting (all commands target this node)
        #[arg(required = false)]
        node: Option<String>,
    },
    /// Store one or more files (supports path:name for renaming)
    /// Use "put <NODE_ID> file1 file2 ..." to put to a remote node
    #[command(aliases = ["in", "add", "store", "import"])]
    Put {
        /// File paths to store (use path:name to rename, e.g. file.txt:stored.txt)
        /// If first arg is a 64-char hex NODE_ID, remaining args are sent to that remote node
        #[arg(required = false)]
        files: Vec<String>,
        /// Read content from stdin instead of file paths (requires one name argument)
        #[arg(long, visible_alias = "data", conflicts_with = "stdin")]
        content: bool,
        /// Read additional file paths from stdin (split on newline/tab/comma)
        #[arg(long, conflicts_with = "content")]
        stdin: bool,
        /// Store by hash only, don't create named tags
        #[arg(long)]
        hash_only: bool,
        /// Disable relay servers (for remote operations)
        #[arg(long)]
        no_relay: bool,
    },
    /// Store content by hash only (no name)
    #[command(name = "put-hash")]
    PutHash {
        /// File path or "-" for stdin
        source: String,
    },
    /// Retrieve one or more files by name or hash (supports source:output for renaming)
    /// Use "get <NODE_ID> name1 name2 ..." to get from a remote node
    Get {
        /// Names or hashes to retrieve (use source:output to rename, e.g. file.txt:out.txt or hash:- for stdout)
        /// If first arg is a 64-char hex NODE_ID, remaining args are fetched from that remote node
        #[arg(required = false)]
        sources: Vec<String>,
        /// Read additional sources from stdin (split on newline/tab/comma)
        #[arg(long)]
        stdin: bool,
        /// Treat all sources as hashes (fail if not found, don't check names)
        #[arg(long, conflicts_with = "name_only")]
        hash: bool,
        /// Treat all sources as names only (don't try as hash even if 64 hex chars)
        #[arg(long, conflicts_with = "hash")]
        name_only: bool,
        /// Output all files to stdout (concatenated) - overrides per-item outputs
        #[arg(long)]
        stdout: bool,
        /// Disable relay servers (for remote operations)
        #[arg(long)]
        no_relay: bool,
    },
    /// Retrieve a file by hash (alias for get --hash)
    #[command(name = "get-hash")]
    GetHash {
        /// The blob hash
        hash: String,
        /// Output path (use "-" for stdout)
        output: String,
    },
    /// Output files to stdout (like get but defaults to stdout)
    #[command(aliases = ["output", "out"])]
    Cat {
        /// Names or hashes to retrieve
        /// If first arg is a 64-char hex NODE_ID, remaining args are fetched from that remote node
        #[arg(required = false)]
        sources: Vec<String>,
        /// Read additional sources from stdin (split on newline/tab/comma)
        #[arg(long)]
        stdin: bool,
        /// Treat all sources as hashes
        #[arg(long, conflicts_with = "name_only")]
        hash: bool,
        /// Treat all sources as names only
        #[arg(long, conflicts_with = "hash")]
        name_only: bool,
        /// Disable relay servers (for remote operations)
        #[arg(long)]
        no_relay: bool,
    },
    /// Find files by name/hash query and output to file (use --stdout for stdout)
    Find {
        /// Search queries (matches name or hash: exact > prefix > contains)
        #[arg(required = true)]
        queries: Vec<String>,
        /// Prefer name matches over hash matches
        #[arg(long)]
        name: bool,
        /// Output to stdout instead of file
        #[arg(long)]
        stdout: bool,
        /// Output all matches (to stdout, or to directory with --dir)
        #[arg(long, visible_aliases = ["out", "export", "save", "full"])]
        all: bool,
        /// Output directory for --all (each file saved by name)
        #[arg(long)]
        dir: Option<String>,
        /// Output format: tag (default), group, or union
        #[arg(long, default_value = "tag")]
        format: String,
        /// Remote node ID to search
        #[arg(long)]
        node: Option<String>,
        /// Disable relay servers
        #[arg(long)]
        no_relay: bool,
    },
    /// Search files by name/hash query and list all matches
    Search {
        /// Search queries (matches name or hash: exact > prefix > contains)
        #[arg(required = true)]
        queries: Vec<String>,
        /// Prefer name matches over hash matches
        #[arg(long)]
        name: bool,
        /// Output all matches (to stdout, or to directory with --dir)
        #[arg(long, visible_aliases = ["out", "export", "save", "full"])]
        all: bool,
        /// Output directory for --all (each file saved by name)
        #[arg(long)]
        dir: Option<String>,
        /// Output format: tag (default), group, or union
        #[arg(long, default_value = "tag")]
        format: String,
        /// Remote node ID to search
        #[arg(long)]
        node: Option<String>,
        /// Disable relay servers
        #[arg(long)]
        no_relay: bool,
    },
    /// List all stored files (local or remote)
    List {
        /// Remote node ID to list (optional - lists local if not provided)
        #[arg(required = false)]
        node: Option<String>,
        /// Disable relay servers (for remote operations)
        #[arg(long)]
        no_relay: bool,
    },
    /// Print node ID
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
