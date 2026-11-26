//! Host/controller dispatch path for spill copies (#7572 successor
//! slice on top of the actor admission gate in [`super::spill_gate`]).
//!
//! Until this module there were **no production callers** of
//! `evaluate_spill` or [`WorldHandle::dispatch_spill_copy`]: the gate
//! existed, but nothing authenticated a successor's replay cursor,
//! passed the residency gate, and carried the copy across. This module
//! is that smallest typed adapter seam. The flow is fixed and has no
//! bypass:
//!
//! 1. **Authentication**: the host adapter ([`CursorAuthenticator`],
//!    supplied by the deployment's transport — session, channel, or
//!    credential proof, exactly as the pool roster's `authenticated`
//!    flag is "as established by the host adapter") verifies the
//!    presented proof for the exact `(holder, epoch, replayed_position)`
//!    triple. Only a verified triple can become an
//!    [`AuthenticatedReplayCursor`]; the type has no other constructor,
//!    so a raw position asserted by a node can never be dressed up as a
//!    verified cursor.
//! 2. **Residency**: the step-1 evaluator
//!    ([`id_core::residency::evaluate`]) must allow the spill path for
//!    the presented hops on the evidence clock.
//! 3. **Serialized final gate**: [`WorldHandle::dispatch_spill_copy`]
//!    asks the world actor to recheck the live holder, epoch, term, and
//!    the current grant/handoff replay boundary immediately before it
//!    releases the journal bytes. This is the final gate — a cursor
//!    authenticated against a stale snapshot still fails here after a
//!    fence, a handback, or a newer grant.
//!
//! Only the bytes the actor returns are handed to the caller's `send`,
//! exactly once. Every refusal — unauthenticated, residency, or the
//! actor's own admission check — sends **no bytes at all**.

use std::collections::BTreeMap;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use id_core::residency::{Decision, DenyReason, EvidenceVerifier, Hop, Scope, evaluate};

use super::WorldHandle;

/// Domain separator bound into every spill-cursor signature, so a
/// cursor proof can never be reinterpreted by another protocol in
/// this crate (the domain-separation rule `admission_lease` applies).
pub const SPILL_CURSOR_DOMAIN: &[u8] = b"developing-today/id/spill-cursor/v1\0";

/// Canonical bytes a cursor signature binds: the domain tag and the
/// exact `(holder, epoch, replayed_position)` triple. Any change to
/// any field after signing invalidates the proof.
fn cursor_signing_bytes(holder: &str, epoch: u64, replayed_position: u64) -> Option<Vec<u8>> {
    postcard::to_allocvec(&(SPILL_CURSOR_DOMAIN, holder, epoch, replayed_position)).ok()
}

/// Mint the transport proof for one cursor triple: an Ed25519
/// signature by the holder's own key over [`cursor_signing_bytes`].
/// This is the holder side of [`SignedCursorAuthenticator`]; a
/// deployment's transport carries the returned 64 bytes as the proof.
#[must_use]
pub fn sign_cursor_proof(
    key: &SigningKey,
    holder: &str,
    epoch: u64,
    replayed_position: u64,
) -> Vec<u8> {
    match cursor_signing_bytes(holder, epoch, replayed_position) {
        Some(bytes) => key.sign(&bytes).to_bytes().to_vec(),
        None => Vec::new(),
    }
}

/// Concrete [`CursorAuthenticator`] for a real deployment (#7572):
/// the roster pins each holder's Ed25519 verifying key, and a proof
/// authenticates only when it is a valid signature by the pinned key
/// for the presented holder over the exact cursor triple. An
/// unpinned holder, a malformed proof, a signature by any other key,
/// and a proof minted for another epoch or position all refuse.
#[derive(Clone, Debug, Default)]
pub struct SignedCursorAuthenticator {
    pinned: BTreeMap<String, VerifyingKey>,
}

impl SignedCursorAuthenticator {
    /// An authenticator with no pinned holders (refuses everything).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pin `key` as the authenticating key for `holder` (builder form
    /// of [`SignedCursorAuthenticator::pin`]).
    #[must_use]
    pub fn with_pinned(mut self, holder: impl Into<String>, key: VerifyingKey) -> Self {
        self.pin(holder, key);
        self
    }

    /// Pin `key` as the authenticating key for `holder`.
    pub fn pin(&mut self, holder: impl Into<String>, key: VerifyingKey) {
        self.pinned.insert(holder.into(), key);
    }
}

impl CursorAuthenticator for SignedCursorAuthenticator {
    fn authenticate_cursor(
        &self,
        holder: &str,
        epoch: u64,
        replayed_position: u64,
        proof: &[u8],
    ) -> bool {
        let Some(key) = self.pinned.get(holder) else {
            return false;
        };
        let Ok(signature) = Signature::from_slice(proof) else {
            return false;
        };
        let Some(bytes) = cursor_signing_bytes(holder, epoch, replayed_position) else {
            return false;
        };
        key.verify_strict(&bytes, &signature).is_ok()
    }
}

/// Host-adapter authentication for a presented replay cursor.
///
/// Implementations are provided by the deployment's host/controller
/// transport (the same seam that establishes roster authentication in
/// [`super::spill_pool`]): verify `proof` as evidence that `holder`,
/// at `epoch`, has genuinely replayed the journal contiguously to
/// `replayed_position`. Returning `false` must be the outcome for a
/// forged, stale-binding, or wrong-epoch proof; the controller treats
/// any `false` as a hard refusal and sends nothing.
pub trait CursorAuthenticator: Send + Sync {
    /// Whether `proof` authenticates the exact cursor triple.
    fn authenticate_cursor(
        &self,
        holder: &str,
        epoch: u64,
        replayed_position: u64,
        proof: &[u8],
    ) -> bool;
}

/// A successor replay cursor that a [`CursorAuthenticator`] has
/// verified for one exact holder and epoch.
///
/// The fields are private and the only constructor is
/// [`AuthenticatedReplayCursor::authenticate`]: holding a value of this
/// type is itself the evidence that authentication happened, so the
/// dispatch path never has to trust a bare position again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedReplayCursor {
    holder: String,
    epoch: u64,
    replayed_position: u64,
}

impl AuthenticatedReplayCursor {
    /// Verify a presented cursor triple against `authenticator`.
    ///
    /// # Errors
    ///
    /// [`SpillDispatchRefusal::UnauthenticatedCursor`] when the
    /// authenticator refuses the proof for this exact triple — a proof
    /// minted for another holder, epoch, or position does not transfer.
    pub fn authenticate(
        authenticator: &dyn CursorAuthenticator,
        holder: impl Into<String>,
        epoch: u64,
        replayed_position: u64,
        proof: &[u8],
    ) -> Result<Self, SpillDispatchRefusal> {
        let holder = holder.into();
        if !authenticator.authenticate_cursor(&holder, epoch, replayed_position, proof) {
            return Err(SpillDispatchRefusal::UnauthenticatedCursor {
                holder,
                epoch,
                replayed_position,
            });
        }
        Ok(Self {
            holder,
            epoch,
            replayed_position,
        })
    }

    /// The authenticated holder identity.
    #[must_use]
    pub fn holder(&self) -> &str {
        &self.holder
    }

    /// The lease epoch the cursor was authenticated for.
    #[must_use]
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The verified contiguous replay position.
    #[must_use]
    pub fn replayed_position(&self) -> u64 {
        self.replayed_position
    }
}

/// Why a controller dispatch sent no bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpillDispatchRefusal {
    /// The presented proof did not authenticate the cursor triple.
    UnauthenticatedCursor {
        /// Holder the proof was presented for.
        holder: String,
        /// Epoch the proof was presented for.
        epoch: u64,
        /// Position the proof was presented for.
        replayed_position: u64,
    },
    /// The residency evaluator denied the spill path.
    Residency(Vec<DenyReason>),
    /// The serialized authority actor refused admission at dispatch
    /// time (stale or wrong-epoch cursor, replay below the current
    /// grant/handoff boundary, expired or foreign-held lease). Carries
    /// the actor's refusal text, which names the gate reason.
    Authority(String),
}

/// The spill host controller: owns the deployment's
/// [`CursorAuthenticator`] and runs the fixed
/// authenticate → residency → actor-gate → send flow.
pub struct SpillController<'a> {
    authenticator: &'a dyn CursorAuthenticator,
}

impl<'a> SpillController<'a> {
    /// Build a controller over the host adapter's authenticator.
    #[must_use]
    pub fn new(authenticator: &'a dyn CursorAuthenticator) -> Self {
        Self { authenticator }
    }

    /// Authenticate a presented cursor triple with this controller's
    /// authenticator.
    ///
    /// # Errors
    ///
    /// [`SpillDispatchRefusal::UnauthenticatedCursor`] on a refused proof.
    pub fn authenticate(
        &self,
        holder: impl Into<String>,
        epoch: u64,
        replayed_position: u64,
        proof: &[u8],
    ) -> Result<AuthenticatedReplayCursor, SpillDispatchRefusal> {
        AuthenticatedReplayCursor::authenticate(
            self.authenticator,
            holder,
            epoch,
            replayed_position,
            proof,
        )
    }

    /// Dispatch `journal_copy` for an already-authenticated cursor:
    /// pass the residency gate, then let the actor's serialized
    /// admission check make the final holder/epoch/term/replay decision,
    /// then call `send` exactly once with the bytes the actor released.
    ///
    /// # Errors
    ///
    /// [`SpillDispatchRefusal::Residency`] when the residency evaluator
    /// denies the path, or [`SpillDispatchRefusal::Authority`] when the
    /// actor refuses admission. On every error, `send` is never called:
    /// no journal bytes cross.
    pub async fn dispatch_copy(
        &self,
        handle: &WorldHandle,
        scope: &Scope,
        hops: &[Hop<'_>],
        now_s: u64,
        verifier: &dyn EvidenceVerifier,
        cursor: &AuthenticatedReplayCursor,
        journal_copy: Vec<u8>,
        send: impl FnOnce(&[u8]),
    ) -> Result<(), SpillDispatchRefusal> {
        if let Decision::Deny(reasons) = evaluate(scope, hops, now_s, verifier) {
            return Err(SpillDispatchRefusal::Residency(reasons));
        }
        let released = handle
            .dispatch_spill_copy(
                cursor.holder(),
                cursor.epoch(),
                cursor.replayed_position(),
                journal_copy,
            )
            .await
            .map_err(|err| SpillDispatchRefusal::Authority(err.to_string()))?;
        send(&released);
        Ok(())
    }

    /// The full presented-cursor path: authenticate the proof, then
    /// [`SpillController::dispatch_copy`]. This is the entry point a
    /// host/controller transport calls when a successor presents a
    /// replay cursor and asks for its spill copy.
    ///
    /// # Errors
    ///
    /// Any [`SpillDispatchRefusal`]; on every error, `send` is never
    /// called: no journal bytes cross.
    #[allow(clippy::too_many_arguments)]
    pub async fn dispatch_presented_copy(
        &self,
        handle: &WorldHandle,
        scope: &Scope,
        hops: &[Hop<'_>],
        now_s: u64,
        verifier: &dyn EvidenceVerifier,
        holder: impl Into<String>,
        epoch: u64,
        replayed_position: u64,
        proof: &[u8],
        journal_copy: Vec<u8>,
        send: impl FnOnce(&[u8]),
    ) -> Result<(), SpillDispatchRefusal> {
        let cursor = self.authenticate(holder, epoch, replayed_position, proof)?;
        self.dispatch_copy(
            handle,
            scope,
            hops,
            now_s,
            verifier,
            &cursor,
            journal_copy,
            send,
        )
        .await
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Deterministic host-adapter double: the only valid proof for a
    /// cursor is the one the "transport" minted for that exact triple,
    /// so a proof replayed for another holder, epoch, or position is
    /// forged by construction.
    struct MintingAuthenticator;

    fn minted_proof(holder: &str, epoch: u64, replayed_position: u64) -> Vec<u8> {
        format!("proof:{holder}:{epoch}:{replayed_position}").into_bytes()
    }

    impl CursorAuthenticator for MintingAuthenticator {
        fn authenticate_cursor(
            &self,
            holder: &str,
            epoch: u64,
            replayed_position: u64,
            proof: &[u8],
        ) -> bool {
            proof == minted_proof(holder, epoch, replayed_position).as_slice()
        }
    }

    fn holder_key() -> SigningKey {
        SigningKey::from_bytes(&[11_u8; 32])
    }

    fn signed_auth() -> SignedCursorAuthenticator {
        SignedCursorAuthenticator::new().with_pinned("native-2", holder_key().verifying_key())
    }

    // Oracle: #7572's remaining item — a concrete transport
    // authenticator for a real deployment. A holder-signed proof for
    // the exact triple authenticates through the controller seam.
    #[test]
    fn a_holder_signed_proof_authenticates_the_exact_triple() {
        let proof = sign_cursor_proof(&holder_key(), "native-2", 3, 7);
        let cursor =
            AuthenticatedReplayCursor::authenticate(&signed_auth(), "native-2", 3, 7, &proof)
                .unwrap();
        assert_eq!(cursor.holder(), "native-2");
        assert_eq!(cursor.epoch(), 3);
        assert_eq!(cursor.replayed_position(), 7);
    }

    // Oracle: the signature binds every field — a proof minted for
    // one triple, key, or holder authenticates no other.
    #[test]
    fn a_signed_proof_never_rebinds_or_transfers() {
        let auth = signed_auth();
        let proof = sign_cursor_proof(&holder_key(), "native-2", 3, 7);
        // Wrong epoch / stale position / different holder.
        assert!(AuthenticatedReplayCursor::authenticate(&auth, "native-2", 4, 7, &proof).is_err());
        assert!(AuthenticatedReplayCursor::authenticate(&auth, "native-2", 3, 2, &proof).is_err());
        assert!(AuthenticatedReplayCursor::authenticate(&auth, "native-9", 3, 7, &proof).is_err());
        // A signature by an unpinned key for the same triple.
        let stranger = SigningKey::from_bytes(&[13_u8; 32]);
        let forged = sign_cursor_proof(&stranger, "native-2", 3, 7);
        assert!(AuthenticatedReplayCursor::authenticate(&auth, "native-2", 3, 7, &forged).is_err());
        // Malformed (non-signature) proof bytes.
        assert!(
            AuthenticatedReplayCursor::authenticate(&auth, "native-2", 3, 7, b"proof").is_err()
        );
    }

    #[test]
    fn an_authenticated_cursor_carries_the_exact_verified_triple() {
        let cursor = AuthenticatedReplayCursor::authenticate(
            &MintingAuthenticator,
            "native-2",
            3,
            7,
            &minted_proof("native-2", 3, 7),
        )
        .unwrap();
        assert_eq!(cursor.holder(), "native-2");
        assert_eq!(cursor.epoch(), 3);
        assert_eq!(cursor.replayed_position(), 7);
    }

    #[test]
    fn a_forged_or_rebound_proof_never_becomes_a_cursor() {
        let auth = MintingAuthenticator;
        // Pure forgery.
        assert_eq!(
            AuthenticatedReplayCursor::authenticate(&auth, "native-2", 3, 7, b"forged"),
            Err(SpillDispatchRefusal::UnauthenticatedCursor {
                holder: "native-2".to_string(),
                epoch: 3,
                replayed_position: 7,
            })
        );
        // A genuine proof minted for a stale position does not
        // authenticate the boundary position.
        assert!(
            AuthenticatedReplayCursor::authenticate(
                &auth,
                "native-2",
                3,
                7,
                &minted_proof("native-2", 3, 2),
            )
            .is_err()
        );
        // Nor does a proof minted at the right position for the wrong epoch.
        assert!(
            AuthenticatedReplayCursor::authenticate(
                &auth,
                "native-2",
                4,
                7,
                &minted_proof("native-2", 3, 7),
            )
            .is_err()
        );
        // Nor one minted for a different holder.
        assert!(
            AuthenticatedReplayCursor::authenticate(
                &auth,
                "native-9",
                3,
                7,
                &minted_proof("native-2", 3, 7),
            )
            .is_err()
        );
    }
}
