//! Live call-site wiring for the relay gate: phase 3 of the #2036
//! messaging/relay protocol v1 (#3796), the ingest slice.
//!
//! The gate in [`crate::relay_gate`] is the only forwarding path for
//! chained messages, and its gate-verification finding stands: the
//! pinned `iroh-gossip` forwards eagerly before the application can
//! accept or reject, so stock gossip broadcast must not carry chained
//! messages. They arrive instead over direct Iroh connections on
//! [`CHAINED_RELAY_ALPN`], and this module is the receiving end.
//!
//! Every frame on that ALPN is decoded strictly (duplicate JSON keys
//! are refused, like the envelope path), then run through
//! [`RelayGate::decide`] BEFORE anything else happens to it. Only a
//! [`GateDecision::Forward`] reaches the host's [`ChainedSink`]; the
//! sink receives the forwarded form — origin envelope byte-identical,
//! this relay's hop appended — delivers it locally, and fans it out
//! to next peers with [`forward_message`]. Refusals map onto the
//! envelope transport's [`Reply`] vocabulary: a duplicate arrival
//! answers [`Reply::Duplicate`], every other refusal answers
//! [`Reply::Refused`] with the gate's reason, so a sender learns a
//! forged or unauthorized message will never succeed on retry.
//!
//! The frontier backing has since landed:
//! [`GraphFrontier`](crate::graph_frontier::GraphFrontier) answers
//! [`MembershipFrontier`](crate::relay_gate::MembershipFrontier) from
//! the signed graph's resolved membership state, so a production host
//! wires the gate with the graph frontier over its shared event
//! store; the ingest acceptance tests below run against both
//! backings.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use iroh::{
    Endpoint, EndpointAddr,
    endpoint::Connection,
    protocol::{AcceptError, ProtocolHandler},
};

use crate::chained_message::RelayedMessage;
use crate::envelope_net::{Reply, answer_frames, send};
use crate::envelope_outbox::Outcome;
use crate::relay_gate::{GateDecision, GateRefusal, MembershipFrontier, RelayGate};

/// ALPN for chained-message relay. Within version 1 fields may be
/// added; removing or changing one needs `/id-chained-relay/2`.
pub const CHAINED_RELAY_ALPN: &[u8] = b"/id-chained-relay/1";

/// What a host does with a chained message its relay gate forwarded.
///
/// The sink receives the forwarded form of the message: the origin
/// envelope is byte-identical to what arrived (so the message ID is
/// the same on every path) and this relay's hop has been appended.
/// Deliver it locally, then fan the same value out to next peers with
/// [`forward_message`]. Implementations must not block: hand the
/// message to a channel or queue and return.
pub trait ChainedSink: Send + Sync {
    /// The gate forwarded `forwarded`; deliver and fan it out.
    fn deliver(&self, forwarded: &RelayedMessage);
}

/// Sends `msg` to the node at `target` over the chained-relay ALPN,
/// and says whether it is done. Mirrors
/// [`deliver`](crate::envelope_net::deliver) for friend envelopes.
pub async fn forward_message(
    endpoint: &Endpoint,
    target: impl Into<EndpointAddr>,
    msg: &RelayedMessage,
) -> Outcome {
    let json = match serde_json::to_vec(msg) {
        Ok(json) => json,
        Err(error) => {
            return Outcome::Refused(format!("chained message does not encode: {error}"));
        }
    };
    send(endpoint, CHAINED_RELAY_ALPN, target.into(), &json).await
}

/// Accepts chained messages over Iroh and runs each one through a
/// [`RelayGate`] before it can be delivered or forwarded.
///
/// Register with `Router::builder(..).accept(CHAINED_RELAY_ALPN,
/// ChainedRelayProtocol::new(gate, sink))`.
#[derive(Clone)]
pub struct ChainedRelayProtocol<F, S> {
    gate: Arc<Mutex<RelayGate<F>>>,
    sink: S,
}

impl<F, S> ChainedRelayProtocol<F, S> {
    /// Serve chained messages through `gate`, delivering forwards to
    /// `sink`.
    #[must_use]
    pub fn new(gate: RelayGate<F>, sink: S) -> Self {
        Self {
            gate: Arc::new(Mutex::new(gate)),
            sink,
        }
    }

    fn lock_gate(&self) -> std::sync::MutexGuard<'_, RelayGate<F>> {
        self.gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The gate holds the relay's secret key bytes, so `Debug` names the
/// type and shows nothing else.
impl<F, S> std::fmt::Debug for ChainedRelayProtocol<F, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChainedRelayProtocol")
            .finish_non_exhaustive()
    }
}

impl<F, S> ProtocolHandler for ChainedRelayProtocol<F, S>
where
    F: MembershipFrontier + Send + 'static,
    S: ChainedSink + Clone + Send + Sync + 'static,
{
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let gate = Arc::clone(&self.gate);
        let sink = self.sink.clone();
        answer_frames(&conn, move |body| {
            let gate = Arc::clone(&gate);
            let sink = sink.clone();
            async move { receive(&gate, &sink, &body) }
        })
        .await
    }
}

/// One inbound frame: decode strictly, decide through the gate,
/// deliver only on a forward. Never panics on malformed input —
/// every failure is a [`Reply::Refused`] (or [`Reply::Duplicate`])
/// the sender can act on.
fn receive<F: MembershipFrontier, S: ChainedSink>(
    gate: &Mutex<RelayGate<F>>,
    sink: &S,
    body: &[u8],
) -> Reply {
    let msg: RelayedMessage = match crate::checked_json::from_slice_unique(body) {
        Ok(msg) => msg,
        Err(error) => {
            return Reply::Refused {
                message: format!("not a chained message: {error:#}"),
            };
        }
    };
    let received_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let decision = {
        let mut gate = gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        gate.decide(&msg, received_at)
    };
    match decision {
        GateDecision::Forward(forwarded) => {
            sink.deliver(&forwarded);
            Reply::Applied
        }
        GateDecision::Refuse(GateRefusal::Duplicate) => Reply::Duplicate,
        GateDecision::Refuse(refusal) => Reply::Refused {
            message: refusal.to_string(),
        },
    }
}

impl<F: MembershipFrontier, S: ChainedSink> ChainedRelayProtocol<F, S> {
    /// How many distinct message IDs this relay has forwarded.
    pub fn forwarded(&self) -> usize {
        self.lock_gate().forwarded()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::{Arc, Mutex};

    use ed25519_dalek::SigningKey;
    use iroh::{
        EndpointAddr,
        endpoint::{RelayMode, presets},
        protocol::Router,
    };

    use super::*;
    use crate::chained_message::{ChainedContent, ChainedEnvelope};
    use crate::hex::hex_encode;
    use crate::relay_gate::AllowlistFrontier;

    use id_core::membership::AuthId;
    use id_core::waiting::{AuthorKey, WaitingLimits};
    use id_events::event_store::EventStore;
    use id_store::chain::{ChainExt, Event, sign};
    use id_store::store_traits::SharedEventStore;
    use p2panda_auth::Access;
    use p2panda_auth::group::{GroupAction, GroupMember};
    use p2panda_core::Hash;

    use crate::graph_frontier::GraphFrontier;

    // Oracle: issue #3796's acceptance (two members with no direct
    // connection exchange chained messages via a bridging member;
    // forged/altered relayed messages are rejected; duplicates are
    // suppressed) exercised at the live ingest call site this module
    // adds, over real Iroh connections — the gate-level half of the
    // same acceptance is covered in `relay_gate`'s tests. The bridge,
    // removed-member and duplicate scenarios run twice: once with the
    // allowlist stand-in and once with the graph-backed frontier a
    // production host wires in.

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

    #[derive(Clone, Default)]
    struct CollectSink(Arc<Mutex<Vec<RelayedMessage>>>);

    impl CollectSink {
        fn received(&self) -> Vec<RelayedMessage> {
            self.0.lock().unwrap().clone()
        }
    }

    impl ChainedSink for CollectSink {
        fn deliver(&self, forwarded: &RelayedMessage) {
            self.0.lock().unwrap().push(forwarded.clone());
        }
    }

    async fn endpoint() -> Endpoint {
        Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap()
    }

    fn loopback_addr(ep: &Endpoint) -> EndpointAddr {
        EndpointAddr::from_parts(
            ep.id(),
            ep.bound_sockets().iter().map(|addr| {
                let ip = if addr.ip().is_unspecified() {
                    std::net::Ipv4Addr::LOCALHOST.into()
                } else {
                    addr.ip()
                };
                iroh::TransportAddr::Ip(std::net::SocketAddr::new(ip, addr.port()))
            }),
        )
    }

    fn member_protocol(
        relay_seed: u8,
        origin_seed: u8,
        sink: CollectSink,
    ) -> ChainedRelayProtocol<AllowlistFrontier, CollectSink> {
        let mut frontier = AllowlistFrontier::new();
        frontier.allow("group-1", &key_id(origin_seed));
        ChainedRelayProtocol::new(RelayGate::new(&key(relay_seed), frontier), sink)
    }

    #[tokio::test]
    async fn bridge_then_leaf_exchange_over_live_connections() {
        // Acceptance: origin -> bridge -> leaf over real connections.
        // The leaf receives the origin's envelope byte-identical (same
        // message ID) with both relays' hops appended in order.
        let origin = endpoint().await;
        let bridge = endpoint().await;
        let leaf = endpoint().await;

        let bridge_sink = CollectSink::default();
        let bridge_protocol = member_protocol(7, 1, bridge_sink.clone());
        let leaf_sink = CollectSink::default();
        let leaf_protocol = member_protocol(8, 1, leaf_sink.clone());
        let _bridge_router = Router::builder(bridge.clone())
            .accept(CHAINED_RELAY_ALPN, bridge_protocol.clone())
            .spawn();
        let _leaf_router = Router::builder(leaf.clone())
            .accept(CHAINED_RELAY_ALPN, leaf_protocol.clone())
            .spawn();

        let msg = RelayedMessage::new(envelope(1, 0));
        let id = msg.message_id().unwrap();

        let outcome = forward_message(&origin, loopback_addr(&bridge), &msg).await;
        assert!(
            matches!(outcome, Outcome::Delivered),
            "bridge must accept a valid member message, got {outcome:?}"
        );
        let bridged = bridge_sink.received();
        assert_eq!(bridged.len(), 1);
        assert_eq!(bridged[0].message_id().unwrap(), id);
        assert_eq!(bridged[0].envelope, msg.envelope);
        assert_eq!(bridged[0].hops.len(), 1);
        assert_eq!(bridged[0].hops[0].relay, key_id(7));
        assert_eq!(bridge_protocol.forwarded(), 1);

        let outcome = forward_message(&bridge, loopback_addr(&leaf), &bridged[0]).await;
        assert!(
            matches!(outcome, Outcome::Delivered),
            "leaf must accept the bridged message, got {outcome:?}"
        );
        let delivered = leaf_sink.received();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].message_id().unwrap(), id);
        assert_eq!(delivered[0].envelope, msg.envelope);
        assert_eq!(delivered[0].hops.len(), 2);
        assert_eq!(delivered[0].hops[0].relay, key_id(7));
        assert_eq!(delivered[0].hops[1].relay, key_id(8));
        assert_eq!(
            delivered[0].hops[1].previous_hop.as_deref(),
            Some(key_id(7).as_str())
        );
        delivered[0].verify_envelope().unwrap();
        delivered[0].check_hops().unwrap();
    }

    #[tokio::test]
    async fn forged_message_is_refused_at_ingest_and_never_delivered() {
        // Acceptance: a forged/altered relayed message is rejected at
        // the ingest call site — refused reply, nothing delivered,
        // nothing forwarded.
        let origin = endpoint().await;
        let bridge = endpoint().await;
        let bridge_sink = CollectSink::default();
        let bridge_protocol = member_protocol(7, 1, bridge_sink.clone());
        let _bridge_router = Router::builder(bridge.clone())
            .accept(CHAINED_RELAY_ALPN, bridge_protocol.clone())
            .spawn();

        let mut forged = envelope(1, 0);
        forged.content = ChainedContent::Inline {
            bytes: b"forged".to_vec(),
        };
        let outcome = forward_message(
            &origin,
            loopback_addr(&bridge),
            &RelayedMessage::new(forged),
        )
        .await;
        assert!(
            matches!(outcome, Outcome::Refused(_)),
            "forged message must be refused at ingest, got {outcome:?}"
        );
        assert!(bridge_sink.received().is_empty());
        assert_eq!(bridge_protocol.forwarded(), 0);
    }

    #[tokio::test]
    async fn non_member_origin_is_refused_at_ingest() {
        // Acceptance: the safe default — a relay only carries traffic
        // for groups whose membership it can authorize. An origin the
        // frontier does not list is refused before any forward.
        let origin = endpoint().await;
        let bridge = endpoint().await;
        let bridge_sink = CollectSink::default();
        let protocol = ChainedRelayProtocol::new(
            RelayGate::new(&key(7), AllowlistFrontier::new()),
            bridge_sink.clone(),
        );
        let _bridge_router = Router::builder(bridge.clone())
            .accept(CHAINED_RELAY_ALPN, protocol.clone())
            .spawn();

        let msg = RelayedMessage::new(envelope(1, 0));
        let outcome = forward_message(&origin, loopback_addr(&bridge), &msg).await;
        match outcome {
            Outcome::Refused(message) => {
                assert!(
                    message.contains("may not write"),
                    "refusal must name the authorization failure, got: {message}"
                );
            }
            other => panic!("non-member origin must be refused at ingest, got {other:?}"),
        }
        assert!(bridge_sink.received().is_empty());
        assert_eq!(protocol.forwarded(), 0);
    }

    #[tokio::test]
    async fn duplicate_arrival_is_suppressed_at_ingest() {
        // Acceptance: duplicates are suppressed — the second arrival
        // of the same message ID delivers nothing new (the transport
        // maps the gate's Duplicate onto a delivered outcome, since
        // the message HAS been delivered exactly once).
        let origin = endpoint().await;
        let bridge = endpoint().await;
        let bridge_sink = CollectSink::default();
        let bridge_protocol = member_protocol(7, 1, bridge_sink.clone());
        let _bridge_router = Router::builder(bridge.clone())
            .accept(CHAINED_RELAY_ALPN, bridge_protocol.clone())
            .spawn();

        let msg = RelayedMessage::new(envelope(1, 0));
        for attempt in 0..2 {
            let outcome = forward_message(&origin, loopback_addr(&bridge), &msg).await;
            assert!(
                matches!(outcome, Outcome::Delivered),
                "attempt {attempt} must report delivered, got {outcome:?}"
            );
        }
        assert_eq!(bridge_sink.received().len(), 1);
        assert_eq!(bridge_protocol.forwarded(), 1);
    }

    // --- Graph-backed frontier harness ---------------------------------
    //
    // The same acceptance with the production frontier: each relay's
    // gate reads authorization from a real event store holding the
    // group's signed membership operations, replicated to both relays
    // the way gossip would carry them.

    /// p2panda key: group operations (the chain signer). The same
    /// seed gives the same author bytes as [`key`].
    fn pkey(seed: u8) -> p2panda_core::SigningKey {
        p2panda_core::SigningKey::from_bytes(&[seed; 32])
    }

    fn individual(key: &p2panda_core::SigningKey) -> GroupMember<AuthId> {
        let author: AuthorKey = *key.verifying_key().as_bytes();
        GroupMember::Individual(AuthId::Key(author))
    }

    /// A signed group operation and its encoded body, built the way
    /// the event store's own tests build them (public constructors).
    fn group_op(
        key: &p2panda_core::SigningKey,
        seq: p2panda_core::SeqNum,
        backlink: Option<Hash>,
        after: Vec<Hash>,
        group: Option<Hash>,
        action: &GroupAction<AuthId, ()>,
    ) -> (Event, Vec<u8>) {
        let body = id_core::membership::body::encode_body(action).unwrap();
        let ext = ChainExt {
            kind: "group.op".to_owned(),
            group,
            after,
            tag: None,
            document: None,
            removes: None,
            note: None,
            link: None,
            checkpoint: None,
            rotation: None,
            chat: None,
            anchor: None,
        };
        let event = sign(key, seq, backlink, Some(&body), ext).unwrap();
        (event, body)
    }

    /// A fresh event store with `ops` ingested in order, wrapped for
    /// sharing with a frontier.
    fn graph_store(ops: &[(Event, Vec<u8>)]) -> (tempfile::TempDir, SharedEventStore<EventStore>) {
        let dir = tempfile::tempdir().unwrap();
        let mut store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
        for (event, body) in ops {
            store
                .ingest_with_body(&event.bytes, Some(body.as_slice()), 0)
                .unwrap();
            assert!(store.event_applied(&event.id).unwrap());
        }
        (dir, SharedEventStore::new(store))
    }

    /// The group both relays serve: Alice (seed 1) creates it with
    /// herself as manager and Bob (seed 2) as a reader. The group ID
    /// is the create event's ID; envelopes name it by its hex text.
    fn graph_group() -> ((Event, Vec<u8>), Hash) {
        let (alice, bob) = (pkey(1), pkey(2));
        let create = GroupAction::Create {
            initial_members: vec![
                (individual(&alice), Access::manage()),
                (individual(&bob), Access::read()),
            ],
        };
        let op = group_op(&alice, 0, None, vec![], None, &create);
        let group = op.0.id;
        (op, group)
    }

    fn graph_protocol(
        relay_seed: u8,
        store: SharedEventStore<EventStore>,
        sink: CollectSink,
    ) -> ChainedRelayProtocol<GraphFrontier, CollectSink> {
        ChainedRelayProtocol::new(
            RelayGate::new(&key(relay_seed), GraphFrontier::new(store)),
            sink,
        )
    }

    fn graph_envelope(origin_seed: u8, group: &Hash, sequence: u64) -> ChainedEnvelope {
        ChainedEnvelope::signed(
            &key(origin_seed),
            &group.to_string(),
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

    #[tokio::test]
    async fn bridge_then_leaf_exchange_with_graph_frontier() {
        // Acceptance, production backing: origin -> bridge -> leaf
        // over real connections, with both relays' gates reading
        // authorization from the signed graph's resolved roster
        // instead of a hand-kept list. The leaf receives the origin's
        // envelope byte-identical (same message ID) with both relays'
        // hops appended in order.
        let (create_op, group) = graph_group();
        let (_bridge_dir, bridge_store) = graph_store(std::slice::from_ref(&create_op));
        let (_leaf_dir, leaf_store) = graph_store(std::slice::from_ref(&create_op));

        let origin = endpoint().await;
        let bridge = endpoint().await;
        let leaf = endpoint().await;

        let bridge_sink = CollectSink::default();
        let bridge_protocol = graph_protocol(7, bridge_store, bridge_sink.clone());
        let leaf_sink = CollectSink::default();
        let leaf_protocol = graph_protocol(8, leaf_store, leaf_sink.clone());
        let _bridge_router = Router::builder(bridge.clone())
            .accept(CHAINED_RELAY_ALPN, bridge_protocol.clone())
            .spawn();
        let _leaf_router = Router::builder(leaf.clone())
            .accept(CHAINED_RELAY_ALPN, leaf_protocol.clone())
            .spawn();

        let msg = RelayedMessage::new(graph_envelope(1, &group, 0));
        let id = msg.message_id().unwrap();

        let outcome = forward_message(&origin, loopback_addr(&bridge), &msg).await;
        assert!(
            matches!(outcome, Outcome::Delivered),
            "bridge must accept a roster member's message, got {outcome:?}"
        );
        let bridged = bridge_sink.received();
        assert_eq!(bridged.len(), 1);
        assert_eq!(bridged[0].message_id().unwrap(), id);
        assert_eq!(bridged[0].envelope, msg.envelope);
        assert_eq!(bridged[0].hops.len(), 1);
        assert_eq!(bridged[0].hops[0].relay, key_id(7));
        assert_eq!(bridge_protocol.forwarded(), 1);

        let outcome = forward_message(&bridge, loopback_addr(&leaf), &bridged[0]).await;
        assert!(
            matches!(outcome, Outcome::Delivered),
            "leaf must accept the bridged message, got {outcome:?}"
        );
        let delivered = leaf_sink.received();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].message_id().unwrap(), id);
        assert_eq!(delivered[0].envelope, msg.envelope);
        assert_eq!(delivered[0].hops.len(), 2);
        assert_eq!(delivered[0].hops[0].relay, key_id(7));
        assert_eq!(delivered[0].hops[1].relay, key_id(8));
        assert_eq!(
            delivered[0].hops[1].previous_hop.as_deref(),
            Some(key_id(7).as_str())
        );
        delivered[0].verify_envelope().unwrap();
        delivered[0].check_hops().unwrap();
    }

    #[tokio::test]
    async fn removed_member_origin_is_refused_at_ingest_with_graph_frontier() {
        // Acceptance, production backing: a signed removal lands in
        // the relay's store and the removed member's traffic is
        // refused at ingest from then on, while a current member's
        // message still forwards. The allowlist stand-in can only
        // model the end state by hand; here the state is the fold of
        // the signed operations themselves.
        let (create_op, group) = graph_group();
        let alice = pkey(1);
        let remove_bob = GroupAction::Remove {
            member: individual(&pkey(2)),
        };
        let remove_op = group_op(
            &alice,
            1,
            Some(group),
            vec![group],
            Some(group),
            &remove_bob,
        );
        let ops = [create_op, remove_op];
        let (_dir, store) = graph_store(&ops);

        let origin = endpoint().await;
        let bridge = endpoint().await;
        let bridge_sink = CollectSink::default();
        let bridge_protocol = graph_protocol(7, store, bridge_sink.clone());
        let _bridge_router = Router::builder(bridge.clone())
            .accept(CHAINED_RELAY_ALPN, bridge_protocol.clone())
            .spawn();

        let removed = RelayedMessage::new(graph_envelope(2, &group, 0));
        let outcome = forward_message(&origin, loopback_addr(&bridge), &removed).await;
        match outcome {
            Outcome::Refused(message) => {
                assert!(
                    message.contains("may not write"),
                    "refusal must name the authorization failure, got: {message}"
                );
            }
            other => panic!("removed member must be refused at ingest, got {other:?}"),
        }
        assert!(bridge_sink.received().is_empty());
        assert_eq!(bridge_protocol.forwarded(), 0);

        let member = RelayedMessage::new(graph_envelope(1, &group, 0));
        let outcome = forward_message(&origin, loopback_addr(&bridge), &member).await;
        assert!(
            matches!(outcome, Outcome::Delivered),
            "a current member must still forward, got {outcome:?}"
        );
        assert_eq!(bridge_sink.received().len(), 1);
        assert_eq!(bridge_protocol.forwarded(), 1);
    }

    #[tokio::test]
    async fn duplicate_arrival_is_suppressed_at_ingest_with_graph_frontier() {
        // Acceptance, production backing: duplicates are suppressed
        // with the graph frontier in the loop exactly as with the
        // stand-in — the second arrival of the same message ID
        // delivers nothing new.
        let (create_op, group) = graph_group();
        let (_dir, store) = graph_store(std::slice::from_ref(&create_op));

        let origin = endpoint().await;
        let bridge = endpoint().await;
        let bridge_sink = CollectSink::default();
        let bridge_protocol = graph_protocol(7, store, bridge_sink.clone());
        let _bridge_router = Router::builder(bridge.clone())
            .accept(CHAINED_RELAY_ALPN, bridge_protocol.clone())
            .spawn();

        let msg = RelayedMessage::new(graph_envelope(1, &group, 0));
        for attempt in 0..2 {
            let outcome = forward_message(&origin, loopback_addr(&bridge), &msg).await;
            assert!(
                matches!(outcome, Outcome::Delivered),
                "attempt {attempt} must report delivered, got {outcome:?}"
            );
        }
        assert_eq!(bridge_sink.received().len(), 1);
        assert_eq!(bridge_protocol.forwarded(), 1);
    }
}
