//! idp-provider — minimal OIDC provider (Slice 3 of #13658, implements
//! #13409). Step 1 served OIDC discovery and a JWKS built at startup
//! from the REAL Slice 2 signing certificate published by
//! pkgs/idp-pki (serial 1001). Step 2 adds the rest of the plan's
//! surface — `GET/POST /authorize`, `POST /token`, `GET /userinfo` —
//! completing the authorization-code + PKCE (S256) flow, signed
//! RS256 with the Slice 2 OIDC private key.
//!
//! The federated doors (#13411 GitHub, #13412 Google, #13414
//! Microsoft) hang off `/door/{name}/start|callback`; link state
//! lives in the runtime identity store (#13415), seeded from the
//! provider config. The config account's declared identities link
//! to that account; any other allowlisted identity enrolls at
//! first login into its own read-only account (#15249). Signing keys
//! are a set (Slice 5 rollover): the JWKS can publish several
//! certificates at once, tokens are signed with the configured
//! active kid, and verification accepts every published kid.
//!
//! The JWKS `kid` is the certificate's SHA-256 thumbprint in
//! base64url form — the `x5t#S256` convention recorded on #13686 — so
//! relying parties can pin either form. The private key is loaded
//! from its on-host path under `IDP_CA_HOME` and decrypted in memory
//! only (passphrase via the process environment, fetched by the
//! launcher from the vault by name); it is never copied into the
//! repo or any config file, and there is no provider-local key
//! generation, ever.
//!
//! Standing constraints: no pkgs/id keypair custody in the login
//! path; SSO never rebinds an id key (#2060).

pub mod audit;
pub mod doors;
pub mod flow;
pub mod identities;
pub mod registry;
pub mod signing;
pub mod upstream;
pub mod verify;

use audit::AuditLog;
use axum::{routing::get, Json, Router};
use base64::{engine::general_purpose::STANDARD as B64_STD, Engine as _};
use der::Decode;
use flow::FlowState;
use identities::IdentityStore;
use pkcs1::DecodeRsaPublicKey;
use rsa::{traits::PublicKeyParts, RsaPublicKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;
use upstream::DoorRuntime;
use x509_cert::Certificate;

pub const DEFAULT_ISSUER: &str = "http://127.0.0.1:8911";
pub const DEFAULT_BIND: &str = "127.0.0.1:8911";

#[derive(Clone, Debug, Default)]
pub struct Config {
    pub issuer: String,
    pub bind: String,
    pub oidc_cert_path: PathBuf,
    /// Path to the provider registry config JSON (clients, account
    /// row, allowlist, credential hash). When unset, the provider
    /// runs public-only (discovery + JWKS) and the flow endpoints
    /// refuse service.
    pub provider_config_path: Option<PathBuf>,
    /// Path to the passphrase-encrypted Slice 2 OIDC private key
    /// (on-host, under `IDP_CA_HOME`). Required when a provider
    /// registry config is set.
    pub signing_key_path: Option<PathBuf>,
    /// Full certificate list for the JWKS, in order (Slice 5
    /// rollover: more than one during the overlap). When empty,
    /// `oidc_cert_path` alone is used.
    pub oidc_cert_paths: Vec<PathBuf>,
    /// Full signing-key list, parallel to `oidc_cert_paths`. When
    /// empty, `signing_key_path` alone is used.
    pub signing_key_paths: Vec<PathBuf>,
    /// The `kid` new tokens are signed with, when the key set
    /// holds several. Default: the first certificate's kid.
    pub oidc_active_kid: Option<String>,
    /// Audit DB path override (else the registry config's
    /// `audit_db_path`, else in-memory).
    pub audit_db_path: Option<PathBuf>,
    /// Identity store DB path override (else the registry config's
    /// `identity_db_path`, else in-memory).
    pub identity_db_path: Option<PathBuf>,
}

impl Config {
    pub fn from_env() -> Self {
        let issuer =
            std::env::var("IDP_PROVIDER_ISSUER").unwrap_or_else(|_| DEFAULT_ISSUER.to_string());
        let bind = std::env::var("IDP_PROVIDER_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_string());
        let oidc_cert_path = std::env::var("IDP_PROVIDER_OIDC_CERT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_cert_path());
        let provider_config_path = std::env::var("IDP_PROVIDER_CONFIG").map(PathBuf::from).ok();
        let signing_key_path = std::env::var("IDP_PROVIDER_OIDC_KEY")
            .map(PathBuf::from)
            .ok();
        let audit_db_path = std::env::var("IDP_PROVIDER_AUDIT_DB")
            .map(PathBuf::from)
            .ok();
        let identity_db_path = std::env::var("IDP_PROVIDER_IDENTITY_DB")
            .map(PathBuf::from)
            .ok();
        let split_paths = |name: &str| -> Vec<PathBuf> {
            std::env::var(name)
                .map(|v| {
                    v.split(',')
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .map(PathBuf::from)
                        .collect()
                })
                .unwrap_or_default()
        };
        let oidc_cert_paths = split_paths("IDP_PROVIDER_OIDC_CERTS");
        let signing_key_paths = split_paths("IDP_PROVIDER_OIDC_KEYS");
        let oidc_active_kid = std::env::var("IDP_PROVIDER_OIDC_ACTIVE_KID").ok();
        Config {
            issuer: issuer.trim_end_matches('/').to_string(),
            bind,
            oidc_cert_path,
            provider_config_path,
            signing_key_path,
            audit_db_path,
            identity_db_path,
            oidc_cert_paths,
            signing_key_paths,
            oidc_active_kid,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Jwks {
    pub keys: Vec<Jwk>,
}

/// Build one JWK by parsing a signing certificate PEM: extract
/// the RSA public key and set kid = x5t#S256 = the SHA-256
/// thumbprint of the certificate DER.
pub fn jwk_from_cert_pem(pem: &str) -> Result<Jwk, String> {
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
    Ok(Jwk {
        kty: "RSA".to_string(),
        use_: "sig".to_string(),
        alg: "RS256".to_string(),
        kid: thumbprint.clone(),
        x5t_s256: thumbprint,
        n: b64url(&pubkey.n().to_bytes_be()),
        e: b64url(&pubkey.e().to_bytes_be()),
    })
}

/// Build the JWKS from one signing certificate PEM.
pub fn jwks_from_cert_pem(pem: &str) -> Result<Jwks, String> {
    Ok(Jwks {
        keys: vec![jwk_from_cert_pem(pem)?],
    })
}

/// Build the JWKS from several signing certificate PEMs — the
/// rollover overlap shape (Slice 5): every key is published, and
/// relying parties pick by `kid`.
pub fn jwks_from_cert_pems(pems: &[&str]) -> Result<Jwks, String> {
    let mut keys = Vec::new();
    for pem in pems {
        keys.push(jwk_from_cert_pem(pem)?);
    }
    Ok(Jwks { keys })
}

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub jwks: Jwks,
    /// The authorize/token/userinfo flow state; `None` in
    /// public-only mode (no registry config supplied).
    pub flow: Option<Arc<FlowState>>,
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
    build_app_with_flow(config, jwks, None)
}

pub fn build_app_with_flow(config: Config, jwks: Jwks, flow: Option<Arc<FlowState>>) -> Router {
    let state = AppState { config, jwks, flow };
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
        .route(
            "/authorize",
            get(flow::authorize_get).post(flow::authorize_post),
        )
        .route("/token", axum::routing::post(flow::token))
        .route("/userinfo", get(flow::userinfo))
        .route("/door/{name}/start", get(doors::door_start))
        .route("/door/{name}/callback", get(doors::door_callback))
        .with_state(state)
}

/// Load the configured cert, build the JWKS, assemble the flow state
/// when a registry config is configured, and return the app.
///
/// Fail-closed rules: when a registry config IS configured, the
/// signing key path and the `IDP_CA_PASSPHRASE` environment value
/// (in process memory only) are required, and the loaded key MUST
/// match the published certificate behind the JWKS — any failure is
/// a startup error, never a degraded provider.
pub fn app_from_config(config: Config) -> Result<Router, String> {
    // The served key set: the configured certificate list, or the
    // single certificate path when no list is configured.
    let cert_paths: Vec<PathBuf> = if config.oidc_cert_paths.is_empty() {
        vec![config.oidc_cert_path.clone()]
    } else {
        config.oidc_cert_paths.clone()
    };
    let mut pems: Vec<String> = Vec::new();
    for path in &cert_paths {
        pems.push(std::fs::read_to_string(path).map_err(|e| {
            format!(
                "failed to read OIDC signing cert at {}: {e}",
                path.display()
            )
        })?);
    }
    let pem_refs: Vec<&str> = pems.iter().map(|s| s.as_str()).collect();
    let jwks = jwks_from_cert_pems(&pem_refs)?;

    let flow = match &config.provider_config_path {
        None => None,
        Some(cfg_path) => {
            let registry = registry::ProviderConfig::load(cfg_path)?;
            // The key list is parallel to the certificate list;
            // with no list configured it is the single key path.
            let key_paths: Vec<PathBuf> = if !config.signing_key_paths.is_empty() {
                config.signing_key_paths.clone()
            } else {
                vec![config
                    .signing_key_path
                    .clone()
                    .ok_or("IDP_PROVIDER_OIDC_KEY is required when IDP_PROVIDER_CONFIG is set")?]
            };
            if key_paths.len() != cert_paths.len() {
                return Err(format!(
                    "signing key count ({}) does not match certificate count ({})",
                    key_paths.len(),
                    cert_paths.len()
                ));
            }
            let passphrase = std::env::var("IDP_CA_PASSPHRASE").map_err(|_| {
                "IDP_CA_PASSPHRASE is not set (the launcher fetches it from the vault by name into process memory)"
            })?;
            // Load every key and prove each one matches the
            // published certificate at the same position before
            // serving anything.
            let mut pairs: Vec<(rsa::RsaPrivateKey, String)> = Vec::new();
            for (i, key_path) in key_paths.iter().enumerate() {
                let key = signing::load_signing_key(key_path, &passphrase)?;
                let kid = jwks.keys[i].kid.clone();
                let probe = signing::TokenSigner::new(key.clone(), kid.clone());
                let (n, e) = probe.public_components();
                if b64url(&n) != jwks.keys[i].n || b64url(&e) != jwks.keys[i].e {
                    return Err(format!(
                        "loaded signing key {} does not match its published OIDC signing certificate",
                        key_path.display()
                    ));
                }
                pairs.push((key, kid));
            }
            let active_kid = config
                .oidc_active_kid
                .clone()
                .unwrap_or_else(|| jwks.keys[0].kid.clone());
            let signer = signing::TokenSigner::from_keys(pairs, &active_kid)?;
            let audit_path = config
                .audit_db_path
                .clone()
                .or_else(|| registry.audit_db_path.clone());
            let audit = Arc::new(AuditLog::open(audit_path.as_deref())?);
            let identity_path = config
                .identity_db_path
                .clone()
                .or_else(|| registry.identity_db_path.clone());
            let identities = Arc::new(IdentityStore::open(identity_path.as_deref())?);
            // Resolve each door's client secret from the
            // environment variable its config names (the launcher
            // fills it from the vault by name). A door without its
            // secret — or an OIDC door without issuer/JWKS — stays
            // disabled: its start endpoint denies with an audit
            // record, and the rest of the provider is unaffected.
            let doors: Vec<DoorRuntime> = registry
                .doors
                .iter()
                .map(|d| {
                    let secret = std::env::var(&d.client_secret_env)
                        .ok()
                        .filter(|s| !s.is_empty());
                    let rt = DoorRuntime {
                        config: d.clone(),
                        client_secret: secret,
                    };
                    if !rt.enabled() {
                        eprintln!(
                            "idp-provider: door {} disabled (secret env {} unset or OIDC issuer/JWKS unconfigured)",
                            d.name, d.client_secret_env
                        );
                    }
                    rt
                })
                .collect();
            Some(Arc::new(FlowState::new(
                registry,
                signer,
                audit,
                identities,
                doors,
                config.issuer.clone(),
            )))
        }
    };
    Ok(build_app_with_flow(config, jwks, flow))
}
