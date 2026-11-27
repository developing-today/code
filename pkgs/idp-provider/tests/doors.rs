//! Door tests (#13411/#13412/#13414 + linking #13415): the full
//! federated login loop over real HTTP against mock upstreams —
//! a Google-shaped OIDC upstream (authorize/token/JWKS, PKCE and
//! nonce enforced by the mock, ID tokens signed by a throwaway
//! key) and a GitHub-shaped OAuth upstream (code exchange + user
//! fetch, numeric id subject).
//!
//! Proven here: a door login for one of the account's declared
//! identities mints tokens for the SAME account as the local
//! door; a never-seen allowlisted identity that is NOT declared
//! enrolls into its own read-only account (#15249, design #9086
//! §4.4) — never the owner's; a non-allowlisted upstream identity
//! is denied and audited; an operator-unlinked identity is denied
//! while the account stays reachable via the local door, with the
//! history intact; door state handles are single-use; a door
//! without its client secret denies at start.

use axum::{
    Json, Router,
    extract::{Form, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD as B64URL};
use idp_provider::audit::AuditLog;
use idp_provider::flow::FlowState;
use idp_provider::identities::IdentityStore;
use idp_provider::registry::{Account, Client, Identity, ProviderConfig, UpstreamDoor, now_unix};
use idp_provider::signing::TokenSigner;
use idp_provider::upstream::DoorRuntime;
use idp_provider::{Config, Jwk, Jwks, b64url, build_app_with_flow};
use rand_core::OsRng;
use rsa::pkcs1v15::{Signature, VerifyingKey};
use rsa::signature::Verifier;
use rsa::traits::PublicKeyParts;
use rsa::{RsaPrivateKey, RsaPublicKey};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const CLIENT_ID: &str = "test-rp";
const RP_REDIRECT: &str = "http://127.0.0.1:8912/callback";
const GOOGLE_SUB: &str = "drew-google-sub";
const GITHUB_ID: i64 = 424242;
const OWNER_CREDENTIAL: &str = "test-owner-credential";
const ROLES: [&str; 4] = ["owner", "orchestrator", "agent", "read-only"];

// ---------------- mock upstreams ----------------

struct MockState {
    key: RsaPrivateKey,
    issuer: String,
    client_id: String,
    subject: Mutex<String>,
    github_id: Mutex<i64>,
    github_scope: Mutex<Option<String>>,
    email: Mutex<Option<String>>,
    email_verified: Mutex<bool>,
    last_nonce: Mutex<Option<String>>,
    last_challenge: Mutex<Option<String>>,
    oidc_scope: Mutex<Option<String>>,
}

fn mock_app(state: Arc<MockState>) -> Router {
    Router::new()
        .route("/oidc/authorize", get(mock_oidc_authorize))
        .route("/oidc/token", post(mock_oidc_token))
        .route("/oidc/jwks", get(mock_oidc_jwks))
        .route("/gh/authorize", get(mock_gh_authorize))
        .route("/gh/token", post(mock_gh_token))
        .route("/gh/user", get(mock_gh_user))
        .route("/gh/user/emails", get(mock_gh_emails))
        .with_state(state)
}

async fn mock_oidc_authorize(
    State(m): State<Arc<MockState>>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    *m.last_nonce.lock().unwrap() = q.get("nonce").cloned();
    *m.last_challenge.lock().unwrap() = q.get("code_challenge").cloned();
    *m.oidc_scope.lock().unwrap() = q.get("scope").cloned();
    let redirect = q.get("redirect_uri").cloned().unwrap_or_default();
    let state = q.get("state").cloned().unwrap_or_default();
    (
        StatusCode::FOUND,
        [(
            header::LOCATION,
            format!("{redirect}?code=mockcode&state={state}"),
        )],
    )
        .into_response()
}

async fn mock_oidc_token(
    State(m): State<Arc<MockState>>,
    Form(f): Form<HashMap<String, String>>,
) -> Response {
    if f.get("code").map(|s| s.as_str()) != Some("mockcode") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "bad_code"})),
        )
            .into_response();
    }
    // The mock enforces PKCE: S256(verifier) must equal the
    // challenge it saw at authorize time.
    let verifier = f.get("code_verifier").cloned().unwrap_or_default();
    let computed = b64url(&Sha256::digest(verifier.as_bytes()));
    if m.last_challenge.lock().unwrap().as_deref() != Some(computed.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "pkce_failed"})),
        )
            .into_response();
    }
    let now = now_unix();
    let mut claims = serde_json::json!({
        "iss": m.issuer,
        "sub": *m.subject.lock().unwrap(),
        "aud": m.client_id,
        "iat": now,
        "exp": now + 600,
    });
    if let Some(email) = m.email.lock().unwrap().clone() {
        claims["email"] = serde_json::Value::String(email);
        claims["email_verified"] = serde_json::Value::Bool(*m.email_verified.lock().unwrap());
    }
    if let Some(nonce) = m.last_nonce.lock().unwrap().clone() {
        claims["nonce"] = serde_json::Value::String(nonce);
    }
    let signer = TokenSigner::new(m.key.clone(), "mock-kid".to_string());
    let id_token = signer.sign_jwt(&claims).expect("mock signing");
    Json(serde_json::json!({
        "access_token": "mock-access-token",
        "id_token": id_token,
        "token_type": "Bearer",
        "expires_in": 600,
    }))
    .into_response()
}

async fn mock_oidc_jwks(State(m): State<Arc<MockState>>) -> Response {
    let pubkey = RsaPublicKey::from(&m.key);
    Json(serde_json::json!({
        "keys": [{
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "kid": "mock-kid",
            "n": b64url(&pubkey.n().to_bytes_be()),
            "e": b64url(&pubkey.e().to_bytes_be()),
        }]
    }))
    .into_response()
}

async fn mock_gh_authorize(
    State(m): State<Arc<MockState>>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    *m.github_scope.lock().unwrap() = q.get("scope").cloned();
    let redirect = q.get("redirect_uri").cloned().unwrap_or_default();
    let state = q.get("state").cloned().unwrap_or_default();
    (
        StatusCode::FOUND,
        [(
            header::LOCATION,
            format!("{redirect}?code=ghcode&state={state}"),
        )],
    )
        .into_response()
}

async fn mock_gh_token(Form(f): Form<HashMap<String, String>>) -> Response {
    if f.get("code").map(|s| s.as_str()) != Some("ghcode") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "bad_code"})),
        )
            .into_response();
    }
    Json(serde_json::json!({
        "access_token": "mock-gh-token",
        "scope": "read:user user:email",
        "token_type": "bearer",
    }))
    .into_response()
}

async fn mock_gh_user(State(m): State<Arc<MockState>>, headers: HeaderMap) -> Response {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer mock-gh-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "bad_token"})),
        )
            .into_response();
    }
    Json(serde_json::json!({
        "id": *m.github_id.lock().unwrap(),
        "login": "drew",
    }))
    .into_response()
}

async fn mock_gh_emails(State(m): State<Arc<MockState>>, headers: HeaderMap) -> Response {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != "Bearer mock-gh-token" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "bad_token"})),
        )
            .into_response();
    }
    let emails = m
        .email
        .lock()
        .unwrap()
        .clone()
        .map(|email| {
            serde_json::json!([{
                "email": email,
                "primary": true,
                "verified": *m.email_verified.lock().unwrap(),
                "visibility": "private"
            }])
        })
        .unwrap_or_else(|| serde_json::json!([]));
    Json(emails).into_response()
}

// ---------------- harness ----------------

struct Harness {
    issuer: String,
    client: reqwest::Client,
    audit: Arc<AuditLog>,
    identities: Arc<IdentityStore>,
    mock: Arc<MockState>,
    provider_pubkey: RsaPublicKey,
}

/// The baseline allowlist: the owner's local identity plus the
/// owner's own Google/GitHub door identities (both also declared
/// in the account row — the owner's identity set). This is the
/// pre-drill (production-shaped) list: the H7 drill must end with
/// exactly this list in force.
fn baseline_allowlist() -> Vec<String> {
    vec![
        "devtoday-local:drew".to_string(),
        format!("google:{GOOGLE_SUB}"),
        format!("github:{GITHUB_ID}"),
    ]
}

async fn harness() -> Harness {
    harness_with_allowlist(baseline_allowlist()).await
}

async fn harness_with_allowlist(allowlist: Vec<String>) -> Harness {
    let mut rng = OsRng;
    // Mock upstream first (its port feeds the door config).
    let mock_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock");
    let mock_base = format!("http://{}", mock_listener.local_addr().unwrap());
    let mock = Arc::new(MockState {
        key: RsaPrivateKey::new(&mut rng, 2048).expect("mock key"),
        issuer: mock_base.clone(),
        client_id: "mock-google-client".to_string(),
        subject: Mutex::new(GOOGLE_SUB.to_string()),
        github_id: Mutex::new(GITHUB_ID),
        github_scope: Mutex::new(None),
        email: Mutex::new(None),
        email_verified: Mutex::new(false),
        last_nonce: Mutex::new(None),
        last_challenge: Mutex::new(None),
        oidc_scope: Mutex::new(None),
    });
    let mock2 = mock.clone();
    tokio::spawn(async move {
        axum::serve(mock_listener, mock_app(mock2)).await.unwrap();
    });

    // Provider listener next (its port is the issuer).
    let prov_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind provider");
    let issuer = format!("http://{}", prov_listener.local_addr().unwrap());

    let provider_key = RsaPrivateKey::new(&mut rng, 2048).expect("provider key");
    let provider_pubkey = RsaPublicKey::from(&provider_key);
    let kid = "test-kid".to_string();
    let jwks = Jwks {
        keys: vec![Jwk {
            kty: "RSA".to_string(),
            use_: "sig".to_string(),
            alg: "RS256".to_string(),
            kid: kid.clone(),
            x5t_s256: kid.clone(),
            n: b64url(&provider_pubkey.n().to_bytes_be()),
            e: b64url(&provider_pubkey.e().to_bytes_be()),
        }],
    };
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
            email: Some("owner@example.test".to_string()),
            roles: ROLES.iter().map(|s| s.to_string()).collect(),
            // The owner's declared identity set: these seed active
            // under the owner account. Any OTHER allowlisted
            // identity enrolls into its own account (#15249).
            identities: vec![
                Identity {
                    provider: "devtoday-local".to_string(),
                    subject: "drew".to_string(),
                },
                Identity {
                    provider: "google".to_string(),
                    subject: GOOGLE_SUB.to_string(),
                },
                Identity {
                    provider: "github".to_string(),
                    subject: GITHUB_ID.to_string(),
                },
            ],
            local_credential_hash: credential_hash,
        },
        allowlist,
        clients: vec![Client {
            client_id: CLIENT_ID.to_string(),
            name: "Test RP".to_string(),
            redirect_uris: vec![RP_REDIRECT.to_string()],
        }],
        doors: vec![
            UpstreamDoor {
                name: "google".to_string(),
                kind: "oidc".to_string(),
                display_name: "Google".to_string(),
                client_id: "mock-google-client".to_string(),
                client_secret_env: "UNUSED_IN_TESTS".to_string(),
                authorize_url: format!("{mock_base}/oidc/authorize"),
                token_url: format!("{mock_base}/oidc/token"),
                jwks_uri: Some(format!("{mock_base}/oidc/jwks")),
                userinfo_url: None,
                issuer: Some(mock_base.clone()),
                subject_claim: None,
                scopes: vec!["openid".to_string()],
            },
            UpstreamDoor {
                name: "github".to_string(),
                kind: "github".to_string(),
                display_name: "GitHub".to_string(),
                client_id: "mock-gh-client".to_string(),
                client_secret_env: "UNUSED_IN_TESTS".to_string(),
                authorize_url: format!("{mock_base}/gh/authorize"),
                token_url: format!("{mock_base}/gh/token"),
                jwks_uri: None,
                userinfo_url: Some(format!("{mock_base}/gh/user")),
                issuer: None,
                subject_claim: None,
                scopes: vec!["read:user".to_string()],
            },
            UpstreamDoor {
                name: "microsoft".to_string(),
                kind: "oidc".to_string(),
                display_name: "Microsoft".to_string(),
                client_id: "mock-ms-client".to_string(),
                client_secret_env: "UNUSED_IN_TESTS".to_string(),
                authorize_url: format!("{mock_base}/oidc/authorize"),
                token_url: format!("{mock_base}/oidc/token"),
                jwks_uri: Some(format!("{mock_base}/oidc/jwks")),
                userinfo_url: None,
                issuer: Some(mock_base.clone()),
                subject_claim: Some("oid".to_string()),
                scopes: vec!["openid".to_string()],
            },
        ],
        audit_db_path: None,
        identity_db_path: None,
    };
    let audit = Arc::new(AuditLog::open(None).expect("audit"));
    let identities = Arc::new(IdentityStore::open(None).expect("identity store"));
    let doors = vec![
        DoorRuntime {
            config: registry.doors[0].clone(),
            client_secret: Some("mock-secret".to_string()),
        },
        DoorRuntime {
            config: registry.doors[1].clone(),
            client_secret: Some("mock-secret".to_string()),
        },
        // Microsoft is configured but its secret never arrived:
        // the door must stay disabled.
        DoorRuntime {
            config: registry.doors[2].clone(),
            client_secret: None,
        },
    ];
    let flow = Arc::new(FlowState::new(
        registry,
        TokenSigner::new(provider_key, kid),
        audit.clone(),
        identities.clone(),
        doors,
        issuer.clone(),
    ));
    let config = Config {
        issuer: issuer.clone(),
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
    let app = build_app_with_flow(config, jwks, Some(flow));
    tokio::spawn(async move {
        axum::serve(prov_listener, app).await.unwrap();
    });

    Harness {
        issuer,
        client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
        audit,
        identities,
        mock,
        provider_pubkey,
    }
}

fn pkce() -> (String, String) {
    let verifier = "test-verifier-0123456789-abcdefghijklmnopqrstuvwxyz";
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    (verifier.to_string(), challenge)
}

fn authorize_query() -> String {
    let (_, challenge) = pkce();
    format!(
        "response_type=code&client_id={CLIENT_ID}&redirect_uri={}&scope=openid&state=rpstate&nonce=rpnonce&code_challenge={challenge}&code_challenge_method=S256",
        url_encode(RP_REDIRECT)
    )
}

fn url_encode(s: &str) -> String {
    idp_provider::registry::url_encode(s)
}

fn location(resp: &reqwest::Response) -> String {
    resp.headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

/// Drive one door login from the RP authorize URL through the
/// upstream and back. Returns the final callback response.
async fn drive_door(h: &Harness, door: &str) -> reqwest::Response {
    let q = authorize_query();
    let start = format!("{}/door/{door}/start?{q}", h.issuer);
    let resp = h.client.get(&start).send().await.expect("door start");
    assert_eq!(resp.status(), 302, "door start should redirect upstream");
    let upstream_url = location(&resp);
    assert!(
        upstream_url.contains("/authorize"),
        "upstream redirect: {upstream_url}"
    );
    let resp = h
        .client
        .get(&upstream_url)
        .send()
        .await
        .expect("upstream authorize");
    assert_eq!(resp.status(), 302, "upstream should redirect back");
    let callback_url = location(&resp);
    assert!(
        callback_url.contains(&format!("/door/{door}/callback")),
        "callback url: {callback_url}"
    );
    h.client
        .get(&callback_url)
        .send()
        .await
        .expect("door callback")
}

/// Exchange a code from a callback redirect Location for the
/// full token document.
async fn exchange_doc(h: &Harness, loc: &str) -> serde_json::Value {
    let code = loc
        .split("code=")
        .nth(1)
        .and_then(|s| s.split('&').next())
        .expect("code in redirect")
        .to_string();
    let (verifier, _) = pkce();
    let resp = h
        .client
        .post(format!("{}/token", h.issuer))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", RP_REDIRECT),
            ("client_id", CLIENT_ID),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await
        .expect("token exchange");
    assert_eq!(resp.status(), 200, "token exchange succeeds");
    resp.json().await.expect("token json")
}

/// Exchange a code and verify the ID token against the harness's
/// provider key, returning its claims.
async fn exchange_and_verify(h: &Harness, loc: &str) -> serde_json::Value {
    let doc = exchange_doc(h, loc).await;
    let id_token = doc["id_token"].as_str().expect("id token");
    verify_with(id_token, &h.provider_pubkey)
}

fn verify_with(token: &str, pubkey: &RsaPublicKey) -> serde_json::Value {
    let parts: Vec<&str> = token.split('.').collect();
    assert_eq!(parts.len(), 3);
    let sig_bytes = B64URL.decode(parts[2]).unwrap();
    let signature = Signature::try_from(sig_bytes.as_slice()).unwrap();
    VerifyingKey::<Sha256>::new(pubkey.clone())
        .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
        .expect("id token signature verifies");
    serde_json::from_slice(&B64URL.decode(parts[1]).unwrap()).unwrap()
}

fn audit_has(h: &Harness, endpoint: &str, outcome: &str, reason: &str, identity: &str) -> bool {
    h.audit.entries().unwrap().iter().any(|e| {
        e.endpoint == endpoint
            && e.outcome == outcome
            && e.reason == reason
            && e.identity == identity
    })
}

// ---------------- tests ----------------

#[tokio::test]
async fn login_form_lists_enabled_doors_only() {
    let h = harness().await;
    let resp = h
        .client
        .get(format!("{}/authorize?{}", h.issuer, authorize_query()))
        .send()
        .await
        .expect("authorize form");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("/door/google/start"), "google listed");
    assert!(body.contains("/door/github/start"), "github listed");
    assert!(
        !body.contains("/door/microsoft/start"),
        "disabled door not listed"
    );
}

#[tokio::test]
async fn google_door_full_loop_reaches_same_account() {
    let h = harness().await;
    let resp = drive_door(&h, "google").await;
    assert_eq!(resp.status(), 302, "callback redirects to the RP");
    assert!(
        location(&resp).contains("state=rpstate"),
        "rp state echoed to the RP"
    );
    let claims = exchange_and_verify(&h, &location(&resp)).await;
    assert_eq!(claims["sub"], "drew", "same account as the local door");
    assert_eq!(claims["preferred_username"], format!("google:{GOOGLE_SUB}"));
    assert_eq!(
        claims["roles"],
        serde_json::json!(ROLES),
        "account role set on the token"
    );
    // The identity is one of the account's declared identities:
    // seeded active under the account at startup (history carries
    // the seed), and the login's code issuance is audited. No
    // door-login link/enrollment happens for a declared identity.
    assert!(h.identities.is_active("google", GOOGLE_SUB));
    let events = h.identities.events().unwrap();
    assert!(events.iter().any(|e| e.event == "seed"
        && e.provider == "google"
        && e.subject == GOOGLE_SUB
        && e.account_id == "drew"
        && e.actor == "config-seed"));
    assert!(audit_has(
        &h,
        "door/google",
        "allowed",
        "code_issued",
        &format!("google:{GOOGLE_SUB}")
    ));

    // Second login: still the same account, no new history events.
    let resp = drive_door(&h, "google").await;
    assert_eq!(resp.status(), 302);
    let _ = exchange_and_verify(&h, &location(&resp)).await;
    let google_events = h
        .identities
        .events()
        .unwrap()
        .iter()
        .filter(|e| e.provider == "google")
        .count();
    assert_eq!(google_events, 1, "only the seed event is recorded");
}

#[tokio::test]
async fn verified_email_match_registers_unallowlisted_identity_pending_without_code() {
    const CLAIM_SUB: &str = "email-claim-sub";
    let h = harness().await;
    *h.mock.subject.lock().unwrap() = CLAIM_SUB.to_string();
    *h.mock.email.lock().unwrap() = Some(" Owner@Example.test ".to_string());
    *h.mock.email_verified.lock().unwrap() = true;

    let resp = drive_door(&h, "google").await;
    assert!(
        h.mock
            .oidc_scope
            .lock()
            .unwrap()
            .as_deref()
            .unwrap()
            .contains("email"),
        "OIDC authorization must request verified email claims"
    );
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert!(
        location(&resp).is_empty(),
        "pending claim must not receive a code"
    );
    assert_eq!(
        h.identities.status("google", CLAIM_SUB).unwrap().as_deref(),
        Some("pending")
    );
    assert_eq!(
        h.identities
            .account_id_for("google", CLAIM_SUB)
            .unwrap()
            .as_deref(),
        Some("drew")
    );
    assert!(h.identities.events().unwrap().iter().any(|event| {
        event.event == "register"
            && event.provider == "google"
            && event.subject == CLAIM_SUB
            && event.account_id == "drew"
            && event.actor == "email-match"
    }));
    assert!(audit_has(
        &h,
        "door/google",
        "denied",
        "identity_pending",
        &format!("google:{CLAIM_SUB}")
    ));
}

#[tokio::test]
async fn unverified_oidc_email_does_not_create_a_claim() {
    const CLAIM_SUB: &str = "unverified-email-sub";
    let h = harness().await;
    *h.mock.subject.lock().unwrap() = CLAIM_SUB.to_string();
    *h.mock.email.lock().unwrap() = Some("owner@example.test".to_string());
    *h.mock.email_verified.lock().unwrap() = false;

    let resp = drive_door(&h, "google").await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert!(audit_has(
        &h,
        "door/google",
        "denied",
        "not_allowlisted",
        &format!("google:{CLAIM_SUB}")
    ));
    assert_eq!(h.identities.status("google", CLAIM_SUB).unwrap(), None);
}

#[tokio::test]
async fn github_verified_primary_email_registers_pending_claim() {
    let h = harness().await;
    *h.mock.github_id.lock().unwrap() = GITHUB_ID + 1;
    *h.mock.email.lock().unwrap() = Some("owner@example.test".to_string());
    *h.mock.email_verified.lock().unwrap() = true;

    let resp = drive_door(&h, "github").await;
    assert!(
        h.mock
            .github_scope
            .lock()
            .unwrap()
            .as_deref()
            .unwrap()
            .contains("user:email"),
        "GitHub authorization must request the private verified-email endpoint scope"
    );
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert!(location(&resp).is_empty());
    assert_eq!(
        h.identities
            .status("github", &(GITHUB_ID + 1).to_string())
            .unwrap()
            .as_deref(),
        Some("pending")
    );
    assert!(h.identities.events().unwrap().iter().any(|event| {
        event.event == "register"
            && event.provider == "github"
            && event.subject == (GITHUB_ID + 1).to_string()
            && event.account_id == "drew"
            && event.actor == "email-match"
    }));

    *h.mock.github_id.lock().unwrap() = GITHUB_ID + 2;
    *h.mock.email_verified.lock().unwrap() = false;
    let resp = drive_door(&h, "github").await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        h.identities
            .status("github", &(GITHUB_ID + 2).to_string())
            .unwrap(),
        None,
        "an unverified GitHub address must not create a claim"
    );
}

#[tokio::test]
async fn pending_identity_is_acknowledged_without_issuing_a_code() {
    const PENDING_SUB: &str = "pending-sub";
    let mut allowlist = baseline_allowlist();
    allowlist.push(format!("google:{PENDING_SUB}"));
    let h = harness_with_allowlist(allowlist).await;
    assert!(
        h.identities
            .register_pending("google", PENDING_SUB, "drew", "email-match")
            .unwrap()
    );
    *h.mock.subject.lock().unwrap() = PENDING_SUB.to_string();

    let resp = drive_door(&h, "google").await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert!(
        location(&resp).is_empty(),
        "pending login must not redirect with a code"
    );
    assert!(audit_has(
        &h,
        "door/google",
        "denied",
        "identity_pending",
        &format!("google:{PENDING_SUB}")
    ));
    assert_eq!(
        h.identities
            .status("google", PENDING_SUB)
            .unwrap()
            .as_deref(),
        Some("pending")
    );
    assert!(
        h.identities
            .events()
            .unwrap()
            .iter()
            .any(|e| e.event == "register")
    );
}

#[test]
fn identity_history_records_pending_disposition_and_provisioning_events() {
    let store = IdentityStore::open(None).unwrap();
    assert!(
        store
            .register_pending("google", "approved-sub", "drew", "session-add")
            .unwrap()
    );
    assert!(
        store
            .approve_pending("google", "approved-sub", "established-session")
            .unwrap()
    );
    assert!(
        store
            .register_pending("microsoft", "rejected-sub", "drew", "email-match")
            .unwrap()
    );
    assert!(
        store
            .reject_pending("microsoft", "rejected-sub", "established-session")
            .unwrap()
    );
    store
        .record_provision("github", "new-sub", "new-account", "provisioner")
        .unwrap();

    let events = store.events().unwrap();
    assert_eq!(
        events
            .iter()
            .map(|event| event.event.as_str())
            .collect::<Vec<_>>(),
        ["register", "approve", "register", "reject", "provision"]
    );
    let approved = store
        .list()
        .unwrap()
        .into_iter()
        .find(|row| row.subject == "approved-sub")
        .unwrap();
    assert_eq!(approved.status, "active");
    assert_eq!(approved.claimed_via.as_deref(), Some("session-add"));
    assert!(approved.claimed_at.is_some());
}

#[test]
fn enrolled_identity_can_be_relinked_to_another_account_by_an_operator() {
    let store = IdentityStore::open(None).unwrap();
    let (account_id, event) = store
        .enroll("google", "teammate-sub", "door-login")
        .unwrap();
    assert_eq!(event, "link");
    assert_ne!(account_id, "drew");
    assert_eq!(
        store.account(&account_id).unwrap().unwrap().roles,
        vec!["read-only".to_string()],
        "enrolled accounts hold only read-only"
    );
    // Deterministic: the same identity always maps to the same
    // enrolled account id.
    assert_eq!(
        idp_provider::identities::enrolled_account_id("google", "teammate-sub"),
        account_id
    );
    // The explicit afterwards grant (#15249): an operator unlinks
    // and re-links the identity onto the owner account, and the
    // row itself moves — not just the event.
    assert!(
        store
            .unlink("google", "teammate-sub", "operator-cli")
            .unwrap()
    );
    assert_eq!(
        store
            .link("google", "teammate-sub", "drew", "operator-cli")
            .unwrap(),
        "relink"
    );
    let row = store
        .list()
        .unwrap()
        .into_iter()
        .find(|r| r.subject == "teammate-sub")
        .unwrap();
    assert_eq!(row.account_id, "drew");
    assert_eq!(row.status, "active");
}

#[tokio::test]
async fn github_door_full_loop_reaches_same_account() {
    let h = harness().await;
    let resp = drive_door(&h, "github").await;
    assert_eq!(resp.status(), 302);
    let claims = exchange_and_verify(&h, &location(&resp)).await;
    assert_eq!(claims["sub"], "drew", "same account as the other doors");
    assert_eq!(
        claims["preferred_username"],
        format!("github:{GITHUB_ID}"),
        "github subject is the numeric user id"
    );
    assert!(h.identities.is_active("github", &GITHUB_ID.to_string()));
}

/// Regression test for #15249 (oracle: design #9086 §4.4). A
/// never-seen allowlisted identity that is NOT one of the owner
/// account's declared identities must enroll into a NEW account
/// holding only `read-only` — it must not land on the owner
/// account or carry the owner's roles, at first login or later.
#[tokio::test]
async fn non_owner_allowlisted_identity_enrolls_into_its_own_read_only_account() {
    const TEAMMATE_SUB: &str = "teammate-sub";
    let mut allowlist = baseline_allowlist();
    allowlist.push(format!("google:{TEAMMATE_SUB}"));
    let h = harness_with_allowlist(allowlist).await;
    *h.mock.subject.lock().unwrap() = TEAMMATE_SUB.to_string();

    let resp = drive_door(&h, "google").await;
    assert_eq!(resp.status(), 302, "enrolled identity logs in");
    let doc = exchange_doc(&h, &location(&resp)).await;
    let claims = verify_with(doc["id_token"].as_str().unwrap(), &h.provider_pubkey);
    assert_ne!(
        claims["sub"], "drew",
        "a non-owner identity must not land on the owner account"
    );
    assert_eq!(
        claims["roles"],
        serde_json::json!(["read-only"]),
        "a newly enrolled account holds only read-only (design #9086 §4.4)"
    );
    assert_eq!(
        claims["preferred_username"],
        format!("google:{TEAMMATE_SUB}")
    );
    let enrolled_id = claims["sub"].as_str().unwrap().to_string();

    // The identity row names the same new account, and the
    // store's account row for it holds only read-only.
    let row = h
        .identities
        .list()
        .unwrap()
        .into_iter()
        .find(|r| r.provider == "google" && r.subject == TEAMMATE_SUB)
        .expect("enrollment wrote an identity row");
    assert_eq!(row.status, "active");
    assert_eq!(row.account_id, enrolled_id);
    let account = h
        .identities
        .account(&enrolled_id)
        .unwrap()
        .expect("enrollment wrote an account row");
    assert_eq!(account.roles, vec!["read-only".to_string()]);
    // The link event and the audit record name the NEW account.
    assert!(h.identities.events().unwrap().iter().any(|e| {
        e.event == "link"
            && e.provider == "google"
            && e.subject == TEAMMATE_SUB
            && e.account_id == enrolled_id
            && e.actor == "door-login"
    }));
    assert!(audit_has(
        &h,
        "link",
        "allowed",
        "link",
        &format!("google:{TEAMMATE_SUB}")
    ));

    // userinfo serves the enrolled account, not the owner's.
    let resp = h
        .client
        .get(format!("{}/userinfo", h.issuer))
        .header(
            header::AUTHORIZATION,
            format!("Bearer {}", doc["access_token"].as_str().unwrap()),
        )
        .send()
        .await
        .expect("userinfo");
    assert_eq!(resp.status(), 200);
    let info: serde_json::Value = resp.json().await.expect("userinfo json");
    assert_eq!(info["sub"], enrolled_id);
    assert_eq!(info["roles"], serde_json::json!(["read-only"]));

    // Second login: the SAME enrolled account, still read-only,
    // and no second enrollment event.
    let resp = drive_door(&h, "google").await;
    assert_eq!(resp.status(), 302);
    let claims = exchange_and_verify(&h, &location(&resp)).await;
    assert_eq!(claims["sub"], enrolled_id);
    assert_eq!(claims["roles"], serde_json::json!(["read-only"]));
    let enroll_events = h
        .identities
        .events()
        .unwrap()
        .iter()
        .filter(|e| e.event == "link" && e.subject == TEAMMATE_SUB)
        .count();
    assert_eq!(enroll_events, 1, "enrollment recorded once");
}

#[tokio::test]
async fn merged_secondary_signs_in_as_the_primary_and_old_tokens_resolve() {
    // #13975 slice 2 acceptance over mock upstreams: two
    // auto-enrolled accounts for one person merge into the one they
    // pick; a login through the former-secondary identity reaches the
    // primary, and a token issued before the merge still resolves.
    const FIRST_SUB: &str = "person-first-sub";
    const SECOND_SUB: &str = "person-second-sub";
    let mut allowlist = baseline_allowlist();
    allowlist.push(format!("google:{FIRST_SUB}"));
    allowlist.push(format!("google:{SECOND_SUB}"));
    let h = harness_with_allowlist(allowlist).await;

    *h.mock.subject.lock().unwrap() = FIRST_SUB.to_string();
    let resp = drive_door(&h, "google").await;
    let primary = exchange_and_verify(&h, &location(&resp)).await["sub"]
        .as_str()
        .unwrap()
        .to_string();
    *h.mock.subject.lock().unwrap() = SECOND_SUB.to_string();
    let resp = drive_door(&h, "google").await;
    let old_doc = exchange_doc(&h, &location(&resp)).await;
    let old_claims = verify_with(old_doc["id_token"].as_str().unwrap(), &h.provider_pubkey);
    let secondary = old_claims["sub"].as_str().unwrap().to_string();
    assert_ne!(primary, secondary);

    let preview = h.identities.merge_preview(&primary, &secondary).unwrap();
    assert!(preview.secondary_only_roles.is_empty());
    h.identities
        .merge(&primary, &secondary, &[], &[], "established-sessions")
        .unwrap();

    // A fresh login through the former-secondary identity lands on
    // the primary.
    let resp = drive_door(&h, "google").await;
    assert_eq!(resp.status(), 302);
    let claims = exchange_and_verify(&h, &location(&resp)).await;
    assert_eq!(claims["sub"], primary.as_str());
    assert_eq!(claims["roles"], serde_json::json!(["read-only"]));

    // The pre-merge token keeps its `sub`; userinfo answers with it
    // and names the account it now reads as.
    let resp = h
        .client
        .get(format!("{}/userinfo", h.issuer))
        .header(
            header::AUTHORIZATION,
            format!("Bearer {}", old_doc["access_token"].as_str().unwrap()),
        )
        .send()
        .await
        .expect("userinfo");
    assert_eq!(resp.status(), 200);
    let info: serde_json::Value = resp.json().await.expect("userinfo json");
    assert_eq!(info["sub"], secondary.as_str());
    assert_eq!(info["merged_into"], primary.as_str());

    // Both histories carry the merge; the secondary's enrollment
    // event still names the secondary.
    let events = h.identities.events().unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.event == "link" && e.subject == SECOND_SUB && e.account_id == secondary)
    );
    assert!(
        events
            .iter()
            .any(|e| e.event == "merge" && e.subject == SECOND_SUB && e.account_id == primary)
    );
    let account_events = h.identities.account_events().unwrap();
    assert!(
        account_events
            .iter()
            .any(|e| e.event == "merge-into" && e.account_id == secondary)
    );
}

#[tokio::test]
async fn google_door_non_allowlisted_denied_and_audited() {
    let h = harness().await;
    *h.mock.subject.lock().unwrap() = "mallory-sub".to_string();
    let resp = drive_door(&h, "google").await;
    assert_eq!(resp.status(), 403, "non-allowlisted identity denied");
    assert!(audit_has(
        &h,
        "door/google",
        "denied",
        "not_allowlisted",
        "google:mallory-sub"
    ));
    // No link, no row: the store never saw a linkable identity.
    assert_eq!(h.identities.status("google", "mallory-sub").unwrap(), None);
}

#[tokio::test]
async fn unlinked_identity_denied_but_account_reachable_via_local_door() {
    let h = harness().await;
    // First login over the identity's seeded link.
    let resp = drive_door(&h, "google").await;
    assert_eq!(resp.status(), 302);
    let _ = exchange_and_verify(&h, &location(&resp)).await;
    // Operator unlinks the Google door.
    assert!(
        h.identities
            .unlink("google", GOOGLE_SUB, "operator-cli")
            .unwrap()
    );
    // The door now denies, with the history intact.
    let resp = drive_door(&h, "google").await;
    assert_eq!(resp.status(), 403);
    assert!(audit_has(
        &h,
        "door/google",
        "denied",
        "identity_unlinked",
        &format!("google:{GOOGLE_SUB}")
    ));
    let rows = h.identities.list().unwrap();
    let row = rows
        .iter()
        .find(|r| r.provider == "google")
        .expect("row retained after unlink");
    assert_eq!(row.status, "unlinked");
    assert!(row.unlinked_at.is_some());
    let events = h.identities.events().unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.event == "seed" && e.provider == "google")
    );
    assert!(
        events
            .iter()
            .any(|e| e.event == "unlink" && e.provider == "google")
    );

    // The account is still reachable via the local door.
    let (verifier, challenge) = pkce();
    let _ = verifier;
    let body = format!(
        "response_type=code&client_id={CLIENT_ID}&redirect_uri={}&scope=openid&state=s2&code_challenge={challenge}&code_challenge_method=S256&login_id=drew&credential={}",
        url_encode(RP_REDIRECT),
        url_encode(OWNER_CREDENTIAL)
    );
    let resp = h
        .client
        .post(format!("{}/authorize", h.issuer))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .expect("local door login");
    assert_eq!(resp.status(), 302, "local door still opens the account");
    let claims = exchange_and_verify(&h, &location(&resp)).await;
    assert_eq!(claims["sub"], "drew");
}

#[tokio::test]
async fn door_state_handle_is_single_use() {
    let h = harness().await;
    let q = authorize_query();
    let resp = h
        .client
        .get(format!("{}/door/google/start?{q}", h.issuer))
        .send()
        .await
        .unwrap();
    let upstream_url = location(&resp);
    let resp = h.client.get(&upstream_url).send().await.unwrap();
    let callback_url = location(&resp);
    let first = h.client.get(&callback_url).send().await.unwrap();
    assert_eq!(first.status(), 302, "first use succeeds");
    let replay = h.client.get(&callback_url).send().await.unwrap();
    assert_eq!(replay.status(), 400, "replayed state is denied");
    assert!(audit_has(
        &h,
        "door/google",
        "denied",
        "unknown_or_expired_state",
        ""
    ));
}

// ---------------- H7 gate-relaxation drill (#14265) ----------------

/// H7 drill (child of #9086, design §4.4 + §5 slice H7): dry-run
/// the gate relaxation — add a test enrollment, verify what the
/// enrolled identity gets, remove it — against throwaway provider
/// configs/stores only. Production policy is never touched: each
/// step builds its own harness, and the drill ends with the
/// baseline allowlist in force and the test identity denied.
///
/// Oracle: design §4.4 — the gate is deny-by-default, relaxation
/// is a data change (allowlist entries), and a newly enrolled
/// account with only `read-only` on no tenant can do nothing
/// until granted. The middle assertion checks the enrolled
/// identity against that oracle directly (see the runbook in
/// `doc/2026-11-25T20-55-00Z_runbook_h7-gate-relaxation-drill/`).
#[tokio::test]
async fn h7_gate_relaxation_drill_add_verify_remove() {
    const TEST_SUB: &str = "h7-drill-test-subject";
    let test_key = format!("google:{TEST_SUB}");

    // Step 0 — pre-drill state: the test identity is not enrolled,
    // is denied at the gate (audited), and leaves no identity row.
    let h = harness_with_allowlist(baseline_allowlist()).await;
    *h.mock.subject.lock().unwrap() = TEST_SUB.to_string();
    let resp = drive_door(&h, "google").await;
    assert_eq!(resp.status(), 403, "pre-drill: test identity denied");
    assert!(audit_has(
        &h,
        "door/google",
        "denied",
        "not_allowlisted",
        &test_key
    ));
    assert_eq!(h.identities.status("google", TEST_SUB).unwrap(), None);
    drop(h);

    // Step 1 — add the test enrollment: one allowlist entry, the
    // §4.4 relaxation as a pure data change (no redeploy).
    let mut relaxed = baseline_allowlist();
    relaxed.push(test_key.clone());
    let h = harness_with_allowlist(relaxed).await;
    *h.mock.subject.lock().unwrap() = TEST_SUB.to_string();
    let resp = drive_door(&h, "google").await;
    assert_eq!(resp.status(), 302, "drill: enrolled test identity logs in");
    let claims = exchange_and_verify(&h, &location(&resp)).await;
    // §4.4 oracle (fixed by #15249 / PR #15313): a never-seen
    // allowlisted identity that is not one of the owner account's
    // declared identities enrolls into its OWN account holding
    // only `read-only` — it does not link to the owner account
    // and does not carry the owner's roles. An allowlist entry is
    // therefore safe for any identity: enrollment alone grants
    // nothing beyond read-only on no tenant. (The drill's first
    // run observed the pre-fix single-account behaviour; that
    // finding was filed as #15249 and is fixed on main.)
    assert_ne!(
        claims["sub"], "drew",
        "enrolled identity must not land on the owner account"
    );
    let enrolled_account = h
        .identities
        .account_id_for("google", TEST_SUB)
        .unwrap()
        .expect("enrollment wrote an identity row");
    assert_eq!(
        claims["sub"], enrolled_account,
        "enrolled identity lands on its own enrolled account"
    );
    assert_eq!(
        claims["roles"],
        serde_json::json!(["read-only"]),
        "newly enrolled account holds only read-only (§4.4)"
    );
    assert!(h.identities.is_active("google", TEST_SUB));
    drop(h);

    // Step 2 — remove the test enrollment: the baseline allowlist
    // is in force again and the same identity is denied, audited,
    // exactly as in the pre-drill state.
    let h = harness_with_allowlist(baseline_allowlist()).await;
    *h.mock.subject.lock().unwrap() = TEST_SUB.to_string();
    let resp = drive_door(&h, "google").await;
    assert_eq!(resp.status(), 403, "post-drill: test identity denied again");
    assert!(audit_has(
        &h,
        "door/google",
        "denied",
        "not_allowlisted",
        &test_key
    ));
}

#[tokio::test]
async fn disabled_door_denies_at_start() {
    let h = harness().await;
    let resp = h
        .client
        .get(format!(
            "{}/door/microsoft/start?{}",
            h.issuer,
            authorize_query()
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 503, "unconfigured door denies");
    assert!(audit_has(
        &h,
        "door/microsoft",
        "denied",
        "door_not_configured",
        ""
    ));
}
