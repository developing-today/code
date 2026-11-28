//! Flow tests for Slice 3 step 2: the full authorization-code +
//! PKCE loop against an in-process provider whose registry holds
//! one owner account, the deny-by-default allowlist, and the static
//! client registry — plus the denial/audit behaviour.
//!
//! The signing key here is a throwaway RSA key generated in-test
//! (never the Slice 2 key — that key never leaves its on-host path;
//! the live proof against the real key is step 3's laptop run).
//! The JWKS served in these tests is built from the same throwaway
//! key, so the ID-token verification below is a genuine
//! verify-against-JWKS check of the crate's signing path.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD as B64URL};
use idp_provider::audit::AuditLog;
use idp_provider::flow::FlowState;
use idp_provider::registry::{Account, Client, Identity, ProviderConfig};
use idp_provider::signing::TokenSigner;
use idp_provider::{Config, Jwk, Jwks, b64url, build_app_with_flow};
use rand_core::OsRng;
use rsa::pkcs1v15::{Signature, VerifyingKey};
use rsa::signature::Verifier;
use rsa::traits::PublicKeyParts;
use rsa::{RsaPrivateKey, RsaPublicKey};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

const ISSUER: &str = "http://127.0.0.1:8911";
const CLIENT_ID: &str = "test-rp";
const REDIRECT_URI: &str = "http://127.0.0.1:8912/callback";
const OWNER_CREDENTIAL: &str = "test-owner-credential";
const ROLES: [&str; 4] = ["owner", "orchestrator", "agent", "read-only"];

struct Harness {
    app: axum::Router,
    audit: Arc<AuditLog>,
    pubkey: RsaPublicKey,
    kid: String,
}

fn harness() -> Harness {
    let mut rng = OsRng;
    let key = RsaPrivateKey::new(&mut rng, 2048).expect("generate throwaway key");
    let pubkey = RsaPublicKey::from(&key);
    let kid = "test-kid".to_string();
    let jwks = Jwks {
        keys: vec![Jwk {
            kty: "RSA".to_string(),
            use_: "sig".to_string(),
            alg: "RS256".to_string(),
            kid: kid.clone(),
            x5t_s256: kid.clone(),
            n: b64url(&pubkey.n().to_bytes_be()),
            e: b64url(&pubkey.e().to_bytes_be()),
        }],
    };
    // Fast hash for tests: same format, low iteration count.
    let credential_hash = {
        let salt = b"0123456789abcdef";
        let hash = idp_provider::registry::pbkdf2_sha256(OWNER_CREDENTIAL.as_bytes(), salt, 1000);
        format!("pbkdf2-sha256$1000${}${}", b64url(salt), b64url(&hash))
    };
    let registry = ProviderConfig {
        saml_sps: vec![],
        account: Account {
            account_id: "drew".to_string(),
            display_name: "Drew".to_string(),
            roles: ROLES.iter().map(|s| s.to_string()).collect(),
            identities: vec![Identity {
                provider: "devtoday-local".to_string(),
                subject: "drew".to_string(),
            }],
            local_credential_hash: credential_hash,
        },
        allowlist: vec!["devtoday-local:drew".to_string()],
        clients: vec![
            Client {
                client_id: CLIENT_ID.to_string(),
                name: "Test RP".to_string(),
                redirect_uris: vec![REDIRECT_URI.to_string()],
            },
            Client {
                client_id: "devtoday-local-door".to_string(),
                name: "devtoday local door".to_string(),
                redirect_uris: vec!["http://127.0.0.1:8913/callback".to_string()],
            },
        ],
        doors: vec![],
        audit_db_path: None,
        identity_db_path: None,
    };
    let audit = Arc::new(AuditLog::open(None).expect("in-memory audit"));
    let identities = Arc::new(
        idp_provider::identities::IdentityStore::open(None).expect("in-memory identity store"),
    );
    let flow = Arc::new(FlowState::new(
        registry,
        TokenSigner::new(key, kid.clone()),
        audit.clone(),
        identities,
        vec![],
        ISSUER.to_string(),
    ));
    let config = Config {
        issuer: ISSUER.to_string(),
        bind: "127.0.0.1:0".to_string(),
        oidc_cert_path: PathBuf::new(),
        provider_config_path: None,
        signing_key_path: None,
        audit_db_path: None,
        identity_db_path: None,
        oidc_cert_paths: vec![],
        signing_key_paths: vec![],
        oidc_active_kid: None,
    };
    Harness {
        app: build_app_with_flow(config, jwks, Some(flow)),
        audit,
        pubkey,
        kid,
    }
}

fn pkce() -> (String, String) {
    let verifier = "test-verifier-0123456789-abcdefghijklmnopqrstuvwxyz";
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    (verifier.to_string(), challenge)
}

fn form_body(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

async fn send(
    app: axum::Router,
    req: Request<Body>,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    (status, headers, bytes)
}

fn authorize_form(login_id: &str, credential: &str, challenge: &str) -> String {
    form_body(&[
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", REDIRECT_URI),
        ("scope", "openid profile"),
        ("state", "state-123"),
        ("nonce", "nonce-456"),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("login_id", login_id),
        ("credential", credential),
    ])
}

async fn post_form(
    app: axum::Router,
    uri: &str,
    body: String,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    send(
        app,
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap(),
    )
    .await
}

/// Run the owner's successful authorize step and return the code.
async fn owner_code(h: &Harness, challenge: &str) -> String {
    let (status, headers, _body) = post_form(
        h.app.clone(),
        "/authorize",
        authorize_form("drew", OWNER_CREDENTIAL, challenge),
    )
    .await;
    assert_eq!(status, StatusCode::FOUND);
    let location = headers[header::LOCATION].to_str().unwrap().to_string();
    assert!(location.starts_with(&format!("{REDIRECT_URI}?code=")));
    assert!(location.ends_with("&state=state-123"));
    location
        .split("code=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .to_string()
}

fn verify_id_token(h: &Harness, token: &str) -> serde_json::Value {
    let parts: Vec<&str> = token.split('.').collect();
    assert_eq!(parts.len(), 3);
    let header_json: serde_json::Value =
        serde_json::from_slice(&B64URL.decode(parts[0]).unwrap()).unwrap();
    assert_eq!(header_json["alg"], "RS256");
    assert_eq!(header_json["kid"], h.kid);
    let sig_bytes = B64URL.decode(parts[2]).unwrap();
    let signature = Signature::try_from(sig_bytes.as_slice()).unwrap();
    let verifying = VerifyingKey::<Sha256>::new(h.pubkey.clone());
    verifying
        .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
        .expect("ID token signature verifies against the JWKS key");
    serde_json::from_slice(&B64URL.decode(parts[1]).unwrap()).unwrap()
}

#[tokio::test]
async fn full_code_pkce_loop_with_role_claims_and_userinfo() {
    let h = harness();
    let (verifier, challenge) = pkce();
    let code = owner_code(&h, &challenge).await;

    let token_body = form_body(&[
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("redirect_uri", REDIRECT_URI),
        ("client_id", CLIENT_ID),
        ("code_verifier", &verifier),
    ]);
    let (status, _headers, bytes) = post_form(h.app.clone(), "/token", token_body).await;
    assert_eq!(status, StatusCode::OK);
    let tokens: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(tokens["token_type"], "Bearer");
    let id_token = tokens["id_token"].as_str().unwrap();
    let access_token = tokens["access_token"].as_str().unwrap();

    let claims = verify_id_token(&h, id_token);
    assert_eq!(claims["iss"], ISSUER);
    assert_eq!(claims["sub"], "drew");
    assert_eq!(claims["aud"], CLIENT_ID);
    assert_eq!(claims["nonce"], "nonce-456");
    assert_eq!(
        claims["roles"],
        serde_json::json!(["owner", "orchestrator", "agent", "read-only"])
    );
    assert!(claims["exp"].as_i64().unwrap() > claims["iat"].as_i64().unwrap());

    // userinfo with the access token.
    let (status, _headers, bytes) = send(
        h.app.clone(),
        Request::builder()
            .uri("/userinfo")
            .header(header::AUTHORIZATION, format!("Bearer {access_token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let info: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(info["sub"], "drew");
    assert_eq!(
        info["roles"],
        serde_json::json!(["owner", "orchestrator", "agent", "read-only"])
    );

    // The ID token is NOT an access token at userinfo.
    let (status, _h, _b) = send(
        h.app.clone(),
        Request::builder()
            .uri("/userinfo")
            .header(header::AUTHORIZATION, format!("Bearer {id_token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Audit: an allowed authorize and an allowed token exchange.
    let entries = h.audit.entries().unwrap();
    assert!(entries.iter().any(|e| e.endpoint == "authorize"
        && e.outcome == "allowed"
        && e.identity == "devtoday-local:drew"));
    assert!(
        entries
            .iter()
            .any(|e| e.endpoint == "token" && e.outcome == "allowed")
    );
}

#[tokio::test]
async fn non_allowlisted_attempt_is_denied_and_audited() {
    let h = harness();
    let (_v, challenge) = pkce();
    let (status, _headers, _bytes) = post_form(
        h.app.clone(),
        "/authorize",
        authorize_form("mallory", "whatever", &challenge),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let entries = h.audit.entries().unwrap();
    assert!(entries.iter().any(|e| e.endpoint == "authorize"
        && e.outcome == "denied"
        && e.reason == "not_allowlisted"
        && e.identity == "devtoday-local:mallory"));
}

#[tokio::test]
async fn bad_credential_is_denied_and_audited() {
    let h = harness();
    let (_v, challenge) = pkce();
    let (status, _headers, _bytes) = post_form(
        h.app.clone(),
        "/authorize",
        authorize_form("drew", "wrong-credential", &challenge),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let entries = h.audit.entries().unwrap();
    assert!(entries.iter().any(|e| e.endpoint == "authorize"
        && e.outcome == "denied"
        && e.reason == "bad_credential"));
}

#[tokio::test]
async fn pkce_failure_and_code_reuse_are_denied_and_audited() {
    let h = harness();
    let (_verifier, challenge) = pkce();
    let code = owner_code(&h, &challenge).await;

    // Wrong verifier.
    let bad = form_body(&[
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("redirect_uri", REDIRECT_URI),
        ("client_id", CLIENT_ID),
        ("code_verifier", "the-wrong-verifier"),
    ]);
    let (status, _hh, _bb) = post_form(h.app.clone(), "/token", bad.clone()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The same code again (single-use: consumed by the first attempt).
    let (status, _hh, _bb) = post_form(h.app.clone(), "/token", bad).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let entries = h.audit.entries().unwrap();
    assert!(entries.iter().any(|e| e.endpoint == "token"
        && e.outcome == "denied"
        && e.reason == "pkce_verification_failed"));
    assert!(entries.iter().any(|e| e.endpoint == "token"
        && e.outcome == "denied"
        && e.reason == "unknown_or_expired_code"));
}

#[tokio::test]
async fn authorize_get_serves_login_form_and_rejects_bad_client() {
    let h = harness();
    let (_v, challenge) = pkce();
    let uri = format!(
        "/authorize?response_type=code&client_id={CLIENT_ID}&redirect_uri={}&scope=openid&code_challenge={challenge}&code_challenge_method=S256",
        url_encode(REDIRECT_URI)
    );
    let (status, _hh, bytes) = send(
        h.app.clone(),
        Request::builder().uri(uri).body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let html = String::from_utf8(bytes).unwrap();
    assert!(html.contains("<form method=\"post\" action=\"/authorize\">"));
    assert!(html.contains("name=\"credential\""));

    // Unknown client: denied, audited, no redirect.
    let (status, _hh, _bb) = post_form(
        h.app.clone(),
        "/authorize",
        form_body(&[
            ("response_type", "code"),
            ("client_id", "no-such-client"),
            ("redirect_uri", REDIRECT_URI),
            ("scope", "openid"),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("login_id", "drew"),
            ("credential", OWNER_CREDENTIAL),
        ]),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let entries = h.audit.entries().unwrap();
    assert!(entries.iter().any(|e| e.endpoint == "authorize"
        && e.outcome == "denied"
        && e.reason == "unknown_client"));
}
