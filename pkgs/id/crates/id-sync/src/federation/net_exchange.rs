//! B2(c) for #12232: the B09 exchange, carried over the existing
//! transport between two processes.
//!
//! [`super::slices::b09_anti_entropy::reconcile`] runs two replicas in
//! one process: it compares inventories, computes each side's offer
//! with [`Replica::offer`], and moves the missing signed versions
//! through [`Replica::receive`]. This module runs the same exchange
//! with the second replica in another process, over the crate's iroh
//! transport — the same framing ([`id_core::frames`]) and connect
//! path ([`id_core::connectivity::connect`]) as
//! [`crate::envelope_net`]. Nothing here admits, merges or invents a
//! version: the wire carries inventories, offers and the original
//! signed envelopes, and every version that arrives is admitted by
//! the receiving replica's own B07 store and B32 gate, exactly as in
//! process.
//!
//! The exchange is initiator-driven, in the order B09 prescribes:
//!
//! 1. **Summaries first.** The initiator sends its inventory summary;
//!    the responder answers with its own. Digests are what replicas
//!    exchange first ([`Inventory::summary`]); if no object diverged,
//!    the exchange is already quiet and nothing else is sent.
//! 2. **Sets for the exchange.** Both sides send their full version
//!    sets and refused sets. The responder computes its offer toward
//!    the initiator from them and holds it; the initiator computes
//!    the divergence and its own offer the same way. (The in-process
//!    exchange holds both full inventories too; the digest step is
//!    the early exit, not a different algorithm.)
//! 3. **Delivery.** The initiator sends the versions its offer names;
//!    the responder admits each through `receive` and reports the
//!    outcome of each.
//! 4. **Fetch.** The initiator asks for the responder's held offer;
//!    the responder sends those versions and the initiator admits
//!    each through its own `receive`.
//!
//! Both offers are computed from the inventories as they stood before
//! anything moved, as in [`reconcile`]. The initiator supplies the
//! logical time; both sides admit at that time.
//!
//! Outcomes cross the wire as [`NetOutcome`]: the store's own
//! outcomes keep their identity (a report id survives the trip), and
//! a refusal keeps its reason as text, because the gate's error type
//! is richer than the wire needs and is not reconstructed from it.
//!
//! Rollback is not starting the exchange: replicas keep their
//! versions, reports and refusals either way.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{Endpoint, EndpointAddr};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::slices::b07_conflict_report::{IngestError, IngestOutcome};
use super::slices::b08_sync_scope::SyncScopeConfig;
use super::slices::b09_anti_entropy::{
    Inventory, ObjectDivergence, Offer, Replica, divergence, diverged_objects,
};
use super::slices::b32_ingest_gate::{Envelope, EventId, IngressPath, LogicalTime};
use crate::hex::{hex_encode, hex_to_array};

/// ALPN for the federation sync exchange. Within version 1 fields may
/// be added; removing or changing one needs `/id-federation-sync/2`.
pub const FEDERATION_SYNC_ALPN: &[u8] = b"/id-federation-sync/1";

/// Bound on one whole exchange, matching `envelope_net`'s delivery
/// bound.
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(30);

/// The protocol's error conversion, as in `crate::artifact`:
/// `AcceptError` takes a boxed standard error, and this crate's
/// fallible operations return `anyhow` errors.
fn accept_error(error: anyhow::Error) -> AcceptError {
    AcceptError::from_boxed(error.into())
}

// ---------------------------------------------------------------------
// Wire forms

fn hex_bytes(bytes: &[u8]) -> String {
    hex_encode(bytes)
}

fn unhex_bytes(text: &str) -> Result<Vec<u8>> {
    if text.len() % 2 != 0 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("not lower-case hex of even length: {text:?}");
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    for index in 0..text.len() / 2 {
        out.push(u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)?);
    }
    Ok(out)
}

fn unhex32(text: &str) -> Result<[u8; 32]> {
    hex_to_array(text).context("expected 64 hex characters")
}

fn unhex64(text: &str) -> Result<[u8; 64]> {
    hex_to_array(text).context("expected 128 hex characters")
}

/// A B32 gate envelope as it crosses the wire: every field of
/// [`Envelope`], byte fields in hex. The envelope's signature covers
/// its preimage, not this encoding, so the encoding only has to be
/// faithful, and `into_envelope` checks every length it relies on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct WireEnvelope {
    signer: String,
    verifying_key: String,
    signature: Option<String>,
    scope: String,
    payload: String,
    payload_digest: String,
    event_id: String,
    issued_at: LogicalTime,
}

impl WireEnvelope {
    fn of(envelope: &Envelope) -> Self {
        Self {
            signer: envelope.signer.clone(),
            verifying_key: hex_bytes(&envelope.verifying_key),
            signature: envelope.signature.as_ref().map(|sig| hex_bytes(sig)),
            scope: envelope.scope.clone(),
            payload: hex_bytes(&envelope.payload),
            payload_digest: hex_bytes(&envelope.payload_digest),
            event_id: hex_bytes(&envelope.event_id),
            issued_at: envelope.issued_at,
        }
    }

    fn into_envelope(self) -> Result<Envelope> {
        Ok(Envelope {
            signer: self.signer,
            verifying_key: unhex32(&self.verifying_key)?,
            signature: self.signature.as_deref().map(unhex64).transpose()?,
            scope: self.scope,
            payload: unhex_bytes(&self.payload)?,
            payload_digest: unhex32(&self.payload_digest)?,
            event_id: unhex32(&self.event_id)?,
            issued_at: self.issued_at,
        })
    }
}

/// One side's offer, as reported over the wire. Withheld versions
/// carry their B08 exclusion by name; the initiator's own offer is
/// computed locally and converted through the same shape so both
/// directions report identically.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetOffer {
    /// Versions to send, ascending, as event-id hex.
    pub send: Vec<String>,
    /// Versions withheld by the link's bounds, with the exclusion.
    pub withheld: Vec<(String, String)>,
    /// Versions not sent because the remote refused them before.
    pub refused_by_remote: Vec<String>,
}

impl NetOffer {
    fn of(offer: &Offer) -> Self {
        Self {
            send: offer.send.iter().map(|id| hex_bytes(id)).collect(),
            withheld: offer
                .withheld
                .iter()
                .map(|(id, why)| (hex_bytes(id), format!("{why:?}")))
                .collect(),
            refused_by_remote: offer
                .refused_by_remote
                .iter()
                .map(|id| hex_bytes(id))
                .collect(),
        }
    }
}

/// What one sent version came to at the receiver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetOutcome {
    /// Applied; the object has one head and no open report.
    Applied,
    /// Applied, and the object is diverged: this report (id, hex).
    Conflict(String),
    /// Applied as the resolution of this report (id, hex).
    Resolved(String),
    /// Admitted and held until its parents are stored.
    Held,
    /// Refused, with the receiver's reason.
    Refused(String),
}

fn net_outcome(result: &Result<IngestOutcome, IngestError>) -> NetOutcome {
    match result {
        Ok(IngestOutcome::Applied) => NetOutcome::Applied,
        Ok(IngestOutcome::Conflict(id)) => NetOutcome::Conflict(hex_bytes(id)),
        Ok(IngestOutcome::Resolved(id)) => NetOutcome::Resolved(hex_bytes(id)),
        Ok(IngestOutcome::Held) => NetOutcome::Held,
        Err(error) => NetOutcome::Refused(format!("{error:?}")),
    }
}

/// One direction of a carried exchange.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetTransfer {
    /// The offer this direction made.
    pub offer: NetOffer,
    /// The receiver's outcome for each sent version, in send order.
    pub outcomes: Vec<(EventId, NetOutcome)>,
}

/// The result of one carried exchange: the shape of
/// [`super::slices::b09_anti_entropy::Exchange`], with wire outcomes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetExchange {
    /// The divergence detected before anything moved, from the
    /// initiator's side.
    pub divergence: Vec<ObjectDivergence>,
    /// From the initiator to the responder.
    pub forward: NetTransfer,
    /// From the responder to the initiator.
    pub backward: NetTransfer,
}

impl NetExchange {
    /// Whether nothing was sent either way.
    pub fn is_quiet(&self) -> bool {
        self.forward.offer.send.is_empty() && self.backward.offer.send.is_empty()
    }
}

type SummaryWire = Vec<(String, String)>; // (object id hex, digest hex)
type SetsWire = Vec<(String, Vec<String>)>; // (object id hex, event id hexes)

fn summary_wire(summary: &BTreeMap<[u8; 32], [u8; 32]>) -> SummaryWire {
    summary
        .iter()
        .map(|(object, digest)| (hex_bytes(object), hex_bytes(digest)))
        .collect()
}

fn summary_from(wire: &SummaryWire) -> Result<BTreeMap<[u8; 32], [u8; 32]>> {
    wire.iter()
        .map(|(object, digest)| Ok((unhex32(object)?, unhex32(digest)?)))
        .collect()
}

fn sets_wire(inventory: &Inventory) -> SetsWire {
    inventory
        .objects()
        .map(|object| {
            (
                hex_bytes(object),
                inventory
                    .versions(object)
                    .map(|ids| ids.iter().map(|id| hex_bytes(id)).collect())
                    .unwrap_or_default(),
            )
        })
        .collect()
}

fn inventory_from(wire: &SetsWire) -> Result<Inventory> {
    let mut objects: BTreeMap<[u8; 32], BTreeSet<EventId>> = BTreeMap::new();
    for (object, ids) in wire {
        let mut set = BTreeSet::new();
        for id in ids {
            set.insert(unhex32(id)?);
        }
        objects.insert(unhex32(object)?, set);
    }
    Ok(Inventory::from_objects(objects))
}

fn refused_wire(replica: &Replica) -> Vec<String> {
    replica.refused().keys().map(|id| hex_bytes(id)).collect()
}

fn refused_from(wire: &[String]) -> Result<BTreeSet<EventId>> {
    wire.iter().map(|id| unhex32(id)).collect()
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "msg", rename_all = "snake_case")]
enum Request {
    Hello {
        from: String,
        now: LogicalTime,
        summary: SummaryWire,
    },
    Sets {
        versions: SetsWire,
        refused: Vec<String>,
    },
    Deliver {
        versions: Vec<WireEnvelope>,
    },
    Fetch,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "msg", rename_all = "snake_case")]
enum Response {
    Hello {
        from: String,
        summary: SummaryWire,
    },
    Sets {
        versions: SetsWire,
        refused: Vec<String>,
        backward_offer: NetOffer,
    },
    Delivered {
        outcomes: Vec<(String, NetOutcome)>,
    },
    Fetch {
        versions: Vec<WireEnvelope>,
    },
}

// ---------------------------------------------------------------------
// Responder

/// What one link's current exchange has established so far, on the
/// responder's side.
#[derive(Debug, Default)]
struct LinkState {
    peer_name: String,
    now: LogicalTime,
    backward_offer: Option<Offer>,
}

/// Serves the federation sync exchange for one replica. Register with
/// `Router::builder(..).accept(FEDERATION_SYNC_ALPN,
/// FederationSyncProtocol::new(replica, scope))`.
#[derive(Clone, Debug)]
pub struct FederationSyncProtocol {
    replica: Arc<Mutex<Replica>>,
    scope: SyncScopeConfig,
    link: Arc<Mutex<LinkState>>,
}

impl FederationSyncProtocol {
    /// Serve `replica`'s exchange over links scoped by `scope`.
    pub fn new(replica: Replica, scope: SyncScopeConfig) -> Self {
        Self {
            replica: Arc::new(Mutex::new(replica)),
            scope,
            link: Arc::new(Mutex::new(LinkState::default())),
        }
    }

    /// The served replica, shared with the process that hosts it so
    /// the host's local writes and reads go through the same lock as
    /// the served exchange.
    pub fn shared_replica(&self) -> Arc<Mutex<Replica>> {
        Arc::clone(&self.replica)
    }
}

impl ProtocolHandler for FederationSyncProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        loop {
            let (mut send, mut recv) = match conn.accept_bi().await {
                Ok(streams) => streams,
                Err(_) => return Ok(()),
            };
            let Some(frame) = id_core::frames::read_frame(&mut recv)
                .await
                .map_err(accept_error)?
            else {
                return Ok(());
            };
            let request: Request = crate::checked_json::from_slice_unique(&frame)
                .map_err(accept_error)?;
            let response = {
                let mut replica = self.replica.lock().await;
                let mut link = self.link.lock().await;
                answer(&mut replica, &self.scope, &mut link, request)
                    .map_err(accept_error)?
            };
            let json = serde_json::to_vec(&response).map_err(accept_error)?;
            id_core::frames::write_frame(&mut send, &json)
                .await
                .map_err(accept_error)?;
            send.finish().map_err(accept_error)?;
        }
    }
}

fn answer(
    replica: &mut Replica,
    scope: &SyncScopeConfig,
    link: &mut LinkState,
    request: Request,
) -> Result<Response> {
    match request {
        Request::Hello { from, now, .. } => {
            link.peer_name = from;
            link.now = now;
            Ok(Response::Hello {
                from: replica.name().to_owned(),
                summary: summary_wire(&replica.inventory().summary()),
            })
        }
        Request::Sets { versions, refused } => {
            let peer_inventory = inventory_from(&versions)?;
            let peer_refused = refused_from(&refused)?;
            let offer = replica.offer(&peer_inventory, &peer_refused, scope, link.now);
            let reply = Response::Sets {
                versions: sets_wire(&replica.inventory()),
                refused: refused_wire(replica),
                backward_offer: NetOffer::of(&offer),
            };
            link.backward_offer = Some(offer);
            Ok(reply)
        }
        Request::Deliver { versions } => {
            let peer_name = link.peer_name.clone();
            let mut outcomes = Vec::with_capacity(versions.len());
            for wire in versions {
                let envelope = wire.into_envelope()?;
                let event_id = envelope.event_id;
                let result = replica.receive(IngressPath::Internal, &envelope, &peer_name, link.now);
                outcomes.push((hex_bytes(&event_id), net_outcome(&result)));
            }
            Ok(Response::Delivered { outcomes })
        }
        Request::Fetch => {
            let offer = link
                .backward_offer
                .take()
                .context("fetch before the sets step")?;
            let versions = offer
                .send
                .iter()
                .filter_map(|id| replica.envelope(id))
                .map(WireEnvelope::of)
                .collect();
            Ok(Response::Fetch { versions })
        }
    }
}

// ---------------------------------------------------------------------
// Initiator

async fn roundtrip(conn: &Connection, request: &Request) -> Result<Response> {
    let (mut send, mut recv) = conn.open_bi().await.context("opening a stream")?;
    let json = serde_json::to_vec(request)?;
    id_core::frames::write_frame(&mut send, &json).await?;
    send.finish().context("finishing the request")?;
    let frame = id_core::frames::read_frame(&mut recv)
        .await?
        .context("no reply")?;
    crate::checked_json::from_slice_unique(&frame).context("malformed reply")
}

/// Reconcile `local` with the replica served at `target`, over one
/// link scoped by `config`, at logical time `now`: the carried form
/// of [`super::slices::b09_anti_entropy::reconcile`], with this
/// process as the first replica.
///
/// # Errors
///
/// Fails if the connection, a stream, or a message fails, or the
/// peer's messages do not decode. A version the peer refuses is not
/// an error; it is reported in the exchange's outcomes.
pub async fn exchange(
    endpoint: &Endpoint,
    target: impl Into<EndpointAddr>,
    local: &mut Replica,
    config: &SyncScopeConfig,
    now: LogicalTime,
) -> Result<NetExchange> {
    let work = exchange_inner(endpoint, target.into(), local, config, now);
    tokio::time::timeout(EXCHANGE_TIMEOUT, work)
        .await
        .context("exchange timed out")?
}

async fn exchange_inner(
    endpoint: &Endpoint,
    target: EndpointAddr,
    local: &mut Replica,
    config: &SyncScopeConfig,
    now: LogicalTime,
) -> Result<NetExchange> {
    let conn = id_core::connectivity::connect(endpoint, target, FEDERATION_SYNC_ALPN).await?;
    let local_inventory = local.inventory();
    let local_summary = local_inventory.summary();

    // 1. Summaries first; no divergence means a quiet exchange.
    let Response::Hello {
        from: remote_name,
        summary: remote_summary_wire,
    } = roundtrip(
        &conn,
        &Request::Hello {
            from: local.name().to_owned(),
            now,
            summary: summary_wire(&local_summary),
        },
    )
    .await?
    else {
        bail!("peer answered the hello with a different message");
    };
    let remote_summary = summary_from(&remote_summary_wire)?;
    if diverged_objects(&local_summary, &remote_summary).is_empty() {
        conn.close(0_u32.into(), b"done");
        return Ok(NetExchange::default());
    }

    // 2. Sets: the responder computes and holds its offer; the
    // initiator computes the divergence and its own offer, both from
    // the inventories as they stood before anything moved.
    let local_refused: BTreeSet<EventId> = local.refused().keys().copied().collect();
    let Response::Sets {
        versions: remote_sets,
        refused: remote_refused_wire,
        backward_offer,
    } = roundtrip(
        &conn,
        &Request::Sets {
            versions: sets_wire(&local_inventory),
            refused: local_refused.iter().map(|id| hex_bytes(id)).collect(),
        },
    )
    .await?
    else {
        bail!("peer answered the sets step with a different message");
    };
    let remote_inventory = inventory_from(&remote_sets)?;
    let remote_refused = refused_from(&remote_refused_wire)?;
    let detected = divergence(&local_inventory, &remote_inventory);
    let forward_offer = local.offer(&remote_inventory, &remote_refused, config, now);

    // 3. Delivery: the responder admits each offered version through
    // its own store and reports the outcomes.
    let forward_versions: Vec<WireEnvelope> = forward_offer
        .send
        .iter()
        .filter_map(|id| local.envelope(id))
        .map(WireEnvelope::of)
        .collect();
    let Response::Delivered { outcomes } = roundtrip(
        &conn,
        &Request::Deliver {
            versions: forward_versions,
        },
    )
    .await?
    else {
        bail!("peer answered the delivery with a different message");
    };
    let forward_outcomes = outcomes
        .into_iter()
        .map(|(id, outcome)| Ok((unhex32(&id)?, outcome)))
        .collect::<Result<Vec<_>>>()?;

    // 4. Fetch: the initiator admits the responder's offered versions
    // through its own store.
    let Response::Fetch {
        versions: fetched,
    } = roundtrip(&conn, &Request::Fetch).await?
    else {
        bail!("peer answered the fetch with a different message");
    };
    let mut backward_outcomes = Vec::with_capacity(fetched.len());
    for wire in fetched {
        let envelope = wire.into_envelope()?;
        let event_id = envelope.event_id;
        let result = local.receive(IngressPath::Internal, &envelope, &remote_name, now);
        backward_outcomes.push((event_id, net_outcome(&result)));
    }
    conn.close(0_u32.into(), b"done");

    Ok(NetExchange {
        divergence: detected,
        forward: NetTransfer {
            offer: NetOffer::of(&forward_offer),
            outcomes: forward_outcomes,
        },
        backward: NetTransfer {
            offer: backward_offer,
            outcomes: backward_outcomes,
        },
    })
}
