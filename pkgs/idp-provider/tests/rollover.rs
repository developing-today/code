//! Rollover tests (Slice 5, #13658): the signer as a key *set*.
//!
//! - A token signed under the old kid verifies against a signer
//!   holding {old, new} (the overlap), and tokens it mints carry
//!   the new kid.
//! - A signer holding only the new key rejects old-kid tokens
//!   (retirement), and vice versa.
//! - At the flow level, /userinfo accepts a pre-rollover access
//!   token during the overlap and rejects it after the old key is
//!   retired from the set.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use idp_provider::audit::AuditLog;
use idp_provider::flow::FlowState;
use idp_provider::identities::IdentityStore;
use idp_provider::registry::{Account, Client, Identity, ProviderConfig, now_unix};
use idp_provider::signing::TokenSigner;
use idp_provider::{Config, Jwk, Jwks, b64url, build_app_with_flow};
use rand_core::OsRng;
use rsa::traits::PublicKeyParts;
use rsa::{RsaPrivateKey, RsaPublicKey};
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

const ISSUER: &str = "http://127.0.0.1:8911";
const OLD_KID: &str = "old-kid";
const NEW_KID: &str = "new-kid";

fn gen_key() -> RsaPrivateKey {
    let mut rng = OsRng;
    RsaPrivateKey::new(&mut rng, 2048).expect("generate throwaway key")
}

fn jwk_for(key: &RsaPrivateKey, kid: &str) -> Jwk {
    let pubkey = RsaPublicKey::from(key);
    Jwk {
        kty: "RSA".to_string(),
        use_: "sig".to_string(),
        alg: "RS256".to_string(),
        kid: kid.to_string(),
        x5t_s256: kid.to_string(),
        n: b64url(&pubkey.n().to_bytes_be()),
        e: b64url(&pubkey.e().to_bytes_be()),
    }
}

fn claims(sub: &str, token_use: &str) -> serde_json::Value {
    let now = now_unix();
    serde_json::json!({
        "iss": ISSUER,
        "sub": sub,
        "aud": "test-rp",
        "iat": now,
        "exp": now + 300,
        "scope": "openid",
        "roles": ["owner", "orchestrator", "agent", "read-only"],
        "preferred_username": "devtoday-local:drew",
        "token_use": token_use,
    })
}

fn header_kid(token: &str) -> String {
    let part = token.split('.').next().unwrap();
    let bytes =
        base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, part).unwrap();
    let header: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    header["kid"].as_str().unwrap().to_string()
}

#[test]
fn overlap_signer_verifies_both_kids_and_signs_with_active() {
    let old_key = gen_key();
    let new_key = gen_key();
    let old_signer = TokenSigner::new(old_key.clone(), OLD_KID.to_string());
    let pre_rollover = old_signer.sign_jwt(&claims("drew", "id")).unwrap();
    assert_eq!(header_kid(&pre_rollover), OLD_KID);

    let overlap = TokenSigner::from_keys(
        vec![
            (old_key.clone(), OLD_KID.to_string()),
            (new_key.clone(), NEW_KID.to_string()),
        ],
        NEW_KID,
    )
    .unwrap();
    // Pre-rollover token still verifies during the overlap.
    let verified = overlap.verify_jwt(&pre_rollover).unwrap();
    assert_eq!(verified["sub"], "drew");
    // New tokens carry the new kid and verify too.
    let fresh = overlap.sign_jwt(&claims("drew", "id")).unwrap();
    assert_eq!(header_kid(&fresh), NEW_KID);
    assert!(overlap.verify_jwt(&fresh).is_ok());
    // The old-only signer rejects new-kid tokens (it never knew
    // the new key) — the mirror image of retirement.
    assert!(old_signer.verify_jwt(&fresh).is_err());
}

#[test]
fn retired_key_no_longer_verifies() {
    let old_key = gen_key();
    let new_key = gen_key();
    let old_signer = TokenSigner::new(old_key.clone(), OLD_KID.to_string());
    let pre_rollover = old_signer.sign_jwt(&claims("drew", "id")).unwrap();

    let retired = TokenSigner::new(new_key, NEW_KID.to_string());
    let err = retired.verify_jwt(&pre_rollover).unwrap_err();
    assert!(
        err.contains("unexpected jwt kid"),
        "retirement rejects on kid, got: {err}"
    );
}

#[test]
fn from_keys_rejects_bad_sets() {
    let key = gen_key();
    assert!(TokenSigner::from_keys(vec![], NEW_KID).is_err());
    assert!(
        TokenSigner::from_keys(vec![(key.clone(), OLD_KID.to_string())], NEW_KID).is_err(),
        "active kid must be in the set"
    );
    assert!(
        TokenSigner::from_keys(
            vec![
                (key.clone(), OLD_KID.to_string()),
                (key, OLD_KID.to_string())
            ],
            OLD_KID
        )
        .is_err(),
        "duplicate kids rejected"
    );
}

// ---- flow level: userinfo across the rollover ----

fn app_with_signer(signer: TokenSigner, jwks: Jwks) -> axum::Router {
    let registry = ProviderConfig {
        saml_sps: vec![],
        account: Account {
            account_id: "drew".to_string(),
            display_name: "Drew".to_string(),
            email: None,
            roles: ["owner", "orchestrator", "agent", "read-only"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            identities: vec![Identity {
                provider: "devtoday-local".to_string(),
                subject: "drew".to_string(),
            }],
            local_credential_hash: String::new(),
        },
        allowlist: vec!["devtoday-local:drew".to_string()],
        clients: vec![Client {
            client_id: "test-rp".to_string(),
            name: "Test RP".to_string(),
            redirect_uris: vec!["http://127.0.0.1:8912/callback".to_string()],
        }],
        doors: vec![],
        audit_db_path: None,
        identity_db_path: None,
    };
    let audit = Arc::new(AuditLog::open(None).expect("audit"));
    let identities = Arc::new(IdentityStore::open(None).expect("identity store"));
    let flow = Arc::new(FlowState::new(
        registry,
        signer,
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
    build_app_with_flow(config, jwks, Some(flow))
}

async fn userinfo_status(app: &axum::Router, token: &str) -> StatusCode {
    app.clone()
        .oneshot(
            Request::builder()
                .uri("/userinfo")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn userinfo_accepts_pre_rollover_token_during_overlap_only() {
    let old_key = gen_key();
    let new_key = gen_key();
    let old_signer = TokenSigner::new(old_key.clone(), OLD_KID.to_string());
    let access = old_signer
        .sign_jwt(&claims("drew", "access"))
        .expect("mint pre-rollover access token");

    // Overlap: signer holds both keys, signs with the new one.
    let overlap = TokenSigner::from_keys(
        vec![
            (old_key.clone(), OLD_KID.to_string()),
            (new_key.clone(), NEW_KID.to_string()),
        ],
        NEW_KID,
    )
    .unwrap();
    let both_jwks = Jwks {
        keys: vec![jwk_for(&old_key, OLD_KID), jwk_for(&new_key, NEW_KID)],
    };
    let app = app_with_signer(overlap, both_jwks);
    assert_eq!(
        userinfo_status(&app, &access).await,
        StatusCode::OK,
        "pre-rollover token verifies during the overlap"
    );

    // Retired: the set holds only the new key.
    let retired = TokenSigner::new(new_key.clone(), NEW_KID.to_string());
    let new_jwks = Jwks {
        keys: vec![jwk_for(&new_key, NEW_KID)],
    };
    let app = app_with_signer(retired, new_jwks);
    assert_eq!(
        userinfo_status(&app, &access).await,
        StatusCode::UNAUTHORIZED,
        "pre-rollover token rejected after retirement"
    );
}
