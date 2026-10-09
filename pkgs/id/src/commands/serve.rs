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

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use anyhow::ensure;
use distributed_topic_tracker::{AutoDiscoveryGossip, RecordPublisher, TopicId};
use futures_lite::StreamExt;
use iroh::{
    endpoint::{Endpoint, RelayMode, presets},
    protocol::Router,
};
use iroh_base::EndpointId;
use iroh_blobs::{ALPN as BLOBS_ALPN, BlobsProtocol};
use iroh_docs::protocol::Docs;
use iroh_gossip::net::Gossip;
use iroh_mdns_address_lookup::MdnsAddressLookup;
#[cfg(all(feature = "world", feature = "sandbox"))]
use rand::RngExt as _;
use serde::{Deserialize, Serialize};
use tokio::fs as afs;
use tracing::{debug, info, warn};

use crate::discovery::{
    ANNOUNCE_INTERVAL, PeerAnnouncement, PeerDiscovery, STALE_CHECK_INTERVAL, STALE_THRESHOLD,
    resolve_config,
};
use crate::protocol::{MetaProtocol, MetaRequest, MetaResponse};
use crate::store::{load_or_create_keypair, open_store};
use crate::tags::TagStore;
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
#[allow(clippy::cast_possible_wrap)] // PID is always positive, wrap is safe for kill()
#[allow(unsafe_code)] // Required for libc::kill
pub fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: libc::kill with signal 0 only checks process existence without
        // sending any signal. The pid cast from u32 to i32 is safe because valid
        // PIDs on Unix are always positive and fit in i32.
        unsafe { libc::kill(pid as i32, 0) == 0 }
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
) -> Result<()> {
    let info = ServeInfo {
        node_id: node_id.to_string(),
        pid: std::process::id(),
        addrs: addrs.iter().map(ToString::to_string).collect(),
        web_port,
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
    /// Iroh QUIC port (0 = random).
    pub iroh_port: u16,
    /// Address the web interface binds to.
    pub bind: std::net::IpAddr,
    /// Token required by the web interface.
    pub web_token: Option<String>,
    /// Host a multiplayer world (durable unless `ephemeral`).
    pub world: bool,
    /// Admin secret required to mint world guest capabilities.
    pub world_admin_token: Option<String>,
    /// Optional Wasm world program, compiled for the sandbox's Roc platform ABI.
    pub world_module: Option<PathBuf>,
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
    /// Capabilities the default world's program may use.
    pub world_caps: Vec<String>,
    /// `deny` or `grant-on-use`.
    pub world_cap_policy: String,
    /// Roc binary for on-the-fly compilation.
    pub roc_bin: Option<String>,
    /// Platform directory for on-the-fly compilation.
    pub roc_platform: Option<PathBuf>,
    /// Serve the world over SSH on this port.
    pub world_ssh_port: Option<u16>,
    /// Accept native (ELF) module installs.
    pub world_native: bool,
    /// Execution bounds for world programs.
    pub world_runtime: crate::cli::WorldRuntimeArgs,
    /// Nodes allowed to modify the store.
    pub allow_node: Vec<String>,
    /// Let every peer modify the store.
    pub open_writes: bool,
}

fn validate_world_options(
    world: bool,
    admin_token: Option<&str>,
    world_module: Option<&PathBuf>,
    world_name: &str,
    world_cap_policy: &str,
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
struct ServeWorlds {
    ephemeral: bool,
    admin_token: Option<String>,
    blobs: iroh_blobs::api::Store,
    docs: Docs,
    checkpoint_every: u64,
    default_world: String,
    default_module: Option<PathBuf>,
    grant_on_use: bool,
    compiler: Option<crate::world_compile::Compiler>,
    native: bool,
    runtime: crate::world_limits::RuntimeLimits,
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
            ..WorldLimits::default()
        };
        let runtime = self.runtime;
        let make = {
            let admin_token = self.admin_token.clone();
            let blobs = self.blobs.clone();
            move |handle| {
                WorldService::new(handle, admin_token)
                    .with_blob_store(blobs)
                    .with_runtime_limits(runtime)
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
        let service = if self.ephemeral {
            make(WorldHandle::spawn(WorldCore::new(name, limits)?)).with_records_store(records)
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
pub async fn cmd_serve(opts: ServeOptions) -> Result<()> {
    let ServeOptions {
        ephemeral,
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
        iroh_port,
        bind,
        web_token,
        world,
        world_admin_token,
        world_module,
        world_name,
        world_checkpoint_every,
        world_max_open,
        world_max_sessions,
        world_idle_secs,
        world_caps,
        world_cap_policy,
        roc_bin,
        roc_platform,
        world_native,
        world_ssh_port,
        world_runtime,
        allow_node,
        open_writes,
    } = opts;
    validate_world_options(
        world,
        world_admin_token.as_deref(),
        world_module.as_ref(),
        &world_name,
        &world_cap_policy,
    )?;
    let key = load_or_create_keypair(KEY_FILE).await?;
    let node_id: EndpointId = key.public();
    info!("serve: {}", node_id);

    // Who may modify this store: this node, this machine's client key (so the
    // local CLI/REPL work with no setup), `--allow-node`, and `.iroh-allowed`.
    let access = build_access_policy(node_id, &allow_node, open_writes).await?;

    let store = open_store(ephemeral).await?;
    let store_handle = store.as_store();

    let mut builder = Endpoint::builder(presets::N0).secret_key(key.clone());
    if no_relay {
        builder = builder.relay_mode(RelayMode::Disabled);
    }
    if !no_mdns {
        builder = builder.address_lookup(MdnsAddressLookup::builder());
    }
    if iroh_port != 0 {
        builder = builder.bind_addr(std::net::SocketAddrV4::new(
            Ipv4Addr::UNSPECIFIED,
            iroh_port,
        ))?;
    }
    let endpoint = builder.bind().await?;

    // Create peer discovery table
    let peer_discovery = PeerDiscovery::new();

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
    let meta = MetaProtocol::new(
        &store_handle,
        Some(peer_discovery.clone()),
        Arc::clone(&tag_store),
        access.clone(),
        node_id,
    );
    // iroh-blobs rejects pushes unless an event handler enables them; this one
    // allows them only from nodes the access policy lets write.
    let blobs = BlobsProtocol::new(
        &store_handle,
        Some(crate::access::blobs_events(access.clone())),
    );

    // Every world lives in one hub, shared by the Iroh protocol and the web
    // bridge. The default world opens now so a bad journal fails startup.
    #[cfg(feature = "world")]
    let world_hub = if world {
        use crate::world_hub::{HubLimits, WorldHub};
        let runtime = crate::world_limits::RuntimeLimits::from_args(&world_runtime)?;
        let opener = Arc::new(ServeWorlds {
            ephemeral,
            admin_token: world_admin_token.clone(),
            blobs: store_handle.clone(),
            docs: docs.clone(),
            checkpoint_every: world_checkpoint_every,
            default_world: world_name.clone(),
            default_module: world_module.clone(),
            grant_on_use: world_cap_policy == "grant-on-use",
            compiler: resolve_compiler(roc_bin, roc_platform)?,
            native: world_native,
            runtime,
        });
        let hub = WorldHub::new(
            opener,
            world_admin_token.clone(),
            world_name.clone(),
            HubLimits {
                max_open_worlds: world_max_open,
                max_sessions: world_max_sessions,
            },
        );
        let default_service = hub.open_default().await?;
        if !world_caps.is_empty() {
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
    #[cfg(not(feature = "world"))]
    ensure!(!world, "this build has no world support (feature `world`)");

    let router_builder = Router::builder(endpoint)
        .accept(META_ALPN, meta)
        .accept(BLOBS_ALPN, blobs)
        .accept(iroh_gossip::net::GOSSIP_ALPN, gossip.clone())
        .accept(iroh_docs::net::ALPN, docs.clone());
    #[cfg(feature = "world")]
    let router_builder = match &world_hub {
        Some(hub) => router_builder.accept(
            crate::world_net::WORLD_ALPN,
            crate::world_net::WorldProtocol::new(hub.clone()),
        ),
        None => router_builder,
    };
    let router = router_builder.spawn();

    if !no_gossip {
        // Resolve effective config from defaults + CLI flags
        let config = resolve_config(
            &bootstrap,
            topic.as_deref(),
            topic_secret.as_deref(),
            replace_defaults,
            no_default_bootstrap,
            no_default_topic,
        );

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
        let gossip_store = store_handle.clone();
        let gossip_node_id = node_id;
        let gossip_endpoint = router.endpoint().clone();
        let _gossip_handle = tokio::spawn(async move {
            run_gossip_loop(
                gossip_node_id,
                sender,
                receiver,
                gossip_peer_discovery,
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

    // Bind the web server early to capture the actual port for the lock file,
    // but don't start serving yet (spawn happens after lock file is written).
    #[allow(unused_mut)] // web_port is only mutated with the `web` feature
    let mut web_port: Option<u16> = None;
    #[cfg(feature = "web")]
    let web_listener = if web {
        let addr = SocketAddr::new(bind, port);
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let actual_port = listener.local_addr()?.port();
        web_port = Some(actual_port);
        Some(listener)
    } else {
        None
    };

    #[cfg(feature = "ssh")]
    let ssh = match (world_ssh_port, &world_hub) {
        (Some(port), Some(hub)) => {
            let listener = tokio::net::TcpListener::bind(SocketAddr::new(bind, port)).await?;
            let ssh_port = listener.local_addr()?.port();
            let host_key =
                crate::world_ssh::load_or_create_host_key(crate::world_ssh::SSH_HOST_KEY_FILE)
                    .await?;
            Some((listener, ssh_port, hub.clone(), host_key))
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
    create_serve_lock(&serve_node_id, &local_addrs, web_port).await?;

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

    // Start web server now that the lock file is written
    #[cfg(feature = "web")]
    let _web_handle = if let Some(listener) = web_listener {
        let identity_db_path = std::path::Path::new(KEY_FILE).with_file_name(".identity.db");
        let web_router = crate::web::web_router(
            store_handle.clone(),
            Some(peer_discovery.clone()),
            node_id.to_string(),
            Arc::clone(&tag_store),
            key.to_bytes(),
            identity_db_path,
            crate::web::WebSecurity::for_bind(bind, web_token.clone(), &[]),
            world_hub.clone(),
        )
        .await?;
        let actual_port = web_port.unwrap_or(port);
        let shown_host = if bind.is_unspecified() || bind.is_loopback() {
            "localhost".to_owned()
        } else {
            bind.to_string()
        };
        if let Some(t) = &web_token {
            status!("web: http://{shown_host}:{actual_port}/?token={t}");
        } else {
            status!("web: http://{shown_host}:{actual_port}");
        }
        #[cfg(feature = "world")]
        if world {
            status!("world: browser http://{shown_host}:{actual_port}/world");
        }
        if !bind.is_loopback() && web_token.is_none() {
            status_err!(
                "warning: the web UI is bound to {bind} without --web-token; \
                 anyone who can reach port {actual_port} can read and modify files"
            );
        }
        Some(tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, web_router).await {
                tracing::error!("web server error: {}", e);
            }
        }))
    } else {
        None
    };

    #[cfg(feature = "ssh")]
    let ssh_handle = ssh.map(|(listener, ssh_port, hub, host_key)| {
        status!("world ssh: port {ssh_port} (ssh -p {ssh_port} <world>@<host>)");
        status!(
            "world ssh host key: {}",
            crate::world_ssh::host_key_fingerprint(&host_key)
        );
        tokio::spawn(async move {
            if let Err(e) = crate::world_ssh::serve(listener, hub, host_key).await {
                tracing::error!("world ssh server error: {}", e);
            }
        })
    });

    tokio::signal::ctrl_c().await?;
    remove_serve_lock().await?;
    router.shutdown().await?;
    #[cfg(feature = "web")]
    if let Some(web_task) = _web_handle {
        web_task.abort();
        let _ = web_task.await;
    }
    #[cfg(feature = "ssh")]
    if let Some(ssh_task) = ssh_handle {
        ssh_task.abort();
        let _ = ssh_task.await;
    }
    #[cfg(feature = "world")]
    if let Some(hub) = &world_hub {
        hub.shutdown_all().await;
    }
    store.shutdown().await?;
    Ok(())
}

/// Background loop for gossip-based peer discovery.
///
/// Runs three concurrent tasks:
/// 1. **Announce**: Broadcasts a [`PeerAnnouncement`] every [`ANNOUNCE_INTERVAL`]
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
    store: iroh_blobs::api::Store,
    endpoint: Endpoint,
) {
    // Timeout for each individual peer probe.
    const PROBE_TIMEOUT: tokio::time::Duration = tokio::time::Duration::from_secs(10);

    let announce_store = store;
    let announce_node_id = node_id;

    // Spawn announce task
    let announce_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(ANNOUNCE_INTERVAL);
        loop {
            interval.tick().await;
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

            match postcard::to_allocvec(&announcement) {
                Ok(bytes) => {
                    if let Err(e) = sender.broadcast(bytes).await {
                        debug!("gossip broadcast error: {}", e);
                    }
                }
                Err(e) => {
                    debug!("gossip announcement serialization error: {}", e);
                }
            }
        }
    });

    // Spawn receive task
    let recv_discovery = peer_discovery.clone();
    let recv_handle = tokio::spawn(async move {
        loop {
            match receiver.next().await {
                Ok(event) => match event {
                    iroh_gossip::api::Event::Received(msg) => {
                        match postcard::from_bytes::<PeerAnnouncement>(&msg.content) {
                            Ok(announcement) => {
                                debug!(
                                    "peer announcement from {}: blob_count={}",
                                    announcement.node_id, announcement.blob_count
                                );
                                recv_discovery.update(announcement);
                            }
                            Err(e) => {
                                debug!("failed to deserialize peer announcement: {}", e);
                            }
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

    let cleanup_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(STALE_CHECK_INTERVAL);

        loop {
            interval.tick().await;

            let stale = cleanup_discovery.stale_peers();
            if stale.is_empty() {
                continue;
            }

            debug!("probing {} stale peer(s)", stale.len());

            for info in &stale {
                let peer_id = info.announcement.node_id;

                // Try to connect and send ListPeers RPC
                let probe_result = tokio::time::timeout(PROBE_TIMEOUT, async {
                    let conn = endpoint.connect(peer_id, META_ALPN).await?;
                    let (mut send, mut recv) = conn.open_bi().await?;
                    let req = postcard::to_allocvec(&MetaRequest::ListPeers)?;
                    send.write_all(&req).await?;
                    send.finish()?;
                    let resp_buf = recv.read_to_end(1024 * 1024).await?;
                    let _resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
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
                    }
                    Err(_) => {
                        debug!("stale peer {} probe timed out, removing", peer_id);
                    }
                }
            }

            // Remove peers that are still stale (probes that failed/timed out
            // didn't call refresh(), so they remain past STALE_THRESHOLD)
            cleanup_discovery.remove_stale(STALE_THRESHOLD);
        }
    });

    // Wait for any task to complete (normally they run until shutdown)
    tokio::select! {
        _ = announce_handle => {}
        _ = recv_handle => {}
        _ = cleanup_handle => {}
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn world_mode_requires_an_admin_token_but_not_web() {
        assert!(validate_world_options(true, None, None, "lobby", "deny").is_err());
        assert!(validate_world_options(true, Some(""), None, "lobby", "deny").is_err());
        assert!(validate_world_options(true, Some("admin"), None, "lobby", "deny").is_ok());
        assert!(validate_world_options(false, None, None, "lobby", "deny").is_ok());
        assert!(
            validate_world_options(
                true,
                Some("admin"),
                Some(&PathBuf::from("world.wasm")),
                "lobby",
                "deny"
            )
            .is_ok()
        );
        assert!(
            validate_world_options(
                false,
                None,
                Some(&PathBuf::from("world.wasm")),
                "lobby",
                "deny"
            )
            .is_err()
        );
    }

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
                validate_world_options(true, Some("admin"), None, bad, "deny").is_err(),
                "{bad:?} must be refused"
            );
        }
        assert!(validate_world_options(true, Some("admin"), None, "tt-2_x", "deny").is_ok());
        assert!(validate_world_options(true, Some("admin"), None, &"a".repeat(64), "deny").is_ok());
        assert!(
            validate_world_options(true, Some("admin"), None, &"a".repeat(65), "deny").is_err()
        );
        assert!(validate_world_options(true, Some("admin"), None, "lobby", "deny").is_ok());
        assert!(validate_world_options(true, Some("admin"), None, "lobby", "grant-on-use").is_ok());
        assert!(
            validate_world_options(true, Some("admin"), None, "lobby", "sometimes").is_err(),
            "an unknown policy is refused"
        );
    }

    #[test]
    fn test_is_process_alive_current_process() {
        let pid = std::process::id();
        assert!(is_process_alive(pid));
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
        // PID 1 (init) should exist on Unix systems, but may not be visible
        // in containerized environments where the container has its own PID namespace
        #[cfg(unix)]
        {
            // Just check that the function doesn't panic - the result depends on environment
            let _ = is_process_alive(1);
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
        };

        let json = serde_json::to_string(&info).unwrap();
        let parsed: ServeInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.web_port, None);
    }

    // Integration tests for lock file functions require file system access
    // and are tested via the integration test suite
}
