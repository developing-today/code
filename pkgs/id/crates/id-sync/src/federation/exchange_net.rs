//! The B09 anti-entropy exchange carried over the existing transport
//! (#12232 step B2(c)): the same exchange [`reconcile`] runs between
//! two in-process [`Replica`]s, run instead between two processes
//! over iroh.
//!
//! One bidirectional stream carries one session, as a sequence of
//! JSON frames ([`id_core::frames`], the framing the friend-envelope
//! protocol `/id-envelope/1` already uses):
//!
//! 1. The initiator sends [`Hello`]: its replica name, the link's
//!    [`SyncScopeConfig`] fields, the exchange's logical time, its
//!    inventory summary and the ids it has refused.
//! 2. The responder answers [`HelloReply`]: its name, summary and
//!    refused ids, and its version ids for the objects whose digests
//!    diverged (B09's digest-first rule: ids are listed only where
//!    the summaries disagree).
//! 3. The initiator sends [`ClientTransfer`]: its own ids for the
//!    diverged objects, and the signed envelopes its offer sends.
//! 4. The responder answers [`ServerTransfer`]: the ingest outcome of
//!    each envelope it received, its own offer, and the envelopes
//!    that offer sends.
//!
//! Both offers are computed from the inventories as they stood before
//! either transfer, exactly as in [`reconcile`], and the receives
//! happen in the same order (initiator's versions first). The
//! initiator assembles the [`Exchange`] the session produced, in the
//! same shape `reconcile` returns it. What makes a version trusted is
//! its signature and the receiving replica's gate, never the iroh
//! node that carried it.
//!
//! The wire types below mirror the federation types field for field;
//! the slices keep no serde derives of their own (the on-disk record
//! store made the same choice), so the conversions live here, with
//! the transport that needs them.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use iroh::{
    Endpoint, EndpointAddr,
    endpoint::{Connection, RecvStream, SendStream},
    protocol::{AcceptError, ProtocolHandler},
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use id_core::connectivity::connect;
use id_core::frames::{read_frame, write_frame};

use super::record_join::JoinRefusal;
use super::slices::b07_conflict_report::{
    ConflictKey, Digest32, IngestError, IngestOutcome, ObjectId, ReportId,
};
use super::slices::b08_sync_scope::{Exclusion, SyncScopeConfig};
use super::slices::b09_anti_entropy::{
    Exchange, Inventory, Offer, Replica, Transfer, diverged_objects, divergence,
};
use super::slices::b32_ingest_gate::{Envelope, EventId, IngressPath, LogicalTime, RejectReason};

/// ALPN for the federation exchange. Within version 1 fields may be
/// added; removing or changing one needs `/id-fed-exchange/2`.
pub const EXCHANGE_ALPN: &[u8] = b"/id-fed-exchange/1";

/// How long one session may take, connect included.
const TIMEOUT: Duration = Duration::from_secs(30);

/// The B32 envelope as it crosses the wire: every field, unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireEnvelope {
    /// The claimed signer identity.
    pub signer: String,
    /// The verifying key the envelope claims for its signer.
    pub verifying_key: [u8; 32],
    /// The signature over the envelope's preimage, if any.
    #[serde(serialize_with = "ser_signature", deserialize_with = "de_signature")]
    pub signature: Option<[u8; 64]>,
    /// The scope the envelope asks to be admitted into.
    pub scope: String,
    /// The payload bytes.
    pub payload: Vec<u8>,
    /// The declared digest of `payload`.
    pub payload_digest: [u8; 32],
    /// The unique event id (replay identity).
    pub event_id: [u8; 32],
    /// When the envelope was issued, in logical seconds.
    pub issued_at: u64,
}

/// Serialize a signature: serde covers byte arrays only up to 32
/// elements, so the 64-byte signature crosses as a byte list, like
/// the payload.
fn ser_signature<S: serde::Serializer>(
    signature: &Option<[u8; 64]>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    signature
        .as_ref()
        .map(|bytes| bytes.as_slice())
        .serialize(serializer)
}

/// Deserialize a signature written by [`ser_signature`]; any length
/// but 64 is malformed.
fn de_signature<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<[u8; 64]>, D::Error> {
    let bytes: Option<Vec<u8>> = Option::deserialize(deserializer)?;
    match bytes {
        None => Ok(None),
        Some(bytes) => bytes
            .try_into()
            .map(Some)
            .map_err(|_| serde::de::Error::custom("signature is not 64 bytes")),
    }
}

impl From<&Envelope> for WireEnvelope {
    fn from(envelope: &Envelope) -> Self {
        Self {
            signer: envelope.signer.clone(),
            verifying_key: envelope.verifying_key,
            signature: envelope.signature,
            scope: envelope.scope.clone(),
            payload: envelope.payload.clone(),
            payload_digest: envelope.payload_digest,
            event_id: envelope.event_id,
            issued_at: envelope.issued_at,
        }
    }
}

impl From<WireEnvelope> for Envelope {
    fn from(wire: WireEnvelope) -> Self {
        Self {
            signer: wire.signer,
            verifying_key: wire.verifying_key,
            signature: wire.signature,
            scope: wire.scope,
            payload: wire.payload,
            payload_digest: wire.payload_digest,
            event_id: wire.event_id,
            issued_at: wire.issued_at,
        }
    }
}

/// What happened to an admitted version, as it crosses the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireOutcome {
    /// Applied; the object has one head and no open report.
    Applied,
    /// Applied, and the object is (still) diverged: this is the report.
    Conflict(ReportId),
    /// Applied as the resolution of this report.
    Resolved(ReportId),
    /// Admitted and held until its parents are stored.
    Held,
}

impl From<&IngestOutcome> for WireOutcome {
    fn from(outcome: &IngestOutcome) -> Self {
        match outcome {
            IngestOutcome::Applied => Self::Applied,
            IngestOutcome::Conflict(id) => Self::Conflict(*id),
            IngestOutcome::Resolved(id) => Self::Resolved(*id),
            IngestOutcome::Held => Self::Held,
        }
    }
}

impl From<WireOutcome> for IngestOutcome {
    fn from(wire: WireOutcome) -> Self {
        match wire {
            WireOutcome::Applied => Self::Applied,
            WireOutcome::Conflict(id) => Self::Conflict(id),
            WireOutcome::Resolved(id) => Self::Resolved(id),
            WireOutcome::Held => Self::Held,
        }
    }
}

/// Why the gate refused an envelope, as it crosses the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireRejectReason {
    /// No signature was presented.
    Unsigned,
    /// The verifying key is bound to no known signer.
    UnknownSigner,
    /// The key is bound to a different identity than claimed.
    SignerMismatch {
        /// The identity the envelope claimed.
        claimed: String,
        /// The identity the key is actually bound to.
        bound_to: String,
    },
    /// The signature does not verify over the preimage.
    InvalidSignature,
    /// The payload is semantically invalid.
    SemanticInvalid,
    /// The bound signer is not authorized for the scope.
    NotAuthorized {
        /// The scope that was refused.
        scope: String,
    },
    /// The envelope is older than the freshness window allows.
    Stale,
    /// The envelope is dated beyond the allowed future skew.
    FutureSkew,
    /// This event id was already admitted: a replay.
    Replay,
}

impl From<&RejectReason> for WireRejectReason {
    fn from(reason: &RejectReason) -> Self {
        match reason {
            RejectReason::Unsigned => Self::Unsigned,
            RejectReason::UnknownSigner => Self::UnknownSigner,
            RejectReason::SignerMismatch { claimed, bound_to } => Self::SignerMismatch {
                claimed: claimed.clone(),
                bound_to: bound_to.clone(),
            },
            RejectReason::InvalidSignature => Self::InvalidSignature,
            RejectReason::SemanticInvalid => Self::SemanticInvalid,
            RejectReason::NotAuthorized { scope } => Self::NotAuthorized {
                scope: scope.clone(),
            },
            RejectReason::Stale => Self::Stale,
            RejectReason::FutureSkew => Self::FutureSkew,
            RejectReason::Replay => Self::Replay,
        }
    }
}

impl From<WireRejectReason> for RejectReason {
    fn from(wire: WireRejectReason) -> Self {
        match wire {
            WireRejectReason::Unsigned => Self::Unsigned,
            WireRejectReason::UnknownSigner => Self::UnknownSigner,
            WireRejectReason::SignerMismatch { claimed, bound_to } => {
                Self::SignerMismatch { claimed, bound_to }
            }
            WireRejectReason::InvalidSignature => Self::InvalidSignature,
            WireRejectReason::SemanticInvalid => Self::SemanticInvalid,
            WireRejectReason::NotAuthorized { scope } => Self::NotAuthorized { scope },
            WireRejectReason::Stale => Self::Stale,
            WireRejectReason::FutureSkew => Self::FutureSkew,
            WireRejectReason::Replay => Self::Replay,
        }
    }
}

/// How a version's body broke the record join, as it crosses the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireJoinRefusal {
    /// The body is not a decodable signed-record envelope at all.
    BodyNotEnvelope,
    /// The body decodes but does not open as a valid signed record.
    BodyNotValidRecord,
    /// The body's digest is not the version's event id.
    EventIdMismatch,
    /// The header's object is not the record's logical graph.
    ObjectMismatch,
    /// The version's key is not the record's key.
    SignerMismatch,
}

impl From<&JoinRefusal> for WireJoinRefusal {
    fn from(refusal: &JoinRefusal) -> Self {
        match refusal {
            JoinRefusal::BodyNotEnvelope => Self::BodyNotEnvelope,
            JoinRefusal::BodyNotValidRecord => Self::BodyNotValidRecord,
            JoinRefusal::EventIdMismatch => Self::EventIdMismatch,
            JoinRefusal::ObjectMismatch => Self::ObjectMismatch,
            JoinRefusal::SignerMismatch => Self::SignerMismatch,
        }
    }
}

impl From<WireJoinRefusal> for JoinRefusal {
    fn from(wire: WireJoinRefusal) -> Self {
        match wire {
            WireJoinRefusal::BodyNotEnvelope => Self::BodyNotEnvelope,
            WireJoinRefusal::BodyNotValidRecord => Self::BodyNotValidRecord,
            WireJoinRefusal::EventIdMismatch => Self::EventIdMismatch,
            WireJoinRefusal::ObjectMismatch => Self::ObjectMismatch,
            WireJoinRefusal::SignerMismatch => Self::SignerMismatch,
        }
    }
}

/// Why a replica refused a version, as it crosses the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireError {
    /// The ingest gate refused the envelope.
    Rejected(WireRejectReason),
    /// The version claims the record join and breaks it.
    JoinRefused(WireJoinRefusal),
    /// The payload is not a canonical version header, or names itself
    /// as a parent.
    MalformedVersion,
    /// A parent belongs to a different object.
    ForeignParent(EventId),
    /// The version merges parents without naming the conflict.
    ResolutionRequired {
        /// The key the version would have to name.
        key: ConflictKey,
    },
    /// The version names a conflict it does not resolve.
    NotAResolution {
        /// The key the version named.
        named: ConflictKey,
    },
    /// A merge names a parent that descends from another parent.
    RedundantParents,
}

impl From<&IngestError> for WireError {
    fn from(error: &IngestError) -> Self {
        match error {
            IngestError::Rejected(reason) => Self::Rejected(reason.into()),
            IngestError::JoinRefused(refusal) => Self::JoinRefused(refusal.into()),
            IngestError::MalformedVersion => Self::MalformedVersion,
            IngestError::ForeignParent(id) => Self::ForeignParent(*id),
            IngestError::ResolutionRequired { key } => Self::ResolutionRequired { key: *key },
            IngestError::NotAResolution { named } => Self::NotAResolution { named: *named },
            IngestError::RedundantParents => Self::RedundantParents,
        }
    }
}

impl From<WireError> for IngestError {
    fn from(wire: WireError) -> Self {
        match wire {
            WireError::Rejected(reason) => Self::Rejected(reason.into()),
            WireError::JoinRefused(refusal) => Self::JoinRefused(refusal.into()),
            WireError::MalformedVersion => Self::MalformedVersion,
            WireError::ForeignParent(id) => Self::ForeignParent(id),
            WireError::ResolutionRequired { key } => Self::ResolutionRequired { key },
            WireError::NotAResolution { named } => Self::NotAResolution { named },
            WireError::RedundantParents => Self::RedundantParents,
        }
    }
}

/// The result of admitting one version, as it crosses the wire.
pub type WireOutcomeResult = Result<WireOutcome, WireError>;

/// An ingest result as it crosses the wire.
pub fn to_wire_outcome(result: &Result<IngestOutcome, IngestError>) -> WireOutcomeResult {
    match result {
        Ok(outcome) => Ok(outcome.into()),
        Err(error) => Err(error.into()),
    }
}

/// An ingest result as it arrived over the wire, back in the
/// federation's own types.
pub fn from_wire_outcome(result: WireOutcomeResult) -> Result<IngestOutcome, IngestError> {
    match result {
        Ok(outcome) => Ok(outcome.into()),
        Err(error) => Err(error.into()),
    }
}

/// Why a version was withheld from a link, as it crosses the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireExclusion {
    /// The record belongs to another scope.
    OutsideScope,
    /// The record belongs to another tenant / trust domain.
    TenantBoundary,
    /// The record is not authorized for this link's scope.
    NotAuthorized,
    /// The content is deleted by policy.
    DeletedByPolicy,
    /// The record is past its retention bound.
    ExpiredRetention,
}

impl From<&Exclusion> for WireExclusion {
    fn from(exclusion: &Exclusion) -> Self {
        match exclusion {
            Exclusion::OutsideScope => Self::OutsideScope,
            Exclusion::TenantBoundary => Self::TenantBoundary,
            Exclusion::NotAuthorized => Self::NotAuthorized,
            Exclusion::DeletedByPolicy => Self::DeletedByPolicy,
            Exclusion::ExpiredRetention => Self::ExpiredRetention,
        }
    }
}

impl From<WireExclusion> for Exclusion {
    fn from(wire: WireExclusion) -> Self {
        match wire {
            WireExclusion::OutsideScope => Self::OutsideScope,
            WireExclusion::TenantBoundary => Self::TenantBoundary,
            WireExclusion::NotAuthorized => Self::NotAuthorized,
            WireExclusion::DeletedByPolicy => Self::DeletedByPolicy,
            WireExclusion::ExpiredRetention => Self::ExpiredRetention,
        }
    }
}

/// What one side of an exchange would send, as it crosses the wire.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireOffer {
    /// Versions to send, ascending.
    pub send: Vec<EventId>,
    /// Versions the link's bounds withhold, with the B08 reason.
    pub withheld: Vec<(EventId, WireExclusion)>,
    /// Versions not sent because the remote refused them before.
    pub refused_by_remote: Vec<EventId>,
}

impl From<&Offer> for WireOffer {
    fn from(offer: &Offer) -> Self {
        Self {
            send: offer.send.clone(),
            withheld: offer
                .withheld
                .iter()
                .map(|(id, exclusion)| (*id, exclusion.into()))
                .collect(),
            refused_by_remote: offer.refused_by_remote.clone(),
        }
    }
}

impl From<WireOffer> for Offer {
    fn from(wire: WireOffer) -> Self {
        Self {
            send: wire.send,
            withheld: wire
                .withheld
                .into_iter()
                .map(|(id, exclusion)| (id, exclusion.into()))
                .collect(),
            refused_by_remote: wire.refused_by_remote,
        }
    }
}

/// One object's version ids, listed because its digest diverged.
pub type IdList = Vec<(ObjectId, Vec<EventId>)>;

/// An inventory summary as it crosses the wire: object, digest.
pub type Summary = Vec<(ObjectId, Digest32)>;

fn summary_of(inventory: &Inventory) -> Summary {
    inventory.summary().into_iter().collect()
}

fn ids_of(inventory: &Inventory, objects: &[ObjectId]) -> IdList {
    objects
        .iter()
        .map(|object| {
            (
                *object,
                inventory
                    .versions(object)
                    .map(|ids| ids.iter().copied().collect())
                    .unwrap_or_default(),
            )
        })
        .collect()
}

/// Assemble a peer's inventory from the exchange: its summary named
/// every object, `remote_ids` listed the diverged ones, and for the
/// objects the digests agree on the peer's set is this replica's own
/// (equal digests cover equal sets).
fn assemble_inventory(
    own: &Inventory,
    own_summary: &BTreeMap<ObjectId, Digest32>,
    remote_summary: &BTreeMap<ObjectId, Digest32>,
    remote_ids: &IdList,
) -> Inventory {
    let diverged: BTreeSet<ObjectId> = diverged_objects(own_summary, remote_summary)
        .into_iter()
        .collect();
    let remote: BTreeMap<ObjectId, BTreeSet<EventId>> = remote_ids
        .iter()
        .map(|(object, ids)| (*object, ids.iter().copied().collect()))
        .collect();
    let mut objects: BTreeMap<ObjectId, BTreeSet<EventId>> = BTreeMap::new();
    for object in own_summary.keys().chain(remote_summary.keys()) {
        if objects.contains_key(object) {
            continue;
        }
        if diverged.contains(object) {
            objects.insert(*object, remote.get(object).cloned().unwrap_or_default());
        } else if let Some(ids) = own.versions(object) {
            objects.insert(*object, ids.clone());
        }
    }
    Inventory::from_object_versions(objects)
}

/// Frame 1, initiator to responder: who is calling, on what link,
/// holding what.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    /// The initiator replica's name: the source label on what it sends.
    pub name: String,
    /// The link's only scope.
    pub scope: String,
    /// The link's only trust domain.
    pub trust_domain: String,
    /// The link's record capacity.
    pub capacity: usize,
    /// The link's retention window in logical seconds, if any.
    pub retention: Option<u64>,
    /// The exchange's logical time.
    pub now: LogicalTime,
    /// The initiator's inventory summary.
    pub summary: Summary,
    /// The ids the initiator has refused, ascending.
    pub refused: Vec<EventId>,
}

/// Frame 2, responder to initiator: the same, from the other side,
/// plus the responder's ids for the diverged objects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloReply {
    /// The responder replica's name.
    pub name: String,
    /// The responder's inventory summary.
    pub summary: Summary,
    /// The ids the responder has refused, ascending.
    pub refused: Vec<EventId>,
    /// The responder's version ids for the diverged objects.
    pub ids: IdList,
}

/// Frame 3, initiator to responder: the initiator's ids for the
/// diverged objects, and the envelopes its offer sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientTransfer {
    /// The initiator's version ids for the diverged objects.
    pub ids: IdList,
    /// The envelopes the initiator's offer sends, in send order.
    pub versions: Vec<WireEnvelope>,
}

/// Frame 4, responder to initiator: what the initiator's envelopes
/// came to, the responder's offer, and the envelopes it sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerTransfer {
    /// The responder's result for each sent version, in send order.
    pub outcomes: Vec<(EventId, WireOutcomeResult)>,
    /// The responder's offer, computed before anything moved.
    pub offer: WireOffer,
    /// The envelopes the responder's offer sends, in send order.
    pub versions: Vec<WireEnvelope>,
}

/// Run one exchange with the replica at `target`, as [`reconcile`]
/// would run it in process: `local` is the initiator, the replica
/// behind the [`ExchangeProtocol`] at `target` is the responder, and
/// the returned [`Exchange`] is the one the session produced.
pub async fn reconcile_remote(
    endpoint: &Endpoint,
    target: impl Into<EndpointAddr>,
    local: &mut Replica,
    config: &SyncScopeConfig,
    now: LogicalTime,
) -> Result<Exchange> {
    let local_inventory = local.inventory();
    let local_summary: BTreeMap<ObjectId, Digest32> = local_inventory.summary();
    let local_refused: BTreeSet<EventId> = local.refused().keys().copied().collect();
    let hello = Hello {
        name: local.name().to_owned(),
        scope: config.scope.clone(),
        trust_domain: config.trust_domain.clone(),
        capacity: config.capacity,
        retention: config.retention,
        now,
        summary: summary_of(&local_inventory),
        refused: local_refused.iter().copied().collect(),
    };

    // The session's first half: everything it sends is computed from
    // the inventories as they stand, before anything moves — the
    // same rule `reconcile` follows.
    let session = async {
        let conn = connect(endpoint, target.into(), EXCHANGE_ALPN)
            .await
            .context("connecting to the responder")?;
        let (mut send, mut recv) = conn.open_bi().await.context("opening a stream")?;
        write_frame(&mut send, &serde_json::to_vec(&hello)?).await?;
        let frame = read_frame(&mut recv)
            .await?
            .context("the responder closed without a hello reply")?;
        let reply: HelloReply =
            crate::checked_json::from_slice_unique(&frame).context("malformed hello reply")?;

        let remote_summary: BTreeMap<ObjectId, Digest32> = reply.summary.iter().copied().collect();
        let remote_refused: BTreeSet<EventId> = reply.refused.iter().copied().collect();
        let remote_inventory = assemble_inventory(
            &local_inventory,
            &local_summary,
            &remote_summary,
            &reply.ids,
        );
        let diverged = diverged_objects(&local_summary, &remote_summary);
        let divergence = divergence(&local_inventory, &remote_inventory);
        let offer = local.offer(&remote_inventory, &remote_refused, config, now);
        let versions: Vec<WireEnvelope> = offer
            .send
            .iter()
            .filter_map(|event_id| local.envelope(event_id))
            .map(WireEnvelope::from)
            .collect();
        let transfer = ClientTransfer {
            ids: ids_of(&local_inventory, &diverged),
            versions,
        };
        write_frame(&mut send, &serde_json::to_vec(&transfer)?).await?;
        send.finish().context("finishing the transfer")?;
        let frame = read_frame(&mut recv)
            .await?
            .context("the responder closed without a transfer reply")?;
        let server: ServerTransfer =
            crate::checked_json::from_slice_unique(&frame).context("malformed transfer reply")?;
        conn.close(0_u32.into(), b"done");
        Ok::<_, anyhow::Error>((reply.name, divergence, offer, server))
    };
    let (remote_name, divergence, offer, server) =
        tokio::time::timeout(TIMEOUT, session)
            .await
            .map_err(|_| anyhow::anyhow!("the exchange timed out after {TIMEOUT:?}"))??;

    // The session's second half: the responder's versions are
    // admitted through the local gate, in send order, as in
    // `reconcile`'s backward transfer.
    let forward = Transfer {
        offer,
        outcomes: server
            .outcomes
            .into_iter()
            .map(|(event_id, result)| (event_id, from_wire_outcome(result)))
            .collect(),
    };
    let mut backward_outcomes = Vec::new();
    for wire in server.versions {
        let envelope: Envelope = wire.into();
        let event_id = envelope.event_id;
        let result = local.receive(IngressPath::Internal, &envelope, &remote_name, now);
        backward_outcomes.push((event_id, result));
    }
    let backward = Transfer {
        offer: server.offer.into(),
        outcomes: backward_outcomes,
    };
    Ok(Exchange {
        divergence,
        forward,
        backward,
    })
}

/// Accepts federation exchanges over iroh. Register with
/// `Router::builder(..).accept(EXCHANGE_ALPN, ExchangeProtocol::new(replica))`.
/// The replica is shared with the node's owner, which produces and
/// reads through the same lock between sessions.
#[derive(Clone, Debug)]
pub struct ExchangeProtocol {
    replica: Arc<Mutex<Replica>>,
}

impl ExchangeProtocol {
    /// Serve exchanges for `replica`.
    #[must_use]
    pub const fn new(replica: Arc<Mutex<Replica>>) -> Self {
        Self { replica }
    }
}

impl ExchangeProtocol {
    /// Serve one exchange session on an accepted stream pair: the
    /// mirror image of [`reconcile_remote`]'s first half, then the
    /// transfers in the order `reconcile` performs them.
    async fn session(&self, send: &mut SendStream, recv: &mut RecvStream) -> Result<()> {
        let frame = read_frame(recv)
            .await?
            .context("the initiator closed without a hello")?;
        let hello: Hello =
            crate::checked_json::from_slice_unique(&frame).context("malformed hello")?;
        let config = SyncScopeConfig {
            scope: hello.scope.clone(),
            trust_domain: hello.trust_domain.clone(),
            capacity: hello.capacity,
            retention: hello.retention,
        };
        let hello_summary: BTreeMap<ObjectId, Digest32> = hello.summary.iter().copied().collect();
        let hello_refused: BTreeSet<EventId> = hello.refused.iter().copied().collect();

        let mut replica = self.replica.lock().await;
        let own_inventory = replica.inventory();
        let own_summary: BTreeMap<ObjectId, Digest32> = own_inventory.summary();
        let own_refused: BTreeSet<EventId> = replica.refused().keys().copied().collect();
        let diverged = diverged_objects(&own_summary, &hello_summary);
        let reply = HelloReply {
            name: replica.name().to_owned(),
            summary: summary_of(&own_inventory),
            refused: own_refused.iter().copied().collect(),
            ids: ids_of(&own_inventory, &diverged),
        };
        write_frame(send, &serde_json::to_vec(&reply)?).await?;

        let frame = read_frame(recv)
            .await?
            .context("the initiator closed without a transfer")?;
        let transfer: ClientTransfer =
            crate::checked_json::from_slice_unique(&frame).context("malformed transfer")?;

        // The responder's offer is computed from the pre-exchange
        // inventories, before the initiator's versions are admitted —
        // the rule that keeps the two offers independent.
        let remote_inventory =
            assemble_inventory(&own_inventory, &own_summary, &hello_summary, &transfer.ids);
        let offer = replica.offer(&remote_inventory, &hello_refused, &config, hello.now);
        let mut outcomes = Vec::new();
        for wire in transfer.versions {
            let envelope: Envelope = wire.into();
            let event_id = envelope.event_id;
            let result = replica.receive(IngressPath::Internal, &envelope, &hello.name, hello.now);
            outcomes.push((event_id, to_wire_outcome(&result)));
        }
        let versions: Vec<WireEnvelope> = offer
            .send
            .iter()
            .filter_map(|event_id| replica.envelope(event_id))
            .map(WireEnvelope::from)
            .collect();
        let server = ServerTransfer {
            outcomes,
            offer: WireOffer::from(&offer),
            versions,
        };
        write_frame(send, &serde_json::to_vec(&server)?).await?;
        send.finish().context("finishing the transfer reply")?;
        Ok(())
    }
}

impl ProtocolHandler for ExchangeProtocol {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        while let Ok((mut send, mut recv)) = connection.accept_bi().await {
            // One session per stream; a session that fails or stalls
            // ends that stream, never the endpoint.
            let _ = tokio::time::timeout(TIMEOUT, self.session(&mut send, &mut recv)).await;
        }
        Ok(())
    }
}
