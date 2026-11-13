//! Issuer key rotation and compromise recovery as signed ID-graph
//! objects (design #2804; implementation issue #5526).
//!
//! A currency issuer's identity is its generation-0 key, committed by
//! the currency definition. Rotation never edits that definition: it
//! is a signed event on the issuer's chain (kind
//! `issuer.rotation`) naming the successor key, and compromise
//! recovery is a distinct rotation mode authorised by a *separate*
//! recovery key whose hash the definition committed at creation
//! (`recovery_commitment`). This module is the protocol core of the
//! #2804 design — the versioned rotation object, the successor
//! proof, and the generation-chain fold:
//!
//! - **Routine rotation** is authored by the current generation key
//!   and countersigned by the successor (the successor proof), so a
//!   routine transition proves control of both keys (§3.2 rule 2).
//! - **Compromise recovery** is authored by the precommitted recovery
//!   key — never by the possibly-stolen generation key — and is
//!   countersigned by the successor (§3.2 rule 3). A declaration
//!   signed only by the current minting key cannot select a
//!   successor or change authoritative generation state (§3.4): in
//!   this module such an object simply fails validation and the fold
//!   ignores it.
//! - Generations are consecutive and predecessor-linked; two valid
//!   transitions claiming the same predecessor **conflict** and
//!   neither settles (§3.2 rule 5) — the fold reports
//!   [`ChainStatus::Conflicted`] instead of picking one by arrival
//!   order or timestamp, neither of which is a graph ordering rule.
//! - A successor key is only authorised for events that causally
//!   reference its rotation event ([`references_rotation`], §3.2
//!   rule 4).
//!
//! The rotation record travels in the signed header extension
//! ([`crate::chain::ChainExt::rotation`]), not in an event body: the
//! event store persists the signed header but not arbitrary body
//! content, so a body-carried rotation would be unrecoverable from
//! stored or synchronized events (design §3.2, §1). A decoded event
//! therefore carries its whole rotation, and any observer can run
//! the fold from received events alone.
//!
//! Following the [`crate::currency`] precedent, state is a fold over
//! the signed objects alone ([`fold_rotation_chain`]): callers obtain
//! authors and rotations from decoded events ([`author_id`],
//! `header.extensions.rotation`) — [`crate::chain`] has already
//! verified every envelope signature before this module sees an
//! object — and supply the definition's recovery-key commitment,
//! exactly as `fold_authorisation` takes the expected issuer. The
//! commitment is an input because no currency-definition type exists
//! in `pkgs/id` yet (#2251 owns that format); the design §10
//! question defaults to Option A (anchored definitions precommit a
//! recovery key) unless Drew rules otherwise.
//!
//! Out of scope here, per the design's phased plan (§7 phases 3–6):
//! counting-view pinning (#2030), the anchored registry/gateway
//! admission barrier (#2062), reconciliation, and wallet adoption.
//! Those layers consume this fold; they do not change it.
//!
//! Library-only: no gateway, registry, or control-plane code lives
//! here.

use ed25519_dalek::{Signature, VerifyingKey};
use p2panda_core::{Hash, SeqNum, SigningKey};
use serde::{Deserialize, Serialize};

use crate::chain::{self, ChainExt, Event};

/// The event kind carrying an issuer rotation body.
/// An issuer identity: the 32 bytes of its signing key's verifying
/// key, the same bytes the event store uses as an author key.
///
/// Moved here from the facade's `currency` module in step 2a of the crate
/// split (#4405): the rotation record names issuers, so the identity shape
/// is rotation vocabulary. `currency` re-exports it at its historical path.
pub type IssuerId = [u8; 32];

/// The issuer id of a signing key.
#[must_use]
pub fn issuer_id(key: &SigningKey) -> IssuerId {
    *key.verifying_key().as_bytes()
}

/// The author of a decoded (signature-verified) event, as an issuer id.
#[must_use]
pub fn author_id(event: &Event) -> IssuerId {
    *event.header.verifying_key.as_bytes()
}

pub const ISSUER_ROTATION_KIND: &str = "issuer.rotation";

/// Whether `kind` is the issuer rotation kind: an event of this
/// kind must carry a rotation in its extension, and no other kind
/// may ([`crate::chain::ChainExt::validate`]).
#[must_use]
pub fn is_rotation_kind(kind: &str) -> bool {
    kind == ISSUER_ROTATION_KIND
}

/// Checks a rotation body against the kind of the event that carries
/// it, running only the body-only stateless subset of
/// [`IssuerRotation::validate`].
///
/// [`ChainExt::validate`](crate::chain::ChainExt) — which knows the
/// event's kind but neither its author nor any currency definition —
/// calls this at the header boundary, so a malformed rotation body is
/// refused at sign/decode instead of signing, storing, and syncing as
/// a fully valid event (#5706). The context-dependent rules stay out
/// of this check by design: that a routine author is the *current*
/// generation key, and that a recovery author is the key the
/// definition committed (and not the key it replaces), need the
/// event's verified author and the definition's recovery commitment,
/// which only [`IssuerRotation::validate`] and [`fold_rotation_chain`]
/// have.
///
/// # Errors
///
/// Returns [`RotationError::KindMismatch`] when the carrying event's
/// kind is not [`ISSUER_ROTATION_KIND`]. Returns the body-rule error
/// naming the first failed check otherwise: non-consecutive
/// generations, the predecessor rule, an asserted compromise start on
/// a routine rotation, or a successor proof that does not verify.
pub fn check_rotation(kind: &str, rotation: &IssuerRotation) -> Result<(), RotationError> {
    if !is_rotation_kind(kind) {
        return Err(RotationError::KindMismatch {
            kind: kind.to_owned(),
        });
    }
    rotation.check_shape()?;
    if rotation.mode == RotationMode::Routine && rotation.asserted_compromise_from.is_some() {
        return Err(RotationError::AssertedCompromiseOnRoutine);
    }
    if !rotation.verify_successor_proof() {
        return Err(RotationError::SuccessorProofInvalid);
    }
    Ok(())
}

/// Domain separator for the successor proof: the proof signs the
/// canonical rotation fields under this domain and no other, so a
/// signature made for any other purpose is never a successor proof.
pub const SUCCESSOR_PROOF_DOMAIN: &str = "issuer-rotation-v1 successor proof";

/// The hash a currency definition commits for its recovery key: the
/// digest of the recovery key's verifying-key bytes. The recovery key
/// itself is a cold credential and appears nowhere in the definition
/// or in any rotation object — only this commitment does (§3.1).
#[must_use]
pub fn recovery_commitment(recovery_key: &IssuerId) -> Hash {
    Hash::digest(recovery_key)
}

/// How a rotation transition is authorised (§3.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RotationMode {
    /// The current generation key rotates to its successor; both
    /// keys sign (envelope + successor proof).
    Routine,
    /// The precommitted recovery key retires a compromised
    /// generation and installs its successor; the compromised key
    /// does not sign.
    CompromiseRecovery,
}

/// An issuer rotation body: generation `from_generation` is replaced
/// by `successor_key` as generation `to_generation`
/// (`from_generation + 1`) for one currency.
///
/// The event carrying this body is signed by the authorising key
/// (the current generation key for [`RotationMode::Routine`], the
/// recovery key for [`RotationMode::CompromiseRecovery`]); the
/// successor countersigns through `successor_proof`, a signature by
/// `successor_key` over [`IssuerRotation::proof_message`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssuerRotation {
    /// The issuer's stable identity anchor: its generation-0 key.
    /// A rotation never changes this (§3.1, invariant 1).
    pub issuer_id: IssuerId,
    /// The one currency this rotation applies to. There is no
    /// issuer-wide wildcard (§3.2 rule 1).
    pub currency_id: Hash,
    /// How this transition is authorised.
    pub mode: RotationMode,
    /// The generation being replaced.
    pub from_generation: u64,
    /// The generation installed; exactly `from_generation + 1`.
    pub to_generation: u64,
    /// The event id of the previous rotation, or `None` for the
    /// initial transition out of generation 0 — absent only there
    /// (§3.2).
    pub previous_rotation: Option<Hash>,
    /// The successor generation's verifying key.
    pub successor_key: IssuerId,
    /// The successor's countersignature over the canonical rotation
    /// fields (see [`IssuerRotation::proof_message`]). All zeroes
    /// until [`IssuerRotation::prove_successor`] runs.
    #[serde(with = "proof_serde")]
    pub successor_proof: [u8; 64],
    /// Recovery mode only: the event id from which activity by the
    /// compromised generation is asserted to be disputed (§3.3). An
    /// event id, never a timestamp — signatures cannot prove when a
    /// key was stolen (§4). Must be `None` for routine rotations.
    pub asserted_compromise_from: Option<Hash>,
}

/// Serde for the 64-byte successor proof: this workspace's serde
/// implements arrays only up to 32 bytes, so the proof serializes as
/// a byte sequence (the same JSON shape `[u8; 32]` issuer ids get)
/// and deserializes through a length-checked vector.
mod proof_serde {
    use serde::{Deserialize as _, Deserializer, Serialize as _, Serializer};

    /// Serializes the proof as a sequence of bytes.
    ///
    /// # Errors
    ///
    /// Returns the serializer's error, if it has one.
    pub fn serialize<S>(proof: &[u8; 64], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        proof.as_slice().serialize(serializer)
    }

    /// Deserializes the proof, rejecting any length but 64 bytes.
    ///
    /// # Errors
    ///
    /// Returns a deserialization error on a malformed or
    /// wrong-length proof.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 64], D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes = Vec::<u8>::deserialize(deserializer)?;
        <[u8; 64]>::try_from(bytes.as_slice())
            .map_err(|_| serde::de::Error::invalid_length(bytes.len(), &"a 64-byte proof"))
    }
}

/// The canonical field set the successor proof signs: every field of
/// [`IssuerRotation`] except the proof itself.
#[derive(Serialize)]
struct SuccessorProofFields<'a> {
    issuer_id: &'a IssuerId,
    currency_id: &'a Hash,
    mode: &'a RotationMode,
    from_generation: u64,
    to_generation: u64,
    previous_rotation: Option<Hash>,
    successor_key: &'a IssuerId,
    asserted_compromise_from: Option<Hash>,
}

impl IssuerRotation {
    /// Builds a rotation with a zeroed successor proof; call
    /// [`IssuerRotation::prove_successor`] with the successor key
    /// before signing the event. `to_generation` is computed as
    /// `from_generation + 1` (saturating — a `from_generation` of
    /// `u64::MAX` yields an object validation always rejects).
    #[must_use]
    pub fn unsigned(
        mode: RotationMode,
        issuer_id: IssuerId,
        currency_id: Hash,
        from_generation: u64,
        previous_rotation: Option<Hash>,
        successor_key: IssuerId,
        asserted_compromise_from: Option<Hash>,
    ) -> Self {
        Self {
            issuer_id,
            currency_id,
            mode,
            from_generation,
            to_generation: from_generation.saturating_add(1),
            previous_rotation,
            successor_key,
            successor_proof: [0; 64],
            asserted_compromise_from,
        }
    }

    /// The domain-separated message the successor proof signs: the
    /// proof domain, then the canonical JSON of every rotation field
    /// except the proof itself.
    ///
    /// # Errors
    ///
    /// Returns a JSON error if the proof fields cannot be serialized
    /// (not expected for this shape).
    pub fn proof_message(&self) -> Result<Vec<u8>, serde_json::Error> {
        let fields = SuccessorProofFields {
            issuer_id: &self.issuer_id,
            currency_id: &self.currency_id,
            mode: &self.mode,
            from_generation: self.from_generation,
            to_generation: self.to_generation,
            previous_rotation: self.previous_rotation,
            successor_key: &self.successor_key,
            asserted_compromise_from: self.asserted_compromise_from,
        };
        let mut message = SUCCESSOR_PROOF_DOMAIN.as_bytes().to_vec();
        message.push(0);
        message.extend_from_slice(&serde_json::to_vec(&fields)?);
        Ok(message)
    }

    /// Countersigns this rotation with the successor key, filling in
    /// `successor_proof`. The caller is responsible for calling this
    /// with the key named by `successor_key`; a proof by any other
    /// key fails verification.
    ///
    /// # Errors
    ///
    /// Returns a JSON error if the proof message cannot be
    /// serialized (not expected for this shape).
    pub fn prove_successor(&mut self, successor: &SigningKey) -> Result<(), serde_json::Error> {
        let message = self.proof_message()?;
        self.successor_proof = successor.sign(&message).to_bytes();
        Ok(())
    }

    /// Whether `successor_proof` is a valid signature by
    /// `successor_key` over [`IssuerRotation::proof_message`].
    /// Verified strictly, so a malleated proof does not pass.
    #[must_use]
    pub fn verify_successor_proof(&self) -> bool {
        let Ok(message) = self.proof_message() else {
            return false;
        };
        let Ok(public) = VerifyingKey::from_bytes(&self.successor_key) else {
            return false;
        };
        public
            .verify_strict(&message, &Signature::from_bytes(&self.successor_proof))
            .is_ok()
    }

    /// The shape rules every validation path shares, needing nothing
    /// beyond the body itself: generations are consecutive, and the
    /// predecessor rule holds (only the initial transition out of
    /// generation 0 names no previous rotation).
    fn check_shape(&self) -> Result<(), RotationError> {
        if self.to_generation != self.from_generation.wrapping_add(1)
            || self.to_generation <= self.from_generation
        {
            return Err(RotationError::NonConsecutiveGeneration);
        }
        match (self.from_generation, self.previous_rotation) {
            (0, Some(_)) => return Err(RotationError::UnexpectedPredecessor),
            (_, None) if self.from_generation != 0 => {
                return Err(RotationError::MissingPredecessor);
            }
            _ => {}
        }
        Ok(())
    }

    /// Checks this body against its event's verified author and the
    /// currency definition's recovery-key commitment.
    ///
    /// This is the stateless half of validation: consecutive
    /// generations, the predecessor rule, the asserted-compromise
    /// rule, the successor proof, and — for recovery mode — that the
    /// author is the committed recovery key. For routine mode it
    /// also refuses a self-rotation — successor equal to the
    /// author — which the fold could never count (#6451).
    /// The chain-dependent
    /// half (a routine author must be the *current* generation key;
    /// a recovery author must not be the key it replaces) is
    /// enforced by [`fold_rotation_chain`], the only place the
    /// current generation key is known.
    ///
    /// # Errors
    ///
    /// Returns the [`RotationError`] naming the first failed rule.
    pub fn validate(
        &self,
        author: &IssuerId,
        committed_recovery: Option<&Hash>,
    ) -> Result<(), RotationError> {
        self.check_shape()?;
        match self.mode {
            RotationMode::Routine => {
                if self.asserted_compromise_from.is_some() {
                    return Err(RotationError::AssertedCompromiseOnRoutine);
                }
                if self.successor_key == *author {
                    return Err(RotationError::RoutineSuccessorIsAuthor);
                }
            }
            RotationMode::CompromiseRecovery => {
                let Some(commitment) = committed_recovery else {
                    return Err(RotationError::RecoveryCommitmentMissing);
                };
                if recovery_commitment(author) != *commitment {
                    return Err(RotationError::RecoveryAuthorMismatch);
                }
            }
        }
        if !self.verify_successor_proof() {
            return Err(RotationError::SuccessorProofInvalid);
        }
        Ok(())
    }
}

/// Why a rotation body is invalid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RotationError {
    /// The carrying event's kind is not the issuer rotation kind.
    KindMismatch {
        /// The kind of the event.
        kind: String,
    },
    /// `to_generation` is not exactly `from_generation + 1`.
    NonConsecutiveGeneration,
    /// A transition out of generation 0 names a previous rotation.
    UnexpectedPredecessor,
    /// A later transition names no previous rotation.
    MissingPredecessor,
    /// A routine rotation carries an asserted compromise start;
    /// only recovery rotations may (§3.3).
    AssertedCompromiseOnRoutine,
    /// A routine rotation names its own author as the successor
    /// key: the transition installs the very key it replaces, so
    /// the fold can never count it (#6451).
    RoutineSuccessorIsAuthor,
    /// A recovery rotation was presented for a currency whose
    /// definition committed no recovery key: no issuer-signed
    /// declaration can be authoritative for it (§3.4).
    RecoveryCommitmentMissing,
    /// The recovery event's author is not the key the definition
    /// committed — in particular, the compromised minting key cannot
    /// authorise its own recovery (invariant 3).
    RecoveryAuthorMismatch,
    /// The successor proof is missing or does not verify against
    /// `successor_key`.
    SuccessorProofInvalid,
}

impl std::fmt::Display for RotationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::KindMismatch { kind } => {
                write!(f, "a {kind:?} event carries an issuer rotation")
            }
            Self::NonConsecutiveGeneration => {
                write!(f, "rotation generation is not consecutive")
            }
            Self::UnexpectedPredecessor => {
                write!(f, "initial rotation names a previous rotation")
            }
            Self::MissingPredecessor => {
                write!(f, "rotation names no previous rotation")
            }
            Self::AssertedCompromiseOnRoutine => {
                write!(f, "routine rotation asserts a compromise start")
            }
            Self::RoutineSuccessorIsAuthor => {
                write!(f, "routine rotation names its author as its successor")
            }
            Self::RecoveryCommitmentMissing => {
                write!(f, "currency definition commits no recovery key")
            }
            Self::RecoveryAuthorMismatch => {
                write!(f, "recovery author is not the committed recovery key")
            }
            Self::SuccessorProofInvalid => {
                write!(f, "successor proof is missing or invalid")
            }
        }
    }
}

impl std::error::Error for RotationError {}

/// Whether a successor key's event causally references its rotation
/// event — the `after` link §3.2 rule 4 requires, because the
/// successor begins a separate author chain and the old key's
/// backlink cannot establish the relation. An unlinked event is not
/// treated as authorised by the rotation.
#[must_use]
pub fn references_rotation(after: &[Hash], rotation_event: &Hash) -> bool {
    after.contains(rotation_event)
}

/// A recovery transition the fold accepted, with the disputed-window
/// assertion it carried (§3.3: activity by the compromised
/// generation from `asserted_compromise_from` up to this recovery is
/// disputed — excluded from settled counts, never erased).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryStep {
    /// The compromised generation this recovery retired.
    pub generation: u64,
    /// The recovery rotation's event id: the recovery cutoff.
    pub event_id: Hash,
    /// The asserted compromise start, if the recovery carried one.
    pub asserted_compromise_from: Option<Hash>,
}

/// How a folded generation chain stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChainStatus {
    /// Every accepted transition was the unique valid successor of
    /// its predecessor; the chain's head is authoritative.
    Settled,
    /// Two or more valid transitions claim the same predecessor and
    /// no single recovery transition resolves the fork. Neither
    /// branch settles (§3.2 rule 5): the chain stops at the fork
    /// generation and later activity on both branches stays disputed
    /// until a recovery resolves the issuer head.
    Conflicted {
        /// The generation the competing transitions each claim to
        /// install.
        generation: u64,
        /// The competing transitions' event ids, sorted.
        candidates: Vec<Hash>,
    },
}

/// The folded generation chain of one currency: which key speaks
/// for each generation, and how the chain stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationChain {
    /// The issuer's generation-0 key; also `keys[0]`.
    pub issuer_id: IssuerId,
    /// The currency this chain belongs to.
    pub currency_id: Hash,
    /// `keys[g]` is generation `g`'s key. Index 0 is the key the
    /// currency definition committed; rotation extends this list
    /// without ever changing earlier entries (§3.1, invariant 1).
    pub keys: Vec<IssuerId>,
    /// `transitions[g]` is the event id of the rotation that
    /// installed generation `g + 1`.
    pub transitions: Vec<Hash>,
    /// The accepted recovery transitions, in generation order.
    pub recoveries: Vec<RecoveryStep>,
    /// Transitions that claimed a predecessor a recovery then
    /// resolved: they are void (§3.2 rule 5), their successors are
    /// not any generation's key, and activity under them stays
    /// disputed. Empty unless a recovery resolved a fork.
    pub voided_transitions: Vec<Hash>,
    /// Whether the chain head settled or forked.
    pub status: ChainStatus,
}

impl GenerationChain {
    /// The highest settled generation (0 when no rotation settled).
    #[must_use]
    pub fn active_generation(&self) -> u64 {
        u64::try_from(self.keys.len().saturating_sub(1)).unwrap_or(u64::MAX)
    }

    /// The key of the highest settled generation.
    #[must_use]
    pub fn active_key(&self) -> &IssuerId {
        self.keys.last().unwrap_or(&self.issuer_id)
    }

    /// The key of `generation`, if that generation settled.
    #[must_use]
    pub fn key_for_generation(&self, generation: u64) -> Option<&IssuerId> {
        self.keys.get(usize::try_from(generation).ok()?)
    }

    /// Whether `key` is the currently active generation's key.
    /// Historical generations remain valid for their own interval
    /// (§3.3) — use [`GenerationChain::key_for_generation`] to check
    /// a specific generation.
    #[must_use]
    pub fn is_active_key(&self, key: &IssuerId) -> bool {
        self.active_key() == key
    }
}

/// Folds one currency's generation chain from its signed rotation
/// objects alone.
///
/// `rotations` pairs each rotation event's verified author and event
/// id with the rotation carried in its extension; `committed_recovery` is the
/// recovery-key hash from the currency definition, or `None` when
/// the definition committed none (recovery transitions are then
/// never authoritative, §3.4).
///
/// The fold is deterministic and independent of input order. From
/// generation 0 (the definition's key) it repeatedly looks for valid
/// transitions out of the current generation whose
/// `previous_rotation` is the current head transition:
///
/// - a *routine* transition counts only when authored by the
///   current generation key;
/// - a *recovery* transition counts only when authored by the
///   committed recovery key and not by the key it replaces;
/// - a transition whose successor is the key it replaces never
///   counts: it would change the generation without changing the
///   key, and as a recovery it would leave the compromised key
///   active;
/// - a rotation failing [`IssuerRotation::validate`] never counts
///   (fail closed, §6).
///
/// Exactly one candidate advances the chain. Zero ends it. Two or
/// more distinct candidates fork it — unless exactly one of them is
/// a recovery transition: a valid committed-key recovery supersedes
/// a compromised generation (§3.4), so it resolves the fork, the
/// chain advances through it, and the rival transitions are recorded
/// in [`GenerationChain::voided_transitions`] rather than settled.
/// A fork no single recovery claims stops the chain with status
/// [`ChainStatus::Conflicted`], and no candidate's successor is
/// treated as settled (§3.2 rule 5).
#[must_use]
pub fn fold_rotation_chain(
    issuer_id: &IssuerId,
    currency_id: &Hash,
    committed_recovery: Option<&Hash>,
    rotations: &[(IssuerId, Hash, IssuerRotation)],
) -> GenerationChain {
    let mut chain = GenerationChain {
        issuer_id: *issuer_id,
        currency_id: *currency_id,
        keys: vec![*issuer_id],
        transitions: Vec::new(),
        recoveries: Vec::new(),
        voided_transitions: Vec::new(),
        status: ChainStatus::Settled,
    };
    loop {
        let generation = chain.active_generation();
        let current_key = *chain.active_key();
        let head = chain.transitions.last().copied();
        let mut candidates: Vec<&(IssuerId, Hash, IssuerRotation)> = rotations
            .iter()
            .filter(|(_, _, rotation)| {
                rotation.issuer_id == *issuer_id && rotation.currency_id == *currency_id
            })
            .filter(|(_, _, rotation)| {
                rotation.from_generation == generation && rotation.previous_rotation == head
            })
            .filter(|(author, _, rotation)| rotation.validate(author, committed_recovery).is_ok())
            .filter(|(author, _, rotation)| match rotation.mode {
                RotationMode::Routine => *author == current_key,
                RotationMode::CompromiseRecovery => *author != current_key,
            })
            // A transition that installs the very key it replaces
            // changes nothing; as a recovery it would leave the
            // compromised key active under a new generation number.
            .filter(|(_, _, rotation)| rotation.successor_key != current_key)
            .collect();
        // Order and deduplicate on the candidate's full content —
        // event id, author, and the rotation body — never on the
        // event id alone. Distinct bodies can share one event id,
        // and an id-only dedup keeps whichever arrived first, so the
        // fold would depend on input order and break the determinism
        // contract above. With the full tuple as the key, identical
        // duplicates still collapse to one, while distinct same-id
        // candidates both survive to the fork handling below (a
        // single recovery resolves them, otherwise the chain
        // conflicts) — a function of the input multiset, never of
        // its listing order. The oracle is permutation invariance:
        // every listing of the same multiset folds identically.
        candidates.sort_by_key(|(author, event_id, rotation)| {
            (
                *event_id,
                *author,
                match rotation.mode {
                    RotationMode::Routine => 0u8,
                    RotationMode::CompromiseRecovery => 1u8,
                },
                rotation.successor_key,
                rotation.successor_proof,
                rotation.asserted_compromise_from,
                rotation.to_generation,
            )
        });
        candidates.dedup_by(|a, b| *a == *b);
        match candidates.as_slice() {
            [] => break,
            [(_, event_id, rotation)] => {
                let event_id = *event_id;
                let rotation = (*rotation).clone();
                if rotation.mode == RotationMode::CompromiseRecovery {
                    chain.recoveries.push(RecoveryStep {
                        generation,
                        event_id,
                        asserted_compromise_from: rotation.asserted_compromise_from,
                    });
                }
                chain.keys.push(rotation.successor_key);
                chain.transitions.push(event_id);
            }
            many => {
                // A single recovery candidate resolves the fork
                // (§3.4): the compromised generation's own transitions
                // cannot veto the committed recovery key. Rival
                // transitions are void, never settled.
                let recovery_candidates: Vec<&(IssuerId, Hash, IssuerRotation)> = many
                    .iter()
                    .filter(|(_, _, rotation)| rotation.mode == RotationMode::CompromiseRecovery)
                    .copied()
                    .collect();
                if let [(_, event_id, rotation)] = recovery_candidates.as_slice() {
                    let event_id = *event_id;
                    let rotation = (*rotation).clone();
                    chain.voided_transitions.extend(
                        many.iter()
                            .map(|(_, rival, _)| *rival)
                            .filter(|rival| *rival != event_id),
                    );
                    chain.recoveries.push(RecoveryStep {
                        generation,
                        event_id,
                        asserted_compromise_from: rotation.asserted_compromise_from,
                    });
                    chain.keys.push(rotation.successor_key);
                    chain.transitions.push(event_id);
                } else {
                    // The status names the competing transitions by
                    // event id; distinct candidates can share one id
                    // (above), so list each id once. `many` is sorted
                    // by event id first, so a plain dedup suffices.
                    let mut candidate_ids: Vec<Hash> =
                        many.iter().map(|(_, event_id, _)| *event_id).collect();
                    candidate_ids.dedup();
                    chain.status = ChainStatus::Conflicted {
                        generation: generation + 1,
                        candidates: candidate_ids,
                    };
                    break;
                }
            }
        }
    }
    chain
}

/// Signs a rotation as an `issuer.rotation` event. The rotation
/// travels in the signed header extension, with no event body: the
/// store persists the header, so a decoded event always carries the
/// whole rotation with it (§3.2).
///
/// # Errors
///
/// Returns [`SignError::Rotation`] with
/// [`RotationError::RoutineSuccessorIsAuthor`] for a routine
/// rotation whose successor is the signing key itself: the fold
/// can never count such a transition, so signing it would hand
/// the caller a valid-looking event that never advances the
/// chain (#6451). Returns a chain error if the extension or
/// backlink is invalid otherwise.
pub fn sign_rotation(
    key: &SigningKey,
    seq_num: SeqNum,
    backlink: Option<Hash>,
    rotation: &IssuerRotation,
    after: Vec<Hash>,
) -> Result<Event, SignError> {
    if rotation.mode == RotationMode::Routine && rotation.successor_key == issuer_id(key) {
        return Err(RotationError::RoutineSuccessorIsAuthor.into());
    }
    Ok(chain::sign(
        key,
        seq_num,
        backlink,
        None,
        ChainExt {
            kind: ISSUER_ROTATION_KIND.to_owned(),
            group: None,
            after,
            tag: None,
            document: None,
            removes: None,
            note: None,
            link: None,
            checkpoint: None,
            rotation: Some(rotation.clone()),

            chat: None,
        },
    )?)
}

/// A signing failure: the chain refused the event.
#[derive(Debug)]
pub enum SignError {
    /// The chain refused the event (extension or backlink rules).
    Chain(chain::ChainError),
    /// The rotation breaks a rule the signer can check locally
    /// before producing an event (#6451).
    Rotation(RotationError),
}

impl From<chain::ChainError> for SignError {
    fn from(err: chain::ChainError) -> Self {
        Self::Chain(err)
    }
}

impl From<RotationError> for SignError {
    fn from(err: RotationError) -> Self {
        Self::Rotation(err)
    }
}

impl std::fmt::Display for SignError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Chain(err) => write!(f, "chain refused the rotation event: {err:?}"),
            Self::Rotation(err) => write!(f, "rotation refused at sign time: {err}"),
        }
    }
}

impl std::error::Error for SignError {}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::issuer_rotation::{author_id, issuer_id};

    fn gen0_key() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

    fn gen1_key() -> SigningKey {
        SigningKey::from_bytes(&[9; 32])
    }

    fn gen2_key() -> SigningKey {
        SigningKey::from_bytes(&[11; 32])
    }

    fn gen3_key() -> SigningKey {
        SigningKey::from_bytes(&[17; 32])
    }

    fn recovery_key() -> SigningKey {
        SigningKey::from_bytes(&[13; 32])
    }

    fn stranger_key() -> SigningKey {
        SigningKey::from_bytes(&[15; 32])
    }

    fn currency() -> Hash {
        Hash::digest([42])
    }

    fn event(n: u8) -> Hash {
        Hash::digest([n])
    }

    /// A proven rotation out of `from_generation` to `successor`,
    /// authored (in the fold inputs) by whoever the caller pairs it
    /// with.
    fn rotation(
        mode: RotationMode,
        from_generation: u64,
        previous: Option<Hash>,
        successor: &SigningKey,
        asserted: Option<Hash>,
    ) -> IssuerRotation {
        let mut rotation = IssuerRotation::unsigned(
            mode,
            issuer_id(&gen0_key()),
            currency(),
            from_generation,
            previous,
            issuer_id(successor),
            asserted,
        );
        rotation
            .prove_successor(successor)
            .expect("prove successor");
        rotation
    }

    fn commitment() -> Hash {
        recovery_commitment(&issuer_id(&recovery_key()))
    }

    #[test]
    fn check_rotation_accepts_well_formed_bodies_without_context() {
        // A routine body passes on its own fields alone.
        let routine = rotation(RotationMode::Routine, 0, None, &gen1_key(), None);
        assert_eq!(check_rotation(ISSUER_ROTATION_KIND, &routine), Ok(()));
        // A recovery body carrying an asserted compromise start also
        // passes: the author and commitment rules are context, and
        // check_rotation has neither — they stay in `validate`.
        let recovery = rotation(
            RotationMode::CompromiseRecovery,
            0,
            None,
            &gen1_key(),
            Some(event(3)),
        );
        assert_eq!(check_rotation(ISSUER_ROTATION_KIND, &recovery), Ok(()));
    }

    #[test]
    fn check_rotation_refuses_the_wrong_kind() {
        let routine = rotation(RotationMode::Routine, 0, None, &gen1_key(), None);
        assert_eq!(
            check_rotation("note.create", &routine),
            Err(RotationError::KindMismatch {
                kind: "note.create".to_owned(),
            })
        );
    }

    #[test]
    fn single_routine_rotation_advances_the_generation() {
        let first = rotation(RotationMode::Routine, 0, None, &gen1_key(), None);
        let chain = fold_rotation_chain(
            &issuer_id(&gen0_key()),
            &currency(),
            Some(&commitment()),
            &[(issuer_id(&gen0_key()), event(1), first)],
        );
        assert_eq!(chain.status, ChainStatus::Settled);
        assert_eq!(chain.active_generation(), 1);
        assert_eq!(chain.active_key(), &issuer_id(&gen1_key()));
        assert_eq!(chain.key_for_generation(0), Some(&issuer_id(&gen0_key())));
        assert!(chain.recoveries.is_empty());
    }

    #[test]
    fn two_sequential_rotations_fold_independent_of_input_order() {
        let first = rotation(RotationMode::Routine, 0, None, &gen1_key(), None);
        let second = rotation(RotationMode::Routine, 1, Some(event(1)), &gen2_key(), None);
        let inputs = [
            (issuer_id(&gen1_key()), event(2), second),
            (issuer_id(&gen0_key()), event(1), first),
        ];
        let chain = fold_rotation_chain(
            &issuer_id(&gen0_key()),
            &currency(),
            Some(&commitment()),
            &inputs,
        );
        assert_eq!(chain.status, ChainStatus::Settled);
        assert_eq!(chain.active_generation(), 2);
        assert_eq!(chain.active_key(), &issuer_id(&gen2_key()));
        assert_eq!(chain.transitions, vec![event(1), event(2)]);
    }

    #[test]
    fn missing_successor_proof_is_rejected() {
        let mut unproven = rotation(RotationMode::Routine, 0, None, &gen1_key(), None);
        unproven.successor_proof = [0; 64];
        assert_eq!(
            unproven.validate(&issuer_id(&gen0_key()), Some(&commitment())),
            Err(RotationError::SuccessorProofInvalid)
        );
        let chain = fold_rotation_chain(
            &issuer_id(&gen0_key()),
            &currency(),
            Some(&commitment()),
            &[(issuer_id(&gen0_key()), event(1), unproven)],
        );
        assert_eq!(chain.active_generation(), 0);
    }

    #[test]
    fn successor_proof_by_another_key_is_rejected() {
        let mut forged = IssuerRotation::unsigned(
            RotationMode::Routine,
            issuer_id(&gen0_key()),
            currency(),
            0,
            None,
            issuer_id(&gen1_key()),
            None,
        );
        forged.prove_successor(&stranger_key()).expect("prove");
        assert_eq!(
            forged.validate(&issuer_id(&gen0_key()), Some(&commitment())),
            Err(RotationError::SuccessorProofInvalid)
        );
    }

    #[test]
    fn nonconsecutive_and_stale_generations_are_rejected() {
        let mut skipped = rotation(RotationMode::Routine, 0, None, &gen1_key(), None);
        skipped.to_generation = 2;
        skipped.prove_successor(&gen1_key()).expect("prove");
        assert_eq!(
            skipped.validate(&issuer_id(&gen0_key()), Some(&commitment())),
            Err(RotationError::NonConsecutiveGeneration)
        );
        // A generation-1 transition whose predecessor is not the
        // chain head is stale: the fold at generation 0 ignores it.
        let stale = rotation(RotationMode::Routine, 1, Some(event(99)), &gen2_key(), None);
        let chain = fold_rotation_chain(
            &issuer_id(&gen0_key()),
            &currency(),
            Some(&commitment()),
            &[(issuer_id(&gen1_key()), event(2), stale)],
        );
        assert_eq!(chain.active_generation(), 0);
        assert_eq!(chain.status, ChainStatus::Settled);
    }

    #[test]
    fn duplicate_predecessor_conflicts_and_neither_branch_settles() {
        let first = rotation(RotationMode::Routine, 0, None, &gen1_key(), None);
        let rival = rotation(RotationMode::Routine, 0, None, &gen2_key(), None);
        let chain = fold_rotation_chain(
            &issuer_id(&gen0_key()),
            &currency(),
            Some(&commitment()),
            &[
                (issuer_id(&gen0_key()), event(1), first),
                (issuer_id(&gen0_key()), event(2), rival),
            ],
        );
        assert_eq!(chain.active_generation(), 0);
        match &chain.status {
            ChainStatus::Conflicted {
                generation,
                candidates,
            } => {
                assert_eq!(*generation, 1);
                assert_eq!(candidates.len(), 2);
                assert!(candidates.contains(&event(1)));
                assert!(candidates.contains(&event(2)));
            }
            other => panic!("expected a conflicted chain, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_event_id_with_distinct_routine_bodies_conflicts_in_either_order() {
        // Two distinct, individually valid routine bodies sharing
        // one event id: neither may be picked by arrival order. The
        // design (§3.2 rule 5) prescribes a conflict for two valid
        // transitions claiming the same predecessor.
        let first = rotation(RotationMode::Routine, 0, None, &gen1_key(), None);
        let rival = rotation(RotationMode::Routine, 0, None, &gen2_key(), None);
        let forward = fold_rotation_chain(
            &issuer_id(&gen0_key()),
            &currency(),
            Some(&commitment()),
            &[
                (issuer_id(&gen0_key()), event(1), first.clone()),
                (issuer_id(&gen0_key()), event(1), rival.clone()),
            ],
        );
        let reversed = fold_rotation_chain(
            &issuer_id(&gen0_key()),
            &currency(),
            Some(&commitment()),
            &[
                (issuer_id(&gen0_key()), event(1), rival),
                (issuer_id(&gen0_key()), event(1), first),
            ],
        );
        assert_eq!(forward, reversed);
        assert_eq!(forward.active_generation(), 0);
        assert_eq!(
            forward.status,
            ChainStatus::Conflicted {
                generation: 1,
                candidates: vec![event(1)],
            }
        );
    }

    #[test]
    fn duplicate_event_id_recovery_resolves_regardless_of_order() {
        // A routine body and a recovery body sharing one event id:
        // the single committed-key recovery resolves the fork in
        // either listing order (§3.4), instead of the survivor being
        // whichever body happened to be listed first.
        let routine = rotation(RotationMode::Routine, 0, None, &gen1_key(), None);
        let recovery = rotation(
            RotationMode::CompromiseRecovery,
            0,
            None,
            &gen2_key(),
            Some(event(50)),
        );
        let forward = fold_rotation_chain(
            &issuer_id(&gen0_key()),
            &currency(),
            Some(&commitment()),
            &[
                (issuer_id(&gen0_key()), event(1), routine.clone()),
                (issuer_id(&recovery_key()), event(1), recovery.clone()),
            ],
        );
        let reversed = fold_rotation_chain(
            &issuer_id(&gen0_key()),
            &currency(),
            Some(&commitment()),
            &[
                (issuer_id(&recovery_key()), event(1), recovery),
                (issuer_id(&gen0_key()), event(1), routine),
            ],
        );
        assert_eq!(forward, reversed);
        assert_eq!(forward.status, ChainStatus::Settled);
        assert_eq!(forward.active_generation(), 1);
        assert_eq!(forward.active_key(), &issuer_id(&gen2_key()));
        assert_eq!(forward.recoveries.len(), 1);
        assert_eq!(forward.recoveries[0].event_id, event(1));
    }

    #[test]
    fn identical_duplicate_candidate_still_counts_once() {
        let first = rotation(RotationMode::Routine, 0, None, &gen1_key(), None);
        let entry = (issuer_id(&gen0_key()), event(1), first);
        let chain = fold_rotation_chain(
            &issuer_id(&gen0_key()),
            &currency(),
            Some(&commitment()),
            &[entry.clone(), entry],
        );
        assert_eq!(chain.status, ChainStatus::Settled);
        assert_eq!(chain.active_generation(), 1);
        assert_eq!(chain.transitions, vec![event(1)]);
    }

    #[test]
    fn fold_is_permutation_invariant_with_duplicate_event_ids() {
        // Property test; the oracle is permutation invariance of
        // the fold. The multiset mixes three distinct bodies under
        // one event id (two routine rivals and one recovery) plus
        // an identical duplicate of the recovery: every listing
        // must fold to the same chain, settled through the
        // recovery exactly once.
        let routine_a = rotation(RotationMode::Routine, 0, None, &gen1_key(), None);
        let routine_b = rotation(RotationMode::Routine, 0, None, &gen2_key(), None);
        let recovery = rotation(
            RotationMode::CompromiseRecovery,
            0,
            None,
            &gen3_key(),
            Some(event(50)),
        );
        let inputs = vec![
            (issuer_id(&gen0_key()), event(1), routine_a),
            (issuer_id(&gen0_key()), event(1), routine_b),
            (issuer_id(&recovery_key()), event(1), recovery),
        ];
        let inputs = {
            let mut with_dup = inputs.clone();
            with_dup.push(inputs[2].clone());
            with_dup
        };
        let expected = fold_rotation_chain(
            &issuer_id(&gen0_key()),
            &currency(),
            Some(&commitment()),
            &inputs,
        );
        assert_eq!(expected.status, ChainStatus::Settled);
        assert_eq!(expected.active_key(), &issuer_id(&gen3_key()));
        assert_eq!(expected.transitions, vec![event(1)]);
        assert_eq!(expected.recoveries.len(), 1);
        fn permutations<T: Clone>(items: &[T]) -> Vec<Vec<T>> {
            if items.len() <= 1 {
                return vec![items.to_vec()];
            }
            let mut out = Vec::new();
            for i in 0..items.len() {
                let mut rest = items.to_vec();
                let head = rest.remove(i);
                for mut tail in permutations(&rest) {
                    let mut perm = vec![head.clone()];
                    perm.append(&mut tail);
                    out.push(perm);
                }
            }
            out
        }
        for perm in permutations(&inputs) {
            let chain = fold_rotation_chain(
                &issuer_id(&gen0_key()),
                &currency(),
                Some(&commitment()),
                &perm,
            );
            assert_eq!(chain, expected);
        }
    }

    #[test]
    fn recovery_by_the_committed_key_supersedes_a_compromised_generation() {
        let first = rotation(RotationMode::Routine, 0, None, &gen1_key(), None);
        let recovery = rotation(
            RotationMode::CompromiseRecovery,
            1,
            Some(event(1)),
            &gen2_key(),
            Some(event(50)),
        );
        let chain = fold_rotation_chain(
            &issuer_id(&gen0_key()),
            &currency(),
            Some(&commitment()),
            &[
                (issuer_id(&gen0_key()), event(1), first),
                (issuer_id(&recovery_key()), event(2), recovery),
            ],
        );
        assert_eq!(chain.status, ChainStatus::Settled);
        assert_eq!(chain.active_generation(), 2);
        assert_eq!(chain.active_key(), &issuer_id(&gen2_key()));
        assert_eq!(
            chain.recoveries,
            vec![RecoveryStep {
                generation: 1,
                event_id: event(2),
                asserted_compromise_from: Some(event(50)),
            }]
        );
    }

    #[test]
    fn the_compromised_key_cannot_author_its_own_recovery() {
        let recovery = rotation(RotationMode::CompromiseRecovery, 0, None, &gen1_key(), None);
        // Authored by generation 0 itself: not the committed
        // recovery key, so validation fails and the fold holds.
        assert_eq!(
            recovery.validate(&issuer_id(&gen0_key()), Some(&commitment())),
            Err(RotationError::RecoveryAuthorMismatch)
        );
        let chain = fold_rotation_chain(
            &issuer_id(&gen0_key()),
            &currency(),
            Some(&commitment()),
            &[(issuer_id(&gen0_key()), event(1), recovery)],
        );
        assert_eq!(chain.active_generation(), 0);
        assert!(chain.recoveries.is_empty());
    }

    #[test]
    fn recovery_without_a_commitment_is_never_authoritative() {
        let recovery = rotation(RotationMode::CompromiseRecovery, 0, None, &gen1_key(), None);
        assert_eq!(
            recovery.validate(&issuer_id(&recovery_key()), None),
            Err(RotationError::RecoveryCommitmentMissing)
        );
        let chain = fold_rotation_chain(
            &issuer_id(&gen0_key()),
            &currency(),
            None,
            &[(issuer_id(&recovery_key()), event(1), recovery)],
        );
        assert_eq!(chain.active_generation(), 0);
    }

    #[test]
    fn routine_rotation_cannot_assert_a_compromise_start() {
        let rotation = rotation(RotationMode::Routine, 0, None, &gen1_key(), Some(event(50)));
        assert_eq!(
            rotation.validate(&issuer_id(&gen0_key()), Some(&commitment())),
            Err(RotationError::AssertedCompromiseOnRoutine)
        );
    }

    #[test]
    fn a_recovery_installing_the_retired_key_does_not_advance() {
        // The successor proof is by generation 0 itself (it is the
        // named successor), so the object validates; the fold must
        // still refuse it — a recovery that keeps the compromised
        // key is not a recovery.
        let recovery = rotation(RotationMode::CompromiseRecovery, 0, None, &gen0_key(), None);
        let chain = fold_rotation_chain(
            &issuer_id(&gen0_key()),
            &currency(),
            Some(&commitment()),
            &[(issuer_id(&recovery_key()), event(1), recovery)],
        );
        assert_eq!(chain.active_generation(), 0);
        assert!(chain.recoveries.is_empty());
        assert_eq!(chain.status, ChainStatus::Settled);
    }

    #[test]
    fn a_routine_rotation_to_the_same_key_does_not_advance() {
        let rotation = rotation(RotationMode::Routine, 0, None, &gen0_key(), None);
        let chain = fold_rotation_chain(
            &issuer_id(&gen0_key()),
            &currency(),
            Some(&commitment()),
            &[(issuer_id(&gen0_key()), event(1), rotation)],
        );
        assert_eq!(chain.active_generation(), 0);
    }

    #[test]
    fn a_routine_rotation_by_a_stranger_does_not_advance_the_chain() {
        let first = rotation(RotationMode::Routine, 0, None, &gen1_key(), None);
        let chain = fold_rotation_chain(
            &issuer_id(&gen0_key()),
            &currency(),
            Some(&commitment()),
            &[(issuer_id(&stranger_key()), event(1), first)],
        );
        assert_eq!(chain.active_generation(), 0);
    }

    #[test]
    fn successor_events_must_reference_their_rotation() {
        assert!(references_rotation(&[event(7), event(1)], &event(1)));
        assert!(!references_rotation(&[event(7)], &event(1)));
        assert!(!references_rotation(&[], &event(1)));
    }

    #[test]
    fn recovery_resolves_a_conflicted_head() {
        // The compromised generation-0 key signs two rival routine
        // rotations; the committed recovery key's rotation from the
        // same predecessor resolves the fork and neither rival
        // settles.
        let rival_a = rotation(RotationMode::Routine, 0, None, &gen1_key(), None);
        let rival_b = rotation(RotationMode::Routine, 0, None, &gen2_key(), None);
        let recovery = rotation(
            RotationMode::CompromiseRecovery,
            0,
            None,
            &gen3_key(),
            Some(event(50)),
        );
        let chain = fold_rotation_chain(
            &issuer_id(&gen0_key()),
            &currency(),
            Some(&commitment()),
            &[
                (issuer_id(&gen0_key()), event(1), rival_a),
                (issuer_id(&gen0_key()), event(2), rival_b),
                (issuer_id(&recovery_key()), event(3), recovery),
            ],
        );
        assert_eq!(chain.status, ChainStatus::Settled);
        assert_eq!(chain.active_generation(), 1);
        assert_eq!(chain.active_key(), &issuer_id(&gen3_key()));
        assert_eq!(chain.voided_transitions.len(), 2);
        assert!(chain.voided_transitions.contains(&event(1)));
        assert!(chain.voided_transitions.contains(&event(2)));
        assert_eq!(
            chain.recoveries,
            vec![RecoveryStep {
                generation: 0,
                event_id: event(3),
                asserted_compromise_from: Some(event(50)),
            }]
        );
    }

    #[test]
    fn a_recovery_racing_a_routine_rotation_wins() {
        let routine = rotation(RotationMode::Routine, 0, None, &gen1_key(), None);
        let recovery = rotation(RotationMode::CompromiseRecovery, 0, None, &gen2_key(), None);
        let chain = fold_rotation_chain(
            &issuer_id(&gen0_key()),
            &currency(),
            Some(&commitment()),
            &[
                (issuer_id(&gen0_key()), event(1), routine),
                (issuer_id(&recovery_key()), event(2), recovery),
            ],
        );
        assert_eq!(chain.status, ChainStatus::Settled);
        assert_eq!(chain.active_key(), &issuer_id(&gen2_key()));
        assert_eq!(chain.voided_transitions, vec![event(1)]);
    }

    #[test]
    fn two_recovery_transitions_still_conflict() {
        let first = rotation(RotationMode::CompromiseRecovery, 0, None, &gen1_key(), None);
        let second = rotation(RotationMode::CompromiseRecovery, 0, None, &gen2_key(), None);
        let chain = fold_rotation_chain(
            &issuer_id(&gen0_key()),
            &currency(),
            Some(&commitment()),
            &[
                (issuer_id(&recovery_key()), event(1), first),
                (issuer_id(&recovery_key()), event(2), second),
            ],
        );
        assert_eq!(chain.active_generation(), 0);
        assert!(matches!(chain.status, ChainStatus::Conflicted { .. }));
        assert!(chain.voided_transitions.is_empty());
    }

    #[test]
    fn signed_events_round_trip_through_the_fold() {
        let issuer = gen0_key();
        let first = rotation(RotationMode::Routine, 0, None, &gen1_key(), None);
        let signed = sign_rotation(&issuer, 0, None, &first, vec![]).expect("sign rotation");
        let decoded = chain::decode(&signed.bytes).expect("decode rotation");
        assert_eq!(author_id(&decoded), issuer_id(&issuer));
        // The rotation is recovered from the decoded event's signed
        // extension alone — the only part of an event the store
        // persists — never from the original object.
        let carried = decoded
            .header
            .extensions
            .rotation
            .clone()
            .expect("rotation in extension");
        assert_eq!(carried, first);
        let chain = fold_rotation_chain(
            &issuer_id(&issuer),
            &currency(),
            Some(&commitment()),
            &[(author_id(&decoded), signed.id, carried)],
        );
        assert_eq!(chain.active_generation(), 1);
        assert_eq!(chain.active_key(), &issuer_id(&gen1_key()));
    }

    #[test]
    fn routine_self_rotation_is_refused_at_sign_time() {
        // #6451 oracle: fold_rotation_chain can never count a
        // routine transition whose successor is the current key
        // (see a_routine_rotation_to_the_same_key_does_not_advance),
        // so sign_rotation must refuse it up front, naming the rule,
        // instead of producing a valid-looking event.
        let issuer = gen0_key();
        let self_rotation = rotation(RotationMode::Routine, 0, None, &gen0_key(), None);
        match sign_rotation(&issuer, 0, None, &self_rotation, vec![]) {
            Ok(_) => panic!("routine self-rotation signed Ok"),
            Err(err) => assert!(
                matches!(
                    err,
                    SignError::Rotation(RotationError::RoutineSuccessorIsAuthor)
                ),
                "unexpected sign error: {err}"
            ),
        }
    }

    #[test]
    fn validate_refuses_a_routine_self_rotation() {
        // The same author-local invariant in validate: a routine
        // rotation whose successor equals its verified author.
        let self_rotation = rotation(RotationMode::Routine, 0, None, &gen0_key(), None);
        assert_eq!(
            self_rotation.validate(&issuer_id(&gen0_key()), Some(&commitment())),
            Err(RotationError::RoutineSuccessorIsAuthor)
        );
    }

    #[test]
    fn routine_rotation_to_a_new_key_still_signs_and_folds() {
        // Control: the ordinary gen0 -> gen1 routine rotation is
        // unaffected by the self-rotation refusal.
        let issuer = gen0_key();
        let first = rotation(RotationMode::Routine, 0, None, &gen1_key(), None);
        let signed = sign_rotation(&issuer, 0, None, &first, vec![]).expect("sign rotation");
        let chain = fold_rotation_chain(
            &issuer_id(&issuer),
            &currency(),
            Some(&commitment()),
            &[(issuer_id(&issuer), signed.id, first)],
        );
        assert_eq!(chain.active_generation(), 1);
        assert_eq!(chain.active_key(), &issuer_id(&gen1_key()));
    }

    #[test]
    fn recovery_rotation_to_the_recovery_key_itself_still_signs_and_validates() {
        // Out of scope for the refusal (#6451): sign_rotation's
        // inputs do not name the key a recovery replaces, so a
        // CompromiseRecovery rotation whose successor is the
        // recovery signing key itself stays accepted at sign time
        // and in validate; only the fold decides advancement.
        let recovery = recovery_key();
        let rotation = rotation(
            RotationMode::CompromiseRecovery,
            0,
            None,
            &recovery_key(),
            None,
        );
        assert_eq!(
            rotation.validate(&issuer_id(&recovery_key()), Some(&commitment())),
            Ok(())
        );
        match sign_rotation(&recovery, 0, None, &rotation, vec![]) {
            Ok(_) => {}
            Err(err) => panic!("recovery self-install refused at sign time: {err}"),
        }
    }
}
