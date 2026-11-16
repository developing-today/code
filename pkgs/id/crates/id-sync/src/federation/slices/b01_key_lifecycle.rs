//! B01 key lifecycle — contract module (slice B01, #10183).
//!
//! Key lifecycle: rotation, generation overlap, and preserving
//! validity of data signed by a then-current key.
//!
//! The contract, in one paragraph: a signer's keys form an explicit
//! lineage of generations. Generation 1 is enrolled at a point in
//! time; each rotation enrolls the next generation and gives the
//! outgoing generation a bounded overlap window during which it may
//! still sign. Verification is always relative to the event's own
//! signing time: an event verifies — and is attributed to its
//! generation — when the signing key's generation window covers the
//! signing time and the signature itself is valid. Rotation therefore
//! never invalidates history: an event signed under a then-current
//! key keeps verifying, with its original attribution, forever. What
//! rotation ends is the old key's authority to sign *new* events once
//! its overlap window closes.
//!
//! Times are logical seconds supplied by the caller (the module never
//! reads a clock), which keeps the state machine deterministic and
//! independently testable. Signature verification is Ed25519
//! (ed25519-dalek, strict). This module is deliberately
//! self-contained: the federation roots record slice paths without
//! declaring them (slice 9110-02), so the slice's integration test
//! includes this file directly and no shared root names it yet;
//! #9110-09 wires versioned public exports after module outputs land.

use ed25519_dalek::{Signature, VerifyingKey};

/// A generation number within one signer's key lineage. Generations
/// start at 1 and increase by exactly 1 per rotation.
pub type Generation = u64;

/// Logical seconds, supplied by the caller.
pub type LogicalTime = u64;

/// One generation of a signer's key: the verifying key, the window
/// during which it may sign, and its place in the lineage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyGeneration {
    pub generation: Generation,
    /// Ed25519 verifying-key bytes; also the key's identity.
    pub verifying_key: [u8; 32],
    /// First logical second at which this generation may sign
    /// (inclusive).
    pub valid_from: LogicalTime,
    /// First logical second at which this generation may no longer
    /// sign (exclusive). `None` while this is the current generation
    /// and no rotation has bounded it.
    pub valid_until: Option<LogicalTime>,
    /// The generation this one replaced, if any.
    pub supersedes: Option<Generation>,
    /// The generation that replaced this one, once rotated away.
    pub superseded_by: Option<Generation>,
}

impl KeyGeneration {
    /// Stable fingerprint of the verifying key: lowercase hex of the
    /// full key bytes. Attribution names a generation by number and
    /// fingerprint together, so two lineages can never blur.
    pub fn fingerprint(&self) -> String {
        let mut out = String::with_capacity(64);
        for b in self.verifying_key {
            out.push_str(&format!("{b:02x}"));
        }
        out
    }

    /// Whether this generation may sign at `at`.
    pub fn covers(&self, at: LogicalTime) -> bool {
        at >= self.valid_from && self.valid_until.map_or(true, |until| at < until)
    }
}

/// Successful verification: the event is authentic and attributable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribution {
    pub generation: Generation,
    pub fingerprint: String,
}

/// Why a lifecycle operation or verification failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleError {
    /// The lineage already has a generation 1.
    AlreadyEnrolled,
    /// Rotation attempted before any enrollment.
    NotEnrolled,
    /// The same verifying key is already a generation in this lineage.
    DuplicateKey,
    /// Rotation time is not strictly after the current generation's
    /// start; generations must move forward.
    NonMonotonicRotation {
        current_from: LogicalTime,
        at: LogicalTime,
    },
    /// No generation in the lineage holds this verifying key.
    UnknownKey,
    /// The key exists but its generation window does not cover the
    /// signing time (not yet valid, or past its overlap window).
    KeyNotValidAtTime {
        generation: Generation,
        at: LogicalTime,
    },
    /// The window covers the time but the signature does not verify.
    BadSignature,
}

impl std::fmt::Display for LifecycleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyEnrolled => write!(f, "lineage already enrolled"),
            Self::NotEnrolled => write!(f, "lineage not enrolled"),
            Self::DuplicateKey => write!(f, "verifying key already in lineage"),
            Self::NonMonotonicRotation { current_from, at } => write!(
                f,
                "rotation at {at} is not after current generation start {current_from}"
            ),
            Self::UnknownKey => write!(f, "verifying key not in lineage"),
            Self::KeyNotValidAtTime { generation, at } => write!(
                f,
                "generation {generation} does not cover signing time {at}"
            ),
            Self::BadSignature => write!(f, "signature does not verify"),
        }
    }
}

impl std::error::Error for LifecycleError {}

/// One signer's key lineage: the full, explicit generation chain.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyLineage {
    generations: Vec<KeyGeneration>,
}

impl KeyLineage {
    pub fn new() -> Self {
        Self::default()
    }

    /// The explicit lineage, oldest generation first. Every entry
    /// names its predecessor and successor, so the chain can be
    /// audited without trusting this type's internals.
    pub fn generations(&self) -> &[KeyGeneration] {
        &self.generations
    }

    /// The current (newest) generation, if any.
    pub fn current(&self) -> Option<&KeyGeneration> {
        self.generations.last()
    }

    fn find_key(&self, verifying_key: &[u8; 32]) -> Option<&KeyGeneration> {
        self.generations
            .iter()
            .find(|g| &g.verifying_key == verifying_key)
    }

    /// Enroll generation 1 for a signer, valid from `at` with no
    /// scheduled end. Fails if the lineage is already enrolled.
    pub fn enroll(
        &mut self,
        verifying_key: [u8; 32],
        at: LogicalTime,
    ) -> Result<Generation, LifecycleError> {
        if !self.generations.is_empty() {
            return Err(LifecycleError::AlreadyEnrolled);
        }
        self.generations.push(KeyGeneration {
            generation: 1,
            verifying_key,
            valid_from: at,
            valid_until: None,
            supersedes: None,
            superseded_by: None,
        });
        Ok(1)
    }

    /// Rotate to a new key at time `at`. The outgoing generation
    /// keeps signing authority until `at + overlap` (its overlap
    /// window); the new generation signs from `at`. History is
    /// untouched: events the outgoing key signed while it was valid
    /// keep their window and their attribution.
    pub fn rotate(
        &mut self,
        new_key: [u8; 32],
        at: LogicalTime,
        overlap: LogicalTime,
    ) -> Result<Generation, LifecycleError> {
        let current = self.generations.last().ok_or(LifecycleError::NotEnrolled)?;
        if at <= current.valid_from {
            return Err(LifecycleError::NonMonotonicRotation {
                current_from: current.valid_from,
                at,
            });
        }
        if self.find_key(&new_key).is_some() {
            return Err(LifecycleError::DuplicateKey);
        }
        let outgoing = self
            .generations
            .last_mut()
            .ok_or(LifecycleError::NotEnrolled)?;
        outgoing.valid_until = Some(at.saturating_add(overlap));
        outgoing.superseded_by = Some(outgoing.generation + 1);
        let generation = outgoing.generation + 1;
        let supersedes = outgoing.generation;
        self.generations.push(KeyGeneration {
            generation,
            verifying_key: new_key,
            valid_from: at,
            valid_until: None,
            supersedes: Some(supersedes),
            superseded_by: None,
        });
        Ok(generation)
    }

    /// Verify an event: `signature` over `message`, produced by
    /// `verifying_key` at signing time `at`. Succeeds only when the
    /// key is in the lineage, its generation window covers `at`, and
    /// the signature verifies; the returned attribution names the
    /// exact generation, so an event signed under a prior — but
    /// then-current — key still attributes to that prior generation
    /// after any number of later rotations.
    pub fn verify_at(
        &self,
        verifying_key: &[u8; 32],
        message: &[u8],
        signature: &[u8; 64],
        at: LogicalTime,
    ) -> Result<Attribution, LifecycleError> {
        let record = self
            .find_key(verifying_key)
            .ok_or(LifecycleError::UnknownKey)?;
        if !record.covers(at) {
            return Err(LifecycleError::KeyNotValidAtTime {
                generation: record.generation,
                at,
            });
        }
        let key =
            VerifyingKey::from_bytes(verifying_key).map_err(|_| LifecycleError::BadSignature)?;
        let sig = Signature::from_bytes(signature);
        key.verify_strict(message, &sig)
            .map_err(|_| LifecycleError::BadSignature)?;
        Ok(Attribution {
            generation: record.generation,
            fingerprint: record.fingerprint(),
        })
    }
}
