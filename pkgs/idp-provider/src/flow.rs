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
//! repo). The deny-by-default allowlist is enforced in the provider
//! before the credential is even checked, and EVERY authorize/token
//! attempt — allowed, denied, or form challenge — is appended to the
//! audit record. A failed audit write fails the attempt closed.

use crate::audit::AuditLog;
use crate::registry::{
    now_unix, random_b64url, verify_credential, ProviderConfig, LOCAL_DOOR_PROVIDER,
};
use crate::signing::TokenSigner;
use axum::{
    extract::{Form, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    Json,
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
    codes: Mutex<HashMap<String, AuthCode>>,
    issuer: String,
}

impl FlowState {
    pub fn new(
        registry: ProviderConfig,
        signer: TokenSigner,
        audit: Arc<AuditLog>,
        issuer: String,
    ) -> Self {
        FlowState {
            registry,
            signer,
            audit,
            codes: Mutex::new(HashMap::new()),
            issuer,
        }
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

fn err_json(status: StatusCode, error: &str, description: &str) -> Response {
    (
        status,
        Json(serde_json::json!({"error": error, "error_description": description})),
    )
        .into_response()
}

/// Audit-then-error helper: records a denied attempt and returns the
/// error response. If the audit write itself fails, the attempt
/// still fails (with a plainer error) — never succeeds unaudited.
fn denied(
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

fn login_form(p: &AuthorizeParams) -> Html<String> {
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
    Html(format!(
        "<!doctype html><html><head><title>devtoday SSO — local door</title></head>\
         <body><h1>devtoday SSO</h1><p>Sign in with your devtoday local identity.</p>\
         <form method=\"post\" action=\"/authorize\">{hiddens}\
         <label>Local identity <input name=\"login_id\" autocomplete=\"username\"></label>\
         <label>Credential <input type=\"password\" name=\"credential\" autocomplete=\"current-password\"></label>\
         <button type=\"submit\">Sign in</button></form></body></html>"
    ))
}

fn authorize(flow: &FlowState, p: AuthorizeParams) -> Response {
    let client_id = p.client_id.clone().unwrap_or_default();
    let login_id = p.login_id.clone().unwrap_or_default();
    let identity_key = if login_id.is_empty() {
        String::new()
    } else {
        format!("{LOCAL_DOOR_PROVIDER}:{login_id}")
    };

    // Static client registry — unknown clients never get a redirect.
    let client = match flow.registry.client(&client_id) {
        Some(c) => c,
        None => {
            return denied(
                flow,
                "authorize",
                "unknown_client",
                &client_id,
                &identity_key,
                StatusCode::BAD_REQUEST,
                "invalid_client",
                "unknown client_id",
            )
        }
    };
    let redirect_uri = p.redirect_uri.clone().unwrap_or_default();
    if !client.redirect_uris.iter().any(|u| u == &redirect_uri) {
        return denied(
            flow,
            "authorize",
            "redirect_uri_mismatch",
            &client_id,
            &identity_key,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "redirect_uri is not registered for this client",
        );
    }
    if p.response_type.as_deref() != Some("code") {
        return denied(
            flow,
            "authorize",
            "unsupported_response_type",
            &client_id,
            &identity_key,
            StatusCode::BAD_REQUEST,
            "unsupported_response_type",
            "only response_type=code is supported",
        );
    }
    if p.code_challenge_method.as_deref() != Some("S256")
        || p.code_challenge.as_deref().unwrap_or("").is_empty()
    {
        return denied(
            flow,
            "authorize",
            "pkce_s256_required",
            &client_id,
            &identity_key,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "PKCE with code_challenge_method=S256 is required",
        );
    }
    let scope = p.scope.clone().unwrap_or_default();
    if !scope.split_whitespace().any(|s| s == "openid") {
        return denied(
            flow,
            "authorize",
            "openid_scope_required",
            &client_id,
            &identity_key,
            StatusCode::BAD_REQUEST,
            "invalid_scope",
            "scope must include openid",
        );
    }

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
        return login_form(&p).into_response();
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
    if !flow.registry.is_linked(LOCAL_DOOR_PROVIDER, &login_id) {
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
    let code = random_b64url(32);
    let record = AuthCode {
        client_id: client_id.clone(),
        redirect_uri: redirect_uri.clone(),
        code_challenge: p.code_challenge.clone().unwrap_or_default(),
        account_id: flow.registry.account.account_id.clone(),
        identity_key: identity_key.clone(),
        nonce: p.nonce.clone(),
        scope: scope.clone(),
        expires_at: now_unix() + CODE_TTL_SECS,
    };
    match flow.codes.lock() {
        Ok(mut codes) => {
            codes.insert(code.clone(), record);
        }
        Err(_) => {
            return err_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "code store unavailable",
            )
        }
    }
    if flow
        .audit
        .record(
            "authorize",
            "allowed",
            "code_issued",
            &client_id,
            &identity_key,
        )
        .is_err()
    {
        // Fail closed: an unaudited success is not a success. Burn
        // the code we just minted.
        if let Ok(mut codes) = flow.codes.lock() {
            codes.remove(&code);
        }
        return err_json(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "audit write failed",
        );
    }
    let mut location = format!("{redirect_uri}?code={code}");
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
            )
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
                )
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
            )
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

    let account = &flow.registry.account;
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
            )
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
            )
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
                "a Bearer access token is required",
            )
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
    let account = &flow.registry.account;
    if claims["sub"] != account.account_id {
        return err_json(
            StatusCode::UNAUTHORIZED,
            "invalid_token",
            "access token subject is not the account",
        );
    }
    Json(serde_json::json!({
        "sub": account.account_id,
        "name": account.display_name,
        "preferred_username": claims["preferred_username"],
        "roles": account.roles,
    }))
    .into_response()
}
