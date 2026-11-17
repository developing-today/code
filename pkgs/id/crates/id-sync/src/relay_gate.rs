//! Validate-before-forward relay gate: phase 3 of the #2036
//! messaging/relay protocol v1 (#3796), part B — the gate slice.
//!
//! A relay runs every inbound chained message through
//! [`RelayGate::decide`] BEFORE forwarding it, per the design record
//! on #2036 (`doc/2026-11-01_design_id2036.md` §4.2): the origin
//! signature, canonical envelope, message ID, class, and local group
//! authorization are all checked first, and only a message that passes
//! every check is forwarded — with this relay's hop appended. The
//! decision is a total function: every input maps to
//! [`GateDecision::Forward`] or [`GateDecision::Refuse`], so there is
//! no path on which an unchecked message reaches the forwarder. The
//! [`GateRefusal`] reason records exactly which check failed.
//!
//! # Gate-verification finding (acceptance item 2)
//!
//! The pinned `iroh-gossip` (0.101.0, `pkgs/id/Cargo.lock`) cannot gate
//! eager forwarding on ID's accept decision, so stock gossip broadcast
//! must NOT carry chained messages:
//!
//! - In `iroh-gossip 0.101.0 `src/proto/plumtree.rs``, `on_gossip`
//!   runs exactly one check before relaying — `message.validate()`,
//!   i.e. "the message ID is the BLAKE3 hash of the message content"
//!   (self-consistency, not authorization) — and on first receipt
//!   unconditionally `eager_push`es *and* `lazy_push`es the message to
//!   peers, emitting to the application only afterwards. Forwarding
//!   happens inside the protocol state machine, before ID ever sees
//!   the bytes.
//! - The public API (`src/api.rs`: `subscribe`, `broadcast`,
//!   `broadcast_neighbors`) exposes no per-message accept/reject hook
//!   in the forward path.
//!
//! Consequence, exactly as §4.2 prescribes: gossip stays on swarm
//! discovery/repair duty only, and chained-message forwarding is
//! ID-controlled over direct Iroh connections, with this gate as the
//! only forwarding path. Shipping stock eager-forwarding plus
//! post-hoc ingest checks would propagate unsigned or unauthorized
//! traffic before validation (#2035). The multi-node bridge test is
//! in this module's tests (two gates chained origin -> bridge ->
//! leaf, plus the unauthorized-at-bridge stop). The remaining
//! follow-on slice is the live call-site wiring: feeding gossip/
//! ingest arrivals through this gate and backing
//! [`MembershipFrontier`] with the signed-graph membership state.
//!
//! # Membership frontier
//!
//! Group authorization is a trait ([`MembershipFrontier`]): the gate
//! asks whether the origin may write the envelope's group, and refuses
//! non-members before any forward. A gossip topic ID is never
//! consulted — it is not a group credential (§4.2). The authoritative
//! membership state (the point in the signed graph against which write
//! access is evaluated) is still open design; [`AllowlistFrontier`] is
//! the local stand-in the graph-backed frontier will replace without
//! changing the gate.

use std::collections::{HashMap, HashSet};

use ed25519_dalek::SigningKey;

use crate::chained_message::{
    DedupeSet, HopRecord, RelayRefusal, RelayedMessage, check_hop_chain, relay_refusal,
};
use crate::hex::hex_encode;

/// Local group-write authorization for the relay gate: is `origin`
/// permitted to write `group`? This is the seam the signed-graph
/// membership state plugs into; the gate itself only asks, never
/// interprets.
pub trait MembershipFrontier {
    /// Whether `origin` (account ID hex) may write `group`.
    fn is_authorized(&self, origin: &str, group: &str) -> bool;
}

/// A local group → member-set stand-in for the frontier: each named
/// group lists the origin account IDs allowed to write it. Unknown
/// groups authorize nobody.
#[derive(Clone, Debug, Default)]
pub struct AllowlistFrontier {
    members: HashMap<String, HashSet<String>>,
}

impl AllowlistFrontier {
    /// An empty frontier: no origin is authorized for any group.
    pub fn new() -> Self {
        Self::default()
    }

    /// Authorize `origin` to write `group`.
    pub fn allow(&mut self, group: &str, origin: &str) {
        self.members
            .entry(group.to_owned())
            .or_default()
            .insert(origin.to_owned());
    }
}

impl MembershipFrontier for AllowlistFrontier {
    fn is_authorized(&self, origin: &str, group: &str) -> bool {
        self.members
            .get(group)
            .is_some_and(|members| members.contains(origin))
    }
}

/// Why the gate refused to forward a message. Variants are ordered by
/// the check that produced them: envelope, chain, authorization,
/// dedupe, then loop/cap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateRefusal {
    /// The origin envelope failed [`RelayedMessage::verify_envelope`]:
    /// bad structure, self-referential predecessor, or a signature
    /// that does not verify. The string is the underlying failure.
    InvalidEnvelope(String),
    /// The envelope is valid but the hop chain failed
    /// [`check_hop_chain`] under the gate's cap: malformed record,
    /// mis-signed hop, broken linkage, or a repeated relay.
    InvalidChain(String),
    /// The origin may not write the envelope's group.
    NotAuthorized {
        /// The envelope's origin account ID.
        origin: String,
        /// The envelope's group.
        group: String,
    },
    /// This message ID was already forwarded; alternate paths dedupe.
    Duplicate,
    /// This relay already appears in the hop chain.
    AlreadyInChain,
    /// The hop chain has reached the gate's cap.
    CapReached,
}

impl std::fmt::Display for GateRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidEnvelope(e) => write!(f, "invalid chained-message envelope: {e}"),
            Self::InvalidChain(e) => write!(f, "invalid hop chain: {e}"),
            Self::NotAuthorized { origin, group } => {
                write!(f, "origin {origin} may not write group {group}")
            }
            Self::Duplicate => write!(f, "message ID already forwarded"),
            Self::AlreadyInChain => write!(f, "relay is already in the hop chain"),
            Self::CapReached => write!(f, "hop chain is at the cap"),
        }
    }
}

impl std::error::Error for GateRefusal {}

impl From<RelayRefusal> for GateRefusal {
    fn from(value: RelayRefusal) -> Self {
        match value {
            RelayRefusal::AlreadyInChain => Self::AlreadyInChain,
            RelayRefusal::CapReached => Self::CapReached,
        }
    }
}

/// The gate's verdict on one inbound message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateDecision {
    /// Forward this message: the input envelope with this relay's hop
    /// appended. The envelope bytes are unchanged, so the message ID
    /// is identical on every path.
    Forward(RelayedMessage),
    /// Do not forward, for the recorded reason.
    Refuse(GateRefusal),
}

/// The validate-before-forward gate. One gate per relay identity: it
/// owns the relay key (as secret bytes), the group-authorization
/// frontier, the dedupe set, and the hop cap. The ONLY forwarding path
/// for chained messages runs through [`RelayGate::decide`].
pub struct RelayGate<F> {
    relay_secret: [u8; 32],
    frontier: F,
    dedupe: DedupeSet,
    max_hops: usize,
}

impl<F: MembershipFrontier> RelayGate<F> {
    /// A gate for `relay_key`'s identity over `frontier`, under the
    /// default hop cap
    /// ([`DEFAULT_MAX_HOPS`](crate::chained_message::DEFAULT_MAX_HOPS)).
    pub fn new(relay_key: &SigningKey, frontier: F) -> Self {
        Self {
            relay_secret: relay_key.to_bytes(),
            frontier,
            dedupe: DedupeSet::new(),
            max_hops: crate::chained_message::DEFAULT_MAX_HOPS,
        }
    }

    /// A gate with a custom hop cap (tests and small topologies).
    pub fn with_max_hops(relay_key: &SigningKey, frontier: F, max_hops: usize) -> Self {
        Self {
            max_hops,
            ..Self::new(relay_key, frontier)
        }
    }

    /// This relay's account ID: the hex of its public key.
    pub fn relay_id(&self) -> String {
        hex_encode(
            &SigningKey::from_bytes(&self.relay_secret)
                .verifying_key()
                .to_bytes(),
        )
    }

    /// How many distinct message IDs this gate has forwarded.
    pub fn forwarded(&self) -> usize {
        self.dedupe.len()
    }

    /// Decide whether `msg` may be forwarded, appending this relay's
    /// hop on the forward path. Checks run in §4.2 order — envelope,
    /// chain, authorization, dedupe, loop/cap — and the first failure
    /// refuses before any forward.
    pub fn decide(&mut self, msg: &RelayedMessage, received_at: u64) -> GateDecision {
        if let Err(e) = msg.verify_envelope() {
            return GateDecision::Refuse(GateRefusal::InvalidEnvelope(e.to_string()));
        }
        let message_id = match msg.message_id() {
            Ok(id) => id,
            Err(e) => return GateDecision::Refuse(GateRefusal::InvalidEnvelope(e.to_string())),
        };
        if let Err(e) = check_hop_chain(&message_id, &msg.hops, self.max_hops) {
            return GateDecision::Refuse(GateRefusal::InvalidChain(e.to_string()));
        }
        if !self
            .frontier
            .is_authorized(&msg.envelope.origin, &msg.envelope.group)
        {
            return GateDecision::Refuse(GateRefusal::NotAuthorized {
                origin: msg.envelope.origin.clone(),
                group: msg.envelope.group.clone(),
            });
        }
        let relay_id = self.relay_id();
        if let Some(refusal) = relay_refusal(&msg.hops, &relay_id, self.max_hops) {
            return GateDecision::Refuse(refusal.into());
        }
        // Dedupe runs last among the refusals: only messages that
        // survive every other check occupy an ID, so `forwarded`
        // counts exactly the messages this gate forwarded and a
        // redelivery of an already-forwarded message reports
        // Duplicate, while an echo carrying our own hop reports the
        // more informative AlreadyInChain above.
        match self.dedupe.observe(&msg.envelope) {
            Ok(true) => {}
            Ok(false) => return GateDecision::Refuse(GateRefusal::Duplicate),
            Err(e) => return GateDecision::Refuse(GateRefusal::InvalidEnvelope(e.to_string())),
        }
        let mut out = msg.clone();
        let previous_hop = out.hops.last().map(|hop| hop.relay.clone());
        let key = SigningKey::from_bytes(&self.relay_secret);
        match HopRecord::signed(&key, &message_id, previous_hop.as_deref(), received_at) {
            Ok(record) => {
                out.hops.push(record);
                GateDecision::Forward(out)
            }
            Err(e) => GateDecision::Refuse(GateRefusal::InvalidChain(e.to_string())),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::chained_message::{ChainedContent, ChainedEnvelope};

    // Oracle: the #2036 design record §4.2 (validate-before-forward)
    // and task #3796 part B's acceptance list (gate slice).

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn key_id(seed: u8) -> String {
        hex_encode(&key(seed).verifying_key().to_bytes())
    }

    fn envelope(origin_seed: u8, sequence: u64) -> ChainedEnvelope {
        ChainedEnvelope::signed(
            &key(origin_seed),
            "group-1",
            sequence,
            Vec::new(),
            "durable",
            ChainedContent::Inline {
                bytes: b"hello".to_vec(),
            },
            1_000,
        )
        .unwrap()
    }

    fn member_gate(relay_seed: u8, origin_seed: u8) -> RelayGate<AllowlistFrontier> {
        let mut frontier = AllowlistFrontier::new();
        frontier.allow("group-1", &key_id(origin_seed));
        RelayGate::new(&key(relay_seed), frontier)
    }

    #[test]
    fn valid_message_forwards_with_hop_and_identical_origin_bytes() {
        // Acceptance: a valid member message is the ONLY forward path —
        // envelope bytes unchanged, hop appended, same message ID.
        let mut gate = member_gate(7, 1);
        let inbound = RelayedMessage::new(envelope(1, 0));
        let id = inbound.message_id().unwrap();
        let decision = gate.decide(&inbound, 2_000);
        let GateDecision::Forward(out) = decision else {
            panic!("valid member message must forward, got {decision:?}");
        };
        assert_eq!(out.message_id().unwrap(), id);
        assert_eq!(out.envelope, inbound.envelope);
        assert_eq!(out.hops.len(), 1);
        assert_eq!(out.hops[0].relay, gate.relay_id());
        assert_eq!(out.hops[0].previous_hop, None);
        out.verify_envelope().unwrap();
        assert_eq!(gate.forwarded(), 1);
    }

    #[test]
    fn forged_envelope_is_refused_before_any_forward() {
        // Acceptance: invalid signature never reaches the forwarder.
        let mut gate = member_gate(7, 1);
        let mut forged = envelope(1, 0);
        forged.content = ChainedContent::Inline {
            bytes: b"forged".to_vec(),
        };
        let decision = gate.decide(&RelayedMessage::new(forged), 2_000);
        assert!(
            matches!(
                decision,
                GateDecision::Refuse(GateRefusal::InvalidEnvelope(_))
            ),
            "forged envelope must be refused, got {decision:?}"
        );
        assert_eq!(gate.forwarded(), 0);
    }

    #[test]
    fn non_member_origin_is_refused_and_never_deduped_in() {
        // Acceptance + §4.2: unauthorized traffic is not forwarded; a
        // gossip topic ID is not a credential, so the empty frontier
        // authorizes nobody.
        let mut gate = RelayGate::new(&key(7), AllowlistFrontier::new());
        let decision = gate.decide(&RelayedMessage::new(envelope(1, 0)), 2_000);
        assert!(
            matches!(
                decision,
                GateDecision::Refuse(GateRefusal::NotAuthorized { .. })
            ),
            "non-member origin must be refused, got {decision:?}"
        );
        assert_eq!(gate.forwarded(), 0);
    }

    #[test]
    fn second_path_dedupes_while_forks_forward() {
        // Acceptance: alternate paths dedupe; forks stay distinguishable.
        let mut gate = member_gate(7, 1);
        let first = RelayedMessage::new(envelope(1, 0));
        assert!(matches!(
            gate.decide(&first, 2_000),
            GateDecision::Forward(_)
        ));
        let again = gate.decide(&first, 2_001);
        assert_eq!(again, GateDecision::Refuse(GateRefusal::Duplicate));
        let fork = RelayedMessage::new(envelope(1, 1));
        assert_ne!(first.message_id().unwrap(), fork.message_id().unwrap());
        assert!(matches!(
            gate.decide(&fork, 2_002),
            GateDecision::Forward(_)
        ));
        assert_eq!(gate.forwarded(), 2);
    }

    #[test]
    fn loop_and_cap_refuse_without_forwarding() {
        // Acceptance: loop/cap rules hold — the relay never signs a hop
        // it should not forward.
        let mut gate = RelayGate::with_max_hops(
            &key(7),
            {
                let mut frontier = AllowlistFrontier::new();
                frontier.allow("group-1", &key_id(1));
                frontier
            },
            2,
        );
        // Build a 2-hop chain through other relays ending at us-adjacent.
        let cap_env = envelope(1, 0);
        let cap_id = RelayedMessage::new(cap_env.clone()).message_id().unwrap();
        let first = HopRecord::signed(&key(2), &cap_id, None, 2_000).unwrap();
        let second = HopRecord::signed(&key(3), &cap_id, Some(&key_id(2)), 2_001).unwrap();
        let at_cap = RelayedMessage {
            envelope: cap_env,
            hops: vec![first, second],
        };
        assert_eq!(
            gate.decide(&at_cap, 2_002),
            GateDecision::Refuse(GateRefusal::CapReached)
        );
        // A chain already containing this relay loops back through it
        // (a distinct envelope, so dedupe does not mask the loop).
        let loop_env = envelope(1, 1);
        let loop_id = RelayedMessage::new(loop_env.clone()).message_id().unwrap();
        let looped = RelayedMessage {
            envelope: loop_env,
            hops: vec![HopRecord::signed(&key(7), &loop_id, None, 2_000).unwrap()],
        };
        assert_eq!(
            gate.decide(&looped, 2_003),
            GateDecision::Refuse(GateRefusal::AlreadyInChain)
        );
        assert_eq!(gate.forwarded(), 0);
    }

    #[test]
    fn corrupt_chain_refuses_while_envelope_stays_valid() {
        // §4.1 carried into the gate: a stripped/corrupt chain fails
        // the chain check, never the envelope — and is not forwarded.
        let mut gate = member_gate(7, 1);
        let mut relayed = RelayedMessage::new(envelope(1, 0));
        relayed
            .hops
            .push(HopRecord::signed(&key(2), &relayed.message_id().unwrap(), None, 2_000).unwrap());
        relayed.hops[0].signature = "00".repeat(64);
        relayed.verify_envelope().unwrap();
        let decision = gate.decide(&relayed, 2_001);
        assert!(
            matches!(decision, GateDecision::Refuse(GateRefusal::InvalidChain(_))),
            "corrupt chain must be refused, got {decision:?}"
        );
        assert_eq!(gate.forwarded(), 0);
    }

    #[test]
    fn bridge_forwards_across_two_relays_with_identical_envelope() {
        // Multi-node bridge topology (task #3796 part B): origin ->
        // relay A -> relay B -> leaf. Each relay runs its own gate;
        // the envelope and its message ID are identical at every hop,
        // and the hop chain records the path in order.
        let origin = envelope(1, 0);
        let origin_id = origin.message_id().unwrap();
        let mut gate_a = member_gate(2, 1);
        let mut gate_b = member_gate(3, 1);

        let after_a = match gate_a.decide(&RelayedMessage::new(origin.clone()), 2_000) {
            GateDecision::Forward(msg) => msg,
            other => panic!("bridge relay A must forward, got {other:?}"),
        };
        assert_eq!(after_a.envelope, origin);
        assert_eq!(after_a.hops.len(), 1);
        assert_eq!(after_a.hops[0].relay, key_id(2));

        let after_b = match gate_b.decide(&after_a, 3_000) {
            GateDecision::Forward(msg) => msg,
            other => panic!("bridge relay B must forward, got {other:?}"),
        };
        assert_eq!(after_b.envelope, origin);
        assert_eq!(after_b.message_id().unwrap(), origin_id);
        assert_eq!(after_b.hops.len(), 2);
        assert_eq!(after_b.hops[0].relay, key_id(2));
        assert_eq!(after_b.hops[1].relay, key_id(3));
        assert_eq!(
            after_b.hops[1].previous_hop.as_deref(),
            Some(key_id(2).as_str())
        );
        assert_eq!(gate_a.forwarded(), 1);
        assert_eq!(gate_b.forwarded(), 1);
    }

    #[test]
    fn bridge_stops_when_the_bridge_relay_does_not_authorize_the_origin() {
        // The bridge node's own frontier governs: an origin the
        // bridge does not authorize is refused at the bridge, so the
        // message never reaches the leaf side of the topology.
        let origin = envelope(1, 0);
        let mut gate_a = member_gate(2, 1);
        let mut gate_b = RelayGate::new(&key(3), AllowlistFrontier::new());

        let after_a = match gate_a.decide(&RelayedMessage::new(origin), 2_000) {
            GateDecision::Forward(msg) => msg,
            other => panic!("bridge relay A must forward, got {other:?}"),
        };
        match gate_b.decide(&after_a, 3_000) {
            GateDecision::Refuse(GateRefusal::NotAuthorized { .. }) => {}
            other => panic!("bridge relay B must refuse as NotAuthorized, got {other:?}"),
        }
        assert_eq!(gate_b.forwarded(), 0);
        assert_eq!(after_a.hops.len(), 1);
    }
}
