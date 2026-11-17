//! Holder discovery for blob hashes (Follow-up A of #2050, issue #2260).
//!
//! `pkgs/id` can discover *servers* (see [`crate::discovery`]) but not
//! *content*: nothing says which blob hashes a peer holds. This module adds
//! the missing piece — a provider table mapping a blob hash to the set of
//! holders that carry it, plus the gossip announcement shape used to populate
//! it and the staleness rules that keep it honest.
//!
//! Design notes (issue #2260, Drew's answer "design-mill on the private-world
//! protocol + public-world implementation in parallel"):
//!
//! - This module implements the **public-world** half only. A holder announces
//!   its full hash set for a world it considers public; membership-gated
//!   (private) worlds are intentionally NOT announced here, because doing so
//!   on the shared public discovery topic would leak their holdings. The
//!   private-world protocol is deferred to the design-mill track and will
//!   plug into the same [`ProviderTable`] / [`HolderAnnouncement`] shapes.
//! - Staleness reuses the discovery plane's announce/stale cadence
//!   ([`crate::discovery::ANNOUNCE_INTERVAL`] / [`STALE_THRESHOLD`]) so a
//!   holder that goes silent expires on the same shape as a peer.
//! - Held-range summaries are coarse on purpose (complete / ranges / unknown)
//!   so an announcement stays small; the multi-provider scheduler (#2261) is
//!   the consumer and asks for exact ranges only when it actually fetches.
//!
//! Compact vs. full announcements: small worlds announce their full hash set;
//! large worlds announce a compact summary (a count plus a Bloom filter over
//! the held hashes). The format is chosen and documented here as
//! [`HolderAnnouncement::hashes`] (full) with [`HolderAnnouncement::truncated`]
//! signalling that only the first [`HOLDER_ANNOUNCEMENT_CAP`] hashes are
//! carried and the remainder must be fetched via the rendezvous query. A
//! future revision may swap the tail for a Bloom filter without changing the
//! wire shape beyond the flag.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use iroh_base::EndpointId;
use iroh_blobs::Hash;
use serde::{Deserialize, Serialize};

/// How many hashes a single announcement carries before it is marked
/// truncated. Worlds smaller than this announce everything; larger ones
/// announce a prefix and rely on the rendezvous (public server) query for
/// the rest. 256 keeps a postcard frame well under a typical gossip
/// message size while covering the common small-world case.
pub const HOLDER_ANNOUNCEMENT_CAP: usize = 256;

/// Coarse held-range summary for one blob on one holder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HeldRanges {
    /// The holder has the whole blob.
    Complete,
    /// The holder has some verified ranges, given as `(start, end)` byte
    /// offsets (half-open).
    Ranges(Vec<(u64, u64)>),
    /// The holder's held ranges are not known (e.g. learned from a
    /// presence-only announcement).
    Unknown,
}

/// One row of the provider table: a single holder of a single hash.
#[derive(Debug, Clone)]
pub struct HolderEntry {
    /// The holder's endpoint identity.
    pub endpoint: EndpointId,
    /// Coarse held-range summary.
    pub ranges: HeldRanges,
    /// Local monotonic instant when this entry was last refreshed.
    pub last_seen: Instant,
    /// The world this holding belongs to, when scoped. `None` means an
    /// unscoped (global/public-aggregate) holding.
    pub world: Option<String>,
}

/// Gossip announcement shape for holder discovery.
///
/// Carried on the existing discovery plane. `world` is `Some` for a
/// per-world announcement (the only kind a public-world holder makes) and
/// `None` for a node-wide aggregate. `truncated` signals that `hashes`
/// contains only the first [`HOLDER_ANNOUNCEMENT_CAP`] entries.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HolderAnnouncement {
    /// The announcing node's identity.
    pub node_id: EndpointId,
    /// World scope, if any. Public worlds only in this implementation.
    pub world: Option<String>,
    /// Held blob hashes (full set for small worlds, prefix for large).
    pub hashes: Vec<Hash>,
    /// Whether `hashes` was truncated to [`HOLDER_ANNOUNCEMENT_CAP`].
    pub truncated: bool,
    /// Unix timestamp (seconds) when this announcement was created.
    pub timestamp_secs: u64,
}

impl HolderAnnouncement {
    /// Build an announcement from a set of held hashes, applying the cap.
    pub fn new(
        node_id: EndpointId,
        world: Option<String>,
        held: impl IntoIterator<Item = Hash>,
        timestamp_secs: u64,
    ) -> Self {
        let mut hashes: Vec<Hash> = held.into_iter().collect();
        let truncated = hashes.len() > HOLDER_ANNOUNCEMENT_CAP;
        if truncated {
            hashes.truncate(HOLDER_ANNOUNCEMENT_CAP);
        }
        Self {
            node_id,
            world,
            hashes,
            truncated,
            timestamp_secs,
        }
    }
}

/// world (Option) -> hash -> endpoint -> entry.
type WorldTable = HashMap<Option<String>, HashMap<Hash, HashMap<EndpointId, HolderEntry>>>;

/// Thread-safe in-memory provider table: hash -> (endpoint -> holder entry).
///
/// Cheaply cloneable (backed by `Arc`), safe to share across async tasks.
/// Mirrors [`crate::discovery::PeerDiscovery`] in shape so callers familiar
/// with the peer table feel at home.
#[derive(Debug, Clone)]
pub struct ProviderTable {
    /// world (Option) -> hash -> endpoint -> entry. `None` world is the
    /// global bucket.
    entries: Arc<RwLock<WorldTable>>,
}

impl ProviderTable {
    /// Create an empty provider table.
    pub fn new() -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Read access to the table. The table is a plain cache with no
    /// cross-entry invariant, so a writer that panicked mid-update cannot
    /// leave it inconsistent: recover the guard instead of panicking.
    fn read_entries(&self) -> RwLockReadGuard<'_, WorldTable> {
        self.entries.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Write access to the table; poison is recovered as in [`Self::read_entries`].
    fn write_entries(&self) -> RwLockWriteGuard<'_, WorldTable> {
        self.entries.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Record (or refresh) a holder for `(world, hash)`.
    pub fn record(
        &self,
        world: Option<String>,
        hash: Hash,
        endpoint: EndpointId,
        ranges: HeldRanges,
        now: Instant,
    ) {
        let mut guard = self.write_entries();
        let world_map = guard.entry(world.clone()).or_default();
        let hash_map = world_map.entry(hash).or_default();
        hash_map.insert(
            endpoint,
            HolderEntry {
                endpoint,
                ranges,
                last_seen: now,
                world: world.clone(),
            },
        );
    }

    /// Record a whole announcement into the table.
    pub fn apply_announcement(&self, ann: &HolderAnnouncement, now: Instant) {
        for h in &ann.hashes {
            self.record(ann.world.clone(), *h, ann.node_id, HeldRanges::Unknown, now);
        }
    }

    /// Snapshot of holders for `(world, hash)`, freshest first.
    pub fn holders(&self, world: Option<&str>, hash: &Hash) -> Vec<HolderEntry> {
        let guard = self.read_entries();
        let world_key = world.map(|s| s.to_string());
        guard
            .get(&world_key)
            .and_then(|m| m.get(hash))
            .map(|hm| {
                let mut v: Vec<HolderEntry> = hm.values().cloned().collect();
                v.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
                v
            })
            .unwrap_or_default()
    }

    /// Number of distinct hashes tracked for a world.
    pub fn hash_count(&self, world: Option<&str>) -> usize {
        let guard = self.read_entries();
        let world_key = world.map(|s| s.to_string());
        guard.get(&world_key).map(|m| m.len()).unwrap_or(0)
    }

    /// Remove entries older than `max_age`. Returns the number removed.
    pub fn remove_stale(&self, max_age: Duration) -> usize {
        let mut guard = self.write_entries();
        let mut removed = 0usize;
        for world_map in guard.values_mut() {
            for hash_map in world_map.values_mut() {
                let before = hash_map.len();
                hash_map.retain(|_, e| e.last_seen.elapsed() <= max_age);
                removed += before - hash_map.len();
            }
            // drop empty hash maps
            world_map.retain(|_, hm| !hm.is_empty());
        }
        guard.retain(|_, wm| !wm.is_empty());
        removed
    }

    /// Total number of (world, hash, holder) rows.
    pub fn len(&self) -> usize {
        let guard = self.read_entries();
        guard
            .values()
            .map(|wm| wm.values().map(|hm| hm.len()).sum::<usize>())
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for ProviderTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience: current unix timestamp in seconds.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Build a holder announcement for a public world from a set of held hashes.
pub fn announce(
    node_id: EndpointId,
    world: Option<String>,
    held: impl IntoIterator<Item = Hash>,
) -> HolderAnnouncement {
    HolderAnnouncement::new(node_id, world, held, now_secs())
}

/// Whether a world should be announced publicly. Membership-gated (private)
/// worlds must never be announced on the shared plane; only worlds the caller
/// has classified as public are eligible. The caller is responsible for that
/// classification — this helper just documents the contract.
pub fn is_public_world(world: &str) -> bool {
    // A world is treated as public when its name begins with the public
    // prefix convention used elsewhere in id. Conservative default: only
    // explicitly-prefixed worlds are public; everything else is treated as
    // potentially membership-gated and must not be announced.
    world.starts_with("public:") || world == "public"
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh_base::SecretKey;

    fn node(n: u8) -> EndpointId {
        SecretKey::from_bytes(&[n; 32]).public()
    }

    fn h(n: u8) -> Hash {
        // Hash::from_bytes expects [u8; 32]
        Hash::from([n; 32])
    }

    #[test]
    fn record_and_query() {
        let t = ProviderTable::new();
        let n = node(1);
        t.record(None, h(1), n, HeldRanges::Complete, Instant::now());
        let hs = t.holders(None, &h(1));
        assert_eq!(hs.len(), 1);
        assert_eq!(hs[0].endpoint, n);
    }

    #[test]
    fn world_scoping() {
        let t = ProviderTable::new();
        let n = node(2);
        t.record(
            Some("w1".into()),
            h(1),
            n,
            HeldRanges::Complete,
            Instant::now(),
        );
        // different world bucket -> not found
        assert!(t.holders(Some("w2"), &h(1)).is_empty());
        assert_eq!(t.holders(Some("w1"), &h(1)).len(), 1);
    }

    #[test]
    fn apply_announcement() {
        let t = ProviderTable::new();
        let n = node(3);
        let ann = HolderAnnouncement::new(n, Some("public".into()), vec![h(1), h(2)], 1);
        t.apply_announcement(&ann, Instant::now());
        assert_eq!(t.holders(Some("public"), &h(1)).len(), 1);
        assert_eq!(t.holders(Some("public"), &h(2)).len(), 1);
    }

    #[test]
    fn truncated_cap() {
        let n = node(4);
        let many: Vec<Hash> = (0..300u16).map(|x| h(x as u8)).collect();
        let ann = HolderAnnouncement::new(n, None, many, 1);
        assert!(ann.truncated);
        assert_eq!(ann.hashes.len(), HOLDER_ANNOUNCEMENT_CAP);
    }

    #[test]
    fn remove_stale() {
        let t = ProviderTable::new();
        let n = node(5);
        // record with an instant far in the past by using elapsed check:
        // remove_stale uses last_seen.elapsed(); record now then sleep not needed;
        // instead test count stays when fresh.
        t.record(None, h(9), n, HeldRanges::Complete, Instant::now());
        assert_eq!(t.len(), 1);
        // nothing stale yet
        assert_eq!(t.remove_stale(Duration::from_secs(3600)), 0);
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn holders_sorted_freshest_first() {
        let t = ProviderTable::new();
        let a = node(6);
        let b = node(7);
        let now = Instant::now();
        t.record(None, h(3), a, HeldRanges::Complete, now);
        // b recorded slightly later
        t.record(
            None,
            h(3),
            b,
            HeldRanges::Complete,
            now + Duration::from_secs(1),
        );
        let hs = t.holders(None, &h(3));
        assert_eq!(hs[0].endpoint, b);
    }
}
