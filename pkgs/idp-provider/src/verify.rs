//! Relying-party side of the provider: verify access tokens
//! against a JWKS and gate on roles (#13416).
//!
//! This is the consumer half of the per-app integration pattern
//! documented in the crate README. A consuming application:
//!
//! 1. builds a [`TokenVerifier`] from the issuer (discovery →
//!    JWKS), or from a JWKS it already holds;
//! 2. calls [`TokenVerifier::verify_access_token`] on every
//!    request's bearer token — the verifier selects the JWKS key
//!    by the token's `kid`, so a provider key rollover (Slice 5)
//!    needs no consumer change and no flag day;
//! 3. gates writes server-side with [`require_any_role`], reading
//!    roles only from the verified token — never from request
//!    bodies, headers, or client-side state.
//!
//! [`reference_app`] is the smallest complete consumer: a router
//! with a read endpoint any valid token may use and a write
//! endpoint gated on the `owner`/`orchestrator` roles. The
//! `role-gated-app` example serves it; other apps copy the shape.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD as B64URL};
use rsa::pkcs1v15::{Signature, VerifyingKey};
use rsa::signature::Verifier;
use rsa::{BigUint, RsaPublicKey};
use serde_json::{Value, json};
use sha2::Sha256;
use std::sync::Arc;

use crate::Jwks;
use crate::registry::now_unix;

/// A verified access token: the claims an app may act on.
#[derive(Debug, Clone)]
pub struct VerifiedToken {
    pub sub: String,
    pub roles: Vec<String>,
    pub claims: Value,
}

/// Verifies this provider's access tokens against a JWKS.
pub struct TokenVerifier {
    issuer: String,
    jwks: Jwks,
}

fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
    B64URL
        .decode(s)
        .map_err(|e| format!("base64url decode failed: {e}"))
}

impl TokenVerifier {
    pub fn new(issuer: impl Into<String>, jwks: Jwks) -> Self {
        TokenVerifier {
            issuer: issuer.into(),
            jwks,
        }
    }

    /// Build a verifier from an issuer base URL: fetch the
    /// discovery document, then the JWKS it names. (A long-lived
    /// consumer re-fetches on a schedule so a provider key
    /// rollover is picked up without a restart; verification
    /// itself is per-token and offline.)
    pub async fn from_issuer(issuer: &str) -> Result<Self, String> {
        let issuer = issuer.trim_end_matches('/');
        let client = reqwest::Client::new();
        let discovery: Value = client
            .get(format!("{issuer}/.well-known/openid-configuration"))
            .send()
            .await
            .map_err(|e| format!("discovery fetch failed: {e}"))?
            .error_for_status()
            .map_err(|e| format!("discovery fetch failed: {e}"))?
            .json()
            .await
            .map_err(|e| format!("discovery parse failed: {e}"))?;
        let jwks_uri = discovery["jwks_uri"]
            .as_str()
            .ok_or("discovery document has no jwks_uri")?;
        let jwks: Jwks = client
            .get(jwks_uri)
            .send()
            .await
            .map_err(|e| format!("jwks fetch failed: {e}"))?
            .error_for_status()
            .map_err(|e| format!("jwks fetch failed: {e}"))?
            .json()
            .await
            .map_err(|e| format!("jwks parse failed: {e}"))?;
        Ok(TokenVerifier::new(issuer.to_string(), jwks))
    }

    /// Verify an access token: RS256 signature against the JWKS
    /// key named by the token's `kid`, then issuer, `token_use`,
    /// and expiry. Any failure is an error string the caller maps
    /// to 401 — the checks are ordered so a forged token fails at
    /// the signature, before any claim is read.
    pub fn verify_access_token(&self, token: &str) -> Result<VerifiedToken, String> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err("token is not a compact JWS".to_string());
        }
        let header: Value = serde_json::from_slice(&b64_decode(parts[0])?)
            .map_err(|e| format!("jwt header parse failed: {e}"))?;
        if header["alg"] != "RS256" {
            return Err("unexpected jwt alg".to_string());
        }
        let kid = header["kid"].as_str().ok_or("jwt header has no kid")?;
        let jwk = self
            .jwks
            .keys
            .iter()
            .find(|k| k.kid == kid)
            .ok_or("unexpected jwt kid")?;
        let pubkey = RsaPublicKey::new(
            BigUint::from_bytes_be(&b64_decode(&jwk.n)?),
            BigUint::from_bytes_be(&b64_decode(&jwk.e)?),
        )
        .map_err(|e| format!("jwks key is not a valid RSA public key: {e}"))?;
        let signature = Signature::try_from(b64_decode(parts[2])?.as_slice())
            .map_err(|_| "bad signature encoding".to_string())?;
        VerifyingKey::<Sha256>::new(pubkey)
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
            .map_err(|_| "jwt signature verification failed".to_string())?;
        let claims: Value = serde_json::from_slice(&b64_decode(parts[1])?)
            .map_err(|e| format!("jwt claims parse failed: {e}"))?;
        if claims["iss"].as_str() != Some(self.issuer.as_str()) {
            return Err("unexpected jwt issuer".to_string());
        }
        if claims["token_use"].as_str() != Some("access") {
            return Err("not an access token".to_string());
        }
        let exp = claims["exp"].as_i64().ok_or("jwt has no exp")?;
        if exp <= now_unix() {
            return Err("jwt expired".to_string());
        }
        let sub = claims["sub"].as_str().ok_or("jwt has no sub")?.to_string();
        let roles = claims["roles"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|r| r.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        Ok(VerifiedToken { sub, roles, claims })
    }
}

/// The write gate: the verified token must carry at least one of
/// `roles`. This is the whole of role enforcement — it runs on
/// the server, on every write, against the verified token.
pub fn require_any_role(token: &VerifiedToken, roles: &[&str]) -> Result<(), String> {
    if token.roles.iter().any(|r| roles.contains(&r.as_str())) {
        Ok(())
    } else {
        Err(format!("requires one of the roles: {}", roles.join(", ")))
    }
}

/// Roles allowed to write in the reference consumer.
pub const WRITE_ROLES: [&str; 2] = ["owner", "orchestrator"];

fn bearer(headers: &HeaderMap) -> Result<&str, Response> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "missing bearer token"})),
            )
                .into_response()
        })
}

async fn read_handler(State(verifier): State<Arc<TokenVerifier>>, headers: HeaderMap) -> Response {
    let token = match bearer(&headers) {
        Ok(t) => t,
        Err(r) => return r,
    };
    match verifier.verify_access_token(token) {
        Ok(v) => (
            StatusCode::OK,
            Json(json!({"sub": v.sub, "roles": v.roles})),
        )
            .into_response(),
        Err(e) => (StatusCode::UNAUTHORIZED, Json(json!({"error": e}))).into_response(),
    }
}

async fn write_handler(State(verifier): State<Arc<TokenVerifier>>, headers: HeaderMap) -> Response {
    let token = match bearer(&headers) {
        Ok(t) => t,
        Err(r) => return r,
    };
    let verified = match verifier.verify_access_token(token) {
        Ok(v) => v,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(json!({"error": e}))).into_response(),
    };
    match require_any_role(&verified, &WRITE_ROLES) {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({"written": true, "sub": verified.sub})),
        )
            .into_response(),
        Err(e) => (StatusCode::FORBIDDEN, Json(json!({"error": e}))).into_response(),
    }
}

/// The reference consumer application: `/healthz`, `/read` (any
/// valid access token), `/write` (valid token + a write role).
pub fn reference_app(verifier: Arc<TokenVerifier>) -> Router {
    Router::new()
        .route("/healthz", get(|| async { Json(json!({"status": "ok"})) }))
        .route("/read", get(read_handler))
        .route("/write", post(write_handler))
        .with_state(verifier)
}
