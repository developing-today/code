//! Mutual cross-signed key links (slice A-GRAPH-9132-02, #10240; design
//! part A §11, statements S5.58, S5.59, S5.60, S5.63).
//!
//! A [`LinkChallenge`] is one statement that two keys may both sign: the
//! ordered key pair, a purpose, an exact [`Scope`], a nonce and an expiry.
//! Its canonical bytes are the challenge. Each key signs an envelope over
//! exactly those bytes, so two statements pair only when every field is
//! identical: a different purpose, scope, nonce or expiry is a different
//! challenge, and unrelated one-sided statements never combine.
//!
//! [`KeyLinks`] collects verified statements and revocations and reports a
//! [`LinkStatus`] per challenge: `Pending` until both keys have signed,
//! `Mutual` once both have, and `Expired` or `Revoked` after that ends.
//! Either key may revoke. A mutual link proves only that both keys
//! approved that statement. It is not evidence that the keys belong to one
//! person, and this module offers no way to merge accounts, identities or
//! authority from it: a status is the only output. Authorization stays
//! with delegation grants (`graph/delegation.rs`), which never consult it.

use std::collections::{HashMap, HashSet};
use std::fmt;

use ed25519_dalek::SigningKey;

use super::delegation::{DELEGATION_PROFILE, Profile, Scope};
use super::term::{ObjectRef, Term};
use crate::canonical_cbor::{Value, decode_canonical, encode_canonical};
use crate::hash_profile::ProfileDispatch;
use crate::id_shape::{ContentId, HashProfile};
use crate::signed_envelope::{Body, Envelope, EnvelopeError, PayloadId, SUITE_ED25519};

/// Signing domain of a link statement.
pub const LINK_DOMAIN: &str = "id/key-link";
/// Signing domain of a link revocation.
pub const LINK_REVOCATION_DOMAIN: &str = "id/key-link-revocation";
/// Signing context (record profile) of both record types.
pub const LINK_PROFILE: &str = DELEGATION_PROFILE;
/// Payload version.
pub const LINK_VERSION: u64 = 1;
/// Longest purpose label, in bytes.
pub const MAX_PURPOSE_BYTES: usize = 64;
/// Longest nonce, in bytes.
pub const MAX_NONCE_BYTES: usize = 64;

/// Profile that names challenges and payloads.
pub const ID_PROFILE: HashProfile = HashProfile::RawBlake3_256;

/// The statement both keys sign.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkChallenge {
    first_key: [u8; 32],
    second_key: [u8; 32],
    /// What the link is for, such as `"same-owner"`.
    pub purpose: String,
    /// Where the link applies.
    pub scope: Scope,
    /// A nonce both parties saw.
    pub nonce: Vec<u8>,
    /// Latest valid time, UTC Unix seconds.
    pub expires: u64,
}

/// The identity of a challenge: the hash of its canonical bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChallengeId(ContentId);

/// Where one challenge stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkStatus {
    /// No statement for this challenge is known.
    Unknown,
    /// Only this key has signed.
    Pending {
        /// The key that signed.
        signed_by: [u8; 32],
    },
    /// Both keys signed the identical challenge.
    Mutual,
    /// The challenge expired.
    Expired,
    /// One of the keys revoked the challenge.
    Revoked,
}

/// Why a record was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    /// The record is not well formed.
    Malformed(&'static str),
    /// The envelope does not verify.
    Envelope(EnvelopeError),
    /// The record was signed for another purpose or profile.
    WrongDomain(String),
    /// The signer is not one of the challenge's two keys.
    WrongSigner,
    /// Both keys of the pair are the same.
    SameKey,
    /// A revocation names a challenge that is not known.
    UnknownChallenge,
}

impl LinkError {
    /// A stable failure class name.
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::Malformed(_) => "malformed",
            Self::Envelope(_) => "bad_signature",
            Self::WrongDomain(_) => "wrong_domain",
            Self::WrongSigner => "wrong_signer",
            Self::SameKey => "same_key",
            Self::UnknownChallenge => "unknown_challenge",
        }
    }
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(why) => write!(f, "malformed key-link record: {why}"),
            Self::Envelope(error) => write!(f, "key-link record: {error}"),
            Self::WrongDomain(domain) => write!(f, "record domain {domain:?} is not expected here"),
            other => f.write_str(other.class()),
        }
    }
}

impl std::error::Error for LinkError {}

impl From<EnvelopeError> for LinkError {
    fn from(error: EnvelopeError) -> Self {
        Self::Envelope(error)
    }
}

fn key_from(value: &Value) -> Result<[u8; 32], LinkError> {
    match value {
        Value::Bytes(bytes) => {
            <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_error| LinkError::Malformed("key"))
        }
        _ => Err(LinkError::Malformed("key")),
    }
}

fn require_canonical(payload: &[u8], value: &Value) -> Result<(), LinkError> {
    match encode_canonical(value) {
        Ok(bytes) if bytes == payload => Ok(()),
        _ => Err(LinkError::Malformed("payload is not canonical")),
    }
}

impl LinkChallenge {
    /// A challenge for two keys, stored in ascending key order so both
    /// parties build the same bytes.
    ///
    /// # Errors
    ///
    /// [`LinkError::SameKey`] for one key twice, or
    /// [`LinkError::Malformed`] for an empty or long purpose or nonce.
    pub fn new(
        a: [u8; 32],
        b: [u8; 32],
        purpose: &str,
        scope: Scope,
        nonce: Vec<u8>,
        expires: u64,
    ) -> Result<Self, LinkError> {
        if a == b {
            return Err(LinkError::SameKey);
        }
        if purpose.is_empty() || purpose.len() > MAX_PURPOSE_BYTES {
            return Err(LinkError::Malformed("purpose must be 1 to 64 bytes"));
        }
        if nonce.is_empty() || nonce.len() > MAX_NONCE_BYTES {
            return Err(LinkError::Malformed("nonce must be 1 to 64 bytes"));
        }
        Ok(Self {
            first_key: a.min(b),
            second_key: a.max(b),
            purpose: purpose.to_owned(),
            scope,
            nonce,
            expires,
        })
    }

    /// The two keys, in ascending order.
    #[must_use]
    pub const fn keys(&self) -> (&[u8; 32], &[u8; 32]) {
        (&self.first_key, &self.second_key)
    }

    fn to_value(&self) -> Result<Value, LinkError> {
        Ok(Value::Array(vec![
            Value::UInt(LINK_VERSION),
            Value::Text(LINK_DOMAIN.to_owned()),
            Value::Bytes(self.first_key.to_vec()),
            Value::Bytes(self.second_key.to_vec()),
            Value::Text(self.purpose.clone()),
            self.scope
                .to_value()
                .map_err(|_error| LinkError::Malformed("scope"))?,
            Value::Bytes(self.nonce.clone()),
            Value::UInt(self.expires),
        ]))
    }

    /// The canonical challenge bytes both keys sign.
    ///
    /// # Errors
    ///
    /// [`LinkError::Malformed`] when a field cannot be encoded.
    pub fn challenge_bytes(&self) -> Result<Vec<u8>, LinkError> {
        encode_canonical(&self.to_value()?).map_err(|_error| LinkError::Malformed("encoding"))
    }

    /// The challenge's identity.
    ///
    /// # Errors
    ///
    /// As [`Self::challenge_bytes`].
    pub fn id(&self) -> Result<ChallengeId, LinkError> {
        let bytes = self.challenge_bytes()?;
        let payload_id = PayloadId::of(ID_PROFILE, &bytes).map_err(EnvelopeError::from)?;
        Ok(ChallengeId(*payload_id.content_id()))
    }

    /// Decodes a challenge from its canonical bytes.
    ///
    /// # Errors
    ///
    /// [`LinkError`] for any other shape, keys out of order or equal, or
    /// a non-canonical encoding (which also covers another version or
    /// domain label).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LinkError> {
        let Ok(Value::Array(items)) = decode_canonical(bytes) else {
            return Err(LinkError::Malformed("challenge must be an array"));
        };
        let [
            _,
            _,
            first,
            second,
            Value::Text(purpose),
            scope,
            Value::Bytes(nonce),
            Value::UInt(expires),
        ] = items.as_slice()
        else {
            return Err(LinkError::Malformed("challenge shape"));
        };
        let (first, second) = (key_from(first)?, key_from(second)?);
        if first >= second {
            return Err(LinkError::Malformed("keys must be in ascending order"));
        }
        let scope = Scope::from_value(scope).map_err(|_error| LinkError::Malformed("scope"))?;
        let challenge = Self::new(first, second, purpose, scope, nonce.clone(), *expires)?;
        require_canonical(bytes, &challenge.to_value()?)?;
        Ok(challenge)
    }

    /// One key's signed statement of this challenge.
    ///
    /// # Errors
    ///
    /// [`LinkError::WrongSigner`] when `key` is neither of the pair, or
    /// an encoding or signing error.
    pub fn sign(&self, signer_id: &str, key: &SigningKey) -> Result<Envelope, LinkError> {
        let public = key.verifying_key().to_bytes();
        if public != self.first_key && public != self.second_key {
            return Err(LinkError::WrongSigner);
        }
        sign_record(LINK_DOMAIN, self.challenge_bytes()?, signer_id, key)
    }

    const fn expired(&self, utc: u64, skew: u64) -> bool {
        self.expires.saturating_add(skew) < utc
    }
}

fn sign_record(
    domain: &str,
    payload: Vec<u8>,
    signer_id: &str,
    key: &SigningKey,
) -> Result<Envelope, LinkError> {
    let body = Body {
        domain: domain.to_owned(),
        context: LINK_PROFILE.to_owned(),
        subject: None,
        graph: None,
        predecessor: None,
        signer_id: signer_id.to_owned(),
        key: key.verifying_key().to_bytes(),
        suite: SUITE_ED25519.to_owned(),
        payload_id: PayloadId::of(ID_PROFILE, &payload).map_err(EnvelopeError::from)?,
        payload_length: payload.len() as u64,
    };
    Ok(Envelope::sign(body, payload, key)?)
}

fn check_record(
    envelope: &Envelope,
    domain: &str,
    dispatch: &ProfileDispatch,
) -> Result<(), LinkError> {
    if envelope.body.domain != domain {
        return Err(LinkError::WrongDomain(envelope.body.domain.clone()));
    }
    if envelope.body.context != LINK_PROFILE {
        return Err(LinkError::WrongDomain(envelope.body.context.clone()));
    }
    envelope.verify(dispatch)?;
    Ok(())
}

fn revocation_value(id: ChallengeId) -> Value {
    Value::Array(vec![
        Value::UInt(LINK_VERSION),
        Term::Object(ObjectRef::Content(id.0))
            .to_value()
            .unwrap_or(Value::Null),
    ])
}

/// A key's signed withdrawal of a challenge.
///
/// # Errors
///
/// An encoding or signing error.
pub fn sign_link_revocation(
    id: ChallengeId,
    signer_id: &str,
    key: &SigningKey,
) -> Result<Envelope, LinkError> {
    let payload = encode_canonical(&revocation_value(id))
        .map_err(|_error| LinkError::Malformed("revocation encoding"))?;
    sign_record(LINK_REVOCATION_DOMAIN, payload, signer_id, key)
}

/// Verified link statements and revocations.
#[derive(Debug, Default, Clone)]
pub struct KeyLinks {
    challenges: HashMap<ChallengeId, LinkChallenge>,
    signed: HashMap<ChallengeId, HashSet<[u8; 32]>>,
    revoked: HashSet<ChallengeId>,
}

impl KeyLinks {
    /// An empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Verifies and records one key's statement, returning its challenge.
    ///
    /// # Errors
    ///
    /// [`LinkError`] for a bad record or a signer outside the pair.
    pub fn admit_statement(
        &mut self,
        envelope: &Envelope,
        dispatch: &ProfileDispatch,
    ) -> Result<ChallengeId, LinkError> {
        check_record(envelope, LINK_DOMAIN, dispatch)?;
        let challenge = LinkChallenge::from_bytes(&envelope.payload)?;
        let signer = envelope.body.key;
        if signer != challenge.first_key && signer != challenge.second_key {
            return Err(LinkError::WrongSigner);
        }
        let id = ChallengeId(*envelope.body.payload_id.content_id());
        self.signed.entry(id).or_default().insert(signer);
        self.challenges.insert(id, challenge);
        Ok(id)
    }

    /// Verifies and records a revocation by either key of a challenge.
    ///
    /// # Errors
    ///
    /// [`LinkError`] for a bad record, an unknown challenge, or a signer
    /// outside the pair.
    pub fn admit_revocation(
        &mut self,
        envelope: &Envelope,
        dispatch: &ProfileDispatch,
    ) -> Result<ChallengeId, LinkError> {
        check_record(envelope, LINK_REVOCATION_DOMAIN, dispatch)?;
        let Ok(Value::Array(items)) = decode_canonical(&envelope.payload) else {
            return Err(LinkError::Malformed("revocation must be an array"));
        };
        let [_, target] = items.as_slice() else {
            return Err(LinkError::Malformed("revocation shape"));
        };
        let Ok(Term::Object(ObjectRef::Content(content))) = Term::from_value(target) else {
            return Err(LinkError::Malformed("revocation target"));
        };
        let id = ChallengeId(content);
        require_canonical(&envelope.payload, &revocation_value(id))?;
        let challenge = self
            .challenges
            .get(&id)
            .ok_or(LinkError::UnknownChallenge)?;
        let signer = envelope.body.key;
        if signer != challenge.first_key && signer != challenge.second_key {
            return Err(LinkError::WrongSigner);
        }
        self.revoked.insert(id);
        Ok(id)
    }

    /// The challenge recorded under `id`.
    #[must_use]
    pub fn challenge(&self, id: &ChallengeId) -> Option<&LinkChallenge> {
        self.challenges.get(id)
    }

    /// Where the challenge stands at verifier time `utc`.
    #[must_use]
    pub fn status(&self, id: &ChallengeId, utc: u64, profile: &Profile) -> LinkStatus {
        let Some(challenge) = self.challenges.get(id) else {
            return LinkStatus::Unknown;
        };
        if self.revoked.contains(id) {
            return LinkStatus::Revoked;
        }
        if challenge.expired(utc, profile.clock_skew_secs) {
            return LinkStatus::Expired;
        }
        let signed = &self.signed[id];
        if signed.contains(&challenge.first_key) && signed.contains(&challenge.second_key) {
            return LinkStatus::Mutual;
        }
        signed
            .iter()
            .next()
            .map_or(LinkStatus::Unknown, |signer| LinkStatus::Pending {
                signed_by: *signer,
            })
    }
}
