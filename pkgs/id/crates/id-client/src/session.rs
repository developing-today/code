//! One authenticated client session against a test world.
//!
//! The session is deliberately transport-free: the test world issues a
//! 32-byte challenge, the client signs it with its Ed25519 identity key,
//! and the world verifies the signature against the client's public key.
//! No world is hosted, no listener is opened, and no server/runtime
//! dependency (`wasmtime`, `axum`, `russh`) is involved — which is the
//! property the Android dependency boundary needs.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

/// Errors from the client-only authenticated session.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// The signature did not verify against the client's public key.
    #[error("session signature did not verify")]
    BadSignature,
    /// The challenge is not the expected 32 bytes.
    #[error("challenge must be exactly 32 bytes")]
    BadChallenge,
}

/// A client-side Ed25519 identity (hardware backing arrives in slice 2;
///
/// here the key is held in memory).
#[derive(Debug)]
pub struct ClientIdentity {
    signing: SigningKey,
}

impl ClientIdentity {
    /// Generate a fresh random identity.
    #[must_use]
    pub fn generate() -> Self {
        Self {
            signing: SigningKey::generate(&mut rand::rng()),
        }
    }

    /// The public half the world pins for this client.
    #[must_use]
    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing.verifying_key()
    }

    /// Sign a 32-byte server challenge.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::BadChallenge`] when the challenge is not
    /// exactly 32 bytes.
    pub fn sign_challenge(&self, challenge: &[u8]) -> Result<Signature, SessionError> {
        if challenge.len() != 32 {
            return Err(SessionError::BadChallenge);
        }
        Ok(self.signing.sign(challenge))
    }
}

/// An in-memory test world: it issues challenges and verifies sessions.
///
/// This is the slice-1 stand-in for "a test world" in the acceptance
/// criteria — no hosting, no drive, no presence, just the auth gate the
/// later shell will build on.
#[derive(Debug)]
pub struct TestWorld {
    challenge: [u8; 32],
}

impl TestWorld {
    /// Create a test world with a fresh random challenge.
    #[must_use]
    pub fn new() -> Self {
        use rand::Rng as _;
        let mut challenge = [0_u8; 32];
        rand::rng().fill_bytes(&mut challenge);
        Self { challenge }
    }

    /// The current challenge the client must sign.
    #[must_use]
    pub const fn challenge(&self) -> &[u8; 32] {
        &self.challenge
    }

    /// Verify one authenticated client session.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::BadSignature`] when the signature does not
    /// verify against `client_key`.
    pub fn verify_session(
        &self,
        client_key: &VerifyingKey,
        signature: &Signature,
    ) -> Result<(), SessionError> {
        client_key
            .verify(&self.challenge, signature)
            .map_err(|_| SessionError::BadSignature)
    }
}

impl Default for TestWorld {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_roundtrip_verifies() {
        let client = ClientIdentity::generate();
        let world = TestWorld::new();
        let sig = client.sign_challenge(world.challenge()).unwrap();
        world.verify_session(&client.verifying_key(), &sig).unwrap();
    }

    #[test]
    fn test_wrong_key_rejected() {
        let client = ClientIdentity::generate();
        let other = ClientIdentity::generate();
        let world = TestWorld::new();
        let sig = client.sign_challenge(world.challenge()).unwrap();
        let err = world
            .verify_session(&other.verifying_key(), &sig)
            .unwrap_err();
        assert!(matches!(err, SessionError::BadSignature));
    }

    #[test]
    fn test_bad_challenge_length_rejected() {
        let client = ClientIdentity::generate();
        let err = client.sign_challenge(&[0_u8; 7]).unwrap_err();
        assert!(matches!(err, SessionError::BadChallenge));
    }
}
