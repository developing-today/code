//! End-to-end tests of server-to-server file transfer (#5345), ported
//! from the transfer module's unit tests: two in-process `LocalNode`s,
//! the source sends with its own server identity, and the destination's
//! write policy decides.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id::access::AccessPolicy;
use id::commands::put::add_raw;
use id::local::LocalNode;
use id::transfer::send_file;
use id::{MetaRequest, MetaResponse};
use iroh_blobs::Hash;

/// Put `data` on `node` under `name`; returns the content hash.
async fn put_named(node: &LocalNode, name: &str, data: &[u8]) -> Hash {
    let (hash, _guard) = add_raw(&node.blobs(), data.to_vec()).await.unwrap();
    node.ops().put(name, hash).await.unwrap();
    hash
}

async fn dest_hash(node: &LocalNode, name: &str) -> Option<Hash> {
    node.blobs().tags().get(name).await.unwrap().map(|t| t.hash)
}

#[tokio::test]
async fn send_copies_blob_registers_name_and_stamps_source_as_origin() {
    let source = LocalNode::open(true).await.unwrap();
    let dest = LocalNode::open(true).await.unwrap();
    // The destination admits the source server (and only it) as writer.
    let dest_router = dest.serve(AccessPolicy::restricted([source.node_id()]));
    let hash = put_named(&source, "report.txt", b"hello transfer").await;

    let receipt = send_file(
        &source.blobs(),
        source.endpoint(),
        dest.addr(),
        "report.txt",
        hash,
        "report.txt",
        false,
    )
    .await
    .unwrap();

    assert_eq!(receipt.hash, hash);
    assert!(!receipt.already_present);
    assert!(!receipt.replaced);
    assert_eq!(dest_hash(&dest, "report.txt").await, Some(hash));

    // Listing contract (#4973): the copy's origin is the authenticated
    // sending server, not the initiating user.
    let resp = dest.request(MetaRequest::List).await;
    match resp {
        MetaResponse::List { items } => {
            let entry = items.iter().find(|i| i.name == "report.txt").unwrap();
            assert_eq!(
                entry.origin.as_deref(),
                Some(source.node_id().to_string().as_str())
            );
            assert!(entry.uploaded_at.is_some());
        }
        _ => panic!("unexpected list response"),
    }

    // The source is untouched: a send is a copy, never a move.
    assert_eq!(dest_hash(&source, "report.txt").await, Some(hash));

    dest_router.shutdown().await.unwrap();
    source.shutdown().await.unwrap();
    dest.shutdown().await.unwrap();
}

#[tokio::test]
async fn resend_is_idempotent() {
    let source = LocalNode::open(true).await.unwrap();
    let dest = LocalNode::open(true).await.unwrap();
    let router = dest.serve(AccessPolicy::restricted([source.node_id()]));
    let hash = put_named(&source, "a.txt", b"same bytes").await;

    let first = send_file(
        &source.blobs(),
        source.endpoint(),
        dest.addr(),
        "a.txt",
        hash,
        "a.txt",
        false,
    )
    .await
    .unwrap();
    assert!(!first.already_present);

    let second = send_file(
        &source.blobs(),
        source.endpoint(),
        dest.addr(),
        "a.txt",
        hash,
        "a.txt",
        false,
    )
    .await
    .unwrap();
    assert!(second.already_present);
    assert_eq!(dest_hash(&dest, "a.txt").await, Some(hash));

    router.shutdown().await.unwrap();
    source.shutdown().await.unwrap();
    dest.shutdown().await.unwrap();
}

#[tokio::test]
async fn conflict_refuses_by_default_and_replace_archives_the_old_file() {
    let source = LocalNode::open(true).await.unwrap();
    let dest = LocalNode::open(true).await.unwrap();
    let router = dest.serve(AccessPolicy::restricted([source.node_id()]));
    let new_hash = put_named(&source, "data.bin", b"new content").await;
    let old_hash = put_named(&dest, "data.bin", b"old content").await;

    let refused = send_file(
        &source.blobs(),
        source.endpoint(),
        dest.addr(),
        "data.bin",
        new_hash,
        "data.bin",
        false,
    )
    .await;
    assert!(refused.is_err(), "conflict without replace must refuse");
    // The refused send changed nothing on the destination.
    assert_eq!(dest_hash(&dest, "data.bin").await, Some(old_hash));

    let receipt = send_file(
        &source.blobs(),
        source.endpoint(),
        dest.addr(),
        "data.bin",
        new_hash,
        "data.bin",
        true,
    )
    .await
    .unwrap();
    assert!(receipt.replaced);
    assert_eq!(dest_hash(&dest, "data.bin").await, Some(new_hash));
    // The replaced file survives under an archive name, and the
    // staging duplicate used during the replacement is gone.
    let resp = dest.request(MetaRequest::List).await;
    match resp {
        MetaResponse::List { items } => {
            assert!(
                items
                    .iter()
                    .any(|i| i.name.starts_with("data.bin.archive.") && i.hash == old_hash),
                "old file must be preserved under an archive name: {items:?}"
            );
            assert!(
                !items
                    .iter()
                    .any(|i| i.name.starts_with("data.bin.incoming.")),
                "staging duplicate must be cleaned up: {items:?}"
            );
        }
        _ => panic!("unexpected list response"),
    }

    router.shutdown().await.unwrap();
    source.shutdown().await.unwrap();
    dest.shutdown().await.unwrap();
}

#[tokio::test]
async fn destination_that_does_not_allow_the_source_denies_the_send() {
    let source = LocalNode::open(true).await.unwrap();
    let dest = LocalNode::open(true).await.unwrap();
    // Nobody may write to this destination.
    let router = dest.serve(AccessPolicy::restricted([]));
    let hash = put_named(&source, "secret.txt", b"classified").await;

    let denied = send_file(
        &source.blobs(),
        source.endpoint(),
        dest.addr(),
        "secret.txt",
        hash,
        "secret.txt",
        false,
    )
    .await;
    assert!(
        denied.is_err(),
        "a destination that does not allowlist the source must deny the send"
    );
    assert_eq!(dest_hash(&dest, "secret.txt").await, None);

    router.shutdown().await.unwrap();
    source.shutdown().await.unwrap();
    dest.shutdown().await.unwrap();
}
