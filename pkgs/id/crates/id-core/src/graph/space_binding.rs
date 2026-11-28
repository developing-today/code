//! Signer-bound space addresses (slice A-GRAPH-9131-01, #10224; design
//! part A §10, statements S5.54, S5.55, S5.62).
//!
//! A space's address is the array ID `[null, typed_public_key_bytes]`
//! under profile `space-binding/v1`. The second slot is the canonical
//! typed-byte term `[0, 237, h'<32-byte Ed25519 key>']`. The address is a
//! pure function of the key, so it can never be re-pointed: a new
//! controlling key cannot appear at the same coordinates.
//!
//! A client checks a space offline from two kinds of signed record:
//!
//! - a **binding**, signed by the address's own key over the address
//!   bytes. [`verify_binding`] decodes the address under the exact
//!   profile, checks the envelope signature and requires the signing key
//!   to be the one in the address. This proves key control over the
//!   binding record and nothing more; a server can store or serve it but
//!   cannot manufacture it.
//! - a **redirect** from an old address to a new one, signed by the
//!   *old* address's key. [`verify_redirect`] refuses one signed by any
//!   other key, so moving to a new address needs old-key proof.
//!
//! Successor chains at the original address (delegation and transfer)
//! belong to slice A-GRAPH-9132; this module never changes the binding.
//! Petnames stay local labels in [`crate::petname`] and play no part in
//! any check here.

use std::fmt;

use ed25519_dalek::{SigningKey, VerifyingKey};

use super::term::Term;
use crate::canonical_cbor::{Value, decode_canonical, encode_canonical};
use crate::hash_profile::ProfileDispatch;
use crate::id_shape::{ArrayId, Id};
use crate::signed_envelope::{Body, Envelope, EnvelopeError, PayloadId, SUITE_ED25519};

/// The address profile, carried as the records' signing context.
pub const SPACE_BINDING_PROFILE: &str = "space-binding/v1";
/// Signing domain of a binding record.
pub const SPACE_BINDING_DOMAIN: &str = "id/space-binding";
/// Signing domain of a redirect record.
pub const SPACE_REDIRECT_DOMAIN: &str = "id/space-redirect";
/// Type tag of an Ed25519 public key term (provisional until a term-tag
/// registry exists).
pub const ED25519_PUBLIC_KEY_TAG: u64 = 237;
/// Version of the redirect payload.
pub const REDIRECT_VERSION: u64 = 1;

const PAYLOAD_PROFILE: crate::id_shape::HashProfile = crate::id_shape::HashProfile::RawBlake3_256;

/// Why a space address or record was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpaceError {
    /// The bytes are not a canonical array ID.
    Malformed(&'static str),
    /// The slots are not exactly `[null, key]`.
    WrongPosition,
    /// The key slot holds a key type this profile does not know.
    UnsupportedKeyType(u64),
    /// The key bytes are not a valid Ed25519 public key.
    InvalidKey,
    /// The record was signed for another purpose.
    WrongDomain(String),
    /// The record names another address profile or payload version.
    UnsupportedProfile(String),
    /// The envelope does not verify.
    Envelope(EnvelopeError),
    /// The record is not signed by the key it must be signed by.
    WrongSigner,
    /// A redirect points an address at itself.
    SelfRedirect,
}

impl SpaceError {
    /// A stable failure class name.
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::Malformed(_) | Self::WrongPosition | Self::InvalidKey => "malformed",
            Self::UnsupportedKeyType(_) | Self::UnsupportedProfile(_) => "unsupported",
            Self::WrongDomain(_) => "wrong_domain",
            Self::Envelope(_) => "bad_signature",
            Self::WrongSigner => "wrong_signer",
            Self::SelfRedirect => "invalid_redirect",
        }
    }
}

impl fmt::Display for SpaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(why) => write!(f, "space address malformed: {why}"),
            Self::WrongPosition => write!(f, "space address must be [null, key]"),
            Self::UnsupportedKeyType(tag) => write!(f, "unsupported key type {tag}"),
            Self::InvalidKey => write!(f, "not a valid Ed25519 public key"),
            Self::WrongDomain(domain) => {
                write!(f, "record domain {domain:?} is not a space record")
            }
            Self::UnsupportedProfile(what) => write!(f, "unsupported space profile {what:?}"),
            Self::Envelope(error) => write!(f, "space record: {error}"),
            Self::WrongSigner => write!(f, "record is not signed by the space's own key"),
            Self::SelfRedirect => write!(f, "a space cannot redirect to itself"),
        }
    }
}

impl std::error::Error for SpaceError {}

impl From<EnvelopeError> for SpaceError {
    fn from(error: EnvelopeError) -> Self {
        Self::Envelope(error)
    }
}

/// A signer-bound space address: the array ID `[null, key]`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SpaceAddress {
    key: [u8; 32],
    id: ArrayId,
}

impl SpaceAddress {
    /// The address bound to `key`.
    ///
    /// # Errors
    ///
    /// [`SpaceError::InvalidKey`] when the bytes are not an Ed25519 key.
    pub fn for_key(key: [u8; 32]) -> Result<Self, SpaceError> {
        VerifyingKey::from_bytes(&key).map_err(|_error| SpaceError::InvalidKey)?;
        let typed = Term::typed(ED25519_PUBLIC_KEY_TAG, key.to_vec())
            .to_bytes()
            .map_err(|_error| SpaceError::Malformed("key term"))?;
        let id = ArrayId::new(vec![None, Some(typed)])
            .map_err(|_error| SpaceError::Malformed("array limits"))?;
        Ok(Self { key, id })
    }

    /// The designated signer's public key.
    #[must_use]
    pub const fn key(&self) -> &[u8; 32] {
        &self.key
    }

    /// The address as an array ID.
    #[must_use]
    pub const fn array_id(&self) -> &ArrayId {
        &self.id
    }

    /// The canonical binary ID bytes.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        Id::Array(self.id.clone()).to_bytes()
    }

    /// Reads an address from an array ID.
    ///
    /// # Errors
    ///
    /// [`SpaceError`] unless the slots are exactly `[null, typed key]`
    /// with an Ed25519 key.
    pub fn from_array_id(id: &ArrayId) -> Result<Self, SpaceError> {
        let [None, Some(typed)] = id.slots() else {
            return Err(SpaceError::WrongPosition);
        };
        let term = Term::from_bytes(typed)
            .map_err(|_error| SpaceError::Malformed("key slot is not a term"))?;
        let Term::Typed { type_tag, bytes } = term else {
            return Err(SpaceError::Malformed("key slot is not typed bytes"));
        };
        if type_tag != ED25519_PUBLIC_KEY_TAG {
            return Err(SpaceError::UnsupportedKeyType(type_tag));
        }
        let key =
            <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_error| SpaceError::InvalidKey)?;
        Self::for_key(key)
    }

    /// Reads an address from canonical binary ID bytes.
    ///
    /// # Errors
    ///
    /// As [`SpaceAddress::from_array_id`], or when the bytes are not a
    /// canonical array ID.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SpaceError> {
        match Id::parse_bytes(bytes) {
            Ok(Id::Array(array)) => Self::from_array_id(&array),
            Ok(_) => Err(SpaceError::Malformed("not an array ID")),
            Err(_) => Err(SpaceError::Malformed("not a canonical ID")),
        }
    }
}

fn sign_record(
    domain: &str,
    payload: Vec<u8>,
    signer_id: &str,
    key: &SigningKey,
) -> Result<Envelope, SpaceError> {
    let body = Body {
        domain: domain.to_owned(),
        context: SPACE_BINDING_PROFILE.to_owned(),
        subject: None,
        graph: None,
        predecessor: None,
        signer_id: signer_id.to_owned(),
        key: key.verifying_key().to_bytes(),
        suite: SUITE_ED25519.to_owned(),
        payload_id: PayloadId::of(PAYLOAD_PROFILE, &payload).map_err(EnvelopeError::from)?,
        payload_length: payload.len() as u64,
    };
    Ok(Envelope::sign(body, payload, key)?)
}

fn check_record(
    envelope: &Envelope,
    domain: &str,
    dispatch: &ProfileDispatch,
) -> Result<(), SpaceError> {
    if envelope.body.domain != domain {
        return Err(SpaceError::WrongDomain(envelope.body.domain.clone()));
    }
    if envelope.body.context != SPACE_BINDING_PROFILE {
        return Err(SpaceError::UnsupportedProfile(
            envelope.body.context.clone(),
        ));
    }
    envelope.verify(dispatch)?;
    Ok(())
}

/// Signs the binding record for `key`'s own space.
///
/// # Errors
///
/// An envelope error from signing.
pub fn sign_binding(key: &SigningKey, signer_id: &str) -> Result<Envelope, SpaceError> {
    let address = SpaceAddress::for_key(key.verifying_key().to_bytes())?;
    sign_record(SPACE_BINDING_DOMAIN, address.to_bytes(), signer_id, key)
}

/// Verifies a binding record offline and returns its address.
///
/// # Errors
///
/// [`SpaceError`] for another domain or profile, a bad signature, a
/// payload that is not a `[null, key]` address, or a signer other than
/// the address's key.
pub fn verify_binding(
    envelope: &Envelope,
    dispatch: &ProfileDispatch,
) -> Result<SpaceAddress, SpaceError> {
    check_record(envelope, SPACE_BINDING_DOMAIN, dispatch)?;
    let address = SpaceAddress::from_bytes(&envelope.payload)?;
    if envelope.body.key != address.key {
        return Err(SpaceError::WrongSigner);
    }
    Ok(address)
}

/// A verified move from one space address to another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceRedirect {
    /// The old address, whose key signed the redirect.
    pub from: SpaceAddress,
    /// The new address.
    pub to: SpaceAddress,
}

fn redirect_payload(from: &SpaceAddress, to: &SpaceAddress) -> Result<Vec<u8>, SpaceError> {
    encode_canonical(&Value::Array(vec![
        Value::UInt(REDIRECT_VERSION),
        Value::Bytes(from.to_bytes()),
        Value::Bytes(to.to_bytes()),
    ]))
    .map_err(|_error| SpaceError::Malformed("redirect encoding"))
}

/// Signs a redirect from `old_key`'s space to `to`.
///
/// # Errors
///
/// [`SpaceError::SelfRedirect`] when `to` is the old address, or an
/// envelope error from signing.
pub fn sign_redirect(
    old_key: &SigningKey,
    to: &SpaceAddress,
    signer_id: &str,
) -> Result<Envelope, SpaceError> {
    let from = SpaceAddress::for_key(old_key.verifying_key().to_bytes())?;
    if from == *to {
        return Err(SpaceError::SelfRedirect);
    }
    sign_record(
        SPACE_REDIRECT_DOMAIN,
        redirect_payload(&from, to)?,
        signer_id,
        old_key,
    )
}

/// Verifies a redirect record offline.
///
/// # Errors
///
/// [`SpaceError`] for another domain or profile, a bad signature, a
/// malformed payload or address, a redirect to itself, or a signer
/// other than the old address's key.
pub fn verify_redirect(
    envelope: &Envelope,
    dispatch: &ProfileDispatch,
) -> Result<SpaceRedirect, SpaceError> {
    check_record(envelope, SPACE_REDIRECT_DOMAIN, dispatch)?;
    let Ok(Value::Array(items)) = decode_canonical(&envelope.payload) else {
        return Err(SpaceError::Malformed("redirect payload is not an array"));
    };
    let [Value::UInt(version), Value::Bytes(from), Value::Bytes(to)] = items.as_slice() else {
        return Err(SpaceError::Malformed(
            "redirect payload must be [v, from, to]",
        ));
    };
    if *version != REDIRECT_VERSION {
        return Err(SpaceError::UnsupportedProfile(format!(
            "redirect version {version}"
        )));
    }
    let from = SpaceAddress::from_bytes(from)?;
    let to = SpaceAddress::from_bytes(to)?;
    if from == to {
        return Err(SpaceError::SelfRedirect);
    }
    if envelope.body.key != from.key {
        return Err(SpaceError::WrongSigner);
    }
    Ok(SpaceRedirect { from, to })
}
