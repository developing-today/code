//! A minimal in-process node: blob store + iroh-docs + [`TagStore`], no network.
//!
//! The CLI and REPL used to fall back to a legacy single-blob metadata
//! document whenever `id serve` was not running, which meant tags set offline
//! were invisible to the server and vice versa. [`LocalNode`] removes that
//! split: it opens the **same** persistent stores `serve` uses (blobs under
//! `.iroh-store/`, documents under `.iroh-store/docs/`, namespace registry
//! under `.iroh-meta/`), so offline and online commands see one set of tags.
//!
//! It spawns an iroh endpoint only because the iroh-docs engine requires one;
//! the endpoint uses the `Minimal` preset with relays disabled (no relay, no
//! address lookup) and binds to loopback, so nothing is reachable from the
//! network.
//!
//! Only one process can open a persistent store at a time. Callers should
//! prefer talking to a running server (see
//! [`crate::commands::serve::get_serve_info`]) and use `LocalNode` only when
//! none is running.

use std::{net::Ipv4Addr, path::PathBuf, sync::Arc};

use anyhow::Result;
use iroh::{
    EndpointAddr,
    endpoint::{Endpoint, RelayMode, presets},
    protocol::{ProtocolHandler, Router},
};
use iroh_base::{EndpointId, SecretKey, TransportAddr};
use iroh_blobs::{ALPN as BLOBS_ALPN, BlobsProtocol, api::Store};
use iroh_docs::protocol::Docs;
use iroh_gossip::net::Gossip;

use crate::access::AccessPolicy;
use crate::fileops::FileOps;
use crate::protocol::{MetaProtocol, MetaRequest, MetaResponse};
use crate::store::{StoreType, load_or_create_keypair, open_store};
use crate::tags::TagStore;
use crate::{KEY_FILE, META_ALPN, STORE_PATH, access::blobs_events};

/// A network-less node exposing the blob store and the metadata [`TagStore`].
#[derive(Debug)]
pub struct LocalNode {
    /// Keeps an ephemeral node's registry directory alive.
    _scratch: Option<tempfile::TempDir>,
    store: StoreType,
    blobs: Store,
    tags: Arc<TagStore>,
    docs: Docs,
    gossip: Gossip,
    endpoint: Endpoint,
    meta: Arc<MetaProtocol>,
    node_id: EndpointId,
}

impl LocalNode {
    /// Open the node in the current working directory.
    ///
    /// With `ephemeral = true` everything lives in memory and no files are
    /// touched (used by tests and `--ephemeral` commands). Otherwise the
    /// persistent stores and the node key (`.iroh-key`) are used, matching
    /// `id serve`.
    ///
    /// Legacy `.meta` metadata (from before iroh-docs) is imported once.
    pub async fn open(ephemeral: bool) -> Result<Self> {
        let key = if ephemeral {
            SecretKey::generate()
        } else {
            load_or_create_keypair(KEY_FILE).await?
        };
        Self::open_with_key(ephemeral, key).await
    }

    /// Like [`LocalNode::open`] with an explicit node key.
    pub async fn open_with_key(ephemeral: bool, key: SecretKey) -> Result<Self> {
        let node_id = key.public();
        let store = open_store(ephemeral).await?;
        let blobs = store.as_store();

        let endpoint = Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .secret_key(key)
            .bind_addr(std::net::SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))?
            .bind()
            .await?;
        let gossip = Gossip::builder().spawn(endpoint.clone());

        let docs = if ephemeral {
            Docs::memory()
                .spawn(endpoint.clone(), blobs.clone(), gossip.clone())
                .await?
        } else {
            let docs_path = PathBuf::from(STORE_PATH).join("docs");
            std::fs::create_dir_all(&docs_path)?;
            Docs::persistent(docs_path)
                .spawn(endpoint.clone(), blobs.clone(), gossip.clone())
                .await?
        };

        // Ephemeral nodes keep their namespace registry in a private temp dir so
        // they never touch (or collide on) the working directory.
        let scratch = if ephemeral {
            Some(tempfile::tempdir()?)
        } else {
            None
        };
        let tags = match &scratch {
            Some(dir) => {
                TagStore::init_in(&docs, &node_id.to_string(), dir.path().join(".iroh-meta"))
                    .await?
            }
            None => TagStore::init(&docs, &node_id.to_string()).await?,
        };
        let imported = tags.migrate_legacy_meta(&blobs).await?;
        if imported > 0 {
            tracing::info!("imported {imported} legacy metadata tag(s) into iroh-docs");
        }

        let tags = Arc::new(tags);
        // The local owner executes requests against itself, so writes are open.
        let meta = {
            let mut m = MetaProtocol::new(
                &blobs,
                None,
                Arc::clone(&tags),
                AccessPolicy::open(),
                node_id,
            );
            // Nothing is in flight for an in-process node: a missing blob is missing.
            if let Some(inner) = Arc::get_mut(&mut m) {
                inner.blob_wait = std::time::Duration::ZERO;
            }
            m
        };

        Ok(Self {
            _scratch: scratch,
            store,
            blobs,
            tags,
            docs,
            gossip,
            endpoint,
            meta,
            node_id,
        })
    }

    /// Handle to the blob store (cheap to clone).
    pub fn blobs(&self) -> Store {
        self.blobs.clone()
    }

    /// The metadata tag store.
    pub const fn tags(&self) -> &Arc<TagStore> {
        &self.tags
    }

    /// File operations (put/rename/copy/delete) over this node's stores.
    pub fn ops(&self) -> FileOps<'_> {
        FileOps::new(&self.blobs, &self.tags)
    }

    /// Answer a meta-protocol request in-process, exactly as a server would.
    ///
    /// This is what makes offline CLI/REPL commands behave identically to
    /// online ones: both go through [`MetaProtocol::handle`].
    pub async fn request(&self, req: MetaRequest) -> MetaResponse {
        self.meta.handle(&self.node_id, req).await
    }

    /// This node's ID.
    pub const fn node_id(&self) -> EndpointId {
        self.node_id
    }

    /// Dialable address of this node on loopback (for in-process tests and
    /// same-machine clients; no discovery is involved).
    pub fn addr(&self) -> EndpointAddr {
        let ips = self.endpoint.bound_sockets().into_iter().map(|a| {
            TransportAddr::Ip(std::net::SocketAddr::new(
                Ipv4Addr::LOCALHOST.into(),
                a.port(),
            ))
        });
        EndpointAddr::from_parts(self.node_id, ips)
    }

    /// Start serving the meta and blobs protocols with the given write policy.
    ///
    /// This is the same wiring `id serve` uses, minus gossip, web and lock file.
    /// Used by tests and by programs embedding an `id` node. Shut the returned
    /// router down before [`LocalNode::shutdown`].
    pub fn serve(&self, access: AccessPolicy) -> Router {
        let meta = MetaProtocol::new(
            &self.blobs,
            None,
            Arc::clone(&self.tags),
            access.clone(),
            self.node_id,
        );
        let blobs = BlobsProtocol::new(&self.blobs, Some(blobs_events(access)));
        Router::builder(self.endpoint.clone())
            .accept(META_ALPN, meta)
            .accept(BLOBS_ALPN, blobs)
            .spawn()
    }

    /// Shut everything down cleanly (flushes persistent stores).
    pub async fn shutdown(self) -> Result<()> {
        ProtocolHandler::shutdown(&self.docs).await;
        // A router started with `serve()` shuts the blob store (and gossip)
        // down when it is shut down; tolerate that so either order works.
        let _ = self.gossip.shutdown().await;
        self.endpoint.close().await;
        let _ = self.store.shutdown().await;
        Ok(())
    }
}
