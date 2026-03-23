//! Peers command -- discover and list known peers.
//!
//! This module implements the `peers` command which discovers other `id`
//! servers using gossip-based networking, RPC queries, or both.
//!
//! # Discovery Modes
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                        cmd_peers                                │
//! └─────────────────────────────────────────────────────────────────┘
//!                              │
//!          ┌───────────────────┼───────────────────┐
//!          ▼                   ▼                   ▼
//! ┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐
//! │   RPC Mode      │ │  Gossip Mode    │ │   Default       │
//! │ --rpc           │ │ --gossip        │ │ (try RPC, then  │
//! │                 │ │                 │ │  fall back to   │
//! │ ListPeers via   │ │ Join topic,     │ │  gossip)        │
//! │ MetaProtocol    │ │ collect announces│ │                 │
//! └─────────────────┘ └─────────────────┘ └─────────────────┘
//! ```
//!
//! # Depth-Based Crawling
//!
//! The `--depth` flag enables recursive peer discovery:
//! - Depth 0: Only return directly known peers
//! - Depth 1: Query each known peer's peer list (default)
//! - Depth N: Recursively query up to N levels deep
//!
//! Safeguards:
//! - `--max-peers` caps total discovered peers (default: 1000)
//! - `--timeout` limits per-crawl time (default: 30s)
//! - Visited node tracking prevents cycles
//!
//! # Examples
//!
//! ```bash
//! # List peers from local serve
//! id peers
//!
//! # List peers from a remote node
//! id peers abc123...def456
//!
//! # Deep crawl up to 3 levels
//! id peers --depth 3
//!
//! # Direct gossip discovery (no serve needed)
//! id peers --gossip
//!
//! # Gossip with custom topic
//! id peers --gossip --topic my-net --topic-secret my-secret
//! ```

use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};
use iroh::{
    address_lookup::MdnsAddressLookup,
    endpoint::{Endpoint, RelayMode, presets},
};
use iroh_base::EndpointId;

use crate::discovery::{PeerAnnouncement, resolve_config};
use crate::protocol::{MetaRequest, MetaResponse};
use crate::store::load_or_create_keypair;
use crate::{CLIENT_KEY_FILE, META_ALPN, is_node_id};

use super::client::create_local_client_endpoint;
use super::serve::get_serve_info;

/// Options for the peers command.
#[derive(Debug)]
pub struct PeersOptions {
    /// Use gossip only (join topic, collect announcements).
    pub gossip: bool,
    /// Use RPC only (query server's `ListPeers`).
    pub rpc: bool,
    /// Recursive depth for peer crawling (default 1, 0 = no crawl).
    pub depth: i32,
    /// Hard cap on total peers discovered.
    pub max_peers: usize,
    /// Per-crawl timeout in seconds.
    pub timeout_secs: u64,
    /// Comma-separated seed node IDs for gossip bootstrapping.
    pub bootstrap: Vec<String>,
    /// Gossip topic name.
    pub topic: Option<String>,
    /// Shared secret for topic access control.
    pub topic_secret: Option<String>,
    /// Skip default bootstrap nodes from `defaults.conf`.
    pub no_default_bootstrap: bool,
    /// Skip default topic/secret from `defaults.conf`.
    pub no_default_topic: bool,
    /// Use only `defaults.conf` values, ignoring hardcoded fallbacks.
    pub replace_defaults: bool,
    /// Disable relay servers.
    pub no_relay: bool,
    /// Disable mDNS local peer discovery.
    pub no_mdns: bool,
}

/// Runs the peers discovery command.
///
/// Discovers peers via RPC (querying a serve instance), gossip (joining
/// a topic and collecting announcements), or both (default).
///
/// # Arguments
///
/// * `node` - Optional specific node to query (64-char hex node ID)
/// * `options` - Configuration for discovery behavior
///
/// # Output
///
/// Prints discovered peers in tab-separated format:
/// ```text
/// <node_id>\t<name>\t<blob_count>\t<timestamp>
/// ```
pub async fn cmd_peers(node: Option<String>, options: PeersOptions) -> Result<()> {
    let mut all_peers: HashMap<EndpointId, PeerAnnouncement> = HashMap::new();

    // Determine mode
    let use_rpc = options.rpc || !options.gossip;
    let use_gossip = options.gossip || !options.rpc;

    // Try RPC first
    if use_rpc {
        let rpc_peers = if let Some(ref node_str) = node {
            // Query specific remote node
            query_remote_peers(node_str, options.no_relay, options.no_mdns).await?
        } else {
            // Try local serve
            match query_local_peers().await {
                Ok(peers) => peers,
                Err(e) => {
                    if options.rpc {
                        // User explicitly requested RPC, fail if no serve
                        return Err(e);
                    }
                    // Default mode: RPC failed, will try gossip
                    Vec::new()
                }
            }
        };

        for peer in &rpc_peers {
            all_peers.insert(peer.node_id, peer.clone());
        }

        // Depth crawling via RPC
        if options.depth != 0 && !rpc_peers.is_empty() {
            let crawled = crawl_peers(
                &rpc_peers,
                options.depth,
                options.max_peers,
                options.timeout_secs,
                options.no_relay,
                options.no_mdns,
            )
            .await;
            for peer in crawled {
                all_peers.insert(peer.node_id, peer);
            }
        }
    }

    // Try gossip if needed
    if use_gossip && (all_peers.is_empty() || options.gossip) {
        let gossip_peers = discover_via_gossip(&options).await;
        match gossip_peers {
            Ok(peers) => {
                for peer in peers {
                    all_peers.insert(peer.node_id, peer);
                }
            }
            Err(e) => {
                if options.gossip {
                    // User explicitly requested gossip, fail if it errors
                    return Err(e);
                }
                // Default mode: gossip failed, just use RPC results
                tracing::debug!("gossip discovery failed: {}", e);
            }
        }
    }

    // Output results
    if all_peers.is_empty() {
        println!("(no peers discovered)");
    } else {
        let mut peers: Vec<_> = all_peers.into_values().collect();
        peers.sort_by(|a, b| a.node_id.to_string().cmp(&b.node_id.to_string()));
        for peer in peers {
            let name = peer.name.as_deref().unwrap_or("-");
            println!(
                "{}\t{}\t{}\t{}",
                peer.node_id, name, peer.blob_count, peer.timestamp_secs
            );
        }
    }

    Ok(())
}

/// Queries the local serve instance for its peer list via RPC.
async fn query_local_peers() -> Result<Vec<PeerAnnouncement>> {
    let serve_info = get_serve_info()
        .await
        .ok_or_else(|| anyhow::anyhow!("no local serve running"))?;

    let (endpoint, endpoint_addr) = create_local_client_endpoint(&serve_info).await?;
    let result = async {
        let meta_conn = endpoint.connect(endpoint_addr, META_ALPN).await?;
        let (mut send, mut recv) = meta_conn.open_bi().await?;
        let req = postcard::to_allocvec(&MetaRequest::ListPeers)?;
        send.write_all(&req).await?;
        send.finish()?;
        let resp_buf = recv.read_to_end(1024 * 1024).await?;
        let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
        meta_conn.close(0u32.into(), b"done");

        match resp {
            MetaResponse::ListPeers { peers } => Ok(peers),
            _ => Err(anyhow::anyhow!("unexpected response")),
        }
    }
    .await;
    endpoint.close().await;
    result
}

/// Queries a specific remote node for its peer list.
async fn query_remote_peers(
    node_str: &str,
    no_relay: bool,
    no_mdns: bool,
) -> Result<Vec<PeerAnnouncement>> {
    if !is_node_id(node_str) {
        bail!("invalid node ID: must be 64 hex characters");
    }
    let node_id: EndpointId = node_str.parse()?;
    let client_key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
    let mut builder = Endpoint::builder(presets::N0).secret_key(client_key);
    if no_relay {
        builder = builder.relay_mode(RelayMode::Disabled);
    }
    if !no_mdns {
        builder = builder.address_lookup(MdnsAddressLookup::builder());
    }
    let endpoint = builder.bind().await?;

    let result = async {
        let meta_conn = endpoint.connect(node_id, META_ALPN).await?;
        let (mut send, mut recv) = meta_conn.open_bi().await?;
        let req = postcard::to_allocvec(&MetaRequest::ListPeers)?;
        send.write_all(&req).await?;
        send.finish()?;
        let resp_buf = recv.read_to_end(1024 * 1024).await?;
        let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
        meta_conn.close(0u32.into(), b"done");

        match resp {
            MetaResponse::ListPeers { peers } => Ok(peers),
            _ => Err(anyhow::anyhow!("unexpected response")),
        }
    }
    .await;
    endpoint.close().await;
    result
}

/// Recursively crawls peers up to the specified depth.
///
/// For each discovered peer, connects and sends `MetaRequest::ListPeers`,
/// merging results. Tracks visited nodes to avoid cycles.
async fn crawl_peers(
    initial_peers: &[PeerAnnouncement],
    depth: i32,
    max_peers: usize,
    timeout_secs: u64,
    no_relay: bool,
    no_mdns: bool,
) -> Vec<PeerAnnouncement> {
    let mut all_peers: HashMap<EndpointId, PeerAnnouncement> = HashMap::new();
    let mut visited: HashSet<EndpointId> = HashSet::new();
    let mut frontier: Vec<EndpointId> = Vec::new();

    // Initialize with known peers
    for peer in initial_peers {
        all_peers.insert(peer.node_id, peer.clone());
        frontier.push(peer.node_id);
    }

    let mut current_depth = 0;
    let max_depth = if depth < 0 { i32::MAX } else { depth };
    let timeout = tokio::time::Duration::from_secs(timeout_secs);

    while current_depth < max_depth && !frontier.is_empty() {
        if all_peers.len() >= max_peers {
            tracing::warn!(
                "peers: max-peers cap ({}) reached at depth {}",
                max_peers,
                current_depth
            );
            break;
        }

        let mut next_frontier: Vec<EndpointId> = Vec::new();

        for peer_id in &frontier {
            if visited.contains(peer_id) {
                continue;
            }
            visited.insert(*peer_id);

            if all_peers.len() >= max_peers {
                break;
            }

            // Try to query this peer with timeout
            let peer_id_str = peer_id.to_string();
            match tokio::time::timeout(timeout, query_remote_peers(&peer_id_str, no_relay, no_mdns))
                .await
            {
                Ok(Ok(peers)) => {
                    for peer in peers {
                        if let std::collections::hash_map::Entry::Vacant(e) =
                            all_peers.entry(peer.node_id)
                        {
                            next_frontier.push(peer.node_id);
                            e.insert(peer);
                        }
                    }
                }
                Ok(Err(e)) => {
                    tracing::debug!("failed to query peer {}: {}", peer_id, e);
                }
                Err(_) => {
                    tracing::debug!("timeout querying peer {}", peer_id);
                }
            }
        }

        frontier = next_frontier;
        current_depth += 1;
    }

    all_peers.into_values().collect()
}

/// Discovers peers via gossip by joining the discovery topic.
///
/// Creates a temporary endpoint, joins the gossip topic via DHT
/// auto-discovery, and collects announcements for a limited time.
/// Uses [`resolve_config`] to merge defaults with CLI flags.
async fn discover_via_gossip(options: &PeersOptions) -> Result<Vec<PeerAnnouncement>> {
    use distributed_topic_tracker::{AutoDiscoveryGossip, RecordPublisher, TopicId};
    use iroh_gossip::net::Gossip;

    let client_key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
    let mut builder = Endpoint::builder(presets::N0).secret_key(client_key.clone());
    if options.no_relay {
        builder = builder.relay_mode(RelayMode::Disabled);
    }
    if !options.no_mdns {
        builder = builder.address_lookup(MdnsAddressLookup::builder());
    }
    let endpoint = builder.bind().await?;

    let gossip = Gossip::builder().spawn(endpoint.clone());

    // Accept gossip connections on this endpoint
    // Note: Without a Router, we need to handle connections manually
    let gossip_clone = gossip.clone();
    let endpoint_clone = endpoint.clone();
    let accept_handle = tokio::spawn(async move {
        while let Some(incoming) = endpoint_clone.accept().await {
            let gossip = gossip_clone.clone();
            tokio::spawn(async move {
                if let Ok(conn) = incoming.accept()
                    && let Ok(conn) = conn.await
                {
                    let _ = gossip.handle_connection(conn).await;
                }
            });
        }
    });

    // Resolve effective config
    let config = resolve_config(
        &options.bootstrap,
        options.topic.as_deref(),
        options.topic_secret.as_deref(),
        options.replace_defaults,
        options.no_default_bootstrap,
        options.no_default_topic,
    );

    let dtt_topic_id = TopicId::new(config.topic);

    let dalek_signing_key = ed25519_dalek::SigningKey::from_bytes(&client_key.to_bytes());
    let dalek_verifying_key = dalek_signing_key.verifying_key();

    let record_publisher = RecordPublisher::new(
        dtt_topic_id,
        dalek_verifying_key,
        dalek_signing_key,
        None,
        config.topic_secret,
    );

    let gossip_topic = gossip
        .subscribe_and_join_with_auto_discovery_no_wait(record_publisher)
        .await?;
    let (sender, receiver) = gossip_topic.split().await?;

    // Join bootstrap peers (defaults + CLI, already merged by resolve_config)
    let bootstrap_ids: Vec<EndpointId> = config
        .bootstrap
        .iter()
        .filter_map(|s| s.parse::<EndpointId>().ok())
        .collect();
    if !bootstrap_ids.is_empty() {
        sender.join_peers(bootstrap_ids, None).await?;
    }

    // Collect announcements for a limited time
    let collect_timeout = tokio::time::Duration::from_secs(options.timeout_secs);
    let mut peers: HashMap<EndpointId, PeerAnnouncement> = HashMap::new();

    let _ = tokio::time::timeout(collect_timeout, async {
        while let Some(Ok(event)) = receiver.next().await {
            if let iroh_gossip::api::Event::Received(msg) = event
                && let Ok(announcement) = postcard::from_bytes::<PeerAnnouncement>(&msg.content)
            {
                peers.insert(announcement.node_id, announcement);
            }
        }
    })
    .await;

    // Cleanup
    accept_handle.abort();
    endpoint.close().await;

    Ok(peers.into_values().collect())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_peers_options_defaults() {
        let options = PeersOptions {
            gossip: false,
            rpc: false,
            depth: 1,
            max_peers: 1000,
            timeout_secs: 30,
            bootstrap: Vec::new(),
            topic: None,
            topic_secret: None,
            no_default_bootstrap: false,
            no_default_topic: false,
            replace_defaults: false,
            no_relay: false,
            no_mdns: false,
        };

        // Default mode: both RPC and gossip
        assert!(!options.gossip);
        assert!(!options.rpc);
        assert_eq!(options.depth, 1);
        assert_eq!(options.max_peers, 1000);
        assert_eq!(options.timeout_secs, 30);
    }

    #[test]
    fn test_peers_options_gossip_only() {
        let options = PeersOptions {
            gossip: true,
            rpc: false,
            depth: 0,
            max_peers: 100,
            timeout_secs: 10,
            bootstrap: vec!["abc123".to_owned()],
            topic: Some("my-topic".to_owned()),
            topic_secret: Some("my-secret".to_owned()),
            no_default_bootstrap: false,
            no_default_topic: false,
            replace_defaults: false,
            no_relay: true,
            no_mdns: false,
        };

        assert!(options.gossip);
        assert!(!options.rpc);
        assert_eq!(options.bootstrap.len(), 1);
        assert_eq!(options.topic, Some("my-topic".to_owned()));
        assert_eq!(options.topic_secret, Some("my-secret".to_owned()));
    }

    #[test]
    fn test_mode_selection_default() {
        // When neither --gossip nor --rpc is set, both are used
        let (rpc, gossip) = (false, false);
        let use_rpc = rpc || !gossip;
        let use_gossip = gossip || !rpc;
        assert!(use_rpc);
        assert!(use_gossip);
    }

    #[test]
    fn test_mode_selection_rpc_only() {
        let rpc = true;
        let gossip = false;
        let use_rpc = rpc || !gossip;
        let use_gossip = gossip || !rpc;
        assert!(use_rpc);
        assert!(!use_gossip);
    }

    #[test]
    fn test_mode_selection_gossip_only() {
        let rpc = false;
        let gossip = true;
        let use_rpc = rpc || !gossip;
        let use_gossip = gossip || !rpc;
        assert!(!use_rpc);
        assert!(use_gossip);
    }

    #[test]
    fn test_mode_selection_both() {
        let rpc = true;
        let gossip = true;
        let use_rpc = rpc || !gossip;
        let use_gossip = gossip || !rpc;
        assert!(use_rpc);
        assert!(use_gossip);
    }
}
