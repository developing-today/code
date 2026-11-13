//! Integration tests proving all public web routers apply their security policy.

#![cfg(feature = "web")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{path::PathBuf, sync::Arc};

use axum::{Router, body::Body, http::Request};
use iroh::endpoint::{Endpoint, RelayMode, presets};
use iroh_blobs::{api::Store, store::mem::MemStore};
use iroh_docs::protocol::Docs;
use iroh_gossip::net::Gossip;
use p2panda_core::SigningKey;
use tokio::sync::Mutex;
use tower::ServiceExt as _;

use id_events::local_events::LocalEvents;
use id_store::tags::TagStore;
use id_web as web;
use id_web::{AppState, WebSecurity};

const TOKEN: &str = "router-security-integration-token";

struct Fixture {
    endpoint: Endpoint,
    _docs: Docs,
    store: Store,
    tags: Arc<TagStore>,
    temp: tempfile::TempDir,
    state: AppState,
}

impl Fixture {
    async fn new() -> Self {
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
        let temp = tempfile::tempdir().unwrap();
        let tags = Arc::new(
            TagStore::init_in(&docs, "router-security-test", temp.path().join("meta"))
                .await
                .unwrap(),
        );
        let state = AppState::new(
            store.clone(),
            None,
            "router-security-test".to_owned(),
            tags.clone(),
            [9; 32],
            identity_path(&temp, "low-level"),
        )
        .await
        .unwrap();

        Self {
            endpoint,
            _docs: docs,
            store,
            tags,
            temp,
            state,
        }
    }
}

fn identity_path(temp: &tempfile::TempDir, name: &str) -> PathBuf {
    let dir = temp.path().join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("identity.redb")
}

fn security() -> WebSecurity {
    WebSecurity {
        token: Some(TOKEN.to_owned()),
        allowed_hosts: vec!["localhost".to_owned()],
        cookie_secure: false,
    }
}

fn request(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header("host", "localhost:3000")
        .body(Body::empty())
        .unwrap()
}

async fn assert_security_rejections(router: &Router) {
    let missing = router.clone().oneshot(request("/")).await.unwrap();
    assert_eq!(missing.status(), axum::http::StatusCode::UNAUTHORIZED);

    let invalid = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .header("host", "localhost:3000")
                .header("authorization", "Bearer wrong-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid.status(), axum::http::StatusCode::UNAUTHORIZED);

    let cross_origin = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .header("host", "localhost:3000")
                .header("origin", "https://attacker.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cross_origin.status(), axum::http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn every_public_router_constructor_enforces_its_security_policy() {
    let fixture = Fixture::new().await;
    let policy = security();
    let interface = web::web_interface_from_app_toml(
        "[interfaces.web]\nmode = \"adapter\"\nroute = \"/arena\"\nsocket = \"/ws/arena\"\n",
    )
    .unwrap()
    .unwrap();

    let create_router = web::create_router(fixture.state.clone(), policy.clone());
    assert_security_rejections(&create_router).await;

    let create_router_for_app =
        web::create_router_for_app(fixture.state.clone(), &interface, policy.clone());
    assert_security_rejections(&create_router_for_app).await;

    let create_program_router =
        web::create_program_router(fixture.state.clone(), "arena", policy.clone());
    assert_security_rejections(&create_program_router).await;

    let web_router = web::web_router(
        fixture.store.clone(),
        None,
        "router-security-test".to_owned(),
        fixture.tags.clone(),
        [9; 32],
        identity_path(&fixture.temp, "web-router"),
        policy.clone(),
        None,
        false,
        None,
    )
    .await
    .unwrap();
    assert_security_rejections(&web_router).await;

    let local_events_dir = fixture.temp.path().join("local-events-router");
    std::fs::create_dir_all(&local_events_dir).unwrap();
    let local_events = Arc::new(Mutex::new(
        LocalEvents::open(&local_events_dir, SigningKey::from_bytes(&[9; 32])).unwrap(),
    ));
    let web_router_with_local_events = web::web_router_with_local_events(
        fixture.store.clone(),
        None,
        "router-security-test".to_owned(),
        fixture.tags.clone(),
        [9; 32],
        identity_path(&fixture.temp, "web-router-local-events"),
        policy.clone(),
        None,
        false,
        None,
        local_events,
    )
    .await
    .unwrap();
    assert_security_rejections(&web_router_with_local_events).await;

    let web_routers = web::web_routers(
        fixture.store.clone(),
        None,
        "router-security-test".to_owned(),
        fixture.tags.clone(),
        [9; 32],
        identity_path(&fixture.temp, "web-routers"),
        vec![policy.clone(), policy],
        None,
        false,
        None,
        vec![None, Some("arena".to_owned())],
        None,
        web::SeitanState::default(),
    )
    .await
    .unwrap();
    assert_eq!(web_routers.len(), 2);
    for router in &web_routers {
        assert_security_rejections(router).await;
    }

    // A non-GET bootstrap reaches the router after the guard and sets one
    // cookie. A guard accidentally applied twice would append two cookies.
    let post = web_routers[0]
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/not-found?token=router-security-integration-token")
                .header("host", "localhost:3000")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(post.status(), axum::http::StatusCode::NOT_FOUND);
    assert_eq!(post.headers().get_all("set-cookie").iter().count(), 1);

    let bootstrap = web_routers[0]
        .clone()
        .oneshot(request("/?token=router-security-integration-token"))
        .await
        .unwrap();
    assert_eq!(bootstrap.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(bootstrap.headers().get("location").unwrap(), "/");
    assert_eq!(bootstrap.headers().get_all("set-cookie").iter().count(), 1);

    // Keep the runtime alive for all requests above.
    drop(fixture.endpoint);
}
