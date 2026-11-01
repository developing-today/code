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

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use p2panda_core::Hash;
use serde::{Deserialize, Serialize};

use crate::groups::GroupView;
use crate::waiting::AuthorKey;

/// How long a heartbeat keeps a node live, in seconds: 75, inside the 60–90 s band
/// #2809 specifies. A node silent for longer than this reads as gone.
pub const PRESENCE_TTL_SECS: u64 = 75;

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
    /// Builds a heartbeat for `node` at `ts`, dropping duplicate channels and typing
    /// claims for channels the heartbeat does not also claim presence in.
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
        let typing: Vec<Hash> = typing
            .into_iter()
            .filter(|channel| channels.contains(channel))
            .collect();
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
            .verify(&self.signing_bytes(), &signature)
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
    /// or `None` if the member has never been seen.
    pub last_seen: Option<u64>,
    /// Whether the member's typing claim for the channel is currently live.
    pub typing: bool,
}

/// The latest accepted heartbeat for one node. Expiry is computed from the
/// heartbeat's own stamp, never from a local arrival time, so re-delivering a
/// captured heartbeat cannot extend its life (see [`PresenceTable::update_at`]).
#[derive(Clone, Debug)]
struct Entry {
    heartbeat: PresenceHeartbeat,
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
    ///   departed node live — and so is one stamped more than the TTL in the future,
    ///   which could otherwise poison the ordering check below.
    /// - **Ordering.** A heartbeat older than the stored one for the same node is
    ///   dropped: out-of-order delivery never regresses the table. Equal timestamps
    ///   are a tie; the most recent arrival wins — but because expiry is computed
    ///   from the stamp (`ts + PRESENCE_TTL_SECS`), a duplicate delivery of the
    ///   same heartbeat never extends its life, and no claim can outlive its
    ///   stamp's freshness bound no matter when copies of it arrive.
    /// - **Scoping.** Channels (and typing claims) the sender is not a member of,
    ///   per `is_member`, are filtered out before the heartbeat is stored.
    ///
    /// Returns whether the heartbeat was accepted (inserted or replacing).
    pub fn update_at(
        &self,
        heartbeat: PresenceHeartbeat,
        now: u64,
        is_member: &dyn Fn(&AuthorKey, &Hash) -> bool,
    ) -> bool {
        if now.saturating_sub(heartbeat.ts) > PRESENCE_TTL_SECS
            || heartbeat.ts.saturating_sub(now) > PRESENCE_TTL_SECS
        {
            return false;
        }
        let node = heartbeat.node;
        let channels: Vec<Hash> = heartbeat
            .channels
            .iter()
            .filter(|channel| is_member(&node, channel))
            .take(MAX_HEARTBEAT_CHANNELS)
            .cloned()
            .collect();
        let typing: Vec<Hash> = heartbeat
            .typing
            .iter()
            .filter(|channel| channels.contains(channel))
            .cloned()
            .collect();
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
        entries.insert(node, Entry { heartbeat });
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
                    && now.saturating_sub(entry.heartbeat.ts) <= PRESENCE_TTL_SECS =>
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
                    && now.saturating_sub(entry.heartbeat.ts) <= PRESENCE_TTL_SECS
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
                    Some(entry) if now.saturating_sub(entry.heartbeat.ts) <= PRESENCE_TTL_SECS => {
                        Liveness::Live
                    }
                    _ => Liveness::Offline,
                };
                RosterEntry {
                    node,
                    liveness,
                    last_seen: entry.map(|entry| entry.heartbeat.ts),
                    typing: match entry {
                        Some(entry) => {
                            liveness == Liveness::Live
                                && entry.heartbeat.is_typing_in(channel)
                                && now.saturating_sub(entry.heartbeat.ts) <= TYPING_TTL_SECS
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

    /// Removes entries whose last heartbeat arrived more than [`PRESENCE_TTL_SECS`]
    /// before `now`, returning how many were removed. Stale entries already read as
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
    postcard::from_bytes(body).ok()
}

#[cfg(test)]
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
}
