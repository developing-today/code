//! Seitan admission on the serve web surface (#8487, design recorded on
//! the issue by task 14857).
//!
//! Two routes, registered by [`super::routes`]:
//!
//! - `POST /api/seitan/invite` — registration, gated by the same
//!   admin-secret check that gates `POST /api/world/invite` (the hub's
//!   admin token, consulted through the same lease + `admin_ok` flow).
//!   The body names the token, the one world the invite joins, and its
//!   expiry; the server builds the [`Invite`] and keeps it in the
//!   registry. The response carries only the invite ID — the token is
//!   never stored (an `Invite` keeps only ID + public key + expiry, by
//!   construction) and never logged.
//! - `POST /api/seitan/admit` — unauthenticated; the signed proof is
//!   the credential. The body is `{message, signature}`, both hex (the
//!   router's existing encoding for keys and signatures). Handler
//!   order matters, per the design: checks that do not consume the
//!   invite (parse, registry lookup, the directory account check) run
//!   before [`Invite::admit`], and on success the `AccountVerified`
//!   entry is appended to the server's directory before the 200
//!   response is returned.
//!
//! The registry is v1 in-memory state beside the server config: a
//! restart drops unredeemed invites, which then refuse as unknown.
//! Invites are short-lived (expiry plus a minutes-scale freshness
//! window) and re-registration is one admin call, so persistence is a
//! named follow-up only if operations ever need it.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use crate::directory::{Actor, Directory};
use crate::seitan::{Invite, Proof, Reject, Token};
use crate::world_hub::ResolveError;

use super::AppState;

/// One registered invite: the world it joins, bound at registration
/// and never taken from the proof, plus the invite itself.
#[derive(Debug)]
struct RegisteredInvite {
    world: String,
    invite: Invite,
}

#[derive(Debug)]
struct SeitanInner {
    invites: HashMap<String, RegisteredInvite>,
    /// The server's directory: the accounts an admitted `uid` must
    /// name, and the journal an admission appends `AccountVerified`
    /// to. In `id serve` this is the directory beside the groups log
    /// (the server-config directory journal); the default state holds
    /// an empty in-memory one.
    directory: Directory,
    /// The freshness window every admit is checked under, resolved
    /// once by the caller (`serve_freshness_window` in serve).
    window: Option<Duration>,
}

/// The serve web surface's seitan admission state: the invite
/// registry plus the directory and freshness window admissions run
/// against. Cloneable — every clone shares the one registry, like the
/// rest of [`AppState`]'s shared handles.
#[derive(Clone)]
pub struct SeitanState {
    inner: Arc<Mutex<SeitanInner>>,
}

impl Default for SeitanState {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(SeitanInner {
                invites: HashMap::new(),
                directory: Directory::new(),
                window: None,
            })),
        }
    }
}

impl fmt::Debug for SeitanState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        f.debug_struct("SeitanState")
            .field("invites", &inner.invites.len())
            .field("window", &inner.window)
            .finish_non_exhaustive()
    }
}

impl SeitanState {
    /// Admission state over a caller-opened directory and the
    /// resolved freshness window.
    #[must_use]
    pub fn new(directory: Directory, window: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SeitanInner {
                invites: HashMap::new(),
                directory,
                window: Some(window),
            })),
        }
    }

    /// The window admits are checked under: the configured one, or
    /// [`crate::seitan::DEFAULT_WINDOW`] for the default (no-config)
    /// state, so a serve without a server config behaves exactly as
    /// before.
    #[must_use]
    pub fn window(&self) -> Duration {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .window
            .unwrap_or(crate::seitan::DEFAULT_WINDOW)
    }

    /// Whether the directory currently marks `account` verified —
    /// the visible effect of a landed admission.
    #[must_use]
    pub fn is_verified(&self, account: &str) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .directory
            .is_verified(account)
    }

    /// Create a keyed account in the admission directory. Startup
    /// provisioning already writes the journal this state opens in
    /// `id serve`; this entry point is for tests, which build the
    /// directory the way provisioning does.
    #[cfg(test)]
    pub(crate) fn sign_up_keyed(
        &self,
        key: &str,
        name: &str,
    ) -> anyhow::Result<crate::directory::DirectoryEntry> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .directory
            .sign_up_keyed(key, name)
    }
}

fn now_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

fn error_response(status: StatusCode, reason: &str) -> Response {
    (
        status,
        Json(serde_json::json!({ "error": reason, "reason": reason })),
    )
        .into_response()
}

/// Registration body: the token (shown to the joiner only), the one
/// world the invite joins, and the invite's expiry in milliseconds
/// since the Unix epoch.
#[derive(Debug, Deserialize)]
pub(crate) struct SeitanInviteRequest {
    token: String,
    world: String,
    expires_ms: u64,
}

#[derive(Debug, Serialize)]
struct SeitanInviteResponse {
    invite_id: String,
}

/// `POST /api/seitan/invite` — see the module docs.
pub(crate) async fn seitan_invite_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SeitanInviteRequest>,
) -> Response {
    let Some(hub) = state.world.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let supplied = headers
        .get("x-world-admin-token")
        .and_then(|value| value.to_str().ok());
    // The same gate as POST /api/world/invite: a lease for the named
    // world (a valid admin token may create it), then the admin check
    // itself.
    match hub.lease(Some(&request.world), supplied).await {
        Ok(_lease) => {}
        Err(ResolveError::Busy | ResolveError::TooManyWorlds) => {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    }
    if !supplied.is_some_and(|token| hub.admin_ok(token)) {
        return error_response(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let token = match Token::parse(&request.token) {
        Ok(token) => token,
        Err(error) => {
            return error_response(StatusCode::BAD_REQUEST, &format!("invalid token: {error}"));
        }
    };
    let invite = match Invite::create(&token, now_ms(), request.expires_ms) {
        Ok(invite) => invite,
        Err(error) => {
            tracing::error!("[seitan] invite derivation failed: {error:#}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };
    let invite_id = invite.id().to_hex();
    // Registration is idempotent for an invite ID the registry
    // already holds (Codex P2 on PR #8529): the ID is deterministic
    // for a token, and `Invite::create` starts with `used: None`, so
    // replacing an existing entry on a retried or duplicated
    // registration would reset a redeemed invite's single-use state
    // (and could silently rebind its world). Preserve the existing
    // entry; a genuinely new invite uses a new token.
    state
        .seitan
        .inner
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .invites
        .entry(invite_id.clone())
        .or_insert(RegisteredInvite {
            world: request.world,
            invite,
        });
    Json(SeitanInviteResponse { invite_id }).into_response()
}

/// Admit body: the signed accept message and its Ed25519 signature,
/// both hex.
#[derive(Debug, Deserialize)]
pub(crate) struct SeitanAdmitRequest {
    message: String,
    signature: String,
}

#[derive(Debug, Serialize)]
struct SeitanAdmitResponse {
    uid: String,
    eldest_seqno: u64,
    world: String,
}

/// The refusal `reason` string for a [`Reject`], per the design's
/// failure table.
fn reject_reason(reject: &Reject) -> &'static str {
    match reject {
        Reject::Malformed => "malformed",
        Reject::UnknownInvite => "unknown_invite",
        Reject::Revoked => "revoked",
        Reject::Expired => "expired",
        Reject::Used => "used",
        Reject::Stale(_) => "stale",
        Reject::FromFuture(_) => "from_future",
        Reject::BadKey => "bad_key",
        Reject::BadSignature => "bad_signature",
    }
}

/// `POST /api/seitan/admit` — see the module docs for the check
/// order and why it matters.
pub(crate) async fn seitan_admit_handler(
    State(state): State<AppState>,
    Json(request): Json<SeitanAdmitRequest>,
) -> Response {
    let (Some(message), Some(signature_bytes)) = (
        crate::world_session::decode_hex(&request.message),
        crate::world_session::decode_hex(&request.signature),
    ) else {
        return error_response(StatusCode::BAD_REQUEST, "malformed");
    };
    let Ok(signature) = <[u8; 64]>::try_from(signature_bytes.as_slice()) else {
        return error_response(StatusCode::BAD_REQUEST, "malformed");
    };
    let proof = Proof { message, signature };
    // Parse first: an unparseable proof is a 400 and names no invite,
    // so nothing else can be checked — and nothing is consumed.
    let fields = match crate::seitan::accept::parse(&proof.message) {
        Ok(fields) => fields,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "malformed"),
    };
    let window = state.seitan.window();
    let mut inner = state.seitan.inner.lock().unwrap_or_else(|e| e.into_inner());
    if !inner.invites.contains_key(&fields.invite_id.to_hex()) {
        return error_response(StatusCode::NOT_FOUND, "unknown_invite");
    }
    // The account check runs before `admit`, from the parsed fields,
    // so an unknown account does not consume the invite: the joiner
    // can retry once the account exists.
    if inner.directory.account(&fields.uid).is_none() {
        return error_response(StatusCode::FORBIDDEN, "unknown_account");
    }
    let Some(registered) = inner.invites.get_mut(&fields.invite_id.to_hex()) else {
        return error_response(StatusCode::NOT_FOUND, "unknown_invite");
    };
    let admitted = match registered.invite.admit(&proof, now_ms(), window) {
        Ok(admitted) => admitted,
        Err(reject) => {
            let reason = reject_reason(&reject);
            let status = if matches!(reject, Reject::Malformed) {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::FORBIDDEN
            };
            return error_response(status, reason);
        }
    };
    let world = registered.world.clone();
    // Append before responding: the 200 means the membership change
    // is in the directory journal. If the append fails after a
    // successful admit, the invite stays consumed in memory
    // (single-use safety wins over retry convenience, matching the
    // seitan threat notes) and the failure is a loud 500.
    if let Err(error) = inner.directory.verify_account(Actor::Server, &admitted.uid) {
        tracing::error!(
            "[seitan] admission for {} consumed its invite but the directory append failed: {error:#}",
            admitted.uid
        );
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal");
    }
    Json(SeitanAdmitResponse {
        uid: admitted.uid,
        eldest_seqno: admitted.eldest_seqno,
        world,
    })
    .into_response()
}
