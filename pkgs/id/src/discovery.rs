//! Peer discovery types and in-memory peer tracking.
//!
//! This module provides the core types for gossip-based peer discovery:
//!
//! - [`PeerAnnouncement`]: The message broadcast over gossip to advertise a server's presence
//! - [`PeerInfo`]: In-memory tracking of a peer including staleness information
//! - [`PeerDiscovery`]: Thread-safe peer table that collects and prunes peer announcements
//!
//! # Architecture
//!
//! ```text
//! Gossip Channel
//!   │
//!   │  PeerAnnouncement (postcard-serialized)
//!   ▼
//! PeerDiscovery (Arc<RwLock<HashMap<EndpointId, PeerInfo>>>)
//!   │
//!   ├── Queried by MetaProtocol (ListPeers RPC)
//!   ├── Queried by Web UI (/peers route)
//!   └── Pruned periodically (remove stale peers)
//! ```
//!
//! # Timing Constants
//!
//! - [`ANNOUNCE_INTERVAL`]: How often servers broadcast their presence (30s)
//! - [`STALE_THRESHOLD`]: Time since last announcement before a peer is stale (120s)
//! - [`STALE_CHECK_INTERVAL`]: How often the server prunes stale peers (60s)
//!
//! # Topic Identity
//!
//! - [`DEFAULT_TOPIC`]: The well-known topic name for the public discovery network
//! - [`DEFAULT_TOPIC_SECRET`]: The shared secret for the public network's DHT records

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};
use std::time::{Duration, Instant};

use iroh_base::EndpointId;
use serde::{Deserialize, Serialize};

// ============================================================================
// Constants
// ============================================================================

/// Default gossip topic name for peer discovery (hardcoded fallback).
///
/// All `id` servers join this topic by default to discover each other.
/// Used with `TopicId::new()` from `distributed-topic-tracker`, which
/// hashes the string via SHA-512 to produce a stable 32-byte topic hash.
///
/// Can be overridden with `--topic` CLI flag for private networks, or
/// by editing `defaults.conf` at build time.
pub const DEFAULT_TOPIC: &str = "id-peer-discovery-v1";

/// Default shared secret for the public discovery network (hardcoded fallback).
///
/// Used by `distributed-topic-tracker` to encrypt/decrypt DHT bootstrap
/// records. Only nodes with the same secret can discover each other via DHT.
///
/// Can be overridden with `--topic-secret` CLI flag for private networks, or
/// by editing `defaults.conf` at build time.
///
/// **Note:** This is public knowledge -- anyone can join the default network.
/// Private networks should use a unique secret.
pub const DEFAULT_TOPIC_SECRET: &[u8] = b"id-public-discovery-v1";

// ============================================================================
// Build-time defaults
// ============================================================================

/// Raw content of `defaults.conf`, embedded at compile time.
const DEFAULTS_CONF: &str = include_str!("defaults.conf");

/// Parsed build-time defaults, initialized once on first access.
static PARSED_DEFAULTS: LazyLock<Defaults> = LazyLock::new(|| parse_defaults(DEFAULTS_CONF));

/// Build-time defaults parsed from `defaults.conf`.
///
/// Values in this struct come from `src/defaults.conf`, which is embedded
/// into the binary via `include_str!()`. Edit that file before compiling
/// to change the defaults for a specific deployment.
///
/// # Resolution Order
///
/// ```text
/// const fallback (hardcoded in source)  ←  used if no conf file entry
///     ↓
/// build defaults (defaults.conf)        ←  --replace-defaults: use ONLY conf, skip hardcoded
///     ↓
///     + CLI --bootstrap, --topic, --topic-secret at serve time
///     ↓
///     - --no-default-bootstrap, --no-default-topic: skip defaults
///     ↓
/// effective config
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defaults {
    /// Bootstrap node IDs from the `[bootstrap]` section.
    pub bootstrap: Vec<String>,
    /// Topic name from the `[topic]` section (first non-comment line).
    pub topic: Option<String>,
    /// Topic secret from the `[topic_secret]` section (first non-comment line).
    pub topic_secret: Option<String>,
}

/// Parses a `defaults.conf`-formatted string into a [`Defaults`] struct.
///
/// Format:
/// - `[section]` starts a section
/// - `# comment` lines are ignored
/// - Blank lines are ignored
/// - Values are one per line; `[topic]` and `[topic_secret]` use only
///   the first non-comment value
///
/// # Example
///
/// ```rust
/// use id::discovery::parse_defaults;
///
/// let conf = r#"
/// [bootstrap]
/// abc123
/// def456
///
/// [topic]
/// my-custom-topic
///
/// [topic_secret]
/// my-secret
/// "#;
///
/// let defaults = parse_defaults(conf);
/// assert_eq!(defaults.bootstrap, vec!["abc123", "def456"]);
/// assert_eq!(defaults.topic.as_deref(), Some("my-custom-topic"));
/// assert_eq!(defaults.topic_secret.as_deref(), Some("my-secret"));
/// ```
pub fn parse_defaults(conf: &str) -> Defaults {
    let mut bootstrap = Vec::new();
    let mut topic: Option<String> = None;
    let mut topic_secret: Option<String> = None;

    let mut current_section: Option<&str> = None;

    for line in conf.lines() {
        let trimmed = line.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Section header
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current_section = Some(match &trimmed[1..trimmed.len() - 1] {
                "bootstrap" => "bootstrap",
                "topic" => "topic",
                "topic_secret" => "topic_secret",
                _ => "unknown",
            });
            continue;
        }

        // Value line — dispatch to the current section
        match current_section {
            Some("bootstrap") => {
                bootstrap.push(trimmed.to_owned());
            }
            Some("topic") if topic.is_none() => {
                topic = Some(trimmed.to_owned());
            }
            Some("topic_secret") if topic_secret.is_none() => {
                topic_secret = Some(trimmed.to_owned());
            }
            _ => {} // ignore unknown sections or extra values
        }
    }

    Defaults {
        bootstrap,
        topic,
        topic_secret,
    }
}

/// Returns the parsed build-time defaults from `defaults.conf`.
///
/// This is a cheap accessor — the file is parsed once and cached in a
/// `LazyLock`. The returned reference is `'static`.
pub fn defaults() -> &'static Defaults {
    &PARSED_DEFAULTS
}

/// Resolved configuration for peer discovery after applying the full
/// flag cascade.
///
/// Produced by [`resolve_config`]. All fields are ready to use — no
/// further defaulting is needed.
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    /// Bootstrap node IDs to contact on startup.
    pub bootstrap: Vec<String>,
    /// Gossip topic name.
    pub topic: String,
    /// Shared secret bytes for DHT record encryption.
    pub topic_secret: Vec<u8>,
}

/// Resolves the effective peer discovery configuration by applying the
/// full flag cascade.
///
/// # Resolution Order
///
/// 1. Start with hardcoded fallback constants.
/// 2. Override with values from `defaults.conf` (if present).
/// 3. If `replace_defaults` is `true`, skip the hardcoded fallback
///    entirely — only `defaults.conf` values are used.
/// 4. Append CLI-provided `bootstrap` node IDs.
/// 5. Override topic/secret with CLI values (if `Some`).
/// 6. If `no_default_bootstrap` is `true`, drop all default bootstrap
///    nodes (keep only CLI-provided ones).
/// 7. If `no_default_topic` is `true`, drop the default topic and
///    secret (keep only CLI-provided values; if none, fall back to
///    hardcoded constants to avoid empty values).
///
/// # Arguments
///
/// * `cli_bootstrap` - Node IDs supplied via `--bootstrap`
/// * `cli_topic` - Topic name supplied via `--topic`
/// * `cli_topic_secret` - Secret supplied via `--topic-secret`
/// * `replace_defaults` - `--replace-defaults`: use ONLY conf file values
/// * `no_default_bootstrap` - `--no-default-bootstrap`: skip default bootstrap nodes
/// * `no_default_topic` - `--no-default-topic`: skip default topic/secret
pub fn resolve_config(
    cli_bootstrap: &[String],
    cli_topic: Option<&str>,
    cli_topic_secret: Option<&str>,
    replace_defaults: bool,
    no_default_bootstrap: bool,
    no_default_topic: bool,
) -> ResolvedConfig {
    let defs = defaults();

    // --- Bootstrap ---
    let mut bootstrap: Vec<String> = if no_default_bootstrap {
        // Skip all defaults — only CLI-provided
        Vec::new()
    } else if replace_defaults {
        // Only conf file values (skip hardcoded — but there are no hardcoded
        // bootstrap nodes currently, so this is the same as the default path)
        defs.bootstrap.clone()
    } else {
        // Hardcoded fallback + conf file (currently no hardcoded bootstrap
        // nodes, so this is just the conf file values)
        defs.bootstrap.clone()
    };
    // Append CLI-provided bootstrap nodes, then deduplicate (preserving order)
    bootstrap.extend_from_slice(cli_bootstrap);
    let mut seen = std::collections::HashSet::new();
    bootstrap.retain(|v| seen.insert(v.clone()));

    // --- Topic ---
    let topic = if let Some(t) = cli_topic {
        // CLI takes precedence
        t.to_owned()
    } else if no_default_topic {
        // Skip defaults — use hardcoded constant as last resort to
        // avoid an empty topic name
        DEFAULT_TOPIC.to_owned()
    } else if replace_defaults {
        // Only conf file value
        defs.topic
            .clone()
            .unwrap_or_else(|| DEFAULT_TOPIC.to_owned())
    } else {
        // Conf file value, falling back to hardcoded
        defs.topic
            .clone()
            .unwrap_or_else(|| DEFAULT_TOPIC.to_owned())
    };

    // --- Topic Secret ---
    let topic_secret = if let Some(s) = cli_topic_secret {
        s.as_bytes().to_vec()
    } else if no_default_topic {
        DEFAULT_TOPIC_SECRET.to_vec()
    } else if replace_defaults {
        defs.topic_secret
            .as_ref()
            .map_or_else(|| DEFAULT_TOPIC_SECRET.to_vec(), |s| s.as_bytes().to_vec())
    } else {
        defs.topic_secret
            .as_ref()
            .map_or_else(|| DEFAULT_TOPIC_SECRET.to_vec(), |s| s.as_bytes().to_vec())
    };

    ResolvedConfig {
        bootstrap,
        topic,
        topic_secret,
    }
}

/// How often servers broadcast a [`PeerAnnouncement`] over gossip.
///
/// Set to 30 seconds. Each broadcast contains the server's current
/// node ID, name, blob count, and timestamp.
pub const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(30);

/// Time since last announcement before a peer is considered stale.
///
/// Set to 120 seconds (4 missed announcements at 30s intervals).
/// Stale peers are removed during periodic cleanup.
pub const STALE_THRESHOLD: Duration = Duration::from_secs(120);

/// How often the server runs stale peer cleanup.
///
/// Set to 60 seconds. Peers older than [`STALE_THRESHOLD`] are removed.
pub const STALE_CHECK_INTERVAL: Duration = Duration::from_secs(60);

// ============================================================================
// Types
// ============================================================================

/// A peer's presence announcement, broadcast over the gossip channel.
///
/// Serialized with [postcard](https://docs.rs/postcard) for compact binary
/// encoding. Typical size: ~50-100 bytes depending on name length.
///
/// # Fields
///
/// - `node_id`: The peer's Ed25519 public key (32 bytes)
/// - `name`: Optional human-readable server name
/// - `blob_count`: Number of blobs currently stored
/// - `timestamp_secs`: Unix timestamp when the announcement was created
///
/// # Example
///
/// ```rust
/// use id::discovery::PeerAnnouncement;
/// use iroh_base::SecretKey;
///
/// let key = SecretKey::generate(&mut rand::rng());
/// let announcement = PeerAnnouncement {
///     node_id: key.public(),
///     name: Some("my-server".to_string()),
///     blob_count: 42,
///     timestamp_secs: 1700000000,
/// };
///
/// // Serialize for gossip broadcast
/// let bytes = postcard::to_allocvec(&announcement).unwrap();
/// let decoded: PeerAnnouncement = postcard::from_bytes(&bytes).unwrap();
/// assert_eq!(decoded.blob_count, 42);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeerAnnouncement {
    /// The peer's Ed25519 public key, serving as its unique identity.
    pub node_id: EndpointId,
    /// Optional human-readable name for this server.
    pub name: Option<String>,
    /// Number of blobs currently stored on this peer.
    pub blob_count: u64,
    /// Unix timestamp (seconds) when this announcement was created.
    pub timestamp_secs: u64,
}

/// In-memory tracking of a discovered peer.
///
/// Wraps a [`PeerAnnouncement`] with local tracking information
/// (when the peer was last seen). Not serialized -- only used
/// within the server process.
#[derive(Debug, Clone)]
pub struct PeerInfo {
    /// The most recent announcement from this peer.
    pub announcement: PeerAnnouncement,
    /// When this peer was last seen (local monotonic clock).
    pub last_seen: Instant,
}

/// Thread-safe in-memory peer table.
///
/// Collects [`PeerAnnouncement`] messages from the gossip channel and
/// tracks peer liveness. Provides methods to query known peers and
/// prune stale entries.
///
/// This type is cheaply cloneable (backed by `Arc`) and safe to share
/// across async tasks.
///
/// # Example
///
/// ```rust
/// use id::discovery::{PeerDiscovery, PeerAnnouncement};
/// use iroh_base::SecretKey;
///
/// let discovery = PeerDiscovery::new();
///
/// let node_id = SecretKey::from_bytes(&[1u8; 32]).public();
/// let announcement = PeerAnnouncement {
///     node_id,
///     name: Some("peer-1".to_string()),
///     blob_count: 10,
///     timestamp_secs: 1700000000,
/// };
///
/// discovery.update(announcement);
/// assert_eq!(discovery.count(), 1);
///
/// let peers = discovery.peers();
/// assert_eq!(peers.len(), 1);
/// assert_eq!(peers[0].announcement.name, Some("peer-1".to_string()));
/// ```
#[derive(Debug, Clone)]
pub struct PeerDiscovery {
    /// Map from node ID to peer info, protected by a read-write lock.
    peers: Arc<RwLock<HashMap<EndpointId, PeerInfo>>>,
}

impl PeerDiscovery {
    /// Creates a new empty peer discovery table.
    pub fn new() -> Self {
        Self {
            peers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Updates the peer table with a new announcement.
    ///
    /// If the peer is already known, its announcement and last-seen
    /// timestamp are updated. If the peer is new, it is inserted.
    pub fn update(&self, announcement: PeerAnnouncement) {
        let node_id = announcement.node_id;
        let info = PeerInfo {
            announcement,
            last_seen: Instant::now(),
        };
        if let Ok(mut peers) = self.peers.write() {
            peers.insert(node_id, info);
        }
    }

    /// Returns all non-stale peers.
    ///
    /// A peer is considered stale if it hasn't been seen for longer
    /// than [`STALE_THRESHOLD`]. Stale peers are excluded from the
    /// result but not removed from the table (use [`remove_stale`](Self::remove_stale)
    /// for that).
    pub fn peers(&self) -> Vec<PeerInfo> {
        let Ok(peers) = self.peers.read() else {
            return Vec::new();
        };
        let now = Instant::now();
        peers
            .values()
            .filter(|info| now.duration_since(info.last_seen) < STALE_THRESHOLD)
            .cloned()
            .collect()
    }

    /// Returns all peers including stale ones.
    ///
    /// Unlike [`peers`](Self::peers), this returns every peer in the table
    /// regardless of when it was last seen. Useful for displaying full
    /// peer history.
    pub fn all_peers(&self) -> Vec<PeerInfo> {
        let Ok(peers) = self.peers.read() else {
            return Vec::new();
        };
        peers.values().cloned().collect()
    }

    /// Removes peers that haven't been seen for longer than `max_age`.
    ///
    /// Call this periodically (every [`STALE_CHECK_INTERVAL`]) to keep
    /// the peer table from growing unboundedly.
    pub fn remove_stale(&self, max_age: Duration) {
        if let Ok(mut peers) = self.peers.write() {
            let now = Instant::now();
            peers.retain(|_, info| now.duration_since(info.last_seen) < max_age);
        }
    }

    /// Returns peers that have exceeded [`STALE_THRESHOLD`] but are still
    /// in the table.
    ///
    /// Use this to identify peers that should be probed before removal.
    pub fn stale_peers(&self) -> Vec<PeerInfo> {
        let Ok(peers) = self.peers.read() else {
            return Vec::new();
        };
        let now = Instant::now();
        peers
            .values()
            .filter(|info| now.duration_since(info.last_seen) >= STALE_THRESHOLD)
            .cloned()
            .collect()
    }

    /// Refreshes the `last_seen` timestamp for a peer, preventing it from
    /// being considered stale.
    ///
    /// Returns `true` if the peer was found and refreshed, `false` if the
    /// peer is not in the table.
    pub fn refresh(&self, node_id: &EndpointId) -> bool {
        if let Ok(mut peers) = self.peers.write()
            && let Some(info) = peers.get_mut(node_id)
        {
            info.last_seen = Instant::now();
            return true;
        }
        false
    }

    /// Returns the number of peers in the table (including stale).
    pub fn count(&self) -> usize {
        self.peers.read().map_or(0, |peers| peers.len())
    }
}

impl Default for PeerDiscovery {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unchecked_duration_subtraction
)]
mod tests {
    use super::*;

    fn make_node_id(seed: u8) -> EndpointId {
        // Use a deterministic secret key derived from the seed byte
        let mut key_bytes = [0u8; 32];
        key_bytes[0] = seed;
        let secret = iroh_base::SecretKey::from_bytes(&key_bytes);
        secret.public()
    }

    fn make_announcement(byte: u8, name: Option<&str>, blob_count: u64) -> PeerAnnouncement {
        PeerAnnouncement {
            node_id: make_node_id(byte),
            name: name.map(ToOwned::to_owned),
            blob_count,
            timestamp_secs: 1_700_000_000,
        }
    }

    #[test]
    fn test_announcement_serialization_roundtrip() {
        let announcement = make_announcement(1, Some("test-peer"), 42);
        let bytes = postcard::to_allocvec(&announcement).unwrap();
        let decoded: PeerAnnouncement = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, announcement);
    }

    #[test]
    fn test_announcement_serialization_no_name() {
        let announcement = make_announcement(2, None, 0);
        let bytes = postcard::to_allocvec(&announcement).unwrap();
        let decoded: PeerAnnouncement = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, announcement);
        assert!(decoded.name.is_none());
    }

    #[test]
    fn test_discovery_new_is_empty() {
        let discovery = PeerDiscovery::new();
        assert_eq!(discovery.count(), 0);
        assert!(discovery.peers().is_empty());
    }

    #[test]
    fn test_discovery_default_is_empty() {
        let discovery = PeerDiscovery::default();
        assert_eq!(discovery.count(), 0);
    }

    #[test]
    fn test_discovery_update_inserts_peer() {
        let discovery = PeerDiscovery::new();
        discovery.update(make_announcement(1, Some("peer-1"), 10));
        assert_eq!(discovery.count(), 1);

        let peers = discovery.peers();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].announcement.name, Some("peer-1".to_owned()));
        assert_eq!(peers[0].announcement.blob_count, 10);
    }

    #[test]
    fn test_discovery_update_replaces_existing() {
        let discovery = PeerDiscovery::new();
        discovery.update(make_announcement(1, Some("peer-1"), 10));
        discovery.update(make_announcement(1, Some("peer-1-updated"), 20));

        assert_eq!(discovery.count(), 1);
        let peers = discovery.peers();
        assert_eq!(
            peers[0].announcement.name,
            Some("peer-1-updated".to_owned())
        );
        assert_eq!(peers[0].announcement.blob_count, 20);
    }

    #[test]
    fn test_discovery_multiple_peers() {
        let discovery = PeerDiscovery::new();
        discovery.update(make_announcement(1, Some("peer-1"), 10));
        discovery.update(make_announcement(2, Some("peer-2"), 20));
        discovery.update(make_announcement(3, Some("peer-3"), 30));

        assert_eq!(discovery.count(), 3);
        assert_eq!(discovery.peers().len(), 3);
    }

    #[test]
    fn test_discovery_remove_stale() {
        let discovery = PeerDiscovery::new();
        discovery.update(make_announcement(1, Some("peer-1"), 10));

        // Remove with zero max_age -- everything is stale
        discovery.remove_stale(Duration::from_secs(0));
        assert_eq!(discovery.count(), 0);
    }

    #[test]
    fn test_discovery_remove_stale_keeps_recent() {
        let discovery = PeerDiscovery::new();
        discovery.update(make_announcement(1, Some("peer-1"), 10));

        // Remove with very large max_age -- nothing is stale
        discovery.remove_stale(Duration::from_secs(3600));
        assert_eq!(discovery.count(), 1);
    }

    #[test]
    fn test_discovery_clone_shares_state() {
        let discovery = PeerDiscovery::new();
        let clone = discovery.clone();

        discovery.update(make_announcement(1, Some("peer-1"), 10));
        assert_eq!(clone.count(), 1);

        clone.update(make_announcement(2, Some("peer-2"), 20));
        assert_eq!(discovery.count(), 2);
    }

    #[test]
    fn test_discovery_all_peers_includes_everything() {
        let discovery = PeerDiscovery::new();
        discovery.update(make_announcement(1, Some("peer-1"), 10));
        discovery.update(make_announcement(2, Some("peer-2"), 20));

        let all = discovery.all_peers();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_stale_peers_returns_empty_when_all_fresh() {
        let discovery = PeerDiscovery::new();
        discovery.update(make_announcement(1, Some("peer-1"), 10));
        discovery.update(make_announcement(2, Some("peer-2"), 20));

        // Just inserted, so nothing is stale
        assert!(discovery.stale_peers().is_empty());
    }

    #[test]
    fn test_stale_peers_returns_stale_entries() {
        let discovery = PeerDiscovery::new();
        let node_id = make_node_id(1);

        // Manually insert a peer with an old last_seen
        {
            let mut peers = discovery.peers.write().unwrap();
            peers.insert(
                node_id,
                PeerInfo {
                    announcement: make_announcement(1, Some("old-peer"), 5),
                    last_seen: Instant::now() - (STALE_THRESHOLD + Duration::from_secs(1)),
                },
            );
        }

        let stale = discovery.stale_peers();
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].announcement.node_id, node_id);
    }

    #[test]
    fn test_stale_peers_excludes_fresh_entries() {
        let discovery = PeerDiscovery::new();
        let stale_id = make_node_id(1);

        // Insert one stale peer and one fresh peer
        {
            let mut peers = discovery.peers.write().unwrap();
            peers.insert(
                stale_id,
                PeerInfo {
                    announcement: make_announcement(1, Some("stale"), 5),
                    last_seen: Instant::now() - (STALE_THRESHOLD + Duration::from_secs(1)),
                },
            );
        }
        discovery.update(make_announcement(2, Some("fresh"), 10));

        let stale = discovery.stale_peers();
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].announcement.node_id, stale_id);

        // peers() should only return fresh ones
        let fresh = discovery.peers();
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].announcement.name, Some("fresh".to_owned()));
    }

    #[test]
    fn test_refresh_existing_peer() {
        let discovery = PeerDiscovery::new();
        let node_id = make_node_id(1);

        // Insert a stale peer
        {
            let mut peers = discovery.peers.write().unwrap();
            peers.insert(
                node_id,
                PeerInfo {
                    announcement: make_announcement(1, Some("stale"), 5),
                    last_seen: Instant::now() - (STALE_THRESHOLD + Duration::from_secs(1)),
                },
            );
        }

        // Should be stale before refresh
        assert_eq!(discovery.stale_peers().len(), 1);

        // Refresh it
        assert!(discovery.refresh(&node_id));

        // Should no longer be stale
        assert!(discovery.stale_peers().is_empty());
        assert_eq!(discovery.peers().len(), 1);
    }

    #[test]
    fn test_refresh_nonexistent_peer() {
        let discovery = PeerDiscovery::new();
        let node_id = make_node_id(99);

        // Refreshing a peer that doesn't exist returns false
        assert!(!discovery.refresh(&node_id));
    }

    #[test]
    fn test_refresh_prevents_removal_by_remove_stale() {
        let discovery = PeerDiscovery::new();
        let alive_id = make_node_id(1);
        let dead_id = make_node_id(2);

        // Insert two stale peers
        {
            let mut peers = discovery.peers.write().unwrap();
            let stale_time = Instant::now() - (STALE_THRESHOLD + Duration::from_secs(1));
            peers.insert(
                alive_id,
                PeerInfo {
                    announcement: make_announcement(1, Some("alive"), 5),
                    last_seen: stale_time,
                },
            );
            peers.insert(
                dead_id,
                PeerInfo {
                    announcement: make_announcement(2, Some("dead"), 3),
                    last_seen: stale_time,
                },
            );
        }

        assert_eq!(discovery.stale_peers().len(), 2);

        // Refresh only the "alive" peer (simulating a successful probe)
        discovery.refresh(&alive_id);

        // Now remove stale — only "dead" should be removed
        discovery.remove_stale(STALE_THRESHOLD);

        assert_eq!(discovery.count(), 1);
        let remaining = discovery.peers();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].announcement.node_id, alive_id);
    }

    #[test]
    fn test_constants() {
        assert_eq!(ANNOUNCE_INTERVAL, Duration::from_secs(30));
        assert_eq!(STALE_THRESHOLD, Duration::from_secs(120));
        assert_eq!(STALE_CHECK_INTERVAL, Duration::from_secs(60));
        assert_eq!(DEFAULT_TOPIC, "id-peer-discovery-v1");
        assert_eq!(DEFAULT_TOPIC_SECRET, b"id-public-discovery-v1");
    }

    // ========================================================================
    // parse_defaults / resolve_config tests
    // ========================================================================

    #[test]
    fn test_parse_defaults_full() {
        let conf = r"
[bootstrap]
abc123
def456

[topic]
my-topic

[topic_secret]
my-secret
";
        let d = parse_defaults(conf);
        assert_eq!(d.bootstrap, vec!["abc123", "def456"]);
        assert_eq!(d.topic.as_deref(), Some("my-topic"));
        assert_eq!(d.topic_secret.as_deref(), Some("my-secret"));
    }

    #[test]
    fn test_parse_defaults_empty() {
        let d = parse_defaults("");
        assert!(d.bootstrap.is_empty());
        assert!(d.topic.is_none());
        assert!(d.topic_secret.is_none());
    }

    #[test]
    fn test_parse_defaults_comments_only() {
        let conf = r"
# just a comment
[bootstrap]
# no actual bootstrap nodes

[topic]
# no topic either
";
        let d = parse_defaults(conf);
        assert!(d.bootstrap.is_empty());
        assert!(d.topic.is_none());
    }

    #[test]
    fn test_parse_defaults_only_first_topic_value() {
        let conf = r"
[topic]
first-topic
second-topic-ignored
";
        let d = parse_defaults(conf);
        assert_eq!(d.topic.as_deref(), Some("first-topic"));
    }

    #[test]
    fn test_parse_defaults_unknown_section_ignored() {
        let conf = r"
[unknown]
some-value
[bootstrap]
node1
";
        let d = parse_defaults(conf);
        assert_eq!(d.bootstrap, vec!["node1"]);
    }

    #[test]
    fn test_parse_defaults_embedded_file() {
        // The actual embedded defaults.conf should parse without error
        let d = parse_defaults(DEFAULTS_CONF);
        // The shipped file has a topic and topic_secret but no bootstrap nodes
        assert!(d.bootstrap.is_empty());
        assert_eq!(d.topic.as_deref(), Some("id-peer-discovery-v1"));
        assert_eq!(d.topic_secret.as_deref(), Some("id-public-discovery-v1"));
    }

    #[test]
    fn test_defaults_accessor() {
        let d = defaults();
        // Should match the embedded file
        assert_eq!(d.topic.as_deref(), Some("id-peer-discovery-v1"));
    }

    #[test]
    fn test_resolve_config_all_defaults() {
        let cfg = resolve_config(&[], None, None, false, false, false);
        assert_eq!(cfg.topic, "id-peer-discovery-v1");
        assert_eq!(cfg.topic_secret, b"id-public-discovery-v1");
        assert!(cfg.bootstrap.is_empty()); // no bootstrap in default conf
    }

    #[test]
    fn test_resolve_config_cli_overrides() {
        let cli_bs = vec!["node1".to_owned()];
        let cfg = resolve_config(
            &cli_bs,
            Some("custom-topic"),
            Some("custom-secret"),
            false,
            false,
            false,
        );
        assert_eq!(cfg.bootstrap, vec!["node1"]);
        assert_eq!(cfg.topic, "custom-topic");
        assert_eq!(cfg.topic_secret, b"custom-secret");
    }

    #[test]
    fn test_resolve_config_no_default_bootstrap() {
        let cli_bs = vec!["cli-node".to_owned()];
        let cfg = resolve_config(&cli_bs, None, None, false, true, false);
        // Only CLI bootstrap, no defaults
        assert_eq!(cfg.bootstrap, vec!["cli-node"]);
    }

    #[test]
    fn test_resolve_config_no_default_topic() {
        let cfg = resolve_config(&[], None, None, false, false, true);
        // Falls back to hardcoded constant
        assert_eq!(cfg.topic, DEFAULT_TOPIC);
        assert_eq!(cfg.topic_secret, DEFAULT_TOPIC_SECRET);
    }

    #[test]
    fn test_resolve_config_no_default_topic_with_cli() {
        let cfg = resolve_config(
            &[],
            Some("cli-topic"),
            Some("cli-secret"),
            false,
            false,
            true,
        );
        // CLI takes precedence even when no_default_topic is set
        assert_eq!(cfg.topic, "cli-topic");
        assert_eq!(cfg.topic_secret, b"cli-secret");
    }

    #[test]
    fn test_resolve_config_replace_defaults() {
        let cfg = resolve_config(&[], None, None, true, false, false);
        // replace_defaults uses conf file values (which match hardcoded for default conf)
        assert_eq!(cfg.topic, "id-peer-discovery-v1");
    }

    #[test]
    fn test_resolve_config_cli_bootstrap_appended_to_defaults() {
        // Even with no default bootstrap in the conf file, CLI nodes are appended
        let cli_bs = vec!["a".to_owned(), "b".to_owned()];
        let cfg = resolve_config(&cli_bs, None, None, false, false, false);
        assert_eq!(cfg.bootstrap, vec!["a", "b"]);
    }

    #[test]
    fn test_resolve_config_deduplicates_bootstrap() {
        // Duplicate entries should be removed, preserving first occurrence order
        let cli_bs = vec![
            "a".to_owned(),
            "b".to_owned(),
            "a".to_owned(),
            "c".to_owned(),
        ];
        let cfg = resolve_config(&cli_bs, None, None, false, false, false);
        assert_eq!(cfg.bootstrap, vec!["a", "b", "c"]);
    }
}
