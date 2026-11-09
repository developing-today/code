//! Server-to-server file transfer (#5345; design #4975).
//!
//! A user-initiated, single-file *copy* from one running `id` server to
//! another. The source server performs the transfer with its own identity:
//! it looks the name up in its store, pushes the blob to the destination
//! over the existing blobs protocol (`BLOBS_ALPN`), and only then registers
//! the destination name over the meta protocol (`/iroh-meta/3`). The
//! destination's existing write policy therefore sees the authenticated
//! *source-server* node ID — never the initiating CLI's client key — and
//! the destination records that node as the copy's `origin` with its own
//! first-write time as `created` (#4973/#5191 listing contract).
//!
//! Contract (from #5345 and the #4975 decision):
//!
//! - Bytes before name: the complete blob is pushed and confirmed before
//!   the destination name is registered, so an interrupted send never
//!   leaves a visible dangling name.
//! - The source is never modified or deleted; a send is a copy.
//! - Repeating the same transfer is idempotent: if the destination name
//!   already points at the same hash, the send reports
//!   [`TransferReceipt::already_present`] without pushing anything.
//! - Name conflicts refuse by default. Replacement must be explicit
//!   (`replace`); the new file is staged and then copied onto the
//!   destination name, and the copy preserves the replaced file under
//!   the file store's canonical `<name>.archive.<unix-ts>` archive name.
//! - No new transport channel: bytes ride `BLOBS_ALPN`, control and
//!   registration ride `/iroh-meta/3`, exactly like `id put` to a remote
//!   node ([`crate::commands::put::push_and_name`] does the push+register
//!   ordering); this module adds the conflict policy and the receipt.
//!
//! The one known race is the same one the rest of the meta protocol
//! accepts: the conflict pre-check and the final registration are separate
//! requests, so a third party writing the same destination name in
//! between can still be overwritten by the registration. The pre-check
//! exists so the *ordinary* conflict — the name was taken long before the
//! send started — is refused before any bytes move.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use iroh::{EndpointAddr, endpoint::Endpoint};
use iroh_base::EndpointId;
use iroh_blobs::{Hash, api::Store};

use crate::META_ALPN;
use crate::commands::put::push_and_name;
use crate::meta_client::{expect_ok, request_over};
use crate::protocol::{MetaRequest, MetaResponse};

/// What a completed (or already-completed) transfer did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferReceipt {
    /// Content hash that was transferred.
    pub hash: Hash,
    /// Name of the file on the source server.
    pub source_name: String,
    /// Name registered on the destination.
    pub dest_name: String,
    /// Destination node ID.
    pub dest: EndpointId,
    /// The destination already had `dest_name` pointing at `hash`; nothing
    /// was pushed. This is what makes a retry of the same send safe.
    pub already_present: bool,
    /// An existing, different file at `dest_name` was archived and replaced
    /// (only possible when the send opted into replacement).
    pub replaced: bool,
}

/// The staging name a replacement transfer registers on the destination
/// before copying onto the real destination name. Copying onto an
/// existing name is the file store's archive behaviour: the old file is
/// preserved under `<name>.archive.<unix-ts>` and the copy lands
/// atomically after it, so staging through a copy is what makes a
/// replacement preserve the old file instead of overwriting it.
pub fn staging_name_for(name: &str, unix_ts: u64) -> String {
    format!("{name}.incoming.{unix_ts}")
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Copy `hash` from this node's `store` to the destination node at `dest`,
/// registering it there as `dest_name`.
///
/// `endpoint` must carry the *source server's* identity: the destination
/// authorizes the push and the registration against that node ID, and
/// records it as the copy's origin.
///
/// Refuses (before pushing any bytes) when `dest_name` already names
/// different content on the destination and `replace` is false.
pub async fn send_file(
    store: &Store,
    endpoint: &Endpoint,
    dest: EndpointAddr,
    source_name: &str,
    hash: Hash,
    dest_name: &str,
    replace: bool,
) -> Result<TransferReceipt> {
    let dest_id = dest.id;

    // Conflict pre-check on a short-lived meta connection, before any
    // bytes move: an ordinary conflict must not cost a full blob push,
    // and a refused send must leave the destination untouched.
    let meta_conn = crate::connectivity::connect(endpoint, dest.clone(), META_ALPN)
        .await
        .context("connecting to the destination for the conflict pre-check")?;
    let existing = match expect_ok(
        request_over(
            &meta_conn,
            &MetaRequest::Get {
                filename: dest_name.to_owned(),
            },
        )
        .await?,
    )? {
        MetaResponse::Get { hash } => hash,
        _ => bail!("unexpected response to destination name lookup"),
    };
    meta_conn.close(0u32.into(), b"done");

    let replaced = match existing {
        Some(existing_hash) if existing_hash == hash => {
            return Ok(TransferReceipt {
                hash,
                source_name: source_name.to_owned(),
                dest_name: dest_name.to_owned(),
                dest: dest_id,
                already_present: true,
                replaced: false,
            });
        }
        Some(_) if !replace => {
            bail!(
                "destination already has a different file named {dest_name:?}; \
                 refusing to replace it (retry with replacement enabled to \
                 archive the existing file and replace it)"
            );
        }
        Some(_) => true,
        None => false,
    };

    if !replaced {
        // Bytes before name, confirmed before success: push_and_name
        // pushes the complete blob and returns only after the
        // destination's Put response confirms registration, so a failure
        // here leaves no visible name behind.
        push_and_name(store, endpoint, dest, dest_name, hash)
            .await
            .context("pushing the blob to the destination and registering its name")?;
    } else {
        // Replacement: register the new file under a staging name first
        // (same bytes-before-name ordering), then copy it onto the
        // destination name. The copy archives the existing destination
        // file under `<dest_name>.archive.<ts>` before landing, so the
        // old content is preserved by the store's own archive behaviour;
        // the staging name is deleted once the copy has landed.
        let staging_name = staging_name_for(dest_name, now_unix());
        push_and_name(store, endpoint, dest.clone(), &staging_name, hash)
            .await
            .context("pushing the blob to the destination and staging its name")?;

        let meta_conn = crate::connectivity::connect(endpoint, dest, META_ALPN)
            .await
            .context("connecting to the destination to land the replacement")?;
        let copied = expect_ok(
            request_over(
                &meta_conn,
                &MetaRequest::Copy {
                    from: staging_name.clone(),
                    to: dest_name.to_owned(),
                },
            )
            .await?,
        )?;
        match copied {
            MetaResponse::Copy { success: true } => {}
            MetaResponse::Copy { success: false } => {
                meta_conn.close(0u32.into(), b"done");
                bail!("destination refused the replacement copy onto {dest_name:?}");
            }
            _ => bail!("unexpected response to the replacement copy"),
        }
        // The copy landed and the old file is archived; remove the
        // staging name. A cleanup failure does not un-send the file, but
        // it leaves a duplicate name behind, so it is reported as an
        // error rather than hidden.
        let cleaned = expect_ok(
            request_over(
                &meta_conn,
                &MetaRequest::Delete {
                    filename: staging_name.clone(),
                },
            )
            .await?,
        )?;
        meta_conn.close(0u32.into(), b"done");
        match cleaned {
            MetaResponse::Delete { .. } => {}
            _ => bail!("unexpected response removing the staging name"),
        }
    }

    Ok(TransferReceipt {
        hash,
        source_name: source_name.to_owned(),
        dest_name: dest_name.to_owned(),
        dest: dest_id,
        already_present: false,
        replaced,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::access::AccessPolicy;
    use crate::commands::put::add_raw;
    use crate::local::LocalNode;

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
}
