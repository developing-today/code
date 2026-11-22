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
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use iroh_base::EndpointId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
/// by editing `defaults.json` at build time.
pub const DEFAULT_TOPIC: &str = "id-peer-discovery-v1";

/// Default shared secret for the public discovery network (hardcoded fallback).
///
/// Used by `distributed-topic-tracker` to encrypt/decrypt DHT bootstrap
/// records. Only nodes with the same secret can discover each other via DHT.
///
/// Can be overridden with `--topic-secret` CLI flag for private networks, or
/// by editing `defaults.json` at build time.
///
/// **Note:** This is public knowledge -- anyone can join the default network.
/// Private networks should use a unique secret.
pub const DEFAULT_TOPIC_SECRET: &[u8] = b"id-public-discovery-v1";

// ============================================================================
// Build-time defaults
// ============================================================================

/// Raw content of `defaults.json`, embedded at compile time.
const DEFAULTS_JSON: &str = include_str!("defaults.json");

/// Build-time replace/append policy for list defaults.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefaultsMode {
    /// Keep built-in defaults and append file values.
    #[default]
    Append,
    /// Use file lists instead of built-in lists.
    Replace,
}

/// Typed defaults parsed from the embedded JSON file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Defaults {
    /// Bootstrap node IDs.
    pub bootstrap: Vec<String>,
    /// Topic name (must be supplied with `topic_secret`).
    pub topic: Option<String>,
    /// Topic secret (must be supplied with `topic`).
    pub topic_secret: Option<String>,
    /// Web listener addresses.
    pub web_bindings: Vec<String>,
    /// Relay policy selector.
    pub relay: crate::connectivity::RelayPolicy,
    /// Whether gossip is enabled by default.
    pub gossip: bool,
    /// List merge mode.
    pub mode: DefaultsMode,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            bootstrap: Vec::new(),
            topic: None,
            topic_secret: None,
            web_bindings: Vec::new(),
            relay: crate::connectivity::RelayPolicy::N0,
            gossip: true,
            mode: DefaultsMode::Append,
        }
    }
}

impl DefaultsMode {
    /// Stable lowercase spelling used in provenance output.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Append => "append",
            Self::Replace => "replace",
        }
    }
}

/// Origin metadata for the embedded defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultsProvenance {
    /// Whether the binary uses only the code constants or an effective file.
    pub source: DefaultsSource,
    /// SHA-256 of the exact embedded JSON bytes, when effective file values exist.
    pub sha256: Option<String>,
    /// List merge mode when file values are effective.
    pub mode: Option<DefaultsMode>,
}

impl DefaultsProvenance {
    /// Formats the origin in a stable form suitable for logs and `--version`.
    pub fn describe(&self) -> String {
        match self.source {
            DefaultsSource::ConstsOnly => "defaults: consts".to_owned(),
            DefaultsSource::File => format!(
                "defaults: file sha256={} mode={}",
                self.sha256.as_deref().unwrap_or("unknown"),
                self.mode.unwrap_or_default().as_str(),
            ),
        }
    }
}

/// Defaults source reported by the binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultsSource {
    /// Embedded file has no effective values.
    ConstsOnly,
    /// One or more embedded-file values affect defaults.
    File,
}

// The file is embedded at build time and the test
// `embedded_defaults_are_valid_and_have_provenance` parses it, so a
// failure here is a build defect. Stopping is the safe outcome: a
// binary must not start with network defaults other than the ones it
// was built to report.
#[allow(clippy::panic)]
static PARSED_DEFAULTS: LazyLock<(Defaults, DefaultsProvenance)> = LazyLock::new(|| {
    parse_embedded_defaults(DEFAULTS_JSON).unwrap_or_else(|error| panic!("{error:#}"))
});

fn parse_embedded_defaults(json: &str) -> Result<(Defaults, DefaultsProvenance)> {
    let defaults: Defaults = serde_json::from_str(json)
        .context("parsing embedded defaults.json")
        .and_then(validate_defaults)?;
    // `mode` by itself is policy, not a customization. An empty or
    // mode-only file is therefore the const-only build promised by the
    // defaults format.
    let effective = defaults.bootstrap.len() > 0
        || defaults.topic.is_some()
        || defaults.topic_secret.is_some()
        || !defaults.web_bindings.is_empty()
        || defaults.relay != crate::connectivity::RelayPolicy::N0
        || !defaults.gossip;
    let provenance = DefaultsProvenance {
        source: if effective {
            DefaultsSource::File
        } else {
            DefaultsSource::ConstsOnly
        },
        sha256: effective.then(|| {
            Sha256::digest(json.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect()
        }),
        mode: effective.then_some(defaults.mode),
    };
    Ok((defaults, provenance))
}

fn validate_defaults(defaults: Defaults) -> Result<Defaults> {
    ensure_topic_pair(
        defaults.topic.as_deref(),
        defaults.topic_secret.as_deref(),
        "defaults.json",
    )?;
    if defaults.mode == DefaultsMode::Replace
        && (defaults.topic.is_none() || defaults.topic_secret.is_none())
    {
        bail!("defaults.json mode=replace requires both topic and topic_secret");
    }
    Ok(defaults)
}

fn ensure_topic_pair(topic: Option<&str>, secret: Option<&str>, layer: &str) -> Result<()> {
    match (topic, secret) {
        (Some(_), None) => bail!("{layer} supplies topic without topic_secret"),
        (None, Some(_)) => bail!("{layer} supplies topic_secret without topic"),
        _ => Ok(()),
    }
}

/// Returns the parsed defaults. Invalid embedded configuration fails loudly.
pub fn defaults() -> &'static Defaults {
    &PARSED_DEFAULTS.0
}

/// Returns provenance for this binary's embedded defaults.
pub fn defaults_provenance() -> &'static DefaultsProvenance {
    &PARSED_DEFAULTS.1
}

/// Resolved configuration for peer discovery after applying the full
/// flag cascade.
///
/// Produced by [`resolve_config`]. All fields are ready to use — no
/// further defaulting is needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConfig {
    /// Bootstrap node IDs to contact on startup.
    pub bootstrap: Vec<String>,
    /// Gossip topic name.
    pub topic: String,
    /// Shared secret bytes for DHT record encryption.
    pub topic_secret: Vec<u8>,
    /// Relay policy selected from defaults and runtime override.
    pub relay: crate::connectivity::RelayPolicy,
    /// Whether gossip is enabled by default.
    pub gossip: bool,
}

/// Resolves the effective peer discovery configuration by applying the
/// full flag cascade.
///
/// # Resolution Order
///
/// consts → defaults.json → CLI additions/overrides → runtime skip flags.
///
/// # Errors
///
/// Returns an error naming the missing flag when `no_default_topic`
/// is `true` and exactly one of `cli_topic` / `cli_topic_secret` is
/// provided.
///
/// # Arguments
///
/// * `cli_bootstrap` - Node IDs supplied via `--bootstrap`
/// * `cli_topic` - Topic name supplied via `--topic`
/// * `cli_topic_secret` - Secret supplied via `--topic-secret`
/// * `no_default_bootstrap` - `--no-default-bootstrap`: skip default bootstrap nodes
/// * `no_default_topic` - `--no-default-topic`: skip default topic/secret
pub fn resolve_config(
    cli_bootstrap: &[String],
    cli_topic: Option<&str>,
    cli_topic_secret: Option<&str>,
    no_default_bootstrap: bool,
    no_default_topic: bool,
) -> Result<ResolvedConfig> {
    resolve_config_with_defaults(
        defaults(),
        cli_bootstrap,
        cli_topic,
        cli_topic_secret,
        no_default_bootstrap,
        no_default_topic,
    )
}

fn resolve_config_with_defaults(
    defs: &Defaults,
    cli_bootstrap: &[String],
    cli_topic: Option<&str>,
    cli_topic_secret: Option<&str>,
    no_default_bootstrap: bool,
    no_default_topic: bool,
) -> Result<ResolvedConfig> {
    // --- Bootstrap ---
    let mut bootstrap: Vec<String> = if no_default_bootstrap {
        Vec::new()
    } else {
        defs.bootstrap.clone()
    };
    // Append CLI-provided bootstrap nodes, then deduplicate (preserving order)
    bootstrap.extend_from_slice(cli_bootstrap);
    let mut seen = std::collections::HashSet::new();
    bootstrap.retain(|v| seen.insert(v.clone()));

    // --- Topic + Secret (resolved as a pair) ---
    ensure_topic_pair(cli_topic, cli_topic_secret, "CLI flags")?;
    let (topic, secret) = if let (Some(topic), Some(secret)) = (cli_topic, cli_topic_secret) {
        (topic.to_owned(), secret.as_bytes().to_vec())
    } else if !no_default_topic {
        if let (Some(topic), Some(secret)) = (&defs.topic, &defs.topic_secret) {
            (topic.clone(), secret.as_bytes().to_vec())
        } else {
            (DEFAULT_TOPIC.to_owned(), DEFAULT_TOPIC_SECRET.to_vec())
        }
    } else {
        (DEFAULT_TOPIC.to_owned(), DEFAULT_TOPIC_SECRET.to_vec())
    };

    Ok(ResolvedConfig {
        bootstrap,
        topic,
        topic_secret: secret,
        relay: defs.relay,
        gossip: defs.gossip,
    })
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

/// Maximum number of peer identities retained by one discovery table.
pub const MAX_PEERS: usize = 1_000;

/// Maximum UTF-8 byte length retained for a peer's advertised name.
pub const MAX_PEER_NAME_BYTES: usize = 256;

// ============================================================================
// Types
// ============================================================================

/// A peer's presence claim. Gossip carries it inside the id crate's signed,
/// versioned envelope; this plain structure is also nested in `/iroh-meta/3`.
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
/// use id_core::discovery::PeerAnnouncement;
/// use iroh_base::SecretKey;
///
/// let key = SecretKey::generate();
/// let announcement = PeerAnnouncement {
///     node_id: key.public(),
///     name: Some("my-server".to_string()),
///     blob_count: 42,
///     timestamp_secs: 1700000000,
/// };
///
/// // Gossip senders wrap this claim in a signed, versioned envelope.
/// assert_eq!(announcement.blob_count, 42);
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
/// Collects authenticated [`PeerAnnouncement`] messages from the gossip
/// channel and tracks peer liveness. Callers must verify the signed gossip
/// envelope before calling [`update`](Self::update). Provides methods to
/// query known peers and prune stale entries.
///
/// This type is cheaply cloneable (backed by `Arc`) and safe to share
/// across async tasks.
///
/// # Example
///
/// ```rust
/// use id_core::discovery::{PeerDiscovery, PeerAnnouncement};
/// use iroh_base::SecretKey;
///
/// let discovery = PeerDiscovery::new();
///
/// let node_id = SecretKey::from_bytes(&[1u8; 32]).public();
/// // Announcements must be stamped near the local wall clock: `update`
/// // refuses stamps older than `STALE_THRESHOLD` (see its docs).
/// let timestamp_secs = std::time::SystemTime::now()
///     .duration_since(std::time::UNIX_EPOCH)
///     .unwrap()
///     .as_secs();
/// let announcement = PeerAnnouncement {
///     node_id,
///     name: Some("peer-1".to_string()),
///     blob_count: 10,
///     timestamp_secs,
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
    /// Two checks guard the table against stale gossip, so every caller
    /// gets them (the `serve.rs` receive loop needs no trust logic of
    /// its own). This is the peer-announcement half of the single-door
    /// gossip admission contract; presence ingress and its audit are
    /// documented in `pkgs/id/src/chat_presence.rs` and issue #4503:
    /// every gossip claim is decoded, authenticated/scoped as applicable,
    /// and then stored only through its table's guards.
    ///
    /// - **Freshness bound.** An announcement stamped older than
    ///   [`STALE_THRESHOLD`] relative to the local wall clock is
    ///   refused: it neither inserts a peer nor refreshes one, so
    ///   replaying a captured announcement cannot keep a departed peer
    ///   live. The bound is symmetric: a stamp more than
    ///   [`STALE_THRESHOLD`] in the future is also refused, so a forged
    ///   far-future stamp (e.g. `u64::MAX`) can never enter the table
    ///   and poison the ordering watermark against every later genuine
    ///   announcement. Stamps within the bound — past or future —
    ///   count as fresh; the ordering check then governs whether they
    ///   may replace stored state.
    /// - **Ordering.** An announcement older than the stored one for
    ///   the same node is dropped: out-of-order delivery never
    ///   regresses the table. Equal timestamps are a tie, broken
    ///   deterministically — the most recent arrival wins.
    ///
    /// An announcement that passes both checks replaces the stored one
    /// (or inserts a new peer), stamped with the local arrival time. The table
    /// retains at most [`MAX_PEERS`]: a new identity may replace the oldest
    /// stale entry (ties break by endpoint-ID bytes); if every entry is fresh,
    /// the new identity is refused. Names longer than [`MAX_PEER_NAME_BYTES`]
    /// are refused. Existing identities can always refresh.
    pub fn update(&self, announcement: PeerAnnouncement) {
        if announcement
            .name
            .as_ref()
            .is_some_and(|name| name.len() > MAX_PEER_NAME_BYTES)
        {
            return;
        }
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        if now_secs.saturating_sub(announcement.timestamp_secs) > STALE_THRESHOLD.as_secs()
            || announcement.timestamp_secs.saturating_sub(now_secs) > STALE_THRESHOLD.as_secs()
        {
            return;
        }
        let node_id = announcement.node_id;
        if let Ok(mut peers) = self.peers.write() {
            if let Some(existing) = peers.get(&node_id)
                && announcement.timestamp_secs < existing.announcement.timestamp_secs
            {
                return;
            }

            let now = Instant::now();
            if !peers.contains_key(&node_id) && peers.len() >= MAX_PEERS {
                let victim = peers
                    .iter()
                    .filter(|(_, info)| now.duration_since(info.last_seen) >= STALE_THRESHOLD)
                    .min_by(|(left_id, left), (right_id, right)| {
                        left.last_seen
                            .cmp(&right.last_seen)
                            .then_with(|| left_id.as_bytes().cmp(right_id.as_bytes()))
                    })
                    .map(|(id, _)| *id);
                let Some(victim) = victim else {
                    return;
                };
                peers.remove(&victim);
            }

            peers.insert(
                node_id,
                PeerInfo {
                    announcement,
                    last_seen: now,
                },
            );
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
    /// Call this periodically (every [`STALE_CHECK_INTERVAL`]) to discard
    /// peers that have not been heard from or successfully probed.
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
        let mut stale: Vec<_> = peers
            .values()
            .filter(|info| now.duration_since(info.last_seen) >= STALE_THRESHOLD)
            .cloned()
            .collect();
        stale.sort_by(|left, right| {
            left.announcement
                .node_id
                .as_bytes()
                .cmp(right.announcement.node_id.as_bytes())
        });
        stale
    }

    /// Removes `node_id` only if it is still older than `max_age`.
    ///
    /// This is used after a bounded probe batch: a concurrent announcement or
    /// successful probe can refresh the peer while its network probe is in flight.
    pub fn remove_if_stale(&self, node_id: &EndpointId, max_age: Duration) -> bool {
        let Ok(mut peers) = self.peers.write() else {
            return false;
        };
        let now = Instant::now();
        let should_remove = peers
            .get(node_id)
            .is_some_and(|info| now.duration_since(info.last_seen) >= max_age);
        if should_remove {
            peers.remove(node_id);
        }
        should_remove
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
    clippy::unchecked_time_subtraction
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

    fn make_node_id_from_index(index: u64) -> EndpointId {
        let mut key_bytes = [0u8; 32];
        key_bytes[..8].copy_from_slice(&index.to_be_bytes());
        iroh_base::SecretKey::from_bytes(&key_bytes).public()
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn make_announcement(byte: u8, name: Option<&str>, blob_count: u64) -> PeerAnnouncement {
        make_announcement_at(byte, name, blob_count, now_secs())
    }

    fn make_announcement_at(
        byte: u8,
        name: Option<&str>,
        blob_count: u64,
        timestamp_secs: u64,
    ) -> PeerAnnouncement {
        PeerAnnouncement {
            node_id: make_node_id(byte),
            name: name.map(ToOwned::to_owned),
            blob_count,
            timestamp_secs,
        }
    }

    #[test]
    fn test_out_of_order_announcement_does_not_overwrite_newer() {
        let discovery = PeerDiscovery::new();
        let now = now_secs();
        discovery.update(make_announcement_at(1, Some("new-name"), 42, now));
        discovery.update(make_announcement_at(1, Some("old-name"), 7, now - 60));
        let peers = discovery.peers();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].announcement.name, Some("new-name".to_owned()));
        assert_eq!(peers[0].announcement.blob_count, 42);
        assert_eq!(peers[0].announcement.timestamp_secs, now);
    }

    #[test]
    fn test_ancient_announcement_is_refused_and_not_fresh() {
        let discovery = PeerDiscovery::new();
        // The probe's ghost case (#1552 case 3): stamped 1970.
        discovery.update(make_announcement_at(1, Some("ghost"), 99, 1));
        assert_eq!(discovery.count(), 0);
        assert!(discovery.peers().is_empty());

        // Nor does an ancient replay touch a live peer's stored state.
        let now = now_secs();
        discovery.update(make_announcement_at(2, Some("live"), 3, now));
        discovery.update(make_announcement_at(2, Some("ghost"), 99, now - 3600));
        let peers = discovery.peers();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].announcement.name, Some("live".to_owned()));
        assert_eq!(peers[0].announcement.blob_count, 3);
    }

    #[test]
    fn test_in_order_announcements_still_replace_and_ties_go_to_the_latest() {
        let discovery = PeerDiscovery::new();
        let now = now_secs();
        discovery.update(make_announcement_at(1, Some("first"), 1, now - 10));
        discovery.update(make_announcement_at(1, Some("second"), 2, now));
        let peers = discovery.peers();
        assert_eq!(peers[0].announcement.name, Some("second".to_owned()));
        assert_eq!(peers[0].announcement.blob_count, 2);
        // Equal timestamps: the documented tie rule — most recent arrival wins.
        discovery.update(make_announcement_at(1, Some("tie"), 3, now));
        let peers = discovery.peers();
        assert_eq!(peers[0].announcement.name, Some("tie".to_owned()));
        assert_eq!(peers[0].announcement.blob_count, 3);
    }

    #[test]
    fn test_far_future_announcement_is_refused_and_cannot_poison_ordering() {
        let discovery = PeerDiscovery::new();
        // A forged stamp at the end of time never enters the table.
        discovery.update(make_announcement_at(1, Some("forged"), 99, u64::MAX));
        assert_eq!(discovery.count(), 0);
        assert!(discovery.peers().is_empty());

        // Nor does a far-future stamp overwrite a live peer's stored
        // state, and the peer's genuine announcements keep updating it
        // afterwards (no ordering-watermark lockout).
        let now = now_secs();
        discovery.update(make_announcement_at(2, Some("live"), 3, now));
        discovery.update(make_announcement_at(
            2,
            Some("forged"),
            99,
            now + STALE_THRESHOLD.as_secs() + 1,
        ));
        discovery.update(make_announcement_at(2, Some("live-2"), 4, now));
        let peers = discovery.peers();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].announcement.name, Some("live-2".to_owned()));
        assert_eq!(peers[0].announcement.blob_count, 4);
        assert_eq!(peers[0].announcement.timestamp_secs, now);
    }

    #[test]
    fn test_near_future_announcement_within_skew_bound_is_accepted() {
        let discovery = PeerDiscovery::new();
        let now = now_secs();
        discovery.update(make_announcement_at(
            1,
            Some("skewed"),
            5,
            now + STALE_THRESHOLD.as_secs(),
        ));
        let peers = discovery.peers();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].announcement.name, Some("skewed".to_owned()));
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
    fn test_discovery_refuses_oversized_peer_names() {
        let discovery = PeerDiscovery::new();
        discovery.update(PeerAnnouncement {
            node_id: make_node_id(88),
            name: Some("x".repeat(MAX_PEER_NAME_BYTES + 1)),
            blob_count: 1,
            timestamp_secs: now_secs(),
        });
        assert_eq!(discovery.count(), 0);
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
    fn test_peer_table_cap_evicts_oldest_stale_with_endpoint_id_tie_break() {
        let discovery = PeerDiscovery::new();
        let now = now_secs();
        for index in 1..=u64::try_from(MAX_PEERS).unwrap() {
            discovery.update(PeerAnnouncement {
                node_id: make_node_id_from_index(index),
                name: None,
                blob_count: index,
                timestamp_secs: now,
            });
        }

        let first = make_node_id_from_index(1);
        let second = make_node_id_from_index(2);
        let tied_stale_time = Instant::now() - (STALE_THRESHOLD + Duration::from_secs(2));
        {
            let mut peers = discovery.peers.write().unwrap();
            peers.get_mut(&first).unwrap().last_seen = tied_stale_time;
            peers.get_mut(&second).unwrap().last_seen = tied_stale_time;
        }
        let expected_victim = [first, second]
            .into_iter()
            .min_by(|left, right| left.as_bytes().cmp(right.as_bytes()))
            .unwrap();
        let new_id = make_node_id_from_index(u64::try_from(MAX_PEERS).unwrap() + 1);

        discovery.update(PeerAnnouncement {
            node_id: new_id,
            name: Some("new".to_owned()),
            blob_count: 1,
            timestamp_secs: now,
        });

        assert_eq!(discovery.count(), MAX_PEERS);
        assert!(discovery.peers.read().unwrap().contains_key(&new_id));
        assert!(
            !discovery
                .peers
                .read()
                .unwrap()
                .contains_key(&expected_victim)
        );
        let other_stale = if expected_victim == first {
            second
        } else {
            first
        };
        assert!(discovery.peers.read().unwrap().contains_key(&other_stale));
    }

    #[test]
    fn test_peer_table_cap_keeps_fresh_entries_and_allows_existing_refresh() {
        let discovery = PeerDiscovery::new();
        let now = now_secs();
        for index in 1..=u64::try_from(MAX_PEERS).unwrap() {
            discovery.update(PeerAnnouncement {
                node_id: make_node_id_from_index(index),
                name: None,
                blob_count: index,
                timestamp_secs: now,
            });
        }
        let new_id = make_node_id_from_index(u64::try_from(MAX_PEERS).unwrap() + 1);
        discovery.update(PeerAnnouncement {
            node_id: new_id,
            name: Some("refused".to_owned()),
            blob_count: 1,
            timestamp_secs: now,
        });
        assert_eq!(discovery.count(), MAX_PEERS);
        assert!(!discovery.peers.read().unwrap().contains_key(&new_id));

        let existing_id = make_node_id_from_index(1);
        discovery.update(PeerAnnouncement {
            node_id: existing_id,
            name: Some("refreshed".to_owned()),
            blob_count: 2,
            timestamp_secs: now + 1,
        });
        let peers = discovery.all_peers();
        let existing = peers
            .iter()
            .find(|peer| peer.announcement.node_id == existing_id)
            .unwrap();
        assert_eq!(existing.announcement.name.as_deref(), Some("refreshed"));
        assert_eq!(discovery.count(), MAX_PEERS);
    }

    #[test]
    fn test_remove_if_stale_preserves_a_peer_refreshed_during_probe() {
        let discovery = PeerDiscovery::new();
        let node_id = make_node_id(77);
        {
            let mut peers = discovery.peers.write().unwrap();
            peers.insert(
                node_id,
                PeerInfo {
                    announcement: make_announcement(77, Some("peer"), 5),
                    last_seen: Instant::now() - (STALE_THRESHOLD + Duration::from_secs(1)),
                },
            );
        }

        assert!(discovery.refresh(&node_id));
        assert!(!discovery.remove_if_stale(&node_id, STALE_THRESHOLD));
        assert_eq!(discovery.count(), 1);
    }

    #[test]
    fn test_constants() {
        assert_eq!(ANNOUNCE_INTERVAL, Duration::from_secs(30));
        assert_eq!(STALE_THRESHOLD, Duration::from_secs(120));
        assert_eq!(STALE_CHECK_INTERVAL, Duration::from_secs(60));
        assert_eq!(MAX_PEERS, 1_000);
        assert_eq!(MAX_PEER_NAME_BYTES, 256);
        assert_eq!(DEFAULT_TOPIC, "id-peer-discovery-v1");
        assert_eq!(DEFAULT_TOPIC_SECRET, b"id-public-discovery-v1");
    }

    // ========================================================================
    // defaults / resolve_config tests

    #[test]
    fn embedded_defaults_are_valid_and_have_provenance() {
        let parsed: Defaults = serde_json::from_str(DEFAULTS_JSON).unwrap();
        validate_defaults(parsed).unwrap();
        assert_eq!(defaults().topic.as_deref(), Some(DEFAULT_TOPIC));
        assert_eq!(
            defaults().topic_secret.as_deref(),
            Some("id-public-discovery-v1")
        );
        assert_eq!(defaults_provenance().source, DefaultsSource::File);
        assert!(
            defaults_provenance()
                .sha256
                .as_ref()
                .is_some_and(|hash| hash.len() == 64)
        );
    }

    #[test]
    fn empty_json_is_const_equivalent_and_unknown_fields_fail() {
        let empty: Defaults = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, Defaults::default());
        let unknown = serde_json::from_str::<Defaults>(r#"{"topc":"typo"}"#);
        assert!(unknown.is_err());
        assert!(serde_json::from_str::<Defaults>(r#"{"bootstrap":"not-a-list"}"#).is_err());
    }

    #[test]
    fn provenance_is_absent_for_consts_and_has_stable_file_format() {
        let (_, consts) = parse_embedded_defaults(r#"{"mode":"append"}"#).unwrap();
        assert_eq!(consts.source, DefaultsSource::ConstsOnly);
        assert_eq!(consts.sha256, None);
        assert_eq!(consts.mode, None);
        assert_eq!(consts.describe(), "defaults: consts");

        let json = r#"{"topic":"private","topic_secret":"secret"}"#;
        let (_, file) = parse_embedded_defaults(json).unwrap();
        let expected_hash = Sha256::digest(json.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(file.source, DefaultsSource::File);
        assert_eq!(file.sha256.as_deref(), Some(expected_hash.as_str()));
        assert_eq!(file.mode, Some(DefaultsMode::Append));
        assert_eq!(
            file.describe(),
            format!("defaults: file sha256={expected_hash} mode=append")
        );
    }

    #[test]
    fn empty_and_const_valued_file_matrices_match_const_defaults() {
        let empty: Defaults = serde_json::from_str("{}").unwrap();
        let const_valued: Defaults = serde_json::from_str(
            r#"{"topic":"id-peer-discovery-v1","topic_secret":"id-public-discovery-v1"}"#,
        )
        .unwrap();
        let topic_pair = [(None, None), (Some("cli-topic"), Some("cli-secret"))];
        for no_bootstrap in [false, true] {
            for no_topic in [false, true] {
                for (topic, secret) in topic_pair {
                    let consts = resolve_config_with_defaults(
                        &empty,
                        &[],
                        topic,
                        secret,
                        no_bootstrap,
                        no_topic,
                    )
                    .unwrap();
                    let file = resolve_config_with_defaults(
                        &const_valued,
                        &[],
                        topic,
                        secret,
                        no_bootstrap,
                        no_topic,
                    )
                    .unwrap();
                    assert_eq!(consts, file);
                }
            }
        }
    }

    #[test]
    fn build_defaults_override_relay_gossip_and_pair_together() {
        let defs: Defaults = serde_json::from_str(
            r#"{"topic":"private","topic_secret":"secret","relay":"disabled","gossip":false}"#,
        )
        .unwrap();
        let actual = resolve_config_with_defaults(&defs, &[], None, None, false, false).unwrap();
        assert_eq!(actual.topic, "private");
        assert_eq!(actual.topic_secret, b"secret");
        assert_eq!(actual.relay, crate::connectivity::RelayPolicy::Disabled);
        assert!(!actual.gossip);

        let cli =
            resolve_config_with_defaults(&defs, &[], Some("cli"), Some("cli-secret"), false, false)
                .unwrap();
        assert_eq!(cli.topic, "cli");
        assert_eq!(cli.topic_secret, b"cli-secret");
    }

    #[test]
    fn file_pair_is_atomic_and_replace_requires_pair() {
        let partial: Defaults = serde_json::from_str(r#"{"topic":"private"}"#).unwrap();
        assert!(
            validate_defaults(partial)
                .unwrap_err()
                .to_string()
                .contains("topic_secret")
        );
        let replace: Defaults = serde_json::from_str(r#"{"mode":"replace"}"#).unwrap();
        assert!(
            validate_defaults(replace)
                .unwrap_err()
                .to_string()
                .contains("requires both")
        );

        let replace_with_pair: Defaults =
            serde_json::from_str(r#"{"mode":"replace","topic":"private","topic_secret":"secret"}"#)
                .unwrap();
        assert!(validate_defaults(replace_with_pair).is_ok());
    }

    #[test]
    fn cli_bootstrap_nodes_append_and_deduplicate_in_order() {
        let defs: Defaults = serde_json::from_str(r#"{"bootstrap":["file-a","file-b"]}"#).unwrap();
        let cli = vec!["file-b".to_owned(), "cli-c".to_owned()];
        let resolved = resolve_config_with_defaults(&defs, &cli, None, None, false, false).unwrap();
        assert_eq!(resolved.bootstrap, ["file-a", "file-b", "cli-c"]);
    }

    #[test]
    fn test_resolve_config_defaults_and_cli_overrides() {
        let cfg = resolve_config(&[], None, None, false, false).unwrap();
        assert_eq!(cfg.topic, DEFAULT_TOPIC);
        assert_eq!(cfg.topic_secret, DEFAULT_TOPIC_SECRET);
        assert!(cfg.gossip);
        assert_eq!(cfg.relay, crate::connectivity::RelayPolicy::N0);

        let cli_bs = vec!["node1".to_owned()];
        let cfg = resolve_config(
            &cli_bs,
            Some("custom-topic"),
            Some("custom-secret"),
            false,
            false,
        )
        .unwrap();
        assert_eq!(cfg.bootstrap, vec!["node1"]);
        assert_eq!(cfg.topic, "custom-topic");
        assert_eq!(cfg.topic_secret, b"custom-secret");
    }

    #[test]
    fn topic_pair_cannot_be_overridden_halfway() {
        let error = resolve_config(&[], Some("private"), None, false, false).unwrap_err();
        assert!(error.to_string().contains("topic_secret"));
        let error = resolve_config(&[], None, Some("secret"), false, false).unwrap_err();
        assert!(error.to_string().contains("topic"));
    }

    #[test]
    fn no_default_flags_keep_cli_values_only_or_const_pair() {
        let nodes = vec!["cli-node".to_owned()];
        let cfg = resolve_config(&nodes, None, None, true, true).unwrap();
        assert_eq!(cfg.bootstrap, nodes);
        assert_eq!(cfg.topic, DEFAULT_TOPIC);
        let cfg = resolve_config(&[], Some("private"), Some("secret"), false, true).unwrap();
        assert_eq!(cfg.topic, "private");
    }
}
