//! `id world` subcommand definitions: invites, joins, programs,
//! records, profiles, and the remaining world-management arguments.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};

/// Execution bounds for world programs. Each is validated when the server
/// starts; see `world_limits::RuntimeLimits`.
#[derive(Args, Debug, Clone)]
pub struct WorldRuntimeArgs {
    /// Wasm fuel (instruction budget) for one world call.
    #[arg(
        long,
        default_value_t = 10_000_000,
        requires = "world",
        value_name = "UNITS"
    )]
    pub world_fuel: u64,
    /// Linear memory for a Wasm world program, in MiB (1..=2048).
    #[arg(long, default_value_t = 16, requires = "world", value_name = "MIB")]
    pub world_memory_mib: u64,
    /// Largest Wasm payload (input or output), in KiB (1..=1024).
    #[arg(long, default_value_t = 1024, requires = "world", value_name = "KIB")]
    pub world_message_kib: u64,
    /// Address-space cap for a native worker, in MiB (16..=65536).
    #[arg(long, default_value_t = 256, requires = "world", value_name = "MIB")]
    pub world_native_memory_mib: u64,
    /// CPU seconds a native worker may use in total.
    #[arg(long, default_value_t = 10, requires = "world", value_name = "SECS")]
    pub world_native_cpu_secs: u64,
    /// Wall-clock budget for one native worker call, in milliseconds.
    #[arg(long, default_value_t = 10_000, requires = "world", value_name = "MS")]
    pub world_native_deadline_ms: u64,
    /// Total per-world logical memory budget, in MiB (1..=65536, #557 Phase 1).
    #[arg(long, default_value_t = 64, requires = "world", value_name = "MIB")]
    pub world_total_memory_mib: u64,
}

/// Phase 1 of the crate split (#3447): `RuntimeLimits::from_args` lives in
/// `id-core`'s `world_limits` and validates through this trait, which the
/// CLI argument struct implements here in its own crate.
#[cfg(feature = "world")]
impl crate::world_limits::WorldLimitArgs for WorldRuntimeArgs {
    fn world_fuel(&self) -> u64 {
        self.world_fuel
    }
    fn world_memory_mib(&self) -> u64 {
        self.world_memory_mib
    }
    fn world_message_kib(&self) -> u64 {
        self.world_message_kib
    }
    fn world_native_memory_mib(&self) -> u64 {
        self.world_native_memory_mib
    }
    fn world_native_cpu_secs(&self) -> u64 {
        self.world_native_cpu_secs
    }
    fn world_native_deadline_ms(&self) -> u64 {
        self.world_native_deadline_ms
    }
    fn world_total_memory_mib(&self) -> u64 {
        self.world_total_memory_mib
    }
}

/// How `id world join` reaches a world host.
#[derive(ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorldVia {
    /// An iroh QUIC stream to the host's node ID.
    #[default]
    Iroh,
    /// An exec channel on the host's world SSH server.
    Ssh,
}

/// The arguments of `id world invite`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldInviteArgs {
    /// The host's node ID (64 hex characters).
    pub node: String,
    /// Admin secret configured with `serve --world-admin-token`.
    #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
    pub admin_token: String,
    /// Display name for the new participant.
    #[arg(long)]
    pub name: String,
    /// Role preset: spectator sees the world but not the chat, observer
    /// also reads chat, player also inputs, guest also chats, delegator
    /// may also mint narrower capabilities. Defaults to guest.
    #[arg(long, value_parser = ["spectator", "observer", "player", "guest", "delegator", "builder", "moderator"])]
    pub role: Option<String>,
    /// World on the host to use (default: the host's default world).
    #[arg(long, env = "ID_WORLD")]
    pub world: Option<String>,
    /// Direct socket address of the host (repeatable); skips discovery.
    #[arg(long = "addr")]
    pub addrs: Vec<SocketAddr>,
    /// Disable relay servers (direct connection only).
    #[arg(long)]
    pub no_relay: bool,
    /// How long the invite works, such as `12h` or `3d`.
    ///
    /// Defaults to 7 days; the maximum is 30 days.
    #[arg(long, value_name = "DURATION", value_parser = crate::helpers::parse_duration_secs)]
    pub expires_in: Option<u64>,
    /// How many committed actions the invite may take (at least 1).
    ///
    /// Unlimited when omitted.
    #[arg(long, value_name = "N")]
    pub uses: Option<u32>,
}

/// The arguments of `id world install`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldInstallArgs {
    /// The host's node ID (64 hex characters).
    pub node: String,
    /// Wasm module built for the `id` Roc world ABI.
    pub module: PathBuf,
    /// Admin secret configured with `serve --world-admin-token`.
    #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
    pub admin_token: String,
    /// Deterministic seed supplied to the guest initializer.
    #[arg(long, default_value_t = 1)]
    pub seed: u64,
    /// World on the host to use (default: the host's default world).
    #[arg(long, env = "ID_WORLD")]
    pub world: Option<String>,
    /// Direct socket address of the host (repeatable); skips discovery.
    #[arg(long = "addr")]
    pub addrs: Vec<SocketAddr>,
    /// Disable relay servers (direct connection only).
    #[arg(long)]
    pub no_relay: bool,
}

/// The arguments of `id world records`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldRecordsArgs {
    /// The host's node ID (64 hex characters).
    pub node: String,
    /// Guest capability from `id world invite`.
    #[arg(long, env = "ID_WORLD_CAPABILITY", hide_env_values = true)]
    pub capability: String,
    /// Only keys starting with this prefix.
    #[arg(long)]
    pub prefix: Option<String>,
    /// World on the host to use (default: the host's default world).
    #[arg(long, env = "ID_WORLD")]
    pub world: Option<String>,
    /// Direct socket address of the host (repeatable); skips discovery.
    #[arg(long = "addr")]
    pub addrs: Vec<SocketAddr>,
    /// Disable relay servers (direct connection only).
    #[arg(long)]
    pub no_relay: bool,
}

/// The arguments of `id world mirror`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldMirrorArgs {
    /// The host's node ID (64 hex characters).
    pub node: String,
    /// Guest capability from `id world invite`.
    #[arg(long, env = "ID_WORLD_CAPABILITY", hide_env_values = true)]
    pub capability: String,
    /// Keep printing whenever the records change.
    #[arg(short, long)]
    pub follow: bool,
    /// Store the replica on disk so it survives between runs.
    #[arg(long, value_name = "PATH")]
    pub dir: Option<PathBuf>,
    /// Give up after this many seconds without a completed sync (maximum 24 hours).
    #[arg(
        long,
        default_value_t = 30,
        value_parser = clap::value_parser!(u64).range(0..=86_400)
    )]
    pub timeout_secs: u64,
    /// World on the host to use (default: the host's default world).
    #[arg(long, env = "ID_WORLD")]
    pub world: Option<String>,
    /// Direct socket address of the host (repeatable); skips discovery.
    #[arg(long = "addr")]
    pub addrs: Vec<SocketAddr>,
    /// Disable relay servers (direct connection only).
    #[arg(long)]
    pub no_relay: bool,
}

/// The arguments of `id world download`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldDownloadArgs {
    /// The host's node ID (64 hex characters).
    pub node: String,
    /// Guest capability from `id world invite`.
    #[arg(long, env = "ID_WORLD_CAPABILITY", hide_env_values = true)]
    pub capability: String,
    /// Where to write the verified module bytes.
    #[arg(long)]
    pub output: PathBuf,
    /// World on the host to use (default: the host's default world).
    #[arg(long, env = "ID_WORLD")]
    pub world: Option<String>,
    /// Direct socket address of the host (repeatable); skips discovery.
    #[arg(long = "addr")]
    pub addrs: Vec<SocketAddr>,
    /// Disable relay servers (direct connection only).
    #[arg(long)]
    pub no_relay: bool,
}

/// The arguments of `id world join`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldJoinArgs {
    /// The host's node ID (64 hex characters).
    pub node: String,
    /// Guest capability from `id world invite`.
    #[arg(long, env = "ID_WORLD_CAPABILITY", hide_env_values = true)]
    pub capability: String,
    /// Replay events after this sequence instead of a full snapshot.
    #[arg(long)]
    pub after: Option<u64>,
    /// Parse stdin as commands (`/chat`, `/records`, `/stats`, `/caps`,
    /// `/quit`, ...) instead of sending every line as chat.
    #[arg(long)]
    pub command_mode: bool,
    /// Emit local command responses as JSONL records on stdout.
    #[arg(long, requires = "command_mode")]
    pub json: bool,
    /// How to reach the host: `iroh` (NODE is a node ID) or `ssh` (NODE
    /// is `[user@]host[:port]` of the host's world SSH server, and the
    /// frames travel over an SSH exec channel, for networks and jump
    /// hosts that only pass SSH).
    #[arg(long, value_enum, default_value_t = WorldVia::Iroh)]
    pub via: WorldVia,
    /// With `--via ssh`: the host key fingerprint `id serve` prints
    /// (`SHA256:...`). Without it, the key must be in ~/.ssh/known_hosts.
    #[arg(long, value_name = "FINGERPRINT")]
    pub ssh_host_key: Option<String>,
    /// World on the host to use (default: the host's default world).
    #[arg(long, env = "ID_WORLD")]
    pub world: Option<String>,
    /// Direct socket address of the host (repeatable); skips discovery.
    #[arg(long = "addr")]
    pub addrs: Vec<SocketAddr>,
    /// Disable relay servers (direct connection only).
    #[arg(long)]
    pub no_relay: bool,
}

/// The arguments of `id world sshtunnel`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldSshTunnelArgs {
    /// The host's node ID (64 hex characters).
    pub node: String,
    /// Local address to listen on (loopback only is recommended).
    #[arg(long, value_name = "ADDR")]
    pub listen: Option<String>,
    /// Speak SSH on stdin/stdout (for `ssh -o ProxyCommand`).
    #[arg(long, conflicts_with = "listen")]
    pub stdio: bool,
    /// Direct socket address of the host (repeatable); skips discovery.
    #[arg(long = "addr")]
    pub addrs: Vec<SocketAddr>,
    /// Disable relay servers (direct connection only).
    #[arg(long)]
    pub no_relay: bool,
}

/// The arguments of `id world compile`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldCompileArgs {
    /// The host's node ID (64 hex characters).
    pub node: String,
    /// The app's main.roc.
    pub main_file: PathBuf,
    /// Additional source files (repeatable; paths become file names).
    #[arg(short = 'f', long = "file")]
    pub files: Vec<PathBuf>,
    /// Seed for the compiled program's initializer (default: random).
    #[arg(long)]
    pub seed: Option<u64>,
    /// Admin secret configured with `serve --world-admin-token`.
    #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
    pub admin_token: String,
    /// Build a native worker (x64musl) instead of Wasm; the host must run
    /// with `--world-native` and a platform with native targets.
    #[arg(long)]
    pub native: bool,
    /// World on the host to use (default: the host's default world).
    #[arg(long, env = "ID_WORLD")]
    pub world: Option<String>,
    /// Direct socket address of the host (repeatable); skips discovery.
    #[arg(long = "addr")]
    pub addrs: Vec<SocketAddr>,
    /// Disable relay servers (direct connection only).
    #[arg(long)]
    pub no_relay: bool,
}

/// The arguments of `id world caps`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldCapsArgs {
    /// The host's node ID (64 hex characters).
    pub node: String,
    /// Guest capability from `id world invite`.
    #[arg(long, env = "ID_WORLD_CAPABILITY", hide_env_values = true)]
    pub capability: Option<String>,
    /// Admin secret configured with `serve --world-admin-token`.
    #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
    pub admin_token: Option<String>,
    /// Capability to grant (repeatable; `*` grants the whole catalog).
    #[arg(long)]
    pub grant: Vec<String>,
    /// Capability to revoke (repeatable).
    #[arg(long)]
    pub revoke: Vec<String>,
    /// Capability ceiling to set (comma-separated or repeatable; only known capabilities, no `*`).
    #[arg(long, value_delimiter = ',')]
    pub ceiling: Option<Vec<String>>,
    /// World on the host to use (default: the host's default world).
    #[arg(long, env = "ID_WORLD")]
    pub world: Option<String>,
    /// Direct socket address of the host (repeatable); skips discovery.
    #[arg(long = "addr")]
    pub addrs: Vec<SocketAddr>,
    /// Disable relay servers (direct connection only).
    #[arg(long)]
    pub no_relay: bool,
}

/// The arguments of `id world profile`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldProfileArgs {
    /// The host's node ID (64 hex characters).
    pub node: String,
    /// Admin secret configured with `serve --world-admin-token`.
    #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
    pub admin_token: Option<String>,
    /// Guest capability from `id world invite` (sufficient to inspect the profile).
    #[arg(long, env = "ID_WORLD_CAPABILITY", hide_env_values = true)]
    pub capability: Option<String>,
    /// Requested Wasm fuel limit per transaction/event.
    #[arg(long, visible_alias = "fuel")]
    pub wasm_fuel: Option<u64>,
    /// Requested Wasm linear memory in bytes.
    #[arg(long, visible_alias = "wasm-memory")]
    pub wasm_memory_bytes: Option<u64>,
    /// Requested Wasm message payload limit in bytes.
    #[arg(long, visible_alias = "wasm-message")]
    pub wasm_message_bytes: Option<u64>,
    /// Requested native address-space cap in bytes.
    #[arg(long, visible_alias = "native-memory")]
    pub native_memory_bytes: Option<u64>,
    /// Requested native CPU cap in seconds.
    #[arg(long, visible_alias = "native-cpu")]
    pub native_cpu_seconds: Option<u64>,
    /// Requested native call deadline in milliseconds.
    #[arg(long, visible_alias = "native-deadline")]
    pub native_call_deadline_ms: Option<u64>,
    /// World on the host to use (default: the host's default world).
    #[arg(long, env = "ID_WORLD")]
    pub world: Option<String>,
    /// Direct socket address of the host (repeatable); skips discovery.
    #[arg(long = "addr")]
    pub addrs: Vec<SocketAddr>,
    /// Disable relay servers (direct connection only).
    #[arg(long)]
    pub no_relay: bool,
}

/// The arguments of `id world doctor`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldDoctorArgs {
    /// Roc compiler binary to check (default: $`ID_ROC_BIN`, then the
    /// pinned nightly, then `roc` on PATH).
    #[arg(long, env = "ID_ROC_BIN")]
    pub roc_bin: Option<String>,
    /// World platform directory to check (default: $`ID_ROC_PLATFORM` or
    /// ./examples/roc-world).
    #[arg(long, env = "ID_ROC_PLATFORM")]
    pub platform: Option<PathBuf>,
    /// World journal (`journal.jsonl`) to check for format errors.
    #[arg(long, value_name = "FILE")]
    pub journal: Option<PathBuf>,
    /// Capability ceiling to check (comma-separated or repeatable; only
    /// known capabilities, no `*`).
    #[arg(long, value_delimiter = ',')]
    pub ceiling: Option<Vec<String>>,
    /// Execution profile to check, as a JSON file whose fields are the
    /// `WorldExecutionProfile` bounds (e.g. `{"wasm_fuel": 0}`).
    #[arg(long, value_name = "FILE")]
    pub profile: Option<PathBuf>,
    /// Print one JSON object instead of text.
    #[arg(long)]
    pub json: bool,
}

/// The arguments of `id world diff`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldDiffArgs {
    /// The host's node ID (64 hex characters).
    pub node: String,
    /// Path to the `world.toml` to compare against.
    #[arg(long, value_name = "FILE")]
    pub file: PathBuf,
    /// Guest capability from `id world invite` (enough to read the world).
    #[arg(long, env = "ID_WORLD_CAPABILITY", hide_env_values = true)]
    pub capability: String,
    /// World on the host to use (default: the host's default world).
    #[arg(long, env = "ID_WORLD")]
    pub world: Option<String>,
    /// Direct socket address of the host (repeatable); skips discovery.
    #[arg(long = "addr")]
    pub addrs: Vec<SocketAddr>,
    /// Disable relay servers (direct connection only).
    #[arg(long)]
    pub no_relay: bool,
}

/// The arguments of `id world apply`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldApplyArgs {
    /// The host's node ID (64 hex characters).
    pub node: String,
    /// Path to the `world.toml` to apply.
    #[arg(long, value_name = "FILE")]
    pub file: PathBuf,
    /// Admin secret configured with `serve --world-admin-token`.
    #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
    pub admin_token: String,
    /// Guest capability from `id world invite`, used to read the current
    /// state. Required for applying (it reads before it writes).
    #[arg(long, env = "ID_WORLD_CAPABILITY", hide_env_values = true)]
    pub capability: String,
    /// Print the plan without changing the world.
    #[arg(long)]
    pub dry_run: bool,
    /// World on the host to use (default: the host's default world).
    #[arg(long, env = "ID_WORLD")]
    pub world: Option<String>,
    /// Direct socket address of the host (repeatable); skips discovery.
    #[arg(long = "addr")]
    pub addrs: Vec<SocketAddr>,
    /// Disable relay servers (direct connection only).
    #[arg(long)]
    pub no_relay: bool,
}

/// The arguments of `id world measure`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldMeasureArgs {
    /// Wasm module built for the `id` Roc world ABI.
    #[arg(long)]
    pub module: PathBuf,
    /// Deterministic seed supplied to the guest initializer.
    #[arg(long, default_value_t = 1)]
    pub seed: u64,
    /// Event to apply as participant 1, hex-encoded (repeatable, in order).
    #[arg(long = "input", value_name = "HEX")]
    pub inputs: Vec<String>,
    /// Viewport width for the `layout` call (needs `--height`).
    #[arg(long, requires = "height")]
    pub width: Option<u16>,
    /// Viewport height for the `layout` call (needs `--width`).
    #[arg(long, requires = "width")]
    pub height: Option<u16>,
}

/// The arguments of `id world logs`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldLogsArgs {
    /// The host's node ID (64 hex characters).
    pub node: String,
    /// Guest capability from `id world invite`.
    #[arg(long, env = "ID_WORLD_CAPABILITY", hide_env_values = true)]
    pub capability: String,
    /// Stop after this many seconds (maximum 24 hours).
    #[arg(
        long,
        default_value_t = 5,
        value_parser = clap::value_parser!(u64).range(0..=86_400)
    )]
    pub seconds: u64,
    /// World on the host to use (default: the host's default world).
    #[arg(long, env = "ID_WORLD")]
    pub world: Option<String>,
    /// Direct socket address of the host (repeatable); skips discovery.
    #[arg(long = "addr")]
    pub addrs: Vec<SocketAddr>,
    /// Disable relay servers (direct connection only).
    #[arg(long)]
    pub no_relay: bool,
}

/// The arguments of `id world stats`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldStatsArgs {
    /// The host's node ID (64 hex characters).
    pub node: String,
    /// Guest capability from `id world invite`.
    #[arg(long, env = "ID_WORLD_CAPABILITY", hide_env_values = true)]
    pub capability: String,
    /// World on the host to use (default: the host's default world).
    #[arg(long, env = "ID_WORLD")]
    pub world: Option<String>,
    /// Direct socket address of the host (repeatable); skips discovery.
    #[arg(long = "addr")]
    pub addrs: Vec<SocketAddr>,
    /// Disable relay servers (direct connection only).
    #[arg(long)]
    pub no_relay: bool,
}

/// The arguments of `id world record`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldRecordArgs {
    /// The host's node ID (64 hex characters).
    pub node: String,
    /// Host admin token.
    #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
    pub admin_token: String,
    /// Write the fixture here instead of standard output.
    #[arg(long, value_name = "FILE")]
    pub out: Option<PathBuf>,
    /// World on the host to use (default: the host's default world).
    #[arg(long, env = "ID_WORLD")]
    pub world: Option<String>,
    /// Direct socket address of the host (repeatable); skips discovery.
    #[arg(long = "addr")]
    pub addrs: Vec<SocketAddr>,
    /// Disable relay servers (direct connection only).
    #[arg(long)]
    pub no_relay: bool,
}

/// The arguments of `id world replay`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldReplayArgs {
    /// The Wasm module the fixture was recorded under.
    #[arg(long, value_name = "FILE")]
    pub module: PathBuf,
    /// Fixture written by `id world record`.
    #[arg(long, value_name = "FILE")]
    pub fixture: PathBuf,
    /// Seed the module was installed with. The admin supplied it at install
    /// time; the fixture does not carry it.
    #[arg(long)]
    pub seed: u64,
    /// Replay a window that does not start at sequence 1. Its records are
    /// not compared.
    #[arg(long)]
    pub allow_partial: bool,
}

/// The arguments of `id world verify`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldVerifyArgs {
    /// The Wasm module the fixture was recorded under.
    #[arg(long, value_name = "FILE")]
    pub module: PathBuf,
    /// Fixture written by `id world record`.
    #[arg(long, value_name = "FILE")]
    pub fixture: PathBuf,
    /// Seed the module was installed with. The admin supplied it at install
    /// time; the fixture does not carry it.
    #[arg(long)]
    pub seed: u64,
    /// Replay a window that does not start at sequence 1. Its records are
    /// not compared against the fixture's header, but the two replays are
    /// still compared against each other.
    #[arg(long)]
    pub allow_partial: bool,
}

/// The arguments of `id world list`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldListArgs {
    /// The host's node ID (64 hex characters).
    pub node: String,
    /// Admin secret configured with `serve --world-admin-token`.
    /// Optional: omit it to read the public directory.
    #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
    pub admin_token: Option<String>,
    /// Print one JSON object instead of tab-separated lines.
    #[arg(long)]
    pub json: bool,
    /// Direct socket address of the host (repeatable); skips discovery.
    #[arg(long = "addr")]
    pub addrs: Vec<SocketAddr>,
    /// Disable relay servers (direct connection only).
    #[arg(long)]
    pub no_relay: bool,
}

/// The arguments of `id world isolation`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldIsolationArgs {
    /// The host's node ID (64 hex characters).
    pub node: String,
    /// `isolated` (no cross-world reads) or `unisolated` (readable from
    /// other unisolated worlds).
    #[arg(value_parser = ["isolated", "unisolated"])]
    pub mode: String,
    /// Admin secret configured with `serve --world-admin-token`.
    #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
    pub admin_token: String,
    /// World on the host to change (default: the host's default world).
    #[arg(long, env = "ID_WORLD")]
    pub world: Option<String>,
    /// Direct socket address of the host (repeatable); skips discovery.
    #[arg(long = "addr")]
    pub addrs: Vec<SocketAddr>,
    /// Disable relay servers (direct connection only).
    #[arg(long)]
    pub no_relay: bool,
}

/// The arguments of `id world crossworldwrite`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldCrossWorldWriteArgs {
    /// The host's node ID (64 hex characters).
    pub node: String,
    /// `on` (other unisolated worlds may write here) or `off`.
    #[arg(value_parser = ["on", "off"])]
    pub mode: String,
    /// Admin secret configured with `serve --world-admin-token`.
    #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
    pub admin_token: String,
    /// World on the host to change (default: the host's default world).
    #[arg(long, env = "ID_WORLD")]
    pub world: Option<String>,
    /// Direct socket address of the host (repeatable); skips discovery.
    #[arg(long = "addr")]
    pub addrs: Vec<SocketAddr>,
    /// Disable relay servers (direct connection only).
    #[arg(long)]
    pub no_relay: bool,
}

/// The arguments of `id world forcecrossworldwrite`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldForceCrossWorldWriteArgs {
    /// The host's node ID (64 hex characters).
    pub node: String,
    /// Admin secret configured with `serve --world-admin-token`.
    #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
    pub admin_token: String,
    /// World on the host to change (default: the host's default world).
    #[arg(long, env = "ID_WORLD")]
    pub world: Option<String>,
    /// Direct socket address of the host (repeatable); skips discovery.
    #[arg(long = "addr")]
    pub addrs: Vec<SocketAddr>,
    /// Disable relay servers (direct connection only).
    #[arg(long)]
    pub no_relay: bool,
}

/// The arguments of `id world filescope`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldFileScopeArgs {
    /// The host's node ID (64 hex characters).
    pub node: String,
    /// `confined` (the world's own sandbox root) or `unrestricted` (any path).
    #[arg(value_parser = ["confined", "unrestricted"])]
    pub scope: String,
    /// Admin file write override for an isolated world: `on` or `off`.
    #[arg(long = "write-override", value_parser = ["on", "off"])]
    pub write_override: Option<String>,
    /// Admin secret configured with `serve --world-admin-token`.
    #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
    pub admin_token: String,
    /// World on the host to change (default: the host's default world).
    #[arg(long, env = "ID_WORLD")]
    pub world: Option<String>,
    /// Direct socket address of the host (repeatable); skips discovery.
    #[arg(long = "addr")]
    pub addrs: Vec<SocketAddr>,
    /// Disable relay servers (direct connection only).
    #[arg(long)]
    pub no_relay: bool,
}

/// Subcommands for `id world`.
#[derive(Subcommand, Debug)]
pub enum WorldCommand {
    /// Mint a guest capability on a world host (needs the host's admin token).
    Invite(WorldInviteArgs),
    /// Upload and activate a compiled `.wasm` world program on a host.
    Install(WorldInstallArgs),
    /// Print the world's structured records (a JSON object).
    Records(WorldRecordsArgs),
    /// Replicate the world's records over Iroh with an iroh-docs replica.
    Mirror(WorldMirrorArgs),
    /// Download the module backing the world's current state, verified
    /// by hash. Prints the module hash; writes the bytes to `--output`.
    Download(WorldDownloadArgs),
    /// Join a world and chat. Stdin lines are chat; `/input <hex>` sends
    /// opaque input. Server frames print as JSON lines.
    Join(WorldJoinArgs),
    /// Reach a host's world SSH server through iroh, so an ordinary `ssh`
    /// client can reach a world behind NAT: `ssh -p 2222 <world>@127.0.0.1`
    /// (password: the guest capability). With `--stdio`, acts as an SSH
    /// ProxyCommand instead of listening.
    SshTunnel(WorldSshTunnelArgs),
    /// Compile Roc sources on the host and install the result.
    ///
    /// The host rewrites the app's `pf: platform "..."` path to its staged
    /// copy of the world platform, so any relative path works.
    Compile(WorldCompileArgs),
    /// Show (and, with `--grant`/`--revoke`/`--ceiling`, change) a world's capabilities.
    ///
    /// Without `--grant`/`--revoke`/`--ceiling` this needs only a guest capability and
    /// prints what the world has granted, what its program wants, and what it
    /// has used. With them it needs the admin token.
    Caps(WorldCapsArgs),
    /// View or configure a world's requested execution profile bounds.
    ///
    /// When any limit option (`--fuel`, `--wasm-memory`, etc.) is specified,
    /// `--admin-token` is required to persist the new profile. When no limits
    /// are specified, prints the current persisted profile (requires either
    /// `--admin-token` or a guest `--capability`).
    Profile(WorldProfileArgs),
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
    /// Diagnose a broken world setup and print the fix for each problem.
    ///
    /// Checks the Roc toolchain, the platform build, the capability ceiling,
    /// the execution profile, the journal format and the clock. Exits nonzero
    /// only when a check FAILS; a warning does not fail the command. The
    /// ceiling and profile checks run against the values given with
    /// `--ceiling` / `--profile`; without them they report that none is
    /// configured. `--json` prints one machine-readable report.
    Doctor(WorldDoctorArgs),
    /// Show the drift between a `world.toml` and a live world (read-only).
    Diff(WorldDiffArgs),
    /// Reconcile a live world to a `world.toml` (idempotent; journaled).
    ///
    /// Prints the plan and applies it. `--dry-run` prints the plan only.
    /// Every step — module install, grants, ceiling, profile, schedules —
    /// is applied through the admin operations and journaled by the host.
    Apply(WorldApplyArgs),
    /// Print the versioned shell contract (the frame protocol every shell
    /// speaks), or check a recorded session against it.
    ///
    /// A transcript has one JSON object per line:
    /// `{"from": "client"|"server", "frame": {...}}`. The check prints each
    /// violation and fails if there is any.
    Contract {
        /// Transcript to check (`-` reads stdin); without it, print the contract.
        #[arg(long, value_name = "FILE")]
        check: Option<PathBuf>,
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
    Measure(WorldMeasureArgs),
    /// Tail a world's recent events (bounded by retention and a deadline).
    Logs(WorldLogsArgs),
    /// Print a world's module, capability, and profile state as one JSON object.
    Stats(WorldStatsArgs),
    /// Write a world's retained journal history and records to a replayable
    /// fixture (JSON Lines). Host events require the host's admin token.
    Record(WorldRecordArgs),
    /// Replay a recorded fixture offline through a module and compare records.
    /// Prints a verdict line first; exits nonzero when the records differ.
    #[cfg(feature = "sandbox")]
    Replay(WorldReplayArgs),
    /// Verify a recorded fixture's determinism: replay it twice offline
    /// through a module and compare the records after every event. Prints a
    /// verdict line first; exits nonzero at the first divergence, or when
    /// the two replays agree with each other but mismatch the fixture's
    /// header records (the header verdict is printed, as `replay` prints
    /// it).
    ///
    /// Verification covers the events captured by the admin-authorized fixture,
    /// including host-originated events in their recorded order. It proves
    /// determinism for that history, not correctness of host policy.
    #[cfg(feature = "sandbox")]
    Verify(WorldVerifyArgs),
    /// List the worlds a host serves. Without `--admin-token` this reads
    /// the host's public directory (#812: names, loaded state, presence
    /// counts); with it, the admin listing is used.
    List(WorldListArgs),
    /// Set whether a world's directory may be read from other unisolated
    /// worlds (needs the host's admin token). Worlds are isolated by default;
    /// writes never cross worlds.
    Isolation(WorldIsolationArgs),
    /// Let other unisolated worlds write into a world (needs the host's admin token).
    CrossWorldWrite(WorldCrossWorldWriteArgs),
    /// Force cross-world writes on for an unisolated world, overriding its own
    /// setting (needs the host's admin token). Moderators cannot undo a force.
    ForceCrossWorldWrite(WorldForceCrossWorldWriteArgs),
    /// Set a world's file scope (needs the host's admin token).
    FileScope(WorldFileScopeArgs),
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
    /// Fork a durable world on disk into a new world (F12.2).
    ///
    /// Copies the source world's journal and modules into the destination
    /// directory under a new world id and records the fork provenance in
    /// the new journal. Only the journal and the modules are copied: no
    /// seed secret or other source file comes along (the F11.11 policy).
    /// Local only; no host is contacted.
    Fork {
        /// Directory of the world to fork (the one holding `journal.jsonl`).
        source: PathBuf,
        /// Directory to create for the fork; it must be absent or empty.
        dir: PathBuf,
        /// The new world's id: `a-z`, `0-9`, `-` and `_`. Defaults to the
        /// destination directory's name, which is the id a host serving
        /// that directory will open it under.
        #[arg(long)]
        name: Option<String>,
        /// Mark the fork reusable as a template (recorded in its
        /// provenance; see `id world provenance`).
        #[arg(long)]
        template: bool,
        /// Print one JSON object instead of text lines.
        #[arg(long)]
        json: bool,
    },
    /// Print a durable world's provenance from its journal: its id and,
    /// for a fork, the world it was forked from, the module hash at the
    /// fork point, and whether it was marked a template (F12.2). Local
    /// only; no host is contacted.
    Provenance {
        /// Directory of the world to inspect (the one holding
        /// `journal.jsonl`).
        dir: PathBuf,
        /// Print one JSON object instead of text lines.
        #[arg(long)]
        json: bool,
    },
}
