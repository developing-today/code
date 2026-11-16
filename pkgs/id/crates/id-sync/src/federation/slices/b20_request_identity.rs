//! B20 — request identity, duplicate-vs-loop classification, finite
//! dedup state and expiry (slice B20, issue #10184; statements
//! S1.64, S1.70–S1.72 of program #9106 part B).
//!
//! Design basis: `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/
//! part-b/versioned-design.md`, sections "Query contract" and
//! "Duplicate/replay/cycle state":
//!
//! - A request ID is derived from the origin key, a fresh origin
//!   nonce, and the canonical immutable query-binding fields, so
//!   independent requests for an identical query/scope/time have
//!   distinct IDs, and any changed binding paired with an old ID is
//!   a stateless [`Classification::BindingConflict`].
//! - A duplicate is a valid repeated request ID: in-flight
//!   duplicates attach to the one work item; completed duplicates
//!   return the deterministic cached outcome and never rerun work.
//! - A replay is a freshness failure: an origin nonce may mint
//!   exactly one request ID; presenting the same nonce under a
//!   different ID is [`Classification::Replay`].
//! - A cycle requires a signed grant-chain path proving the local
//!   peer was already visited; [`DedupLedger::observe_with_path`]
//!   classifies it without registering work.
//! - Dedup state is finite: the ledger is capacity-bounded, live
//!   entries are never evicted, and request-binding tombstones are
//!   retained through origin expiry (plus a finite cleanup grace)
//!   so an expired origin query can never rerun — after expiry the
//!   answer is [`Classification::Expired`], even if the result
//!   cache entry is gone.
//!
//! This module is deliberately not exported from the federation
//! root: slice 9110-02 (#10160) recorded the module path in
//! `B_SLICE_REGISTRY` without exporting it, and slice 9110-09 wires
//! the versioned public exports once the module outputs land. The
//! dedicated integration test includes this file by path.
//!
//! Canonical form (the contract the reference test vectors pin):
//! a binding's canonical bytes are `query_digest` (32 bytes) ‖
//! `scope_len` (u32 big-endian) ‖ `scope` ‖ `issued_at` (u64
//! big-endian) ‖ `expires_at` (u64 big-endian) ‖ `initial_max_hops`
//! (u32 big-endian). The binding digest is
//! `SHA-256("id-atlas/b20/binding/v1" ‖ canonical bytes)` and a
//! request ID is `SHA-256("id-atlas/b20/request-id/v1" ‖ origin
//! (32 bytes) ‖ nonce (32 bytes) ‖ canonical bytes)`.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

/// An origin's public key identity (32-byte Ed25519 key id).
pub type OriginKey = [u8; 32];

/// A fresh origin nonce (32 bytes). One nonce mints one request.
pub type OriginNonce = [u8; 32];

/// A peer identity in a grant-chain path (32 bytes).
pub type PeerKey = [u8; 32];

/// A stable result/outcome identity (32 bytes).
pub type OutcomeId = [u8; 32];

const REQUEST_ID_DOMAIN: &[u8] = b"id-atlas/b20/request-id/v1";
const BINDING_DOMAIN: &[u8] = b"id-atlas/b20/binding/v1";

/// The immutable query-binding fields of an origin query. Every
/// field is committed into both the binding digest and the request
/// ID; changing any of them under an existing request ID is a
/// binding conflict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryBinding {
    /// Digest of the canonical query itself.
    pub query_digest: [u8; 32],
    /// Authorized scope bytes the query runs under.
    pub scope: Vec<u8>,
    /// Origin-signed issue time (seconds, origin clock domain).
    pub issued_at: u64,
    /// Origin-signed expiry (seconds). At and after this time the
    /// query must never (re)run.
    pub expires_at: u64,
    /// Initial hop budget committed by the origin.
    pub initial_max_hops: u32,
}

impl QueryBinding {
    /// The canonical byte encoding defined in the module docs.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(32 + 4 + self.scope.len() + 8 + 8 + 4);
        out.extend_from_slice(&self.query_digest);
        out.extend_from_slice(&(self.scope.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.scope);
        out.extend_from_slice(&self.issued_at.to_be_bytes());
        out.extend_from_slice(&self.expires_at.to_be_bytes());
        out.extend_from_slice(&self.initial_max_hops.to_be_bytes());
        out
    }

    /// `SHA-256` over the domain-separated canonical bytes.
    #[must_use]
    pub fn digest(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(BINDING_DOMAIN);
        hasher.update(self.canonical_bytes());
        hasher.finalize().into()
    }
}

/// A derived request identity: the SHA-256 of the origin key, the
/// origin nonce and the canonical binding, domain-separated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId(pub [u8; 32]);

impl RequestId {
    /// Derive the request ID for `(origin, nonce, binding)`.
    #[must_use]
    pub fn derive(origin: &OriginKey, nonce: &OriginNonce, binding: &QueryBinding) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(REQUEST_ID_DOMAIN);
        hasher.update(origin);
        hasher.update(nonce);
        hasher.update(binding.canonical_bytes());
        Self(hasher.finalize().into())
    }
}

/// An origin query's identity as presented at ingress: the claimed
/// request ID plus every input it must be derived from, so any
/// verifier can recompute the ID statelessly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestIdentity {
    /// Origin key that issued the query.
    pub origin: OriginKey,
    /// Fresh origin nonce for this request.
    pub nonce: OriginNonce,
    /// Immutable query binding.
    pub binding: QueryBinding,
    /// The claimed request ID; must equal the derived ID.
    pub id: RequestId,
}

impl RequestIdentity {
    /// Build an identity, deriving the request ID from its inputs.
    #[must_use]
    pub fn new(origin: OriginKey, nonce: OriginNonce, binding: QueryBinding) -> Self {
        let id = RequestId::derive(&origin, &nonce, &binding);
        Self {
            origin,
            nonce,
            binding,
            id,
        }
    }

    /// Stateless binding check: the claimed ID must be the derived
    /// ID for the presented binding. Any altered field fails.
    #[must_use]
    pub fn binding_is_valid(&self) -> bool {
        RequestId::derive(&self.origin, &self.nonce, &self.binding) == self.id
    }
}

/// The ingress classification of a presented request identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    /// First sight of this request ID: registered as in-flight.
    New,
    /// Valid repeat of an in-flight request: attach to the one
    /// existing work item; no second execution starts.
    DuplicateInFlight,
    /// Valid repeat of a completed request: return the cached
    /// deterministic outcome; the work does not rerun.
    DuplicateCompleted,
    /// Freshness failure: this origin nonce already minted a
    /// different request ID.
    Replay,
    /// The signed grant-chain path proves the local peer was
    /// already visited for this request.
    Cycle,
    /// A changed query binding was presented under an existing
    /// request ID (or the claimed ID does not derive from the
    /// presented inputs at all).
    BindingConflict,
    /// The origin query is at/past expiry: it must never rerun,
    /// whatever the cache still holds.
    Expired,
    /// The finite dedup ledger is full of unexpired entries and
    /// the request cannot be registered without evicting live
    /// state, so it is refused rather than tracked unboundedly.
    LedgerFull,
}

/// Errors from ledger state transitions other than ingress
/// classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DedupError {
    /// Completion was reported for a request the ledger has never
    /// registered (or has already pruned).
    UnknownRequest,
    /// Completion was reported for an in-flight request whose
    /// binding does not match the registered binding.
    BindingMismatch,
    /// A completed request was completed again with a different
    /// outcome ID; the first deterministic outcome stands.
    OutcomeConflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EntryState {
    InFlight,
    Completed(OutcomeId),
    Tombstone,
}

#[derive(Debug, Clone)]
struct Entry {
    binding_digest: [u8; 32],
    expires_at: u64,
    state: EntryState,
}

/// A finite dedup ledger: request-binding entries plus the
/// origin-nonce registry that makes replay detectable. Capacity
/// bounds the number of retained request entries; entries are
/// removable only once they are past origin expiry plus the
/// cleanup grace, so tombstones cover the whole validity window
/// of every request they describe.
#[derive(Debug)]
pub struct DedupLedger {
    capacity: usize,
    cleanup_grace_secs: u64,
    entries: BTreeMap<RequestId, Entry>,
    nonces: BTreeMap<(OriginKey, OriginNonce), RequestId>,
}

impl DedupLedger {
    /// Create a ledger retaining at most `capacity` request
    /// entries, keeping expired entries for `cleanup_grace_secs`
    /// past origin expiry before they become prunable.
    #[must_use]
    pub fn new(capacity: usize, cleanup_grace_secs: u64) -> Self {
        Self {
            capacity,
            cleanup_grace_secs,
            entries: BTreeMap::new(),
            nonces: BTreeMap::new(),
        }
    }

    /// Number of retained request entries (live and tombstoned).
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the ledger retains no request entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Classify a presented identity at time `now`, registering
    /// it as in-flight when it is new. See [`Classification`] for
    /// the verdict order: binding, expiry, duplicate, replay,
    /// capacity.
    pub fn observe(&mut self, identity: &RequestIdentity, now: u64) -> Classification {
        if !identity.binding_is_valid() {
            return Classification::BindingConflict;
        }
        let binding_digest = identity.binding.digest();
        if let Some(entry) = self.entries.get_mut(&identity.id) {
            if entry.binding_digest != binding_digest {
                return Classification::BindingConflict;
            }
            if now >= entry.expires_at {
                entry.state = EntryState::Tombstone;
                return Classification::Expired;
            }
            return match entry.state {
                EntryState::InFlight => Classification::DuplicateInFlight,
                EntryState::Completed(_) => Classification::DuplicateCompleted,
                EntryState::Tombstone => Classification::Expired,
            };
        }
        if now >= identity.binding.expires_at {
            return Classification::Expired;
        }
        if let Some(existing) = self.nonces.get(&(identity.origin, identity.nonce)) {
            if *existing != identity.id {
                return Classification::Replay;
            }
        }
        if self.entries.len() >= self.capacity {
            self.prune(now);
            if self.entries.len() >= self.capacity {
                return Classification::LedgerFull;
            }
        }
        self.entries.insert(
            identity.id,
            Entry {
                binding_digest,
                expires_at: identity.binding.expires_at,
                state: EntryState::InFlight,
            },
        );
        self.nonces
            .insert((identity.origin, identity.nonce), identity.id);
        Classification::New
    }

    /// Classify a presented identity that arrived with a signed
    /// grant-chain `path`: if `local_peer` already appears in the
    /// path, the request has looped and is classified
    /// [`Classification::Cycle`] without registering work;
    /// otherwise the verdict is exactly [`Self::observe`].
    pub fn observe_with_path(
        &mut self,
        identity: &RequestIdentity,
        path: &[PeerKey],
        local_peer: &PeerKey,
        now: u64,
    ) -> Classification {
        if !identity.binding_is_valid() {
            return Classification::BindingConflict;
        }
        if now < identity.binding.expires_at && path.contains(local_peer) {
            return Classification::Cycle;
        }
        self.observe(identity, now)
    }

    /// Record the deterministic outcome of an in-flight request.
    /// Completing twice with the same outcome is idempotent;
    /// completing with a different outcome is an
    /// [`DedupError::OutcomeConflict`] and the first outcome
    /// stands.
    pub fn complete(
        &mut self,
        identity: &RequestIdentity,
        outcome: OutcomeId,
    ) -> Result<(), DedupError> {
        let entry = self
            .entries
            .get_mut(&identity.id)
            .ok_or(DedupError::UnknownRequest)?;
        if entry.binding_digest != identity.binding.digest() {
            return Err(DedupError::BindingMismatch);
        }
        match entry.state {
            EntryState::InFlight => {
                entry.state = EntryState::Completed(outcome);
                Ok(())
            }
            EntryState::Completed(existing) if existing == outcome => Ok(()),
            EntryState::Completed(_) | EntryState::Tombstone => Err(DedupError::OutcomeConflict),
        }
    }

    /// The cached deterministic outcome for a completed request,
    /// if one is retained. This is the only outcome a duplicate
    /// may ever be answered with; `None` never licenses a rerun
    /// of an expired request (see [`Self::observe`]).
    #[must_use]
    pub fn cached_outcome(&self, id: &RequestId) -> Option<OutcomeId> {
        match self.entries.get(id) {
            Some(Entry {
                state: EntryState::Completed(outcome),
                ..
            }) => Some(*outcome),
            _ => None,
        }
    }

    /// Remove entries whose origin expiry plus the cleanup grace
    /// has passed at `now`, together with their nonce mappings.
    /// Live (unexpired) entries are never removed. Returns the
    /// number of entries removed.
    pub fn prune(&mut self, now: u64) -> usize {
        let grace = self.cleanup_grace_secs;
        let doomed: Vec<RequestId> = self
            .entries
            .iter()
            .filter(|(_, entry)| now >= entry.expires_at.saturating_add(grace))
            .map(|(id, _)| *id)
            .collect();
        let removed = doomed.len();
        for id in doomed {
            self.entries.remove(&id);
            self.nonces.retain(|_, mapped| *mapped != id);
        }
        removed
    }
}
