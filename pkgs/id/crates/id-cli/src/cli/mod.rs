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
//!   key        Export or import a local identity key
//!   peers      Discover and list known peers
//!   tag        Manage metadata tags (set, del, list, search) (aliases: label, link)
//!   world        Mint invites for, and join, multiplayer worlds hosted over Iroh
//!   match-audit Export and verify signed match-audit v0 logs
//!   group        Groups: create, policy, add, remove, set roles, list, join
//!   delegate     Grant a delegate a scope in a group
//!   revoke       Revoke a delegation by its grant's event ID
//!   doc          Documents: publish a file; list events, frontier, history
//!   claim        Tag claims on any subject (set, clear, list, search)
//!   writer-sets  Writer sets in the event store (show, expand, discover)
//!   migrate-tags Migrate existing files to name/file auto-tags (alias: migrate)
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

use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Args, CommandFactory, Parser, Subcommand};

pub mod claim;
pub mod doc;
pub mod groups;
#[cfg(feature = "sandbox")]
pub mod match_audit;
pub mod tag;
pub mod world;
pub mod writer_sets;

pub use claim::{ClaimCommand, ClaimScope};
pub use doc::DocCommand;
pub use groups::{DelegateArgs, GroupCommand, GroupRemoveMode, GroupTemplate, RevokeArgs};
#[cfg(feature = "sandbox")]
pub use match_audit::MatchAuditCommand;
pub use tag::TagCommand;
pub use world::{
    WorldApplyArgs, WorldCapsArgs, WorldCommand, WorldCompileArgs, WorldCrossWorldWriteArgs,
    WorldDevArgs, WorldDiffArgs, WorldDoctorArgs, WorldDownloadArgs, WorldFileScopeArgs,
    WorldForceCrossWorldWriteArgs, WorldInstallArgs, WorldInviteArgs, WorldIsolationArgs,
    WorldJoinArgs, WorldListArgs, WorldLogsArgs, WorldMeasureArgs, WorldMirrorArgs,
    WorldProfileArgs, WorldRecordArgs, WorldRecordsArgs, WorldReplayArgs, WorldRuntimeArgs,
    WorldSshTunnelArgs, WorldStatsArgs, WorldVerifyArgs, WorldVia,
};
pub use writer_sets::WriterSetCommand;

/// The main CLI structure for the `id` peer-to-peer file sharing tool.
///
/// When invoked without a subcommand, the CLI defaults to REPL mode.
///
/// # Example
///
/// ```rust
/// use id_cli::cli::Cli;
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
    /// Enable debug logging (equivalent to --log-level debug).
    ///
    /// Takes precedence over --log-level and environment variables.
    #[arg(long, global = true)]
    pub debug: bool,

    /// Set log level (trace, debug, info, warn, error).
    ///
    /// Overrides environment variables but not --debug.
    #[arg(long, global = true, value_name = "LEVEL")]
    pub log_level: Option<String>,

    /// Base directory for all data files (.iroh-store, .iroh-key, .iroh-serve.lock).
    ///
    /// Allows running multiple independent instances on the same machine.
    /// Each instance should use a separate data directory.
    /// The directory is created if it does not exist.
    ///
    /// # Examples
    ///
    /// ```bash
    /// # Run two servers with separate data
    /// id --data-dir ./node1 serve --port 3001
    /// id --data-dir ./node2 serve --port 3002
    ///
    /// # Client commands use the same flag to target a specific instance
    /// id --data-dir ./node1 list
    /// id --data-dir ./node2 repl
    /// ```
    #[arg(
        long,
        short = 'd',
        global = true,
        value_name = "DIR",
        conflicts_with = "new"
    )]
    pub data_dir: Option<PathBuf>,

    /// Create a new instance directory under .iroh/<name>/.
    ///
    /// Shorthand for `--data-dir .iroh/<name>/` that auto-creates
    /// a named (or randomly-named) subdirectory under `.iroh/` in
    /// the current working directory.
    ///
    /// # Examples
    ///
    /// ```bash
    /// # Named instance
    /// id --new my-dev serve --port 0
    ///
    /// # Random name (auto-generated)
    /// id --new serve
    ///
    /// # Client commands targeting the same instance
    /// id --new my-dev list
    /// ```
    #[arg(long, global = true, num_args = 0..=1, default_missing_value = "", value_name = "NAME", conflicts_with = "data_dir")]
    pub new: Option<String>,

    /// The group log that `group`, `delegate` and `revoke` read and append to.
    ///
    /// Defaults to `groups.jsonl` in the data directory. A relative path is relative to the data
    /// directory, so a log shared by several nodes is best given as an absolute path.
    #[arg(long, global = true, value_name = "FILE")]
    pub groups_log: Option<PathBuf>,

    /// The event store that `doc` and `claim` read and append to.
    ///
    /// Defaults to `events` in the data directory. A relative path is relative to the data
    /// directory, so a store shared by several nodes is best given as an absolute path.
    #[arg(long, global = true, value_name = "DIR")]
    pub event_store: Option<PathBuf>,

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
/// Names and aliases of the top-level subcommands, taken from the clap
/// definition itself so the list can never drift from the real parser.
fn top_level_subcommand_names() -> std::collections::HashSet<String> {
    let command = Cli::command();
    let mut names = std::collections::HashSet::new();
    for sub in command.get_subcommands() {
        names.insert(sub.get_name().to_owned());
        names.extend(sub.get_aliases().map(str::to_owned));
        names.extend(sub.get_visible_aliases().map(str::to_owned));
    }
    names
}

/// Rewrite raw argv so the space form of `--new` never swallows a
/// subcommand as the instance name (#3144).
///
/// `--new` takes an optional value (`num_args = 0..=1`), so clap greedily
/// consumes the next token: the documented `id --new serve` parsed as
/// instance name `serve` with no subcommand, created `.iroh/serve/`, and
/// started the REPL. When the token after a bare `--new` is a top-level
/// subcommand name (and no subcommand has been seen yet), rewrite that
/// `--new` to `--new=` — the empty value still means "random name", and
/// the token is then parsed as the subcommand. Every other form is
/// passed through untouched: bare `--new`, `--new=<name>`, `--new <name>
/// <command>`, and a trailing `--new` after the subcommand.
pub fn normalize_args<I, T>(args: I) -> Vec<OsString>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    // Global options that take a separate value token: the token after
    // one of these is a value even when it spells a subcommand name.
    const VALUE_FLAGS: [&str; 5] = [
        "--log-level",
        "--data-dir",
        "-d",
        "--groups-log",
        "--event-store",
    ];
    let subcommands = top_level_subcommand_names();
    let raw: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let mut out: Vec<OsString> = Vec::with_capacity(raw.len());
    let mut seen_subcommand = false;
    let mut expect_value = false;
    let mut iter = raw.into_iter().peekable();
    // The first token is the program name; copy it verbatim.
    if let Some(prog) = iter.next() {
        out.push(prog);
    }
    while let Some(arg) = iter.next() {
        if expect_value {
            out.push(arg);
            expect_value = false;
            continue;
        }
        let text = arg.to_str().map(str::to_owned);
        if !seen_subcommand && text.as_deref() == Some("--new") {
            match iter.peek().and_then(|next| next.to_str()) {
                Some(next) if subcommands.contains(next) => {
                    out.push(OsString::from("--new="));
                }
                _ => {
                    // The next token (if any) is the instance name; do
                    // not mistake it for a subcommand on the next pass.
                    out.push(arg);
                    expect_value = true;
                }
            }
            continue;
        }
        out.push(arg);
        if seen_subcommand {
            continue;
        }
        match text.as_deref() {
            // After `--`, every token is a positional value.
            Some("--") => seen_subcommand = true,
            Some(name) if VALUE_FLAGS.contains(&name) => expect_value = true,
            Some(name) if !name.starts_with('-') && subcommands.contains(name) => {
                seen_subcommand = true;
            }
            _ => {}
        }
    }
    out
}

/// Parses a result limit (`--first` / `--last` / peek `--lines`): zero is
/// rejected with a clear error instead of silently producing an empty result
/// or a header-only preview (#3059). Clap names the flag in its error prefix,
/// so the message stays flag-specific at the boundary.
fn parse_result_limit(raw: &str) -> Result<usize, String> {
    match raw.parse::<usize>() {
        Ok(0) => Err("zero is not a valid limit: use at least 1".to_owned()),
        Ok(n) => Ok(n),
        Err(err) => Err(err.to_string()),
    }
}

/// The arguments of `id serve`, extracted from the `Command`
/// variant into their own `Args` struct (#4751): clap's derive inlines
/// a struct-style variant's argument construction into the parent
/// enum's `augment_subcommands`, and at this variant's size the
/// generated function's stack frame - summed with the Cli/Command
/// frames above it - overflows the default thread stack on every
/// parse in debug builds, aborting the lib test binary. As a tuple
/// variant over an `Args` struct, the construction lives in its own
/// functions (ServeArgs::augment_args / from_arg_matches) and the
/// parent enum's frame stays small.
#[derive(Args, Debug)]
pub struct ServeArgs {
    /// Use in-memory storage instead of persistent disk storage.
    ///
    /// Content is lost when the server stops. Useful for testing
    /// or temporary file sharing sessions.
    #[arg(long)]
    pub ephemeral: bool,
    /// Path to the operator server config TOML (#5119 slice 1).
    ///
    /// When given, the config is loaded at startup: a malformed
    /// file fails startup naming the parse error, and startup
    /// provisioning runs from the loaded config. Without the
    /// flag no file is read and serve behaves exactly as before.
    #[arg(long)]
    pub server_config: Option<PathBuf>,
    /// Disable relay servers and use direct connections only.
    ///
    /// May prevent connections through NATs or firewalls.
    #[arg(long)]
    pub no_relay: bool,
    /// Disable gossip-based peer discovery entirely.
    ///
    /// The server still accepts connections but does not join
    /// any gossip topic, does not announce itself, and does not
    /// discover peers. Useful for isolated servers.
    #[arg(long)]
    pub no_gossip: bool,
    /// Start web interface.
    ///
    /// Enables an HTTP server with a browser-based UI for
    /// file browsing and collaborative editing.
    /// Requires the `web` feature to be enabled at build time.
    #[arg(long)]
    pub web: bool,
    /// Port for the web interface.
    ///
    /// Use port 0 to let the OS assign a random available port.
    #[arg(long, default_value = "3000")]
    pub port: u16,
    /// Comma-separated node IDs for manual peer bootstrapping.
    ///
    /// Supplementary to automatic DHT discovery. Useful when DHT
    /// traffic is blocked or for development/testing.
    #[arg(long, value_delimiter = ',')]
    pub bootstrap: Vec<String>,
    /// Gossip topic name for peer discovery.
    ///
    /// All servers on the same topic can discover each other.
    /// Default: "id-peer-discovery-v1" (public network).
    /// Use a custom topic for private networks.
    #[arg(long)]
    pub topic: Option<String>,
    /// Shared secret for topic access control.
    ///
    /// Used to encrypt DHT bootstrap records. Only nodes with
    /// the same secret can discover each other via DHT.
    /// Default: "id-public-discovery-v1" (public network).
    #[arg(long)]
    pub topic_secret: Option<String>,
    /// Skip default bootstrap node IDs from `defaults.conf`.
    ///
    /// Only CLI-provided `--bootstrap` nodes are used.
    #[arg(long)]
    pub no_default_bootstrap: bool,
    /// Skip default topic and secret from `defaults.conf`.
    ///
    /// Falls back to the hardcoded constant values. CLI-provided
    /// `--topic` and `--topic-secret` still take precedence.
    #[arg(long)]
    pub no_default_topic: bool,
    /// Use only `defaults.conf` values, ignoring hardcoded fallbacks.
    ///
    /// Useful when you want to fully control defaults via the
    /// embedded configuration file at build time.
    #[arg(long)]
    pub replace_defaults: bool,
    /// Disable mDNS local peer discovery.
    ///
    /// By default, the server uses mDNS to discover peers on the
    /// local network, and so announces its node ID and addresses to
    /// every host there. This flag disables that, which is useful in
    /// environments where multicast traffic is blocked or unwanted.
    #[arg(long)]
    pub no_mdns: bool,
    /// Enable mainline DHT address lookup and publishing.
    ///
    /// By default, the server does not use the mainline BitTorrent DHT
    /// for address discovery. This flag enables DHT lookup and publishing
    /// for the server's endpoint.
    #[arg(long)]
    pub dht: bool,
    /// Port for the Iroh QUIC endpoint (peer-to-peer protocol).
    ///
    /// By default, the OS assigns a random available port (0).
    /// Set a specific port to allow firewall rules or when running
    /// multiple servers that need predictable port assignments.
    #[arg(long, default_value = "0")]
    pub iroh_port: u16,
    /// Address the web interface binds to.
    ///
    /// Defaults to loopback, so the UI is reachable only from this
    /// machine. Use `0.0.0.0` to expose it on the network, and set
    /// `--web-token` when you do.
    #[arg(long, default_value = "127.0.0.1")]
    pub bind: std::net::IpAddr,
    /// Web listener to bind, as `IP:PORT` (repeatable; #2145).
    ///
    /// Each occurrence adds one listener; the set replaces any
    /// `[web_bindings]` entries from `defaults.conf` entirely.
    /// Combining `--web-listen` with an explicitly-set `--bind` or
    /// `--port` is a startup error naming the conflict. IPv6
    /// addresses use the bracket form, e.g. `[::1]:3001`.
    #[arg(long = "web-listen", value_name = "IP:PORT")]
    pub web_listen: Vec<SocketAddr>,
    /// Require this token to use the web interface.
    ///
    /// Open `http://host:port/?token=<TOKEN>` once; the browser keeps a
    /// cookie. Scripts may send `Authorization: Bearer <TOKEN>`.
    #[arg(long, env = "ID_WEB_TOKEN")]
    pub web_token: Option<String>,
    /// Mark the web session cookies (`id_token` and the directory explorer's) `Secure`.
    ///
    /// Set this only when HTTPS is terminated in front of the server.
    /// Browsers drop `Secure` cookies over plain HTTP, except on localhost.
    #[arg(long, env = "ID_WEB_COOKIE_SECURE")]
    pub web_cookie_secure: bool,
    /// Host a multiplayer world over Iroh (and `/ws/world` with `--web`).
    #[arg(long, requires = "world_admin_token")]
    pub world: bool,
    /// Admin secret required to mint world guest capabilities.
    #[arg(long, env = "ID_WORLD_ADMIN_TOKEN")]
    pub world_admin_token: Option<String>,
    /// Start with this precompiled Wasm world module (Roc platform ABI).
    #[arg(long, requires = "world")]
    pub world_module: Option<PathBuf>,
    /// Write confirmation mail into this directory, one private file per message.
    #[arg(
        long,
        requires = "world",
        conflicts_with_all = ["world_mail_command", "world_mail_smtp"]
    )]
    pub world_mail_outbox: Option<PathBuf>,
    /// Run this program for each confirmation mail, with `ID_MAIL_TO`, `ID_MAIL_SUBJECT` and `ID_MAIL_BODY` set.
    #[arg(
        long,
        requires = "world",
        conflicts_with_all = ["world_mail_outbox", "world_mail_smtp"]
    )]
    pub world_mail_command: Option<PathBuf>,
    /// Relay confirmation mail through this SMTP server, which must be on loopback and forward with TLS.
    #[arg(
        long,
        requires_all = ["world", "world_mail_from"],
        conflicts_with_all = ["world_mail_outbox", "world_mail_command"]
    )]
    pub world_mail_smtp: Option<SocketAddr>,
    /// Sender address for mail sent with `--world-mail-smtp`.
    #[arg(long, requires = "world_mail_smtp")]
    pub world_mail_from: Option<String>,
    /// Name of the world this server offers (one world per process).
    ///
    /// The world's files live in `.id-worlds/<NAME>/`. Names may contain
    /// `a-z`, `0-9`, `-` and `_`.
    #[arg(long, default_value = "lobby", requires = "world")]
    pub world_name: String,
    /// Trim the world's journal behind a program snapshot every N events.
    ///
    /// A journal otherwise grows (and restarts slow down) without bound.
    /// Programs that export `plaza_snapshot`/`plaza_restore` are trimmed;
    /// others keep their full history. `0` never trims.
    #[arg(long, default_value_t = 1000, requires = "world")]
    pub world_checkpoint_every: u64,
    /// Most worlds open at once; idle durable worlds close to make room.
    #[arg(long, default_value_t = 256, requires = "world")]
    pub world_max_open: usize,
    /// Most world sessions at once, across every world.
    #[arg(long, default_value_t = 1024, requires = "world")]
    pub world_max_sessions: usize,
    /// Close durable worlds that no session used for this many seconds
    /// (they reopen from their journal on demand). `0` keeps them open.
    #[arg(long, default_value_t = 600, requires = "world")]
    pub world_idle_secs: u64,
    /// Refuse signed world and directory requests for this many seconds
    /// after startup, so a request captured before a restart cannot be
    /// replayed into a server with empty replay memory. `0` disables.
    #[arg(long, default_value_t = 600, requires = "world")]
    pub world_boot_window_secs: u64,
    /// Serve the world over SSH on this port, bound to `--bind`.
    ///
    /// Connect with `ssh -p <PORT> <WORLD>@<host>`; the password is the
    /// guest capability from an invite.
    #[arg(long, requires = "world")]
    pub world_ssh_port: Option<u16>,
    /// Accept native (ELF) module installs. Unlike Wasm, the guarantee
    /// is the OS sandbox (Landlock, seccomp, rlimits), not an
    /// import-free format, so this tier is opt-in.
    #[arg(long, requires = "world")]
    pub world_native: bool,
    /// Accept new world programs only as Roc source compiled by this server.
    /// Disables raw Wasm/native module uploads and `--world-module`.
    #[arg(
        long,
        env = "ID_WORLD_SOURCE_ONLY",
        requires = "world",
        conflicts_with = "world_module"
    )]
    pub world_source_only: bool,
    /// Ed25519 module-signing public key trusted by this server (hex; repeatable).
    #[arg(
        long = "world-module-trust-key",
        value_name = "HEX",
        requires = "world"
    )]
    pub world_module_trust_keys: Vec<String>,
    /// Execution bounds for world programs.
    #[command(flatten)]
    pub world_runtime: WorldRuntimeArgs,
    /// Capability the default world's program may use (repeatable; `*`
    /// grants the whole catalog).
    #[arg(long = "world-cap", value_name = "NAME", requires = "world")]
    pub world_caps: Vec<String>,
    /// What an unlisted capability costs the program: `deny` (default) or
    /// `grant-on-use`, which grants a known capability the first time the
    /// program uses it.
    #[arg(long, default_value = "deny", requires = "world")]
    pub world_cap_policy: String,
    /// Capabilities the server allows any world to use, comma-separated
    /// (default: the whole catalog). A world may grant less, never more.
    /// `*` is refused here.
    #[arg(long, value_name = "NAMES", value_delimiter = ',', requires = "world")]
    pub world_allowed_caps: Option<Vec<String>>,
    /// The Roc binary used to compile sources sent to `id world compile`
    /// (default: $`ID_ROC_BIN` or `roc` from PATH).
    #[arg(long, env = "ID_ROC_BIN")]
    pub roc_bin: Option<String>,
    /// The world platform directory used for on-the-fly compilation
    /// (default: $`ID_ROC_PLATFORM` or ./examples/roc-world).
    #[arg(long, env = "ID_ROC_PLATFORM")]
    pub roc_platform: Option<PathBuf>,
    /// Allow this node to modify the store (repeatable, comma-separated).
    ///
    /// Reading names, hashes and tags is public. Writing (put, delete,
    /// rename, copy, tag changes, blob pushes) requires the node's ID to be
    /// listed here or in `.iroh-allowed` (one ID per line). This machine's
    /// own server and client keys are always allowed.
    #[arg(long = "allow-node", value_delimiter = ',')]
    pub allow_node: Vec<String>,
    /// Let every peer modify the store (insecure).
    ///
    /// For demos and trusted networks only.
    #[arg(long)]
    pub open_writes: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Start a server that accepts put/get requests from peers.
    ///
    /// This is the only command that hosts a world or serves stored
    /// content (F13.8 client-only boundary): every other command —
    /// including all `id world ...` subcommands — is a client that
    /// connects out to a host and never hosts or serves a world. SSH
    /// world serving is likewise opt-in here, via the
    /// `--world-ssh-port` family of flags.
    ///
    /// The server runs indefinitely, hosting stored blobs and accepting
    /// new content from remote nodes. Automatically discovers other
    /// servers via gossip-based peer discovery using the `BitTorrent` DHT.
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
    /// # Start with web interface
    /// id serve --web
    ///
    /// # Start with web interface on custom port
    /// id serve --web --port 8080
    ///
    /// # Bootstrap from known peer
    /// id serve --bootstrap abc123...def456
    ///
    /// # Private discovery network
    /// id serve --topic my-private-net --topic-secret my-secret
    ///
    /// # Disable gossip entirely
    /// id serve --no-gossip
    ///
    /// # Skip built-in bootstrap nodes
    /// id serve --no-default-bootstrap
    ///
    /// # Skip built-in topic/secret (use hardcoded constants)
    /// id serve --no-default-topic
    ///
    /// # Use only defaults.conf values (ignore hardcoded fallbacks)
    /// id serve --replace-defaults
    /// ```
    Serve(ServeArgs),
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
        #[arg(
            long,
            conflicts_with = "last",
            num_args = 0..=1,
            default_missing_value = "1",
            value_parser = parse_result_limit
        )]
        first: Option<usize>,
        /// Return only the last N matches (default 1 if no number given).
        #[arg(
            long,
            conflicts_with = "first",
            num_args = 0..=1,
            default_missing_value = "1",
            value_parser = parse_result_limit
        )]
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
        #[arg(short = 'n', long, default_value = "5", value_parser = parse_result_limit)]
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
        #[arg(
            long,
            conflicts_with = "last",
            num_args = 0..=1,
            default_missing_value = "1",
            value_parser = parse_result_limit
        )]
        first: Option<usize>,
        /// Return only the last N matches (default 1 if no number given).
        #[arg(
            long,
            conflicts_with = "first",
            num_args = 0..=1,
            default_missing_value = "1",
            value_parser = parse_result_limit
        )]
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
        #[arg(long, default_value = "tag", value_parser = ["tag", "group", "union"])]
        format: String,
        /// Return only the first N matches (default 1 if no number given).
        #[arg(
            long,
            conflicts_with = "last",
            num_args = 0..=1,
            default_missing_value = "1",
            value_parser = parse_result_limit
        )]
        first: Option<usize>,
        /// Return only the last N matches (default 1 if no number given).
        #[arg(
            long,
            conflicts_with = "first",
            num_args = 0..=1,
            default_missing_value = "1",
            value_parser = parse_result_limit
        )]
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
        #[arg(long, default_value = "tag", value_parser = ["tag", "group", "union"])]
        format: String,
        /// Return only the first N matches (default 1 if no number given).
        #[arg(
            long,
            conflicts_with = "last",
            num_args = 0..=1,
            default_missing_value = "1",
            value_parser = parse_result_limit
        )]
        first: Option<usize>,
        /// Return only the last N matches (default 1 if no number given).
        #[arg(
            long,
            conflicts_with = "first",
            num_args = 0..=1,
            default_missing_value = "1",
            value_parser = parse_result_limit
        )]
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
    /// Manage metadata tags on files.
    ///
    /// Tags are key/value pairs attached to files for organization,
    /// filtering, and search. Backed by iroh-docs CRDT documents
    /// for conflict-free replication.
    ///
    /// # Examples
    ///
    /// ```bash
    /// # Set a tag
    /// id tag set README.md priority high
    ///
    /// # Set a key-only tag (no value)
    /// id tag set README.md pinned
    ///
    /// # Delete a tag
    /// id tag del README.md priority high
    ///
    /// # List all tags
    /// id tag list
    ///
    /// # List tags for a specific file
    /// id tag list README.md
    ///
    /// # Search tags by key
    /// id tag search priority
    ///
    /// # Search tags by key and value
    /// id tag search priority high
    /// ```
    #[command(subcommand, aliases = ["label", "link"])]
    Tag(TagCommand),
    /// Mint invites for, and join, multiplayer worlds hosted over Iroh.
    ///
    /// The host runs `id serve --world --world-admin-token <T>`. Whoever holds
    /// the admin token mints a guest capability with `id world invite`; a guest
    /// presents it with `id world join`. Reaching a node proves only which node
    /// you are, never what you may do: the capability is the authority.
    ///
    /// Every `world` subcommand is a client command (F13.8 client-only
    /// boundary): it connects out to a host, and opening further world
    /// sessions starts no world host or authority on this node. Hosting
    /// begins only through the explicit top-level `id serve` command;
    /// there is no `--client-only` flag because no world subcommand has
    /// a hosting mode to switch off. Two subcommands do open local
    /// listeners without hosting a world: `world ssh-tunnel` without
    /// `--stdio` listens on a local address (loopback by default) to
    /// proxy an SSH client to the host, and `world mirror` runs a
    /// replica endpoint that accepts sync connections for the replica.
    /// Neither grants any world authority: node connectivity is not
    /// world authority, and a valid world capability is still required
    /// for every world operation.
    ///
    /// # Examples
    ///
    /// ```bash
    /// id world invite NODE_ID --admin-token T --name ann
    /// ID_WORLD_CAPABILITY=... id world join NODE_ID
    /// ```
    #[command(subcommand)]
    World(Box<WorldCommand>),
    /// Export and verify signed match-audit v0 logs.
    #[cfg(feature = "sandbox")]
    #[command(subcommand)]
    MatchAudit(MatchAuditCommand),
    /// Groups: create, show the policy, add, remove, set roles, list, and join (P6.7a).
    ///
    /// Each action records one event in the group log and prints one line, or one JSON object with
    /// `--json`. A refused action exits non-zero, and its event stays in the log.
    ///
    /// # Examples
    ///
    /// ```bash
    /// id group create --max 8
    /// id group add GROUP ACCOUNT --role write
    /// id group list --json
    /// ```
    Group {
        /// Print JSON instead of a line (one array for `list`).
        #[arg(long, global = true)]
        json: bool,
        /// The group action to run.
        #[command(subcommand)]
        command: GroupCommand,
    },
    /// Grant a delegate a scope in a group, with the group's delegation mode (P6.7b).
    Delegate(DelegateArgs),
    /// Revoke a delegation by its grant's event ID, as `delegate` printed it (P6.7b).
    Revoke(RevokeArgs),
    /// Documents: publish a file, and list a document's events, frontier and history (P6.7c).
    ///
    /// Each action signs one event into the event store and prints one line, or one JSON object
    /// with `--json`. The store stands in for sync: nodes that share it see each other's events.
    /// Lists print one line per row, or one JSON array with `--json`.
    ///
    /// # Examples
    ///
    /// ```bash
    /// id doc publish notes.md --media-type text/markdown --tag topic=plans
    /// id doc history DOCUMENT
    /// id doc frontier --json
    /// ```
    Doc {
        /// Print JSON instead of lines.
        #[arg(long, global = true)]
        json: bool,
        /// The document action to run.
        #[command(subcommand)]
        command: DocCommand,
    },
    /// Tag claims on any subject: set, clear, list, and search (P6.7c).
    ///
    /// A claim is a signed `tag.set` event, and `clear` removes the live claims it names. These are
    /// the event-backed tags. The file tags of `id tag` are a separate store.
    ///
    /// # Examples
    ///
    /// ```bash
    /// id claim set DOCUMENT topic plans
    /// id claim clear DOCUMENT topic
    /// id claim search topic plans --json
    /// ```
    Claim {
        /// Print JSON instead of lines (one array for `list` and `search`).
        #[arg(long, global = true)]
        json: bool,
        /// The claim action to run.
        #[command(subcommand)]
        command: ClaimCommand,
    },
    /// Writer sets stored in the event store: show one, expand it, or discover
    /// sets by scope (writer-set spec §§4–5; issue #6690).
    ///
    /// A writer set is a signed `idset.writer` event whose body the store
    /// retains at ingest. `show` prints the stored object, `expand` runs the
    /// view projection over the store's resolved group state at the set's own
    /// publishing frontier, and `discover` lists every stored set for a scope
    /// with the writers each expands to. `--resolution` names the resolution
    /// snapshot the projection is pinned under; without one, the zero hash
    /// names the store's current resolved state.
    ///
    /// # Examples
    ///
    /// ```bash
    /// id writer-sets discover --scope metrics
    /// id writer-sets expand EVENT --json
    /// id writer-sets show EVENT
    /// ```
    WriterSets {
        /// Print JSON instead of lines (one array for `discover`).
        #[arg(long, global = true)]
        json: bool,
        /// The writer-set action to run.
        #[command(subcommand)]
        command: WriterSetCommand,
    },
    /// Migrate existing files to have name/file auto-tags.
    ///
    /// Scans all blob tags in the store and adds `name` and `file`
    /// metadata tags for any subject that doesn't already have them.
    /// Existing tags are never overwritten.
    ///
    /// # Example
    ///
    /// ```bash
    /// id migrate-tags
    /// # Output: migrated 42 files
    /// ```
    #[command(alias = "migrate")]
    MigrateTags,
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
    ///
    /// # The identity this machine uses as a *client*. A server owner adds it
    /// # with `id serve --allow-node <ID>` so this machine may write to it.
    /// id id --client
    /// ```
    Id {
        /// Print the client identity (used when connecting to other nodes)
        /// instead of the server identity.
        #[arg(long)]
        client: bool,
    },
    /// Export or import a local identity key using a passphrase-wrapped envelope.
    Key {
        #[command(subcommand)]
        command: KeyCommand,
    },
    /// Discover and list known peers.
    ///
    /// Discovers other `id` servers via gossip-based networking, RPC
    /// queries to a running serve instance, or both (default). Supports
    /// depth-based recursive crawling to find peers-of-peers.
    ///
    /// # Discovery Modes
    ///
    /// - **Default**: Try RPC (query local serve), then gossip as fallback
    /// - **RPC only** (`--rpc`): Only query via `ListPeers` RPC
    /// - **Gossip only** (`--gossip`): Only use gossip topic discovery
    ///
    /// # Examples
    ///
    /// ```bash
    /// # List peers from local serve
    /// id peers
    ///
    /// # List peers from a remote node
    /// id peers abc123...def456
    ///
    /// # Deep crawl up to 3 levels
    /// id peers --depth 3
    ///
    /// # Direct gossip discovery (no serve needed)
    /// id peers --gossip
    ///
    /// # Gossip with custom topic
    /// id peers --gossip --topic my-net --topic-secret my-secret
    ///
    /// # Skip built-in bootstrap nodes
    /// id peers --gossip --no-default-bootstrap
    /// ```
    Peers {
        /// Use gossip-only discovery (join topic, collect announcements).
        ///
        /// Bypasses RPC entirely. Useful when no local serve is running.
        #[arg(long)]
        gossip: bool,
        /// Use RPC-only discovery (query server's `ListPeers`).
        ///
        /// Fails if no local serve is running (unless a node ID is given).
        #[arg(long)]
        rpc: bool,
        /// Recursive depth for peer-of-peer crawling.
        ///
        /// 0 = no crawl, 1 = query direct peers (default),
        /// N = recursively query up to N levels deep.
        #[arg(long, default_value = "1", allow_negative_numbers = true, value_parser = clap::value_parser!(i32).range(0..))]
        depth: i32,
        /// Maximum total peers to discover.
        ///
        /// Hard cap to prevent unbounded crawling.
        #[arg(long, default_value = "1000")]
        max_peers: usize,
        /// Per-crawl timeout in seconds.
        #[arg(long, default_value = "30")]
        timeout: u64,
        /// Comma-separated node IDs for gossip bootstrapping.
        ///
        /// Supplementary to DHT auto-discovery. These nodes are
        /// contacted first when joining the gossip topic.
        #[arg(long, value_delimiter = ',')]
        bootstrap: Vec<String>,
        /// Gossip topic name for peer discovery.
        ///
        /// Default: "id-peer-discovery-v1" (public network).
        /// Use a custom topic for private networks.
        #[arg(long)]
        topic: Option<String>,
        /// Shared secret for topic access control.
        ///
        /// Used to encrypt DHT bootstrap records.
        #[arg(long)]
        topic_secret: Option<String>,
        /// Skip default bootstrap node IDs from `defaults.conf`.
        ///
        /// Only CLI-provided `--bootstrap` nodes are used.
        #[arg(long)]
        no_default_bootstrap: bool,
        /// Skip default topic and secret from `defaults.conf`.
        ///
        /// Falls back to the hardcoded constant values.
        #[arg(long)]
        no_default_topic: bool,
        /// Use only `defaults.conf` values, ignoring hardcoded fallbacks.
        #[arg(long)]
        replace_defaults: bool,
        /// Disable relay servers and use direct connections only.
        #[arg(long)]
        no_relay: bool,
        /// Disable mDNS local peer discovery.
        ///
        /// By default, peers uses mDNS to discover peers on the
        /// local network. This flag disables that.
        #[arg(long)]
        no_mdns: bool,
        /// Remote node ID to query (64-char hex).
        ///
        /// When set, queries this specific node instead of the
        /// local serve instance.
        #[arg(required = false)]
        node: Option<String>,
        /// Show the local serve instance's per-peer abuse counters instead of
        /// listing peers.
        ///
        /// Read-only: frames and bytes admitted by the abuse evaluator, split
        /// by direction, with the policy rates shown alongside. Local serve
        /// only; cannot be combined with `--gossip` or a remote node ID.
        ///
        /// The counters cover world-protocol traffic only: a serve running
        /// without `--world` records none and the report is empty.
        #[arg(long)]
        abuse: bool,
    },
}

/// Subcommands for `id key`.
#[derive(Subcommand, Debug)]
pub enum KeyCommand {
    /// Export the server identity (or `--client` identity) to an encrypted file.
    /// The passphrase is entered at a hidden prompt, never on the command line.
    Export {
        /// Destination for the encrypted key envelope.
        output: PathBuf,
        /// Export the client identity instead of the server identity.
        #[arg(long)]
        client: bool,
    },
    /// Import an encrypted key envelope as the server or client identity.
    /// The passphrase is entered at a hidden prompt, never on the command line.
    Import {
        /// The encrypted key envelope to import.
        input: PathBuf,
        /// Import as the client identity instead of the server identity.
        #[arg(long)]
        client: bool,
        /// Replace an existing identity key file atomically.
        #[arg(long)]
        replace: bool,
    },
    /// Print the server identity (or `--client` identity) as a 24-word
    /// seed phrase. The phrase IS the key: anyone holding it holds the
    /// identity. It is printed once, never written to a file by this
    /// command, and never taken on the command line.
    ExportSeed {
        /// Export the client identity instead of the server identity.
        #[arg(long)]
        client: bool,
    },
    /// Restore the server or client identity from a 24-word seed phrase
    /// entered at a hidden prompt, never on the command line.
    ImportSeed {
        /// Import as the client identity instead of the server identity.
        #[arg(long)]
        client: bool,
        /// Replace an existing identity key file atomically.
        #[arg(long)]
        replace: bool,
    },
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

/// The `RuntimeLimits::from_args` flag tests, re-homed here from
/// `world_limits` in the Phase 1 crate split (#3447): they parse this
/// crate's CLI, so they live with it; the profile/resolve tests stayed
/// in `id-core`'s `world_limits`.
#[cfg(all(test, feature = "world"))]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod world_limits_args_tests;
