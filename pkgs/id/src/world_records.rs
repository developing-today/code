//! World records in iroh-docs: persistent, replicated structured data.
//!
//! A world program publishes records as a pure projection of its state (see
//! [`crate::world::WorldProgram::records`]). The host mirrors the current set
//! into one iroh-docs document per world:
//!
//! - **Durable**: with a persistent docs store, records survive restarts
//!   (they are also recomputable from the journal, so the doc is a cache of
//!   authoritative state, never a second source of truth).
//! - **Replicated**: anyone holding a join capability can ask for a
//!   *read-only* doc ticket and sync the records over Iroh with any
//!   iroh-docs node, including while the world keeps changing.
//! - **Single writer**: only the host's author writes. Readers get the
//!   namespace public key, so a replica cannot forge entries.
//!
//! Doc keys are the record key followed by `\0`. iroh-docs deletes by prefix,
//! and record keys cannot contain control characters, so the terminator makes
//! every delete exact. Values are the record's JSON encoding.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures_lite::StreamExt;
use iroh_blobs::{Hash, api::Store};
use iroh_docs::{
    AuthorId, DocTicket, NamespaceId,
    api::{
        Doc,
        protocol::{AddrInfoOptions, ShareMode},
    },
    engine::LiveEvent,
    protocol::Docs,
    store::Query,
};
use tokio::task::JoinHandle;

use crate::world::{Records, WorldHandle};

/// The host side: one world's records document.
#[derive(Clone, Debug)]
pub struct RecordsStore {
    doc: Doc,
    author: AuthorId,
}

fn doc_key(key: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(key.len() + 1);
    bytes.extend_from_slice(key.as_bytes());
    bytes.push(0);
    bytes
}

fn record_key(doc_key: &[u8]) -> Option<&str> {
    std::str::from_utf8(doc_key.strip_suffix(&[0])?).ok()
}

impl RecordsStore {
    /// Open the world's records doc, creating it on first use. With
    /// `namespace_file`, the doc's namespace ID is remembered there so the
    /// same doc (and its replicas' tickets) survive restarts.
    ///
    /// # Errors
    ///
    /// Docs failures, or an unreadable namespace file.
    pub async fn open(docs: &Docs, namespace_file: Option<&Path>) -> Result<Self> {
        let author = docs.author_default().await?;
        if let Some(path) = namespace_file
            && let Ok(text) = tokio::fs::read_to_string(path).await
        {
            let id: NamespaceId = text
                .trim()
                .parse()
                .with_context(|| format!("parse records namespace in {}", path.display()))?;
            if let Some(doc) = docs.open(id).await? {
                return Ok(Self { doc, author });
            }
            tracing::warn!(
                "records doc {id} from {} is gone; creating a new one",
                path.display()
            );
        }
        let doc = docs.create().await?;
        if let Some(path) = namespace_file {
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::write(path, doc.id().to_string()).await?;
        }
        Ok(Self { doc, author })
    }

    /// The document's namespace.
    #[must_use]
    pub fn namespace(&self) -> NamespaceId {
        self.doc.id()
    }

    /// A read-only ticket for replicating the records doc.
    ///
    /// # Errors
    ///
    /// Docs failures.
    pub async fn read_ticket(&self) -> Result<String> {
        let ticket = self
            .doc
            .share(ShareMode::Read, AddrInfoOptions::RelayAndAddresses)
            .await?;
        Ok(ticket.to_string())
    }

    /// Make the doc hold exactly `records`: write changed values, delete
    /// keys that disappeared. Unchanged records are not rewritten (the
    /// comparison is by content hash), so replicas only sync real changes.
    ///
    /// # Errors
    ///
    /// Docs failures.
    pub async fn reconcile(&self, records: &Records) -> Result<()> {
        let mut existing = std::collections::HashMap::new();
        let entries = self.doc.get_many(Query::all().build()).await?;
        tokio::pin!(entries);
        while let Some(entry) = entries.try_next().await? {
            if let Some(key) = record_key(entry.key()) {
                existing.insert(key.to_owned(), entry.content_hash());
            }
        }
        for (key, value) in records {
            let bytes = serde_json::to_vec(value)?;
            if existing.remove(key) != Some(Hash::new(&bytes)) {
                self.doc.set_bytes(self.author, doc_key(key), bytes).await?;
            }
        }
        for key in existing.keys() {
            self.doc.del(self.author, doc_key(key)).await?;
        }
        Ok(())
    }

    /// Keep the doc in step with `world`'s records until the world stops.
    /// Changes are coalesced: a burst of updates causes one reconcile of the
    /// latest set. Failures are logged and retried on the next change.
    pub fn spawn_publisher(&self, world: &WorldHandle) -> JoinHandle<()> {
        let this = self.clone();
        let mut records = world.watch_records();
        tokio::spawn(async move {
            loop {
                let current = Arc::clone(&records.borrow_and_update().1);
                if let Err(error) = this.reconcile(&current).await {
                    tracing::warn!("world records publish failed: {error:#}");
                }
                if records.changed().await.is_err() {
                    break;
                }
            }
        })
    }
}

/// Read every record from a (local or replicated) records doc.
///
/// # Errors
///
/// Docs or blob failures, or a value that is not JSON.
pub async fn read_records(doc: &Doc, blobs: &Store) -> Result<Records> {
    let mut records = Records::new();
    let entries = doc.get_many(Query::all().build()).await?;
    tokio::pin!(entries);
    while let Some(entry) = entries.try_next().await? {
        let Some(key) = record_key(entry.key()) else {
            continue;
        };
        let bytes = blobs
            .blobs()
            .get_bytes(entry.content_hash())
            .await
            .with_context(|| format!("content of record {key:?} is not available yet"))?;
        records.insert(
            key.to_owned(),
            serde_json::from_slice(&bytes).with_context(|| format!("record {key:?}"))?,
        );
    }
    Ok(records)
}

/// Replicate a records doc from its read ticket into `docs` and return its
/// contents once the first sync and its content downloads have finished.
///
/// The replica stays live afterwards: further host changes keep arriving
/// for as long as `docs` runs.
///
/// # Errors
///
/// A malformed ticket, a sync that does not finish within `timeout`, or a
/// read failure.
pub async fn replicate(
    docs: &Docs,
    blobs: &Store,
    ticket: &str,
    timeout: Duration,
) -> Result<(Doc, Records)> {
    let ticket: DocTicket = ticket.parse().context("parse records ticket")?;
    let (doc, mut events) = docs.import_and_subscribe(ticket).await?;
    tokio::time::timeout(timeout, async {
        while let Some(event) = events.next().await {
            if let LiveEvent::PendingContentReady = event? {
                return Ok(());
            }
        }
        bail!("records sync ended before content was ready")
    })
    .await
    .context("records sync timed out")??;
    let records = read_records(&doc, blobs).await?;
    Ok((doc, records))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::time::Duration;

    use anyhow::Result;
    use iroh::{
        Endpoint,
        endpoint::{RelayMode, presets},
        protocol::Router,
    };
    use iroh_blobs::{BlobsProtocol, store::mem::MemStore};
    use iroh_gossip::net::Gossip;

    use super::*;
    use crate::world::{
        WorldCore, WorldEvent, WorldEventKind, WorldHandle, WorldLimits, WorldScopes,
    };

    /// Publishes `{"count": n}` and increments on every input.
    struct Counter(u64);

    impl crate::world::WorldProgram for Counter {
        fn update(&mut self, event: &WorldEvent) -> Result<()> {
            if let WorldEventKind::Input(_) = &event.kind {
                self.0 += 1;
            }
            Ok(())
        }

        fn records(&mut self) -> Result<Option<String>> {
            Ok(Some(format!("{{\"count\":{}}}", self.0)))
        }
    }

    struct Node {
        router: Router,
        endpoint: Endpoint,
        docs: Docs,
        blobs: Store,
    }

    impl Node {
        async fn spawn() -> Self {
            let endpoint = Endpoint::builder(presets::Minimal)
                .relay_mode(RelayMode::Disabled)
                .bind()
                .await
                .unwrap();
            let blobs: Store = MemStore::new().into();
            let gossip = Gossip::builder().spawn(endpoint.clone());
            let docs = Docs::memory()
                .spawn(endpoint.clone(), blobs.clone(), gossip.clone())
                .await
                .unwrap();
            let router = Router::builder(endpoint.clone())
                .accept(iroh_docs::net::ALPN, docs.clone())
                .accept(iroh_blobs::ALPN, BlobsProtocol::new(&blobs, None))
                .accept(iroh_gossip::net::GOSSIP_ALPN, gossip)
                .spawn();
            Self {
                router,
                endpoint,
                docs,
                blobs,
            }
        }

        async fn shutdown(self) {
            let _ = self.router.shutdown().await;
            self.endpoint.close().await;
        }
    }

    async fn wait_for(node: &Node, doc: &Doc, expected: u64) -> Result<Records> {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            let records = read_records(doc, &node.blobs).await?;
            if records.get("count").and_then(|c| c.as_u64()) == Some(expected) {
                return Ok(records);
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for count={expected}, have {records:?}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[tokio::test]
    async fn records_replicate_peer_to_peer_and_keep_syncing() {
        // Host: a world whose program publishes records.
        let host = Node::spawn().await;
        let world = WorldHandle::spawn_with_program(
            WorldCore::new("lobby", WorldLimits::default()).unwrap(),
            Box::new(Counter(0)),
        );
        let store = RecordsStore::open(&host.docs, None).await.unwrap();
        let ticket = store.read_ticket().await.unwrap();
        let publisher = store.spawn_publisher(&world);
        // Sanity: the writer's own doc converges to the program records.
        let writer_records = wait_for(&host, &store.doc, 0).await.unwrap();
        assert_eq!(writer_records["count"], 0);

        // Replica: a second node with only the ticket (as a peer would have).
        let replica = Node::spawn().await;
        let (doc, records) = replicate(
            &replica.docs,
            &replica.blobs,
            &ticket,
            Duration::from_secs(20),
        )
        .await
        .unwrap();
        assert_eq!(records["count"], 0);

        // A live update reaches the replica without another explicit sync.
        let (_, ann) = world.issue("ann", WorldScopes::GUEST).await.unwrap();
        world.input(ann.clone(), b"inc".to_vec()).await.unwrap();
        wait_for(&host, &store.doc, 1).await.unwrap();
        wait_for(&replica, &doc, 1).await.unwrap();

        // And it keeps up with further changes.
        world.input(ann, b"inc".to_vec()).await.unwrap();
        wait_for(&replica, &doc, 2).await.unwrap();

        publisher.abort();
        world.shutdown().await.unwrap();
        host.shutdown().await;
        replica.shutdown().await;
    }

    #[tokio::test]
    async fn reconcile_writes_deletes_and_leaves_unchanged_records_alone() {
        let host = Node::spawn().await;
        let store = RecordsStore::open(&host.docs, None).await.unwrap();
        let mut records = Records::new();
        records.insert("a".to_owned(), 1.into());
        records.insert("b".to_owned(), "two".into());
        store.reconcile(&records).await.unwrap();
        let read = read_records(&store.doc, &host.blobs).await.unwrap();
        assert_eq!(read, records);

        // Changing nothing rewrites nothing.
        let before: Vec<_> = {
            let mut hashes = Vec::new();
            let entries = store.doc.get_many(Query::all().build()).await.unwrap();
            tokio::pin!(entries);
            while let Some(entry) = entries.try_next().await.unwrap() {
                hashes.push((entry.key().to_vec(), entry.content_hash()));
            }
            hashes
        };
        store.reconcile(&records).await.unwrap();
        let after: Vec<_> = {
            let mut hashes = Vec::new();
            let entries = store.doc.get_many(Query::all().build()).await.unwrap();
            tokio::pin!(entries);
            while let Some(entry) = entries.try_next().await.unwrap() {
                hashes.push((entry.key().to_vec(), entry.content_hash()));
            }
            hashes
        };
        assert_eq!(before, after);

        // A removed record is deleted, not left behind.
        records.remove("a");
        records.insert("b".to_owned(), "three".into());
        store.reconcile(&records).await.unwrap();
        let read = read_records(&store.doc, &host.blobs).await.unwrap();
        assert_eq!(read, records);

        host.shutdown().await;
    }
}
