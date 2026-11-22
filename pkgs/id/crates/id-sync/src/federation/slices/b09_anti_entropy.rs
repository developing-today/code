//! B09: divergence detection and anti-entropy (#10262, statements
//! S1.32-S1.40; Part B "Component dispositions and statement
//! coverage", item 2).
//!
//! Internal peers sync toward sameness of the versions they are both
//! authorized to hold. This slice makes that verifiable and moves the
//! missing versions, and nothing else:
//!
//! 1. **Divergence is detectable (S1.32).** A replica summarizes what it
//!    holds as an [`Inventory`]: for each object, the event ids it has
//!    stored or is holding for missing parents, and a domain-separated
//!    digest of that set. Two replicas compare digests first and list
//!    ids only for the objects whose digests differ, so the difference
//!    is named exactly, version by version ([`ObjectDivergence`]).
//! 2. **Anti-entropy moves signed versions, never merges (S1.33,
//!    S1.35).** Reconciliation sends each side the original signed
//!    envelopes the other is missing, in ascending event-id order, and
//!    the receiver admits every one through its B07 [`ConflictStore`],
//!    and so through the B32 gate, exactly as it would any other peer's
//!    version. Reconciliation never creates, edits or resolves a
//!    version. Two branches that meet on a replica become a conflict
//!    report there (S1.34), and a merge exists only where a writer
//!    signed one; B07 accepts or refuses it the same way whatever order
//!    the versions arrived in, so the order of reconciliation cannot
//!    merge anything silently.
//! 3. **Only what may cross, crosses (B08).** Each version is checked
//!    against the link's [`SyncScopeConfig`] as a B08 [`ScopedRecord`]
//!    (the envelope's scope, the replica's trust domain, the time the
//!    replica admitted it). A version outside the scope, the trust
//!    domain or the retention bound is withheld and reported, never
//!    sent.
//! 4. **Refusals are final and remembered.** A version the receiver
//!    refused (by its gate or its store, including one dropped when its
//!    parents arrived) is recorded with the reason and never offered to
//!    that replica again; the divergence stays visible and tolerated.
//! 5. **Unresolved conflicts never block unrelated sync (S1.38).**
//!    Nothing here reads reports: every object is reconciled in the
//!    same exchange, conflicted or not.
//!
//! Merging is left to later work (S1.39), and because reconciliation
//! only carries signed versions into a report-first store, the conflict
//! report stays the first output of any conflict even once a merge
//! mechanism is adopted (S1.40).
//!
//! The inventory digest of one object is SHA-256 over
//!
//! ```text
//! "id-sync/b09-inventory/v1"
//! object_id               32 bytes
//! count                   u64, big-endian
//! event ids               count x 32 bytes, strictly ascending
//! ```
//!
//! Rollback is pausing reconciliation: replicas keep their versions,
//! reports and refusals, and nothing here deletes any of them. This
//! slice keeps no tombstones; deletion is B08's scoped event.
//!
//! This module is deliberately self-contained: the federation roots
//! record slice paths without declaring them (slice 9110-02), so the
//! slice's integration test includes this file directly and no shared
//! root names it yet; #9110-09 wires versioned public exports after
//! module outputs land.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use super::b07_conflict_report::{
    ConflictStore, Digest32, IngestError, IngestOutcome, ObjectId, VersionHeader,
};
use super::b08_sync_scope::{Exclusion, ObjectKind, ScopedRecord, SyncScopeConfig, exclusion};
use super::b32_ingest_gate::{Envelope, EventId, IngressPath, LogicalTime};

/// Domain prefix of an object's inventory digest.
pub const INVENTORY_DOMAIN: &[u8] = b"id-sync/b09-inventory/v1";

/// The digest of one object's version set, as documented above.
pub fn object_digest(object_id: &ObjectId, versions: &BTreeSet<EventId>) -> Digest32 {
    let mut hasher = Sha256::new();
    hasher.update(INVENTORY_DOMAIN);
    hasher.update(object_id);
    hasher.update((versions.len() as u64).to_be_bytes());
    for id in versions {
        hasher.update(id);
    }
    hasher.finalize().into()
}

/// What one replica holds: for each object, the versions it has
/// stored or is holding for missing parents.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inventory {
    objects: BTreeMap<ObjectId, BTreeSet<EventId>>,
}

impl Inventory {
    /// An inventory built from a peer's report of what it holds, as
    /// received over the wire (B2(c), #12232): the same sets
    /// [`Replica::inventory`] builds locally, from the other side's
    /// telling. Divergence and offers computed against it are the
    /// in-process ones; only the source of the sets differs.
    pub fn from_objects(objects: BTreeMap<ObjectId, BTreeSet<EventId>>) -> Self {
        Self { objects }
    }

    /// The versions held for one object.
    pub fn versions(&self, object_id: &ObjectId) -> Option<&BTreeSet<EventId>> {
        self.objects.get(object_id)
    }

    /// Every object held, ascending.
    pub fn objects(&self) -> impl Iterator<Item = &ObjectId> {
        self.objects.keys()
    }

    /// Whether any object holds this version.
    pub fn contains(&self, event_id: &EventId) -> bool {
        self.objects.values().any(|ids| ids.contains(event_id))
    }

    /// The digest of every object: what two replicas exchange first.
    pub fn summary(&self) -> BTreeMap<ObjectId, Digest32> {
        self.objects
            .iter()
            .map(|(object_id, ids)| (*object_id, object_digest(object_id, ids)))
            .collect()
    }
}

/// The objects whose digests differ between two summaries, including
/// objects only one side holds, ascending.
pub fn diverged_objects(
    local: &BTreeMap<ObjectId, Digest32>,
    remote: &BTreeMap<ObjectId, Digest32>,
) -> Vec<ObjectId> {
    local
        .keys()
        .chain(remote.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|object_id| local.get(*object_id) != remote.get(*object_id))
        .copied()
        .collect()
}

/// One object whose version sets differ, named version by version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectDivergence {
    /// The object.
    pub object_id: ObjectId,
    /// Versions the remote holds and the local replica does not.
    pub missing_local: Vec<EventId>,
    /// Versions the local replica holds and the remote does not.
    pub missing_remote: Vec<EventId>,
}

/// Every divergence between two inventories, by object, ascending. Only
/// objects whose digests differ are compared id by id.
pub fn divergence(local: &Inventory, remote: &Inventory) -> Vec<ObjectDivergence> {
    let empty = BTreeSet::new();
    diverged_objects(&local.summary(), &remote.summary())
        .into_iter()
        .map(|object_id| {
            let ours = local.versions(&object_id).unwrap_or(&empty);
            let theirs = remote.versions(&object_id).unwrap_or(&empty);
            ObjectDivergence {
                object_id,
                missing_local: theirs.difference(ours).copied().collect(),
                missing_remote: ours.difference(theirs).copied().collect(),
            }
        })
        .collect()
}

/// Why a replica refused a version. Refusals are final.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The store (and its gate) refused the envelope on arrival.
    Ingest(IngestError),
    /// The store held the version for its parents and refused it when
    /// they arrived.
    DroppedWhileHeld,
}

/// A version this replica keeps, with what reconciliation needs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Kept {
    object_id: ObjectId,
    envelope: Envelope,
    admitted_at: LogicalTime,
}

/// One node's B07 store, with the signed envelopes it can resend and
/// the versions it refused.
#[derive(Debug, Clone)]
pub struct Replica {
    name: String,
    trust_domain: String,
    store: ConflictStore,
    kept: BTreeMap<EventId, Kept>,
    refused: BTreeMap<EventId, Refusal>,
}

impl Replica {
    /// A replica called `name` in `trust_domain`, over `store`.
    pub fn new(
        name: impl Into<String>,
        trust_domain: impl Into<String>,
        store: ConflictStore,
    ) -> Self {
        Self {
            name: name.into(),
            trust_domain: trust_domain.into(),
            store,
            kept: BTreeMap::new(),
            refused: BTreeMap::new(),
        }
    }

    /// The replica's name, used as the source label on what it sends.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The replica's B07 store.
    pub const fn store(&self) -> &ConflictStore {
        &self.store
    }

    /// The versions this replica refused, and why.
    pub const fn refused(&self) -> &BTreeMap<EventId, Refusal> {
        &self.refused
    }

    /// The signed envelope of a version this replica keeps.
    pub fn envelope(&self, event_id: &EventId) -> Option<&Envelope> {
        self.kept.get(event_id).map(|kept| &kept.envelope)
    }

    /// Admit a version a local producer signed.
    pub fn produce(
        &mut self,
        envelope: &Envelope,
        now: LogicalTime,
    ) -> Result<IngestOutcome, IngestError> {
        let source = self.name.clone();
        self.receive(IngressPath::Local, envelope, &source, now)
    }

    /// Admit one envelope through the store, keeping it if the store
    /// stored or held it and recording the refusal otherwise.
    pub fn receive(
        &mut self,
        path: IngressPath,
        envelope: &Envelope,
        source: &str,
        now: LogicalTime,
    ) -> Result<IngestOutcome, IngestError> {
        let event_id = envelope.event_id;
        let result = self.store.ingest(path, envelope, source, now);
        match &result {
            Ok(_) => {
                if let Some(header) = VersionHeader::decode(&envelope.payload) {
                    self.kept.insert(
                        event_id,
                        Kept {
                            object_id: header.object_id,
                            envelope: envelope.clone(),
                            admitted_at: now,
                        },
                    );
                }
            }
            Err(error) => {
                if !self.kept.contains_key(&event_id) {
                    self.refused
                        .entry(event_id)
                        .or_insert_with(|| Refusal::Ingest(error.clone()));
                }
            }
        }
        self.sweep();
        result
    }

    /// The versions this replica holds, by object.
    pub fn inventory(&self) -> Inventory {
        let mut objects: BTreeMap<ObjectId, BTreeSet<EventId>> = BTreeMap::new();
        for (event_id, kept) in &self.kept {
            objects.entry(kept.object_id).or_default().insert(*event_id);
        }
        Inventory { objects }
    }

    /// What this replica would send a remote holding `remote` and having
    /// refused `remote_refused`, under `config` at `now`.
    pub fn offer(
        &self,
        remote: &Inventory,
        remote_refused: &BTreeSet<EventId>,
        config: &SyncScopeConfig,
        now: LogicalTime,
    ) -> Offer {
        let mut offer = Offer::default();
        for diverged in divergence(&self.inventory(), remote) {
            for event_id in diverged.missing_remote {
                if remote_refused.contains(&event_id) {
                    offer.refused_by_remote.push(event_id);
                    continue;
                }
                let Some(kept) = self.kept.get(&event_id) else {
                    continue;
                };
                match exclusion(
                    &self.scoped_record(&event_id, kept),
                    ObjectKind::Content,
                    config,
                    now,
                ) {
                    None => offer.send.push(event_id),
                    Some(excluded) => offer.withheld.push((event_id, excluded)),
                }
            }
        }
        offer.send.sort_unstable();
        offer.withheld.sort_unstable();
        offer.refused_by_remote.sort_unstable();
        offer
    }

    fn scoped_record(&self, event_id: &EventId, kept: &Kept) -> ScopedRecord {
        ScopedRecord::live(
            hex(event_id),
            kept.envelope.scope.clone(),
            self.trust_domain.clone(),
            hex(&kept.envelope.payload_digest),
            kept.admitted_at,
        )
    }

    /// Move to `refused` every kept version the store no longer stores
    /// or holds: it was refused when its parents arrived.
    fn sweep(&mut self) {
        let dropped: Vec<EventId> = self
            .kept
            .keys()
            .filter(|id| self.store.version(id).is_none() && !self.store.is_held(id))
            .copied()
            .collect();
        for id in dropped {
            self.kept.remove(&id);
            self.refused.insert(id, Refusal::DroppedWhileHeld);
        }
    }
}

/// What one side of an exchange would send.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Offer {
    /// Versions to send, ascending.
    pub send: Vec<EventId>,
    /// Versions the link's bounds withhold, with the B08 reason.
    pub withheld: Vec<(EventId, Exclusion)>,
    /// Versions not sent because the remote refused them before.
    pub refused_by_remote: Vec<EventId>,
}

/// One direction of an exchange: what was offered and what each sent
/// version came to at the receiver.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Transfer {
    /// The offer this direction made.
    pub offer: Offer,
    /// The receiver's result for each sent version, in send order.
    pub outcomes: Vec<(EventId, Result<IngestOutcome, IngestError>)>,
}

/// The result of reconciling two replicas.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Exchange {
    /// The divergence detected before anything moved, from the first
    /// replica's side.
    pub divergence: Vec<ObjectDivergence>,
    /// From the first replica to the second.
    pub forward: Transfer,
    /// From the second replica to the first.
    pub backward: Transfer,
}

impl Exchange {
    /// Whether nothing was sent either way.
    pub const fn is_quiet(&self) -> bool {
        self.forward.offer.send.is_empty() && self.backward.offer.send.is_empty()
    }
}

/// Reconcile two replicas over one link: detect the divergence, then
/// send each side what it is missing and may receive. Both offers are
/// made from the inventories as they stood before either transfer.
pub fn reconcile(
    first: &mut Replica,
    second: &mut Replica,
    config: &SyncScopeConfig,
    now: LogicalTime,
) -> Exchange {
    let first_inventory = first.inventory();
    let second_inventory = second.inventory();
    let first_refused: BTreeSet<EventId> = first.refused.keys().copied().collect();
    let second_refused: BTreeSet<EventId> = second.refused.keys().copied().collect();
    let forward = first.offer(&second_inventory, &second_refused, config, now);
    let backward = second.offer(&first_inventory, &first_refused, config, now);
    Exchange {
        divergence: divergence(&first_inventory, &second_inventory),
        forward: transfer(first, second, forward, now),
        backward: transfer(second, first, backward, now),
    }
}

fn transfer(sender: &Replica, receiver: &mut Replica, offer: Offer, now: LogicalTime) -> Transfer {
    let outcomes = offer
        .send
        .iter()
        .filter_map(|event_id| {
            let envelope = sender.envelope(event_id)?;
            let result = receiver.receive(IngressPath::Internal, envelope, sender.name(), now);
            Some((*event_id, result))
        })
        .collect();
    Transfer { offer, outcomes }
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}
