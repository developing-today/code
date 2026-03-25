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
use distributed_topic_tracker::{AutoDiscoveryGossip, RecordPublisher, TopicId};
use futures_lite::StreamExt;
use iroh::{
    address_lookup::MdnsAddressLookup,
    endpoint::{Endpoint, RelayMode, presets},
    protocol::Router,
};
use iroh_base::EndpointId;
use iroh_blobs::{ALPN as BLOBS_ALPN, BlobsProtocol};
use iroh_docs::protocol::Docs;
use iroh_gossip::net::Gossip;
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

/// Information about a running serve instance.
///
/// Retrieved from the lock file by [`get_serve_info`] to enable
/// other processes to connect to the local serve.
///
/// # Fields
///
/// - `node_id`: The public identity of the serve node
/// - `addrs`: Local socket addresses where the serve is listening
#[derive(Debug, Clone)]
pub struct ServeInfo {
    /// The public node ID derived from the serve's keypair.
    pub node_id: EndpointId,
    /// Socket addresses the serve is bound to.
    pub addrs: Vec<SocketAddr>,
}

/// Checks if a serve instance is running and returns its connection info.
///
/// Reads the lock file, verifies the PID is still alive, and returns
/// the serve info needed to connect. Returns `None` if:
/// - Lock file doesn't exist
/// - Lock file is malformed
/// - Referenced process is no longer running (stale lock)
///
/// # Lock File Format
///
/// ```text
/// <node_id>
/// <pid>
/// <socket_addr_1>
/// <socket_addr_2>
/// ...
/// ```
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
    let mut lines = contents.lines();
    let node_id_str = lines.next()?;
    let pid_str = lines.next()?;
    let pid: u32 = pid_str.parse().ok()?;

    // Check if process is still alive
    if !is_process_alive(pid) {
        // Stale lock file - remove it
        let _ = afs::remove_file(SERVE_LOCK).await;
        return None;
    }

    let node_id: EndpointId = node_id_str.parse().ok()?;

    // Parse socket addresses (remaining lines)
    let addrs: Vec<SocketAddr> = lines.filter_map(|line| line.parse().ok()).collect();

    Some(ServeInfo { node_id, addrs })
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

/// Creates the serve lock file with connection information.
///
/// Writes the node ID, current process ID, and socket addresses
/// to the lock file so other processes can discover and connect.
///
/// # Arguments
///
/// * `node_id` - The serve node's public identity
/// * `addrs` - Socket addresses the serve is listening on
///
/// # Errors
///
/// Returns an error if the lock file cannot be written.
pub async fn create_serve_lock(node_id: &EndpointId, addrs: &[SocketAddr]) -> Result<()> {
    use std::fmt::Write;
    let pid = std::process::id();
    let mut contents = format!("{node_id}\n{pid}");
    for addr in addrs {
        let _ = write!(contents, "\n{addr}");
    }
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
#[allow(clippy::too_many_arguments)]
pub async fn cmd_serve(
    ephemeral: bool,
    no_relay: bool,
    no_gossip: bool,
    web: bool,
    port: u16,
    bootstrap: Vec<String>,
    topic: Option<String>,
    topic_secret: Option<String>,
    no_default_bootstrap: bool,
    no_default_topic: bool,
    replace_defaults: bool,
    no_mdns: bool,
    iroh_port: u16,
) -> Result<()> {
    let key = load_or_create_keypair(KEY_FILE).await?;
    let node_id: EndpointId = key.public();
    info!("serve: {}", node_id);

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

    // Build router — gossip ALPN is always registered (needed by iroh-docs),
    // but peer discovery gossip topic only joins when gossip is enabled
    let meta = MetaProtocol::new(
        &store_handle,
        Some(peer_discovery.clone()),
        Some(tag_store.clone()),
    );
    let blobs = BlobsProtocol::new(&store_handle, None);

    let router = Router::builder(endpoint)
        .accept(META_ALPN, meta)
        .accept(BLOBS_ALPN, blobs)
        .accept(iroh_gossip::net::GOSSIP_ALPN, gossip.clone())
        .accept(iroh_docs::net::ALPN, docs.clone())
        .spawn();

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
        let dalek_verifying_key = dalek_signing_key.verifying_key();

        let record_publisher = RecordPublisher::new(
            dtt_topic_id,
            dalek_verifying_key,
            dalek_signing_key,
            None,
            config.topic_secret.clone(),
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

        println!("peers: gossip enabled (topic: {})", config.topic);
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
    create_serve_lock(&serve_node_id, &local_addrs).await?;

    println!("node: {serve_node_id}");
    if ephemeral {
        println!("mode: ephemeral (in-memory)");
    } else {
        println!("mode: persistent ({STORE_PATH})");
    }
    if no_relay {
        println!("relay: disabled");
    }
    if no_gossip {
        println!("peers: disabled");
    }
    if no_mdns {
        println!("mdns: disabled");
    } else {
        println!("mdns: enabled");
    }

    // Start web server if enabled
    #[cfg(feature = "web")]
    let _web_handle = if web {
        let web_router = crate::web::web_router(
            store_handle.clone(),
            Some(peer_discovery.clone()),
            node_id.to_string(),
            tag_store.clone(),
        );
        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        let listener = tokio::net::TcpListener::bind(addr).await?;
        // Get the actual bound port (important when port 0 is used for random assignment)
        let actual_port = listener.local_addr()?.port();
        println!("web: http://localhost:{actual_port}");
        Some(tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, web_router).await {
                tracing::error!("web server error: {}", e);
            }
        }))
    } else {
        None
    };

    tokio::signal::ctrl_c().await?;
    remove_serve_lock().await?;
    router.shutdown().await?;
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
    receiver: distributed_topic_tracker::GossipReceiver,
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
                Some(Ok(event)) => match event {
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
                Some(Err(e)) => {
                    debug!("gossip receive error: {}", e);
                }
                None => {
                    debug!("gossip receiver stream ended");
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

        let key = SecretKey::generate(&mut rand::rng());
        let node_id = key.public();
        let addrs = vec![
            "127.0.0.1:8080".parse().unwrap(),
            "[::1]:8080".parse().unwrap(),
        ];

        let info = ServeInfo { node_id, addrs };

        assert_eq!(info.node_id, node_id);
        assert_eq!(info.addrs.len(), 2);
        assert_eq!(info.addrs[0].to_string(), "127.0.0.1:8080");
    }

    #[test]
    fn test_serve_info_clone() {
        use iroh_base::SecretKey;

        let key = SecretKey::generate(&mut rand::rng());
        let node_id = key.public();
        let info = ServeInfo {
            node_id,
            addrs: vec!["127.0.0.1:8080".parse().unwrap()],
        };

        let cloned = info.clone();
        assert_eq!(cloned.node_id, info.node_id);
        assert_eq!(cloned.addrs, info.addrs);
    }

    // Integration tests for lock file functions require file system access
    // and are tested via the integration test suite
}
