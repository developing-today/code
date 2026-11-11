//! Server command and lock file management.
//!
//! This module implements the `serve` command which starts a persistent
//! server that accepts connections from peers for blob storage and retrieval.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                     Serve Process                           │
//! ├─────────────────────────────────────────────────────────────┤
//! │  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐      │
//! │  │  Endpoint   │    │   Router    │    │   Store     │      │
//! │  │  (QUIC)     │───►│             │───►│ (blobs/tags)│      │
//! │  └─────────────┘    └─────────────┘    └─────────────┘      │
//! │         │                  │                                 │
//! │         │           ┌──────┴──────┐                          │
//! │         │           │             │                          │
//! │         │     ┌─────▼─────┐ ┌─────▼─────┐                    │
//! │         │     │MetaProtocol│ │BlobsProtocol│                    │
//! │         │     │ /id/meta/1 │ │ /iroh/blobs │                    │
//! │         │     └───────────┘ └───────────┘                    │
//! │         │                                                    │
//! │         ▼                                                    │
//! │  Lock File: .id-serve-lock                                   │
//! │  - Node ID, PID, Socket addresses                            │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Lock File Protocol
//!
//! The serve process creates a lock file (`.id-serve-lock`) containing:
//! 1. Node ID (line 1)
//! 2. Process ID (line 2)
//! 3. Socket addresses (remaining lines)
//!
//! Other processes (REPL, CLI commands) check this file to determine
//! if a local serve is running and how to connect to it.
//!
//! # Examples
//!
//! ```bash
//! # Start persistent server
//! id serve
//!
//! # Start ephemeral server (in-memory)
//! id serve --ephemeral
//!
//! # Start without relay servers
//! id serve --no-relay
//! ```

use std::collections::HashSet;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use anyhow::ensure;
use distributed_topic_tracker::{AutoDiscoveryGossip, RecordPublisher, TopicId};
use futures_lite::StreamExt;
use iroh::{endpoint::Endpoint, protocol::Router};
use iroh_base::EndpointId;
use iroh_blobs::{ALPN as BLOBS_ALPN, BlobsProtocol};
use iroh_docs::protocol::Docs;
use iroh_gossip::net::Gossip;
use iroh_mdns_address_lookup::MdnsAddressLookup;
use p2panda_core::Hash;
#[cfg(all(feature = "world", feature = "sandbox"))]
use rand::RngExt as _;
use serde::{Deserialize, Serialize};
use tokio::fs as afs;
use tracing::{debug, info, warn};

use crate::chat_presence::{
    HEARTBEAT_INTERVAL, PresenceHeartbeat, PresenceTable, decode_frame, encode_frame,
};
use crate::discovery::{
    ANNOUNCE_INTERVAL, PeerAnnouncement, PeerDiscovery, STALE_CHECK_INTERVAL, STALE_THRESHOLD,
    defaults, resolve_config,
};
use crate::groups::log::{FILE_NAME as GROUPS_FILE_NAME, GroupLog};
use crate::groups::warnings::{Evaluator, Observation, Warning};
use crate::groups::{Key as GroupKey, Projection};
use crate::peer_announcement::SignedPeerAnnouncement;
use crate::protocol::{MetaProtocol, MetaRequest, MetaResponse};
use crate::store::{load_or_create_keypair, open_store};
use crate::tags::TagStore;
use crate::waiting::AuthorKey;
use crate::{KEY_FILE, META_ALPN, SERVE_LOCK, STORE_PATH};

/// Print a status line to stdout **without panicking** if stdout is gone.
///
/// `println!` panics on `EPIPE`. A server must not die because whoever
/// launched it stopped reading its output (`id serve | head -1`, a supervisor
/// that closes the pipe after the first line, or a test harness): the status
/// lines are informational. The lock file, not stdout, is how clients find
/// the server.
macro_rules! status {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout(), $($arg)*);
    }};
}

/// Like [`status!`], for stderr.
#[cfg(any(feature = "web", feature = "world"))]
macro_rules! status_err {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stderr(), $($arg)*);
    }};
}

/// Information about a running serve instance.
///
/// Retrieved from the lock file by [`get_serve_info`] to enable
/// other processes to connect to the local serve.
///
/// # Fields
///
/// - `node_id`: The public identity of the serve node
/// - `addrs`: Local socket addresses where the serve is listening
///
/// # JSON Lock File Format
///
/// ```json
/// {
///   "node_id": "abc123...",
///   "pid": 12345,
///   "addrs": ["127.0.0.1:12345", "[::1]:12345"],
///   "web_port": 3001
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServeInfo {
    /// The public node ID derived from the serve's keypair.
    pub node_id: String,
    /// Process ID of the running serve instance.
    pub pid: u32,
    /// Socket addresses the serve is bound to (as strings for JSON).
    pub addrs: Vec<String>,
    /// Port for the web UI, if enabled.
    pub web_port: Option<u16>,
    /// Every web listener as `IP:port` strings, in binding order (#2145).
    /// `web_port` is the first listener's port, kept for back-compat.
    /// Defaults to empty so lock files written before #2145 still parse.
    #[serde(default)]
    pub web_bindings: Vec<String>,
}

impl ServeInfo {
    /// Parse the `node_id` string back to an [`EndpointId`].
    pub fn endpoint_id(&self) -> Option<EndpointId> {
        self.node_id.parse().ok()
    }

    /// Parse the address strings back to [`SocketAddr`]s.
    pub fn socket_addrs(&self) -> Vec<SocketAddr> {
        self.addrs.iter().filter_map(|a| a.parse().ok()).collect()
    }
}

/// Checks if a serve instance is running and returns its connection info.
///
/// Reads the JSON lock file, verifies the PID is still alive, and returns
/// the serve info needed to connect. Returns `None` if:
/// - Lock file doesn't exist
/// - Lock file is malformed
/// - Referenced process is no longer running (stale lock)
///
/// # Example
///
/// ```rust,ignore
/// if let Some(info) = get_serve_info().await {
///     println!("Serve running: {}", info.node_id);
///     // Connect to info.addrs...
/// } else {
///     println!("No serve running");
/// }
/// ```
pub async fn get_serve_info() -> Option<ServeInfo> {
    let contents = afs::read_to_string(SERVE_LOCK).await.ok()?;
    let info: ServeInfo = serde_json::from_str(&contents).ok()?;

    // Check if process is still alive
    if !is_process_alive(info.pid) {
        // Stale lock file - remove it
        let _ = afs::remove_file(SERVE_LOCK).await;
        return None;
    }

    Some(info)
}

/// Checks if a process with the given PID is still running.
///
/// Uses platform-specific methods:
/// - Unix: `kill(pid, 0)` which checks existence without sending a signal
/// - Other: Always returns `true` (conservative fallback)
///
/// # Arguments
///
/// * `pid` - The process ID to check
///
/// # Returns
///
/// `true` if the process exists, `false` otherwise.
#[allow(clippy::cast_possible_wrap)] // The Unix branch validates the PID before casting.
#[allow(unsafe_code)] // Required for libc::kill
pub fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // PIDs read from the serve lock are untrusted u32 values. Zero checks
        // the caller's process group, while values above i32::MAX wrap to
        // negative process selectors, so neither can identify one process.
        if pid == 0 || pid > i32::MAX as u32 {
            return false;
        }

        // SAFETY: libc::kill with signal 0 only checks process existence without
        // sending any signal. The check above ensures pid is positive and fits
        // in i32 before the cast.
        // #2384: a failed probe is not proof of death — EPERM means the
        // process exists but is owned by another user, so it is alive;
        // only ESRCH (no such process) and other errnos mean dead. This
        // mirrors mcpx's ProcessAlive (pkgs/mcpx/internal/daemon/paths.go).
        let rc = unsafe { libc::kill(pid as i32, 0) };
        rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        // On non-Unix, just assume it's alive if we have a PID
        let _ = pid;
        true
    }
}

/// Creates the serve lock file with connection information as JSON.
///
/// Writes the node ID, current process ID, socket addresses, and
/// optional web port to the lock file so other processes can discover
/// and connect.
///
/// # Arguments
///
/// * `node_id` - The serve node's public identity
/// * `addrs` - Socket addresses the serve is listening on
/// * `web_port` - Port for the web UI, if enabled
///
/// # Errors
///
/// Returns an error if the lock file cannot be written.
pub async fn create_serve_lock(
    node_id: &EndpointId,
    addrs: &[SocketAddr],
    web_port: Option<u16>,
    web_bindings: Vec<String>,
) -> Result<()> {
    let info = ServeInfo {
        node_id: node_id.to_string(),
        pid: std::process::id(),
        addrs: addrs.iter().map(ToString::to_string).collect(),
        web_port,
        web_bindings,
    };
    let contents = serde_json::to_string_pretty(&info)?;
    afs::write(SERVE_LOCK, contents).await?;
    Ok(())
}

/// Removes the serve lock file.
///
/// Called during graceful shutdown to indicate the serve is no longer running.
/// Errors are silently ignored (file may already be removed).
pub async fn remove_serve_lock() -> Result<()> {
    let _ = afs::remove_file(SERVE_LOCK).await;
    Ok(())
}
/// Options for [`cmd_serve`], mirroring the `serve` CLI flags.
#[derive(Debug, Clone)]
pub struct ServeOptions {
    /// Use an in-memory store.
    pub ephemeral: bool,
    /// Operator server config to load at startup (#5119 slice 1);
    /// `None` reads no file and keeps today's behaviour exactly.
    pub server_config: Option<PathBuf>,
    /// Group log for the group-warning watcher (the global
    /// `--groups-log` option); `None` reads the default
    /// `groups.jsonl`, exactly as the group commands resolve it.
    pub groups_log: Option<PathBuf>,
    /// Disable relay servers.
    pub no_relay: bool,
    /// Disable gossip peer discovery.
    pub no_gossip: bool,
    /// Start the web interface.
    pub web: bool,
    /// Web interface port (0 = random).
    pub port: u16,
    /// Extra bootstrap node IDs.
    pub bootstrap: Vec<String>,
    /// Gossip topic override.
    pub topic: Option<String>,
    /// Gossip topic secret override.
    pub topic_secret: Option<String>,
    /// Skip default bootstrap nodes.
    pub no_default_bootstrap: bool,
    /// Skip default topic and secret.
    pub no_default_topic: bool,
    /// Use only `defaults.conf` values.
    pub replace_defaults: bool,
    /// Disable mDNS.
    pub no_mdns: bool,
    /// Enable mainline DHT address lookup and publishing.
    pub dht: bool,
    /// Iroh QUIC port (0 = random).
    pub iroh_port: u16,
    /// Address the web interface binds to.
    pub bind: std::net::IpAddr,
    /// Repeatable `--web-listen <IP:PORT>` listeners (#2145 bindings model).
    pub web_listen: Vec<SocketAddr>,
    /// Whether `--bind` was explicitly set on the command line (#2145:
    /// only an explicit `--bind`/`--port` conflicts with `--web-listen`).
    pub bind_explicit: bool,
    /// Whether `--port` was explicitly set on the command line (#2145).
    pub port_explicit: bool,
    /// Token required by the web interface.
    pub web_token: Option<String>,
    /// Mark the directory explorer's session cookies `Secure` (HTTPS in front).
    pub web_cookie_secure: bool,
    /// Host a multiplayer world (durable unless `ephemeral`).
    pub world: bool,
    /// Admin secret required to mint world guest capabilities.
    pub world_admin_token: Option<String>,
    /// Optional Wasm world program, compiled for the sandbox's Roc platform ABI.
    pub world_module: Option<PathBuf>,
    /// Directory that receives confirmation mail, one file per message.
    pub world_mail_outbox: Option<PathBuf>,
    /// Program run once per confirmation mail.
    pub world_mail_command: Option<PathBuf>,
    /// Loopback SMTP relay that receives confirmation mail.
    pub world_mail_smtp: Option<SocketAddr>,
    /// Sender address for mail sent through `world_mail_smtp`.
    pub world_mail_from: Option<String>,
    /// Name of the world this server offers; its files live under
    /// `.id-worlds/<name>/`.
    pub world_name: String,
    /// Trim the journal behind a program snapshot every this many events
    /// (`0` never trims).
    pub world_checkpoint_every: u64,
    /// Most worlds open at once.
    pub world_max_open: usize,
    /// Most world sessions at once, across all worlds.
    pub world_max_sessions: usize,
    /// Close durable worlds idle for this many seconds (`0` never).
    pub world_idle_secs: u64,
    /// Refuse signed world requests for this many seconds after startup (`0` none).
    pub world_boot_window_secs: u64,
    /// Capabilities the default world's program may use.
    pub world_caps: Vec<String>,
    /// `deny` or `grant-on-use`.
    pub world_cap_policy: String,
    /// Capabilities the server allows any world to use (`None`: the whole catalog).
    pub world_allowed_caps: Option<Vec<String>>,
    /// Roc binary for on-the-fly compilation.
    pub roc_bin: Option<String>,
    /// Platform directory for on-the-fly compilation.
    pub roc_platform: Option<PathBuf>,
    /// Serve the world over SSH on this port.
    pub world_ssh_port: Option<u16>,
    /// Accept native (ELF) module installs.
    pub world_native: bool,
    /// Accept program changes only through server-side Roc compilation.
    pub world_source_only: bool,
    /// Ed25519 public keys authorized to sign module artifacts (hex).
    pub world_module_trust_keys: Vec<String>,
    /// Execution bounds for world programs.
    pub world_runtime: crate::cli::WorldRuntimeArgs,
    /// Nodes allowed to modify the store.
    pub allow_node: Vec<String>,
    /// Let every peer modify the store.
    pub open_writes: bool,
}

/// What a web listener serves: the content target of a binding
/// (#2146 design, phase 1: the surface contract and compatibility
/// layer). The design's phased plan adds this type and its validation
/// first, defaulting every existing binding to `Explorer`; routing
/// per target is the next phase and is not changed here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebContentTarget {
    /// The World Explorer surface: the legacy `--web` behaviour.
    Explorer,
    /// A world-backed program surface: the program in the named
    /// world is served as a website. The target selector is the
    /// existing world name, spelled `program:<world-name>` (the
    /// design's selector semantics); there is no separate program ID.
    Program(String),
}

impl Default for WebContentTarget {
    fn default() -> Self {
        Self::Explorer
    }
}

impl WebContentTarget {
    /// Build a `Program` target, validating the world name the same
    /// way the #2147 publication registry validates its world names:
    /// non-empty once trimmed. The stored name is the trimmed one.
    ///
    /// # Errors
    ///
    /// Fails on an empty or all-whitespace world name.
    pub fn program(world: impl Into<String>) -> Result<Self> {
        let world = world.into();
        let world = world.trim();
        ensure!(
            !world.is_empty(),
            "program target world name must not be empty (expected program:<world-name>)"
        );
        Ok(Self::Program(world.to_owned()))
    }

    /// Parse one target spec: `explorer` or `program:<world-name>`.
    /// This is the grammar a target is expressed in wherever a
    /// binding is written as text (the `[web_bindings]` lines in
    /// `defaults.conf`), so a target always travels inside its own
    /// binding's spec -- never as a separate repeated flag paired
    /// with listener flags by position (the design forbids that).
    ///
    /// # Errors
    ///
    /// Fails on an unknown spec or an empty program world name,
    /// naming the spec.
    pub fn parse(spec: &str) -> Result<Self> {
        match spec {
            "explorer" => Ok(Self::Explorer),
            _ => match spec.strip_prefix("program:") {
                Some(world) => Self::program(world)
                    .map_err(|e| anyhow::anyhow!("invalid content target {spec:?}: {e}")),
                None => {
                    return Err(anyhow::anyhow!(
                        "unknown content target {spec:?} (expected `explorer` or `program:<world-name>`)"
                    ));
                }
            },
        }
    }

    /// The canonical spelling of this target: `explorer` or
    /// `program:<world-name>`.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Explorer => "explorer".to_owned(),
            Self::Program(world) => format!("program:{world}"),
        }
    }
}

/// One web listener in the bindings model (#2145), with its #2146
/// content target (phase 1: carried and validated, not yet routed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebBinding {
    /// Full socket address, including any IPv6 scope ID (port 0 = OS-assigned).
    pub addr: SocketAddr,
    /// What this listener serves; `Explorer` for every binding the
    /// pre-#2146 sources produce.
    pub target: WebContentTarget,
}

impl WebBinding {
    /// An Explorer binding on `addr` (the legacy surface).
    #[must_use]
    pub fn explorer(addr: SocketAddr) -> Self {
        Self {
            addr,
            target: WebContentTarget::Explorer,
        }
    }

    /// A program binding on `addr` serving the named world.
    ///
    /// # Errors
    ///
    /// Fails on an empty or all-whitespace world name.
    pub fn program(addr: SocketAddr, world: impl Into<String>) -> Result<Self> {
        Ok(Self {
            addr,
            target: WebContentTarget::program(world)?,
        })
    }

    /// What this binding serves.
    #[must_use]
    pub fn target(&self) -> &WebContentTarget {
        &self.target
    }

    /// This binding's stable identity: its content target and socket
    /// address, e.g. `explorer@127.0.0.1:3000` or
    /// `program:arena@127.0.0.1:3001`. Content metadata attaches by
    /// this identity, never by position in the binding list. A
    /// port-0 binding's identity settles only once the OS assigns
    /// the port at bind time; callers must form the identity from
    /// the bound address (see `with_port`), not the requested one.
    #[must_use]
    pub fn identity(&self) -> String {
        format!("{}@{}", self.target.describe(), self.addr)
    }

    /// The socket address this binding listens on.
    #[must_use]
    pub fn socket_addr(&self) -> SocketAddr {
        self.addr
    }

    /// The same socket address with the port selected by the OS.
    #[must_use]
    pub fn with_port(&self, port: u16) -> SocketAddr {
        match self.addr {
            SocketAddr::V4(mut addr) => {
                addr.set_port(port);
                SocketAddr::V4(addr)
            }
            SocketAddr::V6(mut addr) => {
                addr.set_port(port);
                SocketAddr::V6(addr)
            }
        }
    }
}

/// Resolve the effective web listener set, exactly once at startup
/// (#2145 bindings model, designed in triage task 3155):
///
/// 1. `--web-listen` flags, when given at least once, win entirely.
///    Combining them with an explicitly-set `--bind` or `--port` is an
///    error naming the conflicting flag (clap defaults do not count).
/// 2. Else the `[web_bindings]` section of `defaults.conf`, parsed here
///    so a malformed line fails startup naming the line. A config line
///    is `IP:PORT` (an Explorer binding) or `IP:PORT <target>` with a
///    #2146 content target (`explorer` / `program:<world-name>`); the
///    target travels inside the binding's own line, never as a
///    separate list paired by position.
/// 3. Else one binding synthesized from the legacy `--bind`/`--port`.
///
/// # Errors
///
/// Fails on the `--web-listen`/`--bind`/`--port` conflict, or on a
/// malformed `[web_bindings]` line (named in the error), or a duplicate
/// nonzero listener address in the selected source.
pub fn resolve_web_bindings(
    web_listen: &[SocketAddr],
    bind_explicit: bool,
    port_explicit: bool,
    legacy_bind: std::net::IpAddr,
    legacy_port: u16,
    config_bindings: &[String],
) -> Result<Vec<WebBinding>> {
    if !web_listen.is_empty() {
        ensure!(
            !bind_explicit,
            "--web-listen cannot be combined with --bind: pass listener              addresses with --web-listen only, or use --bind/--port for a              single listener"
        );
        ensure!(
            !port_explicit,
            "--web-listen cannot be combined with --port: pass listener              addresses with --web-listen only, or use --bind/--port for a              single listener"
        );
        ensure_unique_web_listener_addresses(web_listen.iter().copied(), "--web-listen")?;
        return Ok(web_listen
            .iter()
            .map(|addr| WebBinding::explorer(*addr))
            .collect());
    }
    if !config_bindings.is_empty() {
        let bindings: Result<Vec<_>> = config_bindings
            .iter()
            .map(|line| {
                parse_web_binding_line(line).map_err(|e| {
                    anyhow::anyhow!("invalid [web_bindings] entry {line:?} in defaults.conf: {e}")
                })
            })
            .collect();
        let bindings = bindings?;
        ensure_unique_web_listener_addresses(
            bindings.iter().map(WebBinding::socket_addr),
            "[web_bindings] in defaults.conf",
        )?;
        return Ok(bindings);
    }
    Ok(vec![WebBinding::explorer(SocketAddr::new(
        legacy_bind,
        legacy_port,
    ))])
}

/// Parse one `[web_bindings]` line: `IP:PORT` for an Explorer
/// binding, or `IP:PORT <target>` with a #2146 content target (see
/// [`WebContentTarget::parse`]).
///
/// # Errors
///
/// Fails naming the offending part: a malformed address, an unknown
/// or invalid target, or extra trailing text on the line.
pub fn parse_web_binding_line(line: &str) -> Result<WebBinding> {
    let mut parts = line.split_whitespace();
    let addr_part = parts.next().unwrap_or_default();
    let addr: SocketAddr = addr_part.parse().map_err(|e| {
        anyhow::anyhow!(
            "{e} (expected IP:PORT, e.g. 127.0.0.1:3000, optionally followed by a content target)"
        )
    })?;
    let target = match parts.next() {
        None => WebContentTarget::Explorer,
        Some(spec) => WebContentTarget::parse(spec)?,
    };
    ensure!(
        parts.next().is_none(),
        "unexpected extra text in web binding line {line:?} (expected IP:PORT [target])"
    );
    Ok(WebBinding { addr, target })
}

fn ensure_unique_web_listener_addresses(
    addresses: impl IntoIterator<Item = SocketAddr>,
    source: &str,
) -> Result<()> {
    let mut seen = HashSet::new();
    for addr in addresses {
        // Port 0 asks the OS for a fresh port on each bind, so repeated port-0
        // entries do not resolve to the same listener address.
        if addr.port() == 0 {
            continue;
        }
        ensure!(
            seen.insert(addr),
            "duplicate web listener address {addr} from {source}"
        );
    }
    Ok(())
}

fn validate_world_options(
    world: bool,
    admin_token: Option<&str>,
    world_module: Option<&PathBuf>,
    world_name: &str,
    world_cap_policy: &str,
    world_source_only: bool,
) -> Result<()> {
    ensure!(
        !world || admin_token.is_some_and(|token| !token.is_empty()),
        "--world requires --world-admin-token (or ID_WORLD_ADMIN_TOKEN)"
    );
    ensure!(
        world_module.is_none() || world,
        "--world-module requires --world"
    );
    #[cfg(feature = "world")]
    crate::world_hub::validate_world_name(world_name)?;
    ensure!(
        world_cap_policy == "deny" || world_cap_policy == "grant-on-use",
        "--world-cap-policy must be `deny` or `grant-on-use`"
    );
    ensure!(
        !world_source_only || world_module.is_none(),
        "--world-source-only cannot be combined with --world-module"
    );
    ensure!(
        !world_source_only || world,
        "--world-source-only requires --world"
    );
    #[cfg(not(feature = "world"))]
    let _ = world_name;
    Ok(())
}

/// Build the write-access policy for a server.
///
/// Trusted automatically: the server's own key and this data directory's
/// client key (so the local CLI and REPL work without setup). Added to those:
/// `--allow-node` IDs and the `.iroh-allowed` file. With `open_writes`, every
/// peer may write.
///
/// # Errors
///
/// Fails if an `--allow-node` value is not a valid node ID.
pub async fn build_access_policy(
    node_id: EndpointId,
    allow_node: &[String],
    open_writes: bool,
) -> Result<crate::access::AccessPolicy> {
    use crate::access::{AccessPolicy, load_allowed_nodes};
    if open_writes {
        return Ok(AccessPolicy::open());
    }
    let client_key = load_or_create_keypair(crate::CLIENT_KEY_FILE).await?;
    let mut writers = vec![node_id, client_key.public()];
    for id in allow_node {
        writers.push(
            id.parse::<EndpointId>()
                .map_err(|e| anyhow::anyhow!("invalid --allow-node {id:?}: {e}"))?,
        );
    }
    writers.extend(load_allowed_nodes(std::path::Path::new(".")));
    Ok(AccessPolicy::restricted(writers))
}

/// Root directory for durable worlds, relative to the data directory.
#[cfg(feature = "world")]
pub const WORLDS_DIR: &str = ".id-worlds";

#[cfg(feature = "world")]
const OUTBOX_FLUSH_SECS: u64 = 30;

/// Directory of one durable world, relative to the data directory.
#[cfg(feature = "world")]
#[must_use]
pub fn world_dir(name: &str) -> PathBuf {
    PathBuf::from(WORLDS_DIR).join(name)
}

/// Resolve the compile settings: an explicit flag, the environment, or the
/// conventional repository layout. Absence is fine (compilation stays off).
#[cfg(feature = "world")]
fn resolve_compiler(
    roc_bin: Option<String>,
    roc_platform: Option<PathBuf>,
) -> Result<Option<crate::world_compile::Compiler>> {
    let Some(platform) = roc_platform
        .or_else(|| std::env::var("ID_ROC_PLATFORM").ok().map(PathBuf::from))
        .or_else(|| {
            [
                PathBuf::from("examples/roc-world"),
                PathBuf::from("../examples/roc-world"),
            ]
            .into_iter()
            .find(|dir| dir.is_dir())
        })
    else {
        return Ok(None);
    };
    let roc_bin = roc_bin
        .or_else(|| std::env::var("ID_ROC_BIN").ok())
        .unwrap_or_else(|| "roc".to_owned());
    let compiler = crate::world_compile::Compiler::new(platform, roc_bin)?;
    info!(
        platform = %compiler.platform_dir.display(),
        roc = %compiler.roc_bin,
        "world: on-the-fly compilation enabled"
    );
    Ok(Some(compiler))
}

/// Builds this server's worlds: durable under [`WORLDS_DIR`] unless
/// `ephemeral`. The `--world-module` goes through the same journaled install
/// as an admin upload, so it is pinned, downloadable and restored after a
/// restart; it is only installed when it differs from the module the world
/// already runs. It applies to the default world only.
#[cfg(feature = "world")]
fn mail_sink(
    outbox: Option<PathBuf>,
    command: Option<PathBuf>,
    smtp: Option<SocketAddr>,
    from: Option<String>,
) -> Result<Option<Arc<dyn crate::directory_mail::MailSink>>> {
    use crate::directory_mail::{CommandSink, MailSink, OutboxSink, SmtpSink};
    use anyhow::Context as _;
    let sink: Arc<dyn MailSink> = match (outbox, command, smtp) {
        (Some(dir), _, _) => Arc::new(OutboxSink::new(dir)),
        (None, Some(program), _) => Arc::new(CommandSink::new(program, Vec::new())),
        (None, None, Some(relay)) => {
            let from = from.context("--world-mail-from is needed with --world-mail-smtp")?;
            Arc::new(SmtpSink::new(relay, from)?)
        }
        (None, None, None) => return Ok(None),
    };
    Ok(Some(sink))
}

/// The server's capability ceiling: the named capabilities, or the default
/// ceiling ([`crate::world_caps::default_ceiling_names`]) when no list is
/// given — providers whose manifest default is `Excluded` do not join a
/// server by registration alone (#838). Like a world ceiling, it names
/// capabilities; `*` is refused.
#[cfg(feature = "world")]
fn allowed_caps(names: Option<&[String]>) -> Result<std::collections::BTreeSet<String>> {
    match names {
        None => Ok(crate::world_caps::default_ceiling_names()),
        Some(names) => {
            crate::world_caps::validate_ceiling_names(names)?;
            Ok(names.iter().cloned().collect())
        }
    }
}

#[cfg(feature = "world")]
struct ServeWorlds {
    ephemeral: bool,
    admin_token: Option<String>,
    blobs: iroh_blobs::api::Store,
    docs: Docs,
    checkpoint_every: u64,
    default_world: String,
    default_module: Option<PathBuf>,
    grant_on_use: bool,
    server_caps: std::collections::BTreeSet<String>,
    compiler: Option<crate::world_compile::Compiler>,
    native: bool,
    source_only: bool,
    trusted_module_keys: Vec<[u8; 32]>,
    runtime: crate::world_limits::RuntimeLimits,
    mail: Option<Arc<dyn crate::directory_mail::MailSink>>,
}

#[cfg(feature = "world")]
impl crate::world_hub::WorldOpener for ServeWorlds {
    fn open<'a>(&'a self, name: &'a str, create: bool) -> crate::world_hub::OpenFuture<'a> {
        Box::pin(async move {
            if !create && !self.exists(name) {
                return Ok(None);
            }
            self.open_world(name).await.map(Some)
        })
    }

    fn exists(&self, name: &str) -> bool {
        !self.ephemeral && world_dir(name).join("journal.jsonl").is_file()
    }

    fn list(&self) -> Vec<String> {
        if self.ephemeral {
            return Vec::new();
        }
        let mut names: Vec<String> = std::fs::read_dir(WORLDS_DIR)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.path().join("journal.jsonl").is_file())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| crate::world_hub::validate_world_name(name).is_ok())
            .collect();
        names.sort();
        names
    }

    fn durable(&self) -> bool {
        !self.ephemeral
    }
}

#[cfg(feature = "world")]
impl ServeWorlds {
    async fn open_world(&self, name: &str) -> Result<crate::world_session::WorldService> {
        use crate::world::{WorldCore, WorldHandle, WorldLimits};
        use crate::world_session::WorldService;

        let limits = WorldLimits {
            checkpoint_every: self.checkpoint_every,
            grant_on_use: self.grant_on_use,
            server_caps: self.server_caps.clone(),
            ..WorldLimits::default()
        };
        let runtime = self.runtime;
        let make = {
            let admin_token = self.admin_token.clone();
            let blobs = self.blobs.clone();
            let mail = self.mail.clone();
            let source_only = self.source_only;
            let trusted_module_keys = self.trusted_module_keys.clone();
            move |handle| {
                let service = WorldService::new(handle, admin_token)
                    .with_blob_store(blobs)
                    .with_runtime_limits(runtime)
                    .with_source_only(source_only)
                    .with_module_trust_keys(trusted_module_keys);
                match &mail {
                    Some(sink) => service.with_mail(Arc::clone(sink)),
                    None => service,
                }
            }
        };
        let dir = world_dir(name);
        let namespace_file = (!self.ephemeral).then(|| dir.join("records.namespace"));
        let records =
            crate::world_records::RecordsStore::open(&self.docs, namespace_file.as_deref()).await?;
        info!(
            world = name,
            namespace = %records.namespace(),
            "world: records document ready"
        );
        info!(
            world = name,
            ephemeral = self.ephemeral,
            "serve startup: records ready, building world service (#190 breadcrumb)"
        );
        let service = if self.ephemeral {
            make(WorldHandle::spawn_with_device_manifest(
                WorldCore::new(name, limits)?,
                Box::<crate::world::EmptyWorldProgram>::default(),
                Some(WorldHandle::host_device_manifest()),
            ))
            .with_records_store(records)
        } else {
            let (service, report) =
                crate::world_store::open_world(&dir, name, limits, runtime, make).await?;
            status!(
                "world {name}: restored {} at sequence {} ({} input(s) replayed)",
                dir.display(),
                report.sequence,
                report.replayed_inputs
            );
            if let Some(error) = &report.program_error {
                status_err!("warning: world {name}: program not restored ({error}); reinstall it");
            }
            service.with_records_store(records)
        };
        info!(
            world = name,
            "serve startup: world service built (#190 breadcrumb)"
        );

        let service = if self.native {
            service.with_native_enabled()
        } else {
            service
        };
        let service = match self.compiler.clone() {
            Some(compiler) => service.with_compiler(compiler),
            None => service,
        };
        if name == self.default_world
            && let Some(path) = self.default_module.as_deref()
        {
            self.install_default_module(&service, path).await?;
        }
        info!(
            world = name,
            "serve startup: world open complete (#190 breadcrumb)"
        );
        Ok(service)
    }

    #[cfg(feature = "sandbox")]
    async fn install_default_module(
        &self,
        service: &crate::world_session::WorldService,
        path: &std::path::Path,
    ) -> Result<()> {
        let metadata = tokio::fs::metadata(path).await?;
        ensure!(
            usize::try_from(metadata.len())
                .is_ok_and(|len| len <= crate::world_session::MAX_WORLD_MODULE_BYTES),
            "world module exceeds the configured module size limit"
        );
        let wasm = tokio::fs::read(path).await?;
        let hash = crate::world_session::module_hash(&wasm);
        if service.active_module_hash().await.as_deref() == Some(hash.as_str()) {
            info!(module = %path.display(), "world: module already active");
        } else {
            let seed = rand::rng().random::<u64>();
            service
                .install_wasm(self.admin_token.as_deref(), wasm, seed, &hash)
                .await
                .map_err(|e| anyhow::anyhow!("install {}: {e:?}", path.display()))?;
            info!(module = %path.display(), seed, "world: installed module");
        }
        Ok(())
    }

    #[cfg(not(feature = "sandbox"))]
    #[allow(clippy::unused_async)]
    async fn install_default_module(
        &self,
        _service: &crate::world_session::WorldService,
        _path: &std::path::Path,
    ) -> Result<()> {
        anyhow::bail!("--world-module requires a build with the `sandbox` feature");
    }
}

/// The group log the serve warning watcher reads: the global
/// `--groups-log` option when given, else the default file name —
/// resolved with the group commands' semantics (#2778; serve used
/// to hardcode the default and ignore the option).
fn serve_groups_log_path(path: Option<PathBuf>) -> PathBuf {
    crate::commands::group::log_path(path)
}

/// Starts the serve process.
///
/// Initializes the Iroh endpoint, blob store, protocol handlers, and
/// (optionally) gossip-based peer discovery, then waits for incoming
/// connections until interrupted with Ctrl+C.
///
/// # Arguments
///
/// * `ephemeral` - If `true`, use in-memory storage (lost on exit)
/// * `no_relay` - If `true`, disable relay servers (direct connections only)
/// * `no_gossip` - If `true`, disable gossip/DHT peer discovery entirely
/// * `web` - If `true`, start the web interface (requires `web` feature)
/// * `port` - Port for the web interface (default 3000)
/// * `bootstrap` - Additional node IDs for manual peer bootstrapping
/// * `topic` - Custom gossip topic name (default: from `defaults.conf` or `DEFAULT_TOPIC`)
/// * `topic_secret` - Custom shared secret for topic access control
/// * `no_default_bootstrap` - If `true`, skip default bootstrap nodes from `defaults.conf`
/// * `no_default_topic` - If `true`, skip default topic/secret from `defaults.conf`
/// * `replace_defaults` - If `true`, use only `defaults.conf` values (skip hardcoded fallbacks)
///
/// # Behavior
///
/// 1. Loads or creates the node keypair
/// 2. Opens the blob store (persistent or ephemeral)
/// 3. Creates the Iroh endpoint with DNS/Pkarr address lookup
/// 4. Creates peer discovery table
/// 5. Unless `no_gossip`: creates gossip instance, registers gossip ALPN,
///    joins topic via DHT, spawns announce/receive/cleanup tasks
/// 6. Registers `MetaProtocol` and `BlobsProtocol` handlers
/// 7. Optionally starts the web interface on the specified port
/// 8. Creates the lock file for local process discovery
/// 9. Waits for Ctrl+C
/// 10. Cleans up and exits
///
/// # Output
///
/// Prints the node ID, mode, and peer discovery status to stdout.
/// Status messages go to stderr.
#[allow(unused_variables)] // web/port only used with web feature
/// The loaded operator server config and the provisioning it ran
/// (#5119 slice 1). Kept at startup scope so the later slices — the
/// pull gate from `config.pull_policy()` and the freshness window in
/// seitan admission — wire into live serving from this one load.
#[cfg(feature = "world")]
#[derive(Debug)]
pub struct StartedServerConfig {
    /// The parsed config.
    pub config: crate::server_config::ServerConfig,
    /// The accounts and group events provisioning applied.
    pub provisioned: crate::server_config::Provisioned,
    /// What the provisioning run created.
    pub report: crate::server_config::Report,
}

/// Load the operator server config at `path` and run startup
/// provisioning from it (#5119 slice 1).
///
/// # Errors
///
/// Fails when the file cannot be read or parses badly — the
/// [`crate::server_config::ServerConfig::parse`] error is named, per
/// that parser's startup-failing contract — or when provisioning is
/// refused (for example a group creator the create rule does not
/// allow).
#[cfg(feature = "world")]
pub fn start_server_config(path: &Path) -> Result<StartedServerConfig> {
    let config = crate::server_config::ServerConfig::load(path)
        .map_err(|error| anyhow::anyhow!("server config {}: {error}", path.display()))?;
    let mut provisioned = crate::server_config::Provisioned::new();
    let report = provisioned
        .apply(&config)
        .map_err(|error| anyhow::anyhow!("server config {}: {error}", path.display()))?;
    Ok(StartedServerConfig {
        config,
        provisioned,
        report,
    })
}

/// The directory journal the server config provisioning persists
/// its accounts in (#5119 slice 3): `directory.jsonl` beside the
/// groups log, the same layout a world directory uses.
#[cfg(feature = "world")]
fn server_config_directory_path(groups_log: &Path) -> PathBuf {
    groups_log.with_file_name("directory.jsonl")
}

/// Load the operator server config at `path` and run startup
/// provisioning into the live serving state (#5119 slice 3): the
/// provisioned accounts go into the journaled directory beside the
/// groups log and the preformed group events into the groups log
/// itself — the same state live serving folds and replays — so a
/// restart loads them back, provisioning skips them, and the report
/// names no new creations.
///
/// # Errors
///
/// Fails on the same startup contract as [`start_server_config`],
/// and when the persisted directory or groups log cannot be opened
/// or written.
#[cfg(feature = "world")]
pub fn start_server_config_persisted(
    path: &Path,
    groups_log: &Path,
) -> Result<StartedServerConfig> {
    let config = crate::server_config::ServerConfig::load(path)
        .map_err(|error| anyhow::anyhow!("server config {}: {error}", path.display()))?;
    let directory = crate::directory::Directory::open(&server_config_directory_path(groups_log))?;
    let mut log = crate::groups::log::GroupLog::open(groups_log)?;
    let already = log.events().len();
    let mut provisioned =
        crate::server_config::Provisioned::from_parts(directory, log.events().to_vec());
    let report = provisioned
        .apply(&config)
        .map_err(|error| anyhow::anyhow!("server config {}: {error}", path.display()))?;
    // Seed only the events this start added: the rest are the
    // from_parts state itself, and re-checking them on every
    // restart would rescan the whole log history.
    for event in &provisioned.events()[already..] {
        log.record_preformed(event)?;
    }
    Ok(StartedServerConfig {
        config,
        provisioned,
        report,
    })
}

/// Seed the serving world's own directory with the accounts the
/// server config provisions (#5119, PR #8356 review): slice 3
/// persists provisioning into the directory beside the groups log,
/// but a durable world loads `.id-worlds/<name>/directory.jsonl`
/// ([`crate::world_store::open_world`]), so without this seed the
/// live world's directory view treats configured keys as anonymous
/// while startup reports them provisioned. The caller names the
/// default world, the one serve opens at startup. Seeding is the
/// same idempotent account application provisioning itself runs,
/// so a restart creates nothing. Group events are not seeded here:
/// they live in the groups log, which live serving already folds.
///
/// # Errors
///
/// Fails when the world directory cannot be created or opened, or
/// when the directory refuses a configured user -- the same
/// startup-failing contract as the provisioning it mirrors.
#[cfg(feature = "world")]
pub fn seed_world_directory(
    config: &crate::server_config::ServerConfig,
    dir: &Path,
) -> Result<crate::server_config::Report> {
    std::fs::create_dir_all(dir)?;
    let directory = crate::directory::Directory::open(&dir.join("directory.jsonl"))?;
    let mut provisioned = crate::server_config::Provisioned::from_parts(directory, Vec::new());
    provisioned.apply_users(config).map_err(|error| {
        anyhow::anyhow!("server config world directory {}: {error}", dir.display())
    })
}

/// The pull policy the live event-sync gate serves under (#5119): the server
/// config's `[policy] pull_policy` when a config was loaded, else the code
/// default ([`crate::pull_policy::PullPolicy::Open`]), so a `serve` without
/// `--server-config` behaves exactly as before.
#[cfg(feature = "world")]
fn serve_pull_policy(started: Option<&StartedServerConfig>) -> crate::pull_policy::PullPolicy {
    started.map_or(crate::pull_policy::PullPolicy::Open, |started| {
        started.config.pull_policy()
    })
}

/// The freshness window seitan admission serves under (#8487): the loaded
/// server config's `[policy] freshness_window_seconds`, resolved through
/// [`crate::server_config::ServerConfig::freshness`], when a config was
/// loaded, else [`crate::seitan::DEFAULT_WINDOW`] — so a `serve` without
/// `--server-config` behaves exactly as before.
#[cfg(feature = "world")]
fn serve_freshness_window(started: Option<&StartedServerConfig>) -> std::time::Duration {
    started.map_or(crate::seitan::DEFAULT_WINDOW, |started| {
        started.config.freshness().window()
    })
}

/// The world protocol serve registers for a hub: the loaded operator
/// config's #2247 budget policy when a server config was given, the
/// coded defaults when none was. Before this wire, the budgets
/// table was parsed and held at startup scope but never reached the
/// protocol, so configured per-counterparty caps were inert in
/// production.
#[cfg(feature = "world")]
fn world_protocol_for(
    hub: crate::world_hub::WorldHub,
    local_node: [u8; 32],
    started: Option<&StartedServerConfig>,
) -> crate::world_net::WorldProtocol {
    match started {
        Some(started) => crate::world_net::WorldProtocol::with_budgets(
            hub,
            local_node,
            started.config.budgets().clone(),
        ),
        None => crate::world_net::WorldProtocol::with_local_node(hub, local_node),
    }
}

pub async fn cmd_serve(opts: ServeOptions) -> Result<()> {
    let ServeOptions {
        ephemeral,
        server_config,
        groups_log,
        no_relay,
        no_gossip,
        web,
        port,
        bootstrap,
        topic,
        topic_secret,
        no_default_bootstrap,
        no_default_topic,
        replace_defaults,
        no_mdns,
        dht,
        iroh_port,
        bind,
        web_listen,
        bind_explicit,
        port_explicit,
        web_token,
        web_cookie_secure,
        world,
        world_admin_token,
        world_module,
        world_mail_outbox,
        world_mail_command,
        world_mail_smtp,
        world_mail_from,
        world_name,
        world_checkpoint_every,
        world_max_open,
        world_max_sessions,
        world_idle_secs,
        world_boot_window_secs,
        world_caps,
        world_cap_policy,
        world_allowed_caps,
        roc_bin,
        roc_platform,
        world_native,
        world_source_only,
        world_module_trust_keys,
        world_ssh_port,
        world_runtime,
        allow_node,
        open_writes,
    } = opts;
    // Resolved early (not at the group-warning block below) so the gossip
    // receive task can fold the same log for receipt-side membership scoping
    // (#4363): a channel the receiver holds no fold for scopes to empty.
    let groups_log = serve_groups_log_path(groups_log);
    // A web request on a binary built without the `web` feature must
    // fail loudly: the web startup block is compiled out in that build,
    // so without this guard --web was accepted and silently served
    // nothing (no listener bound, no `web:` status line) - the failure
    // mode that left web-waiting integration tests to die on a startup
    // timeout instead of hearing the real reason.
    #[cfg(not(feature = "web"))]
    if web || !web_listen.is_empty() || port_explicit || bind_explicit {
        anyhow::bail!(
            "the web interface was requested (--web/--web-listen/--port/--bind), \
             but this id binary was built without the `web` feature; \
             rebuild with `--features web` to serve it"
        );
    }
    validate_world_options(
        world,
        world_admin_token.as_deref(),
        world_module.as_ref(),
        &world_name,
        &world_cap_policy,
        world_source_only,
    )?;
    // #5119 slice 1: load the operator server config before any
    // startup work, so a malformed file fails fast naming the parse
    // error, and run startup provisioning from it. No flag = no file
    // read and exactly today's behaviour. The loaded state stays in
    // scope for the pull-gate and freshness slices that consume it.
    #[cfg(not(feature = "world"))]
    if server_config.is_some() {
        anyhow::bail!(
            "--server-config was given, but this id binary was built without \
             the `world` feature, which owns the server config; \
             rebuild with `--features world` to serve with one"
        );
    }
    #[cfg(feature = "world")]
    let started_server_config = match &server_config {
        Some(path) => {
            let started = start_server_config_persisted(path, &groups_log)?;
            status!(
                "server config: loaded {} (provisioned {} user(s), {} group(s), {} member(s))",
                path.display(),
                started.report.users_created,
                started.report.groups_created,
                started.report.members_added
            );
            Some(started)
        }
        None => None,
    };
    // #5119 (PR #8356 review): a durable world's live directory is
    // its own `.id-worlds/<name>/directory.jsonl`, not the directory
    // beside the groups log that slice 3 persists into, so seed the
    // default world's directory with the configured accounts too;
    // otherwise the live directory view treats them as anonymous.
    // Ephemeral worlds hold no directory file to seed: their state
    // is in-memory and ends with the process, like the world itself.
    #[cfg(feature = "world")]
    if let Some(started) = &started_server_config {
        if world && !ephemeral {
            let seeded = seed_world_directory(&started.config, &world_dir(&world_name))?;
            if seeded.users_created > 0 {
                status!(
                    "server config: seeded {} configured user(s) into world {world_name}'s directory",
                    seeded.users_created
                );
            }
        }
    }
    // #5119: the pull policy the live event-sync gate uses, resolved once from
    // the same startup config load. No config keeps the code default (open).
    #[cfg(feature = "web")]
    let event_sync_policy = {
        #[cfg(feature = "world")]
        {
            serve_pull_policy(started_server_config.as_ref())
        }
        #[cfg(not(feature = "world"))]
        {
            crate::pull_policy::PullPolicy::Open
        }
    };
    // #2145: resolve the web listener set once, before any startup work,
    // so a flag conflict or a malformed [web_bindings] line fails fast.
    let web_bindings = resolve_web_bindings(
        &web_listen,
        bind_explicit,
        port_explicit,
        bind,
        port,
        &defaults().web_bindings,
    )?;
    // #2147 (slice 1): restore the persisted publication set at boot.
    // The registry is the persisted half of dynamic publication: the
    // set an operator published in an earlier run is read back here,
    // named line by line so the restored set is inspectable in the
    // startup log, and the boot shadow warnings the issue requires
    // are re-issued for any persisted mount that shadows an Explorer
    // route without an acknowledgement. Applying the set to live
    // listeners is the next slice; a malformed registry fails startup
    // loudly (in Registry::load) rather than restoring nothing
    // silently.
    let publications =
        crate::publications::Registry::load(&crate::publications::publications_file())?;
    if !publications.publications.is_empty() {
        status!(
            "publications: restoring {} persisted publication(s)",
            publications.publications.len()
        );
        for publication in &publications.publications {
            status!(
                "publications: world {} at {}",
                publication.world,
                publication.placement.describe()
            );
        }
        for warning in publications.boot_warnings(crate::publications::KNOWN_EXPLORER_ROUTES) {
            status!("{warning}");
        }
    }
    let apps_root = std::env::var_os("ID_APPS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("apps"));
    let app_discovery = crate::app_manifest::discover_apps(&apps_root);
    for app in app_discovery.apps {
        info!(app = %app.manifest.app.id, path = %app.path.display(), "serve: discovered app");
    }
    for error in app_discovery.errors {
        warn!(%error, "serve: app discovery failed");
    }
    #[cfg(not(feature = "world"))]
    let _ = (&world_allowed_caps, &world_module_trust_keys);
    let key = load_or_create_keypair(KEY_FILE).await?;
    let node_id: EndpointId = key.public();
    info!("serve: {}", node_id);

    // Who may modify this store: this node, this machine's client key (so the
    // local CLI/REPL work with no setup), `--allow-node`, and `.iroh-allowed`.
    let access = build_access_policy(node_id, &allow_node, open_writes).await?;

    let store = open_store(ephemeral).await?;
    let store_handle = store.as_store();

    let mut builder = crate::connectivity::public_builder(
        crate::connectivity::RelayPolicy::from_no_relay(no_relay),
    )
    .secret_key(key.clone());
    if !no_mdns {
        builder = builder.address_lookup(MdnsAddressLookup::builder());
    }
    let dht_policy = crate::dht::DhtPolicy::from_server(dht);
    #[cfg(feature = "dht")]
    if dht_policy.is_on() {
        builder = crate::dht::with_lookup(&dht_policy, builder, crate::dht::mainline());
    }
    #[cfg(not(feature = "dht"))]
    if dht_policy.is_on() {
        warn!(
            "--dht was requested, but `id` was compiled without the optional `dht` feature; DHT lookup is not active"
        );
    }
    if iroh_port != 0 {
        builder = builder.bind_addr(std::net::SocketAddrV4::new(
            Ipv4Addr::UNSPECIFIED,
            iroh_port,
        ))?;
    }
    let endpoint = builder.bind().await?;

    // Do not announce readiness before the endpoint is reachable through
    // its home relay (#3028). The wait is bounded and non-fatal: a dead
    // relay entry must not hold startup, so on timeout we warn and carry on.
    // With `--no-relay` no relay ever connects, so the wait is skipped.
    if no_relay {
        info!("serve startup: relays disabled (--no-relay), skipping home-relay wait");
    } else {
        match crate::connectivity::await_home_relay(
            &endpoint,
            crate::connectivity::HOME_RELAY_DEADLINE,
        )
        .await
        {
            Ok(()) => info!("serve startup: home relay connected"),
            Err(e) => warn!(
                "serve startup: proceeding without a home relay after {:?}: {e:#}",
                crate::connectivity::HOME_RELAY_DEADLINE,
            ),
        }
    }

    // Create peer discovery table
    let peer_discovery = PeerDiscovery::new();
    let chat_presence = PresenceTable::new();

    // Always create Gossip — iroh-docs needs it even if peer discovery is off
    let gossip = Gossip::builder().spawn(endpoint.clone());

    // Initialize iroh-docs for tag metadata storage
    let docs = if ephemeral {
        Docs::memory()
            .spawn(endpoint.clone(), store_handle.clone(), gossip.clone())
            .await?
    } else {
        let docs_path = PathBuf::from(STORE_PATH).join("docs");
        std::fs::create_dir_all(&docs_path)?;
        Docs::persistent(docs_path)
            .spawn(endpoint.clone(), store_handle.clone(), gossip.clone())
            .await?
    };

    // Initialize TagStore (creates α/Ω namespace pairs)
    let tag_store = TagStore::init(&docs, &node_id.to_string()).await?;
    let tag_store = Arc::new(tag_store);
    info!("tags: initialized (α/Ω global + node namespaces)");
    match tag_store.migrate_legacy_meta(&store_handle).await {
        Ok(0) => {}
        Ok(n) => info!("tags: imported {n} legacy metadata tag(s) from .meta"),
        Err(e) => warn!("tags: legacy metadata import failed: {e:#}"),
    }

    // Build router — gossip ALPN is always registered (needed by iroh-docs),
    // but peer discovery gossip topic only joins when gossip is enabled
    // iroh-blobs rejects pushes unless an event handler enables them; this one
    // allows them only from nodes the access policy lets write.
    let blobs = BlobsProtocol::new(
        &store_handle,
        Some(crate::access::blobs_events(access.clone())),
    );

    // The node's own event store, shared between the web layer and /id-range-sync/1
    // range sync (#792). Serve opens it once, on the same directory the web
    // state would open (the `events` sibling of the identity database), and
    // hands the one instance to both: the store admits a single opener, and a
    // second store over the same directory would diverge from the one the web
    // layer publishes ProseMirror save revisions into. Only opened when the
    // web UI is on — that is the store whose revisions replicate, and opening
    // it unconditionally would change the local CLI's access to the directory
    // while serve runs.
    #[cfg(feature = "web")]
    let event_events: Option<Arc<tokio::sync::Mutex<crate::local_events::LocalEvents>>> = if web {
        let identity_db_path = std::path::Path::new(KEY_FILE).with_file_name(".identity.redb");
        let events_dir = identity_db_path.with_file_name(crate::local_events::DIR_NAME);
        let events = crate::local_events::LocalEvents::open(
            &events_dir,
            p2panda_core::SigningKey::from_bytes(&key.to_bytes()),
        )?;
        Some(Arc::new(tokio::sync::Mutex::new(events)))
    } else {
        None
    };

    // Every world lives in one hub, shared by the Iroh protocol and the web
    // bridge. The default world opens now so a bad journal fails startup.
    #[cfg(feature = "world")]
    let world_hub = if world {
        use crate::world_hub::{HubLimits, WorldHub};
        let runtime = crate::world_limits::RuntimeLimits::from_args(&world_runtime)?;
        let server_caps = allowed_caps(world_allowed_caps.as_deref())?;
        let compiler = resolve_compiler(roc_bin, roc_platform)?;
        let trusted_module_keys = world_module_trust_keys
            .iter()
            .map(|encoded| {
                let bytes = crate::world_session::decode_hex(encoded).ok_or_else(|| {
                    anyhow::anyhow!("--world-module-trust-key must be 32-byte hex")
                })?;
                <[u8; 32]>::try_from(bytes.as_slice())
                    .map_err(|_| anyhow::anyhow!("--world-module-trust-key must be 32-byte hex"))
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            trusted_module_keys.is_empty() || world_module.is_none(),
            "--world-module-trust-key cannot be combined with --world-module: \
             startup modules cannot carry a signature yet — remove one of the \
             two, or install the module over the session protocol with a signature"
        );
        ensure!(
            !world_source_only || compiler.is_some(),
            "--world-source-only requires a configured Roc compiler and platform (see --roc-bin and --roc-platform)"
        );
        let opener = Arc::new(ServeWorlds {
            ephemeral,
            admin_token: world_admin_token.clone(),
            blobs: store_handle.clone(),
            docs: docs.clone(),
            checkpoint_every: world_checkpoint_every,
            default_world: world_name.clone(),
            default_module: world_module.clone(),
            grant_on_use: world_cap_policy == "grant-on-use",
            server_caps: server_caps.clone(),
            compiler,
            native: world_native,
            source_only: world_source_only,
            trusted_module_keys,
            runtime,
            mail: mail_sink(
                world_mail_outbox,
                world_mail_command,
                world_mail_smtp,
                world_mail_from,
            )?,
        });
        let hub = WorldHub::new(
            opener,
            world_admin_token.clone(),
            world_name.clone(),
            HubLimits {
                max_open_worlds: world_max_open,
                max_sessions: world_max_sessions,
                signed_boot_window: std::time::Duration::from_secs(world_boot_window_secs),
            },
        );
        info!("serve startup: opening default world via hub (#190 breadcrumb)");
        let default_service = hub.open_default().await?;
        info!("serve startup: default world open, building router (#190 breadcrumb)");
        if !world_caps.is_empty() {
            for cap in world_caps.iter().filter(|cap| *cap != "*") {
                if !server_caps.contains(cap) {
                    warn!(
                        "world: --world-cap {cap} is outside --world-allowed-caps; the program will be refused it"
                    );
                }
            }
            default_service
                .world()
                .update_caps(world_caps.clone(), Vec::new())
                .await?;
        }
        if world_idle_secs > 0 && !ephemeral {
            let idle = std::time::Duration::from_secs(world_idle_secs);
            hub.spawn_evictor(
                (idle / 2).clamp(
                    std::time::Duration::from_secs(1),
                    std::time::Duration::from_secs(60),
                ),
                idle,
            );
        }
        Some(hub)
    } else {
        None
    };
    #[cfg(feature = "world")]
    let outbox_flush = match &world_hub {
        Some(hub) if !ephemeral => Some(hub.spawn_outbox_flush(
            std::time::Duration::from_secs(OUTBOX_FLUSH_SECS),
            endpoint.clone(),
        )),
        _ => None,
    };
    #[cfg(not(feature = "world"))]
    ensure!(!world, "this build has no world support (feature `world`)");

    // The world protocol instance is shared between its ALPN registration and
    // the meta protocol, which exposes its abuse counters (#4918).
    #[cfg(feature = "world")]
    let world_protocol = world_hub.as_ref().map(|hub| {
        world_protocol_for(
            hub.clone(),
            *node_id.as_bytes(),
            started_server_config.as_ref(),
        )
    });

    #[cfg(feature = "world")]
    let abuse_handle = world_protocol
        .as_ref()
        .map(crate::world_net::WorldProtocol::abuse_handle);
    #[cfg(not(feature = "world"))]
    let abuse_handle: Option<std::sync::Arc<std::sync::Mutex<crate::abuse::AbuseEvaluator>>> = None;

    let meta = MetaProtocol::new(
        &store_handle,
        Some(peer_discovery.clone()),
        Arc::clone(&tag_store),
        access.clone(),
        node_id,
        abuse_handle,
    );

    #[cfg(all(feature = "world", feature = "ssh"))]
    let (ssh_tunnel, ssh_tunnel_port) = crate::world_ssh::SshTunnelProtocol::deferred();
    let router_builder = Router::builder(endpoint)
        .accept(META_ALPN, meta)
        .accept(BLOBS_ALPN, blobs)
        .accept(iroh_gossip::net::GOSSIP_ALPN, gossip.clone())
        .accept(iroh_docs::net::ALPN, docs.clone());
    #[cfg(feature = "web")]
    let router_builder = match &event_events {
        Some(events) => router_builder.accept(
            crate::event_store::sync::ALPN,
            crate::event_sync::EventSyncProtocol::with_gate(
                Arc::clone(events),
                crate::event_sync::gate(event_sync_policy),
            ),
        ),
        None => router_builder,
    };
    #[cfg(feature = "world")]
    let router_builder = match (world_hub.as_ref(), world_protocol) {
        (Some(hub), Some(world_protocol)) => router_builder
            .accept(crate::world_net::WORLD_ALPN, world_protocol)
            .accept(
                crate::world_replication_net::REPLICATION_ALPN,
                crate::world_replication_net::WorldReplicationProtocol::new(
                    hub.clone(),
                    ed25519_dalek::SigningKey::from_bytes(&key.to_bytes()),
                ),
            )
            .accept(
                crate::envelope_net::ENVELOPE_ALPN,
                crate::envelope_net::EnvelopeProtocol::new(hub.clone()),
            )
            .accept(
                crate::artifact::PUSH_ALPN,
                crate::artifact::ArtifactPushProtocol::new(hub.clone()),
            )
            .accept(
                crate::artifact::PULL_ALPN,
                crate::artifact::ArtifactPullProtocol::new(hub.clone()),
            ),
        _ => router_builder,
    };
    #[cfg(all(feature = "world", feature = "ssh"))]
    let router_builder = router_builder.accept(crate::world_ssh::SSH_TUNNEL_ALPN, ssh_tunnel);
    let router = router_builder.spawn();
    info!("serve startup: router spawned (#190 breadcrumb)");

    // Offer a sync session to every discovered peer, on an interval: a save on
    // either node is discoverable on the other after the next session (#792).
    // Peers that do not serve /id-range-sync/1 fail the dial, which is logged at debug
    // and otherwise ignored — accepting sessions above is what serves the rest.
    #[cfg(feature = "web")]
    let _event_sync_handle = event_events.clone().map(|events| {
        status!("event sync: /id-range-sync/1 enabled (the web event store)");
        // #5119: the live pull gate reads the operator config's
        // `[policy] pull_policy`. Without a config the policy is the code
        // default (open), exactly as before. The local event store is not a
        // world and has no membership source, so `members` fails closed —
        // every pull is refused — and says so here rather than silently
        // serving open.
        let policy = event_sync_policy;
        if policy == crate::pull_policy::PullPolicy::Members {
            status!(
                "event sync: [policy] pull_policy=members — the local event store has no \
                 membership source, so every pull is refused (fail closed)"
            );
        }
        let endpoint = router.endpoint().clone();
        let discovery = peer_discovery.clone();
        let gate = crate::event_sync::gate(policy);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(crate::event_sync::SYNC_INTERVAL).await;
                for peer in discovery.peers() {
                    let peer_id = peer.announcement.node_id;
                    if peer_id == endpoint.id() {
                        continue;
                    }
                    match crate::event_sync::sync_with_peer(&endpoint, peer_id, &events, &gate)
                        .await
                    {
                        Ok(outcome) if outcome.sent + outcome.received > 0 => {
                            info!(
                                "event sync: {peer_id}: sent {} received {}",
                                outcome.sent, outcome.received
                            );
                        }
                        Ok(_) => {}
                        Err(error) => {
                            debug!("event sync: session with {peer_id} failed: {error:#}");
                        }
                    }
                }
            }
        })
    });

    if !no_gossip {
        // Resolve effective config from defaults + CLI flags
        let config = resolve_config(
            &bootstrap,
            topic.as_deref(),
            topic_secret.as_deref(),
            replace_defaults,
            no_default_bootstrap,
            no_default_topic,
        )?;

        let dtt_topic_id = TopicId::new(config.topic.clone());

        // Convert iroh SecretKey to ed25519-dalek types for RecordPublisher
        let dalek_signing_key = ed25519_dalek::SigningKey::from_bytes(&key.to_bytes());

        let record_publisher = RecordPublisher::new(
            dtt_topic_id,
            dalek_signing_key,
            None,
            config.topic_secret.clone(),
            distributed_topic_tracker::Config::default(),
        );

        // Join gossip topic with auto-discovery (non-blocking)
        let gossip_topic = gossip
            .subscribe_and_join_with_auto_discovery_no_wait(record_publisher)
            .await?;
        let (sender, receiver) = gossip_topic.split().await?;

        // Join bootstrap peers (both defaults and CLI-provided, already merged)
        let bootstrap_node_ids: Vec<EndpointId> = config
            .bootstrap
            .iter()
            .filter_map(|id_str| {
                id_str.parse::<EndpointId>().ok().or_else(|| {
                    warn!("invalid bootstrap node ID: {}", id_str);
                    None
                })
            })
            .collect();
        if !bootstrap_node_ids.is_empty() {
            info!("joining {} bootstrap peer(s)", bootstrap_node_ids.len());
            sender.join_peers(bootstrap_node_ids, None).await?;
        }

        // Spawn background gossip task
        let gossip_peer_discovery = peer_discovery.clone();
        let gossip_chat_presence = chat_presence.clone();
        let gossip_groups_log = groups_log.clone();
        let gossip_signing_key = ed25519_dalek::SigningKey::from_bytes(&key.to_bytes());
        let gossip_store = store_handle.clone();
        let gossip_node_id = node_id;
        let gossip_endpoint = router.endpoint().clone();
        let _gossip_handle = tokio::spawn(async move {
            run_gossip_loop(
                gossip_node_id,
                sender,
                receiver,
                gossip_peer_discovery,
                gossip_chat_presence,
                gossip_groups_log,
                gossip_signing_key,
                gossip_store,
                gossip_endpoint,
            )
            .await;
        });

        status!("peers: gossip enabled (topic: {})", config.topic);
    }

    let serve_node_id = router.endpoint().id();
    let bound_addrs = router.endpoint().bound_sockets();
    let local_addrs: Vec<SocketAddr> = bound_addrs
        .iter()
        .map(|addr| match addr {
            SocketAddr::V4(v4) if v4.ip().is_unspecified() => {
                SocketAddr::new(Ipv4Addr::LOCALHOST.into(), v4.port())
            }
            SocketAddr::V6(v6) if v6.ip().is_unspecified() => {
                SocketAddr::new(Ipv6Addr::LOCALHOST.into(), v6.port())
            }
            other => *other,
        })
        .collect();

    // Bind the web listeners early to capture the actual ports for the
    // lock file, but don't start serving yet (spawn happens after the
    // lock file is written). #2145: ALL resolved listeners bind here;
    // any bind failure aborts startup naming the failing listener -- no
    // partial start.
    #[allow(unused_mut)] // web_port is only mutated with the `web` feature
    let mut web_port: Option<u16> = None;
    #[allow(unused_mut)] // web_binding_addrs is only mutated with the `web` feature
    let mut web_binding_addrs: Vec<String> = Vec::new();
    #[cfg(feature = "web")]
    let web_listeners: Vec<(SocketAddr, tokio::net::TcpListener, WebContentTarget)> = if web {
        info!("serve startup: binding web listener(s) (#190 breadcrumb)");
        let mut listeners = Vec::new();
        for binding in &web_bindings {
            let addr = binding.socket_addr();
            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .map_err(|e| anyhow::anyhow!("web: failed to bind listener {addr}: {e}"))?;
            let actual_port = listener.local_addr()?.port();
            let actual_addr = binding.with_port(actual_port);
            info!(
                addr = %actual_addr,
                target = %binding.target().describe(),
                "serve startup: web listener bound (#190 breadcrumb)"
            );
            if web_port.is_none() {
                web_port = Some(actual_port);
            }
            web_binding_addrs.push(actual_addr.to_string());
            listeners.push((actual_addr, listener, binding.target().clone()));
        }
        listeners
    } else {
        Vec::new()
    };

    #[cfg(feature = "ssh")]
    let ssh = match (world_ssh_port, &world_hub) {
        (Some(port), Some(hub)) => {
            let listener = tokio::net::TcpListener::bind(SocketAddr::new(bind, port)).await?;
            let ssh_port = listener.local_addr()?.port();
            #[cfg(feature = "world")]
            let _ = ssh_tunnel_port.set(ssh_port);
            let host_key =
                crate::world_ssh::load_or_create_host_key(crate::world_ssh::SSH_HOST_KEY_FILE)
                    .await?;
            Some((
                listener,
                ssh_port,
                hub.clone(),
                host_key,
                store_handle.clone(),
            ))
        }
        _ => None,
    };
    #[cfg(not(feature = "ssh"))]
    ensure!(
        world_ssh_port.is_none(),
        "this build has no SSH support (feature `ssh`)"
    );

    // Write the lock file before printing status so it exists when callers
    // detect the server via stdout output (integration tests depend on this).
    info!("serve startup: writing serve lock (#190 breadcrumb)");
    create_serve_lock(&serve_node_id, &local_addrs, web_port, web_binding_addrs).await?;
    info!("serve startup: serve lock written, printing node status (#190 breadcrumb)");

    status!("node: {serve_node_id}");
    if ephemeral {
        status!("mode: ephemeral (in-memory)");
    } else {
        status!("mode: persistent ({STORE_PATH})");
    }
    if no_relay {
        status!("relay: disabled");
    }
    if no_gossip {
        status!("peers: disabled");
    }
    if no_mdns {
        status!("mdns: disabled");
    } else {
        status!("mdns: enabled");
    }
    if access.is_open() {
        status!("access: OPEN WRITES (any peer may modify this store)");
    } else {
        status!(
            "access: read-only for peers; {} node(s) may write",
            access.writers().len()
        );
    }
    #[cfg(feature = "world")]
    if world {
        status!(
            "world {world_name}: enabled (iroh {})",
            String::from_utf8_lossy(crate::world_net::WORLD_ALPN)
        );
        #[cfg(feature = "web")]
        if web {
            status!("world: web bridge at /ws/world");
        }
    }

    // Start the web servers now that the lock file is written: one
    // serve task per bound listener (#2145), each listener serving
    // the router for its #2146 content target (Explorer by default,
    // a program-only surface for a `Program` binding) with its own
    // per-bind security.
    #[cfg(feature = "web")]
    let _web_handles = {
        let mut handles = Vec::new();
        if !web_listeners.is_empty() {
            status!("web: {} listener(s)", web_listeners.len());
            // One shared AppState for all listeners (the identity store
            // opens exclusively); each listener's router is finished with
            // its own per-bind security. Do not initialize web persistence
            // when serving without the web interface.
            let securities: Vec<crate::web::WebSecurity> = web_listeners
                .iter()
                .map(|(listener_addr, _, _)| {
                    crate::web::WebSecurity::for_bind(listener_addr.ip(), web_token.clone(), &[])
                })
                .collect();
            // Each listener's #2146 content target, parallel to
            // `securities`: a Program binding gets the program-only
            // router for its world (phase 2); Explorer is the legacy
            // router.
            let surfaces: Vec<Option<String>> = web_listeners
                .iter()
                .map(|(_, _, target)| match target {
                    WebContentTarget::Explorer => None,
                    WebContentTarget::Program(world) => Some(world.clone()),
                })
                .collect();
            // #8487: the seitan admission state the web surface
            // serves: the freshness window resolved from the loaded
            // server config (the code default without one), and the
            // server-config directory journal beside the groups log
            // opened after provisioning persisted into it, so the
            // admit path's account check sees the provisioned
            // accounts and its `AccountVerified` appends land in the
            // same journal.
            #[cfg(feature = "world")]
            let seitan_state = crate::web::SeitanState::new(
                crate::directory::Directory::open(&server_config_directory_path(&groups_log))?,
                serve_freshness_window(started_server_config.as_ref()),
            );
            #[cfg(not(feature = "world"))]
            let seitan_state = crate::web::SeitanState::default();
            let web_routers = crate::web::web_routers(
                store_handle.clone(),
                Some(peer_discovery.clone()),
                node_id.to_string(),
                Arc::clone(&tag_store),
                key.to_bytes(),
                std::path::Path::new(KEY_FILE).with_file_name(".identity.redb"),
                securities,
                world_hub.clone(),
                web_cookie_secure,
                event_events.as_ref().map(Arc::clone),
                surfaces,
                Some(groups_log.clone()),
                seitan_state,
            )
            .await?;
            for ((listener_addr, listener, target), web_router) in
                web_listeners.into_iter().zip(web_routers)
            {
                let listener_ip = listener_addr.ip();
                let shown_host = match listener_addr {
                    SocketAddr::V4(addr)
                        if addr.ip().is_unspecified() || addr.ip().is_loopback() =>
                    {
                        "localhost".to_owned()
                    }
                    SocketAddr::V4(addr) => addr.ip().to_string(),
                    SocketAddr::V6(addr)
                        if addr.ip().is_unspecified() || addr.ip().is_loopback() =>
                    {
                        "localhost".to_owned()
                    }
                    SocketAddr::V6(addr) if addr.scope_id() == 0 => {
                        format!("[{}]", addr.ip())
                    }
                    SocketAddr::V6(addr) => {
                        format!("[{}%25{}]", addr.ip(), addr.scope_id())
                    }
                };
                if web_token.is_some() {
                    status!(
                        "web: http://{shown_host}:{}/?token=<redacted>",
                        listener_addr.port()
                    );
                } else {
                    status!("web: http://{shown_host}:{}", listener_addr.port());
                }
                #[cfg(feature = "world")]
                if world {
                    match &target {
                        WebContentTarget::Explorer => {
                            status!(
                                "world: browser http://{shown_host}:{}/world",
                                listener_addr.port()
                            );
                        }
                        WebContentTarget::Program(program_world) => {
                            status!(
                                "program {program_world}: browser http://{shown_host}:{}/",
                                listener_addr.port()
                            );
                        }
                    }
                }
                if !listener_ip.is_loopback() && web_token.is_none() {
                    status_err!(
                        "warning: the web UI is bound to {listener_addr} without --web-token; \
                         anyone who can reach this listener can read and modify files"
                    );
                }
                if world_hub.is_some() && !listener_ip.is_loopback() && !web_cookie_secure {
                    status_err!(
                        "warning: the admin sign-in form is served over plain HTTP on {listener_addr}; \
                         its token and session cookie travel in clear unless HTTPS is terminated \
                         in front (then pass --web-cookie-secure) or the UI is bound to loopback"
                    );
                }
                handles.push(tokio::spawn(async move {
                    if let Err(e) = axum::serve(listener, web_router).await {
                        tracing::error!("web server error: {}", e);
                    }
                }));
            }
        }
        handles
    };

    #[cfg(feature = "ssh")]
    let ssh_handle = ssh.map(|(listener, ssh_port, hub, host_key, blobs)| {
        status!("world ssh: port {ssh_port} (ssh -p {ssh_port} <world>@<host>)");
        status!(
            "world ssh host key: {}",
            crate::world_ssh::host_key_fingerprint(&host_key)
        );
        status!(
            "world ssh over iroh: `id world ssh-tunnel <node-id>` then `ssh -p 2222 <world>@127.0.0.1`"
        );
        tokio::spawn(async move {
            if let Err(e) = crate::world_ssh::serve(listener, hub, host_key, blobs).await {
                tracing::error!("world ssh server error: {}", e);
            }
        })
    });

    // Group warnings (P4.9, #799): one persistent Evaluator for the life of
    // the process. The first pass runs here so active warnings surface in
    // the CLI status output at startup; the spawned task then re-evaluates
    // as membership changes and writes fired warnings to the server log.
    let mut group_watcher = GroupWarningWatcher::new();
    for (_, warning) in evaluate_group_warnings(&mut group_watcher, &groups_log, unix_now()) {
        status!("group warning: {}", warning.message());
    }
    let group_warning_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(GROUP_WARNING_INTERVAL);
        interval.tick().await;
        loop {
            interval.tick().await;
            evaluate_group_warnings(&mut group_watcher, &groups_log, unix_now());
        }
    });

    tokio::signal::ctrl_c().await?;
    remove_serve_lock().await?;
    router.shutdown().await?;
    #[cfg(feature = "web")]
    if let Some(sync_task) = _event_sync_handle {
        sync_task.abort();
        let _ = sync_task.await;
    }
    #[cfg(feature = "web")]
    for web_task in _web_handles {
        web_task.abort();
        let _ = web_task.await;
    }
    #[cfg(feature = "ssh")]
    if let Some(ssh_task) = ssh_handle {
        ssh_task.abort();
        let _ = ssh_task.await;
    }
    #[cfg(feature = "world")]
    if let Some(flush) = outbox_flush {
        flush.abort();
        let _ = flush.await;
    }
    #[cfg(feature = "world")]
    if let Some(hub) = &world_hub {
        hub.shutdown_all().await;
    }
    group_warning_handle.abort();
    let _ = group_warning_handle.await;
    store.shutdown().await?;
    Ok(())
}

/// How often serve re-evaluates the group warnings against the group log.
const GROUP_WARNING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// The serve-side group warning watcher (P4.9, #799).
///
/// Serve holds exactly one of these for the life of the process, so the
/// [`Evaluator`]'s high-water marks survive across evaluations: a group that
/// crosses a size threshold warns once, not on every pass. The groups in the
/// node's own group log are the ones its operator made through the CLI, so
/// every group is observed as client-made. Member counts come from the fold.
///
/// Group events carry no timestamps, so churn cannot be reconstructed from
/// the log for time before serve started. Instead the watcher counts re-adds
/// from the log events folded since its previous evaluation (#1847): a
/// device authorization that takes effect for a device the watcher has seen
/// authorized before, and that is not authorized at that point in the event
/// stream, is a re-add, stamped with the time of the evaluation that folds
/// it. Counting from the event stream — not from the net device set — means
/// a revoke + re-add cycle that completes inside one evaluation interval
/// still counts. The first evaluation only establishes the baseline (the
/// devices the fold already shows), so history from before serve started
/// never counts.
struct GroupWarningWatcher {
    evaluator: Evaluator,
    /// The devices authorized per group at the last event the watcher has
    /// folded, tracked through the event stream, not just at evaluations,
    /// with the accounts whose grants authorize each device: a revocation
    /// is account-scoped in the fold (it ends only the revoking account's
    /// grant), so a device stays authorized while any account's grant for
    /// it remains. An empty account set means the fold shows the device
    /// authorized but the watcher has not observed a grant for it (the
    /// device was adopted in the fold sync in `evaluate`).
    authorized: std::collections::BTreeMap<
        Hash,
        std::collections::BTreeMap<GroupKey, std::collections::BTreeSet<GroupKey>>,
    >,
    /// Every device ever seen authorized per group.
    seen_devices: std::collections::BTreeMap<Hash, std::collections::BTreeSet<GroupKey>>,
    /// The re-add times per group, as observed by this watcher.
    readds: std::collections::BTreeMap<Hash, Vec<u64>>,
    /// How many log events the watcher has folded, or `None` before the
    /// first evaluation establishes the baseline.
    events_seen: Option<usize>,
}

impl GroupWarningWatcher {
    fn new() -> Self {
        Self {
            evaluator: Evaluator::default(),
            authorized: std::collections::BTreeMap::new(),
            seen_devices: std::collections::BTreeMap::new(),
            readds: std::collections::BTreeMap::new(),
            events_seen: None,
        }
    }

    /// Evaluates every active group in `projection` at `now` (Unix seconds).
    /// Returns the warnings this evaluation adds, and writes each of them to
    /// the server log through the warnings module.
    fn evaluate(
        &mut self,
        projection: &Projection,
        events: &[crate::groups::Event],
        now: u64,
    ) -> Vec<(Hash, Warning)> {
        let delta_readds = self.fold_new_events(projection, events);
        let mut fired = Vec::new();
        for (group, view) in projection.groups() {
            if !view.is_active() {
                continue;
            }
            // Keep the event-tracked authorization in step with the fold:
            // a device can leave or rejoin the active set without a device
            // event of its own (its account was removed or re-added). Such
            // a change is not a re-add — only a device event counts — but
            // the tracked set must match the fold, or a later genuine
            // re-add would be missed or counted twice.
            let current = projection.group_active_devices(group);
            {
                // Refresh the authorizing accounts from the fold as well
                // as the device keys: an account can stop authorizing a
                // device without a device event of its own (its membership
                // was removed), and a stale account left in the tracked
                // set would keep the device authorized through every
                // later revoke by the remaining accounts, hiding genuine
                // churn (Codex review on PR #4035).
                let mut fold_accounts: std::collections::BTreeMap<
                    GroupKey,
                    std::collections::BTreeSet<GroupKey>,
                > = std::collections::BTreeMap::new();
                for account in view.members.keys() {
                    for device in projection.active_devices(group, account) {
                        fold_accounts.entry(device).or_default().insert(*account);
                    }
                }
                let authorized = self.authorized.entry(*group).or_default();
                authorized.retain(|device, _accounts| current.contains(device));
                let seen = self.seen_devices.entry(*group).or_default();
                for device in &current {
                    let is_new = !authorized.contains_key(device);
                    if let Some(accounts) = fold_accounts.get(device) {
                        authorized.insert(*device, accounts.clone());
                    } else if is_new {
                        authorized.insert(*device, std::collections::BTreeSet::new());
                    }
                    if is_new {
                        seen.insert(*device);
                    }
                }
            }
            let fresh_readds = delta_readds.get(group).copied().unwrap_or(0);
            if fresh_readds > 0 {
                let times = self.readds.entry(*group).or_default();
                for _ in 0..fresh_readds {
                    times.push(now);
                }
                times.retain(|at| {
                    now.saturating_sub(*at) < crate::groups::warnings::CHURN_WINDOW_SECS
                });
            }
            let readds = self.readds.get(group).cloned().unwrap_or_default();
            let observation = Observation {
                group: *group,
                client_made: true,
                members: view.members.len(),
                readds: &readds,
            };
            let warnings = self.evaluator.observe(&observation, now);
            if !warnings.is_empty() {
                crate::groups::warnings::log(group, &warnings);
                fired.extend(warnings.into_iter().map(|warning| (*group, warning)));
            }
        }
        fired
    }

    /// Folds the log events recorded since the previous evaluation and
    /// counts the re-adds among them, per group. Only events the fold
    /// applied count: a refused authorization authorizes nothing, and a
    /// refused revocation revokes nothing.
    ///
    /// The first call only establishes the baseline — it adopts the fold's
    /// current devices and counts nothing — because events carry no
    /// timestamps and churn from before serve started must not be
    /// reconstructed. A log that shrank since the previous evaluation (it
    /// was replaced) likewise re-baselines instead of counting.
    fn fold_new_events(
        &mut self,
        projection: &Projection,
        events: &[crate::groups::Event],
    ) -> std::collections::BTreeMap<Hash, usize> {
        let mut counts = std::collections::BTreeMap::new();
        let Some(cursor) = self.events_seen else {
            self.baseline(projection);
            self.events_seen = Some(events.len());
            return counts;
        };
        if cursor > events.len() {
            self.baseline(projection);
            self.events_seen = Some(events.len());
            return counts;
        }
        for event in &events[cursor..] {
            let Some(group) = event.group else {
                continue;
            };
            let crate::groups::Body::Device(op) = &event.body else {
                continue;
            };
            if projection.outcome(&event.id) != Some(&crate::groups::Outcome::Applied) {
                continue;
            }
            match op {
                crate::groups::DeviceOp::Add { device, .. } => {
                    let seen = self.seen_devices.entry(group).or_default();
                    let was_seen = seen.contains(device);
                    seen.insert(*device);
                    let authorized = self.authorized.entry(group).or_default();
                    if !authorized.contains_key(device) && was_seen {
                        *counts.entry(group).or_default() += 1;
                    }
                    authorized.entry(*device).or_default().insert(event.author);
                }
                crate::groups::DeviceOp::Revoke { device } => {
                    // A revocation is account-scoped in the fold: it ends
                    // only the revoking account's grant, so the device
                    // stays authorized while another account's grant for
                    // it remains (Codex review on PR #4035). Removing the
                    // device outright would count the other account's
                    // later re-add as churn that never happened.
                    if let Some(authorized) = self.authorized.get_mut(&group) {
                        let mut ended = false;
                        if let Some(accounts) = authorized.get_mut(device) {
                            ended = accounts.remove(&event.author) && accounts.is_empty();
                        }
                        if ended {
                            authorized.remove(device);
                        }
                    }
                }
            }
        }
        self.events_seen = Some(events.len());
        counts
    }

    /// Adopts the fold's current devices as the watcher's tracked state,
    /// counting nothing: the state before the baseline is history the
    /// watcher did not observe.
    fn baseline(&mut self, projection: &Projection) {
        self.authorized.clear();
        self.seen_devices.clear();
        for (group, view) in projection.groups() {
            let current = projection.group_active_devices(group);
            self.seen_devices.insert(*group, current.clone());
            // Record which accounts' grants authorize each device, so a
            // later account-scoped revocation replays correctly against
            // the baseline too.
            let mut devices: std::collections::BTreeMap<
                GroupKey,
                std::collections::BTreeSet<GroupKey>,
            > = current
                .iter()
                .map(|device| (*device, std::collections::BTreeSet::new()))
                .collect();
            for account in view.members.keys() {
                for device in projection.active_devices(group, account) {
                    devices.entry(device).or_default().insert(*account);
                }
            }
            self.authorized.insert(*group, devices);
        }
    }
}

/// One warning evaluation pass over the group log at `path`.
///
/// A missing log means no groups, not an error. A log that cannot be read or
/// folded is noted in the server log and skipped: warnings never block serve.
fn evaluate_group_warnings(
    watcher: &mut GroupWarningWatcher,
    path: &std::path::Path,
    now: u64,
) -> Vec<(Hash, Warning)> {
    if !path.exists() {
        return Vec::new();
    }
    let log = match GroupLog::open(path) {
        Ok(log) => log,
        Err(error) => {
            debug!("group warnings: cannot open {}: {error}", path.display());
            return Vec::new();
        }
    };
    let projection = match log.projection() {
        Ok(projection) => projection,
        Err(error) => {
            debug!("group warnings: cannot fold {}: {error}", path.display());
            return Vec::new();
        }
    };
    watcher.evaluate(&projection, log.events(), now)
}

/// The current time as Unix seconds, for the warning evaluator's clock.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

const MAX_STALE_PROBES_PER_PASS: usize = 4;

/// Verifies an incoming presence heartbeat and stores only the channels whose
/// folded group membership contains the sender (#4363). Membership comes from
/// the receiver's own group fold for the channel (the `GroupView` member
/// set); a channel the receiver holds no fold for scopes to empty, because
/// membership the receiver cannot see cannot be confirmed. If the local group
/// log cannot be read or folded, the heartbeat is refused rather than
/// accepted without its scope check. Signature verification is unchanged: a
/// heartbeat whose signature by its claimed node does not verify is dropped.
fn receive_presence_heartbeat(
    table: &PresenceTable,
    heartbeat: PresenceHeartbeat,
    groups_log: &Path,
) -> bool {
    if !heartbeat.verify() {
        debug!("presence heartbeat failed signature verification, dropped");
        return false;
    }

    let log = match GroupLog::open(groups_log) {
        Ok(log) => log,
        Err(error) => {
            debug!("presence heartbeat dropped: cannot open groups log: {error}");
            return false;
        }
    };
    let projection = match log.projection() {
        Ok(projection) => projection,
        Err(error) => {
            debug!("presence heartbeat dropped: cannot fold groups log: {error}");
            return false;
        }
    };

    let node = heartbeat.node;
    let channels = heartbeat.channels.len();
    let accepted = table.update_scoped(heartbeat, &|sender: &AuthorKey, group: &Hash| {
        projection
            .group(group)
            .is_some_and(|view| view.members.contains_key(sender))
    });
    if accepted {
        debug!(
            "presence heartbeat from {}: {channels} claimed channel(s)",
            node.iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
    }
    accepted
}

/// The presence-heartbeat period for the gossip loop: [`HEARTBEAT_INTERVAL`],
/// the cadence `chat_presence` documents, as the single source (#4777).
/// A function (not an inline constant at the call site) so a test can pin
/// the wiring and the constant cannot silently go dead again.
fn presence_heartbeat_period() -> std::time::Duration {
    HEARTBEAT_INTERVAL
}

/// Background loop for gossip-based peer discovery.
///
/// Runs three concurrent tasks:
/// 1. **Announce**: Broadcasts a [`PeerAnnouncement`] every [`ANNOUNCE_INTERVAL`],
///    and a presence heartbeat every [`HEARTBEAT_INTERVAL`] on its own timer
/// 2. **Receive**: Listens for incoming gossip events and updates the peer table
/// 3. **Cleanup**: Periodically probes stale peers via `ListPeers` RPC — refreshes
///    their `last_seen` if reachable, removes them if not
///
/// This function runs until the sender/receiver are dropped (on shutdown).
async fn run_gossip_loop(
    node_id: EndpointId,
    sender: distributed_topic_tracker::GossipSender,
    mut receiver: distributed_topic_tracker::GossipReceiver,
    peer_discovery: PeerDiscovery,
    chat_presence: PresenceTable,
    groups_log: PathBuf,
    signing_key: ed25519_dalek::SigningKey,
    store: iroh_blobs::api::Store,
    endpoint: Endpoint,
) {
    // Timeout for each individual peer probe.
    const PROBE_TIMEOUT: tokio::time::Duration = tokio::time::Duration::from_secs(10);

    let announce_store = store;
    let announce_node_id = node_id;

    // Spawn announce task: peer announcements and presence heartbeats run
    // on independent timers in this one task. They used to share the
    // announcement timer, so heartbeats actually went out every 30 s and
    // the documented 25 s HEARTBEAT_INTERVAL cadence (and the TTL
    // drop-resilience it promises) never happened (#4777).
    let announce_handle = tokio::spawn(async move {
        let mut announce_interval = tokio::time::interval(ANNOUNCE_INTERVAL);
        let mut heartbeat_interval = tokio::time::interval(presence_heartbeat_period());
        loop {
            tokio::select! {
                _ = announce_interval.tick() => {
                    // Get blob count from tags
                    let blob_count = match announce_store.tags().list().await {
                        Ok(mut stream) => {
                            let mut count = 0u64;
                            while stream.next().await.is_some() {
                                count += 1;
                            }
                            count
                        }
                        Err(_) => 0,
                    };

                    let announcement = PeerAnnouncement {
                        node_id: announce_node_id,
                        name: None,
                        blob_count,
                        timestamp_secs: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |d| d.as_secs()),
                    };

                    match SignedPeerAnnouncement::sign(announcement, &signing_key) {
                        Some(signed) => match signed.encode_frame() {
                            Ok(bytes) => {
                                if let Err(e) = sender.broadcast(bytes).await {
                                    debug!("gossip broadcast error: {}", e);
                                }
                            }
                            Err(e) => {
                                debug!("gossip announcement serialization error: {}", e);
                            }
                        },
                        None => {
                            warn!(
                                "could not sign peer announcement for endpoint {}",
                                announce_node_id
                            );
                        }
                    }

                }
                _ = heartbeat_interval.tick() => {
                    // Presence rides the same gossip plane (#2809): an ephemeral
                    // heartbeat naming this node. Channel membership lives in the
                    // chat layer above serve, so this heartbeat names no channels;
                    // receivers scope and read it through PresenceTable.
                    let mut presence = PresenceHeartbeat::new(
                        *announce_node_id.as_bytes(),
                        Vec::new(),
                        Vec::new(),
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |d| d.as_secs()),
                    );
                    presence.sign(&signing_key);
                    if let Err(e) = sender.broadcast(encode_frame(&presence)).await {
                        debug!("gossip presence broadcast error: {}", e);
                    }
                }
            }
        }
    });

    // Spawn receive task
    let recv_discovery = peer_discovery.clone();
    let recv_presence = chat_presence.clone();
    let recv_groups_log = groups_log;
    let recv_handle = tokio::spawn(async move {
        loop {
            match receiver.next().await {
                Ok(event) => match event {
                    iroh_gossip::api::Event::Received(msg) => {
                        if let Some(heartbeat) = decode_frame(&msg.content) {
                            // Presence is a claim about `node`, and the gossip
                            // topic is public and does not authenticate
                            // authors: accept the frame only when the
                            // heartbeat's signature by its claimed node
                            // verifies, so nobody can forge or suppress
                            // another node's presence (#2809 Codex review).
                            // The accepted heartbeat is then scoped to the
                            // sender's membership in the receiver's group
                            // fold (#4363).
                            receive_presence_heartbeat(&recv_presence, heartbeat, &recv_groups_log);
                        } else if let Some(announcement) =
                            SignedPeerAnnouncement::decode_frame(&msg.content)
                        {
                            debug!(
                                "peer announcement from {}: blob_count={}",
                                announcement.node_id, announcement.blob_count
                            );
                            recv_discovery.update(announcement);
                        } else {
                            debug!("unauthenticated or invalid peer announcement frame dropped");
                        }
                    }
                    iroh_gossip::api::Event::NeighborUp(peer) => {
                        info!("gossip neighbor up: {}", peer);
                    }
                    iroh_gossip::api::Event::NeighborDown(peer) => {
                        info!("gossip neighbor down: {}", peer);
                    }
                    iroh_gossip::api::Event::Lagged => {
                        warn!("gossip receiver lagged, some messages were missed");
                    }
                },
                Err(e) => {
                    debug!("gossip receiver stream ended: {}", e);
                    break;
                }
            }
        }
    });

    // Spawn stale cleanup task — probes stale peers before removal
    let cleanup_discovery = peer_discovery;
    let cleanup_presence = chat_presence;

    let cleanup_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(STALE_CHECK_INTERVAL);
        let mut probe_cursor: Option<EndpointId> = None;

        loop {
            interval.tick().await;

            // Presence entries expire on their own stamps; prune them here so a
            // long-running server does not retain every node key it has ever
            // seen (#2809 Codex review). This runs even when no discovery peer
            // is stale.
            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let pruned = cleanup_presence.prune_at(now_secs);
            if pruned > 0 {
                debug!("pruned {} expired presence entrie(s)", pruned);
            }

            let stale = cleanup_discovery.stale_peers();
            if stale.is_empty() {
                continue;
            }

            let batch = select_stale_probe_batch(&stale, probe_cursor, MAX_STALE_PROBES_PER_PASS);
            if let Some(last) = batch.last() {
                probe_cursor = Some(last.announcement.node_id);
            }
            debug!("probing {} of {} stale peer(s)", batch.len(), stale.len());

            for info in &batch {
                let peer_id = info.announcement.node_id;

                // Try to connect and send ListPeers RPC
                let probe_result = tokio::time::timeout(PROBE_TIMEOUT, async {
                    let conn = crate::connectivity::connect(&endpoint, peer_id, META_ALPN).await?;
                    let (mut send, mut recv) = conn.open_bi().await?;
                    let req = postcard::to_allocvec(&MetaRequest::ListPeers)?;
                    send.write_all(&req).await?;
                    send.finish()?;
                    let resp_buf = crate::meta_client::read_capped_response(&mut recv).await?;
                    let _resp: MetaResponse = crate::protocol::decode_meta_exact(&resp_buf)?;
                    conn.close(0u32.into(), b"probe");
                    anyhow::Ok(())
                })
                .await;

                match probe_result {
                    Ok(Ok(())) => {
                        debug!("stale peer {} is still alive, refreshing", peer_id);
                        cleanup_discovery.refresh(&peer_id);
                    }
                    Ok(Err(e)) => {
                        debug!("stale peer {} probe failed: {}, removing", peer_id, e);
                        cleanup_discovery.remove_if_stale(&peer_id, STALE_THRESHOLD);
                    }
                    Err(_) => {
                        debug!("stale peer {} probe timed out, removing", peer_id);
                        cleanup_discovery.remove_if_stale(&peer_id, STALE_THRESHOLD);
                    }
                }
            }
        }
    });

    // Wait for any task to complete (normally they run until shutdown)
    tokio::select! {
        _ = announce_handle => {}
        _ = recv_handle => {}
        _ = cleanup_handle => {}
    }
}

/// Select a deterministic, bounded round-robin batch from stale peers.
fn select_stale_probe_batch(
    stale: &[crate::discovery::PeerInfo],
    after: Option<EndpointId>,
    limit: usize,
) -> Vec<crate::discovery::PeerInfo> {
    if stale.is_empty() || limit == 0 {
        return Vec::new();
    }

    let mut ordered = stale.to_vec();
    ordered.sort_by(|left, right| {
        left.announcement
            .node_id
            .as_bytes()
            .cmp(right.announcement.node_id.as_bytes())
    });
    let start = after.map_or(0, |cursor| {
        let next = ordered
            .partition_point(|info| info.announcement.node_id.as_bytes() <= cursor.as_bytes());
        if next == ordered.len() { 0 } else { next }
    });
    (0..limit.min(ordered.len()))
        .map(|offset| ordered[(start + offset) % ordered.len()].clone())
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn stale_test_peer(seed: u8) -> crate::discovery::PeerInfo {
        let node_id = iroh_base::SecretKey::from_bytes(&[seed; 32]).public();
        crate::discovery::PeerInfo {
            announcement: PeerAnnouncement {
                node_id,
                name: None,
                blob_count: 0,
                timestamp_secs: unix_now(),
            },
            last_seen: std::time::Instant::now(),
        }
    }

    #[test]
    fn presence_heartbeat_period_is_the_documented_interval() {
        // #4777: HEARTBEAT_INTERVAL was defined and documented in
        // chat_presence but never used; serve broadcast presence on the
        // 30 s announcement timer instead. Pin the gossip loop period to
        // the constant so it cannot silently go dead again.
        assert_eq!(presence_heartbeat_period(), HEARTBEAT_INTERVAL);
        assert_ne!(presence_heartbeat_period(), ANNOUNCE_INTERVAL);
        // The documented TTL resilience: at least two heartbeats land
        // inside one PRESENCE_TTL_SECS window, so a single dropped
        // broadcast never flickers a live node stale.
        assert!(
            HEARTBEAT_INTERVAL.as_secs() * 2 < crate::chat_presence::PRESENCE_TTL_SECS,
            "two heartbeats must land inside one TTL window"
        );
    }

    #[test]
    fn stale_probe_batches_are_bounded_sorted_and_round_robin() {
        let mut peers: Vec<_> = (1..=5).map(stale_test_peer).collect();
        peers.reverse();
        let mut expected: Vec<_> = peers.iter().map(|peer| peer.announcement.node_id).collect();
        expected.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));

        assert_eq!(MAX_STALE_PROBES_PER_PASS, 4);
        let first = select_stale_probe_batch(&peers, None, 2);
        let first_ids: Vec<_> = first.iter().map(|peer| peer.announcement.node_id).collect();
        assert_eq!(first_ids, expected[..2]);

        let second = select_stale_probe_batch(
            &peers,
            first.last().map(|peer| peer.announcement.node_id),
            2,
        );
        let second_ids: Vec<_> = second
            .iter()
            .map(|peer| peer.announcement.node_id)
            .collect();
        assert_eq!(second_ids, expected[2..4]);

        let third = select_stale_probe_batch(
            &peers,
            second.last().map(|peer| peer.announcement.node_id),
            2,
        );
        let third_ids: Vec<_> = third.iter().map(|peer| peer.announcement.node_id).collect();
        assert_eq!(third_ids, vec![expected[4], expected[0]]);
        assert!(select_stale_probe_batch(&peers, None, 0).is_empty());
    }

    /// A scratch directory unique to this test process.
    fn serve_test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("id-serve-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Builds a group log on disk holding one group of `members` accounts
    /// (the author included), returning the group's ID. This is the file the
    /// serve warning path reads.
    fn group_log_with_members(dir: &std::path::Path, members: usize) -> Hash {
        group_log_with_members_at(&dir.join(GROUPS_FILE_NAME), members)
    }

    fn group_log_with_members_at(path: &std::path::Path, members: usize) -> Hash {
        use crate::groups::Role;
        use crate::groups::actions::NewGroup;

        let mut log = GroupLog::open(path).unwrap();
        let author: GroupKey = [1; 32];
        let created = log
            .create(
                author,
                NewGroup::Normal {
                    max: None,
                    admins_on_create: false,
                },
            )
            .unwrap();
        for i in 1..members {
            let member: GroupKey = [u8::try_from(i + 1).unwrap(); 32];
            log.add(author, created.group, member, Role::Read).unwrap();
        }
        created.group
    }

    #[test]
    fn received_presence_keeps_only_channels_the_sender_belongs_to() {
        // #4363: the serve receipt path must scope an incoming heartbeat to
        // the sender's membership in the receiver's group fold, not store the
        // claimed channels unscoped.
        use ed25519_dalek::SigningKey;

        use crate::chat_presence::Liveness;
        use crate::groups::Role;

        let dir = serve_test_dir("presence-membership");
        let path = dir.join(GROUPS_FILE_NAME);
        let member_channel = group_log_with_members_at(&path, 1);
        let outsider_channel = Hash::digest(b"not-a-member");
        let signing_key = SigningKey::from_bytes(&[2; 32]);
        let node = signing_key.verifying_key().to_bytes();
        // The sender genuinely belongs to `member_channel`: enrol the
        // heartbeat key in the receiver's group fold. Nothing enrols it in
        // `outsider_channel`, which the receiver holds no fold for.
        {
            let mut log = GroupLog::open(&path).unwrap();
            log.add([1; 32], member_channel, node, Role::Read).unwrap();
        }
        let mut heartbeat = PresenceHeartbeat::new(
            node,
            vec![member_channel, outsider_channel],
            vec![member_channel, outsider_channel],
            unix_now(),
        );
        heartbeat.sign(&signing_key);
        assert!(
            heartbeat.verify(),
            "fixture heartbeat must be authenticated"
        );

        let table = PresenceTable::new();
        assert!(receive_presence_heartbeat(&table, heartbeat, &path));
        assert_eq!(table.liveness(&node, &member_channel), Liveness::Live);
        assert_eq!(table.liveness(&node, &outsider_channel), Liveness::Offline);
        let now = unix_now();
        let member_rows = table.roster_at(&member_channel, &[node], now);
        assert!(member_rows.iter().any(|row| row.node == node && row.typing));
        let outsider_rows = table.roster_at(&outsider_channel, &[node], now);
        assert!(outsider_rows.iter().all(|row| !row.typing));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn serve_groups_log_path_honours_override_and_default() {
        // #2778: the group-warning watcher's log path must come
        // from the global --groups-log option (threaded through
        // ServeOptions), resolved exactly like the group commands
        // resolve it; without the option the default file name is
        // used, unchanged.
        let custom = PathBuf::from("shared").join("relocated.jsonl");
        assert_eq!(serve_groups_log_path(Some(custom.clone())), custom);
        assert_eq!(serve_groups_log_path(None), PathBuf::from(GROUPS_FILE_NAME));
    }

    #[test]
    fn serve_group_warnings_evaluate_custom_groups_log() {
        // #2778 regression: warnings are computed from the
        // --groups-log file, not the default groups.jsonl. The
        // custom log holds a 96-member group; the default log in
        // the same directory does not exist, so a watcher reading
        // the default would silently warn about nothing.
        let dir = serve_test_dir("customlog");
        let custom = dir.join("relocated.jsonl");
        let group = group_log_with_members_at(&custom, 96);
        let path = serve_groups_log_path(Some(custom));
        let mut watcher = GroupWarningWatcher::new();
        let fired = evaluate_group_warnings(&mut watcher, &path, 1_000);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].0, group);
        assert!(matches!(
            fired[0].1,
            Warning::Size {
                threshold: 96,
                members: 96
            }
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn serve_group_warnings_fire_once_at_each_size_threshold() {
        let dir = serve_test_dir("size");
        let path = dir.join(GROUPS_FILE_NAME);
        let group = group_log_with_members(&dir, 96);
        let mut watcher = GroupWarningWatcher::new();

        // The integration surface: serve's own evaluation over the on-disk
        // group log warns when the group crosses 96 members (P4.9's e2e
        // column: "the server warns at a threshold").
        let fired = evaluate_group_warnings(&mut watcher, &path, 1_000);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].0, group);
        assert!(matches!(
            fired[0].1,
            Warning::Size {
                threshold: 96,
                members: 96
            }
        ));

        // The persistent Evaluator's high-water mark: the same state warns
        // no second time.
        assert!(evaluate_group_warnings(&mut watcher, &path, 1_001).is_empty());

        // Growing past the next threshold warns once more, for that tier.
        {
            let mut log = GroupLog::open(&path).unwrap();
            let author: GroupKey = [1; 32];
            for i in 96..128 {
                let member: GroupKey = [u8::try_from(i + 1).unwrap(); 32];
                log.add(author, group, member, crate::groups::Role::Read)
                    .unwrap();
            }
        }
        let fired = evaluate_group_warnings(&mut watcher, &path, 1_002);
        assert_eq!(fired.len(), 1);
        assert!(matches!(
            fired[0].1,
            Warning::Size {
                threshold: 128,
                members: 128
            }
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn serve_group_warnings_fire_on_device_readd_churn() {
        use crate::groups::Role;

        let dir = serve_test_dir("churn");
        let path = dir.join(GROUPS_FILE_NAME);
        let group = group_log_with_members(&dir, 2);
        let author: GroupKey = [1; 32];
        let device: GroupKey = [200; 32];
        let mut log = GroupLog::open(&path).unwrap();
        let mut watcher = GroupWarningWatcher::new();

        // The first authorization is not a re-add, and warns about nothing.
        log.device_add(author, group, device, Role::Read).unwrap();
        assert!(evaluate_group_warnings(&mut watcher, &path, 1_000).is_empty());

        // Four revoke + re-authorize cycles inside the window: the fourth
        // re-add crosses the churn threshold and warns exactly once.
        for cycle in 0..4_u64 {
            log.device_revoke(author, group, device).unwrap();
            assert!(evaluate_group_warnings(&mut watcher, &path, 1_001 + cycle * 2).is_empty());
            log.device_add(author, group, device, Role::Read).unwrap();
            let fired = evaluate_group_warnings(&mut watcher, &path, 1_002 + cycle * 2);
            if cycle < 3 {
                assert!(fired.is_empty(), "cycle {cycle} fired early: {fired:?}");
            } else {
                assert_eq!(fired.len(), 1);
                assert!(matches!(fired[0].1, Warning::Churn { readds: 4 }));
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn serve_group_warnings_fire_on_intra_interval_readd_churn() {
        use crate::groups::Role;

        // #1847 regression: revoke + re-add cycles that complete inside
        // one evaluation interval are invisible to a net-set comparison.
        // Four cycles written to the group log with no evaluation in
        // between must still count four re-adds at the next evaluation.
        let dir = serve_test_dir("churn-intra-interval");
        let path = dir.join(GROUPS_FILE_NAME);
        let group = group_log_with_members(&dir, 2);
        let author: GroupKey = [1; 32];
        let device: GroupKey = [200; 32];
        let mut log = GroupLog::open(&path).unwrap();
        let mut watcher = GroupWarningWatcher::new();

        // The first authorization is not a re-add, and warns about nothing.
        log.device_add(author, group, device, Role::Read).unwrap();
        assert!(evaluate_group_warnings(&mut watcher, &path, 1_000).is_empty());

        // Four revoke + re-authorize cycles, all inside one interval:
        // no evaluation runs until every cycle is already in the log.
        for _ in 0..4 {
            log.device_revoke(author, group, device).unwrap();
            log.device_add(author, group, device, Role::Read).unwrap();
        }
        let fired = evaluate_group_warnings(&mut watcher, &path, 1_001);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].0, group);
        assert!(matches!(fired[0].1, Warning::Churn { readds: 4 }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn serve_group_warnings_ignore_other_accounts_revoke_readd_cycles() {
        use crate::groups::Role;

        // Codex review on PR #4035: a revocation is account-scoped in the
        // fold - it ends only the revoking account's grant. When account A
        // has authorized a device, revoke + re-add cycles by account B
        // never leave the device unauthorized (A's grant stands
        // throughout), so they are not re-adds and must not fire the churn
        // warning.
        let dir = serve_test_dir("churn-other-account");
        let path = dir.join(GROUPS_FILE_NAME);
        let group = group_log_with_members(&dir, 2);
        let author: GroupKey = [1; 32];
        let other: GroupKey = [2; 32];
        let device: GroupKey = [200; 32];
        let mut log = GroupLog::open(&path).unwrap();
        let mut watcher = GroupWarningWatcher::new();

        // Account A authorizes the device; the first evaluation adopts it.
        log.device_add(author, group, device, Role::Read).unwrap();
        assert!(evaluate_group_warnings(&mut watcher, &path, 1_000).is_empty());

        // Four revoke + re-add cycles by account B inside one interval:
        // B's revokes do not touch A's grant, so nothing here is a re-add.
        for _ in 0..4 {
            log.device_revoke(other, group, device).unwrap();
            log.device_add(other, group, device, Role::Read).unwrap();
        }
        assert!(evaluate_group_warnings(&mut watcher, &path, 1_001).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn serve_group_warnings_count_churn_after_a_grant_owner_is_removed() {
        use crate::groups::Role;

        // Codex review on PR #4035 (second round): when accounts A and B
        // both authorize a device and A is then removed from the group,
        // the device stays active through B, but the watcher's tracked
        // account set must drop A too. A stale A would keep the device
        // authorized through every later revoke by B, so genuine
        // revoke + re-add cycles by B would never count as churn.
        let dir = serve_test_dir("churn-grant-owner-removed");
        let path = dir.join(GROUPS_FILE_NAME);
        let group = group_log_with_members(&dir, 2);
        let author: GroupKey = [1; 32];
        let other: GroupKey = [2; 32];
        let device: GroupKey = [200; 32];
        let mut log = GroupLog::open(&path).unwrap();
        let mut watcher = GroupWarningWatcher::new();

        // Both accounts authorize the device; the first evaluation
        // adopts it with both grant owners tracked.
        log.device_add(author, group, device, Role::Read).unwrap();
        log.device_add(other, group, device, Role::Read).unwrap();
        assert!(evaluate_group_warnings(&mut watcher, &path, 1_000).is_empty());

        // Removing the other account leaves the device authorized
        // through the author alone; the evaluation reconciles the
        // tracked grant owners with the fold. Not itself a re-add.
        log.remove(author, group, other, crate::groups::RemoveMode::Revoke)
            .unwrap();
        assert!(evaluate_group_warnings(&mut watcher, &path, 1_001).is_empty());

        // Four genuine revoke + re-add cycles by the remaining account,
        // all inside one interval, must count four re-adds and warn.
        for _ in 0..4 {
            log.device_revoke(author, group, device).unwrap();
            log.device_add(author, group, device, Role::Read).unwrap();
        }
        let fired = evaluate_group_warnings(&mut watcher, &path, 1_002);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].0, group);
        assert!(matches!(fired[0].1, Warning::Churn { readds: 4 }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "world")]
    #[test]
    fn test_the_server_ceiling_is_the_named_list_or_the_default_ceiling() {
        assert_eq!(
            allowed_caps(None).unwrap(),
            crate::world_caps::default_ceiling_names()
        );
        let named = ["time.now".to_owned(), "chat.say".to_owned()];
        assert_eq!(
            allowed_caps(Some(&named))
                .unwrap()
                .into_iter()
                .collect::<Vec<_>>(),
            vec!["chat.say".to_owned(), "time.now".to_owned()]
        );
        assert!(allowed_caps(Some(&["*".to_owned()])).is_err());
        assert!(allowed_caps(Some(&["telepathy".to_owned()])).is_err());
    }

    #[test]
    fn world_mode_requires_an_admin_token_but_not_web() {
        assert!(validate_world_options(true, None, None, "lobby", "deny", false).is_err());
        assert!(validate_world_options(true, Some(""), None, "lobby", "deny", false).is_err());
        assert!(validate_world_options(true, Some("admin"), None, "lobby", "deny", false).is_ok());
        assert!(validate_world_options(false, None, None, "lobby", "deny", false).is_ok());
        assert!(
            validate_world_options(
                true,
                Some("admin"),
                Some(&PathBuf::from("world.wasm")),
                "lobby",
                "deny",
                false
            )
            .is_ok()
        );
        assert!(validate_world_options(true, Some("admin"), None, "lobby", "deny", true).is_ok());
        assert!(
            validate_world_options(
                true,
                Some("admin"),
                Some(&PathBuf::from("world.wasm")),
                "lobby",
                "deny",
                true
            )
            .is_err()
        );
        assert!(
            validate_world_options(
                false,
                None,
                Some(&PathBuf::from("world.wasm")),
                "lobby",
                "deny",
                false
            )
            .is_err()
        );
    }

    #[cfg(feature = "world")]
    #[test]
    fn mail_sink_picks_one_transport_and_refuses_an_unusable_relay() {
        let loopback: SocketAddr = "127.0.0.1:25".parse().unwrap();
        let remote: SocketAddr = "203.0.113.5:25".parse().unwrap();
        let sender = Some("id@example.test".to_owned());
        assert!(mail_sink(None, None, None, None).unwrap().is_none());
        assert!(mail_sink(None, None, Some(loopback), None).is_err());
        assert!(mail_sink(None, None, Some(remote), sender.clone()).is_err());
        assert!(
            mail_sink(None, None, Some(loopback), sender)
                .unwrap()
                .is_some()
        );
    }

    #[cfg(feature = "world")]
    #[test]
    fn world_names_are_bounded_and_are_a_single_path_segment() {
        for bad in [
            "",
            "../evil",
            ".",
            "..",
            "Tic-Tac-Toe",
            "a/b",
            "sp ace",
            "wörld",
        ] {
            assert!(
                validate_world_options(true, Some("admin"), None, bad, "deny", false).is_err(),
                "{bad:?} must be refused"
            );
        }
        assert!(validate_world_options(true, Some("admin"), None, "tt-2_x", "deny", false).is_ok());
        assert!(
            validate_world_options(true, Some("admin"), None, &"a".repeat(64), "deny", false)
                .is_ok()
        );
        assert!(
            validate_world_options(true, Some("admin"), None, &"a".repeat(65), "deny", false)
                .is_err()
        );
        assert!(validate_world_options(true, Some("admin"), None, "lobby", "deny", false).is_ok());
        assert!(
            validate_world_options(true, Some("admin"), None, "lobby", "grant-on-use", false)
                .is_ok()
        );
        assert!(
            validate_world_options(true, Some("admin"), None, "lobby", "sometimes", false).is_err(),
            "an unknown policy is refused"
        );
    }

    #[test]
    fn test_is_process_alive_current_process() {
        let pid = std::process::id();
        assert!(is_process_alive(pid));
    }

    #[test]
    fn test_is_process_alive_pid_zero() {
        #[cfg(unix)]
        {
            assert!(!is_process_alive(0));
        }
    }

    #[test]
    fn test_is_process_alive_pid_out_of_i32_range() {
        #[cfg(unix)]
        {
            assert!(!is_process_alive(i32::MAX as u32 + 1));
            assert!(!is_process_alive(u32::MAX));
        }
    }

    #[test]
    fn test_is_process_alive_nonexistent() {
        // Use a very high PID that's unlikely to exist
        // Note: On non-Unix this always returns true
        #[cfg(unix)]
        {
            assert!(!is_process_alive(999_999_999));
        }
    }

    #[test]
    fn test_is_process_alive_pid_1() {
        // PID 1 exists in every Unix PID namespace (as init, or as the
        // namespace's init in a container). #2384: when the test runs
        // unprivileged, pid 1 is foreign-owned and kill(1, 0) fails with
        // EPERM — that must still answer alive; as root the probe succeeds
        // outright. Either way the assertion holds.
        #[cfg(unix)]
        {
            assert!(
                is_process_alive(1),
                "pid 1 exists and must read as alive (EPERM is not death)"
            );
        }
    }

    #[test]
    fn test_serve_info_struct() {
        use iroh_base::SecretKey;

        let key = SecretKey::generate();
        let node_id = key.public();

        let info = ServeInfo {
            node_id: node_id.to_string(),
            pid: 12345,
            addrs: vec!["127.0.0.1:8080".to_owned(), "[::1]:8080".to_owned()],
            web_port: Some(3000),
            web_bindings: vec!["127.0.0.1:3000".to_owned()],
        };

        assert_eq!(info.node_id, node_id.to_string());
        assert_eq!(info.addrs.len(), 2);
        assert_eq!(info.addrs[0], "127.0.0.1:8080");
        assert_eq!(info.pid, 12345);
        assert_eq!(info.web_port, Some(3000));
    }

    #[test]
    fn test_serve_info_clone() {
        use iroh_base::SecretKey;

        let key = SecretKey::generate();
        let node_id = key.public();
        let info = ServeInfo {
            node_id: node_id.to_string(),
            pid: 99,
            addrs: vec!["127.0.0.1:8080".to_owned()],
            web_port: None,
            web_bindings: Vec::new(),
        };

        let cloned = info.clone();
        assert_eq!(cloned.node_id, info.node_id);
        assert_eq!(cloned.addrs, info.addrs);
        assert_eq!(cloned.pid, info.pid);
        assert_eq!(cloned.web_port, info.web_port);
    }

    #[test]
    fn test_serve_info_json_roundtrip() {
        let info = ServeInfo {
            node_id: "abc123".to_owned(),
            pid: 42,
            addrs: vec!["127.0.0.1:8080".to_owned(), "[::1]:9090".to_owned()],
            web_port: Some(3001),
            web_bindings: vec!["127.0.0.1:3001".to_owned(), "[::1]:3001".to_owned()],
        };

        let json = serde_json::to_string(&info).unwrap();
        let parsed: ServeInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.node_id, "abc123");
        assert_eq!(parsed.pid, 42);
        assert_eq!(parsed.addrs.len(), 2);
        assert_eq!(parsed.web_port, Some(3001));
    }

    #[test]
    fn test_serve_info_json_no_web_port() {
        let info = ServeInfo {
            node_id: "def456".to_owned(),
            pid: 100,
            addrs: vec!["127.0.0.1:5555".to_owned()],
            web_port: None,
            web_bindings: Vec::new(),
        };

        let json = serde_json::to_string(&info).unwrap();
        let parsed: ServeInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.web_port, None);
    }

    #[test]
    fn test_serve_info_json_pre_bindings_lock_still_parses() {
        // Lock files written before #2145 have no `web_bindings` key;
        // the serde default must keep them readable.
        let parsed: ServeInfo = serde_json::from_str(
            r#"{"node_id":"abc","pid":7,"addrs":["127.0.0.1:8080"],"web_port":3001}"#,
        )
        .unwrap();
        assert_eq!(parsed.web_port, Some(3001));
        assert!(parsed.web_bindings.is_empty());
    }

    #[test]
    fn test_resolve_web_bindings_legacy_fallback() {
        let bindings =
            resolve_web_bindings(&[], false, false, "127.0.0.1".parse().unwrap(), 3000, &[])
                .unwrap();
        assert_eq!(
            bindings,
            vec![WebBinding::explorer("127.0.0.1:3000".parse().unwrap())]
        );
    }

    #[test]
    fn test_resolve_web_bindings_flags_win_over_config() {
        let flags: Vec<SocketAddr> = vec![
            "127.0.0.1:4001".parse().unwrap(),
            "[::1]:4002".parse().unwrap(),
        ];
        let config = vec!["127.0.0.1:3000".to_owned(), "127.0.0.1:3000".to_owned()];
        let bindings = resolve_web_bindings(
            &flags,
            false,
            false,
            "127.0.0.1".parse().unwrap(),
            3000,
            &config,
        )
        .unwrap();
        assert_eq!(bindings.len(), 2);
        assert_eq!(
            bindings[0].socket_addr(),
            "127.0.0.1:4001".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(
            bindings[1].socket_addr(),
            "[::1]:4002".parse::<SocketAddr>().unwrap()
        );
    }

    #[test]
    fn test_resolve_web_bindings_rejects_duplicate_flags() {
        let duplicate: SocketAddr = "127.0.0.1:4001".parse().unwrap();
        let err = resolve_web_bindings(
            &[duplicate, duplicate],
            false,
            false,
            "127.0.0.1".parse().unwrap(),
            3000,
            &[],
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("127.0.0.1:4001"), "{err}");
        assert!(err.contains("--web-listen"), "{err}");
    }

    #[test]
    fn test_resolve_web_bindings_config_only() {
        let config = vec!["127.0.0.1:3000".to_owned(), "192.0.2.10:3001".to_owned()];
        let bindings = resolve_web_bindings(
            &[],
            false,
            false,
            "127.0.0.1".parse().unwrap(),
            9999,
            &config,
        )
        .unwrap();
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[1].socket_addr().port(), 3001);
    }

    #[test]
    fn test_resolve_web_bindings_rejects_duplicate_config() {
        let config = vec!["127.0.0.1:3000".to_owned(), "127.0.0.1:3000".to_owned()];
        let err = resolve_web_bindings(
            &[],
            false,
            false,
            "127.0.0.1".parse().unwrap(),
            3000,
            &config,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("127.0.0.1:3000"), "{err}");
        assert!(err.contains("[web_bindings]"), "{err}");
        assert!(err.contains("defaults.conf"), "{err}");
    }

    #[test]
    fn test_resolve_web_bindings_allows_repeated_port_zero() {
        let duplicate: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let from_flags = resolve_web_bindings(
            &[duplicate, duplicate],
            false,
            false,
            "127.0.0.1".parse().unwrap(),
            3000,
            &[],
        )
        .unwrap();
        assert_eq!(from_flags.len(), 2);

        let config = vec!["127.0.0.1:0".to_owned(), "127.0.0.1:0".to_owned()];
        let from_config = resolve_web_bindings(
            &[],
            false,
            false,
            "127.0.0.1".parse().unwrap(),
            3000,
            &config,
        )
        .unwrap();
        assert_eq!(from_config.len(), 2);
    }

    #[test]
    fn test_resolve_web_bindings_preserves_ipv6_scope_id() {
        let scoped_addr = SocketAddr::V6(std::net::SocketAddrV6::new(
            "fe80::1".parse().unwrap(),
            4001,
            0,
            3,
        ));
        let from_flags = resolve_web_bindings(
            &[scoped_addr],
            false,
            false,
            "127.0.0.1".parse().unwrap(),
            3000,
            &[],
        )
        .unwrap();
        assert_eq!(from_flags[0].socket_addr(), scoped_addr);
        assert_eq!(
            from_flags[0].with_port(4100).to_string(),
            "[fe80::1%3]:4100"
        );

        let from_config = resolve_web_bindings(
            &[],
            false,
            false,
            "127.0.0.1".parse().unwrap(),
            3000,
            &[scoped_addr.to_string()],
        )
        .unwrap();
        assert_eq!(from_config[0].socket_addr(), scoped_addr);
    }

    #[test]
    fn test_resolve_web_bindings_conflict_names_the_flag() {
        let flags: Vec<SocketAddr> = vec!["127.0.0.1:4001".parse().unwrap()];
        let err =
            resolve_web_bindings(&flags, true, false, "127.0.0.1".parse().unwrap(), 3000, &[])
                .unwrap_err();
        assert!(err.to_string().contains("--bind"), "{err}");
        let err =
            resolve_web_bindings(&flags, false, true, "127.0.0.1".parse().unwrap(), 3000, &[])
                .unwrap_err();
        assert!(err.to_string().contains("--port"), "{err}");
    }

    #[test]
    #[test]
    fn test_web_content_target_program_rejects_empty_world() {
        // #2146 phase 1: a program target is a world name; an empty
        // one can never resolve, so it fails at construction/parse.
        assert!(WebContentTarget::program("").is_err());
        assert!(WebContentTarget::program("   ").is_err());
        assert!(WebContentTarget::parse("program:").is_err());
        assert!(WebContentTarget::parse("program:  ").is_err());
        assert_eq!(
            WebContentTarget::program(" arena ").unwrap(),
            WebContentTarget::Program("arena".to_owned())
        );
        assert_eq!(
            WebContentTarget::parse("program:arena").unwrap().describe(),
            "program:arena"
        );
        assert_eq!(
            WebContentTarget::parse("explorer").unwrap(),
            WebContentTarget::Explorer
        );
        assert!(WebContentTarget::parse("document:readme").is_err());
    }

    #[test]
    fn test_resolve_web_bindings_config_program_target() {
        // #2146 phase 1: a [web_bindings] line may carry its content
        // target inside the line; plain lines stay Explorer, and the
        // two surfaces at different addresses resolve independently.
        let config = vec![
            "127.0.0.1:3000".to_owned(),
            "127.0.0.1:3001 program:arena".to_owned(),
        ];
        let bindings = resolve_web_bindings(
            &[],
            false,
            false,
            "127.0.0.1".parse().unwrap(),
            9999,
            &config,
        )
        .unwrap();
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[0].target(), &WebContentTarget::Explorer);
        assert_eq!(
            bindings[1].target(),
            &WebContentTarget::Program("arena".to_owned())
        );
        // Stable identities differ by target as well as by address.
        assert_eq!(bindings[0].identity(), "explorer@127.0.0.1:3000");
        assert_eq!(bindings[1].identity(), "program:arena@127.0.0.1:3001");
    }

    #[test]
    fn test_resolve_web_bindings_config_rejects_bad_target() {
        // #2146 phase 1: a malformed target fails startup naming the
        // line, like a malformed address does.
        for line in [
            "127.0.0.1:3000 program:",
            "127.0.0.1:3000 document:readme",
            "127.0.0.1:3000 explorer extra",
        ] {
            let config = vec![line.to_owned()];
            let err = resolve_web_bindings(
                &[],
                false,
                false,
                "127.0.0.1".parse().unwrap(),
                3000,
                &config,
            )
            .unwrap_err()
            .to_string();
            assert!(err.contains("[web_bindings]"), "{err}");
            assert!(err.contains("127.0.0.1:3000"), "{err}");
        }
    }

    #[test]
    fn test_resolve_web_bindings_malformed_config_line_is_named() {
        let config = vec!["127.0.0.1:3000".to_owned(), "not-an-addr".to_owned()];
        let err = resolve_web_bindings(
            &[],
            false,
            false,
            "127.0.0.1".parse().unwrap(),
            3000,
            &config,
        )
        .unwrap_err();
        assert!(err.to_string().contains("not-an-addr"), "{err}");
        assert!(err.to_string().contains("[web_bindings]"), "{err}");
    }

    // Integration tests for lock file functions require file system access
    // and are tested via the integration test suite

    // #5119 slice 1: serve loads a ServerConfig through --server-config.

    #[cfg(feature = "world")]
    fn serve_config_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("id-serve-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(feature = "world")]
    #[test]
    fn start_server_config_applies_provisioning() {
        let dir = serve_config_dir("applies");
        let path = dir.join("server.toml");
        // Real Ed25519 public keys: the parser rejects byte patterns
        // that are not valid Edwards points.
        let id = |n: u8| {
            ed25519_dalek::SigningKey::from_bytes(&[n; 32])
                .verifying_key()
                .to_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        std::fs::write(
            &path,
            format!(
                "[[users]]\nkey = \"ann\"\nid = \"{}\"\n\n[[users]]\nkey = \"bo\"\nid = \"{}\"\n\n\
                 [[groups]]\nkey = \"team\"\nname = \"Team\"\ncreator = \"ann\"\n\n\
                 [[groups.members]]\nuser = \"bo\"\nrole = \"write\"\n",
                id(1),
                id(2)
            ),
        )
        .unwrap();
        let started = start_server_config(&path).unwrap();
        assert_eq!(started.report.users_created, 2);
        assert_eq!(started.report.groups_created, 1);
        assert_eq!(started.report.members_added, 1);
        // A second run over the same state creates nothing: the
        // Provisioned value from the first run is what serve keeps.
        let mut provisioned = started.provisioned;
        let again = provisioned.apply(&started.config).unwrap();
        assert_eq!(again, crate::server_config::Report::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "world")]
    #[test]
    fn start_server_config_persisted_survives_a_restart() {
        let dir = serve_config_dir("persisted");
        let path = dir.join("server.toml");
        let groups_log = dir.join("groups.jsonl");
        // Real Ed25519 public keys, as in
        // start_server_config_applies_provisioning.
        let id = |n: u8| {
            ed25519_dalek::SigningKey::from_bytes(&[n; 32])
                .verifying_key()
                .to_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        std::fs::write(
            &path,
            format!(
                "[[users]]\nkey = \"ann\"\nid = \"{}\"\n\n[[users]]\nkey = \"bo\"\nid = \"{}\"\n\n\
                 [[groups]]\nkey = \"team\"\nname = \"Team\"\ncreator = \"ann\"\n\n\
                 [[groups.members]]\nuser = \"bo\"\nrole = \"write\"\n",
                id(1),
                id(2)
            ),
        )
        .unwrap();
        let started = start_server_config_persisted(&path, &groups_log).unwrap();
        assert_eq!(started.report.users_created, 2);
        assert_eq!(started.report.groups_created, 1);
        assert_eq!(started.report.members_added, 1);
        drop(started);
        // A restart over the same persisted state loads the accounts
        // and group events back and creates nothing: no recreations,
        // no new-creation reports (#5119 slice 3 acceptance).
        let restarted = start_server_config_persisted(&path, &groups_log).unwrap();
        assert_eq!(restarted.report, crate::server_config::Report::default());
        assert!(restarted.provisioned.directory().account(&id(1)).is_some());
        assert!(restarted.provisioned.directory().account(&id(2)).is_some());
        // The create event plus the one member add.
        assert_eq!(restarted.provisioned.events().len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #5119 (PR #8356 review): the accounts a server config
    /// provisions must land in the directory the serving world
    /// actually loads (`.id-worlds/<name>/directory.jsonl`), not
    /// only in the directory beside the groups log.
    #[cfg(feature = "world")]
    #[test]
    fn seed_world_directory_provisions_configured_accounts() {
        let dir = serve_config_dir("world-seed");
        let path = dir.join("server.toml");
        // Real Ed25519 public keys, as in
        // start_server_config_applies_provisioning.
        let id = |n: u8| {
            ed25519_dalek::SigningKey::from_bytes(&[n; 32])
                .verifying_key()
                .to_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        std::fs::write(
            &path,
            format!(
                "[[users]]\nkey = \"ann\"\nid = \"{}\"\n\n[[users]]\nkey = \"bo\"\nid = \"{}\"\n",
                id(1),
                id(2)
            ),
        )
        .unwrap();
        let config = crate::server_config::ServerConfig::load(&path).unwrap();
        let world = dir.join("world");
        let seeded = seed_world_directory(&config, &world).unwrap();
        assert_eq!(seeded.users_created, 2);
        // The world's own directory now resolves both configured
        // accounts, as world_store::open_world would load it.
        let directory = crate::directory::Directory::open(&world.join("directory.jsonl")).unwrap();
        assert!(directory.account(&id(1)).is_some());
        assert!(directory.account(&id(2)).is_some());
        drop(directory);
        // A restart seeds nothing again.
        let again = seed_world_directory(&config, &world).unwrap();
        assert_eq!(again, crate::server_config::Report::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "world")]
    #[test]
    fn start_server_config_fails_startup_on_a_malformed_file() {
        let dir = serve_config_dir("malformed");
        let path = dir.join("server.toml");
        std::fs::write(&path, "[[users]]\nkey = \"ann\"\nid = \"not-hex\"\n").unwrap();
        let err = start_server_config(&path).unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains("server config"), "{message}");
        assert!(message.contains(&path.display().to_string()), "{message}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "world")]
    #[test]
    fn start_server_config_fails_startup_on_a_missing_file() {
        let dir = serve_config_dir("missing");
        let path = dir.join("absent.toml");
        let err = start_server_config(&path).unwrap_err();
        assert!(format!("{err:#}").contains("server config"), "{err:#}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "world")]
    #[test]
    fn serve_pull_policy_defaults_to_open_without_a_config() {
        assert_eq!(
            serve_pull_policy(None),
            crate::pull_policy::PullPolicy::Open
        );
    }

    /// #5119: the live event-sync gate takes its policy from the loaded
    /// config's `[policy] pull_policy`, not from the code default.
    #[cfg(feature = "world")]
    #[test]
    fn serve_pull_policy_reads_the_configured_policy_table() {
        let dir = serve_config_dir("pull-policy");
        let path = dir.join("server.toml");
        std::fs::write(&path, "[policy]\npull_policy = \"members\"\n").unwrap();
        let started = start_server_config(&path).unwrap();
        assert_eq!(
            serve_pull_policy(Some(&started)),
            crate::pull_policy::PullPolicy::Members
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #8487: without a server config, seitan admission serves under
    /// the code default window (5 minutes), exactly as before.
    #[cfg(feature = "world")]
    #[test]
    fn serve_freshness_window_defaults_without_a_config() {
        assert_eq!(
            serve_freshness_window(None),
            crate::seitan::DEFAULT_WINDOW
        );
    }

    /// #8487: the live admission path takes its window from the
    /// loaded config's `[policy] freshness_window_seconds`, not from
    /// the code default.
    #[cfg(feature = "world")]
    #[test]
    fn serve_freshness_window_reads_the_configured_window() {
        let dir = serve_config_dir("freshness-window");
        let path = dir.join("server.toml");
        std::fs::write(&path, "[policy]\nfreshness_window_seconds = 60\n").unwrap();
        let started = start_server_config(&path).unwrap();
        assert_eq!(
            serve_freshness_window(Some(&started)),
            std::time::Duration::from_secs(60)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // #2247: the loaded [budgets] must reach the world protocol serve
    // registers, not just the parser.

    #[cfg(feature = "world")]
    #[tokio::test]
    async fn serve_world_protocol_enforces_configured_budgets() {
        let dir = serve_config_dir("budgets");
        let path = dir.join("server.toml");
        let ann = ed25519_dalek::SigningKey::from_bytes(&[1; 32])
            .verifying_key()
            .to_bytes();
        let ann_hex = ann.iter().map(|b| format!("{b:02x}")).collect::<String>();
        std::fs::write(
            &path,
            format!(
                "[[users]]\nkey = \"ann\"\nid = \"{ann_hex}\"\n\n[budgets]\nper_peer_bytes_per_hour = 1000\n\n[[budgets.counterparty]]\npeer = \"ann\"\nbytes_per_window = 500\nwindow_seconds = 60\n"
            ),
        )
        .unwrap();
        let started = start_server_config(&path).unwrap();
        assert_ne!(
            *started.config.budgets(),
            crate::abuse::BudgetPolicy::default()
        );
        let world = crate::world::WorldHandle::spawn(
            crate::world::WorldCore::new("lobby", crate::world::WorldLimits::default()).unwrap(),
        );
        let service = crate::world_session::WorldService::new(world, None);
        let protocol = world_protocol_for(service.into(), [9; 32], Some(&started));
        let handle = protocol.abuse_handle();
        let mut evaluator = handle.lock().unwrap();
        // The configured policy — not the default — is the evaluator's.
        assert_eq!(evaluator.budgets(), Some(started.config.budgets()));
        // Ann's counterparty row replaced her per-peer limits.
        let limits = started.config.budgets().limits_for_peer(&ann);
        assert_eq!(limits.len(), 1);
        assert_eq!(limits[0].bytes_per_window, 500);
        // A charge beyond the configured cap is refused as a budget
        // refusal (the frame is dropped; the peer is not disconnected).
        let refusal = evaluator
            .check_budget(crate::abuse::Direction::Inbound, ann, None, 600, 1_000)
            .unwrap_err();
        assert_eq!(refusal.reason, crate::abuse::RefusalReason::BudgetExhausted);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "world")]
    #[tokio::test]
    async fn serve_world_protocol_defaults_budgets_without_config() {
        let world = crate::world::WorldHandle::spawn(
            crate::world::WorldCore::new("lobby", crate::world::WorldLimits::default()).unwrap(),
        );
        let service = crate::world_session::WorldService::new(world, None);
        let protocol = world_protocol_for(service.into(), [9; 32], None);
        let handle = protocol.abuse_handle();
        let evaluator = handle.lock().unwrap();
        assert_eq!(
            evaluator.budgets(),
            Some(&crate::abuse::BudgetPolicy::default())
        );
    }

    #[test]
    fn serve_server_config_flag_defaults_to_none() {
        use clap::Parser as _;
        let cli = crate::cli::Cli::try_parse_from(["id", "serve"]).unwrap();
        let Some(crate::cli::Command::Serve(args)) = cli.command else {
            panic!("expected the serve command");
        };
        let crate::cli::ServeArgs { server_config, .. } = args;
        assert!(server_config.is_none());
        let cli = crate::cli::Cli::try_parse_from(["id", "serve", "--server-config", "ops.toml"])
            .unwrap();
        let Some(crate::cli::Command::Serve(args)) = cli.command else {
            panic!("expected the serve command");
        };
        let crate::cli::ServeArgs { server_config, .. } = args;
        assert_eq!(server_config, Some(PathBuf::from("ops.toml")));
    }
}
