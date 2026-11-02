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
use iroh_base::EndpointId;
use iroh_mdns_address_lookup::MdnsAddressLookup;

use crate::discovery::{PeerAnnouncement, resolve_config};
use crate::protocol::{MetaRequest, MetaResponse};
use crate::store::load_or_create_keypair;
use crate::{CLIENT_KEY_FILE, META_ALPN, is_node_id};

use super::client::create_local_client_endpoint;
use super::serve::get_serve_info;
use super::text::plain_field;

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
///
/// Peer names arrive over unsigned gossip and are stored and served verbatim,
/// so the printer is where row structure is protected: a name containing a
/// tab, CR, LF or other structure-breaking character is quoted and escaped by
/// [`plain_field`] instead of shifting a column or forging a row (#1613).
pub async fn cmd_peers(node: Option<String>, options: PeersOptions) -> Result<()> {
    if options.depth < 0 {
        bail!(
            "peer crawl depth must be non-negative, got {}",
            options.depth
        );
    }

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
            .await?;
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
        peers.sort_by_key(|a| a.node_id.to_string());
        for peer in peers {
            println!("{}", format_peer_row(&peer));
        }
    }

    Ok(())
}

/// One peer as the four tab-separated fields of the `id peers` row:
/// `<node_id>\t<name>\t<blob_count>\t<timestamp>`.
///
/// The name comes from an unsigned gossip announcement and is stored and
/// served verbatim, so it is rendered through [`plain_field`]: a name holding
/// a tab, CR, LF or other structure-breaking character is quoted and escaped
/// instead of shifting columns or forging a row (#1613).
pub(crate) fn format_peer_row(peer: &PeerAnnouncement) -> String {
    let name = peer
        .name
        .as_deref()
        .map_or_else(|| "-".to_owned(), plain_field);
    format!(
        "{}\t{}\t{}\t{}",
        peer.node_id, name, peer.blob_count, peer.timestamp_secs
    )
}

/// Queries the local serve instance for its peer list via RPC.
async fn query_local_peers() -> Result<Vec<PeerAnnouncement>> {
    let serve_info = get_serve_info()
        .await
        .ok_or_else(|| anyhow::anyhow!("no local serve running"))?;

    let (endpoint, endpoint_addr) = create_local_client_endpoint(&serve_info).await?;
    let result = async {
        let meta_conn = crate::connectivity::connect(&endpoint, endpoint_addr, META_ALPN).await?;
        let (mut send, mut recv) = meta_conn.open_bi().await?;
        let req = postcard::to_allocvec(&MetaRequest::ListPeers)?;
        send.write_all(&req).await?;
        send.finish()?;
        let resp_buf = crate::meta_client::read_capped_response(&mut recv).await?;
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
    let mut builder = crate::connectivity::public_builder(
        crate::connectivity::RelayPolicy::from_no_relay(no_relay),
    )
    .secret_key(client_key);
    if !no_mdns {
        builder = builder.address_lookup(MdnsAddressLookup::builder());
    }
    let endpoint = builder.bind().await?;

    let result = async {
        let meta_conn = crate::connectivity::connect(&endpoint, node_id, META_ALPN).await?;
        let (mut send, mut recv) = meta_conn.open_bi().await?;
        let req = postcard::to_allocvec(&MetaRequest::ListPeers)?;
        send.write_all(&req).await?;
        send.finish()?;
        let resp_buf = crate::meta_client::read_capped_response(&mut recv).await?;
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
) -> Result<Vec<PeerAnnouncement>> {
    if depth < 0 {
        bail!("peer crawl depth must be non-negative, got {depth}");
    }

    let mut all_peers: HashMap<EndpointId, PeerAnnouncement> = HashMap::new();
    let mut visited: HashSet<EndpointId> = HashSet::new();
    let mut frontier: Vec<EndpointId> = Vec::new();

    // Initialize with known peers
    for peer in initial_peers {
        all_peers.insert(peer.node_id, peer.clone());
        frontier.push(peer.node_id);
    }

    let mut current_depth = 0;
    let timeout = tokio::time::Duration::from_secs(timeout_secs);

    while current_depth < depth && !frontier.is_empty() {
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

    Ok(all_peers.into_values().collect())
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
    let mut builder = crate::connectivity::public_builder(
        crate::connectivity::RelayPolicy::from_no_relay(options.no_relay),
    )
    .secret_key(client_key.clone());
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

    let record_publisher = RecordPublisher::new(
        dtt_topic_id,
        dalek_signing_key,
        None,
        config.topic_secret,
        distributed_topic_tracker::Config::default(),
    );

    let gossip_topic = gossip
        .subscribe_and_join_with_auto_discovery_no_wait(record_publisher)
        .await?;
    let (sender, mut receiver) = gossip_topic.split().await?;

    // Join bootstrap peers (defaults + CLI, already merged by resolve_config)
    let bootstrap_ids: Vec<EndpointId> = parse_bootstrap_ids(&config.bootstrap);
    if !bootstrap_ids.is_empty() {
        tracing::info!("joining {} bootstrap peer(s)", bootstrap_ids.len());
        sender.join_peers(bootstrap_ids, None).await?;
    }

    // Collect announcements for a limited time
    let collect_timeout = tokio::time::Duration::from_secs(options.timeout_secs);
    let mut peers: HashMap<EndpointId, PeerAnnouncement> = HashMap::new();

    let _ = tokio::time::timeout(collect_timeout, async {
        while let Ok(event) = receiver.next().await {
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

/// Parse bootstrap node IDs with serve parity (#2016): warn per invalid
/// entry, naming the value, and keep the valid IDs. The previous
/// `filter_map(... .ok())` silently dropped typos - and when no entry
/// survived, join_peers was skipped entirely, so a bad bootstrap ID
/// surfaced only as a full-timeout "no peers" result that looked like a
/// network problem. Serve (serve.rs) and access.rs both warn-and-continue
/// on the same input; a hard error would break otherwise-working joins
/// over one stale entry in the merged defaults + config + CLI list.
fn parse_bootstrap_ids(entries: &[String]) -> Vec<EndpointId> {
    entries
        .iter()
        .filter_map(|id_str| {
            id_str.parse::<EndpointId>().ok().or_else(|| {
                tracing::warn!("invalid bootstrap node ID: {}", id_str);
                None
            })
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::discovery::PeerDiscovery;
    use iroh_base::SecretKey;

    #[tokio::test]
    async fn crawl_peers_refuses_negative_depth_before_networking() {
        let err = crawl_peers(&[], -1, 1000, 30, false, false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("depth must be non-negative"));
    }

    fn now_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn announcement(byte: u8, name: Option<&str>) -> PeerAnnouncement {
        PeerAnnouncement {
            node_id: SecretKey::from_bytes(&[byte; 32]).public(),
            name: name.map(ToOwned::to_owned),
            blob_count: 7,
            // Fresh stamp: PeerDiscovery::update refuses announcements
            // older than STALE_THRESHOLD relative to the wall clock.
            timestamp_secs: now_secs(),
        }
    }

    fn valid_id_string(byte: u8) -> String {
        SecretKey::from_bytes(&[byte; 32]).public().to_string()
    }

    #[test]
    fn test_parse_bootstrap_ids_keeps_valid_and_drops_invalid() {
        // Mixed input (#2016): the one real 64-hex EndpointId survives;
        // invalid entries take the warning path and are dropped.
        let valid = valid_id_string(7);
        let entries = vec![
            "not-a-node-id".to_string(),
            valid.clone(),
            "zzzz".to_string(),
        ];
        let parsed = parse_bootstrap_ids(&entries);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].to_string(), valid);
    }

    #[test]
    fn test_parse_bootstrap_ids_all_invalid_yields_empty() {
        let entries = vec!["nope".to_string(), "123".to_string(), String::new()];
        assert!(parse_bootstrap_ids(&entries).is_empty());
    }

    #[test]
    fn test_parse_bootstrap_ids_valid_parses_unchanged() {
        let valid = valid_id_string(9);
        let parsed = parse_bootstrap_ids(std::slice::from_ref(&valid));
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].to_string(), valid);
    }

    fn fields(row: &str) -> Vec<&str> {
        row.split('\t').collect()
    }

    fn assert_one_row_four_fields(row: &str) {
        assert!(!row.contains('\n'), "row must be one line: {row:?}");
        assert!(!row.contains('\r'), "row must be one line: {row:?}");
        assert_eq!(fields(row).len(), 4, "row must have four fields: {row:?}");
    }

    #[test]
    fn test_peer_row_control_character_names_stay_one_row() {
        for name in [
            "a\nb",
            "a\rb",
            "a\tb",
            "a\r\nb",
            "\n",
            "\u{0}",
            "x\nforged\trow\t0",
        ] {
            let row = format_peer_row(&announcement(1, Some(name)));
            assert_one_row_four_fields(&row);
            assert!(
                !row.contains(name),
                "the raw name must not reach the row: {row:?}"
            );
        }
    }

    #[test]
    fn test_peer_row_ordinary_names_display_unchanged() {
        for name in ["my-server", "my server", "naïve-✦", "サーバー"] {
            let row = format_peer_row(&announcement(1, Some(name)));
            assert_eq!(fields(&row)[1], name, "ordinary name must be unchanged");
        }
    }

    #[test]
    fn test_peer_row_without_a_name_prints_a_dash() {
        let row = format_peer_row(&announcement(1, None));
        assert_eq!(fields(&row)[1], "-");
    }

    /// Ingest -> `ListPeers` -> both printers, for names a gossip announcement
    /// can carry verbatim (#1613).
    #[test]
    fn test_forged_names_through_ingest_and_list_peers_cannot_forge_rows() {
        let discovery = PeerDiscovery::new();
        let names = [
            Some("forged\nname\twith\ttabs"),
            Some("carriage\rreturn"),
            Some("plain-peer"),
            Some("naïve peer"),
        ];
        for (index, name) in names.iter().enumerate() {
            let announced = announcement(u8::try_from(index + 1).unwrap(), *name);
            // Gossip ingest: postcard on the wire, then stored verbatim.
            let wire = postcard::to_allocvec(&announced).unwrap();
            discovery.update(postcard::from_bytes(&wire).unwrap());
        }

        // ListPeers serving, as protocol.rs builds it from the peer table.
        let served = MetaResponse::ListPeers {
            peers: discovery
                .peers()
                .into_iter()
                .map(|info| info.announcement)
                .collect(),
        };
        // The client reads it off the wire.
        let wire = postcard::to_allocvec(&served).unwrap();
        let MetaResponse::ListPeers { peers } = postcard::from_bytes(&wire).unwrap() else {
            panic!("expected ListPeers");
        };

        assert_eq!(peers.len(), names.len());
        // Stored and served values stay verbatim; only the printers escape.
        for name in names.into_iter().flatten() {
            assert!(
                peers.iter().any(|peer| peer.name.as_deref() == Some(name)),
                "announcement for {name:?} must be served verbatim"
            );
        }

        let now = now_secs() + 120;
        let mut rows: Vec<String> = peers.iter().map(format_peer_row).collect();
        rows.extend(
            peers
                .iter()
                .map(|peer| crate::commands::repl::format_peer_line(peer, now)),
        );

        assert_eq!(rows.len(), names.len() * 2, "one row per peer per printer");
        for row in &rows {
            assert_one_row_four_fields(row);
        }
    }

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
