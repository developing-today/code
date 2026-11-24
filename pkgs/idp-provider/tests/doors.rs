//! Door tests (#13411/#13412/#13414 + linking #13415): the full
//! federated login loop over real HTTP against mock upstreams —
//! a Google-shaped OIDC upstream (authorize/token/JWKS, PKCE and
//! nonce enforced by the mock, ID tokens signed by a throwaway
//! key) and a GitHub-shaped OAuth upstream (code exchange + user
//! fetch, numeric id subject).
//!
//! Proven here: a door login mints tokens for the SAME account as
//! the local door; a first allowlisted login links the identity
//! (audited, history written); a non-allowlisted upstream identity
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
use rsa::{BigUint, RsaPrivateKey, RsaPublicKey};
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
    last_nonce: Mutex<Option<String>>,
    last_challenge: Mutex<Option<String>>,
}

fn mock_app(state: Arc<MockState>) -> Router {
    Router::new()
        .route("/oidc/authorize", get(mock_oidc_authorize))
        .route("/oidc/token", post(mock_oidc_token))
        .route("/oidc/jwks", get(mock_oidc_jwks))
        .route("/gh/authorize", get(mock_gh_authorize))
        .route("/gh/token", post(mock_gh_token))
        .route("/gh/user", get(mock_gh_user))
        .with_state(state)
}

async fn mock_oidc_authorize(
    State(m): State<Arc<MockState>>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    *m.last_nonce.lock().unwrap() = q.get("nonce").cloned();
    *m.last_challenge.lock().unwrap() = q.get("code_challenge").cloned();
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

async fn mock_gh_authorize(Query(q): Query<HashMap<String, String>>) -> Response {
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
        "scope": "read:user",
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

// ---------------- harness ----------------

struct Harness {
    issuer: String,
    client: reqwest::Client,
    audit: Arc<AuditLog>,
    identities: Arc<IdentityStore>,
    mock: Arc<MockState>,
    provider_pubkey: RsaPublicKey,
}

async fn harness() -> Harness {
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
        last_nonce: Mutex::new(None),
        last_challenge: Mutex::new(None),
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
            roles: ROLES.iter().map(|s| s.to_string()).collect(),
            identities: vec![Identity {
                provider: "devtoday-local".to_string(),
                subject: "drew".to_string(),
            }],
            local_credential_hash: credential_hash,
        },
        allowlist: vec![
            "devtoday-local:drew".to_string(),
            format!("google:{GOOGLE_SUB}"),
            format!("github:{GITHUB_ID}"),
        ],
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

/// Exchange a code from a callback redirect Location for tokens
/// and verify the ID token against the harness's provider key.
async fn exchange_and_verify(h: &Harness, loc: &str) -> serde_json::Value {
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
    let doc: serde_json::Value = resp.json().await.expect("token json");
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
async fn google_door_full_loop_links_and_reaches_same_account() {
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
    // First login linked the identity: store row active, history
    // has the link, and the audit record carries both events.
    assert!(h.identities.is_active("google", GOOGLE_SUB));
    let events = h.identities.events().unwrap();
    assert!(events.iter().any(|e| e.event == "link"
        && e.provider == "google"
        && e.subject == GOOGLE_SUB
        && e.actor == "door-login"));
    assert!(audit_has(
        &h,
        "link",
        "allowed",
        "link",
        &format!("google:{GOOGLE_SUB}")
    ));
    assert!(audit_has(
        &h,
        "door/google",
        "allowed",
        "code_issued",
        &format!("google:{GOOGLE_SUB}")
    ));

    // Second login: already linked — succeeds, no second link event.
    let resp = drive_door(&h, "google").await;
    assert_eq!(resp.status(), 302);
    let _ = exchange_and_verify(&h, &location(&resp)).await;
    let link_events = h
        .identities
        .events()
        .unwrap()
        .iter()
        .filter(|e| e.event == "link" && e.provider == "google")
        .count();
    assert_eq!(link_events, 1, "link recorded once");
}

#[tokio::test]
async fn pending_identity_is_acknowledged_without_issuing_a_code() {
    let h = harness().await;
    assert!(
        h.identities
            .register_pending("google", GOOGLE_SUB, "drew", "email-match")
            .unwrap()
    );

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
        &format!("google:{GOOGLE_SUB}")
    ));
    assert_eq!(
        h.identities
            .status("google", GOOGLE_SUB)
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
    // Link via a first login.
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
            .any(|e| e.event == "link" && e.provider == "google")
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
