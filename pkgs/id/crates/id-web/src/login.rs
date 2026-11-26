//! Login initiation routes (#13710). The OIDC start route creates the
//! authorization-code + PKCE request and retains its verifier server-side
//! for the one-time callback exchange in the next slice.

use std::collections::HashMap;
use std::env;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use id_core::oidc_login::{AuthorizationRequest, Pkce};

use crate::AppState;

/// Maximum live OIDC attempts held by one serving process.
pub const MAX_PENDING_OIDC_LOGINS: usize = 4096;
/// Authorization attempts expire after ten minutes.
pub const OIDC_LOGIN_TTL_SECS: u64 = 10 * 60;

const AUTHORIZATION_ENDPOINT_ENV: &str = "ID_OIDC_AUTHORIZATION_ENDPOINT";
const CLIENT_ID_ENV: &str = "ID_OIDC_CLIENT_ID";
const REDIRECT_URI_ENV: &str = "ID_OIDC_REDIRECT_URI";

/// Public OIDC client configuration for the authorization-code start route.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OidcLoginConfig {
    /// Provider authorization endpoint.
    pub authorization_endpoint: String,
    /// This relying party's registered client id.
    pub client_id: String,
    /// This server's registered callback URI.
    pub redirect_uri: String,
}

impl OidcLoginConfig {
    /// Validates the public values required to start an OIDC flow.
    pub fn new(
        authorization_endpoint: impl Into<String>,
        client_id: impl Into<String>,
        redirect_uri: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let authorization_endpoint = authorization_endpoint.into();
        let client_id = client_id.into();
        let redirect_uri = redirect_uri.into();
        if !authorization_endpoint.starts_with("https://") {
            anyhow::bail!("OIDC authorization endpoint must use HTTPS");
        }
        if client_id.trim().is_empty() {
            anyhow::bail!("OIDC client id must not be empty");
        }
        if !(redirect_uri.starts_with("https://")
            || redirect_uri.starts_with("http://localhost:")
            || redirect_uri.starts_with("http://127.0.0.1:"))
        {
            anyhow::bail!("OIDC redirect URI must use HTTPS or loopback HTTP");
        }
        Ok(Self {
            authorization_endpoint,
            client_id,
            redirect_uri,
        })
    }
}

/// Pending OIDC callback state. The verifier is never returned to a client.
pub struct PendingOidcLogin {
    nonce: String,
    pkce: Pkce,
    expires_at: u64,
    client_id: String,
    redirect_uri: String,
}

impl PendingOidcLogin {
    /// ID Token nonce expected from the provider.
    pub fn nonce(&self) -> &str {
        &self.nonce
    }

    /// PKCE verifier for the server-side token exchange.
    pub fn code_verifier(&self) -> &str {
        self.pkce.verifier()
    }

    /// Client id used to begin the flow.
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// Registered callback URI used to begin the flow.
    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }
}

/// Per-serving-process OIDC start state. Pending attempts are bounded,
/// expire, and can be consumed only once by the callback route.
#[derive(Clone, Default)]
pub struct OidcLoginState {
    config: Option<OidcLoginConfig>,
    pending: Arc<Mutex<HashMap<String, PendingOidcLogin>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OidcStartError {
    NotConfigured,
    AtCapacity,
    StateUnavailable,
}

impl OidcLoginState {
    /// OIDC is disabled until a complete provider configuration is supplied.
    pub fn disabled() -> Self {
        Self::default()
    }

    /// Configure the OIDC start route with an authorization endpoint and callback.
    pub fn with_config(config: OidcLoginConfig) -> Self {
        Self {
            config: Some(config),
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Read public OIDC client configuration from the serving process environment.
    /// All three variables must be present together; no secret is read or logged.
    pub fn from_env() -> anyhow::Result<Self> {
        let endpoint = env::var(AUTHORIZATION_ENDPOINT_ENV).ok();
        let client_id = env::var(CLIENT_ID_ENV).ok();
        let redirect_uri = env::var(REDIRECT_URI_ENV).ok();
        match (endpoint, client_id, redirect_uri) {
            (None, None, None) => Ok(Self::disabled()),
            (Some(endpoint), Some(client_id), Some(redirect_uri)) => Ok(Self::with_config(
                OidcLoginConfig::new(endpoint, client_id, redirect_uri)?,
            )),
            _ => anyhow::bail!(
                "configure {AUTHORIZATION_ENDPOINT_ENV}, {CLIENT_ID_ENV}, and {REDIRECT_URI_ENV} together"
            ),
        }
    }

    /// Whether a provider is configured.
    pub fn is_configured(&self) -> bool {
        self.config.is_some()
    }

    fn authorization_url(&self) -> Result<String, OidcStartError> {
        let config = self.config.as_ref().ok_or(OidcStartError::NotConfigured)?;
        let pkce = Pkce::generate();
        let request = AuthorizationRequest::new(
            &config.authorization_endpoint,
            &config.client_id,
            &config.redirect_uri,
            &pkce,
        );
        let state = request.state.clone();
        let now = now_seconds();
        let pending = PendingOidcLogin {
            nonce: request.nonce.clone(),
            pkce,
            expires_at: now.saturating_add(OIDC_LOGIN_TTL_SECS),
            client_id: config.client_id.clone(),
            redirect_uri: config.redirect_uri.clone(),
        };
        let mut entries = self
            .pending
            .lock()
            .map_err(|_| OidcStartError::StateUnavailable)?;
        entries.retain(|_, attempt| attempt.expires_at > now);
        if entries.len() >= MAX_PENDING_OIDC_LOGINS || entries.contains_key(&state) {
            return Err(OidcStartError::AtCapacity);
        }
        entries.insert(state, pending);
        Ok(request.url())
    }

    /// Consume one pending callback context. Unknown, expired, and repeated
    /// states all return `None`.
    pub fn take_pending(&self, state: &str) -> Option<PendingOidcLogin> {
        let now = now_seconds();
        let mut entries = self.pending.lock().ok()?;
        let pending = entries.remove(state)?;
        (pending.expires_at > now).then_some(pending)
    }
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

/// Routes for login initiation. A missing provider configuration is an
/// observable 503, never a route that silently pretends login is enabled.
pub fn oidc_login_routes() -> Router<AppState> {
    Router::new().route("/api/login/oidc/start", get(oidc_start_handler))
}

async fn oidc_start_handler(State(state): State<AppState>) -> Response {
    match state.oidc_login.authorization_url() {
        Ok(url) => {
            let mut response = Redirect::temporary(&url).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(OidcStartError::NotConfigured) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error":"oidc_login_not_configured"})),
        )
            .into_response(),
        Err(OidcStartError::AtCapacity) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error":"oidc_login_capacity"})),
        )
            .into_response(),
        Err(OidcStartError::StateUnavailable) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error":"oidc_login_unavailable"})),
        )
            .into_response(),
    }
}
