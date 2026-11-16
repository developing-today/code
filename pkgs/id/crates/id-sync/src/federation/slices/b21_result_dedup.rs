//! B21 — short-circuit and deduplicate in-flight/answered requests
//! into coherent results (slice B21, issue #10210; statements
//! S1.67–S1.68 of program #9106 part B).
//!
//! Design basis: `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/
//! part-b/versioned-design.md`, section "Duplicate/replay/cycle
//! state":
//!
//! - An in-flight duplicate **attaches** to the one existing work
//!   item; it never starts a second execution.
//! - A completed duplicate returns the deterministic cached
//!   outcome, reauthorized under current policy before release; a
//!   revoked/disabled responder's cached result is not disclosed.
//! - Results deduplicate on their **stable result ID** while
//!   retaining every responder's provenance proof: attaching a
//!   second provenance for an ID adds it to the entry, a different
//!   body digest under the same ID is a contradiction and is
//!   refused.
//! - If the result cache was evicted while the origin request is
//!   still valid, the retained ledger permits only rehydration or
//!   resumption against the same reservation and remaining
//!   counters — never a new reservation and never fresh execution
//!   of completed work. If the outcome cannot be rehydrated, the
//!   answer is [`ShortCircuit::ResultEvicted`] without rerun.
//! - After origin expiry the query must not rerun; the ledger
//!   answer is [`ShortCircuit::Expired`].
//!
//! This module is deliberately not exported from the federation
//! root: slice 9110-02 (#10160) recorded the module path in
//! `B_SLICE_REGISTRY` without exporting it, and slice 9110-09 wires
//! the versioned public exports once the module outputs land. The
//! dedicated integration test includes this file by path.
//!
//! Canonical form (the contract the reference test vectors pin):
//! a result entry's canonical bytes are `result_id` (32 bytes) ‖
//! `body_digest` (32 bytes) ‖ `provenance_count` (u32 big-endian) ‖
//! for each responder in ascending key order:
//! `responder` (32 bytes) ‖ `proof_len` (u32 big-endian) ‖ `proof`.
//! The coherent-set digest is
//! `SHA-256("id-atlas/b21/result-set/v1" ‖ entries in ascending
//! result-id order, each as its canonical bytes)`.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

/// A peer identity (32-byte Ed25519 key id).
pub type PeerKey = [u8; 32];

/// A stable result identity (32 bytes). Deduplication key.
pub type ResultId = [u8; 32];

/// A digests-only stable reservation identity (32 bytes). Retries
/// reuse it; it is never reminted for the same work item.
pub type ReservationId = [u8; 32];

/// A stable request identity (32 bytes), matching B20's derived
/// `RequestId` bytes.
pub type RequestId = [u8; 32];

const RESULT_SET_DOMAIN: &[u8] = b"id-atlas/b21/result-set/v1";

/// The semantic outcome of offering a result into the coherent set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Registration {
    /// The result ID was new and its entry was recorded.
    New,
    /// The result ID was already present with the same body digest;
    /// this responder's provenance proof was added (or was already
    /// present) and the entry is unchanged semantically.
    Merged,
    /// The result ID was already present with a different body
    /// digest: a contradiction. The first body stands and nothing
    /// is recorded.
    ConflictingBody,
}

/// Errors from short-circuit state transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DedupError {
    /// A completion was reported for work the ledger has never
    /// registered (or has already pruned).
    UnknownWork,
    /// A completion was reported for a work item whose reservation
    /// is not the one it was registered under.
    ReservationMismatch,
    /// A completed work item was completed again with a different
    /// outcome; the first deterministic outcome stands.
    OutcomeConflict,
}

/// One deduplicated result: the stable ID, the single canonical
/// body digest, and every distinct responder's provenance proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultEntry {
    /// Stable result identity (dedup key).
    pub result_id: ResultId,
    /// Digest of the canonical result body the ID names.
    pub body_digest: [u8; 32],
    /// Provenance proofs, one per responder that attested this
    /// result. Retained, never replaced.
    pub provenance: BTreeMap<PeerKey, Vec<u8>>,
}

impl ResultEntry {
    /// The canonical byte encoding defined in the module docs.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(32 + 32 + 4 + self.provenance.len() * 64);
        out.extend_from_slice(&self.result_id);
        out.extend_from_slice(&self.body_digest);
        out.extend_from_slice(&(self.provenance.len() as u32).to_be_bytes());
        for (responder, proof) in &self.provenance {
            out.extend_from_slice(responder);
            out.extend_from_slice(&(proof.len() as u32).to_be_bytes());
            out.extend_from_slice(proof);
        }
        out
    }
}

/// A coherent result set: results deduplicated on stable ID, each
/// retaining every provenance proof that attested it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResultSet {
    entries: BTreeMap<ResultId, ResultEntry>,
}

impl ResultSet {
    /// An empty coherent result set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of distinct results retained.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the set holds no results.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entry for `id`, if retained.
    #[must_use]
    pub fn get(&self, id: &ResultId) -> Option<&ResultEntry> {
        self.entries.get(id)
    }

    /// Offer a result body with one responder's provenance proof.
    /// See [`Registration`] for the verdict order.
    pub fn register(
        &mut self,
        result_id: ResultId,
        body_digest: [u8; 32],
        responder: PeerKey,
        proof: Vec<u8>,
    ) -> Registration {
        match self.entries.get_mut(&result_id) {
            Some(entry) => {
                if entry.body_digest != body_digest {
                    return Registration::ConflictingBody;
                }
                entry.provenance.entry(responder).or_insert(proof);
                Registration::Merged
            }
            None => {
                let mut provenance = BTreeMap::new();
                provenance.insert(responder, proof);
                self.entries
                    .insert(result_id, ResultEntry {
                        result_id,
                        body_digest,
                        provenance,
                    });
                Registration::New
            }
        }
    }

    /// Entries in ascending result-id order (canonical order).
    #[must_use]
    pub fn entries(&self) -> impl Iterator<Item = &ResultEntry> {
        self.entries.values()
    }

    /// The coherent-set digest defined in the module docs.
    #[must_use]
    pub fn digest(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(RESULT_SET_DOMAIN);
        for entry in self.entries.values() {
            hasher.update(entry.canonical_bytes());
        }
        hasher.finalize().into()
    }
}

/// The outcome of presenting a request to the coordinator: what the
/// caller must do instead of unconditionally executing work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortCircuit {
    /// First sight: start work once under `reservation`. Every
    /// later in-flight duplicate attaches to this same item.
    NewWork { reservation: ReservationId },
    /// A valid in-flight duplicate: attach to the existing work
    /// item; `attachments` is how many duplicates now share it.
    Attached {
        reservation: ReservationId,
        attachments: u32,
    },
    /// A completed duplicate: return these cached results,
    /// reauthorized under current policy. No work reruns.
    Cached { results: ResultSet },
    /// The work completed but its result cache was evicted while
    /// the origin request is still valid, and rehydration under the
    /// same reservation is impossible: report eviction, never
    /// rerun completed work.
    ResultEvicted { reservation: ReservationId },
    /// At/past origin expiry: the query must not rerun, whatever
    /// the cache holds.
    Expired,
    /// Releasing the cached results would disclose them to or from
    /// a responder no longer authorized by current policy.
    ReauthorizationDenied,
    /// The coordinator is full of unexpired items, so new work
    /// cannot be registered without evicting live ledger state; the
    /// caller must not start untracked work.
    LedgerFull,
}

#[derive(Debug, Clone)]
enum WorkState {
    InFlight { attachments: u32 },
    Completed { results: ResultSet },
    /// Completed, then the result cache was evicted; the ledger
    /// entry survives through origin expiry so eviction is reported
    /// rather than papered over with a rerun.
    Evicted,
}

#[derive(Debug, Clone)]
struct WorkItem {
    reservation: ReservationId,
    expires_at: u64,
    state: WorkState,
}

/// Short-circuits repeated requests onto one work item and keeps the
/// resulting coherent sets, bounded by `capacity` retained work
/// items. Live (unexpired) items are never evicted.
#[derive(Debug)]
pub struct RequestCoordinator {
    capacity: usize,
    items: BTreeMap<RequestId, WorkItem>,
}

impl RequestCoordinator {
    /// A coordinator retaining at most `capacity` work items.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            items: BTreeMap::new(),
        }
    }

    /// Number of retained work items (live, completed, evicted).
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the coordinator retains no work items.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Present a request at `now`. `reservation` is the reservation
    /// the caller intended to use; on [`ShortCircuit::Attached`] and
    /// [`ShortCircuit::ResultEvicted`] the reservation that must be
    /// reused (never a fresh one) is returned instead.
    pub fn observe(
        &mut self,
        request: RequestId,
        reservation: ReservationId,
        expires_at: u64,
        now: u64,
    ) -> ShortCircuit {
        if let Some(item) = self.items.get_mut(&request) {
            if now >= item.expires_at {
                return ShortCircuit::Expired;
            }
            return match &mut item.state {
                WorkState::InFlight { attachments } => {
                    *attachments = attachments.saturating_add(1);
                    ShortCircuit::Attached {
                        reservation: item.reservation,
                        attachments: *attachments,
                    }
                }
                WorkState::Completed { results } => ShortCircuit::Cached {
                    results: results.clone(),
                },
                WorkState::Evicted => ShortCircuit::ResultEvicted {
                    reservation: item.reservation,
                },
            };
        }
        if now >= expires_at {
            return ShortCircuit::Expired;
        }
        if self.items.len() >= self.capacity {
            self.prune(now);
            if self.items.len() >= self.capacity {
                // Refuse rather than evict live ledger state; the
                // caller must not start untracked work.
                return ShortCircuit::LedgerFull;
            }
        }
        self.items.insert(
            request,
            WorkItem {
                reservation,
                expires_at,
                state: WorkState::InFlight { attachments: 0 },
            },
        );
        ShortCircuit::NewWork { reservation }
    }

    /// Record the completed coherent result set for `request`.
    /// Idempotent for the same set; a different set under the same
    /// reservation is [`DedupError::OutcomeConflict`].
    pub fn complete(
        &mut self,
        request: RequestId,
        reservation: ReservationId,
        results: ResultSet,
    ) -> Result<(), DedupError> {
        let item = self
            .items
            .get_mut(&request)
            .ok_or(DedupError::UnknownWork)?;
        if item.reservation != reservation {
            return Err(DedupError::ReservationMismatch);
        }
        match &item.state {
            WorkState::InFlight { .. } => {
                item.state = WorkState::Completed { results };
                Ok(())
            }
            WorkState::Completed { results: existing } if *existing == results => Ok(()),
            WorkState::Completed { .. } => Err(DedupError::OutcomeConflict),
            WorkState::Evicted => Err(DedupError::OutcomeConflict),
        }
    }

    /// Mark a completed item's result cache as evicted while the
    /// request remains valid. The ledger entry survives so later
    /// duplicates get [`ShortCircuit::ResultEvicted`], never a
    /// rerun. Returns whether the item was in a completed state.
    pub fn evict_results(&mut self, request: &RequestId, now: u64) -> bool {
        let Some(item) = self.items.get_mut(request) else {
            return false;
        };
        if now >= item.expires_at {
            return false;
        }
        if matches!(item.state, WorkState::Completed { .. }) {
            item.state = WorkState::Evicted;
            return true;
        }
        false
    }

    /// Remove items whose expiry has passed at `now`. Live items are
    /// never removed. Returns the number removed.
    pub fn prune(&mut self, now: u64) -> usize {
        let before = self.items.len();
        self.items.retain(|_, item| now < item.expires_at);
        before - self.items.len()
    }
}

/// A policy gate applied to cached results before release: a
/// responder the gate rejects means the cached result may not be
/// disclosed. The check is per responder and applied to every
/// provenance proof in the set.
pub fn reauthorize<F>(results: &ResultSet, mut allowed: F) -> bool
where
    F: FnMut(&PeerKey) -> bool,
{
    results
        .entries()
        .all(|entry| entry.provenance.keys().all(&mut allowed))
}

impl ShortCircuit {
    /// Apply [`reauthorize`] to a cached verdict: a cached set the
    /// current policy would expose an unauthorized responder through
    /// becomes [`ShortCircuit::ReauthorizationDenied`].
    #[must_use]
    pub fn reauthorized<F>(self, allowed: F) -> Self
    where
        F: FnMut(&PeerKey) -> bool,
    {
        match self {
            Self::Cached { results } => {
                if reauthorize(&results, allowed) {
                    Self::Cached { results }
                } else {
                    Self::ReauthorizationDenied
                }
            }
            other => other,
        }
    }
}
