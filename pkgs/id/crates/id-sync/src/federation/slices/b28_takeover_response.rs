//! B28: response to a highly trusted server's takeover or key
//! compromise (#10232, statements S1.83-S1.87; Part B "Emergency
//! key/peer controls" and "Takeover response").
//!
//! A trusted server can turn hostile, either because someone took the
//! server over or because one of its keys leaked while the server
//! stayed honest. The system survives it with four rules:
//!
//! 1. **Revocation is immediate (S1.84).** Declaring an incident writes
//!    B02 revocation records in the same call, effective from the
//!    operator's suspected start and with no end, so from the detection
//!    instant on no new data under the incident's keys (or, for a
//!    takeover, from the server at all) is accepted or relayed. Nothing
//!    here lifts a takeover: rolling back the automation never restores
//!    the server.
//! 2. **The blast radius is a set (S1.85).** [`TakeoverResponder::blast_radius`]
//!    names every accepted record the incident touches, and the objects
//!    they belong to, from key lineage and local admission time alone:
//!    records admitted at or after the suspected start are *suspect*,
//!    earlier ones *prior*. The signer's own claimed signing time is
//!    stored for audit and never read, because a hostile signer can
//!    backdate it; so the historical validity of every touched record
//!    is [`HistoricalValidity::Indeterminate`] until it is revalidated.
//! 3. **The posture for prior data is fixed (S1.86).** Suspect records
//!    are quarantined (withheld from release); prior records are flagged
//!    (still released, marked indeterminate). Both keep their signed
//!    bytes and admission evidence. Only explicit evidence clears them:
//!    an independently witnessed good-through instant, or a fresh
//!    attestation over the stored bytes by a key that is not revoked
//!    and, after a takeover, does not belong to the server at all.
//! 4. **Takeover and key compromise are distinguished, and an unknown
//!    cause is conflated with takeover (S1.87).** A *key compromise*
//!    names the leaked keys of the server's lineage: only those keys are
//!    revoked, the server stays enabled, and its later keys may attest
//!    for its earlier data. A *takeover* revokes every key in the
//!    server's lineage and disables the server as a peer; data signed
//!    by any key the server enrolls later is refused too, and none of
//!    its keys may attest. An *undetermined* incident gets the takeover
//!    response, and its cause is recorded as indeterminate.
//!
//! Times are logical seconds supplied by the caller. Cross-peer
//! attribution and reputation are later work (V4).
//!
//! This module is deliberately self-contained: the federation roots
//! record slice paths without declaring them (slice 9110-02), so the
//! slice's integration test includes this file directly and no shared
//! root names it yet; #9110-09 wires versioned public exports after
//! module outputs land.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use super::b01_key_lifecycle::{KeyLineage, LifecycleError};
use super::b02_key_revocation::{
    AcceptedRecord, Decision, DenyReason, HistoricalValidity, LogicalTime, RecordStatus,
    RevalidationEvidence, RevalidationMethod, RevocationError, RevocationReason, RevocationRecord,
    RevocationRegistry, RevocationSubject,
};

/// The id of the object a record belongs to.
pub type ObjectId = [u8; 32];
/// An Ed25519 verifying key, raw.
pub type KeyBytes = [u8; 32];

/// What an operator declares happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncidentKind {
    /// These keys of the server leaked; the server itself is honest.
    KeyCompromise {
        /// The leaked keys; each must be in the server's lineage.
        keys: BTreeSet<KeyBytes>,
    },
    /// The server itself is hostile.
    ServerTakeover,
    /// Not yet known which. Handled as a takeover.
    Undetermined,
}

/// What the incident record says caused it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cause {
    /// Leaked keys on an honest server.
    KeyCompromise,
    /// A hostile server.
    ServerTakeover,
    /// Unknown; the takeover response applies.
    Indeterminate,
}

impl Cause {
    /// Whether the response treats the server itself as hostile.
    pub const fn server_hostile(self) -> bool {
        matches!(self, Self::ServerTakeover | Self::Indeterminate)
    }
}

/// An operator's declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// Unique incident id.
    pub incident_id: String,
    /// The server (its peer id).
    pub server: String,
    /// What happened.
    pub kind: IncidentKind,
    /// The earliest instant the operator suspects, in local admission
    /// time. Never taken from a signer's claim.
    pub suspected_from: LogicalTime,
}

/// A declared incident, as recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incident {
    /// Unique incident id.
    pub id: String,
    /// The server (its peer id).
    pub server: String,
    /// The recorded cause.
    pub cause: Cause,
    /// The keys revoked by this incident.
    pub keys: BTreeSet<KeyBytes>,
    /// Whether the server was disabled as a peer.
    pub peer_disabled: bool,
    /// The suspected start.
    pub suspected_from: LogicalTime,
    /// When it was declared.
    pub detected_at: LogicalTime,
}

impl Incident {
    /// Whether this incident touches a record signed by `key` and
    /// supplied by `peer_id`.
    pub fn touches(&self, key: &KeyBytes, peer_id: &str) -> bool {
        self.keys.contains(key) || (self.peer_disabled && peer_id == self.server)
    }
}

/// Which side of the suspected start a touched record was admitted on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Window {
    /// Admitted before the suspected start.
    Prior,
    /// Admitted at or after the suspected start.
    Suspect,
}

/// One record an incident touches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exposure {
    /// The record's object.
    pub object_id: ObjectId,
    /// When it was admitted locally.
    pub admitted_at: LogicalTime,
    /// Which side of the suspected start.
    pub window: Window,
    /// Its current status.
    pub status: RecordStatus,
}

/// Everything one incident touches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlastRadius {
    /// The incident.
    pub incident_id: String,
    /// Its recorded cause.
    pub cause: Cause,
    /// Touched records, by record id.
    pub records: BTreeMap<String, Exposure>,
    /// The objects those records belong to.
    pub objects: BTreeSet<ObjectId>,
}

impl BlastRadius {
    /// Record ids in one window.
    pub fn in_window(&self, window: Window) -> BTreeSet<String> {
        self.records
            .iter()
            .filter(|(_, e)| e.window == window)
            .map(|(id, _)| id.clone())
            .collect()
    }
}

/// Why acceptance or relay was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Refusal {
    /// The key is revoked (B02).
    RevokedKey,
    /// The peer is disabled (B02).
    DisabledPeer,
    /// The key belongs to a server under a takeover response, though
    /// it was enrolled after the declaration.
    HostileLineage,
}

/// Why an operation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TakeoverError {
    /// The server is not enrolled.
    UnknownServer(String),
    /// The server is already enrolled.
    DuplicateServer(String),
    /// An incident with this id exists.
    DuplicateIncident(String),
    /// A key compromise named no keys.
    NoKeys,
    /// A key compromise named a key outside the server's lineage.
    KeyNotInLineage(KeyBytes),
    /// The suspected start is after the declaration.
    SuspicionAfterDetection,
    /// New data was refused.
    Refused(Refusal),
    /// After a takeover, a key of that server tried to attest.
    AttestingKeyInHostileLineage,
    /// The key lineage refused an operation (B01).
    Lineage(LifecycleError),
    /// The revocation registry refused an operation (B02).
    Revocation(RevocationError),
}

/// The takeover responder: trusted servers' key lineages, the B02
/// registry of accepted records, each record's object, and the
/// incidents declared.
#[derive(Debug, Clone, Default)]
pub struct TakeoverResponder {
    servers: BTreeMap<String, KeyLineage>,
    registry: RevocationRegistry,
    objects: BTreeMap<String, ObjectId>,
    incidents: Vec<Incident>,
}

impl TakeoverResponder {
    /// An empty responder.
    pub fn new() -> Self {
        Self::default()
    }

    /// The B02 registry.
    pub const fn registry(&self) -> &RevocationRegistry {
        &self.registry
    }

    /// Declared incidents, oldest first.
    pub fn incidents(&self) -> &[Incident] {
        &self.incidents
    }

    /// A server's key lineage.
    pub fn lineage(&self, server: &str) -> Option<&KeyLineage> {
        self.servers.get(server)
    }

    /// Enroll a trusted server with its first key.
    pub fn enroll_server(
        &mut self,
        server: &str,
        key: KeyBytes,
        at: LogicalTime,
    ) -> Result<(), TakeoverError> {
        if self.servers.contains_key(server) {
            return Err(TakeoverError::DuplicateServer(server.to_owned()));
        }
        let mut lineage = KeyLineage::new();
        lineage.enroll(key, at).map_err(TakeoverError::Lineage)?;
        self.servers.insert(server.to_owned(), lineage);
        Ok(())
    }

    /// Rotate a server's key (B01).
    pub fn rotate(
        &mut self,
        server: &str,
        key: KeyBytes,
        at: LogicalTime,
        overlap: LogicalTime,
    ) -> Result<(), TakeoverError> {
        let lineage = self
            .servers
            .get_mut(server)
            .ok_or_else(|| TakeoverError::UnknownServer(server.to_owned()))?;
        lineage
            .rotate(key, at, overlap)
            .map(|_| ())
            .map_err(TakeoverError::Lineage)
    }

    /// Whether data signed by `key` and supplied by `peer_id` may be
    /// accepted or relayed at `at`.
    pub fn decision(&self, key: &KeyBytes, peer_id: &str, at: LogicalTime) -> Result<(), Refusal> {
        match self.registry.admission_decision(key, peer_id, at) {
            Decision::Deny(DenyReason::RevokedKey) => return Err(Refusal::RevokedKey),
            Decision::Deny(DenyReason::DisabledPeer) => return Err(Refusal::DisabledPeer),
            Decision::Allow => {}
        }
        if self.hostile_lineage_holds(key) {
            return Err(Refusal::HostileLineage);
        }
        Ok(())
    }

    /// Accept a record of `object_id`, if [`Self::decision`] allows it.
    /// `issuer_claimed_at` is kept for audit and never read.
    #[allow(clippy::too_many_arguments)]
    pub fn admit(
        &mut self,
        record_id: &str,
        object_id: ObjectId,
        key: KeyBytes,
        peer_id: &str,
        admitted_at: LogicalTime,
        issuer_claimed_at: Option<LogicalTime>,
        signed_bytes: Vec<u8>,
    ) -> Result<(), TakeoverError> {
        self.decision(&key, peer_id, admitted_at)
            .map_err(TakeoverError::Refused)?;
        self.registry
            .admit(
                record_id.to_owned(),
                key,
                peer_id.to_owned(),
                admitted_at,
                0,
                issuer_claimed_at,
                signed_bytes,
            )
            .map_err(TakeoverError::Revocation)?;
        self.objects.insert(record_id.to_owned(), object_id);
        Ok(())
    }

    /// Declare an incident at `now`. Its revocations take effect in this
    /// call; the returned blast radius is the incident query's answer.
    pub fn declare(
        &mut self,
        declaration: Declaration,
        now: LogicalTime,
    ) -> Result<BlastRadius, TakeoverError> {
        let Declaration {
            incident_id,
            server,
            kind,
            suspected_from,
        } = declaration;
        if self.incidents.iter().any(|i| i.id == incident_id) {
            return Err(TakeoverError::DuplicateIncident(incident_id));
        }
        if suspected_from > now {
            return Err(TakeoverError::SuspicionAfterDetection);
        }
        let lineage = self
            .servers
            .get(&server)
            .ok_or_else(|| TakeoverError::UnknownServer(server.clone()))?;
        let all_keys: BTreeSet<KeyBytes> = lineage
            .generations()
            .iter()
            .map(|g| g.verifying_key)
            .collect();
        let (cause, keys) = match kind {
            IncidentKind::KeyCompromise { keys } => {
                if keys.is_empty() {
                    return Err(TakeoverError::NoKeys);
                }
                if let Some(key) = keys.iter().find(|k| !all_keys.contains(*k)) {
                    return Err(TakeoverError::KeyNotInLineage(*key));
                }
                (Cause::KeyCompromise, keys)
            }
            IncidentKind::ServerTakeover => (Cause::ServerTakeover, all_keys),
            IncidentKind::Undetermined => (Cause::Indeterminate, all_keys),
        };
        let reason = if cause.server_hostile() {
            RevocationReason::PeerCompromise
        } else {
            RevocationReason::KeyCompromise
        };
        let peer_subject = cause.server_hostile().then(|| {
            (
                format!("{incident_id}/peer"),
                RevocationSubject::Peer(server.clone()),
            )
        });
        let records: Vec<RevocationRecord> = keys
            .iter()
            .map(|key| {
                (
                    format!("{incident_id}/key/{}", hex(key)),
                    RevocationSubject::Key(*key),
                )
            })
            .chain(peer_subject)
            .map(|(record_id, subject)| RevocationRecord {
                record_id,
                subject,
                effective_from: suspected_from,
                effective_until: None,
                reason,
                recorded_at: now,
            })
            .collect();
        if let Some(clash) = records.iter().find(|r| {
            self.registry
                .revocations()
                .iter()
                .any(|existing| existing.record_id == r.record_id)
        }) {
            return Err(TakeoverError::Revocation(
                RevocationError::DuplicateRevocationId(clash.record_id.clone()),
            ));
        }
        for record in records {
            self.registry
                .apply_revocation(record)
                .map_err(TakeoverError::Revocation)?;
        }
        self.incidents.push(Incident {
            id: incident_id.clone(),
            server,
            cause,
            peer_disabled: cause.server_hostile(),
            keys,
            suspected_from,
            detected_at: now,
        });
        self.blast_radius(&incident_id)
            .ok_or(TakeoverError::DuplicateIncident(incident_id))
    }

    /// The incident query: every accepted record the incident touches,
    /// from key lineage and admission time, with its current status.
    pub fn blast_radius(&self, incident_id: &str) -> Option<BlastRadius> {
        let incident = self.incidents.iter().find(|i| i.id == incident_id)?;
        let records: BTreeMap<String, Exposure> = self
            .registry
            .records()
            .iter()
            .filter(|r| incident.touches(&r.key, &r.peer_id))
            .map(|r| {
                (
                    r.record_id.clone(),
                    Exposure {
                        object_id: self.objects.get(&r.record_id).copied().unwrap_or_default(),
                        admitted_at: r.admitted_at,
                        window: if r.admitted_at >= incident.suspected_from {
                            Window::Suspect
                        } else {
                            Window::Prior
                        },
                        status: r.status.clone(),
                    },
                )
            })
            .collect();
        let objects = records.values().map(|e| e.object_id).collect();
        Some(BlastRadius {
            incident_id: incident.id.clone(),
            cause: incident.cause,
            records,
            objects,
        })
    }

    /// What can be said about a record's historical validity.
    pub fn historical_validity(&self, record_id: &str) -> Option<HistoricalValidity> {
        self.record(record_id)
            .map(|r| r.status.historical_validity())
    }

    /// One accepted record.
    pub fn record(&self, record_id: &str) -> Option<&AcceptedRecord> {
        self.registry.find_record(record_id)
    }

    /// Clear a flagged or quarantined record with explicit evidence. After
    /// a takeover response touching the record, no key of that server's
    /// lineage (including keys enrolled later) may attest for it.
    pub fn revalidate(
        &mut self,
        record_id: &str,
        evidence: &RevalidationEvidence,
        now: LogicalTime,
    ) -> Result<RevalidationMethod, TakeoverError> {
        if let RevalidationEvidence::Reattested { key, .. } = evidence
            && let Some(record) = self.registry.find_record(record_id)
            && self.incidents.iter().any(|incident| {
                incident.cause.server_hostile()
                    && incident.touches(&record.key, &record.peer_id)
                    && self.in_lineage(&incident.server, key)
            })
        {
            return Err(TakeoverError::AttestingKeyInHostileLineage);
        }
        self.registry
            .revalidate(record_id, evidence, now)
            .map_err(TakeoverError::Revocation)
    }

    fn in_lineage(&self, server: &str, key: &KeyBytes) -> bool {
        self.servers
            .get(server)
            .is_some_and(|l| l.generations().iter().any(|g| &g.verifying_key == key))
    }

    fn hostile_lineage_holds(&self, key: &KeyBytes) -> bool {
        self.incidents
            .iter()
            .any(|i| i.cause.server_hostile() && self.in_lineage(&i.server, key))
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}
