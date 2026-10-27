//! `id world` subcommand definitions. Split from `cli.rs` (F11.10, #540); moves only.

use std::path::PathBuf;

use clap::{Args, Subcommand};

use crate::cli::WorldRuntimeArgs;

/// Subcommands for `id world`.
#[derive(Subcommand, Debug)]
pub enum WorldCommand {
    /// Mint a guest capability on a world host (needs the host's admin token).
    Invite {
        /// The host's node ID (64 hex characters).
        node: String,
        /// Admin secret configured with `serve --world-admin-token`.
        #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
        admin_token: String,
        /// Display name for the new participant.
        #[arg(long)]
        name: String,
        /// Role preset: spectator sees the world but not the chat, observer
        /// also reads chat, player also inputs, guest also chats, delegator
        /// may also mint narrower capabilities. Defaults to guest.
        #[arg(long, value_parser = ["spectator", "observer", "player", "guest", "delegator"])]
        role: Option<String>,
        /// World on the host to use (default: the host's default world).
        #[arg(long, env = "ID_WORLD")]
        world: Option<String>,
        /// Direct socket address of the host (repeatable); skips discovery.
        #[arg(long = "addr")]
        addrs: Vec<SocketAddr>,
        /// Disable relay servers (direct connection only).
        #[arg(long)]
        no_relay: bool,
        /// How long the invite works, such as `12h` or `3d`.
        ///
        /// Defaults to 7 days; the maximum is 30 days.
        #[arg(long, value_name = "DURATION", value_parser = crate::helpers::parse_duration_secs)]
        expires_in: Option<u64>,
        /// How many committed actions the invite may take (at least 1).
        ///
        /// Unlimited when omitted.
        #[arg(long, value_name = "N")]
        uses: Option<u32>,
    },
    /// Upload and activate a compiled `.wasm` world program on a host.
    Install {
        /// The host's node ID (64 hex characters).
        node: String,
        /// Wasm module built for the `id` Roc world ABI.
        module: PathBuf,
        /// Admin secret configured with `serve --world-admin-token`.
        #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
        admin_token: String,
        /// Deterministic seed supplied to the guest initializer.
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// World on the host to use (default: the host's default world).
        #[arg(long, env = "ID_WORLD")]
        world: Option<String>,
        /// Direct socket address of the host (repeatable); skips discovery.
        #[arg(long = "addr")]
        addrs: Vec<SocketAddr>,
        /// Disable relay servers (direct connection only).
        #[arg(long)]
        no_relay: bool,
    },
    /// Print the world's structured records (a JSON object).
    Records {
        /// The host's node ID (64 hex characters).
        node: String,
        /// Guest capability from `id world invite`.
        #[arg(long, env = "ID_WORLD_CAPABILITY", hide_env_values = true)]
        capability: String,
        /// Only keys starting with this prefix.
        #[arg(long)]
        prefix: Option<String>,
        /// World on the host to use (default: the host's default world).
        #[arg(long, env = "ID_WORLD")]
        world: Option<String>,
        /// Direct socket address of the host (repeatable); skips discovery.
        #[arg(long = "addr")]
        addrs: Vec<SocketAddr>,
        /// Disable relay servers (direct connection only).
        #[arg(long)]
        no_relay: bool,
    },
    /// Replicate the world's records over Iroh with an iroh-docs replica.
    Mirror {
        /// The host's node ID (64 hex characters).
        node: String,
        /// Guest capability from `id world invite`.
        #[arg(long, env = "ID_WORLD_CAPABILITY", hide_env_values = true)]
        capability: String,
        /// Keep printing whenever the records change.
        #[arg(short, long)]
        follow: bool,
        /// Store the replica on disk so it survives between runs.
        #[arg(long, value_name = "PATH")]
        dir: Option<PathBuf>,
        /// Give up after this many seconds without a completed sync.
        #[arg(long, default_value_t = 30)]
        timeout_secs: u64,
        /// World on the host to use (default: the host's default world).
        #[arg(long, env = "ID_WORLD")]
        world: Option<String>,
        /// Direct socket address of the host (repeatable); skips discovery.
        #[arg(long = "addr")]
        addrs: Vec<SocketAddr>,
        /// Disable relay servers (direct connection only).
        #[arg(long)]
        no_relay: bool,
    },
    /// Download the module backing the world's current state, verified
    /// by hash. Prints the module hash; writes the bytes to `--output`.
    Download {
        /// The host's node ID (64 hex characters).
        node: String,
        /// Guest capability from `id world invite`.
        #[arg(long, env = "ID_WORLD_CAPABILITY", hide_env_values = true)]
        capability: String,
        /// Where to write the verified module bytes.
        #[arg(long)]
        output: PathBuf,
        /// World on the host to use (default: the host's default world).
        #[arg(long, env = "ID_WORLD")]
        world: Option<String>,
        /// Direct socket address of the host (repeatable); skips discovery.
        #[arg(long = "addr")]
        addrs: Vec<SocketAddr>,
        /// Disable relay servers (direct connection only).
        #[arg(long)]
        no_relay: bool,
    },
    /// Join a world and chat. Stdin lines are chat; `/input <hex>` sends
    /// opaque input. Server frames print as JSON lines.
    Join {
        /// The host's node ID (64 hex characters).
        node: String,
        /// Guest capability from `id world invite`.
        #[arg(long, env = "ID_WORLD_CAPABILITY", hide_env_values = true)]
        capability: String,
        /// Replay events after this sequence instead of a full snapshot.
        #[arg(long)]
        after: Option<u64>,
        /// World on the host to use (default: the host's default world).
        #[arg(long, env = "ID_WORLD")]
        world: Option<String>,
        /// Direct socket address of the host (repeatable); skips discovery.
        #[arg(long = "addr")]
        addrs: Vec<SocketAddr>,
        /// Disable relay servers (direct connection only).
        #[arg(long)]
        no_relay: bool,
    },
    /// Compile Roc sources on the host and install the result.
    ///
    /// The host rewrites the app's `pf: platform "..."` path to its staged
    /// copy of the world platform, so any relative path works.
    Compile {
        /// The host's node ID (64 hex characters).
        node: String,
        /// The app's main.roc.
        main_file: PathBuf,
        /// Additional source files (repeatable; paths become file names).
        #[arg(short = 'f', long = "file")]
        files: Vec<PathBuf>,
        /// Seed for the compiled program's initializer (default: random).
        #[arg(long)]
        seed: Option<u64>,
        /// Admin secret configured with `serve --world-admin-token`.
        #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
        admin_token: String,
        /// Build a native worker (x64musl) instead of Wasm; the host must run
        /// with `--world-native` and a platform with native targets.
        #[arg(long)]
        native: bool,
        /// World on the host to use (default: the host's default world).
        #[arg(long, env = "ID_WORLD")]
        world: Option<String>,
        /// Direct socket address of the host (repeatable); skips discovery.
        #[arg(long = "addr")]
        addrs: Vec<SocketAddr>,
        /// Disable relay servers (direct connection only).
        #[arg(long)]
        no_relay: bool,
    },
    /// Show (and, with `--grant`/`--revoke`/`--ceiling`, change) a world's capabilities.
    ///
    /// Without `--grant`/`--revoke`/`--ceiling` this needs only a guest capability and
    /// prints what the world has granted, what its program wants, and what it
    /// has used. With them it needs the admin token.
    Caps {
        /// The host's node ID (64 hex characters).
        node: String,
        /// Guest capability from `id world invite`.
        #[arg(long, env = "ID_WORLD_CAPABILITY", hide_env_values = true)]
        capability: Option<String>,
        /// Admin secret configured with `serve --world-admin-token`.
        #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
        admin_token: Option<String>,
        /// Capability to grant (repeatable; `*` grants the whole catalog).
        #[arg(long)]
        grant: Vec<String>,
        /// Capability to revoke (repeatable).
        #[arg(long)]
        revoke: Vec<String>,
        /// Capability ceiling to set (comma-separated or repeatable; only known capabilities, no `*`).
        #[arg(long, value_delimiter = ',')]
        ceiling: Option<Vec<String>>,
        /// World on the host to use (default: the host's default world).
        #[arg(long, env = "ID_WORLD")]
        world: Option<String>,
        /// Direct socket address of the host (repeatable); skips discovery.
        #[arg(long = "addr")]
        addrs: Vec<SocketAddr>,
        /// Disable relay servers (direct connection only).
        #[arg(long)]
        no_relay: bool,
    },
    /// View or configure a world's requested execution profile bounds.
    ///
    /// When any limit option (`--fuel`, `--wasm-memory`, etc.) is specified,
    /// `--admin-token` is required to persist the new profile. When no limits
    /// are specified, prints the current persisted profile (requires either
    /// `--admin-token` or a guest `--capability`).
    Profile {
        /// The host's node ID (64 hex characters).
        node: String,
        /// Admin secret configured with `serve --world-admin-token`.
        #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
        admin_token: Option<String>,
        /// Guest capability from `id world invite` (sufficient to inspect the profile).
        #[arg(long, env = "ID_WORLD_CAPABILITY", hide_env_values = true)]
        capability: Option<String>,
        /// Requested Wasm fuel limit per transaction/event.
        #[arg(long, visible_alias = "fuel")]
        wasm_fuel: Option<u64>,
        /// Requested Wasm linear memory in bytes.
        #[arg(long, visible_alias = "wasm-memory")]
        wasm_memory_bytes: Option<u64>,
        /// Requested Wasm message payload limit in bytes.
        #[arg(long, visible_alias = "wasm-message")]
        wasm_message_bytes: Option<u64>,
        /// Requested native address-space cap in bytes.
        #[arg(long, visible_alias = "native-memory")]
        native_memory_bytes: Option<u64>,
        /// Requested native CPU cap in seconds.
        #[arg(long, visible_alias = "native-cpu")]
        native_cpu_seconds: Option<u64>,
        /// Requested native call deadline in milliseconds.
        #[arg(long, visible_alias = "native-deadline")]
        native_call_deadline_ms: Option<u64>,
        /// World on the host to use (default: the host's default world).
        #[arg(long, env = "ID_WORLD")]
        world: Option<String>,
        /// Direct socket address of the host (repeatable); skips discovery.
        #[arg(long = "addr")]
        addrs: Vec<SocketAddr>,
        /// Disable relay servers (direct connection only).
        #[arg(long)]
        no_relay: bool,
    },
    /// Scaffold a new Roc world app directory.
    ///
    /// Writes `main.roc`, `build.sh`, and `README.md`. The platform path in the
    /// scaffold points at `../roc-world/platform.roc`, so create the directory
    /// next to a checkout of the `examples/roc-world` platform.
    New {
        /// Directory to create; it must be absent or empty.
        dir: PathBuf,
        /// App name used in the README and the Wasm output.
        #[arg(long, default_value = "world")]
        name: String,
    },
    /// Measure a compiled `.wasm` world program's fuel use, offline.
    ///
    /// Compiles the module with the default limits, runs `view`, then
    /// `layout` (with `--width`/`--height`), `records`, `wants`, and
    /// `snapshot`, then each `--input` as participant 1 followed by a `view`.
    /// Prints one JSON object with, per call kind, the calls made, total and
    /// worst fuel, and the worst call as a percent of the fuel limit. Exits
    /// nonzero only if the module fails to compile or a call traps.
    #[cfg(feature = "sandbox")]
    Measure {
        /// Wasm module built for the `id` Roc world ABI.
        #[arg(long)]
        module: PathBuf,
        /// Deterministic seed supplied to the guest initializer.
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Event to apply as participant 1, hex-encoded (repeatable, in order).
        #[arg(long = "input", value_name = "HEX")]
        inputs: Vec<String>,
        /// Viewport width for the `layout` call (needs `--height`).
        #[arg(long, requires = "height")]
        width: Option<u16>,
        /// Viewport height for the `layout` call (needs `--width`).
        #[arg(long, requires = "width")]
        height: Option<u16>,
    },
    /// Tail a world's recent events (bounded by retention and a deadline).
    Logs {
        /// The host's node ID (64 hex characters).
        node: String,
        /// Guest capability from `id world invite`.
        #[arg(long, env = "ID_WORLD_CAPABILITY", hide_env_values = true)]
        capability: String,
        /// Stop after this many seconds.
        #[arg(long, default_value_t = 5)]
        seconds: u64,
        /// World on the host to use (default: the host's default world).
        #[arg(long, env = "ID_WORLD")]
        world: Option<String>,
        /// Direct socket address of the host (repeatable); skips discovery.
        #[arg(long = "addr")]
        addrs: Vec<SocketAddr>,
        /// Disable relay servers (direct connection only).
        #[arg(long)]
        no_relay: bool,
    },
    /// Print a world's module, capability, and profile state as one JSON object.
    Stats {
        /// The host's node ID (64 hex characters).
        node: String,
        /// Guest capability from `id world invite`.
        #[arg(long, env = "ID_WORLD_CAPABILITY", hide_env_values = true)]
        capability: String,
        /// World on the host to use (default: the host's default world).
        #[arg(long, env = "ID_WORLD")]
        world: Option<String>,
        /// Direct socket address of the host (repeatable); skips discovery.
        #[arg(long = "addr")]
        addrs: Vec<SocketAddr>,
        /// Disable relay servers (direct connection only).
        #[arg(long)]
        no_relay: bool,
    },
    /// Write a world's retained history and records to a replayable fixture
    /// (JSON Lines). Needs only a join capability, never admin rights.
    Record {
        /// The host's node ID (64 hex characters).
        node: String,
        /// Guest capability from `id world invite`.
        #[arg(long, env = "ID_WORLD_CAPABILITY", hide_env_values = true)]
        capability: String,
        /// Write the fixture here instead of standard output.
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
        /// World on the host to use (default: the host's default world).
        #[arg(long, env = "ID_WORLD")]
        world: Option<String>,
        /// Direct socket address of the host (repeatable); skips discovery.
        #[arg(long = "addr")]
        addrs: Vec<SocketAddr>,
        /// Disable relay servers (direct connection only).
        #[arg(long)]
        no_relay: bool,
    },
    /// Replay a recorded fixture offline through a module and compare records.
    /// Prints a verdict line first; exits nonzero when the records differ.
    #[cfg(feature = "sandbox")]
    Replay {
        /// The Wasm module the fixture was recorded under.
        #[arg(long, value_name = "FILE")]
        module: PathBuf,
        /// Fixture written by `id world record`.
        #[arg(long, value_name = "FILE")]
        fixture: PathBuf,
        /// Seed the module was installed with. The admin supplied it at install
        /// time; the fixture does not carry it.
        #[arg(long)]
        seed: u64,
        /// Replay a window that does not start at sequence 1. Its records are
        /// not compared.
        #[arg(long)]
        allow_partial: bool,
    },
    /// List the worlds a host serves (needs the host's admin token).
    List {
        /// The host's node ID (64 hex characters).
        node: String,
        /// Admin secret configured with `serve --world-admin-token`.
        #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
        admin_token: String,
        /// Print one JSON object instead of tab-separated lines.
        #[arg(long)]
        json: bool,
        /// Direct socket address of the host (repeatable); skips discovery.
        #[arg(long = "addr")]
        addrs: Vec<SocketAddr>,
        /// Disable relay servers (direct connection only).
        #[arg(long)]
        no_relay: bool,
    },
    /// Set whether a world's directory may be read from other unisolated
    /// worlds (needs the host's admin token). Worlds are isolated by default;
    /// writes never cross worlds.
    Isolation {
        /// The host's node ID (64 hex characters).
        node: String,
        /// `isolated` (no cross-world reads) or `unisolated` (readable from
        /// other unisolated worlds).
        #[arg(value_parser = ["isolated", "unisolated"])]
        mode: String,
        /// Admin secret configured with `serve --world-admin-token`.
        #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
        admin_token: String,
        /// World on the host to change (default: the host's default world).
        #[arg(long, env = "ID_WORLD")]
        world: Option<String>,
        /// Direct socket address of the host (repeatable); skips discovery.
        #[arg(long = "addr")]
        addrs: Vec<SocketAddr>,
        /// Disable relay servers (direct connection only).
        #[arg(long)]
        no_relay: bool,
    },
    /// Let other unisolated worlds write into a world (needs the host's admin token).
    CrossWorldWrite {
        /// The host's node ID (64 hex characters).
        node: String,
        /// `on` (other unisolated worlds may write here) or `off`.
        #[arg(value_parser = ["on", "off"])]
        mode: String,
        /// Admin secret configured with `serve --world-admin-token`.
        #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
        admin_token: String,
        /// World on the host to change (default: the host's default world).
        #[arg(long, env = "ID_WORLD")]
        world: Option<String>,
        /// Direct socket address of the host (repeatable); skips discovery.
        #[arg(long = "addr")]
        addrs: Vec<SocketAddr>,
        /// Disable relay servers (direct connection only).
        #[arg(long)]
        no_relay: bool,
    },
    /// Force cross-world writes on for an unisolated world, overriding its own
    /// setting (needs the host's admin token). Moderators cannot undo a force.
    ForceCrossWorldWrite {
        /// The host's node ID (64 hex characters).
        node: String,
        /// Admin secret configured with `serve --world-admin-token`.
        #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
        admin_token: String,
        /// World on the host to change (default: the host's default world).
        #[arg(long, env = "ID_WORLD")]
        world: Option<String>,
        /// Direct socket address of the host (repeatable); skips discovery.
        #[arg(long = "addr")]
        addrs: Vec<SocketAddr>,
        /// Disable relay servers (direct connection only).
        #[arg(long)]
        no_relay: bool,
    },
    /// Set a world's file scope (needs the host's admin token).
    FileScope {
        /// The host's node ID (64 hex characters).
        node: String,
        /// `confined` (the world's own sandbox root) or `unrestricted` (any path).
        #[arg(value_parser = ["confined", "unrestricted"])]
        scope: String,
        /// Admin file write override for an isolated world: `on` or `off`.
        #[arg(long = "write-override", value_parser = ["on", "off"])]
        write_override: Option<String>,
        /// Admin secret configured with `serve --world-admin-token`.
        #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
        admin_token: String,
        /// World on the host to change (default: the host's default world).
        #[arg(long, env = "ID_WORLD")]
        world: Option<String>,
        /// Direct socket address of the host (repeatable); skips discovery.
        #[arg(long = "addr")]
        addrs: Vec<SocketAddr>,
        /// Disable relay servers (direct connection only).
        #[arg(long)]
        no_relay: bool,
    },
    /// Create a world on a host (needs the host's admin token).
    Create {
        /// The host's node ID (64 hex characters).
        node: String,
        /// Name of the new world: `a-z`, `0-9`, `-` and `_`.
        name: String,
        /// Admin secret configured with `serve --world-admin-token`.
        #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
        admin_token: String,
        /// Direct socket address of the host (repeatable); skips discovery.
        #[arg(long = "addr")]
        addrs: Vec<SocketAddr>,
        /// Disable relay servers (direct connection only).
        #[arg(long)]
        no_relay: bool,
    },
}

/// Subcommands for `id doc`. A document is a `document.put` event over the content's hash.
#[derive(Subcommand, Debug)]
