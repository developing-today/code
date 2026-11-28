//! B05 signer identity binding — key-to-signer binding and legacy
//! preimage compatibility (slice B05, #10208; design A-9114, Part B
//! "Conformance and tests").
//!
//! The contract, in one paragraph: a signature proves only that
//! *some* key produced it; it says nothing about **who** the signer
//! claims to be. B05 makes the second question explicit. A verifier
//! holds a registry of bindings, each naming one identity and the
//! verifying keys that identity may sign with. Verifying a signed
//! artifact is a two-part decision: (1) the key that produced the
//! signature must be bound to the claimed identity — a valid
//! signature under an unbound key, or under a key bound to a
//! *different* identity, is rejected as an identity failure, never
//! as a mere crypto failure; and only then (2) the signature must
//! verify under that key. Possession of a key therefore never
//! implies the identity it claims: the binding is the authority, and
//! the registry is where rotation and delegation are recorded.
//!
//! Legacy compatibility: the existing `SignedBody` preimage in
//! `artifact.rs` is `serde_json::to_vec` of a fixed field order
//! (`author`, `seq`, `prev`, `kind`, `body`, optional `after`) — not
//! the canonical-CBOR, domain-prefixed profile the newer slices
//! describe. That mismatch is documented here deliberately and is
//! **not** repaired: old artifacts keep verifying over their exact
//! old bytes. [`legacy_preimage_v1`] reproduces those bytes and the
//! integration test pins a literal fixture plus a cross-check against
//! `id_sync::artifact::signed_bytes`, so any drift fails the test
//! instead of silently re-baselining history. Never re-sign old
//! data.
//!
//! This module is deliberately self-contained: the federation roots
//! record slice paths without declaring them (slice 9110-02), so the
//! slice's integration test includes this file directly and no shared
//! root names it yet; #9110-09 wires versioned public exports after
//! module outputs land.

use ed25519_dalek::{Signature, VerifyingKey};
use serde::Serialize;

/// Ed25519 public-key length in bytes.
pub const ED25519_PUBLIC_KEY_LEN: usize = 32;

/// Ed25519 signature length in bytes.
pub const ED25519_SIGNATURE_LEN: usize = 64;

/// Lowercase hex of arbitrary bytes. Kept local so the module does
/// not depend on crate-internal helpers when included by path.
fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Stable fingerprint of a verifying key: lowercase hex of its bytes.
pub fn key_fingerprint(verifying_key: &[u8; ED25519_PUBLIC_KEY_LEN]) -> String {
    to_hex(verifying_key)
}

/// One explicit key-to-identity binding: `identity` may sign with
/// `verifying_key`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignerBinding {
    /// The signer identity as claimed on the wire (an account id, an
    /// author string, or a federation peer id).
    pub identity: String,
    /// The Ed25519 verifying key bound to that identity.
    pub verifying_key: [u8; ED25519_PUBLIC_KEY_LEN],
}

/// Successful identity-bound verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedIdentity {
    /// The claimed and confirmed identity.
    pub identity: String,
    /// Fingerprint of the key that produced the valid signature, so
    /// a rotated-away-but-then-current key stays attributable.
    pub key_fingerprint: String,
}

/// Why identity binding verification failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingError {
    /// The claimed identity has no binding at all.
    UnknownIdentity(String),
    /// The key is not bound to any identity. A signature may still
    /// verify under it — that is precisely the case this slice
    /// rejects, because presence is not identity.
    UnboundKey { key_fingerprint: String },
    /// The key is bound, but to a different identity than the one
    /// claimed: an impersonation attempt, not a crypto failure.
    IdentityMismatch {
        claimed: String,
        bound_to: String,
        key_fingerprint: String,
    },
    /// The key is bound to the claimed identity, but the signature
    /// does not verify over the given preimage.
    BadSignature,
}

impl std::fmt::Display for BindingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownIdentity(identity) => {
                write!(f, "identity {identity} has no key binding")
            }
            Self::UnboundKey { key_fingerprint } => {
                write!(f, "key {key_fingerprint} is not bound to any identity")
            }
            Self::IdentityMismatch {
                claimed,
                bound_to,
                key_fingerprint,
            } => write!(
                f,
                "key {key_fingerprint} is bound to {bound_to}, not claimed identity {claimed}"
            ),
            Self::BadSignature => write!(f, "signature does not verify"),
        }
    }
}

impl std::error::Error for BindingError {}

/// A registry of explicit key-to-identity bindings.
///
/// Invariants: one key is bound to at most one identity (binding the
/// same key to a second identity is an error, never a silent
/// overwrite), while one identity may hold several keys so that key
/// rotation (slice B01) can overlap generations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BindingRegistry {
    bindings: Vec<SignerBinding>,
}

impl BindingRegistry {
    /// An empty registry: every verification fails closed until a
    /// binding is recorded.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every recorded binding, in insertion order. Auditable without
    /// trusting the registry's internals.
    pub fn bindings(&self) -> &[SignerBinding] {
        &self.bindings
    }

    /// The identity a key is bound to, if any.
    pub fn identity_for(&self, verifying_key: &[u8; ED25519_PUBLIC_KEY_LEN]) -> Option<&str> {
        self.bindings
            .iter()
            .find(|binding| &binding.verifying_key == verifying_key)
            .map(|binding| binding.identity.as_str())
    }

    /// Whether `identity` may sign with `verifying_key`.
    pub fn is_bound(&self, identity: &str, verifying_key: &[u8; ED25519_PUBLIC_KEY_LEN]) -> bool {
        self.bindings
            .iter()
            .any(|binding| binding.identity == identity && &binding.verifying_key == verifying_key)
    }

    /// Bind `verifying_key` to `identity`. Re-binding the same pair
    /// is idempotent; binding an already-bound key to a different
    /// identity is refused.
    pub fn bind(
        &mut self,
        identity: &str,
        verifying_key: [u8; ED25519_PUBLIC_KEY_LEN],
    ) -> Result<(), BindingError> {
        if let Some(existing) = self.identity_for(&verifying_key) {
            if existing == identity {
                return Ok(());
            }
            return Err(BindingError::IdentityMismatch {
                claimed: identity.to_owned(),
                bound_to: existing.to_owned(),
                key_fingerprint: key_fingerprint(&verifying_key),
            });
        }
        self.bindings.push(SignerBinding {
            identity: identity.to_owned(),
            verifying_key,
        });
        Ok(())
    }

    /// Remove every binding for `identity` (key retirement). Returns
    /// how many bindings were removed.
    pub fn retire_identity(&mut self, identity: &str) -> usize {
        let before = self.bindings.len();
        self.bindings.retain(|binding| binding.identity != identity);
        before - self.bindings.len()
    }
}

/// Whether *some* key produced this signature — and nothing more.
///
/// This helper exists to make the distinction explicit: it proves
/// possession of a key, not an identity. Callers that care who the
/// signer is must use [`verify_bound_signature`] with a registry;
/// using this function for authorization is the bug B05 closes.
pub fn verify_signature_presence(
    verifying_key: &[u8; ED25519_PUBLIC_KEY_LEN],
    preimage: &[u8],
    signature: &[u8; ED25519_SIGNATURE_LEN],
) -> Result<(), BindingError> {
    let key = VerifyingKey::from_bytes(verifying_key).map_err(|_| BindingError::BadSignature)?;
    let sig = Signature::from_bytes(signature);
    key.verify_strict(preimage, &sig)
        .map_err(|_| BindingError::BadSignature)
}

/// Verify a signed preimage **and bind its key to the claimed
/// identity**.
///
/// The order is the point: identity is resolved from the registry
/// before the signature is even parsed, so an unbound key or a key
/// bound to another identity fails with an identity error even when
/// the signature itself is cryptographically valid.
pub fn verify_bound_signature(
    registry: &BindingRegistry,
    claimed_identity: &str,
    verifying_key: &[u8; ED25519_PUBLIC_KEY_LEN],
    preimage: &[u8],
    signature: &[u8; ED25519_SIGNATURE_LEN],
) -> Result<VerifiedIdentity, BindingError> {
    let bound_to =
        registry
            .identity_for(verifying_key)
            .ok_or_else(|| BindingError::UnboundKey {
                key_fingerprint: key_fingerprint(verifying_key),
            })?;
    if bound_to != claimed_identity {
        return Err(BindingError::IdentityMismatch {
            claimed: claimed_identity.to_owned(),
            bound_to: bound_to.to_owned(),
            key_fingerprint: key_fingerprint(verifying_key),
        });
    }
    if !registry
        .bindings()
        .iter()
        .any(|binding| binding.identity == claimed_identity)
    {
        return Err(BindingError::UnknownIdentity(claimed_identity.to_owned()));
    }
    verify_signature_presence(verifying_key, preimage, signature)?;
    Ok(VerifiedIdentity {
        identity: claimed_identity.to_owned(),
        key_fingerprint: key_fingerprint(verifying_key),
    })
}

/// A legacy (v1) artifact signed body. Field order and serde
/// attributes mirror `artifact::SignedBody` exactly; the mismatch
/// with the documented canonical-CBOR profile is preserved on
/// purpose (see the module docs).
pub struct LegacyBodyV1<'a> {
    /// Hex Ed25519 public key of the author (the legacy wire form).
    pub author: &'a str,
    /// One-based position in the author's chain.
    pub seq: u64,
    /// Hash of the previous artifact, empty for `seq` 1.
    pub prev: &'a str,
    /// The artifact kind.
    pub kind: &'a str,
    /// The kind-specific body.
    pub body: &'a serde_json::Value,
    /// The author's applied-artifact frontier; omitted when empty.
    pub after: &'a [String],
}

#[derive(Serialize)]
struct LegacySignedBodyV1<'a> {
    author: &'a str,
    seq: u64,
    prev: &'a str,
    kind: &'a str,
    body: &'a serde_json::Value,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    after: Vec<String>,
}

/// The exact legacy preimage bytes: compact `serde_json` of the v1
/// signed body. Byte-for-byte stable; never re-serialize old
/// artifacts with a different writer.
pub fn legacy_preimage_v1(body: &LegacyBodyV1<'_>) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&LegacySignedBodyV1 {
        author: body.author,
        seq: body.seq,
        prev: body.prev,
        kind: body.kind,
        body: body.body,
        after: body.after.to_vec(),
    })
}
