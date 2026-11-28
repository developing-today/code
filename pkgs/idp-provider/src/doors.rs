//! Federated door handlers (#13411/#13412/#13414, linking per
//! #13415): `GET /door/{name}/start` and `GET /door/{name}/callback`.
//!
//! The shape mirrors the local door's discipline exactly:
//!
//! - `start` validates the relying party's authorize request with
//!   the same checks as `/authorize` (static client registry,
//!   redirect URI, code + PKCE S256, `openid` scope) *before* the
//!   user is sent anywhere, parks the request server-side under a
//!   random state handle, and redirects to the upstream.
//! - `callback` consumes the state handle (single-use), exchanges
//!   the upstream code, verifies the upstream identity (OIDC: the
//!   ID token's signature against the upstream JWKS plus iss/aud/
//!   exp/nonce; GitHub: the numeric user id), then enforces the
//!   deny-by-default allowlist in the provider — before any link or
//!   code exists.
//! - Linking (#13415): an allowlisted identity the store has never
//!   seen is linked to the account row by this login, with the link
//!   written to the identity history and the audit record. An
//!   identity an operator has unlinked is denied
//!   (`identity_unlinked`) — unlinking is never silently undone by
//!   a login.
//! - On success the handler mints one of *our* authorization codes
//!   (the same code store as the local door) and redirects to the
//!   relying party. Every outcome — redirect, denial, link — is
//!   audited; a failed audit write fails the attempt closed.

use crate::flow::{
    AuthorizeParams, FlowState, denied, err_json, issue_code, validate_authorize_request,
};
use crate::registry::{now_unix, random_b64url, url_encode};
use crate::upstream::{
    DoorRuntime, exchange_upstream_code, fetch_github_user, fetch_upstream_jwks,
    subject_from_claims, verify_upstream_id_token,
};
use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const PENDING_TTL_SECS: i64 = 300;

/// A relying party's authorize request, parked server-side while
/// the user is at the upstream door.
#[derive(Clone)]
pub struct PendingDoor {
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub nonce: Option<String>,
    pub scope: String,
    pub rp_state: Option<String>,
    pub door: String,
    pub upstream_nonce: Option<String>,
    pub pkce_verifier: Option<String>,
    pub expires_at: i64,
}

#[derive(Debug, Default, Deserialize)]
pub struct CallbackParams {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

fn find_door<'a>(flow: &'a FlowState, name: &str) -> Option<&'a DoorRuntime> {
    flow.doors.iter().find(|d| d.name() == name)
}

fn endpoint(name: &str) -> String {
    format!("door/{name}")
}

pub async fn door_start(
    State(st): State<crate::AppState>,
    Path(name): Path<String>,
    Query(p): Query<AuthorizeParams>,
) -> Response {
    let flow = match &st.flow {
        Some(f) => f,
        None => {
            return err_json(
                StatusCode::SERVICE_UNAVAILABLE,
                "server_error",
                "authorization flow is not configured on this provider instance",
            );
        }
    };
    let client_id = p.client_id.clone().unwrap_or_default();
    let door = match find_door(flow, &name) {
        Some(d) => d,
        None => {
            return denied(
                flow,
                &endpoint(&name),
                "unknown_door",
                &client_id,
                "",
                StatusCode::NOT_FOUND,
                "invalid_request",
                "unknown door",
            );
        }
    };
    // The relying party's request must be valid before the user is
    // sent upstream — same checks (and audit rows) as /authorize.
    let valid = match validate_authorize_request(flow, &p) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !door.enabled() {
        return denied(
            flow,
            &endpoint(&name),
            "door_not_configured",
            &client_id,
            "",
            StatusCode::SERVICE_UNAVAILABLE,
            "temporarily_unavailable",
            "this door is not configured on this provider instance",
        );
    }

    let handle = random_b64url(32);
    let is_oidc = door.config.kind == "oidc";
    let upstream_nonce = if is_oidc {
        Some(random_b64url(16))
    } else {
        None
    };
    let pkce_verifier = if is_oidc {
        Some(random_b64url(32))
    } else {
        None
    };
    let pending = PendingDoor {
        client_id: valid.client_id.clone(),
        redirect_uri: valid.redirect_uri.clone(),
        code_challenge: valid.code_challenge.clone(),
        nonce: p.nonce.clone(),
        scope: valid.scope.clone(),
        rp_state: p.state.clone(),
        door: name.clone(),
        upstream_nonce: upstream_nonce.clone(),
        pkce_verifier: pkce_verifier.clone(),
        expires_at: now_unix() + PENDING_TTL_SECS,
    };
    match flow.pending_doors.lock() {
        Ok(mut map) => {
            let now = now_unix();
            map.retain(|_, v| v.expires_at >= now);
            map.insert(handle.clone(), pending);
        }
        Err(_) => {
            return err_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "door state store unavailable",
            );
        }
    }
    let _ = flow.audit.record(
        &endpoint(&name),
        "challenge",
        "upstream_redirect",
        &client_id,
        "",
    );

    let callback = door.callback_uri(&flow.issuer);
    let mut params: Vec<(&str, String)> = vec![
        ("client_id", door.config.client_id.clone()),
        ("redirect_uri", callback),
        ("state", handle),
    ];
    if is_oidc {
        let scope = if door.config.scopes.is_empty() {
            "openid".to_string()
        } else {
            door.config.scopes.join(" ")
        };
        params.push(("response_type", "code".to_string()));
        params.push(("scope", scope));
        if let Some(nonce) = &upstream_nonce {
            params.push(("nonce", nonce.clone()));
        }
        if let Some(verifier) = &pkce_verifier {
            let challenge = crate::b64url(&Sha256::digest(verifier.as_bytes()));
            params.push(("code_challenge", challenge));
            params.push(("code_challenge_method", "S256".to_string()));
        }
    } else {
        let scope = if door.config.scopes.is_empty() {
            "read:user".to_string()
        } else {
            door.config.scopes.join(" ")
        };
        params.push(("scope", scope));
    }
    let query = params
        .iter()
        .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    let location = format!("{}?{}", door.config.authorize_url, query);
    (StatusCode::FOUND, [(header::LOCATION, location)]).into_response()
}

pub async fn door_callback(
    State(st): State<crate::AppState>,
    Path(name): Path<String>,
    Query(cb): Query<CallbackParams>,
) -> Response {
    let flow = match &st.flow {
        Some(f) => f,
        None => {
            return err_json(
                StatusCode::SERVICE_UNAVAILABLE,
                "server_error",
                "authorization flow is not configured on this provider instance",
            );
        }
    };
    let door = match find_door(flow, &name) {
        Some(d) => d.clone(),
        None => {
            return denied(
                flow,
                &endpoint(&name),
                "unknown_door",
                "",
                "",
                StatusCode::NOT_FOUND,
                "invalid_request",
                "unknown door",
            );
        }
    };
    if cb.error.is_some() {
        return denied(
            flow,
            &endpoint(&name),
            "upstream_error",
            "",
            "",
            StatusCode::BAD_REQUEST,
            "access_denied",
            "the upstream door returned an error",
        );
    }
    // Consume the pending request: single-use, whether the login
    // then succeeds or fails.
    let handle = cb.state.clone().unwrap_or_default();
    let pending = {
        let mut map = match flow.pending_doors.lock() {
            Ok(m) => m,
            Err(_) => {
                return err_json(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "server_error",
                    "door state store unavailable",
                );
            }
        };
        map.remove(&handle)
    };
    let pending = match pending {
        Some(p) if p.expires_at >= now_unix() && p.door == name => p,
        _ => {
            return denied(
                flow,
                &endpoint(&name),
                "unknown_or_expired_state",
                "",
                "",
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "door state is unknown, expired, or already used",
            );
        }
    };
    let client_id = pending.client_id.clone();
    let code = cb.code.clone().unwrap_or_default();
    if code.is_empty() {
        return denied(
            flow,
            &endpoint(&name),
            "missing_code",
            &client_id,
            "",
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "the upstream door returned no code",
        );
    }
    let callback_uri = door.callback_uri(&flow.issuer);
    let token_doc = match exchange_upstream_code(
        &flow.http,
        &door,
        &code,
        &callback_uri,
        pending.pkce_verifier.as_deref(),
    )
    .await
    {
        Ok(d) => d,
        Err(_) => {
            return denied(
                flow,
                &endpoint(&name),
                "upstream_exchange_failed",
                &client_id,
                "",
                StatusCode::BAD_GATEWAY,
                "server_error",
                "the upstream token exchange failed",
            );
        }
    };

    // Resolve the verified upstream identity.
    let subject = if door.config.kind == "github" {
        let access_token = token_doc["access_token"].as_str().unwrap_or("");
        if access_token.is_empty() {
            return denied(
                flow,
                &endpoint(&name),
                "upstream_no_access_token",
                &client_id,
                "",
                StatusCode::BAD_GATEWAY,
                "server_error",
                "the upstream token exchange returned no access token",
            );
        }
        match fetch_github_user(&flow.http, &door, access_token).await {
            Ok(user) => subject_from_claims(&door, &user),
            Err(_) => {
                return denied(
                    flow,
                    &endpoint(&name),
                    "upstream_user_fetch_failed",
                    &client_id,
                    "",
                    StatusCode::BAD_GATEWAY,
                    "server_error",
                    "the upstream user fetch failed",
                );
            }
        }
    } else {
        let id_token = token_doc["id_token"].as_str().unwrap_or("");
        if id_token.is_empty() {
            return denied(
                flow,
                &endpoint(&name),
                "upstream_no_id_token",
                &client_id,
                "",
                StatusCode::BAD_GATEWAY,
                "server_error",
                "the upstream token exchange returned no ID token",
            );
        }
        let jwks_uri = door.config.jwks_uri.clone().unwrap_or_default();
        let jwks = match fetch_upstream_jwks(&flow.http, &jwks_uri).await {
            Ok(j) => j,
            Err(_) => {
                return denied(
                    flow,
                    &endpoint(&name),
                    "upstream_jwks_failed",
                    &client_id,
                    "",
                    StatusCode::BAD_GATEWAY,
                    "server_error",
                    "the upstream JWKS fetch failed",
                );
            }
        };
        match verify_upstream_id_token(
            id_token,
            &jwks,
            &door,
            pending.upstream_nonce.as_deref(),
            now_unix(),
        ) {
            Ok(claims) => subject_from_claims(&door, &claims),
            Err(_) => {
                return denied(
                    flow,
                    &endpoint(&name),
                    "upstream_token_invalid",
                    &client_id,
                    "",
                    StatusCode::FORBIDDEN,
                    "access_denied",
                    "the upstream ID token did not verify",
                );
            }
        }
    };
    let subject = match subject {
        Some(s) if !s.is_empty() => s,
        _ => {
            return denied(
                flow,
                &endpoint(&name),
                "upstream_no_subject",
                &client_id,
                "",
                StatusCode::FORBIDDEN,
                "access_denied",
                "the upstream identity has no subject",
            );
        }
    };
    let identity_key = format!("{}:{}", door.identity_provider(), subject);

    // Deny-by-default gate, enforced in the provider, before any
    // link or code exists.
    if !flow.registry.is_allowlisted(&identity_key) {
        return denied(
            flow,
            &endpoint(&name),
            "not_allowlisted",
            &client_id,
            &identity_key,
            StatusCode::FORBIDDEN,
            "access_denied",
            "identity is not on the allowlist",
        );
    }

    // Link state (#13415/#13972): active proceeds; pending is denied
    // without issuing a code; unlinked is denied (an operator's unlink
    // is never silently undone); never-seen is linked by this login,
    // with the event in the identity history and the audit record.
    match flow.identities.status(&name, &subject) {
        Ok(Some(ref s)) if s == "active" => {}
        Ok(Some(ref s)) if s == "pending" => {
            return denied(
                flow,
                &endpoint(&name),
                "identity_pending",
                &client_id,
                &identity_key,
                StatusCode::FORBIDDEN,
                "access_denied",
                "identity is registered and awaiting approval",
            );
        }
        Ok(Some(_)) => {
            return denied(
                flow,
                &endpoint(&name),
                "identity_unlinked",
                &client_id,
                &identity_key,
                StatusCode::FORBIDDEN,
                "access_denied",
                "identity is not linked to an account",
            );
        }
        Ok(None) => {
            let account_id = flow.registry.account.account_id.clone();
            match flow
                .identities
                .link(&name, &subject, &account_id, "door-login")
            {
                Ok(event) if event != "none" => {
                    if flow
                        .audit
                        .record("link", "allowed", event, &client_id, &identity_key)
                        .is_err()
                    {
                        return err_json(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "server_error",
                            "audit write failed",
                        );
                    }
                }
                Ok(_) => {}
                Err(_) => {
                    return err_json(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "server_error",
                        "identity link failed",
                    );
                }
            }
        }
        Err(_) => {
            return err_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "identity store unavailable",
            );
        }
    }

    // Success: mint one of our own codes, bound to the relying
    // party's original request, and hand the user back.
    let code = match issue_code(
        flow,
        &pending.client_id,
        &pending.redirect_uri,
        &pending.code_challenge,
        &identity_key,
        pending.nonce.as_deref(),
        &pending.scope,
        &endpoint(&name),
    ) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let mut location = format!("{}?code={code}", pending.redirect_uri);
    if let Some(state) = &pending.rp_state {
        location.push_str(&format!("&state={state}"));
    }
    (StatusCode::FOUND, [(header::LOCATION, location)]).into_response()
}
