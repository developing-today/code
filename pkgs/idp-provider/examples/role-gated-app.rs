//! Reference consumer for the minimal IdP (#13416): the
//! smallest complete application that takes the provider's
//! tokens seriously.
//!
//! - `GET /read` — any valid access token.
//! - `POST /write` — a valid access token carrying a write role
//!   (`owner` or `orchestrator`); enforcement is server-side, on
//!   every request, against the verified token.
//!
//! Configuration is environment-only, per the integration
//! pattern in the crate README:
//!
//! - `APP_ISSUER` — the IdP issuer URL (default
//!   `http://127.0.0.1:8911`). Discovery + JWKS are fetched from
//!   it at startup; verification after that is offline.
//! - `APP_BIND` — listen address (default `127.0.0.1:8913`).
//!
//! Run it next to a live provider and drive it with any token
//! the provider mints (the test-rp example completes a loop and
//! userinfo shows the claims; curl works too).

use idp_provider::verify::{reference_app, TokenVerifier};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let issuer =
        std::env::var("APP_ISSUER").unwrap_or_else(|_| "http://127.0.0.1:8911".to_string());
    let bind = std::env::var("APP_BIND").unwrap_or_else(|_| "127.0.0.1:8913".to_string());
    let verifier = TokenVerifier::from_issuer(&issuer)
        .await
        .unwrap_or_else(|e| panic!("cannot build verifier from {issuer}: {e}"));
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .unwrap_or_else(|e| panic!("cannot bind {bind}: {e}"));
    println!("role-gated-app listening on {bind}, issuer {issuer}");
    axum::serve(listener, reference_app(Arc::new(verifier)))
        .await
        .expect("server error");
}
