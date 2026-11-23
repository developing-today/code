//! RS256 signing with the PKI-issued OIDC private keys.
//!
//! Keys are loaded from their on-host paths (under `IDP_CA_HOME`,
//! `~/idp-ca/issued/...`), where they sit passphrase-encrypted
//! (PKCS#8 PBES2 / PBKDF2-HMAC-SHA256). The passphrase arrives in
//! process memory only (the launcher fetches it from the vault by
//! name, `idp.ca-passphrase`, into the process environment); keys
//! are decrypted in memory and plaintext key material is never
//! written to disk, the repo, or any log.
//!
//! The signer holds a *set* of keys keyed by `kid` (Slice 5 key
//! rollover, #13658): tokens are signed with the configured active
//! key, and verification accepts any key in the set, selected by
//! the token header's `kid`. During a rollover overlap the set
//! holds the old and new keys, so pre-rollover tokens keep
//! verifying; retiring a key is removing it from the set.

use der::Decode;
use pkcs8::EncryptedPrivateKeyInfo;
use rsa::pkcs1v15::{Signature, SigningKey, VerifyingKey};
use rsa::pkcs8::DecodePrivateKey;
use rsa::signature::{Signer as _, Verifier as _};
use rsa::RsaPrivateKey;
use sha2::Sha256;
use std::path::Path;

/// Load and decrypt the PKCS#8-encrypted RSA private key at `path`.
/// Error strings never include key material or the passphrase.
pub fn load_signing_key(path: &Path, passphrase: &str) -> Result<RsaPrivateKey, String> {
    let pem = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read signing key at {}: {e}", path.display()))?;
    let der_bytes = crate::pem_to_der(&pem)?;
    let encrypted = EncryptedPrivateKeyInfo::from_der(&der_bytes)
        .map_err(|e| format!("signing key is not an encrypted PKCS#8 key: {e}"))?;
    let info = encrypted
        .decrypt(passphrase.as_bytes())
        .map_err(|_| "signing key decryption failed (wrong passphrase or unsupported scheme)")?;
    let der = info.as_bytes().to_vec();
    RsaPrivateKey::from_pkcs8_der(&der)
        .map_err(|e| format!("decrypted key is not a valid RSA PKCS#8 key: {e}"))
}

#[derive(Clone)]
pub struct TokenSigner {
    /// (kid, key) pairs — every key verification accepts.
    keys: Vec<(String, RsaPrivateKey)>,
    /// The kid new tokens are signed with.
    active_kid: String,
}

impl TokenSigner {
    /// A signer holding exactly one key (the pre-rollover shape,
    /// and what tests use).
    pub fn new(key: RsaPrivateKey, kid: String) -> Self {
        TokenSigner {
            keys: vec![(kid.clone(), key)],
            active_kid: kid,
        }
    }

    /// A signer holding several keys, signing with `active_kid`.
    /// Fails when the set is empty, contains a duplicate kid, or
    /// the active kid is not in the set.
    pub fn from_keys(keys: Vec<(RsaPrivateKey, String)>, active_kid: &str) -> Result<Self, String> {
        if keys.is_empty() {
            return Err("signing key set is empty".to_string());
        }
        let mut seen: Vec<&str> = Vec::new();
        for (_, kid) in &keys {
            if seen.contains(&kid.as_str()) {
                return Err(format!("duplicate signing kid in key set: {kid}"));
            }
            seen.push(kid.as_str());
        }
        if !seen.contains(&active_kid) {
            return Err(format!(
                "active signing kid {active_kid} is not in the key set"
            ));
        }
        Ok(TokenSigner {
            keys: keys.into_iter().map(|(key, kid)| (kid, key)).collect(),
            active_kid: active_kid.to_string(),
        })
    }

    /// The active signing kid (what new tokens carry).
    pub fn kid(&self) -> &str {
        &self.active_kid
    }

    /// Every kid verification currently accepts, active first is
    /// not guaranteed — order is configuration order.
    pub fn kids(&self) -> Vec<&str> {
        self.keys.iter().map(|(kid, _)| kid.as_str()).collect()
    }

    fn key_for(&self, kid: &str) -> Option<&RsaPrivateKey> {
        self.keys.iter().find(|(k, _)| k == kid).map(|(_, key)| key)
    }

    /// RSA public components (n, e) of the ACTIVE key, as
    /// big-endian bytes.
    pub fn public_components(&self) -> (Vec<u8>, Vec<u8>) {
        self.public_components_for(&self.active_kid)
            .expect("active key is in the set by construction")
    }

    /// RSA public components (n, e) of the key with `kid`, if the
    /// set holds it — used at startup to prove each loaded private
    /// key matches its published certificate.
    pub fn public_components_for(&self, kid: &str) -> Option<(Vec<u8>, Vec<u8>)> {
        use rsa::traits::PublicKeyParts;
        self.key_for(kid).map(|key| {
            let pubkey = rsa::RsaPublicKey::from(key);
            (pubkey.n().to_bytes_be(), pubkey.e().to_bytes_be())
        })
    }

    /// Mint a compact JWS (RS256) over the given claims object,
    /// signed with the active key.
    pub fn sign_jwt(&self, claims: &serde_json::Value) -> Result<String, String> {
        let header = serde_json::json!({
            "alg": "RS256",
            "typ": "JWT",
            "kid": self.active_kid,
        });
        let encode = |v: &serde_json::Value| -> Result<String, String> {
            let bytes = serde_json::to_vec(v).map_err(|e| format!("jwt encode failed: {e}"))?;
            Ok(crate::b64url(&bytes))
        };
        let signing_input = format!("{}.{}", encode(&header)?, encode(claims)?);
        let key = self
            .key_for(&self.active_kid)
            .ok_or("active signing key missing")?;
        let signing_key = SigningKey::<Sha256>::new(key.clone());
        let signature: Signature = signing_key.sign(signing_input.as_bytes());
        use rsa::signature::SignatureEncoding;
        Ok(format!(
            "{}.{}",
            signing_input,
            crate::b64url(&signature.to_bytes())
        ))
    }

    /// Verify a compact JWS against the key set — the token's
    /// header `kid` selects the key — and return its claims.
    /// Signature only; claim validation (iss, exp, token_use) is
    /// the caller's job.
    pub fn verify_jwt(&self, token: &str) -> Result<serde_json::Value, String> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err("token is not a compact JWS".to_string());
        }
        let decode = |s: &str| -> Result<Vec<u8>, String> {
            base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, s)
                .map_err(|e| format!("token base64url decode failed: {e}"))
        };
        let header: serde_json::Value =
            serde_json::from_slice(&decode(parts[0])?).map_err(|_| "bad jwt header")?;
        if header["alg"] != "RS256" {
            return Err("unexpected jwt alg".to_string());
        }
        let kid = header["kid"].as_str().unwrap_or("");
        let key = self
            .key_for(kid)
            .ok_or_else(|| "unexpected jwt kid".to_string())?;
        let signature_bytes = decode(parts[2])?;
        let signature = Signature::try_from(signature_bytes.as_slice())
            .map_err(|_| "bad jwt signature encoding")?;
        let verifying_key = VerifyingKey::<Sha256>::new(rsa::RsaPublicKey::from(key));
        verifying_key
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
            .map_err(|_| "jwt signature verification failed")?;
        serde_json::from_slice(&decode(parts[1])?).map_err(|_| "bad jwt claims".to_string())
    }
}
