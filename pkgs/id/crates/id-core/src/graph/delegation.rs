//! Delegation chains and two-key control transfer (slice
//! A-GRAPH-9132-01, #10239; design part A §11, statements S5.56, S5.57,
//! S5.61, S5.64).
//!
//! # Grants
//!
//! A [`Grant`] lets `grantee_key` act in a space within a [`Scope`] (exact
//! typed array-address prefixes) and a [`Permissions`] set, between
//! `not_before` and `expires` (UTC Unix seconds). It is a signed envelope
//! whose signer must be `grantor_key`. A root grant (no predecessor) must
//! come from the space's own key; any other grant names the grant that
//! gave its grantor authority. [`DelegationSet::authorize`] walks from a
//! leaf grant to the root and fails closed on the first problem, in this
//! order for each grant, leaf first:
//!
//! 1. chain longer than the profile's depth budget: `DelegationChainTooDeep`;
//! 2. a grantor key seen earlier in the chain (or the requester): `DelegationCycle`;
//! 3. a grant for another space: `WrongSpace`;
//! 4. a revoked grant: `Revoked`;
//! 5. outside its time window, widened by the clock skew allowance:
//!    `NotYetValid` or `Expired`;
//! 6. at the leaf, a grantee, permission or path that does not cover the
//!    request: `NotAuthorized`; above the leaf, a parent that did not
//!    grant to the child's grantor (`BrokenChain`), lacks `delegate`
//!    (`NotDelegable`), or grants less than the child claims
//!    (`PermissionExceeded`, `ScopeExceeded`);
//! 7. a root not granted by the space's key (`NotAuthorized`), or a
//!    predecessor that is not known (`MissingLink`).
//!
//! There is no implicit transitive delegation: a grantee can pass on
//! authority only with `delegate`. Before any of that, a delegated write
//! (anything but read) fails with `StaleFrontier` when the latest
//! verified authority frontier is older than the profile's revocation
//! staleness bound; a read is allowed and reported as stale.
//!
//! # Replay and revocation
//!
//! [`DelegationSet::admit_grant`] keeps `(grantor, domain, nonce)` unique:
//! the identical signed envelope again is idempotent, the same nonce with
//! a different body is `ReplayConflict`. A revocation is a signed record
//! naming a grant ID, signed by that grant's grantor or the space's key.
//!
//! # Transfer
//!
//! A transfer replaces the controlling key only when the old controller
//! signs an offer and the new controller signs an acceptance over the
//! *identical* [`TransferStatement`] bytes (space, both keys, scope,
//! nonce, challenge, purpose and expiry). It is a separate record type
//! from a grant and never changes the immutable space address.
//!
//! Cross-signing statements and recovery policy are not part of this
//! slice. Module wiring into `graph/mod.rs` belongs to slice 9110-09;
//! until then the integration test includes this file by path.

use std::collections::{HashMap, HashSet};
use std::fmt;

use ed25519_dalek::SigningKey;

use super::space_binding::SpaceAddress;
use super::term::{ObjectRef, Term};
use crate::canonical_cbor::{Value, decode_canonical, encode_canonical};
use crate::hash_profile::ProfileDispatch;
use crate::id_shape::HashProfile;
use crate::signed_envelope::{Body, Envelope, EnvelopeError, EnvelopeId, PayloadId, SUITE_ED25519};

/// Signing domain of a grant.
pub const GRANT_DOMAIN: &str = "id/delegation-grant";
/// Signing domain of a revocation.
pub const REVOCATION_DOMAIN: &str = "id/delegation-revocation";
/// Signing domain of a transfer offer.
pub const TRANSFER_OFFER_DOMAIN: &str = "id/space-transfer-offer";
/// Signing domain of a transfer acceptance.
pub const TRANSFER_ACCEPT_DOMAIN: &str = "id/space-transfer-accept";
/// Purpose bound inside every transfer statement.
pub const TRANSFER_PURPOSE: &str = "id/space-transfer";
/// Signing context (record profile) of every record here.
pub const DELEGATION_PROFILE: &str = "delegation/v1";
/// Payload version of every record here.
pub const PAYLOAD_VERSION: u64 = 1;
/// Longest nonce, in bytes.
pub const MAX_NONCE_BYTES: usize = 64;
/// Most prefixes in one scope.
pub const MAX_SCOPE_PREFIXES: usize = 64;

/// Profile that names record and payload IDs.
pub const ID_PROFILE: HashProfile = HashProfile::RawBlake3_256;

/// Operator-validated limits. Requests cannot change them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile {
    /// Most grants in one chain.
    pub max_depth: usize,
    /// Allowed clock difference, in seconds, on both ends of a window.
    pub clock_skew_secs: u64,
    /// Oldest authority frontier, in seconds, a delegated write accepts.
    pub max_revocation_staleness_secs: u64,
}

impl Profile {
    /// The design's initial deployment profile.
    pub const INITIAL: Self = Self {
        max_depth: 16,
        clock_skew_secs: 300,
        max_revocation_staleness_secs: 24 * 60 * 60,
    };
}

/// One permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Permission {
    /// Read.
    Read,
    /// Append new records.
    Append,
    /// Edit existing records.
    Edit,
    /// Moderate.
    Moderate,
    /// Pass authority on to others.
    Delegate,
}

impl Permission {
    const fn bit(self) -> u8 {
        match self {
            Self::Read => 1,
            Self::Append => 2,
            Self::Edit => 4,
            Self::Moderate => 8,
            Self::Delegate => 16,
        }
    }
}

/// A set of permissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Permissions(u8);

impl Permissions {
    /// Every defined permission bit.
    pub const ALL_BITS: u8 = 31;

    /// The set from its bits.
    ///
    /// # Errors
    ///
    /// [`DelegationError::Malformed`] for an undefined bit.
    pub const fn from_bits(bits: u8) -> Result<Self, DelegationError> {
        if bits & !Self::ALL_BITS != 0 {
            return Err(DelegationError::Malformed("unknown permission bit"));
        }
        Ok(Self(bits))
    }

    /// The set of these permissions.
    #[must_use]
    pub fn of(permissions: &[Permission]) -> Self {
        Self(permissions.iter().fold(0, |bits, p| bits | p.bit()))
    }

    /// The bits.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether `permission` is in the set.
    #[must_use]
    pub const fn has(self, permission: Permission) -> bool {
        self.0 & permission.bit() != 0
    }

    /// Whether every permission in `other` is in this set.
    #[must_use]
    pub const fn covers(self, other: Self) -> bool {
        other.0 & !self.0 == 0
    }
}

/// Exact typed array-address prefixes. An empty prefix covers every
/// path; a path is covered when it starts with one of the prefixes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    prefixes: Vec<Vec<Term>>,
}

fn prefix_value(prefix: &[Term]) -> Result<Value, DelegationError> {
    prefix
        .iter()
        .map(|t| {
            t.to_value()
                .map_err(|_error| DelegationError::Malformed("scope term"))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

fn sort_key(prefix: &[Term]) -> Vec<u8> {
    prefix_value(prefix)
        .ok()
        .and_then(|value| encode_canonical(&value).ok())
        .unwrap_or_default()
}

impl Scope {
    /// A scope from prefixes, stored in canonical order without repeats.
    ///
    /// # Errors
    ///
    /// [`DelegationError::Malformed`] for too many prefixes or a term
    /// that cannot be encoded.
    pub fn new(mut prefixes: Vec<Vec<Term>>) -> Result<Self, DelegationError> {
        if prefixes.len() > MAX_SCOPE_PREFIXES {
            return Err(DelegationError::Malformed("too many scope prefixes"));
        }
        for prefix in &prefixes {
            prefix_value(prefix)?;
        }
        prefixes.sort_by_cached_key(|p| sort_key(p));
        prefixes.dedup();
        Ok(Self { prefixes })
    }

    /// The prefixes, in canonical order.
    #[must_use]
    pub fn prefixes(&self) -> &[Vec<Term>] {
        &self.prefixes
    }

    /// Whether `path` starts with one of the prefixes.
    #[must_use]
    pub fn covers_path(&self, path: &[Term]) -> bool {
        self.prefixes.iter().any(|p| path.starts_with(p))
    }

    /// Whether every path `other` covers is covered here.
    #[must_use]
    pub fn contains(&self, other: &Self) -> bool {
        other.prefixes.iter().all(|p| self.covers_path(p))
    }

    fn to_value(&self) -> Result<Value, DelegationError> {
        self.prefixes
            .iter()
            .map(|p| prefix_value(p))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array)
    }

    fn from_value(value: &Value) -> Result<Self, DelegationError> {
        let Value::Array(prefixes) = value else {
            return Err(DelegationError::Malformed("scope must be an array"));
        };
        let prefixes = prefixes
            .iter()
            .map(|p| {
                let Value::Array(terms) = p else {
                    return Err(DelegationError::Malformed("scope prefix must be an array"));
                };
                terms
                    .iter()
                    .map(|t| {
                        Term::from_value(t)
                            .map_err(|_error| DelegationError::Malformed("scope term"))
                    })
                    .collect()
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(prefixes)
    }
}

/// A delegation grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    /// The space the grant is for.
    pub space: SpaceAddress,
    /// The key that grants, and signs.
    pub grantor_key: [u8; 32],
    /// The key that receives authority.
    pub grantee_key: [u8; 32],
    /// Where the authority applies.
    pub scope: Scope,
    /// What the authority allows.
    pub permissions: Permissions,
    /// Earliest valid time, UTC Unix seconds.
    pub not_before: u64,
    /// Latest valid time, UTC Unix seconds.
    pub expires: u64,
    /// Replay-protection nonce, unique per grantor.
    pub nonce: Vec<u8>,
    /// The grant that gave the grantor authority; `None` for a root.
    pub predecessor: Option<EnvelopeId>,
}

/// Why a delegation record or request was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DelegationError {
    /// A record is not well formed.
    Malformed(&'static str),
    /// A record's envelope does not verify.
    Envelope(EnvelopeError),
    /// A record was signed for another purpose or profile.
    WrongDomain(String),
    /// A record is not signed by the key it must be signed by.
    WrongSigner,
    /// The same grantor, domain and nonce name a different record.
    ReplayConflict,
    /// A revocation names a grant this set does not hold.
    UnknownGrant,
    /// The request is not covered.
    NotAuthorized(&'static str),
    /// A grant is for another space.
    WrongSpace,
    /// A grant was revoked.
    Revoked,
    /// A grant's window has not started.
    NotYetValid,
    /// A grant or transfer has expired.
    Expired,
    /// A parent did not grant to the child's grantor.
    BrokenChain,
    /// A parent lacks `delegate`.
    NotDelegable,
    /// A child claims a permission its parent lacks.
    PermissionExceeded,
    /// A child claims scope its parent lacks.
    ScopeExceeded,
    /// A predecessor grant is not known.
    MissingLink,
    /// A key grants twice in one chain.
    DelegationCycle,
    /// The chain is longer than the depth budget.
    DelegationChainTooDeep,
    /// A delegated write against a too-old authority frontier.
    StaleFrontier,
    /// A transfer's offer and acceptance sign different statements.
    TransferMismatch,
    /// A transfer hands control to the key that already holds it.
    SameKey,
}

impl DelegationError {
    /// A stable failure class name.
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::Malformed(_) => "malformed",
            Self::Envelope(_) => "bad_signature",
            Self::WrongDomain(_) => "wrong_domain",
            Self::WrongSigner => "wrong_signer",
            Self::ReplayConflict => "replay_conflict",
            Self::UnknownGrant => "unknown_grant",
            Self::NotAuthorized(_) => "not_authorized",
            Self::WrongSpace => "wrong_space",
            Self::Revoked => "revoked",
            Self::NotYetValid => "not_yet_valid",
            Self::Expired => "expired",
            Self::BrokenChain => "broken_chain",
            Self::NotDelegable => "not_delegable",
            Self::PermissionExceeded => "permission_exceeded",
            Self::ScopeExceeded => "scope_exceeded",
            Self::MissingLink => "missing_link",
            Self::DelegationCycle => "delegation_cycle",
            Self::DelegationChainTooDeep => "delegation_chain_too_deep",
            Self::StaleFrontier => "stale_frontier",
            Self::TransferMismatch => "transfer_mismatch",
            Self::SameKey => "same_key",
        }
    }
}

impl fmt::Display for DelegationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(why) => write!(f, "malformed delegation record: {why}"),
            Self::Envelope(error) => write!(f, "delegation record: {error}"),
            Self::WrongDomain(domain) => write!(f, "record domain {domain:?} is not expected here"),
            Self::NotAuthorized(why) => write!(f, "not authorized: {why}"),
            other => f.write_str(other.class()),
        }
    }
}

impl std::error::Error for DelegationError {}

impl From<EnvelopeError> for DelegationError {
    fn from(error: EnvelopeError) -> Self {
        Self::Envelope(error)
    }
}

fn key_from(value: &Value, what: &'static str) -> Result<[u8; 32], DelegationError> {
    let Value::Bytes(bytes) = value else {
        return Err(DelegationError::Malformed(what));
    };
    <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_error| DelegationError::Malformed(what))
}

fn space_from(value: &Value) -> Result<SpaceAddress, DelegationError> {
    let Value::Bytes(bytes) = value else {
        return Err(DelegationError::Malformed("space must be bytes"));
    };
    SpaceAddress::from_bytes(bytes).map_err(|_error| DelegationError::Malformed("space address"))
}

const fn uint(value: &Value, what: &'static str) -> Result<u64, DelegationError> {
    match value {
        Value::UInt(n) => Ok(*n),
        _ => Err(DelegationError::Malformed(what)),
    }
}

fn nonce_from(value: &Value) -> Result<Vec<u8>, DelegationError> {
    match value {
        Value::Bytes(bytes) if !bytes.is_empty() && bytes.len() <= MAX_NONCE_BYTES => {
            Ok(bytes.clone())
        }
        _ => Err(DelegationError::Malformed("nonce must be 1 to 64 bytes")),
    }
}

fn envelope_ref(id: EnvelopeId) -> Result<Value, DelegationError> {
    Term::Object(ObjectRef::Envelope(id))
        .to_value()
        .map_err(|_error| DelegationError::Malformed("grant reference"))
}

fn envelope_ref_from(value: &Value) -> Result<EnvelopeId, DelegationError> {
    match Term::from_value(value) {
        Ok(Term::Object(ObjectRef::Envelope(id))) => Ok(id),
        _ => Err(DelegationError::Malformed("grant reference")),
    }
}

/// Decodes a payload array of the expected length. Each record decoder
/// then calls [`require_canonical`], which rebuilds the payload from the
/// decoded fields (with this module's version and, for transfers, its
/// purpose) and needs identical bytes: each record has one encoding, and
/// an unknown version or purpose is refused there.
fn payload_items(payload: &[u8], len: usize) -> Result<Vec<Value>, DelegationError> {
    let Ok(Value::Array(items)) = decode_canonical(payload) else {
        return Err(DelegationError::Malformed("payload must be an array"));
    };
    if items.len() != len {
        return Err(DelegationError::Malformed("payload has the wrong length"));
    }
    Ok(items)
}

fn require_canonical(payload: &[u8], value: &Value) -> Result<(), DelegationError> {
    match encode_canonical(value) {
        Ok(bytes) if bytes == payload => Ok(()),
        _ => Err(DelegationError::Malformed("payload is not canonical")),
    }
}

impl Grant {
    fn to_value(&self) -> Result<Value, DelegationError> {
        Ok(Value::Array(vec![
            Value::UInt(PAYLOAD_VERSION),
            Value::Bytes(self.space.to_bytes()),
            Value::Bytes(self.grantor_key.to_vec()),
            Value::Bytes(self.grantee_key.to_vec()),
            self.scope.to_value()?,
            Value::UInt(u64::from(self.permissions.bits())),
            Value::UInt(self.not_before),
            Value::UInt(self.expires),
            Value::Bytes(self.nonce.clone()),
            match self.predecessor {
                Some(id) => envelope_ref(id)?,
                None => Value::Null,
            },
        ]))
    }

    /// The canonical payload bytes.
    ///
    /// # Errors
    ///
    /// [`DelegationError::Malformed`] when a field cannot be encoded.
    pub fn payload_bytes(&self) -> Result<Vec<u8>, DelegationError> {
        encode_canonical(&self.to_value()?)
            .map_err(|_error| DelegationError::Malformed("grant encoding"))
    }

    /// Decodes a grant from its canonical payload.
    ///
    /// # Errors
    ///
    /// [`DelegationError::Malformed`] for any other shape, a window that
    /// ends before it starts, or a non-canonical encoding.
    pub fn from_payload(payload: &[u8]) -> Result<Self, DelegationError> {
        let items = payload_items(payload, 10)?;
        let bits = u8::try_from(uint(&items[5], "permissions")?)
            .map_err(|_error| DelegationError::Malformed("unknown permission bit"))?;
        let grant = Self {
            space: space_from(&items[1])?,
            grantor_key: key_from(&items[2], "grantor key")?,
            grantee_key: key_from(&items[3], "grantee key")?,
            scope: Scope::from_value(&items[4])?,
            permissions: Permissions::from_bits(bits)?,
            not_before: uint(&items[6], "not_before")?,
            expires: uint(&items[7], "expires")?,
            nonce: nonce_from(&items[8])?,
            predecessor: match &items[9] {
                Value::Null => None,
                other => Some(envelope_ref_from(other)?),
            },
        };
        if grant.expires < grant.not_before {
            return Err(DelegationError::Malformed("grant expires before it starts"));
        }
        require_canonical(payload, &grant.to_value()?)?;
        Ok(grant)
    }

    /// Signs the grant with the grantor's key.
    ///
    /// # Errors
    ///
    /// [`DelegationError::WrongSigner`] when `key` is not the grantor's,
    /// or an encoding or signing error.
    pub fn sign(&self, signer_id: &str, key: &SigningKey) -> Result<Envelope, DelegationError> {
        if key.verifying_key().to_bytes() != self.grantor_key {
            return Err(DelegationError::WrongSigner);
        }
        sign_record(GRANT_DOMAIN, self.payload_bytes()?, signer_id, key)
    }

    const fn window(&self, utc: u64, skew: u64) -> Result<(), DelegationError> {
        if self.not_before > utc.saturating_add(skew) {
            return Err(DelegationError::NotYetValid);
        }
        if self.expires.saturating_add(skew) < utc {
            return Err(DelegationError::Expired);
        }
        Ok(())
    }
}

fn sign_record(
    domain: &str,
    payload: Vec<u8>,
    signer_id: &str,
    key: &SigningKey,
) -> Result<Envelope, DelegationError> {
    let body = Body {
        domain: domain.to_owned(),
        context: DELEGATION_PROFILE.to_owned(),
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
) -> Result<(), DelegationError> {
    if envelope.body.domain != domain {
        return Err(DelegationError::WrongDomain(envelope.body.domain.clone()));
    }
    if envelope.body.context != DELEGATION_PROFILE {
        return Err(DelegationError::WrongDomain(envelope.body.context.clone()));
    }
    envelope.verify(dispatch)?;
    Ok(())
}

/// Signs a revocation of `grant` with `key`.
///
/// # Errors
///
/// An encoding or signing error.
pub fn sign_revocation(
    grant: EnvelopeId,
    signer_id: &str,
    key: &SigningKey,
) -> Result<Envelope, DelegationError> {
    let payload = encode_canonical(&Value::Array(vec![
        Value::UInt(PAYLOAD_VERSION),
        envelope_ref(grant)?,
    ]))
    .map_err(|_error| DelegationError::Malformed("revocation encoding"))?;
    sign_record(REVOCATION_DOMAIN, payload, signer_id, key)
}

/// What [`DelegationSet::authorize`] established.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Authority {
    /// The requester holds the space's own key.
    Owner,
    /// A valid chain of this many grants covers the request.
    Delegated {
        /// Grants from leaf to root.
        depth: usize,
        /// The authority frontier is past the staleness bound; only a
        /// read gets this far, and must be shown as stale.
        stale: bool,
    },
}

/// One action asked of a space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The space acted in.
    pub space: SpaceAddress,
    /// The key acting.
    pub key: [u8; 32],
    /// The typed array-address path acted on.
    pub path: Vec<Term>,
    /// The permission needed.
    pub permission: Permission,
}

/// The verifier's trusted view of time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    /// Trusted verifier time, UTC Unix seconds.
    pub verifier_utc: u64,
    /// Age, in seconds, of the latest verified authority frontier.
    pub frontier_age_secs: u64,
}

/// Grants and revocations verified at one authority frontier.
#[derive(Debug, Default, Clone)]
pub struct DelegationSet {
    grants: HashMap<EnvelopeId, Grant>,
    nonces: HashMap<([u8; 32], Vec<u8>), EnvelopeId>,
    revoked: HashSet<EnvelopeId>,
}

impl DelegationSet {
    /// An empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Verifies and admits a grant, returning its ID.
    ///
    /// # Errors
    ///
    /// [`DelegationError`] for a bad record, a signer other than the
    /// grantor, or a reused nonce with a different record.
    pub fn admit_grant(
        &mut self,
        envelope: &Envelope,
        dispatch: &ProfileDispatch,
    ) -> Result<EnvelopeId, DelegationError> {
        check_record(envelope, GRANT_DOMAIN, dispatch)?;
        let grant = Grant::from_payload(&envelope.payload)?;
        if envelope.body.key != grant.grantor_key {
            return Err(DelegationError::WrongSigner);
        }
        let id = envelope.id(ID_PROFILE)?;
        let slot = (grant.grantor_key, grant.nonce.clone());
        match self.nonces.get(&slot) {
            Some(existing) if *existing == id => return Ok(id),
            Some(_) => return Err(DelegationError::ReplayConflict),
            None => {}
        }
        self.nonces.insert(slot, id);
        self.grants.insert(id, grant);
        Ok(id)
    }

    /// Verifies and admits a revocation.
    ///
    /// # Errors
    ///
    /// [`DelegationError`] for a bad record, an unknown grant, or a
    /// signer that is neither the grant's grantor nor the space's key.
    pub fn admit_revocation(
        &mut self,
        envelope: &Envelope,
        dispatch: &ProfileDispatch,
    ) -> Result<EnvelopeId, DelegationError> {
        check_record(envelope, REVOCATION_DOMAIN, dispatch)?;
        let items = payload_items(&envelope.payload, 2)?;
        let target = envelope_ref_from(&items[1])?;
        require_canonical(
            &envelope.payload,
            &Value::Array(vec![Value::UInt(PAYLOAD_VERSION), envelope_ref(target)?]),
        )?;
        let grant = self
            .grants
            .get(&target)
            .ok_or(DelegationError::UnknownGrant)?;
        if envelope.body.key != grant.grantor_key && envelope.body.key != *grant.space.key() {
            return Err(DelegationError::WrongSigner);
        }
        self.revoked.insert(target);
        Ok(target)
    }

    /// The admitted grant with this ID.
    #[must_use]
    pub fn grant(&self, id: &EnvelopeId) -> Option<&Grant> {
        self.grants.get(id)
    }

    /// Decides `request`, following the chain that ends at `leaf`.
    ///
    /// # Errors
    ///
    /// The first [`DelegationError`] in the order the module docs give.
    pub fn authorize(
        &self,
        request: &Request,
        leaf: Option<&EnvelopeId>,
        clock: Clock,
        profile: &Profile,
    ) -> Result<Authority, DelegationError> {
        if request.key == *request.space.key() {
            return Ok(Authority::Owner);
        }
        let stale = clock.frontier_age_secs > profile.max_revocation_staleness_secs;
        if stale && request.permission != Permission::Read {
            return Err(DelegationError::StaleFrontier);
        }
        let Some(leaf) = leaf else {
            return Err(DelegationError::NotAuthorized("no grant presented"));
        };
        let mut seen: HashSet<[u8; 32]> = HashSet::from([request.key]);
        let mut child: Option<&Grant> = None;
        let mut at = *leaf;
        let mut depth = 0;
        loop {
            let grant = self.grants.get(&at).ok_or(DelegationError::MissingLink)?;
            depth += 1;
            if depth > profile.max_depth {
                return Err(DelegationError::DelegationChainTooDeep);
            }
            if !seen.insert(grant.grantor_key) {
                return Err(DelegationError::DelegationCycle);
            }
            if grant.space != request.space {
                return Err(DelegationError::WrongSpace);
            }
            if self.revoked.contains(&at) {
                return Err(DelegationError::Revoked);
            }
            grant.window(clock.verifier_utc, profile.clock_skew_secs)?;
            match child {
                None => {
                    if grant.grantee_key != request.key {
                        return Err(DelegationError::NotAuthorized("grant is for another key"));
                    }
                    if !grant.permissions.has(request.permission) {
                        return Err(DelegationError::NotAuthorized("permission not granted"));
                    }
                    if !grant.scope.covers_path(&request.path) {
                        return Err(DelegationError::NotAuthorized("path outside the grant"));
                    }
                }
                Some(child) => {
                    if grant.grantee_key != child.grantor_key {
                        return Err(DelegationError::BrokenChain);
                    }
                    if !grant.permissions.has(Permission::Delegate) {
                        return Err(DelegationError::NotDelegable);
                    }
                    if !grant.permissions.covers(child.permissions) {
                        return Err(DelegationError::PermissionExceeded);
                    }
                    if !grant.scope.contains(&child.scope) {
                        return Err(DelegationError::ScopeExceeded);
                    }
                }
            }
            match grant.predecessor {
                None if grant.grantor_key == *request.space.key() => {
                    return Ok(Authority::Delegated { depth, stale });
                }
                None => {
                    return Err(DelegationError::NotAuthorized(
                        "root not granted by the space",
                    ));
                }
                Some(parent) => {
                    child = Some(grant);
                    at = parent;
                }
            }
        }
    }
}

/// The statement both keys sign to transfer control of a space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferStatement {
    /// The space whose control moves.
    pub space: SpaceAddress,
    /// The current controller.
    pub old_key: [u8; 32],
    /// The new controller.
    pub new_key: [u8; 32],
    /// What control moves.
    pub scope: Scope,
    /// Replay-protection nonce.
    pub nonce: Vec<u8>,
    /// A challenge both parties saw.
    pub challenge: Vec<u8>,
    /// Latest valid time, UTC Unix seconds.
    pub expires: u64,
}

impl TransferStatement {
    fn to_value(&self) -> Result<Value, DelegationError> {
        Ok(Value::Array(vec![
            Value::UInt(PAYLOAD_VERSION),
            Value::Text(TRANSFER_PURPOSE.to_owned()),
            Value::Bytes(self.space.to_bytes()),
            Value::Bytes(self.old_key.to_vec()),
            Value::Bytes(self.new_key.to_vec()),
            self.scope.to_value()?,
            Value::Bytes(self.nonce.clone()),
            Value::Bytes(self.challenge.clone()),
            Value::UInt(self.expires),
        ]))
    }

    /// The canonical bytes both parties sign.
    ///
    /// # Errors
    ///
    /// [`DelegationError::Malformed`] when a field cannot be encoded.
    pub fn payload_bytes(&self) -> Result<Vec<u8>, DelegationError> {
        encode_canonical(&self.to_value()?)
            .map_err(|_error| DelegationError::Malformed("transfer encoding"))
    }

    fn from_payload(payload: &[u8]) -> Result<Self, DelegationError> {
        let items = payload_items(payload, 9)?;
        let Value::Bytes(challenge) = &items[7] else {
            return Err(DelegationError::Malformed("challenge must be bytes"));
        };
        let statement = Self {
            space: space_from(&items[2])?,
            old_key: key_from(&items[3], "old key")?,
            new_key: key_from(&items[4], "new key")?,
            scope: Scope::from_value(&items[5])?,
            nonce: nonce_from(&items[6])?,
            challenge: challenge.clone(),
            expires: uint(&items[8], "expires")?,
        };
        require_canonical(payload, &statement.to_value()?)?;
        Ok(statement)
    }

    /// The old controller's signed offer.
    ///
    /// # Errors
    ///
    /// [`DelegationError::WrongSigner`] when `key` is not the old key.
    pub fn sign_offer(
        &self,
        signer_id: &str,
        key: &SigningKey,
    ) -> Result<Envelope, DelegationError> {
        if key.verifying_key().to_bytes() != self.old_key {
            return Err(DelegationError::WrongSigner);
        }
        sign_record(TRANSFER_OFFER_DOMAIN, self.payload_bytes()?, signer_id, key)
    }

    /// The new controller's signed acceptance.
    ///
    /// # Errors
    ///
    /// [`DelegationError::WrongSigner`] when `key` is not the new key.
    pub fn sign_acceptance(
        &self,
        signer_id: &str,
        key: &SigningKey,
    ) -> Result<Envelope, DelegationError> {
        if key.verifying_key().to_bytes() != self.new_key {
            return Err(DelegationError::WrongSigner);
        }
        sign_record(
            TRANSFER_ACCEPT_DOMAIN,
            self.payload_bytes()?,
            signer_id,
            key,
        )
    }
}

/// Verifies a transfer of control away from `controller`.
///
/// # Errors
///
/// [`DelegationError`] when either record fails to verify, the two do
/// not sign identical statement bytes, a signer is not the statement's
/// key, the old key is not `controller`, both keys are the same, or the
/// statement has expired.
pub fn verify_transfer(
    offer: &Envelope,
    acceptance: &Envelope,
    controller: &[u8; 32],
    clock: Clock,
    profile: &Profile,
    dispatch: &ProfileDispatch,
) -> Result<TransferStatement, DelegationError> {
    check_record(offer, TRANSFER_OFFER_DOMAIN, dispatch)?;
    check_record(acceptance, TRANSFER_ACCEPT_DOMAIN, dispatch)?;
    if offer.payload != acceptance.payload {
        return Err(DelegationError::TransferMismatch);
    }
    let statement = TransferStatement::from_payload(&offer.payload)?;
    if offer.body.key != statement.old_key
        || acceptance.body.key != statement.new_key
        || statement.old_key != *controller
    {
        return Err(DelegationError::WrongSigner);
    }
    if statement.old_key == statement.new_key {
        return Err(DelegationError::SameKey);
    }
    if statement.expires.saturating_add(profile.clock_skew_secs) < clock.verifier_utc {
        return Err(DelegationError::Expired);
    }
    Ok(statement)
}
