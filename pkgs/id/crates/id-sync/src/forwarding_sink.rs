//! Production fan-out for chained-relay deliveries (#5534, phase 3 of
//! the #2036 relay protocol).
//!
//! The [`ChainedSink`] contract has two halves: deliver the forwarded
//! message locally, then fan it out to next peers with
//! [`forward_message`]. The ingest slice wired the gate in front of an
//! arbitrary sink and the five-node `PoC` (`tests/relay_poc.rs`, #2249)
//! proved the second half works — but only with a sink written inside
//! the test. This module is that sink as production code, so a host
//! builder can attach automatic fan-out without copying the `PoC`.
//!
//! [`ForwardingSink`] hands each forwarded message to its [`LocalSink`],
//! then spawns one [`forward_message`] per visible neighbour, excluding
//! this relay itself and the neighbour the message arrived from. The
//! arrival neighbour is read off the hop chain: the previous hop of this
//! relay's own (last) hop, or the origin for a direct delivery. Both
//! exclusions are what stops an echo straight back and a self-delivery;
//! cycles further round the graph are cut by the gate's duplicate
//! suppression, because the fan-out reuses the message ID unchanged.
//!
//! Delivery must not block the accepting connection, so forwarding runs
//! on spawned tasks; [`deliver`](ChainedSink::deliver) returns after the
//! local delivery and the task launches. A failed forward is logged and
//! dropped: chained relay is best-effort at each live hop, and the
//! origin's outbox owns retries.

use std::sync::Arc;

use iroh::{Endpoint, EndpointAddr};

use crate::chained_message::RelayedMessage;
use crate::relay_ingest::{ChainedSink, forward_message};

/// Where a [`ForwardingSink`] hands each message it delivers locally.
///
/// Kept separate from the fan-out so the local half stays a plain,
/// synchronous hand-off (an event channel, a recorder, or
/// [`DropLocal`]); the trait deliberately does not allow blocking work.
pub trait LocalSink: Send + Sync {
    /// The relay gate forwarded `forwarded`; take it as a local delivery.
    fn deliver(&self, forwarded: &RelayedMessage);
}

/// A [`LocalSink`] that keeps nothing.
///
/// For relays whose local delivery is observed elsewhere, or not at all
/// in this process; the fan-out half still runs.
#[derive(Debug, Clone, Copy, Default)]
pub struct DropLocal;

impl LocalSink for DropLocal {
    fn deliver(&self, _forwarded: &RelayedMessage) {}
}

/// The peers a relay forwards to: its view of the visibility graph.
///
/// Read on every delivery, so an implementation backed by live
/// membership state can change the fan-out without rebuilding the sink.
pub trait NextPeers: Send + Sync {
    /// Relay IDs, each with the dialable address that reaches it.
    fn next_peers(&self) -> Vec<(String, EndpointAddr)>;
}

/// A neighbour set fixed when the relay starts.
///
/// The simple backing for [`NextPeers`]: the hosts that know their
/// neighbours at start-up pass the vector straight to
/// [`ForwardingSink::new`].
impl NextPeers for Vec<(String, EndpointAddr)> {
    fn next_peers(&self) -> Vec<(String, EndpointAddr)> {
        self.clone()
    }
}

/// The production [`ChainedSink`]: deliver locally, then fan out.
///
/// Build one per relay with the relay's own ID (the hex of its Ed25519
/// key, as it appears in a [`HopRecord`](crate::chained_message::HopRecord)),
/// its Iroh [`Endpoint`], the neighbours to forward to, and where local
/// deliveries go. Wrap it in
/// [`ChainedRelayProtocol::new`](crate::relay_ingest::ChainedRelayProtocol::new)
/// behind a [`RelayGate`](crate::relay_gate::RelayGate) to serve the
/// chained-relay ALPN.
///
/// Needs a Tokio runtime when it delivers: the fan-out tasks are spawned
/// with [`tokio::spawn`], which the gate's accepting connection already
/// provides in a served relay.
#[derive(Clone)]
pub struct ForwardingSink {
    self_id: String,
    endpoint: Endpoint,
    peers: Arc<dyn NextPeers>,
    local: Arc<dyn LocalSink>,
}

impl ForwardingSink {
    /// A sink that delivers locally to `local` and fans out to `peers`.
    pub fn new(
        self_id: impl Into<String>,
        endpoint: Endpoint,
        peers: impl NextPeers + 'static,
        local: impl LocalSink + 'static,
    ) -> Self {
        Self {
            self_id: self_id.into(),
            endpoint,
            peers: Arc::new(peers),
            local: Arc::new(local),
        }
    }

    /// A sink that fans out and keeps no local copy ([`DropLocal`]).
    pub fn fanning_out(
        self_id: impl Into<String>,
        endpoint: Endpoint,
        peers: impl NextPeers + 'static,
    ) -> Self {
        Self::new(self_id, endpoint, peers, DropLocal)
    }

    /// The neighbours this delivery goes on to: every configured peer
    /// except this relay and the neighbour the message arrived from.
    fn next_targets(&self, forwarded: &RelayedMessage) -> Vec<(String, EndpointAddr)> {
        // This relay's own hop is last; its previous hop is the arrival
        // neighbour. A direct delivery (no hops, or this relay's first
        // hop) arrived from the origin.
        let arrival = forwarded
            .hops
            .last()
            .and_then(|hop| hop.previous_hop.clone())
            .unwrap_or_else(|| forwarded.envelope.origin.clone());
        self.peers
            .next_peers()
            .into_iter()
            .filter(|(relay, _)| relay != &arrival && relay != &self.self_id)
            .collect()
    }
}

impl std::fmt::Debug for ForwardingSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForwardingSink")
            .field("self_id", &self.self_id)
            .finish_non_exhaustive()
    }
}

impl ChainedSink for ForwardingSink {
    fn deliver(&self, forwarded: &RelayedMessage) {
        self.local.deliver(forwarded);
        for (relay, addr) in self.next_targets(forwarded) {
            let endpoint = self.endpoint.clone();
            let msg = forwarded.clone();
            tokio::spawn(async move {
                match forward_message(&endpoint, addr, &msg).await {
                    crate::envelope_outbox::Outcome::Delivered => {}
                    other => tracing::warn!(
                        relay = %relay,
                        outcome = ?other,
                        "chained relay fan-out did not deliver"
                    ),
                }
            });
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::chained_message::{ChainedContent, ChainedEnvelope, HopRecord};
    use crate::hex::hex_encode;
    use ed25519_dalek::SigningKey;
    use iroh::endpoint::{RelayMode, presets};

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn key_id(seed: u8) -> String {
        hex_encode(&key(seed).verifying_key().to_bytes())
    }

    async fn endpoint() -> Endpoint {
        Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap()
    }

    fn addr_of(ep: &Endpoint) -> EndpointAddr {
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

    fn envelope() -> ChainedEnvelope {
        ChainedEnvelope::signed(
            &key(1),
            "group-1",
            0,
            Vec::new(),
            "durable",
            ChainedContent::Inline {
                bytes: b"hello".to_vec(),
            },
            1_000,
        )
        .unwrap()
    }

    fn hop(relay_seed: u8, previous: Option<u8>) -> HopRecord {
        HopRecord {
            relay: key_id(relay_seed),
            previous_hop: previous.map(key_id),
            received_at: 1_000,
            signature: String::new(),
        }
    }

    /// Oracle: the fan-out set is every peer minus the arrival neighbour
    /// and the relay itself (the #2036 §4.1 "in the same manner" rule and
    /// the #2249 `PoC`'s exclusion, stated as a pure function so the
    /// exclusion needs no network timing).
    #[tokio::test]
    async fn next_targets_excludes_arrival_and_self() {
        let ep = endpoint().await;
        let peers = vec![
            (key_id(7), addr_of(&ep)), // this relay
            (key_id(1), addr_of(&ep)), // origin / arrival
            (key_id(2), addr_of(&ep)), // next peer
            (key_id(3), addr_of(&ep)), // next peer
        ];
        let sink = ForwardingSink::fanning_out(key_id(7), ep, peers);

        // A direct delivery arrived from the origin.
        let direct = RelayedMessage {
            envelope: envelope(),
            hops: vec![hop(7, None)],
        };
        let targets = sink.next_targets(&direct);
        assert_eq!(
            targets.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>(),
            vec![key_id(2), key_id(3)]
        );

        // A relayed delivery arrived from seed 3, which is also a
        // configured peer: it must not be echoed back to.
        let relayed = RelayedMessage {
            envelope: envelope(),
            hops: vec![hop(3, None), hop(7, Some(3))],
        };
        let targets = sink.next_targets(&relayed);
        assert_eq!(
            targets.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>(),
            vec![key_id(1), key_id(2)]
        );
    }
}
