//! Friend envelopes: a request or acceptance, signed by its sender's
//! account key (moved out of the facade's `directory` module in step 3c of
//! the crate split, #4406). The directory's acceptance policy (time-to-live,
//! pending-request caps) stays with the directory; this is the wire type
//! and its signature.

use anyhow::{Context, Result, bail};
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use rand::RngExt as _;
use serde::{Deserialize, Serialize};

use crate::hex::{hex_encode, hex_to_array};

/// What a friend envelope says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvelopeKind {
    /// A request to become friends.
    FriendRequest,
    /// Acceptance of a request.
    FriendAccept,
}

/// A friend request or acceptance, signed by its sender's account key. Anyone
/// holding it can check the signature without asking the server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    /// What the envelope says.
    pub kind: EnvelopeKind,
    /// The sender's account ID.
    pub from: String,
    /// The recipient's account ID.
    pub to: String,
    /// The world ID of the server the envelope is for.
    pub audience: String,
    /// A random ID unique to this envelope. A request's ID names the request.
    pub id: String,
    /// For an acceptance, the ID of the request it accepts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
    /// When it was made, in Unix milliseconds.
    pub at: u64,
    /// Hex Ed25519 signature over the canonical body.
    pub signature: String,
}

/// The canonical envelope body the signature covers.
#[derive(Serialize)]
pub struct SignedBody<'a> {
    kind: EnvelopeKind,
    from: &'a str,
    to: &'a str,
    audience: &'a str,
    id: &'a str,
    request: Option<&'a str>,
    at: u64,
}

impl Envelope {
    pub fn body(&self) -> SignedBody<'_> {
        SignedBody {
            kind: self.kind,
            from: &self.from,
            to: &self.to,
            audience: &self.audience,
            id: &self.id,
            request: self.request.as_deref(),
            at: self.at,
        }
    }

    /// Sign an envelope with `key`, giving it a fresh random ID.
    ///
    /// `from` must name `key`: [`Envelope::verify`] checks the signature
    /// against the key that `from` spells, so an envelope signed by any
    /// other key could never verify anywhere. That mismatch is refused
    /// here, at sign time, rather than discovered at every receiver
    /// (#8843).
    ///
    /// # Errors
    ///
    /// Fails if `from` is not the hex of `key`'s verifying key, or if the
    /// canonical body does not encode.
    pub fn signed(
        key: &SigningKey,
        kind: EnvelopeKind,
        from: &str,
        to: &str,
        audience: &str,
        request: Option<&str>,
        at: u64,
    ) -> Result<Self> {
        // Mirror the receipt rule exactly: `verify` decodes `from` as a
        // 32-byte key, so compare the decoded bytes, not the spelling.
        let named = hex_to_array::<32>(from);
        if named != Some(key.verifying_key().to_bytes()) {
            bail!(
                "envelope sender `{from}` is not the signing key's account ID: \
                 an envelope verifies only against the key its sender names"
            );
        }
        let mut nonce = [0_u8; 16];
        rand::rng().fill(&mut nonce);
        let mut envelope = Self {
            kind,
            from: from.to_owned(),
            to: to.to_owned(),
            audience: audience.to_owned(),
            id: hex_encode(&nonce),
            request: request.map(str::to_owned),
            at,
            signature: String::new(),
        };
        let signature = key.sign(&serde_json::to_vec(&envelope.body())?);
        envelope.signature = hex_encode(&signature.to_bytes());
        Ok(envelope)
    }

    /// Check the signature against the sender's account key.
    ///
    /// # Errors
    ///
    /// Fails if the sender is not a key, the signature is malformed, or it
    /// does not match the body.
    pub fn verify(&self) -> Result<()> {
        let key = VerifyingKey::from_bytes(
            &hex_to_array::<32>(&self.from).context("malformed envelope sender")?,
        )
        .context("envelope sender is not a key")?;
        let signature = Signature::from_bytes(
            &hex_to_array::<64>(&self.signature).context("malformed envelope signature")?,
        );
        let body = serde_json::to_vec(&self.body())?;
        key.verify_strict(&body, &signature)
            .context("envelope signature does not verify")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn account_id(key: &SigningKey) -> String {
        hex_encode(&key.verifying_key().to_bytes())
    }

    #[test]
    fn an_envelope_from_its_own_key_signs_and_verifies() {
        let key = SigningKey::from_bytes(&[1; 32]);
        let to = account_id(&SigningKey::from_bytes(&[2; 32]));
        let envelope = Envelope::signed(
            &key,
            EnvelopeKind::FriendRequest,
            &account_id(&key),
            &to,
            "home",
            None,
            1,
        )
        .unwrap();
        assert!(envelope.verify().is_ok());
    }

    #[test]
    fn a_sender_that_is_not_the_signing_key_is_refused_at_sign_time() {
        let key = SigningKey::from_bytes(&[1; 32]);
        let other = account_id(&SigningKey::from_bytes(&[2; 32]));
        for from in [other.as_str(), "not-hex", ""] {
            let err = Envelope::signed(
                &key,
                EnvelopeKind::FriendAccept,
                from,
                &other,
                "home",
                None,
                1,
            )
            .unwrap_err();
            assert!(
                err.to_string()
                    .contains("is not the signing key's account ID"),
                "{err}"
            );
        }
    }
}
