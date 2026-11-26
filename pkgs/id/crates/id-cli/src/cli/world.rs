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
    /// Concurrent synchronous guest burns admitted hub-wide
    /// (#1067); 0 = auto (parallelism − 2).
    #[arg(long, default_value_t = 0, requires = "world", value_name = "COUNT")]
    pub world_guest_burn_concurrency: usize,
    /// Concurrent synchronous guest burns admitted per charged
    /// account (#1067 per-account quotas); 0 = no per-account bound.
    #[arg(long, default_value_t = 0, requires = "world", value_name = "COUNT")]
    pub world_guest_burn_per_account_concurrency: usize,
}

/// Phase 1 of the crate split (#3447): `RuntimeLimits::from_args` lives in
/// `id-core`'s `world_limits` and validates through this trait, which the
/// CLI argument struct implements here in its own crate.
#[cfg(feature = "world")]
impl id_caps::world_limits::WorldLimitArgs for WorldRuntimeArgs {
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
    fn world_guest_burn_concurrency(&self) -> usize {
        self.world_guest_burn_concurrency
    }
    fn world_guest_burn_per_account_concurrency(&self) -> usize {
        self.world_guest_burn_per_account_concurrency
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

/// The arguments of `id world presentation`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751).
#[derive(Args, Debug)]
pub struct WorldPresentationArgs {
    /// The host's node ID (64 hex characters).
    pub node: String,
    /// Replace the policy with the one in this JSON file (needs
    /// `--admin-token`). A duplicated key is refused.
    #[arg(long, value_name = "FILE")]
    pub set: Option<PathBuf>,
    /// Admin secret configured with `serve --world-admin-token`.
    #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
    pub admin_token: Option<String>,
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

/// The arguments of `id world dev`, extracted into
/// their own `Args` struct for the same reason as `ServeArgs`
/// (#4751): keeping clap's derive-generated construction in its
/// own function bounds the stack frame of the parent enum's
/// `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldDevArgs {
    /// App directory holding the app's `main.roc` and its sibling
    /// `.roc` sources.
    pub dir: PathBuf,
    /// World platform directory (or its `platform.roc` file). Default:
    /// the platform path `main.roc` declares, resolved against the
    /// app directory.
    #[arg(long, value_name = "DIR")]
    pub platform: Option<PathBuf>,
    /// Roc compiler binary. Default: `ID_ROC_BIN`, then the bundled
    /// toolchain lookup the host compiler uses.
    #[arg(long, value_name = "BIN")]
    pub roc: Option<String>,
    /// Deterministic seed for the scratch world's guest initializer.
    #[arg(long, default_value_t = 1)]
    pub seed: u64,
    /// Input to send to the scratch world after launch, as the dev
    /// guest (repeatable, in order). The view is reported after each.
    #[arg(long = "input", value_name = "TEXT")]
    pub inputs: Vec<String>,
    /// Fixture to replay against the compiled candidate before
    /// launch (repeatable, in order). A fixture whose replayed
    /// records mismatch its recorded expectation blocks promotion:
    /// the candidate is not launched and the command fails. Fixtures
    /// recorded under an earlier module hash still replay (the dev
    /// cross-module regression mode) and the report says so.
    #[arg(long = "fixture", value_name = "FILE")]
    pub fixtures: Vec<PathBuf>,
    /// Deterministic fuzz declaration file (Phase 4): a bounded,
    /// versioned JSON file naming a generated input alphabet and
    /// the invariants the compiled candidate must hold. Given, the
    /// command compiles the candidate and runs a fuzz campaign
    /// instead of launching a session; it never runs on the
    /// ordinary edit-to-running path.
    #[arg(long = "fuzz", value_name = "FILE")]
    pub fuzz: Option<PathBuf>,
    /// Seed for the fuzz campaign's input generator. The same
    /// declaration, seed, and case budget reproduce the same input
    /// sequences and the same first failure.
    #[arg(long, default_value_t = 42)]
    pub fuzz_seed: u64,
    /// Bounded fuzz case budget (cases are generated input
    /// sequences, each applied to a fresh scratch world).
    #[arg(long, default_value_t = 32)]
    pub fuzz_cases: u32,
    /// Watch the app's declared sources and hot-swap each debounced
    /// change into the scratch world (Phase 2): a bounded,
    /// long-running session that polls for edits, compiles one
    /// candidate per snapshot, and promotes it only when the
    /// fixture gates and the F12.9 migration commit pass — a failed
    /// or stale candidate leaves the last-good program running.
    #[arg(long)]
    pub watch: bool,
    /// Bounded watch poll budget (the session ends on its own
    /// after this many polls).
    #[arg(long, default_value_t = 1000)]
    pub watch_max_cycles: u32,
    /// Debounce window for watch polls, in milliseconds: change
    /// events inside the window coalesce into one candidate.
    #[arg(long, default_value_t = 150)]
    pub watch_debounce_ms: u64,
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

/// The arguments of `id world replica`, extracted into their own
/// `Args` struct for the same reason as `ServeArgs` (#4751): keeping
/// clap's derive-generated construction in its own function bounds
/// the stack frame of the parent enum's `augment_subcommands`.
#[derive(Args, Debug)]
pub struct WorldReplicaArgs {
    /// The writer host's node ID (64 hex characters).
    pub node: String,
    /// Replica directory: the durable journal lives here and is
    /// opened in place on later runs (created on first follow).
    #[arg(long, value_name = "DIR")]
    pub dir: PathBuf,
    /// Admin secret of the writer host, authorizing the raw journal feed.
    #[arg(long, env = "ID_WORLD_ADMIN_TOKEN", hide_env_values = true)]
    pub admin_token: String,
    /// World to replicate (the writer's world name).
    #[arg(long, env = "ID_WORLD", default_value = "lobby")]
    pub world: String,
    /// Pinned writer source key (64 hex characters): the replication
    /// stream and the admission lease must both verify against it.
    /// Defaults to the writer's node ID, which is the key the writer
    /// signs its replication stream with.
    #[arg(long, value_name = "HEX")]
    pub source: Option<String>,
    /// Path to a writer-signed admission lease (JSON) to hold while
    /// serving: new spectators are admitted only while it verifies
    /// (policy B). Without one the replica serves gated, admitting
    /// no new spectators.
    #[arg(long, value_name = "FILE")]
    pub lease: Option<PathBuf>,
    /// Sync once from the writer, print the replica status as JSON,
    /// and exit without serving.
    #[arg(long, conflicts_with = "follow")]
    pub once: bool,
    /// Keep following the writer (reconnecting across sessions),
    /// printing one JSON line per committed cursor; never serves.
    #[arg(long)]
    pub follow: bool,
    /// Direct socket address of the writer host (repeatable); skips discovery.
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
    /// Compile a Roc world app locally and launch it in a disposable
    /// in-memory scratch world — the `id world dev` authoring loop
    /// (F12.15, phase 1: one-shot compile + launch).
    ///
    /// Fully offline: no host is contacted, nothing is installed into
    /// a named world, and the scratch world is discarded when the
    /// session ends. Prints one JSON report: the compiled module's
    /// provenance, the initial view, and the view after each `--input`.
    Dev(WorldDevArgs),
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
    /// View or replace a world's presentation policy.
    ///
    /// Without `--set` it prints the policy: what appearances the world
    /// lets render (user uploads, the richest rung, filters, shaders,
    /// its fallback art). Reading needs no token, because the policy is
    /// public before anyone joins. `--set FILE` replaces it with the
    /// policy in a JSON file and needs `--admin-token`.
    Presentation(WorldPresentationArgs),
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
    /// speaks), or check a recorded session against it. With `--check-suite`,
    /// run the whole conformance suite from a manifest instead.
    ///
    /// A transcript has one JSON object per line:
    /// `{"from": "client"|"server", "frame": {...}}`. The check prints each
    /// violation and fails if there is any.
    Contract {
        /// Transcript to check (`-` reads stdin); without it, print the contract.
        #[arg(long, value_name = "FILE")]
        check: Option<PathBuf>,
        /// Suite manifest to run (`contract/shell-suite.v1.json`): every
        /// shell's capability statement and recorded session, checked in
        /// one pass. Fails if any shell fails.
        #[arg(long, value_name = "FILE", conflicts_with = "check")]
        check_suite: Option<PathBuf>,
    },
    /// Measure a compiled `.wasm` world program's fuel use, offline.
    ///
    /// Compiles the module with the default limits, runs `view`, then
    /// `layout` (with `--width`/`--height`), `records`, `wants`, and
    /// `snapshot`, then each `--input` as participant 1 followed by a `view`.
    /// Prints one JSON object with, per call kind, the calls made, total and
    /// worst fuel, and the worst call as a percent of the fuel limit. Exits
    /// nonzero only if the module fails to compile or a call traps.
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
    /// Follow a pinned writer into a durable replica, open it under
    /// the policy-B admission gate, and serve it (F12.13 phase 5).
    ///
    /// Syncs the replica directory from the writer's signed journal
    /// feed, prints the replica status (source, world, cursor,
    /// staleness) as one JSON object, then serves the replica's world
    /// sessions on this node. `--once` stops after the status print;
    /// `--follow` keeps the replica journal current without serving.
    Replica(WorldReplicaArgs),
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
    /// Export a durable world on disk as a template bundle (#8282).
    ///
    /// Writes the template as JSON: the active program as a bundle
    /// reference and the `kv` entries named with `--keep-state`.
    /// Capabilities, author keys and grants are stripped and only
    /// counted. A program that was not installed from a bundle is
    /// refused. Local only; no host is contacted.
    TemplateExport {
        /// Directory of the world to export (the one holding
        /// `journal.jsonl`).
        dir: PathBuf,
        /// File to write the template to; it must not exist yet.
        out: PathBuf,
        /// A `kv` key to carry as template state (repeatable).
        #[arg(long = "keep-state", value_name = "KEY")]
        keep_state: Vec<String>,
        /// Print one JSON object instead of text lines.
        #[arg(long)]
        json: bool,
    },
    /// Create a new durable world on disk from a template (#8282).
    ///
    /// The world gets a fresh id and starts with the template's state.
    /// Each program the template names installs from a bundle file given
    /// with `--bundle`, through the same verification as a sideloaded
    /// bundle. A program with no matching bundle is recorded in the
    /// world's `template-gaps.json` instead. Local only; no host or
    /// network is contacted.
    #[cfg(feature = "sandbox")]
    TemplateCreate {
        /// Template file written by `id world template-export`.
        template: PathBuf,
        /// Directory to create the world in; it must be absent or empty.
        dir: PathBuf,
        /// The new world's id: `a-z`, `0-9`, `-` and `_`. Defaults to the
        /// directory's name.
        #[arg(long)]
        name: Option<String>,
        /// A program bundle file the template may name (repeatable).
        #[arg(long = "bundle", value_name = "FILE")]
        bundles: Vec<PathBuf>,
        /// An author key (64 hex characters) whose bundles the new world
        /// trusts (repeatable). With none, the world keeps no trust list.
        #[arg(long = "trust-key", value_name = "HEX")]
        trust_keys: Vec<String>,
        /// Deterministic seed supplied to the program's initializer.
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Print one JSON object instead of text lines.
        #[arg(long)]
        json: bool,
    },
    /// Install the programs a template-created world is still missing
    /// (#8282).
    ///
    /// Reads the world's `template-gaps.json` and installs each recorded
    /// program whose exact bundle is given with `--bundle`, through the
    /// same verification as a sideloaded bundle. Programs still missing
    /// stay recorded. Local only; no host or network is contacted.
    #[cfg(feature = "sandbox")]
    TemplateResume {
        /// Directory of the template-created world.
        dir: PathBuf,
        /// A program bundle file that may fill a gap (repeatable).
        #[arg(long = "bundle", value_name = "FILE")]
        bundles: Vec<PathBuf>,
        /// An author key (64 hex characters) whose bundles the world
        /// trusts (repeatable). With none, the world keeps no trust list.
        #[arg(long = "trust-key", value_name = "HEX")]
        trust_keys: Vec<String>,
        /// Deterministic seed supplied to the program's initializer.
        #[arg(long, default_value_t = 1)]
        seed: u64,
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
    /// Print a durable world's owner report (#6308): joins, on-disk
    /// storage, and which facts are unavailable and why. Local only: it
    /// reads the world directory, contacts no host, never wakes a world
    /// and writes nothing, so live-only facts (connected members,
    /// program runs) are reported unavailable. Only the user who owns the
    /// world directory, or root, may read it.
    Owner {
        /// Directory of the world to report on (the one holding
        /// `journal.jsonl`).
        dir: PathBuf,
        /// The window the report covers, ending now, such as 12h or 7d.
        #[arg(long, default_value = "7d", value_parser = crate::helpers::parse_duration_secs)]
        since: u64,
        /// Print one JSON object (schema version 1) instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Inspect or copy a durable world's journal file (F11.12 journal
    /// format management). Local only; no host is contacted.
    Journal(WorldJournalArgs),
    /// Publish a signed world announcement to pkarr relays (#10706).
    ///
    /// Signs with the world key in `--key-file` (an owner-only file,
    /// created on first use) and prints one JSON line per publish.
    /// With `--watch` it stays up and republishes on the heartbeat.
    Announce(WorldAnnounceArgs),
    /// Fetch a world's announcement from pkarr relays by its world id,
    /// verify it, and print it as JSON (#10706).
    Lookup(WorldLookupArgs),
}

/// The arguments of `id world announce` (#10706).
#[derive(Args, Debug)]
pub struct WorldAnnounceArgs {
    /// File holding the world's signing key (created on first use).
    /// The world id is this key's public half.
    #[arg(long)]
    pub key_file: String,
    /// The node id (64 hex characters) that hosts the world.
    #[arg(long)]
    pub node: String,
    /// A relay URL that reaches the node (never a raw IP and port).
    #[arg(long)]
    pub relay_hint: Option<String>,
    /// Display name (at most 64 characters).
    #[arg(long)]
    pub name: String,
    /// Short description (at most 200 characters).
    #[arg(long, default_value = "")]
    pub description: String,
    /// A tag (repeatable, at most 8).
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    /// Mark the world listed, so registries may include it. Unlisted
    /// worlds are found only by their id.
    #[arg(long)]
    pub listed: bool,
    /// Coarse occupancy.
    #[arg(long, default_value = "has_room", value_parser = ["has_room", "near_full", "full"])]
    pub occupancy: String,
    /// A pkarr relay base URL (repeatable). Defaults to the
    /// developing-today relay.
    #[arg(long = "relay")]
    pub relays: Vec<String>,
    /// How long each announcement stays valid, such as 3h.
    #[arg(long, default_value = "3h", value_parser = crate::helpers::parse_duration_secs)]
    pub lifetime: u64,
    /// Stay up and republish on the heartbeat until interrupted.
    #[arg(long)]
    pub watch: bool,
}

/// The arguments of `id world lookup` (#10706).
#[derive(Args, Debug)]
pub struct WorldLookupArgs {
    /// The world id (64 hex characters).
    pub world_id: String,
    /// A pkarr relay base URL (repeatable). Defaults to the
    /// developing-today relay.
    #[arg(long = "relay")]
    pub relays: Vec<String>,
}

/// The arguments of `id world journal`: a subcommand over a world
/// directory's `journal.jsonl` (F11.12).
#[derive(Args, Debug)]
pub struct WorldJournalArgs {
    #[command(subcommand)]
    pub action: JournalCommand,
}

/// What `id world journal` does to a world directory's journal.
#[derive(Subcommand, Debug)]
pub enum JournalCommand {
    /// Check a world directory's journal: report its format version and
    /// validity, flagging a journal written in a newer format than this
    /// binary can read. Exits nonzero when the journal is not valid.
    Check {
        /// Directory of the world to check (the one holding
        /// `journal.jsonl`).
        dir: PathBuf,
        /// Print one JSON object instead of text lines.
        #[arg(long)]
        json: bool,
    },
    /// Copy a world directory's journal to a backup file, byte for byte,
    /// so the copy restores the world exactly. The journal must check
    /// valid first, and the output file must not already exist.
    Backup {
        /// Directory of the world to back up (the one holding
        /// `journal.jsonl`).
        dir: PathBuf,
        /// The backup file to write.
        out: PathBuf,
    },
}
