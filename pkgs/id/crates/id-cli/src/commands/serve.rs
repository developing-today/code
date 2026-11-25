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
//! │  Lock File: .iroh-serve.lock                                 │
//! │  - Node ID, PID, Socket addresses                            │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Lock File Protocol
//!
//! The serve process creates a lock file (`.iroh-serve.lock`) containing:
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
use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
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
use crate::peer_announcement::SignedPeerAnnouncement;
use crate::protocol::{MetaProtocol, MetaRequest, MetaResponse};
use crate::{KEY_FILE, META_ALPN, SERVE_LOCK, STORE_PATH};
use id_core::discovery::{
    ANNOUNCE_INTERVAL, PeerAnnouncement, PeerDiscovery, STALE_CHECK_INTERVAL, STALE_THRESHOLD,
    defaults, resolve_config,
};
use id_core::waiting::AuthorKey;
use id_events::groups::{Key as GroupKey, Projection};
use id_groups::groups::log::{FILE_NAME as GROUPS_FILE_NAME, GroupLog};
use id_groups::groups::warnings::{Evaluator, Observation, Warning};
use id_store::store::{load_or_create_keypair, open_store};
use id_store::tags::TagStore;
#[cfg(feature = "web")]
use id_sync::forwarding_sink::ForwardingSink;
#[cfg(feature = "web")]
use id_sync::graph_frontier::GraphFrontier;
#[cfg(feature = "web")]
use id_sync::relay_gate::RelayGate;
#[cfg(feature = "web")]
use id_sync::relay_ingest::{CHAINED_RELAY_ALPN, ChainedRelayProtocol};

mod gossip;
mod startup;

use gossip::{
    GROUP_WARNING_INTERVAL, GroupWarningWatcher, MAX_STALE_PROBES_PER_PASS,
    evaluate_group_warnings, presence_heartbeat_period, receive_presence_heartbeat,
    run_gossip_loop, select_stale_probe_batch, unix_now,
};
#[cfg(feature = "web")]
use startup::build_blob_gate;
#[cfg(feature = "world")]
pub use startup::{
    StartedServerConfig, seed_world_directory, start_server_config, start_server_config_persisted,
};
#[cfg(feature = "world")]
use startup::{
    serve_freshness_window, serve_pull_policy, server_config_directory_path, world_protocol_for,
};

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
    /// Process start time of the serve instance (Linux: /proc/<pid>/stat
    /// field 22, clock ticks since boot), tying the lock to the exact
    /// process incarnation that wrote it (#3040). Defaults to None so
    /// lock files written before this field still parse; None also on
    /// platforms where the start time cannot be read — those locks fall
    /// back to PID-existence liveness only.
    #[serde(default)]
    pub start_time: Option<u64>,
}

/// Ownership token for the discovery lock written by one `id serve` process.
///
/// The serialized lock contents are retained so shutdown can remove the file
/// only if it still contains the record published by this serve instance.
#[derive(Debug)]
pub struct ServeLock {
    contents: String,
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
    if !is_process_alive(info.pid, info.start_time) {
        // Only remove the file we inspected. The shared sidecar lock protects
        // the ownership check and unlink from concurrent publication.
        remove_stale_serve_lock(Path::new(SERVE_LOCK), &contents);
        return None;
    }

    Some(info)
}

/// Opens the stable sidecar used to serialize all mutations of a serve lock.
/// The sidecar is deliberately never removed: unlinking a locked sidecar can
/// let processes lock different inodes and enter the critical section at once.
fn lock_serve_lock(path: &Path) -> Result<File> {
    let mut guard_name = path.as_os_str().to_os_string();
    guard_name.push(".guard");
    let guard = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(PathBuf::from(guard_name))?;
    guard.lock()?;
    Ok(guard)
}

/// Removes a stale lock only when the contents still match the inspected
/// snapshot. Publication and cleanup share a stable OS file lock, so checking
/// ownership and unlinking cannot expose a replacement lock path as vacant.
fn remove_stale_serve_lock(path: &Path, expected_contents: &str) {
    let _guard = match lock_serve_lock(path) {
        Ok(guard) => guard,
        Err(error) => {
            warn!(?error, ?path, "could not lock serve lock for stale cleanup");
            return;
        }
    };
    remove_stale_serve_lock_locked(path, expected_contents);
}

fn remove_stale_serve_lock_locked(path: &Path, expected_contents: &str) {
    match fs::read_to_string(path) {
        Ok(contents) if contents == expected_contents => {
            if let Err(error) = fs::remove_file(path) {
                warn!(?error, ?path, "could not remove stale serve lock");
            }
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => warn!(
            ?error,
            ?path,
            "could not inspect serve lock for stale cleanup"
        ),
    }
}

mod process;

use process::get_process_start_time;
pub use process::is_process_alive;
#[cfg(test)]
use process::parse_proc_stat_start_time;

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
) -> Result<ServeLock> {
    let info = ServeInfo {
        node_id: node_id.to_string(),
        pid: std::process::id(),
        addrs: addrs.iter().map(ToString::to_string).collect(),
        web_port,
        web_bindings,
        start_time: get_process_start_time(std::process::id()),
    };
    let contents = serde_json::to_string_pretty(&info)?;
    let lock_path = Path::new(SERVE_LOCK);
    let _guard = lock_serve_lock(lock_path)?;
    publish_serve_lock_reclaiming_stale_locked(lock_path, &contents)?;
    Ok(ServeLock { contents })
}

/// Atomically publishes a lock only if no serve process already owns it.
fn publish_serve_lock(lock_path: &Path, contents: &str) -> Result<()> {
    let _guard = lock_serve_lock(lock_path)?;
    publish_serve_lock_locked(lock_path, contents)
}

fn publish_serve_lock_locked(lock_path: &Path, contents: &str) -> Result<()> {
    let parent = lock_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::Builder::new()
        .prefix(".iroh-serve-lock-")
        .tempfile_in(parent)?;
    temporary.write_all(contents.as_bytes())?;
    temporary
        .persist_noclobber(lock_path)
        .map(|_| ())
        .map_err(|error| error.error)?;
    Ok(())
}

fn publish_serve_lock_reclaiming_stale_locked(lock_path: &Path, contents: &str) -> Result<()> {
    if let Ok(existing) = fs::read_to_string(lock_path) {
        if let Ok(existing_info) = serde_json::from_str::<ServeInfo>(&existing) {
            if !is_process_alive(existing_info.pid, existing_info.start_time) {
                remove_stale_serve_lock_locked(lock_path, &existing);
            }
        }
    }
    publish_serve_lock_locked(lock_path, contents)
}

/// Removes this serve instance's lock file, if it is still the current owner.
///
/// Called during graceful shutdown. A replacement lock published by another
/// serve process is preserved.
pub async fn remove_serve_lock(lock: ServeLock) -> Result<()> {
    remove_serve_lock_at(Path::new(SERVE_LOCK), &lock.contents);
    Ok(())
}

fn remove_serve_lock_at(lock_path: &Path, expected_contents: &str) {
    remove_stale_serve_lock(lock_path, expected_contents);
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
    /// Explicitly enable gossip, overriding a build-time disabled default.
    pub gossip: bool,
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
    /// Serve the program gallery stored in this directory (#10003): over
    /// Iroh as a program registry (#8281), and over the web with `--web`.
    pub gallery_dir: Option<std::path::PathBuf>,
    /// Accept registry publishes from any peer over Iroh.
    pub gallery_accept_publish: bool,
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
    /// Build a `Program` target, validating the world name against
    /// the world-name rule ([`id_worlds::world_hub::validate_world_name`]):
    /// a program target names a world, so it must be a name a world
    /// can bear -- 1 to 64 bytes of `a-z`, `0-9`, `-` and `_`, never
    /// a path segment. The stored name is the trimmed one.
    ///
    /// # Errors
    ///
    /// Fails on an empty or all-whitespace world name, or on any
    /// name the world-name rule rejects.
    pub fn program(world: impl Into<String>) -> Result<Self> {
        let world = world.into();
        let world = world.trim();
        ensure!(
            !world.is_empty(),
            "program target world name must not be empty (expected program:<world-name>)"
        );
        id_worlds::world_hub::validate_world_name(world)
            .map_err(|e| anyhow::anyhow!("invalid program target world name {world:?}: {e}"))?;
        Ok(Self::Program(world.to_owned()))
    }

    /// Parse one target spec: `explorer` or `program:<world-name>`.
    /// This is the grammar a target is expressed in wherever a
    /// binding is written as text (the `web_bindings` entries in
    /// `defaults.json`), so a target always travels inside its own
    /// binding's spec -- never as a separate repeated flag paired
    /// with listener flags by position (the design forbids that).
    ///
    /// # Errors
    ///
    /// Fails on an unknown spec or a program world name the
    /// world-name rule rejects, naming the spec.
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
    /// Fails on an empty or all-whitespace world name, or on any
    /// name the world-name rule rejects.
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
/// 2. Else the `web_bindings` list from `defaults.json`, parsed here
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
                    anyhow::anyhow!("invalid web_bindings entry {line:?} in defaults.json: {e}")
                })
            })
            .collect();
        let bindings = bindings?;
        ensure_unique_web_listener_addresses(
            bindings.iter().map(WebBinding::socket_addr),
            "web_bindings in defaults.json",
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
    id_worlds::world_hub::validate_world_name(world_name)?;
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
) -> Result<id_core::access::AccessPolicy> {
    use id_core::access::{AccessPolicy, load_allowed_nodes};
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
) -> Result<Option<id_worlds::world_compile::Compiler>> {
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
    let compiler = id_worlds::world_compile::Compiler::new(platform, roc_bin)?;
    info!(
        platform = %compiler.platform_dir.display(),
        roc = %compiler.roc_bin,
        "world: on-the-fly compilation enabled"
    );
    // The shared compiled-world cache (design #5965 §2/§4, P1 in
    // #6235) is opt-in: ID_WORLD_COMPILE_CACHE_DIR names its root,
    // ID_WORLD_COMPILE_CACHE_LRU_BYTES its tier-1 budget (default
    // 2 GiB). Unset, compiles behave exactly as before the cache.
    let Some(cache_dir) = std::env::var("ID_WORLD_COMPILE_CACHE_DIR")
        .ok()
        .filter(|dir| !dir.is_empty())
    else {
        return Ok(Some(compiler));
    };
    let lru_bytes = std::env::var("ID_WORLD_COMPILE_CACHE_LRU_BYTES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(id_worlds::world_cache::DEFAULT_LRU_BYTES);
    let cache = id_worlds::world_cache::WorldCache::open(PathBuf::from(&cache_dir), lru_bytes)?;
    info!(
        cache_dir,
        lru_bytes, "world: shared compiled-world cache enabled"
    );
    Ok(Some(compiler.with_cache(std::sync::Arc::new(cache))))
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
) -> Result<Option<Arc<dyn id_directory::directory_mail::MailSink>>> {
    use anyhow::Context as _;
    use id_directory::directory_mail::{CommandSink, MailSink, OutboxSink, SmtpSink};
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
/// ceiling ([`id_caps::world_caps::default_ceiling_names`]) when no list is
/// given — providers whose manifest default is `Excluded` do not join a
/// server by registration alone (#838). Like a world ceiling, it names
/// capabilities; `*` is refused.
#[cfg(feature = "world")]
fn allowed_caps(names: Option<&[String]>) -> Result<std::collections::BTreeSet<String>> {
    match names {
        None => Ok(id_caps::world_caps::default_ceiling_names()),
        Some(names) => {
            id_caps::world_caps::validate_ceiling_names(names)?;
            Ok(names.iter().cloned().collect())
        }
    }
}

#[cfg(feature = "world")]
struct ServeWorlds {
    ephemeral: bool,
    admin_token: Option<String>,
    boundary_config: Option<crate::server_config::Boundaries>,
    boundary_server: String,
    blobs: iroh_blobs::api::Store,
    docs: Docs,
    checkpoint_every: u64,
    default_world: String,
    default_module: Option<PathBuf>,
    grant_on_use: bool,
    server_caps: std::collections::BTreeSet<String>,
    compiler: Option<id_worlds::world_compile::Compiler>,
    native: bool,
    source_only: bool,
    trusted_module_keys: Vec<[u8; 32]>,
    runtime: id_caps::world_limits::RuntimeLimits,
    mail: Option<Arc<dyn id_directory::directory_mail::MailSink>>,
}

#[cfg(feature = "world")]
impl id_worlds::world_hub::WorldOpener for ServeWorlds {
    fn open<'a>(&'a self, name: &'a str, create: bool) -> id_worlds::world_hub::OpenFuture<'a> {
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
            .filter(|name| id_worlds::world_hub::validate_world_name(name).is_ok())
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
    async fn open_world(&self, name: &str) -> Result<id_worlds::world_session::WorldService> {
        use id_worlds::world::{WorldCore, WorldHandle, WorldLimits};
        use id_worlds::world_session::WorldService;

        let limits = WorldLimits {
            checkpoint_every: self.checkpoint_every,
            grant_on_use: self.grant_on_use,
            server_caps: self.server_caps.clone(),
            ..WorldLimits::default()
        };
        let runtime = self.runtime;
        // Until the serving process has live placement evidence sources, an
        // enabled boundary receives no claims and therefore fails closed.
        let boundary_gate = self.boundary_config.as_ref().map(|boundaries| {
            Arc::new(
                crate::boundary_gate::ServerBoundaryGate::without_hop_evidence(
                    boundaries.clone(),
                    self.boundary_server.clone(),
                    Some(name.to_owned()),
                    Arc::new(crate::boundary_evidence::AppEvidenceVerifier::default()),
                ),
            ) as Arc<dyn id_directory::directory::BoundaryGate>
        });
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
            id_worlds::world_records::RecordsStore::open(&self.docs, namespace_file.as_deref())
                .await?;
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
            let mut core = WorldCore::new(name, limits)?;
            if let Some(gate) = boundary_gate {
                core.directory_mut().set_boundary_gate(Arc::clone(&gate));
                core.set_world_boundary_gate(gate);
            }
            make(WorldHandle::spawn_with_device_manifest(
                core,
                Box::<id_worlds::world::EmptyWorldProgram>::default(),
                Some(WorldHandle::host_device_manifest()),
            ))
            .with_records_store(records)
        } else {
            let (service, report) = id_worlds::world_store::open_world_with_directory_gate(
                &dir,
                name,
                limits,
                runtime,
                boundary_gate,
                make,
            )
            .await?;
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
        service: &id_worlds::world_session::WorldService,
        path: &std::path::Path,
    ) -> Result<()> {
        let metadata = tokio::fs::metadata(path).await?;
        ensure!(
            usize::try_from(metadata.len())
                .is_ok_and(|len| len <= id_worlds::world_session::MAX_WORLD_MODULE_BYTES),
            "world module exceeds the configured module size limit"
        );
        let wasm = tokio::fs::read(path).await?;
        let hash = id_worlds::world_session::module_hash(&wasm);
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
        _service: &id_worlds::world_session::WorldService,
        _path: &std::path::Path,
    ) -> Result<()> {
        anyhow::bail!("--world-module requires a build with the `sandbox` feature");
    }
}

/// The group log the serve warning watcher reads: the global
/// `--groups-log` option when given, else the default file name —
/// resolved with the group commands' semantics (#2778; serve used
/// to hardcode the default and ignore the option).
/// The transport set this node currently advertises (#8214
/// slice 4): its endpoint's direct UDP socket addresses and its
/// home relay, in the #5671 fallback order (direct first, relay
/// second). Recomputed from the live endpoint address, so a
/// watcher re-minting on change always signs the current set.
fn ad_transports(
    endpoint: &iroh::endpoint::Endpoint,
) -> Vec<id_core::transport_advertisement::TransportEntry> {
    use id_core::transport_advertisement::{TransportEntry, TransportKind};
    let mut out = Vec::new();
    for addr in endpoint.addr().addrs {
        match addr {
            iroh::TransportAddr::Ip(socket) => out.push(TransportEntry {
                kind: TransportKind::Udp,
                endpoint: socket.to_string(),
                priority: 10,
                broker: None,
            }),
            iroh::TransportAddr::Relay(url) => out.push(TransportEntry {
                kind: TransportKind::RelayUdp,
                endpoint: url.to_string(),
                priority: 20,
                broker: None,
            }),
            _ => {}
        }
    }
    out
}

/// Unix seconds now, for advertisement minting.
fn ad_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

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
/// * `topic` - Custom gossip topic name (default: from `defaults.json` or `DEFAULT_TOPIC`)
/// * `topic_secret` - Custom shared secret for topic access control
/// * `no_default_bootstrap` - If `true`, skip default bootstrap nodes from `defaults.json`
/// * `no_default_topic` - If `true`, skip file topic/secret defaults
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

fn resolve_network_overrides(
    default_relay: id_core::connectivity::RelayPolicy,
    default_gossip: bool,
    no_relay: bool,
    gossip: bool,
    no_gossip: bool,
) -> (id_core::connectivity::RelayPolicy, bool) {
    let relay = if no_relay {
        id_core::connectivity::RelayPolicy::Disabled
    } else {
        default_relay
    };
    let gossip = gossip || (default_gossip && !no_gossip);
    (relay, gossip)
}

pub async fn cmd_serve(opts: ServeOptions) -> Result<()> {
    let ServeOptions {
        ephemeral,
        server_config,
        groups_log,
        no_relay,
        no_gossip,
        gossip,
        web,
        port,
        bootstrap,
        topic,
        topic_secret,
        no_default_bootstrap,
        no_default_topic,
        no_mdns,
        dht,
        iroh_port,
        bind,
        web_listen,
        bind_explicit,
        port_explicit,
        web_token,
        web_cookie_secure,
        gallery_dir,
        gallery_accept_publish,
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
            id_events::pull_policy::PullPolicy::Open
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
    // listeners happens below, once the web surface exists: the
    // serve-owned PublicationManager restores it through the same
    // apply path runtime publishes use (slice 3). A malformed
    // registry fails startup loudly (in Registry::load) rather than
    // restoring nothing silently.
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
    // #14226: the discovered apps' run manifests, which the web
    // surface's `POST /v1/runs` loader checks app runs against.
    #[cfg(feature = "web")]
    let run_apps = crate::app_manifest::run_apps(&app_discovery.apps);
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
    match id_directory::restored_key::pending(
        id_directory::restored_key::RESTORED_KEY_FILE,
        id_directory::keyset_file::KEYSET_FILE,
    ) {
        Ok(Some(public)) => warn!("serve: {}", id_directory::restored_key::warning(&public)),
        Ok(None) => {}
        Err(error) => warn!(%error, "serve: could not read the restore flag"),
    }

    // Who may modify this store: this node, this machine's client key (so the
    // local CLI/REPL work with no setup), `--allow-node`, and `.iroh-allowed`.
    let access = build_access_policy(node_id, &allow_node, open_writes).await?;

    let store = open_store(ephemeral).await?;
    let store_handle = store.as_store();

    let resolved_defaults = resolve_config(
        &bootstrap,
        topic.as_deref(),
        topic_secret.as_deref(),
        no_default_bootstrap,
        no_default_topic,
    )?;
    info!("{}", id_core::discovery::defaults_provenance().describe());
    let (relay_policy, gossip_enabled) = resolve_network_overrides(
        resolved_defaults.relay,
        resolved_defaults.gossip,
        no_relay,
        gossip,
        no_gossip,
    );
    // The build's defaults.json can disable relays without --no-relay, so
    // everything downstream asks the resolved policy, not the flag.
    let relays_disabled = relay_policy == id_core::connectivity::RelayPolicy::Disabled;
    let mut builder = id_core::connectivity::public_builder(relay_policy).secret_key(key.clone());
    if !no_mdns {
        builder = builder.address_lookup(MdnsAddressLookup::builder());
    }
    let dht_policy = id_sync::dht::DhtPolicy::from_server(dht);
    #[cfg(feature = "dht")]
    if dht_policy.is_on() {
        builder = id_sync::dht::with_lookup(&dht_policy, builder, id_sync::dht::mainline());
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
    // With relays disabled (`--no-relay` or the build's defaults) no relay
    // ever connects, so the wait is skipped.
    if relays_disabled {
        info!("serve startup: relays disabled, skipping home-relay wait");
    } else {
        match id_core::connectivity::await_home_relay(
            &endpoint,
            id_core::connectivity::HOME_RELAY_DEADLINE,
        )
        .await
        {
            Ok(()) => info!("serve startup: home relay connected"),
            Err(e) => warn!(
                "serve startup: proceeding without a home relay after {:?}: {e:#}",
                id_core::connectivity::HOME_RELAY_DEADLINE,
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
    let event_events: Option<Arc<tokio::sync::Mutex<id_events::local_events::LocalEvents>>> = if web
    {
        let identity_db_path = std::path::Path::new(KEY_FILE).with_file_name(".identity.redb");
        let events_dir = identity_db_path.with_file_name(id_events::local_events::DIR_NAME);
        let mut events = id_events::local_events::LocalEvents::open(
            &events_dir,
            p2panda_core::SigningKey::from_bytes(&key.to_bytes()),
        )?;
        // Publish this node's identity event if its log has none (#160, Option A):
        // its ID is the node's user subject for tags and aliases. A failure is
        // logged, never fatal — resolution falls back to the earliest event
        // (Option B) and must not gate startup on the identity event.
        match events.ensure_identity_event() {
            Ok(Some(id)) => info!("serve: published identity event {id}"),
            Ok(None) => info!("serve: identity event already in the log"),
            Err(err) => {
                warn!(
                    "serve: identity event not published ({err:#}); user subject falls back to the earliest event"
                );
            }
        }
        Some(Arc::new(tokio::sync::Mutex::new(events)))
    } else {
        None
    };

    // The blob membership gate (#173). With the event store open, serve builds the
    // gate from the group log (membership) and the store's document references
    // (which blobs each world names), and a refresh task below keeps it current.
    // Without the store there is no document source to index, so blob reads stay
    // ungated this run — push-only, as before — and serve says so at startup.
    #[cfg(feature = "web")]
    let blob_gate: Option<Arc<crate::blob_gate::RefreshingWorlds>> = match &event_events {
        Some(events) => {
            // A failed first build has no previous fold to keep: the gate
            // starts empty — fail-closed, as before — and the refresh task
            // below retries on its cadence (#13821).
            let folded = match build_blob_gate(events, &store_handle, &groups_log).await {
                Ok(folded) => folded,
                Err(error) => {
                    tracing::error!(
                        "blob gate: initial build failed: {error:#}; \
                         starting with an empty gate until a refresh succeeds"
                    );
                    crate::blob_gate::FoldedWorlds::new(
                        crate::groups::worlds::Worlds::empty(),
                        std::collections::HashMap::new(),
                    )
                }
            };
            Some(Arc::new(crate::blob_gate::RefreshingWorlds::new(folded)))
        }
        None => None,
    };
    #[cfg(not(feature = "web"))]
    let blob_gate: Option<Arc<crate::blob_gate::RefreshingWorlds>> = None;
    if blob_gate.is_none() {
        warn!(
            "blob gate (#173): inactive — the event store is only opened with the web UI, \
             so blob reads are not membership-checked this run"
        );
    }

    // iroh-blobs rejects pushes unless an event handler enables them; this one
    // allows them only from nodes the access policy lets write. With the blob
    // gate built, the gate's handler decides pushes the same way and also
    // membership-checks fetches and observes (#173).
    // The gate's local readers: the node's own key and the data directory's
    // client key — the keys the write policy already trusts automatically —
    // so the local CLI and REPL keep reading the node's own store with the
    // gate on (#173, #7181 review).
    let local_readers: std::collections::BTreeSet<EndpointId> =
        match load_or_create_keypair(crate::CLIENT_KEY_FILE).await {
            Ok(client_key) => [node_id, client_key.public()].into_iter().collect(),
            Err(error) => {
                warn!("blob gate: cannot load the client key for local reads: {error}");
                [node_id].into_iter().collect()
            }
        };
    let blobs = match &blob_gate {
        Some(gate) => BlobsProtocol::new(
            &store_handle,
            Some(crate::blob_gate::blob_events(
                Arc::clone(gate),
                crate::blob_gate::BlobPolicy::default(),
                access.clone(),
                local_readers,
            )),
        ),
        None => BlobsProtocol::new(
            &store_handle,
            Some(id_core::access::blobs_events(access.clone())),
        ),
    };

    // Every world lives in one hub, shared by the Iroh protocol and the web
    // bridge. The default world opens now so a bad journal fails startup.
    #[cfg(feature = "world")]
    let world_hub = if world {
        use id_worlds::world_hub::{HubLimits, WorldHub};
        let runtime = id_caps::world_limits::RuntimeLimits::from_args(&world_runtime)?;
        let server_caps = allowed_caps(world_allowed_caps.as_deref())?;
        let compiler = resolve_compiler(roc_bin, roc_platform)?;
        let trusted_module_keys = world_module_trust_keys
            .iter()
            .map(|encoded| {
                let bytes = id_worlds::world_session::decode_hex(encoded).ok_or_else(|| {
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
            boundary_config: started_server_config
                .as_ref()
                .map(|started| started.config.boundaries().clone()),
            boundary_server: node_id.to_string(),
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
        .map(id_worlds::world_net::WorldProtocol::abuse_handle);
    #[cfg(not(feature = "world"))]
    let abuse_handle: Option<std::sync::Arc<std::sync::Mutex<id_core::abuse::AbuseEvaluator>>> =
        None;

    let mut meta = MetaProtocol::new(
        &store_handle,
        Some(peer_discovery.clone()),
        Arc::clone(&tag_store),
        access.clone(),
        node_id,
        abuse_handle,
    );
    // Server-to-server transfers (#5345) are initiated from this handler
    // with the server's own endpoint and identity.
    // #2147: this machine's client identity is the handler's
    // designated local operator for publication control -- the
    // local CLI presents that key (an endpoint cannot connect to
    // itself, so the node's own key cannot), and holding either
    // local key file is the operator credential. Peers granted
    // store writes (--allow-node, .iroh-allowed, open writes) are
    // NOT operators.
    let local_operator = load_or_create_keypair(crate::CLIENT_KEY_FILE)
        .await?
        .public();
    if let Some(inner) = Arc::get_mut(&mut meta) {
        inner.endpoint = Some(endpoint.clone());
        inner.local_operator = Some(local_operator);
    }

    #[cfg(all(feature = "world", feature = "ssh"))]
    let (ssh_tunnel, ssh_tunnel_port) = id_worlds::world_ssh::SshTunnelProtocol::deferred();
    // #2147 slice 3b: a second handle on the meta handler, kept so
    // the publication manager built below can register its
    // operator control with the already-registered handler.
    #[cfg(feature = "web")]
    let meta_handle = Arc::clone(&meta);
    // #10003/#8281: the program gallery, from verified bundle files and
    // operator data in --gallery-dir. A file that no longer verifies stops
    // the start rather than being served or skipped. The same gallery backs
    // the Iroh registry protocol here and the web read surface below.
    let gallery: Option<id_worlds::program_registry_net::SharedGallery> = match &gallery_dir {
        Some(dir) => {
            let gallery = id_worlds::program_gallery::Gallery::open_dir(
                dir,
                id_worlds::program_registry::AdmitAll,
            )
            .map_err(|error| anyhow::anyhow!("open the gallery in {}: {error:#}", dir.display()))?;
            status!(
                "gallery: {} program bundles ({} listed, {} admission), {} featured collections and {} transparency entries from {}; registry over Iroh{}",
                gallery.registry().len(),
                gallery
                    .list(&id_worlds::program_registry::ListQuery::default())
                    .len(),
                gallery.admission().config().posture.as_str(),
                gallery.collections().count(),
                gallery.admission().list().len(),
                dir.display(),
                if gallery_accept_publish {
                    ", accepting publishes"
                } else {
                    ", read-only"
                }
            );
            Some(Arc::new(std::sync::RwLock::new(gallery)))
        }
        None => None,
    };
    let router_builder = Router::builder(endpoint.clone())
        .accept(META_ALPN, meta)
        .accept(BLOBS_ALPN, blobs)
        .accept(iroh_gossip::net::GOSSIP_ALPN, gossip.clone())
        .accept(iroh_docs::net::ALPN, docs.clone());
    #[cfg(feature = "web")]
    let router_builder = match &event_events {
        Some(events) => router_builder.accept(
            id_events::event_store::sync::ALPN,
            id_events::event_sync::EventSyncProtocol::with_gate(
                Arc::clone(events),
                id_events::event_sync::gate(event_sync_policy),
            ),
        ),
        None => router_builder,
    };
    #[cfg(feature = "world")]
    let router_builder = match (world_hub.as_ref(), world_protocol) {
        (Some(hub), Some(world_protocol)) => router_builder
            .accept(id_worlds::world_net::WORLD_ALPN, world_protocol)
            .accept(
                id_worlds::world_replication_net::REPLICATION_ALPN,
                id_worlds::world_replication_net::WorldReplicationProtocol::new(
                    hub.clone(),
                    ed25519_dalek::SigningKey::from_bytes(&key.to_bytes()),
                ),
            )
            .accept(
                id_sync::envelope_net::ENVELOPE_ALPN,
                id_sync::envelope_net::EnvelopeProtocol::new(hub.clone()),
            )
            .accept(
                id_sync::artifact::PUSH_ALPN,
                id_sync::artifact::ArtifactPushProtocol::new(hub.clone()),
            )
            .accept(
                id_sync::artifact::PULL_ALPN,
                id_sync::artifact::ArtifactPullProtocol::new(hub.clone()),
            ),
        _ => router_builder,
    };
    #[cfg(all(feature = "world", feature = "ssh"))]
    let router_builder = router_builder.accept(id_worlds::world_ssh::SSH_TUNNEL_ALPN, ssh_tunnel);
    let router_builder = match &gallery {
        Some(gallery) => {
            let registry =
                id_worlds::program_registry_net::RegistryProtocol::new(Arc::clone(gallery));
            router_builder.accept(
                id_worlds::program_registry_net::REGISTRY_ALPN,
                if gallery_accept_publish {
                    registry.accepting_publish()
                } else {
                    registry
                },
            )
        }
        None => router_builder,
    };
    // Relay authorization uses the same signed-graph store as LocalEvents.
    // Serve has no neighbour-discovery view at this layer yet, so this slice
    // serves the gated ALPN with an empty fan-out set.
    #[cfg(feature = "web")]
    let router_builder = match &event_events {
        Some(events) => {
            let store = events.lock().await.shared_store();
            let frontier = GraphFrontier::new(store);
            let relay_key = ed25519_dalek::SigningKey::from_bytes(&key.to_bytes());
            let sink =
                ForwardingSink::fanning_out(node_id.to_string(), endpoint.clone(), Vec::new());
            let protocol = ChainedRelayProtocol::new(RelayGate::new(&relay_key, frontier), sink);
            router_builder.accept(CHAINED_RELAY_ALPN, protocol)
        }
        None => router_builder,
    };
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
        if policy == id_events::pull_policy::PullPolicy::Members {
            status!(
                "event sync: [policy] pull_policy=members — the local event store has no \
                 membership source, so every pull is refused (fail closed)"
            );
        }
        let endpoint = router.endpoint().clone();
        let discovery = peer_discovery.clone();
        let gate = id_events::event_sync::gate(policy);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(id_events::event_sync::SYNC_INTERVAL).await;
                for peer in discovery.peers() {
                    let peer_id = peer.announcement.node_id;
                    if peer_id == endpoint.id() {
                        continue;
                    }
                    match id_events::event_sync::sync_with_peer(&endpoint, peer_id, &events, &gate)
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

    if gossip_enabled {
        let config = resolved_defaults;

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
            let host_key = id_worlds::world_ssh::load_or_create_host_key(
                id_worlds::world_ssh::SSH_HOST_KEY_FILE,
            )
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
    let serve_lock =
        create_serve_lock(&serve_node_id, &local_addrs, web_port, web_binding_addrs).await?;
    info!("serve startup: serve lock written, printing node status (#190 breadcrumb)");

    status!("node: {serve_node_id}");
    if ephemeral {
        status!("mode: ephemeral (in-memory)");
    } else {
        status!("mode: persistent ({STORE_PATH})");
    }
    // Which defaults this binary embeds, shown at the default log level so
    // an operator can tell a customized build apart without verbose logs.
    status!("{}", id_core::discovery::defaults_provenance().describe());
    if relays_disabled {
        status!("relay: disabled");
    }
    if !gossip_enabled {
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
            String::from_utf8_lossy(id_worlds::world_net::WORLD_ALPN)
        );
        #[cfg(feature = "web")]
        if web {
            status!("world: web bridge at /ws/world");
        }
    }

    // #2147 slice 3: the web surface factory the configured
    // listeners are built from, captured for the publication
    // manager below (dynamic listeners and path mounts are built
    // over the same shared web state).
    #[cfg(feature = "web")]
    let mut publication_surface: Option<id_web::WebSurfaceFactory> = None;

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
            let securities: Vec<id_web::WebSecurity> = web_listeners
                .iter()
                .map(|(listener_addr, _, _)| {
                    id_web::WebSecurity::for_bind(listener_addr.ip(), web_token.clone(), &[])
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
            let seitan_state = id_web::SeitanState::new(
                id_directory::directory::Directory::open(&server_config_directory_path(
                    &groups_log,
                ))?,
                serve_freshness_window(started_server_config.as_ref()),
            );
            #[cfg(not(feature = "world"))]
            let seitan_state = id_web::SeitanState::default();
            // #8214 slice 4: the transport-advertisement minter the
            // web surface serves at
            // /.well-known/id-transport-advertisement — signed by
            // the node key, its sequence counter beside the
            // identity key (KEY_FILE), first mint over the
            // endpoint's current transports. A watcher re-mints
            // when the endpoint's address set changes (Decision 3:
            // mint on any transport change); the minter itself
            // re-mints per TTL when the handler asks for the
            // current bytes.
            let transport_ad_minter = {
                let minter = std::sync::Arc::new(id_core::transport_advertisement::AdMinter::new(
                    ed25519_dalek::SigningKey::from_bytes(&key.to_bytes()),
                    id_core::transport_advertisement::SequenceFile::new(
                        std::path::Path::new(KEY_FILE).with_file_name(".transport-ad-sequence"),
                    ),
                    id_core::transport_advertisement::RECOMMENDED_TTL_SECONDS,
                ));
                if let Err(error) = minter.refresh(ad_transports(&endpoint), ad_now()) {
                    warn!("transport advertisement: initial mint failed: {error}");
                }
                let watcher_minter = std::sync::Arc::clone(&minter);
                let watcher_endpoint = endpoint.clone();
                tokio::spawn(async move {
                    let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
                    loop {
                        tick.tick().await;
                        if let Err(error) =
                            watcher_minter.refresh(ad_transports(&watcher_endpoint), ad_now())
                        {
                            warn!("transport advertisement: refresh failed: {error}");
                        }
                    }
                });
                minter
            };
            let gallery = gallery.clone();
            let (web_routers, surface) = id_web::web_routers_with_surface(
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
                Some(endpoint.clone()),
                Some(std::sync::Arc::clone(&transport_ad_minter)),
                gallery,
                Some(run_apps),
            )
            .await?;
            publication_surface = Some(surface);
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
                    if let Err(e) = axum::serve(
                        listener,
                        web_router.into_make_service_with_connect_info::<SocketAddr>(),
                    )
                    .await
                    {
                        tracing::error!("web server error: {}", e);
                    }
                }));
            }
        }
        handles
    };

    // #2147 slice 3: apply the persisted publication set to the
    // live web surface through the serve-owned PublicationManager:
    // listener placements bind real listeners serving the published
    // world's program surface, path mounts land in the Explorer
    // routers' shared mount table. Restore goes through the same
    // apply path as runtime publishes; a failure rolls back what
    // was applied and fails startup loudly rather than serving a
    // half-applied set. The manager stays alive for the life of
    // the process (dropping it would orphan its listeners' handles
    // without closing them; unpublish closes them explicitly).
    // When no web surface is running there is nothing to apply a
    // publication TO in this build/configuration: say so in the
    // startup log instead of pretending the set was restored.
    #[cfg(feature = "web")]
    let _publication_manager: Option<
        std::sync::Arc<
            std::sync::Mutex<crate::commands::serve_publications::ServePublicationManager>,
        >,
    > = match publication_surface {
        Some(surface) => {
            let has_explorer = web_bindings
                .iter()
                .any(|binding| matches!(binding.target(), WebContentTarget::Explorer));
            let factory = crate::commands::serve_publications::ServeListenerFactory::new(
                tokio::runtime::Handle::current(),
                surface.clone(),
                web_token.clone(),
            );
            let sink = crate::commands::serve_publications::ServeMountSink::new(
                surface.publication_mounts(),
                has_explorer,
            );
            let mut manager = crate::publications_runtime::PublicationManager::new(
                publications,
                crate::publications::publications_file(),
                crate::publications::KNOWN_EXPLORER_ROUTES
                    .iter()
                    .map(|route| (*route).to_owned())
                    .collect(),
                factory,
                sink,
            );
            // Boot shadow warnings were already issued by the
            // slice-1 block above; restore's copies are dropped.
            let _warnings = manager.restore()?;
            if !manager.registry().publications.is_empty() {
                status!(
                    "publications: {} persisted publication(s) live",
                    manager.registry().publications.len()
                );
            }
            // Slice 3b: share the manager with the meta protocol's
            // operator-only publication control, so `id publish` /
            // `id unpublish` / `id publications` drive THIS manager
            // in the running process.
            let manager = std::sync::Arc::new(std::sync::Mutex::new(manager));
            meta_handle.set_publication_control(
                crate::commands::serve_publications::ServePublicationControl::new(
                    std::sync::Arc::clone(&manager),
                )
                .handle(),
            );
            Some(manager)
        }
        None => {
            if !publications.publications.is_empty() {
                status!(
                    "publications: {} persisted publication(s) NOT applied \u{2014} \
                         no web surface is running in this serve (dynamic \
                         publications need the web interface)",
                    publications.publications.len()
                );
            }
            None
        }
    };

    #[cfg(feature = "ssh")]
    let ssh_handle = ssh.map(|(listener, ssh_port, hub, host_key, blobs)| {
        status!("world ssh: port {ssh_port} (ssh -p {ssh_port} <world>@<host>)");
        status!(
            "world ssh host key: {}",
            id_worlds::world_ssh::host_key_fingerprint(&host_key)
        );
        status!(
            "world ssh over iroh: `id world ssh-tunnel <node-id>` then `ssh -p 2222 <world>@127.0.0.1`"
        );
        // #787 Slice E: kitty keyboard negotiation is opt-in per server,
        // for worlds compiled with the extended Key module; every other
        // world keeps the legacy input semantics by default.
        let kitty_keyboard = std::env::var("ID_WORLD_SSH_KITTY_KEYBOARD")
            .is_ok_and(|value| value == "1" || value.eq_ignore_ascii_case("true"));
        if kitty_keyboard {
            status!(
                "world ssh kitty keyboard: enabled (kitty-disambiguate-v1, negotiated per session)"
            );
        }
        let options = id_worlds::world_ssh::SshServeOptions { kitty_keyboard };
        tokio::spawn(async move {
            if let Err(e) =
                id_worlds::world_ssh::serve_with_options(listener, hub, host_key, blobs, options)
                    .await
            {
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
    #[cfg(feature = "web")]
    let gate_groups_log = groups_log.clone();
    let group_warning_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(GROUP_WARNING_INTERVAL);
        interval.tick().await;
        loop {
            interval.tick().await;
            evaluate_group_warnings(&mut group_watcher, &groups_log, unix_now());
        }
    });

    // Blob gate refresh (#173): on the same cadence as the group warnings,
    // re-fold the group log and the document references so a removed member
    // loses fetch and a new document becomes fetchable without a restart.
    #[cfg(feature = "web")]
    let blob_gate_handle = match (&blob_gate, &event_events) {
        (Some(gate), Some(events)) => {
            let gate = Arc::clone(gate);
            let events = Arc::clone(events);
            let store = store_handle.clone();
            let groups_log = gate_groups_log;
            Some(tokio::spawn(async move {
                let mut interval = tokio::time::interval(GROUP_WARNING_INTERVAL);
                interval.tick().await;
                loop {
                    interval.tick().await;
                    // A failed rebuild keeps the previous fold (#13821):
                    // a transient read error must not empty the live gate.
                    if let Err(error) =
                        gate.try_refresh(build_blob_gate(&events, &store, &groups_log).await)
                    {
                        tracing::error!(
                            "blob gate: refresh failed: {error:#}; keeping the previous fold"
                        );
                    }
                }
            }))
        }
        _ => None,
    };

    tokio::signal::ctrl_c().await?;
    remove_serve_lock(serve_lock).await?;
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
    #[cfg(feature = "web")]
    if let Some(gate_task) = blob_gate_handle {
        gate_task.abort();
        let _ = gate_task.await;
    }
    group_warning_handle.abort();
    let _ = group_warning_handle.await;
    store.shutdown().await?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use proptest::test_runner::{Config as ProptestConfig, TestCaseError};

    #[test]
    fn relay_and_gossip_cli_flags_override_build_defaults() {
        use id_core::connectivity::RelayPolicy;

        assert_eq!(
            resolve_network_overrides(RelayPolicy::N0, false, false, false, false),
            (RelayPolicy::N0, false),
        );
        assert_eq!(
            resolve_network_overrides(RelayPolicy::N0, false, false, true, false),
            (RelayPolicy::N0, true),
        );
        assert_eq!(
            resolve_network_overrides(RelayPolicy::N0, true, false, false, true),
            (RelayPolicy::N0, false),
        );
        assert_eq!(
            resolve_network_overrides(RelayPolicy::Disabled, true, false, false, false),
            (RelayPolicy::Disabled, true),
        );
        assert_eq!(
            resolve_network_overrides(RelayPolicy::N0, true, true, false, false),
            (RelayPolicy::Disabled, true),
        );
    }

    #[tokio::test]
    async fn stale_lock_cleanup_preserves_replacement_written_after_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.lock");
        let stale = r#"{"pid":1}"#;
        let replacement = r#"{"pid":2}"#;
        afs::write(&path, stale).await.unwrap();

        // Deterministically model a writer publishing a new lock after the
        // caller read the stale contents but before cleanup takes the path.
        let replacement_file = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
        std::fs::write(replacement_file.path(), replacement).unwrap();
        replacement_file.persist(&path).unwrap();
        remove_stale_serve_lock(&path, stale);

        assert_eq!(afs::read_to_string(path).await.unwrap(), replacement);
    }

    #[test]
    fn competing_serve_starts_publish_only_one_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.lock");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let publishers = [r#"{"pid":1}"#, r#"{"pid":2}"#].map(|contents| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                publish_serve_lock(&path, contents).is_ok()
            })
        });

        barrier.wait();
        let successes = publishers
            .into_iter()
            .map(|publisher| publisher.join().unwrap())
            .filter(|success| *success)
            .count();

        assert_eq!(successes, 1, "exactly one serve start must own the lock");
        let contents = std::fs::read_to_string(path).unwrap();
        assert!(
            contents == r#"{"pid":1}"# || contents == r#"{"pid":2}"#,
            "the published lock must be one complete contender's record"
        );
    }

    #[test]
    fn shutdown_preserves_a_replacement_serve_lock_without_vacating_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.lock");
        let incumbent = r#"{"pid":1}"#;
        let replacement = r#"{"pid":2}"#;
        let third = r#"{"pid":3}"#;
        publish_serve_lock(&path, incumbent).unwrap();

        // Hold the same interprocess guard while installing a replacement and
        // starting both a late cleanup and a third publisher. Neither may
        // observe a temporarily vacant public lock path.
        let guard = lock_serve_lock(&path).unwrap();
        fs::write(&path, replacement).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let cleanup_path = path.clone();
        let cleanup_barrier = barrier.clone();
        let cleanup = std::thread::spawn(move || {
            cleanup_barrier.wait();
            remove_serve_lock_at(&cleanup_path, incumbent);
        });
        let publish_path = path.clone();
        let publish_barrier = barrier.clone();
        let publisher = std::thread::spawn(move || {
            publish_barrier.wait();
            publish_serve_lock(&publish_path, third)
        });
        barrier.wait();
        assert_eq!(fs::read_to_string(&path).unwrap(), replacement);
        drop(guard);

        cleanup.join().unwrap();
        assert!(publisher.join().unwrap().is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), replacement);
    }

    #[test]
    fn startup_reclaims_a_dead_process_lock_before_no_clobber_publish() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.lock");
        let stale = serde_json::to_string(&ServeInfo {
            node_id: "stale".to_owned(),
            pid: i32::MAX as u32,
            addrs: Vec::new(),
            web_port: None,
            web_bindings: Vec::new(),
            start_time: None,
        })
        .unwrap();
        let current = r#"{"pid":42}"#;
        fs::write(&path, stale).unwrap();

        let _guard = lock_serve_lock(&path).unwrap();
        publish_serve_lock_reclaiming_stale_locked(&path, current).unwrap();

        assert_eq!(fs::read_to_string(path).unwrap(), current);
    }

    fn stale_test_peer(seed: u8) -> id_core::discovery::PeerInfo {
        let node_id = iroh_base::SecretKey::from_bytes(&[seed; 32]).public();
        id_core::discovery::PeerInfo {
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
        use id_events::groups::Role;
        use id_groups::groups::actions::NewGroup;

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
        use id_events::groups::Role;

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
                log.add(author, group, member, id_events::groups::Role::Read)
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
        use id_events::groups::Role;

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
        use id_events::groups::Role;

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
        use id_events::groups::Role;

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
        use id_events::groups::Role;

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
        log.remove(author, group, other, id_events::groups::RemoveMode::Revoke)
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
            id_caps::world_caps::default_ceiling_names()
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

    /// Build a `/proc/<pid>/stat` string whose every field carries a
    /// distinct, field-number-derived value, with field 22 (starttime)
    /// set to `start_time`. The builder counts fields by position as
    /// it writes them, so the expected value is known by construction
    /// — independently of the parser under test (#8816).
    #[cfg(target_os = "linux")]
    fn stat_string_with_oracle(comm: &str, start_time: u64) -> String {
        // Fields 3..=52 follow comm. Field f (for f != 22) gets the
        // value f * 1_000, which never collides with a start_time in
        // the generated range below, so extracting any neighbouring
        // field yields a different value and fails the assertion.
        let mut stat = format!("4242 ({comm})");
        for field in 3..=52u64 {
            if field == 3 {
                stat.push_str(" S");
            } else if field == 22 {
                stat.push_str(&format!(" {start_time}"));
            } else {
                stat.push_str(&format!(" {}", field * 1_000));
            }
        }
        stat
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn parse_proc_stat_start_time_reads_field_22_by_independent_oracle() {
        // Hand-counted stat line: after "(my proc)" the fields are
        // state S (3), then fields 4..=21 are 4..=21, field 22 is
        // 987654, and fields 23..=25 are 23, 24, 25. Neighbouring
        // fields hold 21 and 23, so a wrong-field extraction fails.
        let stat =
            "1234 (my proc) S 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 987654 23 24 25";
        assert_eq!(parse_proc_stat_start_time(stat), Some(987654));
        // comm containing parentheses: parsing resumes after the
        // FINAL ')', so the same field layout still yields field 22.
        let stat_parens = "1234 (weird (proc) name) S 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 987654 23 24 25";
        assert_eq!(parse_proc_stat_start_time(stat_parens), Some(987654));
        // Malformed inputs have no field 22 to read.
        assert_eq!(parse_proc_stat_start_time(""), None);
        assert_eq!(parse_proc_stat_start_time("1234 (unclosed S 1 2 3"), None);
        assert_eq!(parse_proc_stat_start_time("1234 (p) S 1 2 3"), None);
        assert_eq!(
            parse_proc_stat_start_time(
                "1234 (p) S 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 notanumber 23 24 25"
            ),
            None
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn parse_proc_stat_start_time_matches_generated_oracle() {
        // Oracle: `property_parse_proc_stat_start_time_matches_oracle` —
        // for generated comm values (spaces/parens included) and an
        // independently chosen field-22 value, the parser returns
        // exactly that value; every other field holds a distinct
        // value, so extracting the wrong field fails.
        let check = |comm: &str, start_time: u64| -> Result<(), TestCaseError> {
            let stat = stat_string_with_oracle(comm, start_time);
            prop_assert_eq!(parse_proc_stat_start_time(&stat), Some(start_time));
            Ok(())
        };
        let config = ProptestConfig {
            cases: 256,
            ..ProptestConfig::default()
        };
        proptest!(config, |(comm in proptest::string::string_regex("[a-zA-Z0-9 ()_-]{0,24}").expect("comm strategy"), start_time in 1_000_000u64..2_000_000u64)| {
            check(&comm, start_time)?;
        });
    }

    #[test]
    fn test_is_process_alive_current_process() {
        let pid = std::process::id();
        assert!(is_process_alive(pid, get_process_start_time(pid)));
    }

    #[test]
    fn test_is_process_alive_reused_pid() {
        // #3040: this live process holding the PID, but a lock recording
        // a different start time names a dead incarnation of the PID —
        // the answer must be stale (false), not alive. Linux-only: the
        // start time is read from /proc there.
        #[cfg(target_os = "linux")]
        {
            let pid = std::process::id();
            let actual = get_process_start_time(pid).expect("own start time is readable");
            assert!(is_process_alive(pid, Some(actual)));
            assert!(!is_process_alive(pid, Some(actual + 1)));
        }
    }

    #[test]
    fn test_is_process_alive_pid_zero() {
        #[cfg(unix)]
        {
            assert!(!is_process_alive(0, None));
        }
    }

    #[test]
    fn test_is_process_alive_pid_out_of_i32_range() {
        #[cfg(unix)]
        {
            assert!(!is_process_alive(i32::MAX as u32 + 1, None));
            assert!(!is_process_alive(u32::MAX, None));
        }
    }

    #[test]
    fn test_is_process_alive_nonexistent() {
        // Use a very high PID that's unlikely to exist
        // Note: On non-Unix this always returns true
        #[cfg(unix)]
        {
            assert!(!is_process_alive(999_999_999, None));
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
                is_process_alive(1, None),
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
            start_time: None,
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
            start_time: None,
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
            start_time: None,
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
            start_time: None,
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
        assert!(err.contains("web_bindings in defaults.json"), "{err}");
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
    fn test_web_content_target_program_validates_world_name_rule() {
        // #5711: a program target names a world, so program()/parse
        // reject every name id_worlds::world_hub::validate_world_name
        // rejects -- a target no world can bear could never resolve.
        for bad in [
            "Arena",         // uppercase
            ".",             // dot segment
            "..",            // parent segment
            "a/b",           // slash
            "a.b",           // embedded dot
            "has space",     // interior whitespace survives trim
            "caf\u{e9}",     // non-ASCII
            &"a".repeat(65), // over the 64-byte cap
        ] {
            assert!(
                WebContentTarget::program(bad).is_err(),
                "program target accepted {bad:?}"
            );
            let spec = format!("program:{bad}");
            let err = WebContentTarget::parse(&spec).unwrap_err();
            assert!(
                err.to_string().contains(&spec),
                "parse error must name the spec {spec:?}: {err}"
            );
        }
        // Legal names keep working: exactly at the byte cap, and
        // hyphen/underscore/digit mixes, with trim preserved.
        let max = "a".repeat(64);
        assert_eq!(
            WebContentTarget::program(max.clone()).unwrap(),
            WebContentTarget::Program(max)
        );
        assert_eq!(
            WebContentTarget::program(" my-world_2 ").unwrap(),
            WebContentTarget::Program("my-world_2".to_owned())
        );
        assert_eq!(
            WebContentTarget::parse("program:my-world_2")
                .unwrap()
                .describe(),
            "program:my-world_2"
        );
    }

    #[test]
    fn prop_program_target_acceptance_matches_world_name_oracle() {
        // Oracle (AGENTS.md testing doctrine; Codex finding on PR #8682):
        // id_worlds::world_hub::validate_world_name applied to the trimmed
        // input decides acceptance. program() and parse("program:<input>")
        // must agree with the oracle, and on acceptance the stored name
        // is exactly the trimmed input.
        fn check(input: &str) -> Result<(), TestCaseError> {
            let trimmed = input.trim();
            let oracle_ok = id_worlds::world_hub::validate_world_name(trimmed).is_ok();
            let program = WebContentTarget::program(input.to_owned());
            prop_assert!(
                program.is_ok() == oracle_ok,
                "program({:?}) disagrees with validate_world_name({:?})",
                input,
                trimmed
            );
            if oracle_ok {
                prop_assert_eq!(
                    program.expect("oracle-accepted program target"),
                    WebContentTarget::Program(trimmed.to_owned())
                );
            }
            let spec = format!("program:{input}");
            let parsed = WebContentTarget::parse(&spec);
            prop_assert!(
                parsed.is_ok() == oracle_ok,
                "parse({:?}) disagrees with validate_world_name({:?})",
                spec,
                trimmed
            );
            if oracle_ok {
                prop_assert_eq!(
                    parsed.expect("oracle-accepted parse").describe(),
                    format!("program:{trimmed}")
                );
            }
            Ok(())
        }
        let config = ProptestConfig {
            cases: 256,
            ..ProptestConfig::default()
        };
        proptest!(config, |(input in proptest::string::string_regex(".{0,70}").expect("input strategy"))| {
            check(&input)?;
        });
        // Targeted alphabet: characters at the rule boundary (case, dot,
        // slash, separators, whitespace) at boundary lengths.
        proptest!(config, |(input in proptest::string::string_regex("[a-zA-Z0-9._/ -]{0,70}").expect("boundary strategy"))| {
            check(&input)?;
        });
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
            assert!(
                err.contains("web_bindings entry") && err.contains("defaults.json"),
                "{err}"
            );
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
        assert!(err.to_string().contains("defaults.json"), "{err}");
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
        let directory =
            id_directory::directory::Directory::open(&world.join("directory.jsonl")).unwrap();
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
            id_events::pull_policy::PullPolicy::Open
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
            id_events::pull_policy::PullPolicy::Members
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
            id_directory::seitan::DEFAULT_WINDOW
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
            id_core::abuse::BudgetPolicy::default()
        );
        let world = id_worlds::world::WorldHandle::spawn(
            id_worlds::world::WorldCore::new("lobby", id_worlds::world::WorldLimits::default())
                .unwrap(),
        );
        let service = id_worlds::world_session::WorldService::new(world, None);
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
            .check_budget(id_core::abuse::Direction::Inbound, ann, None, 600, 1_000)
            .unwrap_err();
        assert_eq!(
            refusal.reason,
            id_core::abuse::RefusalReason::BudgetExhausted
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "world")]
    #[tokio::test]
    async fn serve_world_protocol_defaults_budgets_without_config() {
        let world = id_worlds::world::WorldHandle::spawn(
            id_worlds::world::WorldCore::new("lobby", id_worlds::world::WorldLimits::default())
                .unwrap(),
        );
        let service = id_worlds::world_session::WorldService::new(world, None);
        let protocol = world_protocol_for(service.into(), [9; 32], None);
        let handle = protocol.abuse_handle();
        let evaluator = handle.lock().unwrap();
        assert_eq!(
            evaluator.budgets(),
            Some(&id_core::abuse::BudgetPolicy::default())
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
