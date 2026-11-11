//! The operator block list vocabulary: what a block names ([`BlockKind`])
//! and the list as the enforcement seams consult it ([`BlockList`]).
//!
//! Step 3a of the pkgs/id crate split (#4406): these two pieces are the
//! only store-level types the abuse evaluator in this crate names, and
//! the evaluator belongs in the kernel with the wire framing that
//! charges through it. The kind and the trait are leaf vocabulary - a
//! kind tag and three questions over raw bytes - so they live here; the
//! block row stays in `id-store`'s `event_types` with the store error
//! its decoding needs, and `id-store` re-exports both names at their
//! historical paths.

/// What an operator block names. The tag byte leads the row key in `blocks`,
/// so rows sort by kind, then by value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// An Iroh endpoint key (the EndpointId of abuse.rs): refuse the peer's
    /// new sessions, blob serving, and gossip subscriptions.
    PeerKey,
    /// An IP address, enforced at the listener only (best-effort).
    Ip,
    /// A topic tag, composing with the pull policy.
    TopicTag,
}

impl BlockKind {
    pub fn tag(self) -> u8 {
        match self {
            BlockKind::PeerKey => 0,
            BlockKind::Ip => 1,
            BlockKind::TopicTag => 2,
        }
    }

    pub fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            0 => Some(BlockKind::PeerKey),
            1 => Some(BlockKind::Ip),
            2 => Some(BlockKind::TopicTag),
            _ => None,
        }
    }
}

/// The operator block list as the enforcement seams consult it (#2245).
///
/// A seam holds an `Arc<dyn BlockList>` and asks it about a peer, a topic
/// tag, or an IP; it never reads the `blocks` table itself. Implementations
/// read the list live on every call, so a block (or an unblock) lands on the
/// next check, and they judge expiry against the wall clock
/// ([`crate::time::unix_ms`]) — the same epoch `Block::added_at_ms` and
/// `Block::expires_at_ms` are stamped in. That is why this trait takes no
/// `now_ms`: the session clocks the sync seams carry are not all wall
/// clocks (the abuse evaluator's is monotonic), and a block's expiry is a
/// wall-clock fact.
///
/// The invariant from the module docs rides on this trait's callers: blocks
/// change serving and subscribing only. No ingest path consults a
/// `BlockList`, so an already-signed object from a blocked peer is decided
/// by the store's ordinary rules, exactly as if the block did not exist.
pub trait BlockList: std::fmt::Debug + Send + Sync + 'static {
    /// Whether (`kind`, `value`) is blocked now. An expired block is not
    /// enforced, so this is false for one, however long it stays listed.
    fn is_blocked(&self, kind: BlockKind, value: &[u8]) -> bool;

    /// Whether the endpoint with this 32-byte key is blocked.
    fn peer_blocked(&self, peer: &[u8; 32]) -> bool {
        self.is_blocked(BlockKind::PeerKey, peer)
    }

    /// Whether this topic tag is blocked. The tag is raw bytes: a pull
    /// gate passes its world's name, the sync handshake passes the topic's
    /// hash bytes; an operator blocks whichever form a surface names.
    fn topic_tag_blocked(&self, tag: &[u8]) -> bool {
        self.is_blocked(BlockKind::TopicTag, tag)
    }

    /// Whether this IP is blocked. IP rows name the address in its text
    /// form (as the storage tests write them), so the check formats `ip`
    /// the same way.
    fn ip_blocked(&self, ip: &std::net::IpAddr) -> bool {
        self.is_blocked(BlockKind::Ip, ip.to_string().as_bytes())
    }
}

impl<T: BlockList + ?Sized> BlockList for std::sync::Arc<T> {
    fn is_blocked(&self, kind: BlockKind, value: &[u8]) -> bool {
        (**self).is_blocked(kind, value)
    }
}
