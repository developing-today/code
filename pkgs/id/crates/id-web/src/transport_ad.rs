//! The `.well-known/id-transport-advertisement` endpoint (#8214
//! slice 4, implementing Decision 2.2 of the #5793 record): the
//! server's own HTTPS endpoint serves its current signed transport
//! advertisement. This is the authoritative self-hosted substrate
//! — the handler serves the minter's current bytes byte-for-byte
//! and never re-encodes, re-signs, or invents an ad: with no
//! minter wired (any state built outside `id serve`) or a subject
//! that is not this server's, the answer is 404. The signature,
//! not this endpoint, is what the client trusts (Decision 1), so
//! the handler needs no per-request authorization logic of its
//! own beyond the router-wide WebSecurity guard every route here
//! already sits behind.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    extract::{Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};

use super::AppState;

/// `GET /.well-known/id-transport-advertisement?subject=<fp>`:
/// serve the current signed advertisement byte-for-byte. The
/// `subject` query parameter is the client's assertion of which
/// server it wants; this server publishes exactly one ad, so a
/// mismatched subject is a 404, never a different ad. An absent
/// subject serves the one ad (a browser or operator fetching the
/// endpoint directly gets the same bytes a client would).
pub(crate) async fn transport_advertisement_handler(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let Some(minter) = &state.transport_ad else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if let Some(subject) = params.get("subject") {
        if subject != &minter.subject() {
            return StatusCode::NOT_FOUND.into_response();
        }
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match minter.current_bytes(now) {
        Ok(Some(bytes)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            bytes,
        )
            .into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::{AppState, WebSecurity, create_router};
    use axum::{Router, body::Body, http::Request};
    use ed25519_dalek::SigningKey;
    use id_core::transport_advertisement::{AdMinter, SequenceFile, TransportEntry, TransportKind};
    use id_store::tags::TagStore;
    use iroh::endpoint::{Endpoint, RelayMode, presets};
    use iroh_blobs::{api::Store, store::mem::MemStore};
    use iroh_docs::protocol::Docs;
    use iroh_gossip::net::Gossip;
    use std::sync::Arc;
    use tower::ServiceExt as _;

    // Oracle: the #5793 decision record, Decision 2.2 (well-known
    // HTTPS serves the current signed ad byte-for-byte) and
    // Decision 1 (the client trusts the signature, not the
    // substrate).

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    async fn bare_state() -> (AppState, tempfile::TempDir) {
        let endpoint = Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap();
        let store: Store = MemStore::new().into();
        let gossip = Gossip::builder().spawn(endpoint.clone());
        let docs = Docs::memory()
            .spawn(endpoint.clone(), store.clone(), gossip)
            .await
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let tags = Arc::new(
            TagStore::init_in(&docs, "transport-ad-test", dir.path().join("meta"))
                .await
                .unwrap(),
        );
        let state = AppState::new(
            store,
            None,
            "transport-ad-test".to_owned(),
            tags,
            [9; 32],
            dir.path().join(".identity.redb"),
        )
        .await
        .unwrap();
        (state, dir)
    }

    async fn state_with_minter() -> (AppState, Arc<AdMinter>, tempfile::TempDir) {
        let (mut state, dir) = bare_state().await;
        let minter = Arc::new(AdMinter::new(
            SigningKey::from_bytes(&[7u8; 32]),
            SequenceFile::new(dir.path().join("transport-ad.sequence")),
            3600,
        ));
        minter
            .mint(
                vec![TransportEntry {
                    kind: TransportKind::Tcp,
                    endpoint: "203.0.113.7:443".to_string(),
                    priority: 30,
                    broker: None,
                }],
                now(),
            )
            .unwrap();
        state.transport_ad = Some(Arc::clone(&minter));
        (state, minter, dir)
    }

    async fn get(router: Router, uri: &str) -> (StatusCode, Vec<u8>) {
        let resp = router
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec();
        (status, bytes)
    }

    #[tokio::test]
    async fn serves_the_current_ad_byte_for_byte() {
        let (state, minter, _dir) = state_with_minter().await;
        let router = create_router(state, WebSecurity::default());
        let uri = format!(
            "/.well-known/id-transport-advertisement?subject={}",
            minter.subject()
        );
        let (status, body) = get(router, &uri).await;
        assert_eq!(status, StatusCode::OK);
        let expected = minter.current_bytes(now()).unwrap().unwrap();
        assert_eq!(
            body, expected,
            "the handler serves the minted bytes verbatim"
        );
        // And the served object is the verifiable signed ad.
        let signed: id_core::transport_advertisement::SignedAdvertisement =
            serde_json::from_slice(&body).unwrap();
        signed.verify(&minter.subject()).unwrap();
    }

    #[tokio::test]
    async fn subject_mismatch_and_missing_minter_are_404() {
        let (state, _minter, _dir) = state_with_minter().await;
        let router = create_router(state, WebSecurity::default());
        let (status, _) = get(
            router,
            "/.well-known/id-transport-advertisement?subject=0000",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let (bare_state, _dir2) = bare_state().await;
        let router = create_router(bare_state, WebSecurity::default());
        let (status, _) = get(router, "/.well-known/id-transport-advertisement").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
