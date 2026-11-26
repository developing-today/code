//! The authorization-code + PKCE flow: `GET/POST /authorize`,
//! `POST /token`, `GET /userinfo`.
//!
//! Shape (Slice 3 plan, #13409): authorization code + PKCE (S256
//! only). No password grant, no implicit flow, no dynamic client
//! registration — clients are the static registry in the provider
//! config. The stand-up door is the design's *devtoday local*
//! identity: `GET /authorize` with valid client parameters and no
//! credential serves a small login form; the form POSTs back to
//! `/authorize` with the local credential, which is verified against
//! the PBKDF2 hash in the on-host provider config (never in the
//! repo). The form also lists the configured federated doors
//! (#13411/#13412/#13414), which start at `/door/{name}/start` and
//! are handled in `doors.rs`.
//!
//! The deny-by-default allowlist is enforced in the provider before
//! the credential is even checked, and EVERY authorize/token
//! attempt — allowed, denied, or form challenge — is appended to
//! the audit record. A failed audit write fails the attempt closed.
//! Link state lives in the identity store (#13415): the local door
//! checks it exactly like a federated door does.

use crate::audit::AuditLog;
use crate::doors::PendingDoor;
use crate::identities::IdentityStore;
use crate::registry::{
    LOCAL_DOOR_PROVIDER, ProviderConfig, now_unix, random_b64url, url_encode, verify_credential,
};
use crate::signing::TokenSigner;
use crate::upstream::DoorRuntime;
use axum::{
    Json,
    extract::{Form, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const CODE_TTL_SECS: i64 = 120;
pub const TOKEN_TTL_SECS: i64 = 300;

#[derive(Clone)]
struct AuthCode {
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    account_id: String,
    identity_key: String,
    nonce: Option<String>,
    scope: String,
    expires_at: i64,
}

pub struct FlowState {
    pub registry: ProviderConfig,
    pub signer: TokenSigner,
    pub audit: Arc<AuditLog>,
    /// Link state for the account's identities (#13415), seeded
    /// from the registry config at construction.
    pub identities: Arc<IdentityStore>,
    /// Configured federated doors (empty until doors are
    /// registered and configured on-host).
    pub doors: Vec<DoorRuntime>,
    /// HTTP client for upstream door calls.
    pub http: reqwest::Client,
    /// Parked federated-door authorize requests, by state handle.
    pub pending_doors: Mutex<HashMap<String, PendingDoor>>,
    codes: Mutex<HashMap<String, AuthCode>>,
    pub issuer: String,
}

/// An account tokens can be issued for, resolved by id: the
/// config account (the owner's), or an enrolled account from the
/// identity store (#15249).
pub struct ResolvedAccount {
    pub account_id: String,
    pub display_name: String,
    pub roles: Vec<String>,
}

impl FlowState {
    pub fn new(
        registry: ProviderConfig,
        signer: TokenSigner,
        audit: Arc<AuditLog>,
        identities: Arc<IdentityStore>,
        doors: Vec<DoorRuntime>,
        issuer: String,
    ) -> Self {
        // The config's identity list is the store's seed (#13415);
        // after seeding, the store is authoritative.
        let _ = identities.seed(&registry.account.account_id, &registry.account.identities);
        let _ = identities.seed_account(&registry.account);
        FlowState {
            registry,
            signer,
            audit,
            identities,
            doors,
            http: reqwest::Client::new(),
            pending_doors: Mutex::new(HashMap::new()),
            codes: Mutex::new(HashMap::new()),
            issuer,
        }
    }

    /// Resolve an account id to the row tokens are issued for:
    /// the config account when the id is the owner's, otherwise
    /// the enrolled account in the identity store (#15249).
    /// `None` for an id this provider has never heard of — token
    /// issuance and userinfo fail closed on it.
    pub fn account_by_id(&self, account_id: &str) -> Option<ResolvedAccount> {
        if account_id == self.registry.account.account_id {
            return Some(ResolvedAccount {
                account_id: account_id.to_string(),
                display_name: self.registry.account.display_name.clone(),
                roles: self.registry.account.roles.clone(),
            });
        }
        self.identities
            .account(account_id)
            .ok()
            .flatten()
            .map(|row| ResolvedAccount {
                account_id: row.account_id,
                display_name: row.display_name,
                roles: row.roles,
            })
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct AuthorizeParams {
    pub response_type: Option<String>,
    pub client_id: Option<String>,
    pub redirect_uri: Option<String>,
    pub scope: Option<String>,
    pub state: Option<String>,
    pub nonce: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    /// Local-door fields (the login form posts these back).
    pub login_id: Option<String>,
    pub credential: Option<String>,
}

pub(crate) fn err_json(status: StatusCode, error: &str, description: &str) -> Response {
    (
        status,
        Json(serde_json::json!({"error": error, "error_description": description})),
    )
        .into_response()
}

/// Audit-then-error helper: records a denied attempt and returns the
/// error response. If the audit write itself fails, the attempt
/// still fails (with a plainer error) — never succeeds unaudited.
pub(crate) fn denied(
    flow: &FlowState,
    endpoint: &str,
    reason: &str,
    client_id: &str,
    identity: &str,
    status: StatusCode,
    error: &str,
    description: &str,
) -> Response {
    let _ = flow
        .audit
        .record(endpoint, "denied", reason, client_id, identity);
    err_json(status, error, description)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A relying-party authorize request that passed the static checks
/// shared by every door: known client, registered redirect URI,
/// `response_type=code`, PKCE S256, `openid` scope.
pub struct ValidatedAuthorize {
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub scope: String,
}

/// The static authorize-request checks, in the local door's order,
/// with the local door's audit rows. Used by `/authorize` itself
/// and by the federated doors' start handler — a request that
/// would be denied here is denied before the user is sent upstream.
pub(crate) fn validate_authorize_request(
    flow: &FlowState,
    p: &AuthorizeParams,
) -> Result<ValidatedAuthorize, Response> {
    let client_id = p.client_id.clone().unwrap_or_default();
    let login_id = p.login_id.clone().unwrap_or_default();
    let identity_key = if login_id.is_empty() {
        String::new()
    } else {
        format!("{LOCAL_DOOR_PROVIDER}:{login_id}")
    };

    let client = match flow.registry.client(&client_id) {
        Some(c) => c,
        None => {
            return Err(denied(
                flow,
                "authorize",
                "unknown_client",
                &client_id,
                &identity_key,
                StatusCode::BAD_REQUEST,
                "invalid_client",
                "unknown client_id",
            ));
        }
    };
    let redirect_uri = p.redirect_uri.clone().unwrap_or_default();
    if !client.redirect_uris.iter().any(|u| u == &redirect_uri) {
        return Err(denied(
            flow,
            "authorize",
            "redirect_uri_mismatch",
            &client_id,
            &identity_key,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "redirect_uri is not registered for this client",
        ));
    }
    if p.response_type.as_deref() != Some("code") {
        return Err(denied(
            flow,
            "authorize",
            "unsupported_response_type",
            &client_id,
            &identity_key,
            StatusCode::BAD_REQUEST,
            "unsupported_response_type",
            "only response_type=code is supported",
        ));
    }
    if p.code_challenge_method.as_deref() != Some("S256")
        || p.code_challenge.as_deref().unwrap_or("").is_empty()
    {
        return Err(denied(
            flow,
            "authorize",
            "pkce_s256_required",
            &client_id,
            &identity_key,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "PKCE with code_challenge_method=S256 is required",
        ));
    }
    let scope = p.scope.clone().unwrap_or_default();
    if !scope.split_whitespace().any(|s| s == "openid") {
        return Err(denied(
            flow,
            "authorize",
            "openid_scope_required",
            &client_id,
            &identity_key,
            StatusCode::BAD_REQUEST,
            "invalid_scope",
            "scope must include openid",
        ));
    }
    Ok(ValidatedAuthorize {
        client_id,
        redirect_uri,
        code_challenge: p.code_challenge.clone().unwrap_or_default(),
        scope,
    })
}

/// Mint a single-use authorization code bound to an already-gated
/// identity and the account that identity resolved to (the config
/// account for the owner's declared identities, the enrolled
/// account for a first-login enrollment, #15249), and audit the
/// issuance under `audit_endpoint` (`authorize` for the local
/// door, `door/{name}` for a federated door). A failed audit write
/// burns the code and fails closed.
pub(crate) fn issue_code(
    flow: &FlowState,
    client_id: &str,
    redirect_uri: &str,
    code_challenge: &str,
    account_id: &str,
    identity_key: &str,
    nonce: Option<&str>,
    scope: &str,
    audit_endpoint: &str,
) -> Result<String, Response> {
    let code = random_b64url(32);
    let record = AuthCode {
        client_id: client_id.to_string(),
        redirect_uri: redirect_uri.to_string(),
        code_challenge: code_challenge.to_string(),
        account_id: account_id.to_string(),
        identity_key: identity_key.to_string(),
        nonce: nonce.map(|s| s.to_string()),
        scope: scope.to_string(),
        expires_at: now_unix() + CODE_TTL_SECS,
    };
    match flow.codes.lock() {
        Ok(mut codes) => {
            codes.insert(code.clone(), record);
        }
        Err(_) => {
            return Err(err_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "code store unavailable",
            ));
        }
    }
    if flow
        .audit
        .record(
            audit_endpoint,
            "allowed",
            "code_issued",
            client_id,
            identity_key,
        )
        .is_err()
    {
        // Fail closed: an unaudited success is not a success. Burn
        // the code we just minted.
        if let Ok(mut codes) = flow.codes.lock() {
            codes.remove(&code);
        }
        return Err(err_json(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "audit write failed",
        ));
    }
    Ok(code)
}

fn login_form(flow: &FlowState, p: &AuthorizeParams) -> Html<String> {
    let hidden = |name: &str, value: &Option<String>| -> String {
        match value {
            Some(v) => format!(
                "<input type=\"hidden\" name=\"{name}\" value=\"{}\">",
                html_escape(v)
            ),
            None => String::new(),
        }
    };
    let hiddens = [
        hidden("response_type", &p.response_type),
        hidden("client_id", &p.client_id),
        hidden("redirect_uri", &p.redirect_uri),
        hidden("scope", &p.scope),
        hidden("state", &p.state),
        hidden("nonce", &p.nonce),
        hidden("code_challenge", &p.code_challenge),
        hidden("code_challenge_method", &p.code_challenge_method),
    ]
    .concat();
    // Federated doors: one link per enabled door, carrying the
    // relying party's authorize parameters to the door's start
    // endpoint (which re-validates them before redirecting).
    let door_query = [
        ("response_type", &p.response_type),
        ("client_id", &p.client_id),
        ("redirect_uri", &p.redirect_uri),
        ("scope", &p.scope),
        ("state", &p.state),
        ("nonce", &p.nonce),
        ("code_challenge", &p.code_challenge),
        ("code_challenge_method", &p.code_challenge_method),
    ]
    .iter()
    .filter_map(|(k, v)| {
        v.as_ref()
            .map(|v| format!("{}={}", url_encode(k), url_encode(v)))
    })
    .collect::<Vec<_>>()
    .join("&");
    let door_links = flow
        .doors
        .iter()
        .filter(|d| d.enabled())
        .map(|d| {
            let label = if d.config.display_name.is_empty() {
                d.name().to_string()
            } else {
                d.config.display_name.clone()
            };
            format!(
                "<li><a href=\"/door/{}/start?{}\">Sign in with {}</a></li>",
                html_escape(d.name()),
                html_escape(&door_query),
                html_escape(&label)
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let doors_block = if door_links.is_empty() {
        String::new()
    } else {
        format!("<p>Or sign in with a linked door:</p><ul>{door_links}</ul>")
    };
    Html(format!(
        "<!doctype html><html><head><title>devtoday SSO — local door</title></head>\
         <body><h1>devtoday SSO</h1><p>Sign in with your devtoday local identity.</p>\
         <form method=\"post\" action=\"/authorize\">{hiddens}\
         <label>Local identity <input name=\"login_id\" autocomplete=\"username\"></label>\
         <label>Credential <input type=\"password\" name=\"credential\" autocomplete=\"current-password\"></label>\
         <button type=\"submit\">Sign in</button></form>{doors_block}</body></html>"
    ))
}

fn authorize(flow: &FlowState, p: AuthorizeParams) -> Response {
    let valid = match validate_authorize_request(flow, &p) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let client_id = valid.client_id.clone();
    let login_id = p.login_id.clone().unwrap_or_default();
    let identity_key = if login_id.is_empty() {
        String::new()
    } else {
        format!("{LOCAL_DOOR_PROVIDER}:{login_id}")
    };

    // No credential presented yet: serve the local-door login form.
    // This is a challenge, not an authentication attempt outcome.
    if p.login_id.is_none() || p.credential.is_none() {
        let _ = flow.audit.record(
            "authorize",
            "challenge",
            "login_form_served",
            &client_id,
            "",
        );
        return login_form(flow, &p).into_response();
    }

    // Deny-by-default gate, enforced in the provider, BEFORE the
    // credential is checked: only the owner's allowlisted identity
    // keys pass, and the identity must be linked to the account row.
    if !flow.registry.is_allowlisted(&identity_key) {
        return denied(
            flow,
            "authorize",
            "not_allowlisted",
            &client_id,
            &identity_key,
            StatusCode::FORBIDDEN,
            "access_denied",
            "identity is not on the allowlist",
        );
    }
    // The local door serves only the config account: its credential
    // is that account's credential, so the identity must be actively
    // linked to THAT account (an identity enrolled into its own
    // account at a federated door, #15249, does not open here).
    let account_id = match flow
        .identities
        .account_id_for(LOCAL_DOOR_PROVIDER, &login_id)
    {
        Ok(Some(id))
            if id == flow.registry.account.account_id
                && flow.identities.is_active(LOCAL_DOOR_PROVIDER, &login_id) =>
        {
            id
        }
        _ => {
            return denied(
                flow,
                "authorize",
                "identity_not_linked",
                &client_id,
                &identity_key,
                StatusCode::FORBIDDEN,
                "access_denied",
                "identity is not linked to an account",
            );
        }
    };
    if !verify_credential(
        p.credential.as_deref().unwrap_or(""),
        &flow.registry.account.local_credential_hash,
    ) {
        return denied(
            flow,
            "authorize",
            "bad_credential",
            &client_id,
            &identity_key,
            StatusCode::UNAUTHORIZED,
            "access_denied",
            "credential verification failed",
        );
    }

    // Success: mint a single-use authorization code.
    let code = match issue_code(
        flow,
        &valid.client_id,
        &valid.redirect_uri,
        &valid.code_challenge,
        &account_id,
        &identity_key,
        p.nonce.as_deref(),
        &valid.scope,
        "authorize",
    ) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let mut location = format!("{}?code={code}", valid.redirect_uri);
    if let Some(state) = &p.state {
        location.push_str(&format!("&state={state}"));
    }
    (StatusCode::FOUND, [(header::LOCATION, location)]).into_response()
}

pub async fn authorize_get(
    State(st): State<crate::AppState>,
    Query(p): Query<AuthorizeParams>,
) -> Response {
    match &st.flow {
        Some(flow) => authorize(flow, p),
        None => err_json(
            StatusCode::SERVICE_UNAVAILABLE,
            "server_error",
            "authorization flow is not configured on this provider instance",
        ),
    }
}

pub async fn authorize_post(
    State(st): State<crate::AppState>,
    Form(p): Form<AuthorizeParams>,
) -> Response {
    match &st.flow {
        Some(flow) => authorize(flow, p),
        None => err_json(
            StatusCode::SERVICE_UNAVAILABLE,
            "server_error",
            "authorization flow is not configured on this provider instance",
        ),
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct TokenParams {
    pub grant_type: Option<String>,
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub client_id: Option<String>,
    pub code_verifier: Option<String>,
}

pub async fn token(State(st): State<crate::AppState>, Form(p): Form<TokenParams>) -> Response {
    let flow = match &st.flow {
        Some(f) => f,
        None => {
            return err_json(
                StatusCode::SERVICE_UNAVAILABLE,
                "server_error",
                "token flow is not configured on this provider instance",
            );
        }
    };
    let client_id = p.client_id.clone().unwrap_or_default();
    if p.grant_type.as_deref() != Some("authorization_code") {
        return denied(
            flow,
            "token",
            "unsupported_grant_type",
            &client_id,
            "",
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "only grant_type=authorization_code is supported",
        );
    }
    let code = p.code.clone().unwrap_or_default();
    // Single use: the code is consumed by its first presentation,
    // whether the exchange then succeeds or fails.
    let record = {
        let mut codes = match flow.codes.lock() {
            Ok(c) => c,
            Err(_) => {
                return err_json(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "server_error",
                    "code store unavailable",
                );
            }
        };
        codes.remove(&code)
    };
    let record = match record {
        Some(r) if r.expires_at >= now_unix() => r,
        _ => {
            return denied(
                flow,
                "token",
                "unknown_or_expired_code",
                &client_id,
                "",
                StatusCode::BAD_REQUEST,
                "invalid_grant",
                "authorization code is unknown, expired, or already used",
            );
        }
    };
    let identity = record.identity_key.clone();
    if record.client_id != client_id {
        return denied(
            flow,
            "token",
            "client_mismatch",
            &client_id,
            &identity,
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "code was issued to a different client",
        );
    }
    if record.redirect_uri != p.redirect_uri.clone().unwrap_or_default() {
        return denied(
            flow,
            "token",
            "redirect_uri_mismatch",
            &client_id,
            &identity,
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "redirect_uri does not match the authorization request",
        );
    }
    // PKCE S256: base64url(SHA-256(code_verifier)) == code_challenge.
    let verifier = p.code_verifier.clone().unwrap_or_default();
    let computed = crate::b64url(&Sha256::digest(verifier.as_bytes()));
    if computed != record.code_challenge {
        return denied(
            flow,
            "token",
            "pkce_verification_failed",
            &client_id,
            &identity,
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "PKCE code_verifier does not match code_challenge",
        );
    }

    // The code names its account: tokens carry THAT account's
    // roles, never the config account's by default (#15249).
    let account = match flow.account_by_id(&record.account_id) {
        Some(a) => a,
        None => {
            return denied(
                flow,
                "token",
                "account_unknown",
                &client_id,
                &identity,
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "the code's account is not known to this provider",
            );
        }
    };
    let iat = now_unix();
    let exp = iat + TOKEN_TTL_SECS;
    let mut id_claims = serde_json::json!({
        "iss": flow.issuer,
        "sub": record.account_id,
        "aud": client_id,
        "iat": iat,
        "exp": exp,
        "auth_time": iat,
        "name": account.display_name,
        "preferred_username": identity.clone(),
        "roles": account.roles,
        "token_use": "id",
    });
    if let Some(nonce) = &record.nonce {
        id_claims["nonce"] = serde_json::Value::String(nonce.clone());
    }
    let access_claims = serde_json::json!({
        "iss": flow.issuer,
        "sub": account.account_id,
        "aud": client_id,
        "iat": iat,
        "exp": exp,
        "scope": record.scope,
        "roles": account.roles,
        "preferred_username": identity,
        "token_use": "access",
        "jti": random_b64url(16),
    });
    let (id_token, access_token) = match (
        flow.signer.sign_jwt(&id_claims),
        flow.signer.sign_jwt(&access_claims),
    ) {
        (Ok(a), Ok(b)) => (a, b),
        _ => {
            return denied(
                flow,
                "token",
                "signing_failed",
                &client_id,
                &identity,
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "token signing failed",
            );
        }
    };
    if flow
        .audit
        .record("token", "allowed", "tokens_issued", &client_id, &identity)
        .is_err()
    {
        return err_json(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "audit write failed",
        );
    }
    Json(serde_json::json!({
        "access_token": access_token,
        "id_token": id_token,
        "token_type": "Bearer",
        "expires_in": TOKEN_TTL_SECS,
        "scope": record.scope,
    }))
    .into_response()
}

pub async fn userinfo(State(st): State<crate::AppState>, headers: HeaderMap) -> Response {
    let flow = match &st.flow {
        Some(f) => f,
        None => {
            return err_json(
                StatusCode::SERVICE_UNAVAILABLE,
                "server_error",
                "userinfo is not configured on this provider instance",
            );
        }
    };
    let token_value = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let token_value = match token_value {
        Some(t) => t,
        None => {
            return err_json(
                StatusCode::UNAUTHORIZED,
                "invalid_token",
                "a Bearer <redacted> token is required",
            );
        }
    };
    let claims = match flow.signer.verify_jwt(token_value) {
        Ok(c) => c,
        Err(e) => return err_json(StatusCode::UNAUTHORIZED, "invalid_token", &e),
    };
    if claims["iss"] != flow.issuer
        || claims["token_use"] != "access"
        || claims["exp"].as_i64().unwrap_or(0) < now_unix()
    {
        return err_json(
            StatusCode::UNAUTHORIZED,
            "invalid_token",
            "access token is expired or not valid for this issuer",
        );
    }
    // The token's subject names its account (the owner's, or an
    // enrolled account, #15249); an unknown subject is not valid
    // for this provider.
    let subject = claims["sub"].as_str().unwrap_or("");
    let account = match flow.account_by_id(subject) {
        Some(a) => a,
        None => {
            return err_json(
                StatusCode::UNAUTHORIZED,
                "invalid_token",
                "access token subject is not a known account",
            );
        }
    };
    Json(serde_json::json!({
        "sub": account.account_id,
        "name": account.display_name,
        "preferred_username": claims["preferred_username"],
        "roles": account.roles,
    }))
    .into_response()
}
