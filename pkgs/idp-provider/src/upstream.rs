//! Upstream federated doors (#13411 GitHub, #13412 Google, #13414
//! Microsoft): the HTTP client side of talking to an upstream
//! identity provider — authorization-code exchange, upstream
//! ID-token verification against the upstream's JWKS, and the
//! GitHub user fetch.
//!
//! A door's client secret is never in the repo or the provider
//! config file: the config names an environment variable
//! (`client_secret_env`), and the launcher fills it from the vault
//! by name — the same pattern as `idp.ca-passphrase` for the
//! signing key. A door whose secret is absent (or, for OIDC doors,
//! whose expected issuer is unconfigured) is disabled at startup:
//! its start endpoint denies with an audit record and nothing else
//! about the provider changes.
//!
//! Error strings from this module never include upstream response
//! bodies, tokens, or the client secret — an upstream failure is a
//! reason string, not a transcript.

use crate::registry::UpstreamDoor;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD as B64URL};
use rsa::pkcs1v15::{Signature, VerifyingKey};
use rsa::signature::Verifier as _;
use rsa::{BigUint, RsaPublicKey};
use sha2::Sha256;

#[derive(Clone)]
pub struct DoorRuntime {
    pub config: UpstreamDoor,
    /// The door's client secret, resolved from the configured
    /// environment variable at startup. `None` = door disabled.
    pub client_secret: Option<String>,
}

impl DoorRuntime {
    pub fn name(&self) -> &str {
        &self.config.name
    }

    pub fn enabled(&self) -> bool {
        if self.client_secret.is_none() {
            return false;
        }
        // OIDC doors additionally require the expected issuer and a
        // JWKS URI: without them there is nothing to verify against.
        if self.config.kind == "oidc" {
            return self.config.issuer.is_some() && self.config.jwks_uri.is_some();
        }
        true
    }

    /// The identity provider key this door mints (`github`,
    /// `google`, ...). Allowlist and identity keys are
    /// `<door>:<subject>`.
    pub fn identity_provider(&self) -> &str {
        &self.config.name
    }

    /// The callback URL the upstream returns to.
    pub fn callback_uri(&self, issuer: &str) -> String {
        format!("{issuer}/door/{}/callback", self.config.name)
    }
}

/// One RSA key from an upstream JWKS document.
pub struct UpstreamJwk {
    pub kid: Option<String>,
    pub pubkey: RsaPublicKey,
}

fn b64url_decode(s: &str) -> Result<Vec<u8>, String> {
    B64URL
        .decode(s)
        .map_err(|e| format!("base64url decode failed: {e}"))
}

/// Parse an upstream JWKS JSON document into RSA public keys.
/// Non-RSA keys are skipped (this provider verifies RS256 only).
pub fn parse_upstream_jwks(doc: &serde_json::Value) -> Result<Vec<UpstreamJwk>, String> {
    let keys = doc["keys"]
        .as_array()
        .ok_or("upstream JWKS has no keys array")?;
    let mut out = Vec::new();
    for k in keys {
        if k["kty"] != "RSA" {
            continue;
        }
        let (n_s, e_s) = match (k["n"].as_str(), k["e"].as_str()) {
            (Some(n), Some(e)) => (n, e),
            _ => continue,
        };
        let n = BigUint::from_bytes_be(&b64url_decode(n_s)?);
        let e = BigUint::from_bytes_be(&b64url_decode(e_s)?);
        let pubkey = RsaPublicKey::new(n, e).map_err(|e| format!("upstream JWK invalid: {e}"))?;
        out.push(UpstreamJwk {
            kid: k["kid"].as_str().map(|s| s.to_string()),
            pubkey,
        });
    }
    if out.is_empty() {
        return Err("upstream JWKS contains no usable RSA keys".to_string());
    }
    Ok(out)
}

/// Verify a compact JWS (RS256) against an explicit RSA public key
/// and return (header, claims). Signature only — claim validation
/// (iss/aud/exp/nonce) is the caller's job.
pub fn verify_rs256(
    token: &str,
    pubkey: &RsaPublicKey,
) -> Result<(serde_json::Value, serde_json::Value), String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("token is not a compact JWS".to_string());
    }
    let header: serde_json::Value =
        serde_json::from_slice(&b64url_decode(parts[0])?).map_err(|_| "bad jwt header")?;
    if header["alg"] != "RS256" {
        return Err("unexpected jwt alg".to_string());
    }
    let signature_bytes = b64url_decode(parts[2])?;
    let signature = Signature::try_from(signature_bytes.as_slice())
        .map_err(|_| "bad jwt signature encoding")?;
    let verifying_key = VerifyingKey::<Sha256>::new(pubkey.clone());
    verifying_key
        .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
        .map_err(|_| "jwt signature verification failed")?;
    let claims: serde_json::Value =
        serde_json::from_slice(&b64url_decode(parts[1])?).map_err(|_| "bad jwt claims")?;
    Ok((header, claims))
}

/// Fetch and parse an upstream JWKS document.
pub async fn fetch_upstream_jwks(
    http: &reqwest::Client,
    uri: &str,
) -> Result<Vec<UpstreamJwk>, String> {
    let doc: serde_json::Value = http
        .get(uri)
        .send()
        .await
        .map_err(|_| "upstream JWKS fetch failed".to_string())?
        .error_for_status()
        .map_err(|_| "upstream JWKS fetch failed".to_string())?
        .json()
        .await
        .map_err(|_| "upstream JWKS fetch failed".to_string())?;
    parse_upstream_jwks(&doc)
}

/// Exchange an authorization code at the upstream token endpoint.
/// Returns the token endpoint's JSON document. For OIDC doors the
/// PKCE verifier is included; GitHub's token endpoint is asked for
/// JSON explicitly.
pub async fn exchange_upstream_code(
    http: &reqwest::Client,
    door: &DoorRuntime,
    code: &str,
    redirect_uri: &str,
    pkce_verifier: Option<&str>,
) -> Result<serde_json::Value, String> {
    let secret = door
        .client_secret
        .as_deref()
        .ok_or("door is not configured")?;
    let mut form: Vec<(&str, &str)> = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", door.config.client_id.as_str()),
        ("client_secret", secret),
    ];
    if let Some(v) = pkce_verifier {
        form.push(("code_verifier", v));
    }
    let doc: serde_json::Value = http
        .post(&door.config.token_url)
        .header(reqwest::header::ACCEPT, "application/json")
        .form(&form)
        .send()
        .await
        .map_err(|_| "upstream token exchange failed".to_string())?
        .error_for_status()
        .map_err(|_| "upstream token exchange failed".to_string())?
        .json()
        .await
        .map_err(|_| "upstream token exchange failed".to_string())?;
    Ok(doc)
}

/// Verify an upstream OIDC ID token: signature against the
/// upstream JWKS (selected by the header `kid`), then iss / aud /
/// exp / nonce. Returns the token's claims.
pub fn verify_upstream_id_token(
    id_token: &str,
    jwks: &[UpstreamJwk],
    door: &DoorRuntime,
    expected_nonce: Option<&str>,
    now: i64,
) -> Result<serde_json::Value, String> {
    // Select the verification key by the header kid; fall back to
    // trying every key when the token carries no kid.
    let header: serde_json::Value = {
        let parts: Vec<&str> = id_token.split('.').collect();
        if parts.len() != 3 {
            return Err("upstream id token is not a compact JWS".to_string());
        }
        serde_json::from_slice(&b64url_decode(parts[0])?).map_err(|_| "bad jwt header")?
    };
    let kid = header["kid"].as_str();
    let candidates: Vec<&UpstreamJwk> = match kid {
        Some(k) => jwks
            .iter()
            .filter(|j| j.kid.as_deref() == Some(k))
            .collect(),
        None => jwks.iter().collect(),
    };
    if candidates.is_empty() {
        return Err("upstream id token kid not found in JWKS".to_string());
    }
    let mut claims = None;
    for jwk in candidates {
        if let Ok((_, c)) = verify_rs256(id_token, &jwk.pubkey) {
            claims = Some(c);
            break;
        }
    }
    let claims = claims.ok_or("upstream id token signature verification failed")?;

    let expected_iss = door.config.issuer.as_deref().unwrap_or("");
    if claims["iss"].as_str() != Some(expected_iss) {
        return Err("upstream id token issuer mismatch".to_string());
    }
    let aud_ok = match &claims["aud"] {
        serde_json::Value::String(s) => s == &door.config.client_id,
        serde_json::Value::Array(a) => a
            .iter()
            .any(|v| v.as_str() == Some(door.config.client_id.as_str())),
        _ => false,
    };
    if !aud_ok {
        return Err("upstream id token audience mismatch".to_string());
    }
    if claims["exp"].as_i64().unwrap_or(0) <= now {
        return Err("upstream id token expired".to_string());
    }
    if let Some(nonce) = expected_nonce {
        if claims["nonce"].as_str() != Some(nonce) {
            return Err("upstream id token nonce mismatch".to_string());
        }
    }
    Ok(claims)
}

/// The subject this door records for a verified upstream identity.
/// OIDC doors use the configured subject claim (`sub` by default;
/// Microsoft records `oid`, falling back to `sub` when absent).
/// GitHub uses the numeric user id from the user document.
pub fn subject_from_claims(door: &DoorRuntime, claims: &serde_json::Value) -> Option<String> {
    if door.config.kind == "github" {
        return claims["id"]
            .as_i64()
            .map(|n| n.to_string())
            .or_else(|| claims["id"].as_str().map(|s| s.to_string()));
    }
    let primary = door.config.subject_claim.as_deref().unwrap_or("sub");
    claims[primary]
        .as_str()
        .map(|s| s.to_string())
        .or_else(|| claims["sub"].as_str().map(|s| s.to_string()))
}

/// Fetch the GitHub user document for an upstream access token;
/// the subject is its numeric `id`.
pub async fn fetch_github_user(
    http: &reqwest::Client,
    door: &DoorRuntime,
    access_token: &str,
) -> Result<serde_json::Value, String> {
    let url = door
        .config
        .userinfo_url
        .as_deref()
        .unwrap_or("https://api.github.com/user");
    let doc: serde_json::Value = http
        .get(url)
        .header(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {access_token}"),
        )
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header(reqwest::header::USER_AGENT, "devtoday-idp-provider")
        .send()
        .await
        .map_err(|_| "upstream user fetch failed".to_string())?
        .error_for_status()
        .map_err(|_| "upstream user fetch failed".to_string())?
        .json()
        .await
        .map_err(|_| "upstream user fetch failed".to_string())?;
    Ok(doc)
}
