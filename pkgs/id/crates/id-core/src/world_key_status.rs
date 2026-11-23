//! Signer key history: rotation and revocation resolution for signer-local
//! world definitions (slice A9124-world-key-history, issue #10283; design
//! A-9124.3, statement S4.93).
//!
//! A signer's keys form an explicit, signed history: a key is introduced,
//! may be rotated into a signed successor, and may be revoked (retired or
//! compromised).  Verification keeps three dimensions separate, as the
//! design requires: cryptographic validity of a signature is evaluated
//! against the key's *historical* state at signing time, currentness is
//! whether the key is the signer's current authority *now*, and verifier
//! trust is a local policy input that neither dimension can manufacture.
//! Unknown key status is never reported as valid, and revocation never
//! silently rebinds an old key's history to a different key.
//!
//! The crate-root export belongs to the separate crate-wiring slice.  The
//! module therefore uses the same `crate::` leaf paths as the other
//! extracted ID slices and can be included by path by its integration test.

use std::collections::BTreeMap;
use std::fmt;

use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};

use crate::canonical_cbor::{CborError, Value, decode_canonical, encode_canonical};

/// Domain separation for the key-history signature preimage.
pub const KEY_HISTORY_DOMAIN: &str = "id-world-key-history-v1";

/// The only record version currently admitted.
pub const RECORD_VERSION: u64 = 1;

const SIGNATURE_PREFIX: &[u8] = b"id/world-key-history/v1\0";
const MAX_LABEL_BYTES: usize = 256;
const RECORD_FIELDS: &[&str] = &[
    "v",
    "domain",
    "signer_id",
    "signer_key_id",
    "kind",
    "key",
    "successor_key_id",
    "successor_key",
    "reason",
    "effective_at",
    "sequence",
    "author_key",
    "signature",
];

/// The kind of a key-history event.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum KeyEventKind {
    /// A key becomes known to the history, signed by the key itself.
    Introduced,
    /// The subject key names a signed successor and stops being current.
    Rotated,
    /// The subject key is revoked with a recorded reason.
    Revoked,
}

impl KeyEventKind {
    /// The canonical record token.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Introduced => "introduced",
            Self::Rotated => "rotated",
            Self::Revoked => "revoked",
        }
    }

    fn from_token(token: &str) -> Result<Self, KeyStatusError> {
        match token {
            "introduced" => Ok(Self::Introduced),
            "rotated" => Ok(Self::Rotated),
            "revoked" => Ok(Self::Revoked),
            _ => Err(KeyStatusError::Field {
                field: "kind",
                problem: "is not a known event kind",
            }),
        }
    }
}

/// Why a key was revoked.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RevocationReason {
    /// Planned end of the key's service; history before revocation stands.
    Retired,
    /// The key is believed compromised from the revocation's effective
    /// time; signatures at or after that time lose historical validity.
    Compromised,
}

impl RevocationReason {
    /// The canonical record token.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Retired => "retired",
            Self::Compromised => "compromised",
        }
    }

    fn from_token(token: &str) -> Result<Self, KeyStatusError> {
        match token {
            "retired" => Ok(Self::Retired),
            "compromised" => Ok(Self::Compromised),
            _ => Err(KeyStatusError::Field {
                field: "reason",
                problem: "is not a known revocation reason",
            }),
        }
    }
}

/// A key's status at the head of the history.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum KeyStatus {
    /// The history records no introduction for this key id.  Unknown is
    /// never a form of valid.
    Unknown,
    /// Introduced and neither rotated nor revoked: the current authority.
    Active,
    /// Superseded by a signed successor event; historically meaningful,
    /// no longer current.
    Rotated,
    /// Revoked as retired; no longer current.
    Revoked,
    /// Revoked as compromised; no longer current, and signatures from the
    /// compromise effective time onward lose historical validity.
    Compromised,
}

impl KeyStatus {
    /// The canonical status token.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Active => "active",
            Self::Rotated => "rotated",
            Self::Revoked => "revoked",
            Self::Compromised => "compromised",
        }
    }
}

/// Whether a signature made at a stated time falls inside the key's
/// historical validity window.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum HistoricalValidity {
    /// The key was introduced and not yet rotated or revoked at that time.
    Valid,
    /// The key is known, but the time is outside its validity window.
    OutsideWindow,
    /// The history records no such key; never a form of valid.
    UnknownKey,
}

/// The verifier-local trust dimension, supplied by the caller's policy.
/// It is reported as its own dimension and never feeds the cryptographic
/// or currentness dimensions.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TrustPolicyResult {
    /// The local policy trusts this signer.
    Trusted,
    /// The local policy does not trust this signer.
    Untrusted,
}

/// The separated verification dimensions for one key and one signing time
/// (design A-9124.3): historical signature validity, current key status
/// and verifier trust are independent outputs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyStatusReport {
    /// Signer-local identity label the history belongs to.
    pub signer_id: String,
    /// Key-history identity label that was evaluated.
    pub signer_key_id: String,
    /// Current status of the key at the head of the history.
    pub key_status: KeyStatus,
    /// Whether a signature made at the evaluated time is historically
    /// valid for this key.  Always false for an unknown key.
    pub historical_signature_valid: bool,
    /// Whether the key is the signer's current authority now.
    pub currently_authorized: bool,
    /// The verifier-local trust decision, echoed as its own dimension.
    pub trust_policy_result: TrustPolicyResult,
}

/// Why a key-history event or record was rejected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeyStatusError {
    /// Canonical CBOR failed.
    Cbor(CborError),
    /// A field is absent or malformed.
    Field {
        /// Field name.
        field: &'static str,
        /// Stable reason.
        problem: &'static str,
    },
    /// A future record version was supplied.
    UnsupportedVersion(u64),
    /// The signed bytes do not verify under the author key.
    BadSignature,
    /// The author key is not a valid Ed25519 key.
    BadKey,
    /// The event was signed by a key not authorized to speak for its
    /// subject, or names a different signer than the history.
    SignerMismatch,
    /// The subject key id has no introduction in the history.
    UnknownKey {
        /// Key id that was not found.
        key_id: String,
    },
    /// The key id is already recorded in the history.
    DuplicateKey {
        /// Key id that was already present.
        key_id: String,
    },
    /// The event's key bytes do not match the recorded key for its id.
    KeyMismatch {
        /// Key id whose bytes disagreed.
        key_id: String,
    },
    /// The subject key is rotated or revoked and can no longer rotate.
    KeyNotActive {
        /// Key id that is no longer active.
        key_id: String,
    },
    /// The subject key already carries a revocation.
    AlreadyRevoked {
        /// Key id that was already revoked.
        key_id: String,
    },
    /// The event sequence is not the next expected sequence.
    OutOfOrder {
        /// Sequence the history expected.
        expected: u64,
        /// Sequence the event carried.
        found: u64,
    },
}

impl KeyStatusError {
    /// Stable error class for protocol adapters and tests.
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::Cbor(_) | Self::Field { .. } => "malformed",
            Self::UnsupportedVersion(_) => "unsupported_version",
            Self::BadSignature => "bad_signature",
            Self::BadKey => "bad_key",
            Self::SignerMismatch => "signer_mismatch",
            Self::UnknownKey { .. } => "unknown_key",
            Self::DuplicateKey { .. } => "duplicate_key",
            Self::KeyMismatch { .. } => "key_mismatch",
            Self::KeyNotActive { .. } => "key_not_active",
            Self::AlreadyRevoked { .. } => "already_revoked",
            Self::OutOfOrder { .. } => "out_of_order",
        }
    }
}

impl fmt::Display for KeyStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cbor(error) => write!(f, "key history encoding: {error}"),
            Self::Field { field, problem } => write!(f, "field {field}: {problem}"),
            Self::UnsupportedVersion(version) => {
                write!(f, "key history version {version} is unsupported")
            }
            Self::BadSignature => f.write_str("key history signature is invalid"),
            Self::BadKey => f.write_str("key history author key is invalid"),
            Self::SignerMismatch => {
                f.write_str("key history event signer is not authorized for its subject")
            }
            Self::UnknownKey { key_id } => write!(f, "key {key_id:?} is not in the history"),
            Self::DuplicateKey { key_id } => {
                write!(f, "key {key_id:?} is already in the history")
            }
            Self::KeyMismatch { key_id } => {
                write!(f, "key bytes for {key_id:?} do not match the history")
            }
            Self::KeyNotActive { key_id } => write!(f, "key {key_id:?} is no longer active"),
            Self::AlreadyRevoked { key_id } => write!(f, "key {key_id:?} is already revoked"),
            Self::OutOfOrder { expected, found } => {
                write!(
                    f,
                    "key history sequence {found} is not the expected {expected}"
                )
            }
        }
    }
}

impl std::error::Error for KeyStatusError {}

impl From<CborError> for KeyStatusError {
    fn from(error: CborError) -> Self {
        Self::Cbor(error)
    }
}

/// One unsigned key-history event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyHistoryEvent {
    /// Signer-local identity label.
    pub signer_id: String,
    /// Subject key-history identity label.
    pub signer_key_id: String,
    /// What this event does to the subject key.
    pub kind: KeyEventKind,
    /// Subject key bytes.  For an introduction this is the new key; for a
    /// rotation or revocation it must match the recorded key bytes.
    pub key: [u8; 32],
    /// Successor key id; required for a rotation, absent otherwise.
    pub successor_key_id: Option<String>,
    /// Successor key bytes; required for a rotation, absent otherwise.
    pub successor_key: Option<[u8; 32]>,
    /// Revocation reason; required for a revocation, absent otherwise.
    pub reason: Option<RevocationReason>,
    /// Time the event takes effect, in the declared ID time unit.  A
    /// compromise revocation may be effective before earlier events were
    /// recorded; introductions and rotations are chain-monotonic.
    pub effective_at: u64,
    /// Position in the signer's history, starting at one with no gaps.
    pub sequence: u64,
}

impl KeyHistoryEvent {
    fn validate(&self) -> Result<(), KeyStatusError> {
        for (field, value) in [
            ("signer_id", self.signer_id.as_str()),
            ("signer_key_id", self.signer_key_id.as_str()),
        ] {
            check_label(field, value)?;
        }
        if self.effective_at == 0 {
            return Err(KeyStatusError::Field {
                field: "effective_at",
                problem: "must be positive",
            });
        }
        if self.sequence == 0 {
            return Err(KeyStatusError::Field {
                field: "sequence",
                problem: "must be positive",
            });
        }
        match self.kind {
            KeyEventKind::Introduced => {
                if self.successor_key_id.is_some() || self.successor_key.is_some() {
                    return Err(KeyStatusError::Field {
                        field: "successor_key_id",
                        problem: "must be absent for an introduction",
                    });
                }
                if self.reason.is_some() {
                    return Err(KeyStatusError::Field {
                        field: "reason",
                        problem: "must be absent for an introduction",
                    });
                }
            }
            KeyEventKind::Rotated => {
                let successor = self
                    .successor_key_id
                    .as_deref()
                    .ok_or(KeyStatusError::Field {
                        field: "successor_key_id",
                        problem: "is required for a rotation",
                    })?;
                check_label("successor_key_id", successor)?;
                if successor == self.signer_key_id {
                    return Err(KeyStatusError::Field {
                        field: "successor_key_id",
                        problem: "must differ from the rotated key",
                    });
                }
                if self.successor_key.is_none() {
                    return Err(KeyStatusError::Field {
                        field: "successor_key",
                        problem: "is required for a rotation",
                    });
                }
                if self.reason.is_some() {
                    return Err(KeyStatusError::Field {
                        field: "reason",
                        problem: "must be absent for a rotation",
                    });
                }
            }
            KeyEventKind::Revoked => {
                if self.reason.is_none() {
                    return Err(KeyStatusError::Field {
                        field: "reason",
                        problem: "is required for a revocation",
                    });
                }
                if self.successor_key_id.is_some() || self.successor_key.is_some() {
                    return Err(KeyStatusError::Field {
                        field: "successor_key_id",
                        problem: "must be absent for a revocation",
                    });
                }
            }
        }
        Ok(())
    }
}

/// A complete signed key-history event, including the author key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignedKeyHistoryEvent {
    /// The signed event.
    pub event: KeyHistoryEvent,
    /// Public key that signed the event.  Which key may authorize an
    /// event is a state-machine rule enforced by [`KeyHistory::apply`].
    pub author_key: [u8; 32],
    /// Domain-separated signature over the event and author key.
    pub signature: [u8; 64],
    domain: String,
}

impl SignedKeyHistoryEvent {
    /// Sign a new event with the author's Ed25519 key.
    pub fn sign(event: KeyHistoryEvent, key: &SigningKey) -> Result<Self, KeyStatusError> {
        event.validate()?;
        let author_key = key.verifying_key().to_bytes();
        let domain = KEY_HISTORY_DOMAIN.to_owned();
        let preimage = signature_preimage(&event, &author_key, &domain)?;
        let signature = key.sign(&preimage).to_bytes();
        Ok(Self {
            event,
            author_key,
            signature,
            domain,
        })
    }

    /// The domain string carried by this signed record.
    #[must_use]
    pub fn domain(&self) -> &str {
        &self.domain
    }

    /// Verify structure and the exact signature bytes under the author key.
    pub fn verify(&self) -> Result<(), KeyStatusError> {
        self.event.validate()?;
        if self.domain != KEY_HISTORY_DOMAIN {
            return Err(KeyStatusError::BadSignature);
        }
        let key =
            VerifyingKey::from_bytes(&self.author_key).map_err(|_error| KeyStatusError::BadKey)?;
        let signature = Signature::from_bytes(&self.signature);
        let preimage = signature_preimage(&self.event, &self.author_key, &self.domain)?;
        key.verify(&preimage, &signature)
            .map_err(|_error| KeyStatusError::BadSignature)
    }

    /// Encode the complete signed record as canonical CBOR.
    pub fn encode(&self) -> Result<Vec<u8>, KeyStatusError> {
        self.event.validate()?;
        Ok(encode_canonical(&self.to_value()?)?)
    }

    /// Decode a canonical signed record.  Call [`Self::verify`] before trust.
    pub fn decode(bytes: &[u8]) -> Result<Self, KeyStatusError> {
        let value = decode_canonical(bytes)?;
        Self::from_value(&value)
    }

    fn to_value(&self) -> Result<Value, KeyStatusError> {
        let event = &self.event;
        let mut entries = vec![
            ("v", Value::UInt(RECORD_VERSION)),
            ("domain", text(&self.domain)),
            ("signer_id", text(&event.signer_id)),
            ("signer_key_id", text(&event.signer_key_id)),
            ("kind", text(event.kind.token())),
            ("key", Value::Bytes(event.key.to_vec())),
            ("effective_at", Value::UInt(event.effective_at)),
            ("sequence", Value::UInt(event.sequence)),
            ("author_key", Value::Bytes(self.author_key.to_vec())),
            ("signature", Value::Bytes(self.signature.to_vec())),
        ];
        if let Some(successor) = &event.successor_key_id {
            entries.push(("successor_key_id", text(successor)));
        }
        if let Some(successor_key) = &event.successor_key {
            entries.push(("successor_key", Value::Bytes(successor_key.to_vec())));
        }
        if let Some(reason) = &event.reason {
            entries.push(("reason", text(reason.token())));
        }
        Ok(map(entries))
    }

    fn from_value(value: &Value) -> Result<Self, KeyStatusError> {
        let fields = Fields::new(value, RECORD_FIELDS)?;
        let version = fields.uint("v")?;
        if version != RECORD_VERSION {
            return Err(KeyStatusError::UnsupportedVersion(version));
        }
        let event = KeyHistoryEvent {
            signer_id: fields.text("signer_id")?,
            signer_key_id: fields.text("signer_key_id")?,
            kind: KeyEventKind::from_token(&fields.text("kind")?)?,
            key: fields.fixed_bytes::<32>("key")?,
            successor_key_id: fields
                .optional("successor_key_id")
                .map(|value| fields.text_value(value, "successor_key_id"))
                .transpose()?,
            successor_key: fields
                .optional("successor_key")
                .map(|value| fields.fixed_bytes_value::<32>(value, "successor_key"))
                .transpose()?,
            reason: fields
                .optional("reason")
                .map(|value| {
                    let token = fields.text_value(value, "reason")?;
                    RevocationReason::from_token(&token)
                })
                .transpose()?,
            effective_at: fields.uint("effective_at")?,
            sequence: fields.uint("sequence")?,
        };
        event.validate()?;
        Ok(Self {
            event,
            author_key: fields.fixed_bytes::<32>("author_key")?,
            signature: fields.fixed_bytes::<64>("signature")?,
            domain: fields.text("domain")?,
        })
    }

    #[cfg(test)]
    pub fn set_domain_for_test(&mut self, domain: &str) {
        self.domain = domain.to_owned();
    }
}

/// Recorded state of one key in the history.
#[derive(Clone, Debug, Eq, PartialEq)]
struct KeyRecord {
    key: [u8; 32],
    introduced_at: u64,
    rotated_at: Option<u64>,
    successor_key_id: Option<String>,
    revoked: Option<(u64, RevocationReason)>,
}

/// One signer's explicit key-history state machine.  Events fold in
/// sequence order; a rejected event changes no state.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct KeyHistory {
    signer_id: Option<String>,
    keys: BTreeMap<String, KeyRecord>,
    last_sequence: u64,
    chain_effective_at: u64,
}

impl KeyHistory {
    /// An empty history with no signer and no keys.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The signer this history belongs to, once any event has applied.
    #[must_use]
    pub fn signer_id(&self) -> Option<&str> {
        self.signer_id.as_deref()
    }

    /// The sequence the next event must carry.
    #[must_use]
    pub fn next_sequence(&self) -> u64 {
        self.last_sequence + 1
    }

    /// Verify and fold one signed event into the history.
    pub fn apply(&mut self, signed: &SignedKeyHistoryEvent) -> Result<(), KeyStatusError> {
        signed.verify()?;
        let event = &signed.event;
        if let Some(signer) = &self.signer_id
            && signer != &event.signer_id
        {
            return Err(KeyStatusError::SignerMismatch);
        }
        let expected = self.next_sequence();
        if event.sequence != expected {
            return Err(KeyStatusError::OutOfOrder {
                expected,
                found: event.sequence,
            });
        }
        match event.kind {
            KeyEventKind::Introduced => self.apply_introduction(signed)?,
            KeyEventKind::Rotated => self.apply_rotation(signed)?,
            KeyEventKind::Revoked => self.apply_revocation(signed)?,
        }
        if self.signer_id.is_none() {
            self.signer_id = Some(event.signer_id.clone());
        }
        self.last_sequence = event.sequence;
        Ok(())
    }

    fn apply_introduction(&mut self, signed: &SignedKeyHistoryEvent) -> Result<(), KeyStatusError> {
        let event = &signed.event;
        if self.keys.contains_key(&event.signer_key_id) {
            return Err(KeyStatusError::DuplicateKey {
                key_id: event.signer_key_id.clone(),
            });
        }
        if signed.author_key != event.key {
            return Err(KeyStatusError::SignerMismatch);
        }
        self.check_chain_time(event)?;
        self.keys.insert(
            event.signer_key_id.clone(),
            KeyRecord {
                key: event.key,
                introduced_at: event.effective_at,
                rotated_at: None,
                successor_key_id: None,
                revoked: None,
            },
        );
        self.chain_effective_at = event.effective_at;
        Ok(())
    }

    fn apply_rotation(&mut self, signed: &SignedKeyHistoryEvent) -> Result<(), KeyStatusError> {
        let event = &signed.event;
        let record = self.record(&event.signer_key_id)?;
        if record.rotated_at.is_some() || record.revoked.is_some() {
            return Err(KeyStatusError::KeyNotActive {
                key_id: event.signer_key_id.clone(),
            });
        }
        if record.key != event.key {
            return Err(KeyStatusError::KeyMismatch {
                key_id: event.signer_key_id.clone(),
            });
        }
        if signed.author_key != record.key {
            return Err(KeyStatusError::SignerMismatch);
        }
        self.check_chain_time(event)?;
        let successor_id = event
            .successor_key_id
            .clone()
            .ok_or(KeyStatusError::Field {
                field: "successor_key_id",
                problem: "is required for a rotation",
            })?;
        let successor_key = event.successor_key.ok_or(KeyStatusError::Field {
            field: "successor_key",
            problem: "is required for a rotation",
        })?;
        if self.keys.contains_key(&successor_id) {
            return Err(KeyStatusError::DuplicateKey {
                key_id: successor_id,
            });
        }
        let record =
            self.keys
                .get_mut(&event.signer_key_id)
                .ok_or_else(|| KeyStatusError::UnknownKey {
                    key_id: event.signer_key_id.clone(),
                })?;
        record.rotated_at = Some(event.effective_at);
        record.successor_key_id = Some(successor_id.clone());
        self.keys.insert(
            successor_id,
            KeyRecord {
                key: successor_key,
                introduced_at: event.effective_at,
                rotated_at: None,
                successor_key_id: None,
                revoked: None,
            },
        );
        self.chain_effective_at = event.effective_at;
        Ok(())
    }

    fn apply_revocation(&mut self, signed: &SignedKeyHistoryEvent) -> Result<(), KeyStatusError> {
        let event = &signed.event;
        let record = self.record(&event.signer_key_id)?;
        if record.revoked.is_some() {
            return Err(KeyStatusError::AlreadyRevoked {
                key_id: event.signer_key_id.clone(),
            });
        }
        if record.key != event.key {
            return Err(KeyStatusError::KeyMismatch {
                key_id: event.signer_key_id.clone(),
            });
        }
        let successor_key = record
            .successor_key_id
            .as_ref()
            .and_then(|id| self.keys.get(id))
            .map(|successor| successor.key);
        if signed.author_key != record.key && Some(signed.author_key) != successor_key {
            return Err(KeyStatusError::SignerMismatch);
        }
        if event.effective_at < record.introduced_at {
            return Err(KeyStatusError::Field {
                field: "effective_at",
                problem: "precedes the key introduction",
            });
        }
        let reason = event.reason.ok_or(KeyStatusError::Field {
            field: "reason",
            problem: "is required for a revocation",
        })?;
        let record =
            self.keys
                .get_mut(&event.signer_key_id)
                .ok_or_else(|| KeyStatusError::UnknownKey {
                    key_id: event.signer_key_id.clone(),
                })?;
        record.revoked = Some((event.effective_at, reason));
        Ok(())
    }

    fn record(&self, key_id: &str) -> Result<&KeyRecord, KeyStatusError> {
        self.keys
            .get(key_id)
            .ok_or_else(|| KeyStatusError::UnknownKey {
                key_id: key_id.to_owned(),
            })
    }

    fn check_chain_time(&self, event: &KeyHistoryEvent) -> Result<(), KeyStatusError> {
        if event.effective_at < self.chain_effective_at {
            return Err(KeyStatusError::Field {
                field: "effective_at",
                problem: "precedes the previous history event",
            });
        }
        Ok(())
    }

    /// The current status of one key at the head of the history.
    #[must_use]
    pub fn status_of(&self, key_id: &str) -> KeyStatus {
        let Some(record) = self.keys.get(key_id) else {
            return KeyStatus::Unknown;
        };
        match record.revoked {
            Some((_, RevocationReason::Compromised)) => KeyStatus::Compromised,
            Some((_, RevocationReason::Retired)) => KeyStatus::Revoked,
            None if record.rotated_at.is_some() => KeyStatus::Rotated,
            None => KeyStatus::Active,
        }
    }

    /// The recorded successor of a rotated key.  This is an explicit
    /// link, never a rebinding: the rotated key keeps its own history
    /// and status.
    #[must_use]
    pub fn successor_of(&self, key_id: &str) -> Option<&str> {
        self.keys
            .get(key_id)
            .and_then(|record| record.successor_key_id.as_deref())
    }

    /// The signer's current key: the unique key whose status is
    /// [`KeyStatus::Active`], or none when there is no such key or the
    /// history holds several independent active keys.
    #[must_use]
    pub fn current_key_id(&self) -> Option<&str> {
        let mut active = self
            .keys
            .iter()
            .filter(|(id, _)| self.status_of(id) == KeyStatus::Active);
        let first = active.next()?;
        if active.next().is_some() {
            return None;
        }
        Some(first.0.as_str())
    }

    /// Whether a signature made at `signed_at` falls inside the key's
    /// historical validity window: introduced by then, and before any
    /// rotation or revocation effective time.  Revocation and compromise
    /// invalidate current authority; they erase history only from their
    /// own effective time onward.
    #[must_use]
    pub fn historical_validity(&self, key_id: &str, signed_at: u64) -> HistoricalValidity {
        let Some(record) = self.keys.get(key_id) else {
            return HistoricalValidity::UnknownKey;
        };
        if signed_at < record.introduced_at {
            return HistoricalValidity::OutsideWindow;
        }
        if let Some(rotated_at) = record.rotated_at
            && signed_at >= rotated_at
        {
            return HistoricalValidity::OutsideWindow;
        }
        if let Some((revoked_at, _)) = record.revoked
            && signed_at >= revoked_at
        {
            return HistoricalValidity::OutsideWindow;
        }
        HistoricalValidity::Valid
    }

    /// Evaluate the separated verification dimensions for one key and one
    /// signing time.  `trust` is the verifier's own local policy result;
    /// it is reported as its own dimension and changes no other field.
    #[must_use]
    pub fn evaluate(
        &self,
        key_id: &str,
        signed_at: u64,
        trust: TrustPolicyResult,
    ) -> KeyStatusReport {
        let status = self.status_of(key_id);
        KeyStatusReport {
            signer_id: self.signer_id.clone().unwrap_or_default(),
            signer_key_id: key_id.to_owned(),
            key_status: status,
            historical_signature_valid: self.historical_validity(key_id, signed_at)
                == HistoricalValidity::Valid,
            currently_authorized: status == KeyStatus::Active,
            trust_policy_result: trust,
        }
    }
}

fn check_label(field: &'static str, value: &str) -> Result<(), KeyStatusError> {
    if value.is_empty() {
        return Err(KeyStatusError::Field {
            field,
            problem: "must not be empty",
        });
    }
    if value.len() > MAX_LABEL_BYTES {
        return Err(KeyStatusError::Field {
            field,
            problem: "is too long",
        });
    }
    Ok(())
}

fn signature_preimage(
    event: &KeyHistoryEvent,
    author_key: &[u8; 32],
    domain: &str,
) -> Result<Vec<u8>, KeyStatusError> {
    let body = map_event(event)?;
    let encoded = encode_canonical(&body)?;
    let domain_bytes = domain.as_bytes();
    let domain_len = u32::try_from(domain_bytes.len()).map_err(|_error| KeyStatusError::Field {
        field: "domain",
        problem: "is too long",
    })?;
    let mut out = Vec::with_capacity(
        SIGNATURE_PREFIX.len() + 4 + domain_bytes.len() + author_key.len() + encoded.len(),
    );
    out.extend_from_slice(SIGNATURE_PREFIX);
    out.extend_from_slice(&domain_len.to_be_bytes());
    out.extend_from_slice(domain_bytes);
    out.extend_from_slice(author_key);
    out.extend_from_slice(&encoded);
    Ok(out)
}

fn map_event(event: &KeyHistoryEvent) -> Result<Value, KeyStatusError> {
    let mut entries = vec![
        ("signer_id", text(&event.signer_id)),
        ("signer_key_id", text(&event.signer_key_id)),
        ("kind", text(event.kind.token())),
        ("key", Value::Bytes(event.key.to_vec())),
        ("effective_at", Value::UInt(event.effective_at)),
        ("sequence", Value::UInt(event.sequence)),
    ];
    if let Some(successor) = &event.successor_key_id {
        entries.push(("successor_key_id", text(successor)));
    }
    if let Some(successor_key) = &event.successor_key {
        entries.push(("successor_key", Value::Bytes(successor_key.to_vec())));
    }
    if let Some(reason) = &event.reason {
        entries.push(("reason", text(reason.token())));
    }
    Ok(map(entries))
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn map(mut entries: Vec<(&'static str, Value)>) -> Value {
    entries.sort_by(|(left, _), (right, _)| {
        left.len().cmp(&right.len()).then_with(|| left.cmp(right))
    });
    Value::Map(
        entries
            .into_iter()
            .map(|(key, value)| (text(key), value))
            .collect(),
    )
}

struct Fields<'a> {
    entries: &'a [(Value, Value)],
}

impl<'a> Fields<'a> {
    fn new(value: &'a Value, known: &[&str]) -> Result<Self, KeyStatusError> {
        let Value::Map(entries) = value else {
            return Err(KeyStatusError::Field {
                field: "record",
                problem: "must be a map",
            });
        };
        for (key, _) in entries {
            let Value::Text(name) = key else {
                return Err(KeyStatusError::Field {
                    field: "record",
                    problem: "keys must be text",
                });
            };
            if !known.contains(&name.as_str()) {
                return Err(KeyStatusError::Cbor(CborError::UnknownCriticalField {
                    field: name.clone(),
                }));
            }
        }
        Ok(Self { entries })
    }

    fn optional(&self, field: &str) -> Option<&'a Value> {
        self.entries
            .iter()
            .find(|(key, _)| matches!(key, Value::Text(name) if name == field))
            .map(|(_, value)| value)
    }

    fn required(&self, field: &'static str) -> Result<&'a Value, KeyStatusError> {
        self.optional(field).ok_or(KeyStatusError::Field {
            field,
            problem: "is missing",
        })
    }

    fn text(&self, field: &'static str) -> Result<String, KeyStatusError> {
        self.text_value(self.required(field)?, field)
    }

    fn text_value(&self, value: &Value, field: &'static str) -> Result<String, KeyStatusError> {
        match value {
            Value::Text(value) => Ok(value.clone()),
            _ => Err(KeyStatusError::Field {
                field,
                problem: "must be text",
            }),
        }
    }

    fn uint(&self, field: &'static str) -> Result<u64, KeyStatusError> {
        match self.required(field)? {
            Value::UInt(value) => Ok(*value),
            _ => Err(KeyStatusError::Field {
                field,
                problem: "must be an unsigned integer",
            }),
        }
    }

    fn fixed_bytes<const N: usize>(&self, field: &'static str) -> Result<[u8; N], KeyStatusError> {
        self.fixed_bytes_value(self.required(field)?, field)
    }

    fn fixed_bytes_value<const N: usize>(
        &self,
        value: &Value,
        field: &'static str,
    ) -> Result<[u8; N], KeyStatusError> {
        match value {
            Value::Bytes(bytes) => {
                bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_error| KeyStatusError::Field {
                        field,
                        problem: "has the wrong length",
                    })
            }
            _ => Err(KeyStatusError::Field {
                field,
                problem: "must be bytes",
            }),
        }
    }
}
