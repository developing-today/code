//! B04 signature suites — signature-suite agility and algorithm
//! deprecation/migration (slice B04, #10207; design A-9114,
//! Part B "Conformance and tests").
//!
//! The contract, in one paragraph: a new signed envelope names its
//! signature suite by a numeric id. A verifier holds an explicit,
//! versioned allowlist of suites it understands; the id is resolved
//! through that allowlist before any signature work happens, so an
//! unknown or not-allowed suite fails closed instead of falling
//! through to a default algorithm. For the one suite that exists
//! today — Ed25519 over a deterministic-CBOR body with a domain
//! prefix — the envelope is a canonical-CBOR map, the signature is
//! detached and covers `domain-prefix || canonical body`, and any
//! noncanonical encoding, malformed field, or bad signature rejects
//! with a typed error. Agility is therefore a verifier policy
//! change, never a wire change: a second suite is added by
//! registering it, and a deprecated suite is retired by removing it
//! from the allowlist, which makes every envelope naming it fail
//! closed (no silent fallback, no re-signing of existing bytes).
//!
//! This module is deliberately self-contained: the federation roots
//! record slice paths without declaring them (slice 9110-02), so the
//! slice's integration test includes this file directly and no shared
//! root names it yet; #9110-09 wires versioned public exports after
//! module outputs land.

use ed25519_dalek::{Signature, VerifyingKey};
use id_core::canonical_cbor::{CborError, Value, decode_canonical, encode_canonical};

/// The only suite defined so far: Ed25519 over a deterministic-CBOR
/// body with a domain prefix (design A-9114.3).
pub const SUITE_ED25519_DET_CBOR_V1: u16 = 1;

/// Ed25519 public-key length in bytes.
pub const ED25519_PUBLIC_KEY_LEN: usize = 32;

/// Ed25519 signature length in bytes.
pub const ED25519_SIGNATURE_LEN: usize = 64;

/// The stable domain prefix every new-profile signature covers.
/// Domain separation keeps a signature made for one protocol
/// context from verifying in another.
pub const SIGNATURE_DOMAIN_PREFIX: &[u8] = b"id/atlas/sig/v1";

/// Name of the one field excluded from the signed body.
const SIGNATURE_FIELD: &str = "signature";

/// One signature suite the program knows about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Suite {
    /// Numeric suite id carried by every envelope (`suite` field).
    pub id: u16,
    /// Stable human-readable name, for capability reports.
    pub name: &'static str,
}

/// The suites this program knows how to verify. Knowing a suite is
/// not the same as allowing it: policy lives in [`SuiteRegistry`].
pub const KNOWN_SUITES: &[Suite] = &[Suite {
    id: SUITE_ED25519_DET_CBOR_V1,
    name: "ed25519-det-cbor-v1",
}];

/// Look up a suite this program knows how to verify, by id.
pub fn known_suite(id: u16) -> Option<&'static Suite> {
    KNOWN_SUITES.iter().find(|suite| suite.id == id)
}

/// A verifier's allowlist. Envelopes naming a suite outside the
/// allowlist fail closed even when this program knows the suite,
/// which is how a deprecated suite is retired without touching a
/// single envelope's bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuiteRegistry {
    allowed: &'static [u16],
}

impl SuiteRegistry {
    /// The version-1 registry: exactly Ed25519 + deterministic CBOR.
    pub fn v1() -> Self {
        Self {
            allowed: &[SUITE_ED25519_DET_CBOR_V1],
        }
    }

    /// A registry over an explicit allowlist (migration and
    /// deprecation tests, and a deployment choosing its policy).
    pub fn with_allowlist(allowed: &'static [u16]) -> Self {
        Self { allowed }
    }

    /// Whether this registry will verify an envelope naming `id`.
    pub fn allows(&self, id: u16) -> bool {
        self.allowed.contains(&id)
    }

    /// The allowlist, in registry order.
    pub fn allowed(&self) -> &'static [u16] {
        self.allowed
    }

    /// Verify one signed envelope.
    ///
    /// `public_key` is the verifying key resolved by the caller from
    /// the signer's key lineage (slice B01); this module never
    /// guesses a key from the envelope and never reads a clock.
    pub fn verify(
        &self,
        envelope_bytes: &[u8],
        public_key: &[u8; ED25519_PUBLIC_KEY_LEN],
    ) -> Result<VerifiedEnvelope, SuiteError> {
        // 1. Decoding is strict: anything the canonical encoder would
        //    not produce (non-shortest forms, duplicate or
        //    out-of-order keys, trailing bytes) is already rejected.
        let record = decode_canonical(envelope_bytes).map_err(SuiteError::Cbor)?;

        // 2. Split the detached signature from the signed body. The
        //    body is the same map with the signature field removed,
        //    re-encoded canonically, so signer and verifier agree on
        //    the preimage bytes without depending on wire order.
        let (body, signature) = split_signature(record)?;

        // 3. Suites resolve through the allowlist before any
        //    cryptographic work: an unknown or retired suite fails
        //    closed instead of defaulting to Ed25519.
        let suite_id = required_uint(&body, "suite")?;
        let suite_id = u16::try_from(suite_id).map_err(|_| SuiteError::UnknownSuite(suite_id))?;
        let suite = known_suite(suite_id).ok_or(SuiteError::UnknownSuite(u64::from(suite_id)))?;
        if !self.allows(suite_id) {
            return Err(SuiteError::DisallowedSuite(suite_id));
        }
        if suite.id != SUITE_ED25519_DET_CBOR_V1 {
            // A known-but-unimplemented suite is a wiring gap, never
            // a silent success.
            return Err(SuiteError::UnimplementedSuite(suite_id));
        }

        // 4. Typed fields. Every failure is its own error; nothing
        //    is coerced (a text "1" is not suite 1).
        let key_id = required_text(&body, "key_id")?;
        let event_id = required_bytes(&body, "event_id")?;
        let object_id = required_bytes(&body, "object_id")?;
        let payload_digest = required_bytes(&body, "payload_digest")?;
        let payload_len = required_uint(&body, "payload_len")?;
        let issued_at = required_uint(&body, "issued_at")?;
        let scope = required_text(&body, "scope")?;
        let signer_id = optional_text(&body, "signer_id")?;
        let sequence = optional_uint(&body, "sequence")?;
        let parent_ids = optional_bytes_array(&body, "parent_ids")?;

        // 5. Signature shape is checked before it is handed to
        //    dalek: a malformed length is a typed error, not a panic.
        if signature.len() != ED25519_SIGNATURE_LEN {
            return Err(SuiteError::BadSignatureLength(signature.len()));
        }
        let mut sig_bytes = [0u8; ED25519_SIGNATURE_LEN];
        sig_bytes.copy_from_slice(&signature);
        let signature = Signature::from_bytes(&sig_bytes);

        let verifying_key = VerifyingKey::from_bytes(public_key).map_err(|_| SuiteError::BadKey)?;

        let body_bytes = encode_canonical(&body).map_err(SuiteError::Cbor)?;
        let preimage = signing_preimage(&body_bytes);
        verifying_key
            .verify_strict(&preimage, &signature)
            .map_err(|_| SuiteError::InvalidSignature)?;

        Ok(VerifiedEnvelope {
            suite: suite_id,
            key_id,
            signer_id,
            event_id,
            object_id,
            sequence,
            parent_ids,
            issued_at,
            payload_digest,
            payload_len,
            scope,
        })
    }
}

/// The signature preimage for a canonical body: the domain prefix,
/// a zero separator, then the canonical body bytes. Signer and
/// verifier both call this, so the construction has one home.
pub fn signing_preimage(body_bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(SIGNATURE_DOMAIN_PREFIX.len() + 1 + body_bytes.len());
    out.extend_from_slice(SIGNATURE_DOMAIN_PREFIX);
    out.push(0);
    out.extend_from_slice(body_bytes);
    out
}

/// An envelope that verified: the fields the caller may act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedEnvelope {
    pub suite: u16,
    pub key_id: String,
    pub signer_id: Option<String>,
    pub event_id: Vec<u8>,
    pub object_id: Vec<u8>,
    pub sequence: Option<u64>,
    pub parent_ids: Vec<Vec<u8>>,
    pub issued_at: u64,
    pub payload_digest: Vec<u8>,
    pub payload_len: u64,
    pub scope: String,
}

/// Typed rejections. Every failure path of [`SuiteRegistry::verify`]
/// is one of these — never a panic, never a fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuiteError {
    /// The envelope was not a canonical-profile record.
    Cbor(CborError),
    /// The suite id names no suite this program implements.
    UnknownSuite(u64),
    /// The suite is known, but this registry's policy does not
    /// allow it (a retired/deprecated suite).
    DisallowedSuite(u16),
    /// The suite is known and allowed, but no verifier is wired.
    UnimplementedSuite(u16),
    /// A required field is absent.
    MissingField(&'static str),
    /// A field is present with the wrong canonical-profile type.
    WrongType(&'static str),
    /// The `signature` field is absent or not a byte string.
    MissingSignature,
    /// The signature is not exactly 64 bytes.
    BadSignatureLength(usize),
    /// The verifying key bytes are not a valid Ed25519 point.
    BadKey,
    /// The signature does not verify over the canonical body.
    InvalidSignature,
}

impl std::fmt::Display for SuiteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SuiteError::Cbor(e) => write!(f, "noncanonical or malformed envelope: {e}"),
            SuiteError::UnknownSuite(id) => write!(f, "unknown signature suite {id}"),
            SuiteError::DisallowedSuite(id) => {
                write!(f, "signature suite {id} not allowed by policy")
            }
            SuiteError::UnimplementedSuite(id) => write!(f, "no verifier wired for suite {id}"),
            SuiteError::MissingField(name) => write!(f, "missing required field {name}"),
            SuiteError::WrongType(name) => write!(f, "field {name} has the wrong type"),
            SuiteError::MissingSignature => write!(f, "missing detached signature"),
            SuiteError::BadSignatureLength(len) => write!(f, "signature length {len} is not 64"),
            SuiteError::BadKey => write!(f, "invalid Ed25519 verifying key"),
            SuiteError::InvalidSignature => write!(f, "signature does not verify"),
        }
    }
}

impl std::error::Error for SuiteError {}

/// Split the detached signature from the signed body.
fn split_signature(record: Value) -> Result<(Value, Vec<u8>), SuiteError> {
    let Value::Map(entries) = record else {
        return Err(SuiteError::WrongType("envelope"));
    };
    let mut body = Vec::with_capacity(entries.len());
    let mut signature = None;
    for (key, value) in entries {
        if key == Value::Text(SIGNATURE_FIELD.to_string()) {
            let Value::Bytes(bytes) = value else {
                return Err(SuiteError::MissingSignature);
            };
            signature = Some(bytes);
        } else {
            body.push((key, value));
        }
    }
    let signature = signature.ok_or(SuiteError::MissingSignature)?;
    Ok((Value::Map(body), signature))
}

fn required_field<'a>(body: &'a Value, name: &'static str) -> Result<&'a Value, SuiteError> {
    let Value::Map(entries) = body else {
        return Err(SuiteError::WrongType("envelope"));
    };
    entries
        .iter()
        .find(|(key, _)| *key == Value::Text(name.to_string()))
        .map(|(_, value)| value)
        .ok_or(SuiteError::MissingField(name))
}

fn required_uint(body: &Value, name: &'static str) -> Result<u64, SuiteError> {
    match required_field(body, name)? {
        Value::UInt(value) => Ok(*value),
        _ => Err(SuiteError::WrongType(name)),
    }
}

fn required_text(body: &Value, name: &'static str) -> Result<String, SuiteError> {
    match required_field(body, name)? {
        Value::Text(value) => Ok(value.clone()),
        _ => Err(SuiteError::WrongType(name)),
    }
}

fn required_bytes(body: &Value, name: &'static str) -> Result<Vec<u8>, SuiteError> {
    match required_field(body, name)? {
        Value::Bytes(value) => Ok(value.clone()),
        _ => Err(SuiteError::WrongType(name)),
    }
}

fn optional_uint(body: &Value, name: &'static str) -> Result<Option<u64>, SuiteError> {
    let Value::Map(entries) = body else {
        return Err(SuiteError::WrongType("envelope"));
    };
    match entries
        .iter()
        .find(|(key, _)| *key == Value::Text(name.to_string()))
    {
        None => Ok(None),
        Some((_, Value::UInt(value))) => Ok(Some(*value)),
        Some(_) => Err(SuiteError::WrongType(name)),
    }
}

fn optional_text(body: &Value, name: &'static str) -> Result<Option<String>, SuiteError> {
    let Value::Map(entries) = body else {
        return Err(SuiteError::WrongType("envelope"));
    };
    match entries
        .iter()
        .find(|(key, _)| *key == Value::Text(name.to_string()))
    {
        None => Ok(None),
        Some((_, Value::Text(value))) => Ok(Some(value.clone())),
        Some(_) => Err(SuiteError::WrongType(name)),
    }
}

fn optional_bytes_array(body: &Value, name: &'static str) -> Result<Vec<Vec<u8>>, SuiteError> {
    let Value::Map(entries) = body else {
        return Err(SuiteError::WrongType("envelope"));
    };
    match entries
        .iter()
        .find(|(key, _)| *key == Value::Text(name.to_string()))
    {
        None => Ok(Vec::new()),
        Some((_, Value::Array(items))) => items
            .iter()
            .map(|item| match item {
                Value::Bytes(bytes) => Ok(bytes.clone()),
                _ => Err(SuiteError::WrongType(name)),
            })
            .collect(),
        Some(_) => Err(SuiteError::WrongType(name)),
    }
}
