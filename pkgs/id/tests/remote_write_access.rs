//! End-to-end tests of remote writes over real QUIC on loopback.
//!
//! Two in-process nodes talk to each other directly (no relay, no discovery,
//! no DHT). This is the path that changed most when iroh-blobs 0.103 started
//! rejecting pushes unless an event handler enables them, and where write
//! authorization is enforced: the server must refuse writes from nodes that
//! are not allowed, accept them from nodes that are, never create a name for a
//! blob it does not have, and keep reads open to everyone.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id::{
    MetaRequest, MetaResponse,
    access::AccessPolicy,
    commands::put::{add_raw, push_and_name},
    local::LocalNode,
    meta_client::request_over,
};
use iroh::endpoint::{Endpoint, RelayMode, presets};
use iroh_base::SecretKey;

struct Client {
    endpoint: Endpoint,
    id: iroh_base::EndpointId,
}

async fn client() -> Client {
    let key = SecretKey::generate();
    let id = key.public();
    let endpoint = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .secret_key(key)
        .bind()
        .await
        .unwrap();
    Client { endpoint, id }
}

async fn names_on(server: &LocalNode) -> Vec<String> {
    match server.request(MetaRequest::List).await {
        MetaResponse::List { items } => items.into_iter().map(|(_, n)| n).collect(),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn pushes_and_names_are_refused_for_unlisted_nodes() {
    let server = LocalNode::open(true).await.unwrap();
    let router = server.serve(AccessPolicy::restricted([server.node_id()]));
    let c = client().await;

    let store = iroh_blobs::store::mem::MemStore::new();
    let store_handle: iroh_blobs::api::Store = store.clone().into();
    let (hash, _guard) = add_raw(&store_handle, b"stranger data".to_vec())
        .await
        .unwrap();

    let err = push_and_name(&store_handle, &c.endpoint, server.addr(), "evil.txt", hash)
        .await
        .expect_err("an unlisted node must not be able to write");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("allow-node") || msg.contains("permission"),
        "error should tell the user what to do: {msg}"
    );
    assert!(names_on(&server).await.is_empty(), "no name may be created");
    assert!(
        !server.blobs().blobs().has(hash).await.unwrap(),
        "the blob must not have been stored either"
    );

    c.endpoint.close().await;
    router.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn allowed_nodes_can_push_and_name_and_the_bytes_arrive() {
    let server = LocalNode::open(true).await.unwrap();
    let c = client().await;
    let router = server.serve(AccessPolicy::restricted([server.node_id(), c.id]));

    let store = iroh_blobs::store::mem::MemStore::new();
    let store_handle: iroh_blobs::api::Store = store.clone().into();
    let (hash, _guard) = add_raw(&store_handle, b"hello from a trusted peer".to_vec())
        .await
        .unwrap();

    push_and_name(&store_handle, &c.endpoint, server.addr(), "hello.txt", hash)
        .await
        .expect("an allowed node can write");

    assert_eq!(names_on(&server).await, vec!["hello.txt".to_owned()]);
    let got = server.blobs().blobs().get_bytes(hash).await.unwrap();
    assert_eq!(&*got, b"hello from a trusted peer");
    // Server-side metadata was recorded exactly as for a local put.
    let tags = server
        .tags()
        .get_tags(&server.tags().global, b"hello.txt")
        .await
        .unwrap();
    assert!(tags.iter().any(|t| t.key == "created"));
    assert!(tags.iter().any(|t| t.key == "name"));

    c.endpoint.close().await;
    router.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn naming_a_blob_that_was_never_uploaded_is_refused_even_for_writers() {
    // The old client named first and uploaded second; an interrupted upload left
    // a dangling name. The server now refuses to name content it does not have.
    let server = LocalNode::open(true).await.unwrap();
    let c = client().await;
    let router = server.serve(AccessPolicy::restricted([server.node_id(), c.id]));

    let conn = c
        .endpoint
        .connect(server.addr(), id::META_ALPN)
        .await
        .unwrap();
    let resp = request_over(
        &conn,
        &MetaRequest::Put {
            filename: "ghost.txt".to_owned(),
            hash: iroh_blobs::Hash::from_bytes([9u8; 32]),
        },
    )
    .await
    .unwrap();
    let msg = resp.error_message().expect("must be refused");
    assert!(msg.contains("not present"), "{msg}");
    assert!(names_on(&server).await.is_empty());

    conn.close(0u32.into(), b"done");
    c.endpoint.close().await;
    router.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn reads_stay_open_to_unlisted_nodes() {
    let server = LocalNode::open(true).await.unwrap();
    let c = client().await;
    let router = server.serve(AccessPolicy::restricted([server.node_id()]));

    // Server-side owner creates a file.
    let (hash, _guard) = add_raw(&server.blobs(), b"public".to_vec()).await.unwrap();
    server.ops().put("pub.txt", hash).await.unwrap();

    let conn = c
        .endpoint
        .connect(server.addr(), id::META_ALPN)
        .await
        .unwrap();
    match request_over(&conn, &MetaRequest::List).await.unwrap() {
        MetaResponse::List { items } => {
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].1, "pub.txt");
        }
        other => panic!("{other:?}"),
    }
    match request_over(&conn, &MetaRequest::Whoami).await.unwrap() {
        MetaResponse::Whoami {
            node_id,
            can_write,
            open_writes,
            ..
        } => {
            assert_eq!(node_id, server.node_id());
            assert!(!can_write, "the client is told it cannot write");
            assert!(!open_writes);
        }
        other => panic!("{other:?}"),
    }
    // A read-only blob fetch works too.
    let blobs_conn = c
        .endpoint
        .connect(server.addr(), iroh_blobs::ALPN)
        .await
        .unwrap();
    let store = iroh_blobs::store::mem::MemStore::new();
    let store_handle: iroh_blobs::api::Store = store.clone().into();
    store_handle.remote().fetch(blobs_conn, hash).await.unwrap();
    let got = store_handle.blobs().get_bytes(hash).await.unwrap();
    assert_eq!(&*got, b"public");

    conn.close(0u32.into(), b"done");
    c.endpoint.close().await;
    router.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn open_writes_policy_lets_anyone_write() {
    let server = LocalNode::open(true).await.unwrap();
    let c = client().await;
    let router = server.serve(AccessPolicy::open());

    let store = iroh_blobs::store::mem::MemStore::new();
    let store_handle: iroh_blobs::api::Store = store.clone().into();
    let (hash, _guard) = add_raw(&store_handle, b"anyone".to_vec()).await.unwrap();
    push_and_name(&store_handle, &c.endpoint, server.addr(), "open.txt", hash)
        .await
        .expect("open writes lets any peer write");
    assert_eq!(names_on(&server).await, vec!["open.txt".to_owned()]);

    c.endpoint.close().await;
    router.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}
