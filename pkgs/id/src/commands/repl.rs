//! REPL context and command execution.
//!
//! This module provides the [`ReplContext`] type, which manages state and
//! connections for the interactive REPL. It abstracts away the differences
//! between local-only mode, local serve mode, and remote node mode.
//!
//! # Operating Modes
//!
//! The REPL can operate in three modes, automatically selected at startup:
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                        ReplContext                              │
//! │              (unified interface for all modes)                  │
//! └─────────────────────────────────────────────────────────────────┘
//!                              │
//!          ┌───────────────────┼───────────────────┐
//!          ▼                   ▼                   ▼
//! ┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐
//! │ ReplContextInner│ │ ReplContextInner│ │ ReplContextInner│
//! │     ::Local     │ │    ::Remote     │ │  ::RemoteNode   │
//! │                 │ │                 │ │                 │
//! │ Direct store    │ │ Local serve     │ │ Remote peer     │
//! │ access, no      │ │ instance via    │ │ via network,    │
//! │ networking      │ │ local socket    │ │ DNS discovery   │
//! └─────────────────┘ └─────────────────┘ └─────────────────┘
//! ```
//!
//! ## Local Mode (`id repl` with no serve running)
//!
//! Direct access to the blob store. All operations read/write locally.
//!
//! ## Remote Mode (`id repl` with serve running)
//!
//! Connects to the local serve instance via QUIC. Operations are proxied
//! through the server, enabling concurrent access and network publishing.
//!
//! ## Remote Node Mode (`id repl <NODE_ID>`)
//!
//! Connects to a remote peer node over the network. Uses DNS-based address
//! lookup to resolve the node ID. All operations target the remote peer.
//!
//! # Connection Management
//!
//! The context lazily establishes connections:
//! - [`meta_conn()`](ReplContext::meta_conn): Connection for metadata operations
//! - [`blobs_conn()`](ReplContext::blobs_conn): Connection for blob transfers
//!
//! Connections are reused across operations and automatically reconnected
//! if they close.
//!
//! # Available Operations
//!
//! The context provides high-level methods for all blob operations:
//!
//! | Operation | Description |
//! |-----------|-------------|
//! | [`list()`](ReplContext::list) | List all stored blobs |
//! | [`put()`](ReplContext::put) | Store a new blob |
//! | [`get()`](ReplContext::get) | Retrieve a blob by name |
//! | [`gethash()`](ReplContext::gethash) | Retrieve a blob by hash |
//! | [`delete()`](ReplContext::delete) | Remove a blob |
//! | [`rename()`](ReplContext::rename) | Change a blob's name |
//! | [`copy()`](ReplContext::copy) | Duplicate a blob with a new name |
//! | [`find()`](ReplContext::find) | Search for blobs by pattern |
//!
//! # Remote Targeting with @`NODE_ID`
//!
//! In Remote mode (connected to local serve), commands can target specific
//! remote nodes using the `@NODE_ID` prefix:
//!
//! ```text
//! > list @abc123...  # List files on remote node
//! > put @abc123... file.txt  # Store on remote node
//! > get @abc123... config  # Get from remote node
//! ```

use anyhow::{Result, anyhow, bail};
use futures_lite::StreamExt;
use iroh::{
    address_lookup::{DnsAddressLookup, PkarrPublisher},
    endpoint::{Connection, Endpoint},
};
use iroh_base::EndpointId;
use iroh_blobs::{
    ALPN as BLOBS_ALPN, BlobFormat, Hash,
    api::{Store, blobs::AddBytesOptions},
    protocol::{ChunkRanges, ChunkRangesSeq, PushRequest},
};
use std::{io::Read, path::PathBuf};
use tokio::fs as afs;

use crate::commands::client::create_local_client_endpoint;
use crate::commands::serve::get_serve_info;
use crate::{
    CLIENT_KEY_FILE, FindMatch, META_ALPN, MatchKind, MetaRequest, MetaResponse, StoreType,
    export_blob, is_node_id, load_or_create_keypair, open_store,
};

/// REPL execution context managing connections and store access.
///
/// This struct provides a unified interface for blob operations regardless
/// of whether we're operating locally, through a local serve instance, or
/// against a remote peer node.
///
/// # Creating a Context
///
/// Use [`ReplContext::new()`] to create a context. The target mode is
/// automatically detected:
///
/// ```rust,ignore
/// // Auto-detect: local serve if running, otherwise direct local
/// let ctx = ReplContext::new(None).await?;
///
/// // Connect to a specific remote node
/// let ctx = ReplContext::new(Some("abc123...".to_string())).await?;
/// ```
///
/// # Thread Safety
///
/// `ReplContext` is not `Send` or `Sync` because it holds mutable connection
/// state. Use it from a single async task.
#[derive(Debug)]
pub struct ReplContext {
    inner: ReplContextInner,
    /// Session-level remote target (from `id repl <NODE_ID>`) - reserved for future use
    #[allow(dead_code)]
    session_target: Option<EndpointId>,
}

/// Internal state for different REPL operating modes.
///
/// This enum represents the three possible connection states:
///
/// - [`Local`](ReplContextInner::Local): Direct store access, no networking
/// - [`Remote`](ReplContextInner::Remote): Connected to local serve instance
/// - [`RemoteNode`](ReplContextInner::RemoteNode): Connected to remote peer
#[derive(Debug)]
pub enum ReplContextInner {
    /// Connected to a running serve instance on the local machine.
    ///
    /// Uses QUIC connections to the local serve for all operations.
    /// The serve instance manages the actual blob store.
    Remote {
        /// QUIC endpoint for creating connections
        endpoint: Endpoint,
        /// Address of the local serve instance
        endpoint_addr: iroh::EndpointAddr,
        /// Cached `META_ALPN` connection (lazy, reconnects if closed)
        meta_conn: Option<Connection>,
        /// Cached `BLOBS_ALPN` connection (lazy, reconnects if closed)
        blobs_conn: Option<Connection>,
        /// Ephemeral store for blob transfers
        store: StoreType,
    },
    /// Direct local store access (no serve instance running).
    ///
    /// All operations go directly to the local blob store.
    /// No networking is available in this mode.
    Local {
        /// The local blob store
        store: StoreType,
    },
    /// Connected to a remote peer node over the network.
    ///
    /// Uses DNS-based address lookup to find the peer and
    /// establishes QUIC connections for operations.
    RemoteNode {
        /// QUIC endpoint for creating connections
        endpoint: Endpoint,
        /// The remote peer's node ID
        node_id: EndpointId,
        /// Cached `META_ALPN` connection (lazy, reconnects if closed)
        meta_conn: Option<Connection>,
        /// Cached `BLOBS_ALPN` connection (lazy, reconnects if closed)
        blobs_conn: Option<Connection>,
        /// Local store for blob transfers
        store: StoreType,
    },
}

impl ReplContext {
    /// Create a new REPL context with automatic mode detection.
    ///
    /// # Arguments
    ///
    /// * `target_node` - Optional remote node ID to connect to.
    ///   - If `Some(node_id)`, connects to that remote peer
    ///   - If `None`, checks for local serve; if not running, uses direct local access
    ///
    /// # Mode Selection
    ///
    /// 1. If `target_node` is provided, creates `RemoteNode` context
    /// 2. Otherwise, checks for running serve via `get_serve_info()`
    /// 3. If serve is running, creates `Remote` context
    /// 4. Otherwise, creates `Local` context
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - `target_node` is not a valid 64-character hex node ID
    /// - Cannot bind the QUIC endpoint
    /// - Cannot open the blob store
    pub async fn new(target_node: Option<String>) -> Result<Self> {
        // If a target node is specified, connect to that remote node
        if let Some(node_str) = target_node {
            if !is_node_id(&node_str) {
                bail!("invalid node ID: must be 64 hex characters");
            }
            let node_id: EndpointId = node_str.parse()?;

            let client_key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
            let endpoint = Endpoint::builder()
                .secret_key(client_key)
                .address_lookup(PkarrPublisher::n0_dns())
                .address_lookup(DnsAddressLookup::n0_dns())
                .bind()
                .await?;

            let store = open_store(true).await?;
            return Ok(Self {
                inner: ReplContextInner::RemoteNode {
                    endpoint,
                    node_id,
                    meta_conn: None,
                    blobs_conn: None,
                    store,
                },
                session_target: Some(node_id),
            });
        }

        if let Some(serve_info) = get_serve_info().await {
            let (endpoint, endpoint_addr) = create_local_client_endpoint(&serve_info).await?;
            // Use ephemeral store for remote mode (just for blob transfers)
            let store = open_store(true).await?;
            Ok(Self {
                inner: ReplContextInner::Remote {
                    endpoint,
                    endpoint_addr,
                    meta_conn: None,
                    blobs_conn: None,
                    store,
                },
                session_target: None,
            })
        } else {
            let store = open_store(false).await?;
            Ok(Self {
                inner: ReplContextInner::Local { store },
                session_target: None,
            })
        }
    }

    /// Get a human-readable string describing the current mode.
    ///
    /// Returns:
    /// - `"local-serve"` for Remote mode
    /// - `"local"` for Local mode
    /// - `"remote:XXXXXXXX"` for `RemoteNode` mode (first 8 chars of node ID)
    pub fn mode_str(&self) -> String {
        match &self.inner {
            ReplContextInner::Remote { .. } => "local-serve".to_owned(),
            ReplContextInner::Local { .. } => "local".to_owned(),
            ReplContextInner::RemoteNode { node_id, .. } => {
                format!("remote:{}", &node_id.to_string()[..8])
            }
        }
    }

    /// Check if connected to a server (local serve or remote node).
    ///
    /// Returns `true` for Remote and `RemoteNode` modes, `false` for Local mode.
    /// This affects how operations are performed (protocol vs direct store access).
    pub const fn is_connected(&self) -> bool {
        matches!(
            &self.inner,
            ReplContextInner::Remote { .. } | ReplContextInner::RemoteNode { .. }
        )
    }

    /// Get a handle to the blob store.
    ///
    /// Returns a [`Store`] handle that can be used for blob operations.
    /// Works in all modes - the store is either:
    /// - The main store (Local mode)
    /// - An ephemeral transfer store (Remote/RemoteNode modes)
    #[allow(clippy::match_same_arms)] // Different enum variants, same action - intentional for exhaustiveness
    pub fn store_handle(&self) -> Store {
        match &self.inner {
            ReplContextInner::Remote { store, .. } => store.as_store(),
            ReplContextInner::Local { store } => store.as_store(),
            ReplContextInner::RemoteNode { store, .. } => store.as_store(),
        }
    }

    /// Get or create a connection for metadata operations (`META_ALPN`).
    ///
    /// This method lazily establishes a connection to the serve instance or
    /// remote node. The connection is cached and reused for subsequent calls.
    /// If the connection has closed, a new one is automatically created.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Called in Local mode (no server to connect to)
    /// - Cannot establish a QUIC connection
    #[allow(clippy::unwrap_used)] // Safe: we just assigned Some(conn) before unwrapping
    pub async fn meta_conn(&mut self) -> Result<&Connection> {
        match &mut self.inner {
            ReplContextInner::Remote {
                endpoint,
                endpoint_addr,
                meta_conn,
                ..
            } => {
                if let Some(conn) = meta_conn.as_ref()
                    && conn.close_reason().is_none()
                {
                    return Ok(meta_conn.as_ref().unwrap());
                }
                let conn = endpoint.connect(endpoint_addr.clone(), META_ALPN).await?;
                *meta_conn = Some(conn);
                Ok(meta_conn.as_ref().unwrap())
            }
            ReplContextInner::RemoteNode {
                endpoint,
                node_id,
                meta_conn,
                ..
            } => {
                if let Some(conn) = meta_conn.as_ref()
                    && conn.close_reason().is_none()
                {
                    return Ok(meta_conn.as_ref().unwrap());
                }
                let conn = endpoint.connect(*node_id, META_ALPN).await?;
                *meta_conn = Some(conn);
                Ok(meta_conn.as_ref().unwrap())
            }
            ReplContextInner::Local { .. } => bail!("meta_conn called in local mode"),
        }
    }

    /// Get or create a connection for blob transfers (`BLOBS_ALPN`).
    ///
    /// Similar to [`meta_conn()`](Self::meta_conn), this lazily establishes
    /// and caches a connection for the iroh-blobs protocol.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Called in Local mode (no server to connect to)
    /// - Cannot establish a QUIC connection
    #[allow(clippy::unwrap_used)] // Safe: we just assigned Some(conn) before unwrapping
    pub async fn blobs_conn(&mut self) -> Result<&Connection> {
        match &mut self.inner {
            ReplContextInner::Remote {
                endpoint,
                endpoint_addr,
                blobs_conn,
                ..
            } => {
                if let Some(conn) = blobs_conn.as_ref()
                    && conn.close_reason().is_none()
                {
                    return Ok(blobs_conn.as_ref().unwrap());
                }
                let conn = endpoint.connect(endpoint_addr.clone(), BLOBS_ALPN).await?;
                *blobs_conn = Some(conn);
                Ok(blobs_conn.as_ref().unwrap())
            }
            ReplContextInner::RemoteNode {
                endpoint,
                node_id,
                blobs_conn,
                ..
            } => {
                if let Some(conn) = blobs_conn.as_ref()
                    && conn.close_reason().is_none()
                {
                    return Ok(blobs_conn.as_ref().unwrap());
                }
                let conn = endpoint.connect(*node_id, BLOBS_ALPN).await?;
                *blobs_conn = Some(conn);
                Ok(blobs_conn.as_ref().unwrap())
            }
            ReplContextInner::Local { .. } => bail!("blobs_conn called in local mode"),
        }
    }

    /// Get the QUIC endpoint for creating ad-hoc connections.
    ///
    /// Returns `None` in Local mode (no networking available).
    /// Used by `@NODE_ID` targeting to create connections to arbitrary nodes.
    #[allow(clippy::match_same_arms)] // Different enum variants - intentional for exhaustiveness
    pub const fn endpoint(&self) -> Option<&Endpoint> {
        match &self.inner {
            ReplContextInner::Remote { endpoint, .. } => Some(endpoint),
            ReplContextInner::RemoteNode { endpoint, .. } => Some(endpoint),
            ReplContextInner::Local { .. } => None,
        }
    }

    /// List all stored blobs.
    ///
    /// Prints a tab-separated list of hash and name for each stored blob.
    /// Output format: `<hash>\t<name>`
    ///
    /// In connected modes, sends `MetaRequest::List` to the server.
    /// In local mode, directly iterates the store's tags.
    pub async fn list(&mut self) -> Result<()> {
        if matches!(
            &self.inner,
            ReplContextInner::Remote { .. } | ReplContextInner::RemoteNode { .. }
        ) {
            let meta_conn = self.meta_conn().await?;
            let (mut send, mut recv) = meta_conn.open_bi().await?;
            let req = postcard::to_allocvec(&MetaRequest::List)?;
            send.write_all(&req).await?;
            send.finish()?;
            let resp_buf = recv.read_to_end(1024 * 1024).await?;
            let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;

            match resp {
                MetaResponse::List { items } => {
                    if items.is_empty() {
                        println!("(no files stored)");
                    } else {
                        for (hash, name) in items {
                            println!("{hash}\t{name}");
                        }
                    }
                }
                _ => bail!("unexpected response"),
            }
        } else if let ReplContextInner::Local { store } = &self.inner {
            let store_handle = store.as_store();
            let mut list = store_handle.tags().list().await?;
            let mut count = 0;
            while let Some(item) = list.next().await {
                let item = item?;
                let name = String::from_utf8_lossy(item.name.as_ref());
                println!("{}\t{}", item.hash, name);
                count += 1;
            }
            if count == 0 {
                println!("(no files stored)");
            }
        }
        Ok(())
    }

    /// Store a blob with an optional custom name.
    ///
    /// # Arguments
    ///
    /// * `path` - Source path, or `"-"` for stdin, or `__STDIN_CONTENT__:data` for inline content
    /// * `name` - Optional custom name; if None, uses the filename from path
    ///
    /// # Protocol Flow (connected mode)
    ///
    /// 1. Read data from source and add to local store
    /// 2. Send `MetaRequest::Put { filename, hash }` to register the name
    /// 3. Push blob data via `BLOBS_ALPN` connection
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Path cannot be read
    /// - Stdin input without a name provided
    /// - Server rejects the put request
    pub async fn put(&mut self, path: &str, name: Option<&str>) -> Result<()> {
        let (data, filename) = if let Some(content) = path.strip_prefix("__STDIN_CONTENT__:") {
            let name = name.ok_or_else(|| anyhow!("content requires a name"))?;
            (content.as_bytes().to_vec(), name.to_owned())
        } else if path == "-" {
            let name = name.ok_or_else(|| anyhow!("stdin requires a name: put - <NAME>"))?;
            let mut data = Vec::new();
            std::io::stdin().read_to_end(&mut data)?;
            (data, name.to_owned())
        } else {
            let path_buf = PathBuf::from(path);
            let data = afs::read(&path_buf).await?;
            let filename = name.map_or_else(
                || {
                    path_buf
                        .file_name()
                        .map_or_else(|| "unnamed".to_owned(), |f| f.to_string_lossy().to_string())
                },
                ToOwned::to_owned,
            );
            (data, filename)
        };

        if self.is_connected() {
            let hash = {
                let store_handle = self.store_handle();
                let result = store_handle
                    .add_bytes_with_opts(AddBytesOptions {
                        data: data.into(),
                        format: BlobFormat::Raw,
                    })
                    .await?;
                result.hash
            };

            let meta_conn = self.meta_conn().await?;
            let (mut send, mut recv) = meta_conn.open_bi().await?;
            let req = postcard::to_allocvec(&MetaRequest::Put {
                filename: filename.clone(),
                hash,
            })?;
            send.write_all(&req).await?;
            send.finish()?;
            let resp_buf = recv.read_to_end(64 * 1024).await?;
            let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;

            match resp {
                MetaResponse::Put { success: true } => {
                    let blobs_conn = self.blobs_conn().await?.clone();
                    let store_handle = self.store_handle();
                    let push_request =
                        PushRequest::new(hash, ChunkRangesSeq::from_ranges([ChunkRanges::all()]));
                    store_handle
                        .remote()
                        .execute_push(blobs_conn, push_request)
                        .await?;
                    println!("stored: {filename} -> {hash}");
                }
                MetaResponse::Put { success: false } => bail!("server rejected"),
                _ => bail!("unexpected response"),
            }
        } else {
            let store_handle = self.store_handle();
            let result = store_handle
                .add_bytes_with_opts(AddBytesOptions {
                    data: data.into(),
                    format: BlobFormat::Raw,
                })
                .await?;
            store_handle.tags().set(&filename, result.hash).await?;
            println!("stored: {} -> {}", filename, result.hash);
        }
        Ok(())
    }

    /// Retrieve a blob by name and export to a destination.
    ///
    /// # Arguments
    ///
    /// * `name` - The tag name to look up
    /// * `output` - Optional destination; defaults to `name`. Use `"-"` for stdout.
    ///
    /// # Protocol Flow (connected mode)
    ///
    /// 1. Send `MetaRequest::Get { filename }` to resolve name → hash
    /// 2. Fetch blob data via `BLOBS_ALPN` connection
    /// 3. Export to the destination
    ///
    /// # Errors
    ///
    /// Returns an error if the name is not found or export fails.
    pub async fn get(&mut self, name: &str, output: Option<&str>) -> Result<()> {
        let output = output.unwrap_or(name);

        if self.is_connected() {
            let meta_conn = self.meta_conn().await?;
            let (mut send, mut recv) = meta_conn.open_bi().await?;
            let req = postcard::to_allocvec(&MetaRequest::Get {
                filename: name.to_owned(),
            })?;
            send.write_all(&req).await?;
            send.finish()?;
            let resp_buf = recv.read_to_end(64 * 1024).await?;
            let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;

            match resp {
                MetaResponse::Get { hash: Some(hash) } => {
                    let blobs_conn = self.blobs_conn().await?.clone();
                    let store_handle = self.store_handle();
                    store_handle.remote().fetch(blobs_conn, hash).await?;
                    export_blob(&store_handle, hash, output).await?;
                }
                MetaResponse::Get { hash: None } => bail!("not found: {name}"),
                _ => bail!("unexpected response"),
            }
        } else {
            let store_handle = self.store_handle();
            let tag = store_handle
                .tags()
                .get(name)
                .await?
                .ok_or_else(|| anyhow!("not found: {name}"))?;
            export_blob(&store_handle, tag.hash, output).await?;
        }
        Ok(())
    }

    /// Retrieve a blob by its content hash.
    ///
    /// # Arguments
    ///
    /// * `hash_str` - The blob's hash as a hex string
    /// * `output` - Destination path, or `"-"` for stdout
    ///
    /// # Errors
    ///
    /// Returns an error if the hash is invalid or the blob cannot be found.
    pub async fn gethash(&mut self, hash_str: &str, output: &str) -> Result<()> {
        let hash: Hash = hash_str.parse().map_err(|e| anyhow!("invalid hash: {e}"))?;

        if self.is_connected() {
            let blobs_conn = self.blobs_conn().await?.clone();
            let store_handle = self.store_handle();
            store_handle.remote().fetch(blobs_conn, hash).await?;
            export_blob(&store_handle, hash, output).await?;
        } else {
            let store_handle = self.store_handle();
            export_blob(&store_handle, hash, output).await?;
        }
        Ok(())
    }

    /// Delete a blob by name.
    ///
    /// Removes the tag (name → hash mapping). The underlying blob data may
    /// be garbage collected if no other tags reference it.
    ///
    /// # Errors
    ///
    /// Returns an error if the name is not found.
    pub async fn delete(&mut self, name: &str) -> Result<()> {
        if self.is_connected() {
            let meta_conn = self.meta_conn().await?;
            let (mut send, mut recv) = meta_conn.open_bi().await?;
            let req = postcard::to_allocvec(&MetaRequest::Delete {
                filename: name.to_owned(),
            })?;
            send.write_all(&req).await?;
            send.finish()?;
            let resp_buf = recv.read_to_end(64 * 1024).await?;
            let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;

            match resp {
                MetaResponse::Delete { success: true } => println!("deleted: {name}"),
                MetaResponse::Delete { success: false } => bail!("not found: {name}"),
                _ => bail!("unexpected response"),
            }
        } else {
            let store_handle = self.store_handle();
            store_handle.tags().delete(name).await?;
            println!("deleted: {name}");
        }
        Ok(())
    }

    /// Rename a blob (change its tag name).
    ///
    /// This creates a new tag with the same hash and deletes the old tag.
    /// The underlying blob data is not affected.
    ///
    /// # Arguments
    ///
    /// * `from` - Current name
    /// * `to` - New name
    ///
    /// # Errors
    ///
    /// Returns an error if `from` is not found.
    pub async fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        if self.is_connected() {
            let meta_conn = self.meta_conn().await?;
            let (mut send, mut recv) = meta_conn.open_bi().await?;
            let req = postcard::to_allocvec(&MetaRequest::Rename {
                from: from.to_owned(),
                to: to.to_owned(),
            })?;
            send.write_all(&req).await?;
            send.finish()?;
            let resp_buf = recv.read_to_end(64 * 1024).await?;
            let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;

            match resp {
                MetaResponse::Rename { success: true } => println!("renamed: {from} -> {to}"),
                MetaResponse::Rename { success: false } => bail!("not found: {from}"),
                _ => bail!("unexpected response"),
            }
        } else {
            let store_handle = self.store_handle();
            let tag = store_handle
                .tags()
                .get(from)
                .await?
                .ok_or_else(|| anyhow!("not found: {from}"))?;
            store_handle.tags().set(to, tag.hash).await?;
            store_handle.tags().delete(from).await?;
            println!("renamed: {from} -> {to}");
        }
        Ok(())
    }

    /// Copy a blob to a new name.
    ///
    /// Creates a new tag pointing to the same blob hash. This is a metadata
    /// operation only - no data is duplicated.
    ///
    /// # Arguments
    ///
    /// * `from` - Source name
    /// * `to` - New name for the copy
    ///
    /// # Errors
    ///
    /// Returns an error if `from` is not found.
    pub async fn copy(&mut self, from: &str, to: &str) -> Result<()> {
        if self.is_connected() {
            let meta_conn = self.meta_conn().await?;
            let (mut send, mut recv) = meta_conn.open_bi().await?;
            let req = postcard::to_allocvec(&MetaRequest::Copy {
                from: from.to_owned(),
                to: to.to_owned(),
            })?;
            send.write_all(&req).await?;
            send.finish()?;
            let resp_buf = recv.read_to_end(64 * 1024).await?;
            let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;

            match resp {
                MetaResponse::Copy { success: true } => println!("copied: {from} -> {to}"),
                MetaResponse::Copy { success: false } => bail!("not found: {from}"),
                _ => bail!("unexpected response"),
            }
        } else {
            let store_handle = self.store_handle();
            let tag = store_handle
                .tags()
                .get(from)
                .await?
                .ok_or_else(|| anyhow!("not found: {from}"))?;
            store_handle.tags().set(to, tag.hash).await?;
            println!("copied: {from} -> {to}");
        }
        Ok(())
    }

    /// Search for blobs matching a query.
    ///
    /// Performs case-insensitive matching against tag names and blob hashes.
    /// Returns matches sorted by relevance (exact > prefix > contains).
    ///
    /// # Arguments
    ///
    /// * `query` - Search pattern
    /// * `prefer_name` - If true, name matches sort before hash matches
    ///
    /// # Returns
    ///
    /// A vector of [`FindMatch`] entries describing each match.
    pub async fn find(&mut self, query: &str, prefer_name: bool) -> Result<Vec<FindMatch>> {
        let matches = if self.is_connected() {
            let meta_conn = self.meta_conn().await?;
            let (mut send, mut recv) = meta_conn.open_bi().await?;
            let req = postcard::to_allocvec(&MetaRequest::Find {
                query: query.to_owned(),
                prefer_name,
            })?;
            send.write_all(&req).await?;
            send.finish()?;
            let resp_buf = recv.read_to_end(64 * 1024).await?;
            let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;

            match resp {
                MetaResponse::Find { matches } => matches,
                _ => bail!("unexpected response"),
            }
        } else {
            let store_handle = self.store_handle();
            let mut matches = Vec::new();
            let query_lower = query.to_lowercase();

            if let Ok(mut list) = store_handle.tags().list().await {
                while let Some(item) = list.next().await {
                    if let Ok(item) = item {
                        let name = String::from_utf8_lossy(item.name.as_ref()).to_string();
                        let hash_str = item.hash.to_string();
                        let name_lower = name.to_lowercase();

                        if let Some(kind) = Self::match_kind(&name_lower, &query_lower) {
                            matches.push(FindMatch {
                                hash: item.hash,
                                name: name.clone(),
                                kind,
                                is_hash_match: false,
                            });
                        } else if let Some(kind) = Self::match_kind(&hash_str, &query_lower) {
                            matches.push(FindMatch {
                                hash: item.hash,
                                name,
                                kind,
                                is_hash_match: true,
                            });
                        }
                    }
                }
            }

            matches.sort_by(|a, b| match a.kind.cmp(&b.kind) {
                std::cmp::Ordering::Equal => {
                    if prefer_name {
                        a.is_hash_match.cmp(&b.is_hash_match)
                    } else {
                        b.is_hash_match.cmp(&a.is_hash_match)
                    }
                }
                other => other,
            });

            matches
        };

        Ok(matches)
    }

    /// Determine the type of match between a haystack and needle.
    ///
    /// Used internally by the find operation to categorize matches.
    ///
    /// # Returns
    ///
    /// - `Some(MatchKind::Exact)` if haystack equals needle
    /// - `Some(MatchKind::Prefix)` if haystack starts with needle
    /// - `Some(MatchKind::Contains)` if haystack contains needle
    /// - `None` if no match
    fn match_kind(haystack: &str, needle: &str) -> Option<MatchKind> {
        if haystack == needle {
            Some(MatchKind::Exact)
        } else if haystack.starts_with(needle) {
            Some(MatchKind::Prefix)
        } else if haystack.contains(needle) {
            Some(MatchKind::Contains)
        } else {
            None
        }
    }

    /// List files on a specific remote node using @`NODE_ID` syntax.
    ///
    /// This creates a one-off connection to the specified node and lists
    /// its stored blobs. Requires connected mode (serve must be running).
    ///
    /// # Arguments
    ///
    /// * `node_str` - The 64-character hex node ID
    ///
    /// # Errors
    ///
    /// Returns an error if not in connected mode or connection fails.
    pub async fn list_on_node(&mut self, node_str: &str) -> Result<()> {
        let node_id: EndpointId = node_str.parse()?;
        let endpoint = self.endpoint().ok_or_else(|| {
            anyhow!("@NODE_ID requires a connected mode (use 'id repl' with a running serve)")
        })?;

        let conn = endpoint.connect(node_id, META_ALPN).await?;
        let (mut send, mut recv) = conn.open_bi().await?;
        let req = postcard::to_allocvec(&MetaRequest::List)?;
        send.write_all(&req).await?;
        send.finish()?;
        let resp_buf = recv.read_to_end(1024 * 1024).await?;
        let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;

        match resp {
            MetaResponse::List { items } => {
                if items.is_empty() {
                    println!("(no files stored on @{})", &node_str[..8]);
                } else {
                    for (hash, name) in items {
                        println!("{hash}\t{name}");
                    }
                }
            }
            _ => bail!("unexpected response"),
        }
        conn.close(0u32.into(), b"done");
        Ok(())
    }

    /// Store a file on a specific remote node using @`NODE_ID` syntax.
    ///
    /// Uploads the file to the specified remote peer. The blob data is
    /// pushed after the metadata is registered on the remote.
    ///
    /// # Arguments
    ///
    /// * `node_str` - The 64-character hex node ID
    /// * `path` - Source path, `"-"` for stdin, or `__STDIN_CONTENT__:data`
    /// * `name` - Optional custom name
    ///
    /// # Errors
    ///
    /// Returns an error if not in connected mode or the operation fails.
    pub async fn put_on_node(
        &mut self,
        node_str: &str,
        path: &str,
        name: Option<&str>,
    ) -> Result<()> {
        let node_id: EndpointId = node_str.parse()?;
        let endpoint = self.endpoint().ok_or_else(|| {
            anyhow!("@NODE_ID requires a connected mode (use 'id repl' with a running serve)")
        })?;

        let (data, filename) = if let Some(content) = path.strip_prefix("__STDIN_CONTENT__:") {
            let name = name.ok_or_else(|| anyhow!("content requires a name"))?;
            (content.as_bytes().to_vec(), name.to_owned())
        } else if path == "-" {
            let name = name.ok_or_else(|| anyhow!("stdin requires a name: put - <NAME>"))?;
            let mut data = Vec::new();
            std::io::stdin().read_to_end(&mut data)?;
            (data, name.to_owned())
        } else {
            let path_buf = PathBuf::from(path);
            let data = afs::read(&path_buf).await?;
            let filename = name.map_or_else(
                || {
                    path_buf
                        .file_name()
                        .map_or_else(|| "unnamed".to_owned(), |f| f.to_string_lossy().to_string())
                },
                ToOwned::to_owned,
            );
            (data, filename)
        };

        let hash = {
            let store_handle = self.store_handle();
            let result = store_handle
                .add_bytes_with_opts(AddBytesOptions {
                    data: data.into(),
                    format: BlobFormat::Raw,
                })
                .await?;
            result.hash
        };

        let meta_conn = endpoint.connect(node_id, META_ALPN).await?;
        let (mut send, mut recv) = meta_conn.open_bi().await?;
        let req = postcard::to_allocvec(&MetaRequest::Put {
            filename: filename.clone(),
            hash,
        })?;
        send.write_all(&req).await?;
        send.finish()?;
        let resp_buf = recv.read_to_end(64 * 1024).await?;
        let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;

        match resp {
            MetaResponse::Put { success: true } => {
                let blobs_conn = endpoint.connect(node_id, BLOBS_ALPN).await?;
                let store_handle = self.store_handle();
                let push_request =
                    PushRequest::new(hash, ChunkRangesSeq::from_ranges([ChunkRanges::all()]));
                store_handle
                    .remote()
                    .execute_push(blobs_conn, push_request)
                    .await?;
                println!("stored: {} -> {} (@{})", filename, hash, &node_str[..8]);
            }
            MetaResponse::Put { success: false } => bail!("server rejected"),
            _ => bail!("unexpected response"),
        }
        meta_conn.close(0u32.into(), b"done");
        Ok(())
    }

    /// Retrieve a file from a specific remote node using @`NODE_ID` syntax.
    ///
    /// Downloads a blob from the specified remote peer by name.
    ///
    /// # Arguments
    ///
    /// * `node_str` - The 64-character hex node ID
    /// * `name` - The tag name to retrieve
    /// * `output` - Optional destination; defaults to `name`. Use `"-"` for stdout.
    ///
    /// # Errors
    ///
    /// Returns an error if not in connected mode, the file is not found,
    /// or the operation fails.
    pub async fn get_on_node(
        &mut self,
        node_str: &str,
        name: &str,
        output: Option<&str>,
    ) -> Result<()> {
        let node_id: EndpointId = node_str.parse()?;
        let endpoint = self.endpoint().ok_or_else(|| {
            anyhow!("@NODE_ID requires a connected mode (use 'id repl' with a running serve)")
        })?;
        let output = output.unwrap_or(name);

        let meta_conn = endpoint.connect(node_id, META_ALPN).await?;
        let (mut send, mut recv) = meta_conn.open_bi().await?;
        let req = postcard::to_allocvec(&MetaRequest::Get {
            filename: name.to_owned(),
        })?;
        send.write_all(&req).await?;
        send.finish()?;
        let resp_buf = recv.read_to_end(64 * 1024).await?;
        let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;

        match resp {
            MetaResponse::Get { hash: Some(hash) } => {
                let blobs_conn = endpoint.connect(node_id, BLOBS_ALPN).await?;
                let store_handle = self.store_handle();
                store_handle.remote().fetch(blobs_conn, hash).await?;
                export_blob(&store_handle, hash, output).await?;
            }
            MetaResponse::Get { hash: None } => bail!("not found: {} (@{})", name, &node_str[..8]),
            _ => bail!("unexpected response"),
        }
        meta_conn.close(0u32.into(), b"done");
        Ok(())
    }

    /// Delete a file on a specific remote node using @`NODE_ID` syntax.
    ///
    /// Removes a tag from the specified remote peer.
    ///
    /// # Arguments
    ///
    /// * `node_str` - The 64-character hex node ID
    /// * `name` - The tag name to delete
    ///
    /// # Errors
    ///
    /// Returns an error if not in connected mode, the file is not found,
    /// or the operation fails.
    pub async fn delete_on_node(&mut self, node_str: &str, name: &str) -> Result<()> {
        let node_id: EndpointId = node_str.parse()?;
        let endpoint = self.endpoint().ok_or_else(|| {
            anyhow!("@NODE_ID requires a connected mode (use 'id repl' with a running serve)")
        })?;

        let conn = endpoint.connect(node_id, META_ALPN).await?;
        let (mut send, mut recv) = conn.open_bi().await?;
        let req = postcard::to_allocvec(&MetaRequest::Delete {
            filename: name.to_owned(),
        })?;
        send.write_all(&req).await?;
        send.finish()?;
        let resp_buf = recv.read_to_end(64 * 1024).await?;
        let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;

        match resp {
            MetaResponse::Delete { success: true } => {
                println!("deleted: {} (@{})", name, &node_str[..8]);
            }
            MetaResponse::Delete { success: false } => {
                bail!("not found: {} (@{})", name, &node_str[..8])
            }
            _ => bail!("unexpected response"),
        }
        conn.close(0u32.into(), b"done");
        Ok(())
    }

    /// Gracefully shut down the REPL context.
    ///
    /// Closes all connections and shuts down the blob store.
    /// This should be called when exiting the REPL.
    pub async fn shutdown(self) -> Result<()> {
        match self.inner {
            ReplContextInner::Remote {
                meta_conn,
                blobs_conn,
                store,
                ..
            } => {
                if let Some(conn) = meta_conn {
                    conn.close(0u32.into(), b"done");
                }
                if let Some(conn) = blobs_conn {
                    conn.close(0u32.into(), b"done");
                }
                store.shutdown().await?;
            }
            ReplContextInner::RemoteNode {
                meta_conn,
                blobs_conn,
                store,
                ..
            } => {
                if let Some(conn) = meta_conn {
                    conn.close(0u32.into(), b"done");
                }
                if let Some(conn) = blobs_conn {
                    conn.close(0u32.into(), b"done");
                }
                store.shutdown().await?;
            }
            ReplContextInner::Local { store } => {
                store.shutdown().await?;
            }
        }
        Ok(())
    }
}
