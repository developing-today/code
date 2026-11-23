//! RS256 signing with the Slice 2 OIDC private key.
//!
//! The key is loaded from its on-host path (under `IDP_CA_HOME`,
//! `~/idp-ca/issued/oidc-signing/oidc-signing.key`), where it sits
//! passphrase-encrypted (PKCS#8 PBES2 / PBKDF2-HMAC-SHA256). The
//! passphrase arrives in process memory only (the launcher fetches
//! it from the vault by name, `idp.ca-passphrase`, into the process
//! environment); the key is decrypted in memory and the plaintext
//! key is never written to disk, the repo, or any log.

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
    key: RsaPrivateKey,
    kid: String,
}

impl TokenSigner {
    pub fn new(key: RsaPrivateKey, kid: String) -> Self {
        TokenSigner { key, kid }
    }

    pub fn kid(&self) -> &str {
        &self.kid
    }

    /// RSA public components (n, e) as big-endian bytes — used at
    /// startup to prove the loaded private key matches the published
    /// certificate behind the JWKS.
    pub fn public_components(&self) -> (Vec<u8>, Vec<u8>) {
        use rsa::traits::PublicKeyParts;
        let pubkey = rsa::RsaPublicKey::from(&self.key);
        (pubkey.n().to_bytes_be(), pubkey.e().to_bytes_be())
    }

    /// Mint a compact JWS (RS256) over the given claims object.
    pub fn sign_jwt(&self, claims: &serde_json::Value) -> Result<String, String> {
        let header = serde_json::json!({
            "alg": "RS256",
            "typ": "JWT",
            "kid": self.kid,
        });
        let encode = |v: &serde_json::Value| -> Result<String, String> {
            let bytes = serde_json::to_vec(v).map_err(|e| format!("jwt encode failed: {e}"))?;
            Ok(crate::b64url(&bytes))
        };
        let signing_input = format!("{}.{}", encode(&header)?, encode(claims)?);
        let signing_key = SigningKey::<Sha256>::new(self.key.clone());
        let signature: Signature = signing_key.sign(signing_input.as_bytes());
        use rsa::signature::SignatureEncoding;
        Ok(format!(
            "{}.{}",
            signing_input,
            crate::b64url(&signature.to_bytes())
        ))
    }

    /// Verify a compact JWS against this signer's public key and
    /// return its claims. Signature only — claim validation (iss,
    /// exp, token_use) is the caller's job.
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
        if header["kid"] != self.kid {
            return Err("unexpected jwt kid".to_string());
        }
        let signature_bytes = decode(parts[2])?;
        let signature = Signature::try_from(signature_bytes.as_slice())
            .map_err(|_| "bad jwt signature encoding")?;
        let verifying_key = VerifyingKey::<Sha256>::new(rsa::RsaPublicKey::from(&self.key));
        verifying_key
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
            .map_err(|_| "jwt signature verification failed")?;
        serde_json::from_slice(&decode(parts[1])?).map_err(|_| "bad jwt claims".to_string())
    }
}
