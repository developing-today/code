//! Chat presence: ephemeral TTL heartbeats and roster liveness (ID chat v1, #2809).
//!
//! Presence is a separately-scoped ephemeral protocol, distinct from chat messages
//! ([`crate::chat`]): a node periodically broadcasts a [`PresenceHeartbeat`] —
//! `{node, channels[], status, ts}` — over the already-wired gossip plane (see
//! [`crate::commands::serve`]), and every receiver keeps the latest heartbeat per node in
//! an in-memory [`PresenceTable`]. There are no leave messages: a node that stops
//! heartbeating (crash, netsplit, `kill -STOP`) simply goes stale and reads as gone once
//! its last heartbeat ages past [`PRESENCE_TTL_SECS`]; when it resumes, the next
//! heartbeat makes it live again.
//!
//! # Clock skew and the expiry basis (#8865)
//!
//! A sender's clock is not the receiver's clock. Expiry is therefore NOT
//! measured from the sender-provided stamp alone: a heartbeat is accepted
//! only when its stamp is at most [`MAX_FUTURE_SKEW_SECS`] ahead of the
//! receiver's clock — a small allowance documented separately from
//! [`PRESENCE_TTL_SECS`] — and the entry's expiry runs from its *effective*
//! stamp, `min(sender stamp, local receipt time)`. An accepted future-dated
//! heartbeat thus stays live for at most one TTL *from receipt*, never a
//! TTL plus its skew (pruning retains the entry a little longer, purely as
//! a replay watermark — see [`PresenceTable::prune_at`]), and roster
//! `last_seen` reports the effective stamp,
//! so it is never future-dated. A heartbeat stamped in the past keeps its
//! sender stamp as its effective stamp, so re-delivery cannot extend its
//! life either (see [`PresenceTable::update_at`]).
//!
//! # Never journaled
//!
//! Presence is **never** written to the signed event journal, event sync, or replay.
//! The table holds no store handle and offers no persistence API: dropping the table
//! drops the state, and a restarted node starts with an empty table and re-learns
//! liveness from fresh heartbeats alone. Typing indicators ride the same path — a
//! heartbeat names the channels the node is currently typing in, and that claim expires
//! on the much shorter [`TYPING_TTL_SECS`] clock.
//!
//! # Membership scoping
//!
//! A channel is a chat group (the channel's identity is its group [`Hash`], as in
//! [`crate::chat`]), and presence metadata is scoped to authorized channel members:
//!
//! - On receipt, [`PresenceTable::update_scoped`] filters a heartbeat's channels to
//!   those the *sender* is a member of (the caller's membership predicate), so a node
//!   cannot advertise presence in — or probe — a channel it does not belong to.
//! - On read, [`PresenceTable::roster_for_viewer_at`] returns nothing for a viewer who
//!   is not a member of the channel, so a private channel's roster (and the fact that
//!   anyone is live in it) never leaks to non-members.
//!
//! # Authentication
//!
//! A heartbeat is a claim about its `node`, and the gossip plane does not
//! authenticate message authors — membership filtering alone cannot stop
//! impersonation, because it checks the claimed key. So every heartbeat is
//! signed by its node's key ([`PresenceHeartbeat::sign`]) and receivers verify
//! the signature ([`PresenceHeartbeat::verify`]) before the heartbeat reaches
//! the table: a forged heartbeat naming another node, or one whose channels
//! were tampered with after signing, does not verify and is dropped.
//!
//! # Roster
//!
//! The roster for a channel is the backing group's membership — the member set of the
//! group fold ([`crate::groups::GroupView`]) — annotated with last-seen liveness from
//! the table: [`Liveness::Live`] when a heartbeat for that channel is within TTL,
//! [`Liveness::Offline`] when the member was never seen or its heartbeat went stale.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use p2panda_core::Hash;
use serde::{Deserialize, Serialize};

use crate::groups::GroupView;
use crate::waiting::AuthorKey;

/// How long a heartbeat keeps a node live, in seconds: 75, inside the 60–90 s band
/// #2809 specifies. A node silent for longer than this reads as gone.
pub const PRESENCE_TTL_SECS: u64 = 75;

/// Maximum future clock skew accepted on a heartbeat, in seconds: 10.
///
/// This allowance is deliberately separate from — and much smaller than —
/// [`PRESENCE_TTL_SECS`]: the TTL is how long silence may last before a node
/// reads as gone, while this bound is only how far a sender's clock may run
/// ahead of the receiver's before its stamps stop being trusted at all.
/// 10 s is generous for NTP-disciplined clocks and well under one
/// [`HEARTBEAT_INTERVAL`], so a node whose clock is merely fast still lands
/// two to three heartbeats inside every TTL window. A heartbeat stamped
/// further ahead is refused outright (see [`PresenceTable::update_at`]),
/// and an accepted future stamp never extends liveness past one TTL from
/// receipt, because expiry runs from the receipt-clamped effective stamp.
pub const MAX_FUTURE_SKEW_SECS: u64 = 10;

/// How often a node broadcasts its presence heartbeat. Two to three heartbeats land
/// inside one TTL window, so a single dropped gossip message never flickers a live
/// node stale.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(25);

/// How long a typing claim stays live, in seconds. Typing heartbeats are republished
/// on typing changes (and at least this often while typing continues), so a claim
/// whose sender stopped or crashed clears itself quickly.
pub const TYPING_TTL_SECS: u64 = 12;

/// The most channels one heartbeat may name. Bounds the frame size and the work a
/// single received heartbeat can make the table do.
pub const MAX_HEARTBEAT_CHANNELS: usize = 64;

/// Frame prefix distinguishing presence frames from peer-discovery announcements on
/// the shared gossip topic. A frame without this prefix is not a presence frame, and
/// [`decode_frame`] returns `None` for it; legacy announcement bytes never decode as
/// presence.
pub const PRESENCE_FRAME_MAGIC: &[u8] = b"id-presence/v1\n";

/// A node's presence claim, as broadcast. `status` carries no offline value on
/// purpose: going away is expressed by *stopping*, never by a leave message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceHeartbeat {
    /// The node's key: the account key that authors its chat events.
    pub node: AuthorKey,
    /// The channels (chat group IDs) the node is present in.
    pub channels: Vec<Hash>,
    /// The channels the node is currently typing in. A subset of `channels`.
    #[serde(default)]
    pub typing: Vec<Hash>,
    /// Unix timestamp (seconds) when the heartbeat was created.
    pub ts: u64,
    /// Ed25519 signature by `node` over the rest of the heartbeat (see
    /// [`PresenceHeartbeat::sign`]). Empty until signed; an unsigned or badly
    /// signed heartbeat is not authenticated and receivers must drop it (see
    /// [`PresenceHeartbeat::verify`]).
    #[serde(default)]
    pub signature: Vec<u8>,
}

impl PresenceHeartbeat {
    /// Builds a heartbeat for `node` at `ts`, dropping duplicate channels, duplicate
    /// typing claims, and typing claims for channels the heartbeat does not also
    /// claim presence in — so `typing` is always a duplicate-free subset of
    /// `channels` and can never outgrow it.
    #[must_use]
    pub fn new(node: AuthorKey, channels: Vec<Hash>, typing: Vec<Hash>, ts: u64) -> Self {
        let mut unique: Vec<Hash> = Vec::new();
        for channel in channels {
            if !unique.contains(&channel) {
                unique.push(channel);
            }
        }
        unique.truncate(MAX_HEARTBEAT_CHANNELS);
        let channels = unique;
        let mut unique_typing: Vec<Hash> = Vec::new();
        for channel in typing {
            if channels.contains(&channel) && !unique_typing.contains(&channel) {
                unique_typing.push(channel);
            }
        }
        let typing = unique_typing;
        Self {
            node,
            channels,
            typing,
            ts,
            signature: Vec::new(),
        }
    }

    /// The bytes the signature covers: the postcard encoding of
    /// `(node, channels, typing, ts)`, in that order.
    fn signing_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(&(&self.node, &self.channels, &self.typing, self.ts))
            .unwrap_or_default()
    }

    /// Signs the heartbeat with `key`, setting `node` to the key's verifying
    /// key: the claim and the identity that vouches for it are the same key.
    pub fn sign(&mut self, key: &SigningKey) {
        self.node = key.verifying_key().to_bytes();
        self.signature = key.sign(&self.signing_bytes()).to_bytes().to_vec();
    }

    /// Whether the heartbeat carries a valid signature by `node` over its
    /// contents. Gossip in this stack does not authenticate message authors,
    /// so this signature is the authentication: receivers verify before the
    /// heartbeat reaches the table, and anything unsigned or forged — a claim
    /// naming another node, channels tampered with after signing — is dropped.
    #[must_use]
    pub fn verify(&self) -> bool {
        let Ok(verifying_key) = VerifyingKey::from_bytes(&self.node) else {
            return false;
        };
        let Ok(signature) = Signature::from_slice(&self.signature) else {
            return false;
        };
        verifying_key
            .verify_strict(&self.signing_bytes(), &signature)
            .is_ok()
    }

    /// Whether the heartbeat claims typing in `channel`.
    #[must_use]
    pub fn is_typing_in(&self, channel: &Hash) -> bool {
        self.typing.contains(channel)
    }
}

/// Whether a roster member currently reads as present.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Liveness {
    /// A heartbeat for the channel arrived within [`PRESENCE_TTL_SECS`].
    Live,
    /// Never seen, or the last heartbeat aged past the TTL: stale reads as gone.
    Offline,
}

/// One roster row: a channel member annotated with last-seen liveness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RosterEntry {
    /// The member's key.
    pub node: AuthorKey,
    /// Whether the member is live in the channel right now.
    pub liveness: Liveness,
    /// Unix seconds of the member's most recent accepted heartbeat for the channel,
    /// or `None` if the member has never been seen. This is the entry's
    /// *effective* stamp — the sender stamp clamped to the local receipt
    /// time — so a future-dated sender stamp never makes `last_seen` read
    /// as a time in the future (#8865).
    pub last_seen: Option<u64>,
    /// Whether the member's typing claim for the channel is currently live.
    pub typing: bool,
}

/// The latest accepted heartbeat for one node, plus its *effective* stamp:
/// `min(heartbeat.ts, local receipt time)` at acceptance. Expiry (liveness
/// and typing) is computed from the effective stamp, never from the raw
/// sender stamp and never from a later arrival time: a past-stamped
/// heartbeat keeps its own stamp as the basis, so re-delivering a captured
/// heartbeat cannot extend its life, and a future-stamped heartbeat accepted
/// inside [`MAX_FUTURE_SKEW_SECS`] is clamped to its receipt time, so its
/// skew cannot extend its life either (see [`PresenceTable::update_at`]).
#[derive(Clone, Debug)]
struct Entry {
    heartbeat: PresenceHeartbeat,
    effective_ts: u64,
}

/// Thread-safe in-memory table of the latest presence heartbeat per node.
///
/// Cheaply cloneable (backed by `Arc`) and safe to share across async tasks, like
/// [`crate::discovery::PeerDiscovery`]. All state is ephemeral by construction: the
/// table is never persisted, journaled, synced, or replayed.
#[derive(Debug, Clone, Default)]
pub struct PresenceTable {
    entries: Arc<RwLock<HashMap<AuthorKey, Entry>>>,
}

/// Current wall-clock time in unix seconds.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl PresenceTable {
    /// Creates a new empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a heartbeat received at the local wall clock, with no membership
    /// scoping (the transport layer's view; scoping is the receiver's job — see
    /// [`update_scoped`](Self::update_scoped)). Returns whether it was accepted.
    pub fn update(&self, heartbeat: PresenceHeartbeat) -> bool {
        self.update_at(heartbeat, now_secs(), &|_, _| true)
    }

    /// Records a heartbeat received at the local wall clock, keeping only the
    /// channels `is_member` confirms the sender belongs to. Returns whether the
    /// heartbeat was accepted.
    pub fn update_scoped(
        &self,
        heartbeat: PresenceHeartbeat,
        is_member: &dyn Fn(&AuthorKey, &Hash) -> bool,
    ) -> bool {
        self.update_at(heartbeat, now_secs(), is_member)
    }

    /// Records a heartbeat as of `now` (unix seconds), the deterministic core of
    /// [`update`](Self::update) and [`update_scoped`](Self::update_scoped).
    ///
    /// Three guards, mirroring [`crate::discovery::PeerDiscovery`]:
    ///
    /// - **Freshness bound.** A heartbeat stamped older than [`PRESENCE_TTL_SECS`]
    ///   relative to `now` is refused — replaying a captured heartbeat cannot keep a
    ///   departed node live — and so is one stamped more than
    ///   [`MAX_FUTURE_SKEW_SECS`] in the future (#8865: the skew allowance is
    ///   separate from, and much smaller than, the TTL), which could otherwise
    ///   poison the ordering check below and stretch liveness past the TTL.
    /// - **Ordering.** A heartbeat older than the stored one for the same node is
    ///   dropped: out-of-order delivery never regresses the table. Equal timestamps
    ///   are a tie; the most recent arrival wins — but because expiry is computed
    ///   from the *effective* stamp (`min(ts, receipt time)` at first acceptance),
    ///   a duplicate delivery of the same heartbeat never extends its life, and
    ///   no claim can outlive `effective_ts + PRESENCE_TTL_SECS` — at most one
    ///   TTL from receipt — no matter when copies of it arrive.
    /// - **Scoping.** Channels (and typing claims) the sender is not a member of,
    ///   per `is_member`, are filtered out before the heartbeat is stored, and
    ///   duplicates are dropped: a wire-decoded heartbeat bypasses
    ///   [`PresenceHeartbeat::new`], so the constructor's dedupe/bound on
    ///   channels and typing is enforced here too.
    ///
    /// Returns whether the heartbeat was accepted (inserted or replacing).
    pub fn update_at(
        &self,
        heartbeat: PresenceHeartbeat,
        now: u64,
        is_member: &dyn Fn(&AuthorKey, &Hash) -> bool,
    ) -> bool {
        if now.saturating_sub(heartbeat.ts) > PRESENCE_TTL_SECS
            || heartbeat.ts.saturating_sub(now) > MAX_FUTURE_SKEW_SECS
        {
            return false;
        }
        let node = heartbeat.node;
        let mut channels: Vec<Hash> = Vec::new();
        for channel in &heartbeat.channels {
            if channels.len() >= MAX_HEARTBEAT_CHANNELS {
                break;
            }
            if !channels.contains(channel) && is_member(&node, channel) {
                channels.push(channel.clone());
            }
        }
        let mut typing: Vec<Hash> = Vec::new();
        for channel in &heartbeat.typing {
            if channels.contains(channel) && !typing.contains(channel) {
                typing.push(channel.clone());
            }
        }
        let heartbeat = PresenceHeartbeat {
            channels,
            typing,
            ..heartbeat
        };
        let Ok(mut entries) = self.entries.write() else {
            return false;
        };
        if let Some(existing) = entries.get(&node)
            && heartbeat.ts < existing.heartbeat.ts
        {
            return false;
        }
        // The effective stamp (#8865): a past stamp stands as-is; a future
        // stamp inside the skew allowance is clamped to the receipt time,
        // so neither replay nor skew can push expiry past its basis. A
        // duplicate delivery of the same stamp keeps the earliest
        // effective stamp that stamp ever earned.
        let mut effective_ts = heartbeat.ts.min(now);
        if let Some(existing) = entries.get(&node)
            && heartbeat.ts == existing.heartbeat.ts
        {
            effective_ts = effective_ts.min(existing.effective_ts);
        }
        entries.insert(
            node,
            Entry {
                heartbeat,
                effective_ts,
            },
        );
        true
    }

    /// The liveness of `node` in `channel` at the wall clock.
    #[must_use]
    pub fn liveness(&self, node: &AuthorKey, channel: &Hash) -> Liveness {
        self.liveness_at(node, channel, now_secs())
    }

    /// The liveness of `node` in `channel` as of `now`: [`Liveness::Live`] only when
    /// the node's latest heartbeat names the channel and arrived within the TTL.
    /// Silence past the TTL — a crash, a netsplit, a stopped process — reads as
    /// [`Liveness::Offline`]; there is no leave message to wait for.
    #[must_use]
    pub fn liveness_at(&self, node: &AuthorKey, channel: &Hash, now: u64) -> Liveness {
        let Ok(entries) = self.entries.read() else {
            return Liveness::Offline;
        };
        match entries.get(node) {
            Some(entry)
                if entry.heartbeat.channels.contains(channel)
                    && now.saturating_sub(entry.effective_ts) <= PRESENCE_TTL_SECS =>
            {
                Liveness::Live
            }
            _ => Liveness::Offline,
        }
    }

    /// The nodes live in `channel` as of `now`, sorted by key.
    #[must_use]
    pub fn live_nodes_at(&self, channel: &Hash, now: u64) -> Vec<AuthorKey> {
        let Ok(entries) = self.entries.read() else {
            return Vec::new();
        };
        let mut nodes: Vec<AuthorKey> = entries
            .iter()
            .filter(|(_, entry)| {
                entry.heartbeat.channels.contains(channel)
                    && now.saturating_sub(entry.effective_ts) <= PRESENCE_TTL_SECS
            })
            .map(|(node, _)| *node)
            .collect();
        nodes.sort_unstable();
        nodes
    }

    /// The roster for `channel` as of `now`: `members` (the backing group's folded
    /// member set) each annotated with last-seen liveness, sorted by key. Members
    /// never seen appear as [`Liveness::Offline`] with no `last_seen`.
    #[must_use]
    pub fn roster_at(&self, channel: &Hash, members: &[AuthorKey], now: u64) -> Vec<RosterEntry> {
        let mut sorted: Vec<AuthorKey> = members.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        let Ok(entries) = self.entries.read() else {
            return sorted
                .into_iter()
                .map(|node| RosterEntry {
                    node,
                    liveness: Liveness::Offline,
                    last_seen: None,
                    typing: false,
                })
                .collect();
        };
        sorted
            .into_iter()
            .map(|node| {
                let entry = entries
                    .get(&node)
                    .filter(|entry| entry.heartbeat.channels.contains(channel));
                let liveness = match entry {
                    Some(entry) if now.saturating_sub(entry.effective_ts) <= PRESENCE_TTL_SECS => {
                        Liveness::Live
                    }
                    _ => Liveness::Offline,
                };
                RosterEntry {
                    node,
                    liveness,
                    last_seen: entry.map(|entry| entry.effective_ts),
                    typing: match entry {
                        Some(entry) => {
                            liveness == Liveness::Live
                                && entry.heartbeat.is_typing_in(channel)
                                && now.saturating_sub(entry.effective_ts) <= TYPING_TTL_SECS
                        }
                        None => false,
                    },
                }
            })
            .collect()
    }

    /// The roster for `channel` as of `now`, with membership taken from the backing
    /// group's fold: `view` is the [`GroupView`] for the channel's group.
    #[must_use]
    pub fn roster_for_group_at(
        &self,
        channel: &Hash,
        view: &GroupView,
        now: u64,
    ) -> Vec<RosterEntry> {
        let members: Vec<AuthorKey> = view.members.keys().copied().collect();
        self.roster_at(channel, &members, now)
    }

    /// The roster for `channel` as `viewer` may see it as of `now`: `None` when the
    /// viewer is not one of `members`, so a non-member learns nothing about a private
    /// channel — not its roster, not who is live in it.
    #[must_use]
    pub fn roster_for_viewer_at(
        &self,
        channel: &Hash,
        members: &[AuthorKey],
        viewer: &AuthorKey,
        now: u64,
    ) -> Option<Vec<RosterEntry>> {
        if !members.contains(viewer) {
            return None;
        }
        Some(self.roster_at(channel, members, now))
    }

    /// Removes entries whose last heartbeat can no longer be replayed into
    /// the table, returning how many were removed. Liveness expires on the
    /// effective stamp, but an entry is retained as a replay watermark until
    /// its *sender* stamp also ages past [`PRESENCE_TTL_SECS`]: pruning on
    /// the effective stamp alone would delete a future-dated entry while
    /// its stamp still passes the freshness bound, and a replay would then
    /// re-insert it with a later effective stamp and resurrect the node
    /// past its receipt-based expiry (#8865). Stale entries already read as
    /// gone; pruning only bounds the table's size.
    pub fn prune_at(&self, now: u64) -> usize {
        let Ok(mut entries) = self.entries.write() else {
            return 0;
        };
        let before = entries.len();
        entries.retain(|_, entry| now.saturating_sub(entry.heartbeat.ts) <= PRESENCE_TTL_SECS);
        before - entries.len()
    }

    /// The number of nodes with a stored heartbeat (live or not yet pruned).
    #[must_use]
    pub fn count(&self) -> usize {
        self.entries.read().map_or(0, |entries| entries.len())
    }
}

/// Encodes a heartbeat as a gossip frame: [`PRESENCE_FRAME_MAGIC`] plus the
/// postcard-serialized heartbeat, the encoding the gossip plane carries.
///
/// A heartbeat is plain data and serializes infallibly in practice; if
/// it ever failed, the frame degrades to the bare magic, which
/// [`decode_frame`] refuses.
#[must_use]
pub fn encode_frame(heartbeat: &PresenceHeartbeat) -> Vec<u8> {
    let mut frame = PRESENCE_FRAME_MAGIC.to_vec();
    let bytes = postcard::to_allocvec(heartbeat).unwrap_or_default();
    frame.extend_from_slice(&bytes);
    frame
}

/// Decodes a gossip frame as a presence heartbeat, or `None` when the frame is not
/// a presence frame (no magic prefix) or does not decode. Anything else on the
/// shared gossip topic — peer announcements included — is refused, never guessed at.
#[must_use]
pub fn decode_frame(frame: &[u8]) -> Option<PresenceHeartbeat> {
    let body = frame.strip_prefix(PRESENCE_FRAME_MAGIC)?;
    // `take_from_bytes`, not `from_bytes`: trailing bytes after the
    // heartbeat must refuse the frame, not be silently ignored — the
    // signature is verified over a canonical re-encoding of the parsed
    // fields, so an unsigned suffix would otherwise give one signed
    // heartbeat many accepted byte spellings (#6197; same pattern as
    // world/replication.rs).
    let (heartbeat, rest) = postcard::take_from_bytes(body).ok()?;
    if !rest.is_empty() {
        return None;
    }
    Some(heartbeat)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn key(byte: u8) -> AuthorKey {
        [byte; 32]
    }

    fn channel(name: &str) -> Hash {
        Hash::digest(name.as_bytes())
    }

    fn heartbeat(node: AuthorKey, channels: Vec<Hash>, ts: u64) -> PresenceHeartbeat {
        PresenceHeartbeat::new(node, channels, Vec::new(), ts)
    }

    #[test]
    fn heartbeat_keeps_a_node_live_until_the_ttl_then_reads_as_gone() {
        let table = PresenceTable::new();
        let chan = channel("general");
        let alice = key(1);
        assert!(table.update_at(heartbeat(alice, vec![chan], 1_000), 1_000, &|_, _| true));
        assert_eq!(table.liveness_at(&alice, &chan, 1_000), Liveness::Live);
        assert_eq!(
            table.liveness_at(&alice, &chan, 1_000 + PRESENCE_TTL_SECS),
            Liveness::Live
        );
        // No leave message arrives; silence past the TTL is the whole signal.
        assert_eq!(
            table.liveness_at(&alice, &chan, 1_001 + PRESENCE_TTL_SECS),
            Liveness::Offline
        );
        assert_eq!(table.prune_at(1_001 + PRESENCE_TTL_SECS), 1);
        assert_eq!(table.count(), 0);
    }

    #[test]
    fn two_nodes_exchange_frames_and_recover_across_a_simulated_stop_resume() {
        // Two tables, one per node, exchanging exactly the frames the gossip plane
        // carries. The simulated netsplit is a stop: alice's heartbeats cease (as
        // under kill -STOP), the clock runs past the TTL, then she resumes.
        let chan = channel("general");
        let (alice, bob) = (key(1), key(2));
        let members = [alice, bob];
        let alice_table = PresenceTable::new();
        let bob_table = PresenceTable::new();
        let exchange =
            |from: &PresenceTable, to: &PresenceTable, hb: PresenceHeartbeat, now: u64| {
                let frame = encode_frame(&hb);
                let decoded = decode_frame(&frame).expect("presence frame decodes");
                assert!(to.update_at(decoded, now, &|_, _| true));
                let _ = from;
            };

        exchange(
            &alice_table,
            &bob_table,
            heartbeat(alice, vec![chan], 2_000),
            2_000,
        );
        exchange(
            &bob_table,
            &alice_table,
            heartbeat(bob, vec![chan], 2_000),
            2_000,
        );
        // Both nodes show each other live.
        assert_eq!(bob_table.liveness_at(&alice, &chan, 2_010), Liveness::Live);
        assert_eq!(alice_table.liveness_at(&bob, &chan, 2_010), Liveness::Live);

        // Stop: no heartbeats from alice. Past the TTL she reads as gone on bob's
        // roster, while bob (still heartbeating) stays live.
        let after_stop = 2_000 + PRESENCE_TTL_SECS + 10;
        assert!(bob_table.update_at(
            heartbeat(bob, vec![chan], after_stop),
            after_stop,
            &|_, _| true
        ));
        let roster = bob_table.roster_at(&chan, &members, after_stop);
        let alice_row = roster.iter().find(|row| row.node == alice).unwrap();
        assert_eq!(alice_row.liveness, Liveness::Offline);
        assert_eq!(alice_row.last_seen, Some(2_000));

        // Resume: alice's next heartbeat makes her live again, no rejoin needed.
        let resumed = after_stop + 5;
        exchange(
            &alice_table,
            &bob_table,
            heartbeat(alice, vec![chan], resumed),
            resumed,
        );
        assert_eq!(
            bob_table.liveness_at(&alice, &chan, resumed),
            Liveness::Live
        );
        let roster = bob_table.roster_at(&chan, &members, resumed);
        assert!(roster.iter().all(|row| row.liveness == Liveness::Live));
    }

    #[test]
    fn roster_is_the_member_set_annotated_with_liveness() {
        let table = PresenceTable::new();
        let chan = channel("general");
        let (alice, bob, carol) = (key(1), key(2), key(3));
        assert!(table.update_at(heartbeat(alice, vec![chan], 3_000), 3_000, &|_, _| true));
        // Bob was seen, but in a different channel; carol was never seen.
        let other = channel("other");
        assert!(table.update_at(heartbeat(bob, vec![other], 3_000), 3_000, &|_, _| true));

        let roster = table.roster_at(&chan, &[alice, bob, carol], 3_010);
        assert_eq!(roster.len(), 3);
        let by_node = |node: AuthorKey| roster.iter().find(|row| row.node == node).unwrap();
        assert_eq!(by_node(alice).liveness, Liveness::Live);
        assert_eq!(by_node(alice).last_seen, Some(3_000));
        assert_eq!(by_node(bob).liveness, Liveness::Offline);
        assert_eq!(by_node(bob).last_seen, None);
        assert_eq!(by_node(carol).liveness, Liveness::Offline);
        // A heartbeat from a non-member never enters the roster.
        let mallory = key(9);
        assert!(table.update_at(heartbeat(mallory, vec![chan], 3_000), 3_000, &|_, _| true));
        assert_eq!(table.roster_at(&chan, &[alice, bob, carol], 3_010).len(), 3);
        assert_eq!(table.live_nodes_at(&chan, 3_010), vec![alice, mallory]);
    }

    #[test]
    fn private_channel_presence_is_scoped_to_members() {
        let table = PresenceTable::new();
        let public = channel("public");
        let private = channel("private");
        let (alice, bob) = (key(1), key(2));
        // Alice belongs to both channels; bob only to the public one. Bob's table
        // applies that membership on receipt: bob's predicate rejects the private
        // channel outright, because bob is not a member of it and so may not store
        // (or even confirm) presence metadata for it.
        let bob_predicate = |_node: &AuthorKey, chan: &Hash| -> bool { *chan != private };
        assert!(table.update_at(
            heartbeat(alice, vec![public, private], 4_000),
            4_000,
            &bob_predicate,
        ));
        assert_eq!(table.liveness_at(&alice, &public, 4_000), Liveness::Live);
        assert_eq!(
            table.liveness_at(&alice, &private, 4_000),
            Liveness::Offline
        );

        // Read-side scoping: a non-member viewer gets no roster for the private
        // channel at all; a member viewer does.
        let members = [alice, bob];
        assert!(
            table
                .roster_for_viewer_at(&private, &[alice], &bob, 4_000)
                .is_none()
        );
        assert!(
            table
                .roster_for_viewer_at(&public, &members, &bob, 4_000)
                .is_some()
        );
    }

    #[test]
    fn typing_rides_the_same_path_on_the_shorter_ttl() {
        let table = PresenceTable::new();
        let chan = channel("general");
        let alice = key(1);
        let mut hb = heartbeat(alice, vec![chan], 5_000);
        hb.typing = vec![chan];
        assert!(table.update_at(hb, 5_000, &|_, _| true));
        let roster = table.roster_at(&chan, &[alice], 5_000 + TYPING_TTL_SECS);
        assert!(roster[0].typing);
        // The typing claim expires on its own clock while presence stays live.
        let roster = table.roster_at(&chan, &[alice], 5_001 + TYPING_TTL_SECS);
        assert!(!roster[0].typing);
        assert_eq!(roster[0].liveness, Liveness::Live);
    }

    #[test]
    fn replayed_and_out_of_order_heartbeats_are_refused() {
        let table = PresenceTable::new();
        let chan = channel("general");
        let alice = key(1);
        assert!(table.update_at(heartbeat(alice, vec![chan], 6_100), 6_100, &|_, _| true));
        // Out of order: an older heartbeat never regresses the table.
        assert!(!table.update_at(heartbeat(alice, vec![], 6_050), 6_100, &|_, _| true));
        assert_eq!(table.liveness_at(&alice, &chan, 6_100), Liveness::Live);
        // Replay: re-delivering the last genuine heartbeat after the node went
        // silent is refused by the freshness bound, so it cannot resurrect her.
        let silent_now = 6_100 + PRESENCE_TTL_SECS + 30;
        assert!(
            !table.update_at(heartbeat(alice, vec![chan], 6_100), silent_now, &|_, _| {
                true
            })
        );
        assert_eq!(
            table.liveness_at(&alice, &chan, silent_now),
            Liveness::Offline
        );
        // A far-future stamp is refused too: it must not poison ordering.
        assert!(!table.update_at(heartbeat(alice, vec![chan], u64::MAX), 6_100, &|_, _| true));
    }

    #[test]
    fn future_skew_at_the_allowance_expires_from_receipt_not_from_the_stamp() {
        // #8865 oracle, boundary at the allowance: a heartbeat stamped exactly
        // MAX_FUTURE_SKEW_SECS ahead of receipt is accepted, but its expiry
        // runs from receipt — live at receipt + TTL, gone one second later,
        // NOT live until stamp + TTL. Typing rides the same basis, and
        // last_seen reports the receipt-clamped effective stamp.
        let table = PresenceTable::new();
        let chan = channel("general");
        let alice = key(1);
        let receipt = 1_000;
        let mut hb = heartbeat(alice, vec![chan], receipt + MAX_FUTURE_SKEW_SECS);
        hb.typing = vec![chan];
        assert!(table.update_at(hb, receipt, &|_, _| true));
        assert_eq!(table.liveness_at(&alice, &chan, receipt), Liveness::Live);
        assert_eq!(
            table.liveness_at(&alice, &chan, receipt + PRESENCE_TTL_SECS),
            Liveness::Live
        );
        assert_eq!(
            table.liveness_at(&alice, &chan, receipt + PRESENCE_TTL_SECS + 1),
            Liveness::Offline,
            "a future-dated heartbeat must not stay live past one TTL from receipt"
        );
        // Typing expires on the typing TTL from receipt, not from the stamp.
        let roster = table.roster_at(&chan, &[alice], receipt + TYPING_TTL_SECS);
        assert!(roster[0].typing);
        assert_eq!(roster[0].last_seen, Some(receipt));
        let roster = table.roster_at(&chan, &[alice], receipt + TYPING_TTL_SECS + 1);
        assert!(!roster[0].typing);
        // Pruning retains the entry past its liveness expiry as a replay
        // watermark (see the replay test below); it is removed once the
        // sender stamp itself ages past the TTL.
        assert_eq!(table.prune_at(receipt + PRESENCE_TTL_SECS + 1), 0);
        assert_eq!(
            table.prune_at(receipt + MAX_FUTURE_SKEW_SECS + PRESENCE_TTL_SECS + 1),
            1
        );
        assert_eq!(table.count(), 0);
    }

    #[test]
    fn replay_after_receipt_expiry_does_not_resurrect_a_future_dated_heartbeat() {
        // #8865, Codex finding on the fix: a future-dated heartbeat pruned on
        // its effective stamp could be replayed while its sender stamp is
        // still fresh, re-entering with a later effective stamp. The entry
        // is therefore retained as a watermark: the replay is a tie that
        // keeps the first effective stamp, liveness stays Offline, and once
        // the watermark itself is pruned the stamp is too old to replay.
        let table = PresenceTable::new();
        let chan = channel("general");
        let alice = key(1);
        let hb = heartbeat(alice, vec![chan], 1_010);
        assert!(table.update_at(hb.clone(), 1_000, &|_, _| true));
        let after_expiry = 1_000 + PRESENCE_TTL_SECS + 1;
        assert_eq!(
            table.liveness_at(&alice, &chan, after_expiry),
            Liveness::Offline
        );
        assert_eq!(table.prune_at(after_expiry), 0, "watermark retained");
        assert!(table.update_at(hb.clone(), after_expiry, &|_, _| true));
        assert_eq!(
            table.liveness_at(&alice, &chan, after_expiry),
            Liveness::Offline,
            "a replay must not resurrect the node past its receipt-based expiry"
        );
        let roster = table.roster_at(&chan, &[alice], after_expiry);
        assert_eq!(roster[0].last_seen, Some(1_000));
        // Once the sender stamp ages past the TTL the watermark is pruned,
        // and the same replay is refused by the freshness bound.
        let stamp_expired = 1_010 + PRESENCE_TTL_SECS + 1;
        assert_eq!(table.prune_at(stamp_expired), 1);
        assert!(!table.update_at(hb, stamp_expired, &|_, _| true));
        assert_eq!(table.count(), 0);
    }

    #[test]
    fn future_skew_just_beyond_the_allowance_is_refused() {
        // #8865 oracle, boundary just beyond the allowance — including the
        // stamp the old TTL-sized allowance used to accept (receipt + 75):
        // both are refused outright, so neither can extend liveness at all.
        let table = PresenceTable::new();
        let chan = channel("general");
        let alice = key(1);
        let receipt = 2_000;
        assert!(!table.update_at(
            heartbeat(alice, vec![chan], receipt + MAX_FUTURE_SKEW_SECS + 1),
            receipt,
            &|_, _| true
        ));
        assert!(!table.update_at(
            heartbeat(alice, vec![chan], receipt + PRESENCE_TTL_SECS),
            receipt,
            &|_, _| true
        ));
        assert_eq!(table.count(), 0);
        assert_eq!(table.liveness_at(&alice, &chan, receipt), Liveness::Offline);
    }

    #[test]
    fn redelivered_future_heartbeat_keeps_its_first_effective_stamp() {
        // Re-delivery inside the skew window must not re-clamp to the later
        // receipt and so extend the claim: the earliest effective stamp wins.
        let table = PresenceTable::new();
        let chan = channel("general");
        let alice = key(1);
        let hb = heartbeat(alice, vec![chan], 3_010);
        assert!(table.update_at(hb.clone(), 3_000, &|_, _| true));
        assert!(table.update_at(hb, 3_010, &|_, _| true));
        assert_eq!(
            table.liveness_at(&alice, &chan, 3_000 + PRESENCE_TTL_SECS + 1),
            Liveness::Offline
        );
        let roster = table.roster_at(&chan, &[alice], 3_010);
        assert_eq!(roster[0].last_seen, Some(3_000));
    }

    #[test]
    fn frames_require_the_presence_magic() {
        let chan = channel("general");
        let hb = heartbeat(key(1), vec![chan], 7_000);
        let frame = encode_frame(&hb);
        assert_eq!(decode_frame(&frame), Some(hb.clone()));
        // The same heartbeat serialized without the frame magic — the shape of a
        // legacy announcement payload — never decodes as presence.
        let bare = postcard::to_allocvec(&hb).unwrap();
        assert_eq!(decode_frame(&bare), None);
        assert_eq!(decode_frame(b"garbage"), None);
        assert_eq!(decode_frame(&[]), None);
    }

    #[test]
    fn frames_refuse_trailing_bytes() {
        // #6197 oracle: the full-consumption invariant — a frame is exactly
        // one postcard value after the magic; any remainder refuses it,
        // while the genuine frame still decodes.
        let chan = channel("general");
        let hb = heartbeat(key(1), vec![chan], 7_100);
        let frame = encode_frame(&hb);
        assert_eq!(decode_frame(&frame), Some(hb.clone()));

        let mut trailing_byte = frame.clone();
        trailing_byte.push(0x00);
        assert_eq!(decode_frame(&trailing_byte), None);

        let mut trailing_garbage = frame.clone();
        trailing_garbage.extend_from_slice(b"garbage!!");
        assert_eq!(decode_frame(&trailing_garbage), None);

        let mut trailing_second = frame.clone();
        trailing_second.extend_from_slice(&postcard::to_allocvec(&hb).unwrap());
        assert_eq!(decode_frame(&trailing_second), None);
    }

    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn frames_refuse_any_trailing_suffix(
            node_byte in any::<u8>(),
            ts in any::<u64>(),
            suffix in proptest::collection::vec(any::<u8>(), 1..32),
        ) {
            // #6197 property form of the full-consumption invariant
            // (oracle: the invariant itself, checked over generated
            // heartbeats and arbitrary nonempty suffixes): the genuine
            // frame round-trips, the suffixed frame never decodes.
            let chan = channel("general");
            let hb = heartbeat(key(node_byte), vec![chan], ts);
            let frame = encode_frame(&hb);
            prop_assert_eq!(decode_frame(&frame), Some(hb));
            let mut suffixed = frame;
            suffixed.extend_from_slice(&suffix);
            prop_assert_eq!(decode_frame(&suffixed), None);
        }
    }

    #[test]
    fn duplicate_delivery_does_not_extend_expiry() {
        // A captured heartbeat re-delivered inside the freshness window is
        // accepted (equal stamp, latest arrival wins) but must not extend the
        // claim's life: expiry runs from the stamp, so the node still goes
        // stale at ts + TTL, and a typing claim dies on its own stamp clock.
        let table = PresenceTable::new();
        let chan = channel("general");
        let alice = key(1);
        let mut hb = heartbeat(alice, vec![chan], 9_000);
        hb.typing = vec![chan];
        assert!(table.update_at(hb.clone(), 9_000, &|_, _| true));
        let replay_at = 9_000 + PRESENCE_TTL_SECS;
        assert!(table.update_at(hb, replay_at, &|_, _| true));
        assert_eq!(table.liveness_at(&alice, &chan, replay_at), Liveness::Live);
        assert_eq!(
            table.liveness_at(&alice, &chan, replay_at + 1),
            Liveness::Offline
        );
        let roster = table.roster_at(&chan, &[alice], replay_at);
        assert!(
            !roster[0].typing,
            "a replayed typing claim must not refresh"
        );
    }

    #[test]
    fn heartbeats_authenticate_by_signature() {
        let chan = channel("general");
        let alice_key = SigningKey::from_bytes(&[7u8; 32]);
        let alice = alice_key.verifying_key().to_bytes();

        let mut hb = heartbeat(alice, vec![chan], 9_500);
        assert!(!hb.verify(), "an unsigned heartbeat is not authenticated");
        hb.sign(&alice_key);
        assert_eq!(hb.node, alice);
        assert!(hb.verify());

        // Tampering after signing — channels, stamp, or the claimed node —
        // breaks the signature.
        let mut forged = hb.clone();
        forged.channels = Vec::new();
        assert!(!forged.verify());
        let mut forged = hb.clone();
        forged.ts += 1;
        assert!(!forged.verify());
        let mut forged = hb;
        forged.node = SigningKey::from_bytes(&[8u8; 32])
            .verifying_key()
            .to_bytes();
        assert!(!forged.verify());
    }

    #[test]
    fn verify_rejects_the_small_order_identity_forgery() {
        // #1386: the identity point is a small-order key, and the shared
        // forgery (R = identity, S = 0) passes plain verify for any
        // message under it — only verify_strict keeps it out of the
        // presence authentication gate.
        use crate::test_forgeries::{IDENTITY_KEY, identity_forgery};
        let chan = channel("general");
        let mut hb = heartbeat(IDENTITY_KEY, vec![chan], 9_600);
        hb.signature = identity_forgery().to_vec();
        assert!(!hb.verify());
    }

    #[test]
    fn verify_rejects_a_non_canonical_s() {
        // #1386: a genuine signature mutated to S + L must be refused,
        // while the original verifies.
        use crate::test_forgeries::with_non_canonical_s;
        let chan = channel("general");
        let alice_key = SigningKey::from_bytes(&[7u8; 32]);
        let alice = alice_key.verifying_key().to_bytes();
        let mut hb = heartbeat(alice, vec![chan], 9_700);
        hb.sign(&alice_key);
        assert!(hb.verify());
        let signature: [u8; 64] = hb
            .signature
            .clone()
            .try_into()
            .expect("signed heartbeat carries a 64-byte signature");
        hb.signature = with_non_canonical_s(signature).to_vec();
        assert!(!hb.verify());
    }

    #[test]
    fn presence_state_is_memory_only() {
        // Nothing about presence is journaled or persisted: a fresh table — the
        // state after a restart, before any sync or replay could run — knows
        // nothing, and liveness is re-learned only from new heartbeats.
        let chan = channel("general");
        let alice = key(1);
        let first = PresenceTable::new();
        assert!(first.update_at(heartbeat(alice, vec![chan], 8_000), 8_000, &|_, _| true));
        let restarted = PresenceTable::new();
        assert_eq!(restarted.count(), 0);
        assert_eq!(
            restarted.liveness_at(&alice, &chan, 8_000),
            Liveness::Offline
        );
    }

    #[test]
    fn constructor_deduplicates_and_bounds_typing_claims() {
        let chan = channel("general");
        let other = channel("random");
        let alice = key(1);
        // 100,000 duplicate typing entries for one retained channel collapse to
        // the single claim; a claim for a channel the heartbeat does not claim
        // presence in drops too.
        let mut typing = vec![chan; 100_000];
        typing.push(other);
        let hb = PresenceHeartbeat::new(alice, vec![chan], typing, 10_000);
        assert_eq!(hb.typing, vec![chan]);
        assert!(hb.typing.len() <= hb.channels.len());
        // The encoded frame stays small: the ~3.3 MB duplicate blowup is gone.
        assert!(encode_frame(&hb).len() < 1_000);
    }

    #[test]
    fn wire_decoded_heartbeat_typing_is_deduplicated_and_bounded_on_store() {
        // A heartbeat decoded off the wire bypasses PresenceHeartbeat::new, so
        // update_at must enforce the same dedupe/bound before storing it.
        let table = PresenceTable::new();
        let chan = channel("general");
        let alice = key(1);
        let mut hb = heartbeat(alice, vec![chan], 11_000);
        hb.typing = vec![chan; 50_000];
        let frame = encode_frame(&hb);
        let decoded = decode_frame(&frame).expect("presence frame decodes");
        assert_eq!(
            decoded.typing.len(),
            50_000,
            "decode itself does not sanitize"
        );
        assert!(table.update_at(decoded, 11_000, &|_, _| true));
        let entries = table.entries.read().unwrap();
        let stored = &entries.get(&alice).expect("heartbeat stored").heartbeat;
        assert_eq!(stored.typing, vec![chan]);
        assert!(stored.typing.len() <= stored.channels.len());
    }

    #[test]
    fn wire_decoded_duplicate_channels_are_deduplicated_on_store() {
        let table = PresenceTable::new();
        let chan = channel("general");
        let alice = key(1);
        let mut hb = heartbeat(alice, vec![chan], 12_000);
        hb.channels = vec![chan; 500];
        hb.typing = vec![chan; 500];
        assert!(table.update_at(hb, 12_000, &|_, _| true));
        let entries = table.entries.read().unwrap();
        let stored = &entries.get(&alice).expect("heartbeat stored").heartbeat;
        assert_eq!(stored.channels, vec![chan]);
        assert_eq!(stored.typing, vec![chan]);
    }
    #[test]
    fn update_at_dedups_wire_channels_before_the_cap() {
        // #4404: a decoded wire heartbeat may repeat a channel (the constructor
        // dedups, but the wire carries the raw signed list). Duplicates must not
        // displace distinct channels the heartbeat genuinely claims.
        let table = PresenceTable::new();
        let alice = key(1);
        let a = channel("a");
        let b = channel("b");
        let mut raw = vec![a; MAX_HEARTBEAT_CHANNELS];
        raw.push(b);
        let wire = PresenceHeartbeat {
            node: alice,
            channels: raw,
            typing: Vec::new(),
            ts: 4_100,
            signature: Vec::new(),
        };
        // Through the encode/decode round trip, as the gossip plane carries it.
        let frame = encode_frame(&wire);
        let decoded = decode_frame(&frame).expect("presence frame decodes");
        assert!(table.update_at(decoded, 4_100, &|_, _| true));
        assert_eq!(table.liveness_at(&alice, &a, 4_100), Liveness::Live);
        assert_eq!(
            table.liveness_at(&alice, &b, 4_100),
            Liveness::Live,
            "[A x64, B] must keep B live: duplicates fill the cap, not displace B"
        );
    }

    #[test]
    fn update_at_keeps_every_distinct_channel_past_duplicate_fill() {
        // [A x63, B, C]: after dedup only three distinct channels exist, so all
        // three survive the cap — C must not be truncated away.
        let table = PresenceTable::new();
        let alice = key(1);
        let (a, b, c) = (channel("a"), channel("b"), channel("c"));
        let mut raw = vec![a; MAX_HEARTBEAT_CHANNELS - 2];
        raw.push(b);
        raw.push(c);
        let wire = PresenceHeartbeat {
            node: alice,
            channels: raw,
            typing: Vec::new(),
            ts: 4_200,
            signature: Vec::new(),
        };
        let frame = encode_frame(&wire);
        let decoded = decode_frame(&frame).expect("presence frame decodes");
        assert!(table.update_at(decoded, 4_200, &|_, _| true));
        for chan in [a, b, c] {
            assert_eq!(table.liveness_at(&alice, &chan, 4_200), Liveness::Live);
        }
    }
}
