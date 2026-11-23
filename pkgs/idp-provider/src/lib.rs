//! idp-provider — minimal OIDC provider (Slice 3 of #13658, implements
//! #13409). This first step serves OIDC discovery and a JWKS built at
//! startup from the REAL Slice 2 signing certificate published by
//! pkgs/idp-pki (serial 1001). Only the PUBLIC certificate is used
//! here; no private key material is read by this slice.
//!
//! The JWKS `kid` is the certificate's SHA-256 thumbprint in
//! base64url form — the `x5t#S256` convention recorded on #13686 — so
//! relying parties can pin either form.

use axum::{routing::get, Json, Router};
use base64::{engine::general_purpose::STANDARD as B64_STD, Engine as _};
use der::Decode;
use pkcs1::DecodeRsaPublicKey;
use rsa::{traits::PublicKeyParts, RsaPublicKey};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use x509_cert::Certificate;

pub const DEFAULT_ISSUER: &str = "http://127.0.0.1:8911";
pub const DEFAULT_BIND: &str = "127.0.0.1:8911";

#[derive(Clone, Debug)]
pub struct Config {
    pub issuer: String,
    pub bind: String,
    pub oidc_cert_path: PathBuf,
}

impl Config {
    pub fn from_env() -> Self {
        let issuer =
            std::env::var("IDP_PROVIDER_ISSUER").unwrap_or_else(|_| DEFAULT_ISSUER.to_string());
        let bind = std::env::var("IDP_PROVIDER_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_string());
        let oidc_cert_path = std::env::var("IDP_PROVIDER_OIDC_CERT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_cert_path());
        Config {
            issuer: issuer.trim_end_matches('/').to_string(),
            bind,
            oidc_cert_path,
        }
    }
}

/// Default signing-cert location: `pkgs/idp-pki/trust/oidc-signing.crt.pem`
/// relative to the repository checkout. Resolved against the current
/// directory first, then against this crate's manifest directory
/// (i.e. `<repo>/pkgs/idp-provider` -> `../idp-pki/trust/...`).
pub fn default_cert_path() -> PathBuf {
    let rel = PathBuf::from("pkgs/idp-pki/trust/oidc-signing.crt.pem");
    if rel.exists() {
        return rel;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../idp-pki/trust/oidc-signing.crt.pem")
}

pub fn b64url(data: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

/// Decode a PEM certificate to its DER bytes.
pub fn pem_to_der(pem: &str) -> Result<Vec<u8>, String> {
    let body: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("");
    B64_STD
        .decode(body.trim())
        .map_err(|e| format!("PEM base64 decode failed: {e}"))
}

/// base64url(SHA-256(cert DER)) — the x5t#S256 thumbprint.
pub fn cert_thumbprint_sha256(der_bytes: &[u8]) -> String {
    b64url(&Sha256::digest(der_bytes))
}

#[derive(Clone, Debug, Serialize)]
pub struct Jwk {
    pub kty: String,
    #[serde(rename = "use")]
    pub use_: String,
    pub alg: String,
    pub kid: String,
    #[serde(rename = "x5t#S256")]
    pub x5t_s256: String,
    pub n: String,
    pub e: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Jwks {
    pub keys: Vec<Jwk>,
}

/// Build the JWKS by parsing the configured signing certificate PEM:
/// extract the RSA public key and set kid = x5t#S256 = the SHA-256
/// thumbprint of the certificate DER.
pub fn jwks_from_cert_pem(pem: &str) -> Result<Jwks, String> {
    let der_bytes = pem_to_der(pem)?;
    let thumbprint = cert_thumbprint_sha256(&der_bytes);
    let cert = Certificate::from_der(&der_bytes)
        .map_err(|e| format!("certificate DER parse failed: {e}"))?;
    let spki = &cert.tbs_certificate.subject_public_key_info;
    let pk_der = spki
        .subject_public_key
        .as_bytes()
        .ok_or("subject public key bit string is not byte-aligned")?;
    let pubkey = RsaPublicKey::from_pkcs1_der(pk_der)
        .map_err(|e| format!("RSA public key parse failed: {e}"))?;
    let jwk = Jwk {
        kty: "RSA".to_string(),
        use_: "sig".to_string(),
        alg: "RS256".to_string(),
        kid: thumbprint.clone(),
        x5t_s256: thumbprint,
        n: b64url(&pubkey.n().to_bytes_be()),
        e: b64url(&pubkey.e().to_bytes_be()),
    };
    Ok(Jwks { keys: vec![jwk] })
}

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub jwks: Jwks,
}

#[derive(Serialize)]
struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    userinfo_endpoint: String,
    jwks_uri: String,
    response_types_supported: Vec<String>,
    grant_types_supported: Vec<String>,
    code_challenge_methods_supported: Vec<String>,
    subject_types_supported: Vec<String>,
    id_token_signing_alg_values_supported: Vec<String>,
    scopes_supported: Vec<String>,
}

pub fn build_app(config: Config, jwks: Jwks) -> Router {
    let state = AppState { config, jwks };
    Router::new()
        .route(
            "/healthz",
            get(|| async { Json(serde_json::json!({"status": "ok"})) }),
        )
        .route(
            "/.well-known/openid-configuration",
            get(
                |axum::extract::State(st): axum::extract::State<AppState>| async move {
                    let i = &st.config.issuer;
                    Json(Discovery {
                        issuer: i.clone(),
                        authorization_endpoint: format!("{i}/authorize"),
                        token_endpoint: format!("{i}/token"),
                        userinfo_endpoint: format!("{i}/userinfo"),
                        jwks_uri: format!("{i}/jwks"),
                        response_types_supported: vec!["code".to_string()],
                        grant_types_supported: vec!["authorization_code".to_string()],
                        code_challenge_methods_supported: vec!["S256".to_string()],
                        subject_types_supported: vec!["public".to_string()],
                        id_token_signing_alg_values_supported: vec!["RS256".to_string()],
                        scopes_supported: vec!["openid".to_string(), "profile".to_string()],
                    })
                },
            ),
        )
        .route(
            "/jwks",
            get(
                |axum::extract::State(st): axum::extract::State<AppState>| async move {
                    Json(st.jwks.clone())
                },
            ),
        )
        .with_state(state)
}

/// Load the configured cert, build the JWKS, and return the app.
pub fn app_from_config(config: Config) -> Result<Router, String> {
    let pem = std::fs::read_to_string(&config.oidc_cert_path).map_err(|e| {
        format!(
            "failed to read OIDC signing cert at {}: {e}",
            config.oidc_cert_path.display()
        )
    })?;
    let jwks = jwks_from_cert_pem(&pem)?;
    Ok(build_app(config, jwks))
}
