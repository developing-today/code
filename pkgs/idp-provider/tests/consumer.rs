//! Consumer tests (#13416): the reference consumer verifies the
//! provider's access tokens against the JWKS and enforces roles
//! on writes, server-side.
//!
//! The owner token comes through the provider's real
//! authorize/token loop; the read-only and expired tokens are
//! minted directly with the provider's signer (the live
//! provider's single account holds every role, so a low-role
//! token can only be produced at this level — the denial it
//! proves is a property of the consumer's gate, not of any one
//! account).

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use idp_provider::audit::AuditLog;
use idp_provider::flow::FlowState;
use idp_provider::identities::IdentityStore;
use idp_provider::registry::{Account, Client, Identity, ProviderConfig, now_unix, url_encode};
use idp_provider::signing::TokenSigner;
use idp_provider::verify::{TokenVerifier, reference_app};
use idp_provider::{Config, Jwk, Jwks, b64url, build_app_with_flow};
use rand_core::OsRng;
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
const KID: &str = "test-kid";

struct Harness {
    provider: axum::Router,
    consumer: axum::Router,
    signer: TokenSigner,
}

fn jwks_for(pubkey: &RsaPublicKey) -> Jwks {
    Jwks {
        keys: vec![Jwk {
            kty: "RSA".to_string(),
            use_: "sig".to_string(),
            alg: "RS256".to_string(),
            kid: KID.to_string(),
            x5t_s256: KID.to_string(),
            n: b64url(&pubkey.n().to_bytes_be()),
            e: b64url(&pubkey.e().to_bytes_be()),
        }],
    }
}

fn harness() -> Harness {
    let mut rng = OsRng;
    let key = RsaPrivateKey::new(&mut rng, 2048).expect("generate throwaway key");
    let pubkey = RsaPublicKey::from(&key);
    let credential_hash = {
        let salt = b"0123456789abcdef";
        let hash = idp_provider::registry::pbkdf2_sha256(OWNER_CREDENTIAL.as_bytes(), salt, 1000);
        format!("pbkdf2-sha256$1000${}${}", b64url(salt), b64url(&hash))
    };
    let registry = ProviderConfig {
        account: Account {
            account_id: "drew".to_string(),
            display_name: "Drew".to_string(),
            roles: ["owner", "orchestrator", "agent", "read-only"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            identities: vec![Identity {
                provider: "devtoday-local".to_string(),
                subject: "drew".to_string(),
            }],
            local_credential_hash: credential_hash,
        },
        allowlist: vec!["devtoday-local:drew".to_string()],
        clients: vec![Client {
            client_id: CLIENT_ID.to_string(),
            name: "Test RP".to_string(),
            redirect_uris: vec![REDIRECT_URI.to_string()],
        }],
        doors: vec![],
        audit_db_path: None,
        identity_db_path: None,
    };
    let audit = Arc::new(AuditLog::open(None).expect("audit"));
    let identities = Arc::new(IdentityStore::open(None).expect("identity store"));
    let signer = TokenSigner::new(key.clone(), KID.to_string());
    let flow = Arc::new(FlowState::new(
        registry,
        TokenSigner::new(key, KID.to_string()),
        audit,
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
    let provider = build_app_with_flow(config, jwks_for(&pubkey), Some(flow));
    let verifier = Arc::new(TokenVerifier::new(ISSUER, jwks_for(&pubkey)));
    let consumer = reference_app(verifier);
    Harness {
        provider,
        consumer,
        signer,
    }
}

fn form_body(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Run the provider's real loop and return (id_token, access_token).
async fn owner_tokens(provider: &axum::Router) -> (String, String) {
    let verifier = "consumer-test-verifier-0123456789-abcdefghij";
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    let resp = provider
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/authorize")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(form_body(&[
                    ("response_type", "code"),
                    ("client_id", CLIENT_ID),
                    ("redirect_uri", REDIRECT_URI),
                    ("scope", "openid"),
                    ("state", "consumer-test"),
                    ("nonce", "consumer-test"),
                    ("code_challenge", &challenge),
                    ("code_challenge_method", "S256"),
                    ("login_id", "drew"),
                    ("credential", OWNER_CREDENTIAL),
                ])))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FOUND, "authorize redirects");
    let location = resp
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let code = location
        .split("code=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .to_string();
    let resp = provider
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/token")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(form_body(&[
                    ("grant_type", "authorization_code"),
                    ("code", &code),
                    ("redirect_uri", REDIRECT_URI),
                    ("client_id", CLIENT_ID),
                    ("code_verifier", verifier),
                ])))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "token issued");
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let tokens: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    (
        tokens["id_token"].as_str().unwrap().to_string(),
        tokens["access_token"].as_str().unwrap().to_string(),
    )
}

fn mint(signer: &TokenSigner, roles: &[&str], exp_offset: i64, token_use: &str) -> String {
    let now = now_unix();
    signer
        .sign_jwt(&serde_json::json!({
            "iss": ISSUER,
            "sub": "drew",
            "aud": CLIENT_ID,
            "iat": now,
            "exp": now + exp_offset,
            "scope": "openid",
            "roles": roles,
            "preferred_username": "devtoday-local:drew",
            "token_use": token_use,
        }))
        .expect("mint token")
}

async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let resp = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, body)
}

#[tokio::test]
async fn consumer_verifies_tokens_and_gates_writes_on_roles() {
    let h = harness();
    let (id_token, access_token) = owner_tokens(&h.provider).await;

    // Owner token (all roles, through the real loop): read + write.
    let (status, body) = call(&h.consumer, "GET", "/read", Some(&access_token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["sub"], "drew");
    let (status, body) = call(&h.consumer, "POST", "/write", Some(&access_token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["written"], true);

    // Read-only token: read passes, the write gate refuses.
    let readonly = mint(&h.signer, &["read-only"], 300, "access");
    let (status, _) = call(&h.consumer, "GET", "/read", Some(&readonly)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = call(&h.consumer, "POST", "/write", Some(&readonly)).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "write gate refuses read-only"
    );
    assert!(body["error"].as_str().unwrap().contains("roles"));

    // An ID token is not an access token.
    let (status, _) = call(&h.consumer, "GET", "/read", Some(&id_token)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Expired token, garbage token, missing token: all 401.
    let expired = mint(&h.signer, &["owner"], -60, "access");
    let (status, _) = call(&h.consumer, "POST", "/write", Some(&expired)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = call(&h.consumer, "GET", "/read", Some("not.a.token")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = call(&h.consumer, "GET", "/read", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
