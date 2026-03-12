use anyhow::{Context, Result, anyhow, bail};
use futures_lite::StreamExt;
use iroh::{
    address_lookup::{DnsAddressLookup, PkarrPublisher},
    endpoint::{Connection, Endpoint, RelayMode},
    protocol::{AcceptError, ProtocolHandler, Router},
};
use iroh_base::{EndpointAddr, EndpointId, SecretKey, TransportAddr};
use iroh_blobs::{
    ALPN as BLOBS_ALPN, BlobFormat, BlobsProtocol, Hash,
    api::{Store, blobs::AddBytesOptions},
    protocol::{ChunkRanges, ChunkRangesSeq, PushRequest},
    store::{fs::FsStore, mem::MemStore},
};
use rustyline::{DefaultEditor, error::ReadlineError};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    path::PathBuf,
    sync::Arc,
};
use tokio::fs as afs;
use tracing::info;

const KEY_FILE: &str = ".iroh-key";
const CLIENT_KEY_FILE: &str = ".iroh-key-client";
const STORE_PATH: &str = ".iroh-store";
const SERVE_LOCK: &str = ".iroh-serve.lock";
const META_ALPN: &[u8] = b"/iroh-meta/1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum MatchKind {
    Exact,    // Best: exact match
    Prefix,   // Good: starts with query
    Contains, // Okay: contains query
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FindMatch {
    hash: Hash,
    name: String,
    kind: MatchKind,
    is_hash_match: bool, // true if matched against hash, false if matched against name
}

#[derive(Debug, Serialize, Deserialize)]
enum MetaRequest {
    Put { filename: String, hash: Hash },
    Get { filename: String },
    List,
    Delete { filename: String },
    Rename { from: String, to: String },
    Copy { from: String, to: String },
    Find { query: String, prefer_name: bool },
}

#[derive(Debug, Serialize, Deserialize)]
enum MetaResponse {
    Put { success: bool },
    Get { hash: Option<Hash> },
    List { items: Vec<(Hash, String)> },
    Delete { success: bool },
    Rename { success: bool },
    Copy { success: bool },
    Find { matches: Vec<FindMatch> },
}

#[derive(Clone, Debug)]
struct MetaProtocol {
    store: Store,
}

impl MetaProtocol {
    fn new(store: &Store) -> Arc<Self> {
        Arc::new(Self {
            store: store.clone(),
        })
    }

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
}

impl ProtocolHandler for MetaProtocol {
    async fn accept(&self, conn: Connection) -> std::result::Result<(), AcceptError> {
        // Handle multiple requests per connection
        loop {
            let (mut send, mut recv) = match conn.accept_bi().await {
                Ok(streams) => streams,
                Err(_) => break, // Connection closed
            };
            let buf = match recv.read_to_end(64 * 1024).await {
                Ok(buf) => buf,
                Err(_) => break,
            };
            let req: MetaRequest = match postcard::from_bytes(&buf) {
                Ok(req) => req,
                Err(_) => break,
            };
            match req {
                MetaRequest::Put { filename, hash } => {
                    self.store
                        .tags()
                        .set(&filename, hash)
                        .await
                        .map_err(AcceptError::from_err)?;
                    let resp = postcard::to_allocvec(&MetaResponse::Put { success: true })
                        .map_err(AcceptError::from_err)?;
                    send.write_all(&resp).await.map_err(AcceptError::from_err)?;
                    send.finish()?;
                }
                MetaRequest::Get { filename } => {
                    let mut found: Option<Hash> = None;
                    if let Ok(Some(tag)) = self.store.tags().get(&filename).await {
                        found = Some(tag.hash);
                    } else {
                        if let Ok(mut list) = self.store.tags().list().await {
                            while let Some(item) = list.next().await {
                                let item = item.map_err(AcceptError::from_err)?;
                                if item.name.as_ref() == filename.as_bytes() {
                                    found = Some(item.hash);
                                    break;
                                }
                            }
                        }
                    }
                    let resp = postcard::to_allocvec(&MetaResponse::Get { hash: found })
                        .map_err(AcceptError::from_err)?;
                    send.write_all(&resp).await.map_err(AcceptError::from_err)?;
                    send.finish()?;
                }
                MetaRequest::List => {
                    let mut items = Vec::new();
                    if let Ok(mut list) = self.store.tags().list().await {
                        while let Some(item) = list.next().await {
                            if let Ok(item) = item {
                                let name = String::from_utf8_lossy(item.name.as_ref()).to_string();
                                items.push((item.hash, name));
                            }
                        }
                    }
                    let resp = postcard::to_allocvec(&MetaResponse::List { items })
                        .map_err(AcceptError::from_err)?;
                    send.write_all(&resp).await.map_err(AcceptError::from_err)?;
                    send.finish()?;
                }
                MetaRequest::Delete { filename } => {
                    let success = self.store.tags().delete(&filename).await.is_ok();
                    let resp = postcard::to_allocvec(&MetaResponse::Delete { success })
                        .map_err(AcceptError::from_err)?;
                    send.write_all(&resp).await.map_err(AcceptError::from_err)?;
                    send.finish()?;
                }
                MetaRequest::Rename { from, to } => {
                    let success = if let Ok(Some(tag)) = self.store.tags().get(&from).await {
                        let hash = tag.hash;
                        if self.store.tags().set(&to, hash).await.is_ok() {
                            self.store.tags().delete(&from).await.is_ok()
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    let resp = postcard::to_allocvec(&MetaResponse::Rename { success })
                        .map_err(AcceptError::from_err)?;
                    send.write_all(&resp).await.map_err(AcceptError::from_err)?;
                    send.finish()?;
                }
                MetaRequest::Copy { from, to } => {
                    let success = if let Ok(Some(tag)) = self.store.tags().get(&from).await {
                        self.store.tags().set(&to, tag.hash).await.is_ok()
                    } else {
                        false
                    };
                    let resp = postcard::to_allocvec(&MetaResponse::Copy { success })
                        .map_err(AcceptError::from_err)?;
                    send.write_all(&resp).await.map_err(AcceptError::from_err)?;
                    send.finish()?;
                }
                MetaRequest::Find { query, prefer_name } => {
                    let mut matches = Vec::new();
                    let query_lower = query.to_lowercase();

                    if let Ok(mut list) = self.store.tags().list().await {
                        while let Some(item) = list.next().await {
                            if let Ok(item) = item {
                                let name = String::from_utf8_lossy(item.name.as_ref()).to_string();
                                let hash_str = item.hash.to_string();
                                let name_lower = name.to_lowercase();

                                // Check name matches
                                if let Some(kind) = Self::match_kind(&name_lower, &query_lower) {
                                    matches.push(FindMatch {
                                        hash: item.hash,
                                        name: name.clone(),
                                        kind,
                                        is_hash_match: false,
                                    });
                                }
                                // Check hash matches (only if no name match or query looks like a hash)
                                else if let Some(kind) = Self::match_kind(&hash_str, &query_lower)
                                {
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

                    // Sort: by match kind first, then by preference (hash vs name)
                    matches.sort_by(|a, b| {
                        match a.kind.cmp(&b.kind) {
                            std::cmp::Ordering::Equal => {
                                // If prefer_name, name matches come first (is_hash_match=false < true)
                                // If prefer_hash (default), hash matches come first (is_hash_match=true < false)
                                if prefer_name {
                                    a.is_hash_match.cmp(&b.is_hash_match)
                                } else {
                                    b.is_hash_match.cmp(&a.is_hash_match)
                                }
                            }
                            other => other,
                        }
                    });

                    let resp = postcard::to_allocvec(&MetaResponse::Find { matches })
                        .map_err(AcceptError::from_err)?;
                    send.write_all(&resp).await.map_err(AcceptError::from_err)?;
                    send.finish()?;
                }
            }
        }
        Ok(())
    }
}

async fn load_or_create_keypair(path: &str) -> Result<SecretKey> {
    match afs::read(path).await {
        Ok(bytes) => {
            let bytes: [u8; 32] = bytes
                .try_into()
                .map_err(|_| anyhow!("invalid key length"))?;
            Ok(SecretKey::from(bytes))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let key = SecretKey::generate(&mut rand::rng());
            afs::write(path, key.to_bytes()).await?;
            Ok(key)
        }
        Err(e) => Err(e.into()),
    }
}

enum StoreType {
    Persistent(FsStore),
    Ephemeral(MemStore),
}

impl StoreType {
    fn as_store(&self) -> Store {
        match self {
            StoreType::Persistent(s) => s.clone().into(),
            StoreType::Ephemeral(s) => s.clone().into(),
        }
    }

    async fn shutdown(self) -> Result<()> {
        match self {
            StoreType::Persistent(s) => s.shutdown().await?,
            StoreType::Ephemeral(s) => s.shutdown().await?,
        }
        Ok(())
    }
}

async fn open_store(ephemeral: bool) -> Result<StoreType> {
    if ephemeral {
        Ok(StoreType::Ephemeral(MemStore::new()))
    } else {
        let store = FsStore::load(STORE_PATH).await?;
        Ok(StoreType::Persistent(store))
    }
}

fn to_absolute(path: &PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.clone())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

async fn export_blob(store: &Store, hash: Hash, output: &str) -> Result<()> {
    if output == "-" {
        let data = store.blobs().get_bytes(hash).await?;
        std::io::stdout().write_all(&data)?;
    } else {
        let path = to_absolute(&PathBuf::from(output))?;
        store.blobs().export(hash, &path).await?;
        eprintln!("exported: {} -> {}", hash, path.display());
    }
    Ok(())
}

async fn read_input(input: &str) -> Result<Vec<u8>> {
    if input == "-" {
        let mut data = Vec::new();
        std::io::stdin().read_to_end(&mut data)?;
        Ok(data)
    } else {
        Ok(afs::read(input).await?)
    }
}

/// Info about a running serve instance
struct ServeInfo {
    node_id: EndpointId,
    addrs: Vec<SocketAddr>,
}

/// Check if serve is running by reading the lock file and verifying the PID
async fn get_serve_info() -> Option<ServeInfo> {
    let contents = afs::read_to_string(SERVE_LOCK).await.ok()?;
    let mut lines = contents.lines();
    let node_id_str = lines.next()?;
    let pid_str = lines.next()?;
    let pid: u32 = pid_str.parse().ok()?;

    // Check if process is still alive
    if !is_process_alive(pid) {
        // Stale lock file - remove it
        let _ = afs::remove_file(SERVE_LOCK).await;
        return None;
    }

    let node_id: EndpointId = node_id_str.parse().ok()?;

    // Parse socket addresses (remaining lines)
    let addrs: Vec<SocketAddr> = lines.filter_map(|line| line.parse().ok()).collect();

    Some(ServeInfo { node_id, addrs })
}

/// Check if a process with the given PID is still running
fn is_process_alive(pid: u32) -> bool {
    // On Unix, sending signal 0 checks if process exists without actually sending a signal
    #[cfg(unix)]
    {
        // kill -0 checks existence without sending a signal
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        // On non-Unix, just assume it's alive if we have a PID
        let _ = pid;
        true
    }
}

/// Create serve lock file with node ID, PID, and socket addresses
async fn create_serve_lock(node_id: &EndpointId, addrs: &[SocketAddr]) -> Result<()> {
    let pid = std::process::id();
    let mut contents = format!("{}\n{}", node_id, pid);
    for addr in addrs {
        contents.push_str(&format!("\n{}", addr));
    }
    afs::write(SERVE_LOCK, contents).await?;
    Ok(())
}

/// Remove serve lock file
async fn remove_serve_lock() -> Result<()> {
    let _ = afs::remove_file(SERVE_LOCK).await;
    Ok(())
}

/// Create a client endpoint configured to connect to the local serve
async fn create_local_client_endpoint(serve_info: &ServeInfo) -> Result<(Endpoint, EndpointAddr)> {
    let client_key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
    let endpoint = Endpoint::builder()
        .secret_key(client_key)
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await?;

    // Build EndpointAddr with known socket addresses to bypass DNS discovery
    // Prefer IPv4 localhost for reliability on systems with IPv6 issues
    let addrs: Vec<_> = serve_info
        .addrs
        .iter()
        .filter(|addr| addr.is_ipv4())
        .map(|addr| TransportAddr::Ip(*addr))
        .collect();

    // Fall back to all addresses if no IPv4 found
    let addrs = if addrs.is_empty() {
        serve_info
            .addrs
            .iter()
            .map(|addr| TransportAddr::Ip(*addr))
            .collect()
    } else {
        addrs
    };

    let endpoint_addr = EndpointAddr::from_parts(serve_info.node_id, addrs);

    Ok((endpoint, endpoint_addr))
}

/// REPL context - holds either remote connections or local store access
struct ReplContext {
    inner: ReplContextInner,
}

enum ReplContextInner {
    /// Connected to a running serve instance
    Remote {
        endpoint: Endpoint,
        endpoint_addr: EndpointAddr,
        meta_conn: Option<Connection>,
        blobs_conn: Option<Connection>,
        store: StoreType,
    },
    /// Direct local store access (no serve running)
    Local { store: StoreType },
}

impl ReplContext {
    async fn new() -> Result<Self> {
        if let Some(serve_info) = get_serve_info().await {
            let (endpoint, endpoint_addr) = create_local_client_endpoint(&serve_info).await?;
            // Use ephemeral store for remote mode (just for blob transfers)
            let store = open_store(true).await?;
            Ok(ReplContext {
                inner: ReplContextInner::Remote {
                    endpoint,
                    endpoint_addr,
                    meta_conn: None,
                    blobs_conn: None,
                    store,
                },
            })
        } else {
            let store = open_store(false).await?;
            Ok(ReplContext {
                inner: ReplContextInner::Local { store },
            })
        }
    }

    fn mode_str(&self) -> &'static str {
        match &self.inner {
            ReplContextInner::Remote { .. } => "remote",
            ReplContextInner::Local { .. } => "local",
        }
    }

    /// Get or create meta connection
    async fn meta_conn(&mut self) -> Result<&Connection> {
        match &mut self.inner {
            ReplContextInner::Remote {
                endpoint,
                endpoint_addr,
                meta_conn,
                ..
            } => {
                // Check if existing connection is still valid
                if let Some(conn) = meta_conn.as_ref() {
                    if conn.close_reason().is_none() {
                        return Ok(meta_conn.as_ref().unwrap());
                    }
                }
                // Create new connection
                let conn = endpoint.connect(endpoint_addr.clone(), META_ALPN).await?;
                *meta_conn = Some(conn);
                Ok(meta_conn.as_ref().unwrap())
            }
            ReplContextInner::Local { .. } => bail!("meta_conn called in local mode"),
        }
    }

    /// Get or create blobs connection
    async fn blobs_conn(&mut self) -> Result<&Connection> {
        match &mut self.inner {
            ReplContextInner::Remote {
                endpoint,
                endpoint_addr,
                blobs_conn,
                ..
            } => {
                // Check if existing connection is still valid
                if let Some(conn) = blobs_conn.as_ref() {
                    if conn.close_reason().is_none() {
                        return Ok(blobs_conn.as_ref().unwrap());
                    }
                }
                // Create new connection
                let conn = endpoint.connect(endpoint_addr.clone(), BLOBS_ALPN).await?;
                *blobs_conn = Some(conn);
                Ok(blobs_conn.as_ref().unwrap())
            }
            ReplContextInner::Local { .. } => bail!("blobs_conn called in local mode"),
        }
    }

    async fn list(&mut self) -> Result<()> {
        if matches!(&self.inner, ReplContextInner::Remote { .. }) {
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
                            println!("{}\t{}", hash, name);
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

    async fn put(&mut self, path: &str, name: Option<&str>) -> Result<()> {
        let (data, filename) = if path == "-" {
            let name = name.ok_or_else(|| anyhow!("stdin requires a name: put - <NAME>"))?;
            let mut data = Vec::new();
            std::io::stdin().read_to_end(&mut data)?;
            (data, name.to_string())
        } else {
            let path_buf = PathBuf::from(path);
            let data = afs::read(&path_buf).await?;
            let filename = name
                .map(|s| s.to_string())
                .unwrap_or_else(|| path_buf.file_name().unwrap().to_string_lossy().to_string());
            (data, filename)
        };

        if matches!(&self.inner, ReplContextInner::Remote { .. }) {
            // Add to local ephemeral store first
            let hash = {
                let store_handle = match &self.inner {
                    ReplContextInner::Remote { store, .. } => store.as_store(),
                    _ => unreachable!(),
                };
                let result = store_handle
                    .add_bytes_with_opts(AddBytesOptions {
                        data: data.into(),
                        format: BlobFormat::Raw,
                    })
                    .await?;
                result.hash
            };

            // Request server to accept
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
                    // Push blob to serve
                    let blobs_conn = self.blobs_conn().await?.clone();
                    let store_handle = match &self.inner {
                        ReplContextInner::Remote { store, .. } => store.as_store(),
                        _ => unreachable!(),
                    };
                    let push_request =
                        PushRequest::new(hash, ChunkRangesSeq::from_ranges([ChunkRanges::all()]));
                    store_handle
                        .remote()
                        .execute_push(blobs_conn, push_request)
                        .await?;
                    println!("stored: {} -> {}", filename, hash);
                }
                MetaResponse::Put { success: false } => bail!("server rejected"),
                _ => bail!("unexpected response"),
            }
        } else if let ReplContextInner::Local { store } = &self.inner {
            let store_handle = store.as_store();
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

    async fn get(&mut self, name: &str, output: Option<&str>) -> Result<()> {
        let output = output.unwrap_or(name);

        if matches!(&self.inner, ReplContextInner::Remote { .. }) {
            // Get hash from serve
            let meta_conn = self.meta_conn().await?;
            let (mut send, mut recv) = meta_conn.open_bi().await?;
            let req = postcard::to_allocvec(&MetaRequest::Get {
                filename: name.to_string(),
            })?;
            send.write_all(&req).await?;
            send.finish()?;
            let resp_buf = recv.read_to_end(64 * 1024).await?;
            let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;

            match resp {
                MetaResponse::Get { hash: Some(hash) } => {
                    // Fetch blob from serve
                    let blobs_conn = self.blobs_conn().await?.clone();
                    let store_handle = match &self.inner {
                        ReplContextInner::Remote { store, .. } => store.as_store(),
                        _ => unreachable!(),
                    };
                    store_handle.remote().fetch(blobs_conn, hash).await?;
                    export_blob(&store_handle, hash, output).await?;
                }
                MetaResponse::Get { hash: None } => bail!("not found: {}", name),
                _ => bail!("unexpected response"),
            }
        } else if let ReplContextInner::Local { store } = &self.inner {
            let store_handle = store.as_store();
            let tag = store_handle
                .tags()
                .get(name)
                .await?
                .ok_or_else(|| anyhow!("not found: {}", name))?;
            export_blob(&store_handle, tag.hash, output).await?;
        }
        Ok(())
    }

    async fn gethash(&mut self, hash_str: &str, output: &str) -> Result<()> {
        let hash: Hash = hash_str.parse().context("invalid hash")?;

        if matches!(&self.inner, ReplContextInner::Remote { .. }) {
            let blobs_conn = self.blobs_conn().await?.clone();
            let store_handle = match &self.inner {
                ReplContextInner::Remote { store, .. } => store.as_store(),
                _ => unreachable!(),
            };
            store_handle.remote().fetch(blobs_conn, hash).await?;
            export_blob(&store_handle, hash, output).await?;
        } else if let ReplContextInner::Local { store } = &self.inner {
            let store_handle = store.as_store();
            export_blob(&store_handle, hash, output).await?;
        }
        Ok(())
    }

    async fn delete(&mut self, name: &str) -> Result<()> {
        if matches!(&self.inner, ReplContextInner::Remote { .. }) {
            let meta_conn = self.meta_conn().await?;
            let (mut send, mut recv) = meta_conn.open_bi().await?;
            let req = postcard::to_allocvec(&MetaRequest::Delete {
                filename: name.to_string(),
            })?;
            send.write_all(&req).await?;
            send.finish()?;
            let resp_buf = recv.read_to_end(64 * 1024).await?;
            let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;

            match resp {
                MetaResponse::Delete { success: true } => println!("deleted: {}", name),
                MetaResponse::Delete { success: false } => bail!("not found: {}", name),
                _ => bail!("unexpected response"),
            }
        } else if let ReplContextInner::Local { store } = &self.inner {
            let store_handle = store.as_store();
            store_handle.tags().delete(name).await?;
            println!("deleted: {}", name);
        }
        Ok(())
    }

    async fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        if matches!(&self.inner, ReplContextInner::Remote { .. }) {
            let meta_conn = self.meta_conn().await?;
            let (mut send, mut recv) = meta_conn.open_bi().await?;
            let req = postcard::to_allocvec(&MetaRequest::Rename {
                from: from.to_string(),
                to: to.to_string(),
            })?;
            send.write_all(&req).await?;
            send.finish()?;
            let resp_buf = recv.read_to_end(64 * 1024).await?;
            let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;

            match resp {
                MetaResponse::Rename { success: true } => println!("renamed: {} -> {}", from, to),
                MetaResponse::Rename { success: false } => bail!("not found: {}", from),
                _ => bail!("unexpected response"),
            }
        } else if let ReplContextInner::Local { store } = &self.inner {
            let store_handle = store.as_store();
            let tag = store_handle
                .tags()
                .get(from)
                .await?
                .ok_or_else(|| anyhow!("not found: {}", from))?;
            store_handle.tags().set(to, tag.hash).await?;
            store_handle.tags().delete(from).await?;
            println!("renamed: {} -> {}", from, to);
        }
        Ok(())
    }

    async fn copy(&mut self, from: &str, to: &str) -> Result<()> {
        if matches!(&self.inner, ReplContextInner::Remote { .. }) {
            let meta_conn = self.meta_conn().await?;
            let (mut send, mut recv) = meta_conn.open_bi().await?;
            let req = postcard::to_allocvec(&MetaRequest::Copy {
                from: from.to_string(),
                to: to.to_string(),
            })?;
            send.write_all(&req).await?;
            send.finish()?;
            let resp_buf = recv.read_to_end(64 * 1024).await?;
            let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;

            match resp {
                MetaResponse::Copy { success: true } => println!("copied: {} -> {}", from, to),
                MetaResponse::Copy { success: false } => bail!("not found: {}", from),
                _ => bail!("unexpected response"),
            }
        } else if let ReplContextInner::Local { store } = &self.inner {
            let store_handle = store.as_store();
            let tag = store_handle
                .tags()
                .get(from)
                .await?
                .ok_or_else(|| anyhow!("not found: {}", from))?;
            store_handle.tags().set(to, tag.hash).await?;
            println!("copied: {} -> {}", from, to);
        }
        Ok(())
    }

    async fn find(&mut self, query: &str, prefer_name: bool) -> Result<Vec<FindMatch>> {
        let matches = if matches!(&self.inner, ReplContextInner::Remote { .. }) {
            let meta_conn = self.meta_conn().await?;
            let (mut send, mut recv) = meta_conn.open_bi().await?;
            let req = postcard::to_allocvec(&MetaRequest::Find {
                query: query.to_string(),
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
        } else if let ReplContextInner::Local { store } = &self.inner {
            let store_handle = store.as_store();
            let mut matches = Vec::new();
            let query_lower = query.to_lowercase();

            if let Ok(mut list) = store_handle.tags().list().await {
                while let Some(item) = list.next().await {
                    if let Ok(item) = item {
                        let name = String::from_utf8_lossy(item.name.as_ref()).to_string();
                        let hash_str = item.hash.to_string();
                        let name_lower = name.to_lowercase();

                        // Check name matches
                        if let Some(kind) = Self::match_kind(&name_lower, &query_lower) {
                            matches.push(FindMatch {
                                hash: item.hash,
                                name: name.clone(),
                                kind,
                                is_hash_match: false,
                            });
                        }
                        // Check hash matches
                        else if let Some(kind) = Self::match_kind(&hash_str, &query_lower) {
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

            // Sort by match kind, then by preference
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
        } else {
            Vec::new()
        };

        Ok(matches)
    }

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

    async fn shutdown(self) -> Result<()> {
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
            ReplContextInner::Local { store } => {
                store.shutdown().await?;
            }
        }
        Ok(())
    }
}

async fn run_repl() -> Result<()> {
    let mut ctx = ReplContext::new().await?;
    println!("id repl ({})", ctx.mode_str());
    println!("commands: list, put, get, cat, gethash, help, quit");

    let mut rl = DefaultEditor::new()?;

    loop {
        match rl.readline("> ") {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let _ = rl.add_history_entry(line);

                // Shell escape: !command
                if let Some(cmd) = line.strip_prefix('!') {
                    let cmd = cmd.trim();
                    if !cmd.is_empty() {
                        let status = std::process::Command::new("sh").arg("-c").arg(cmd).status();
                        match status {
                            Ok(s) if !s.success() => {
                                if let Some(code) = s.code() {
                                    println!("exit: {}", code);
                                }
                            }
                            Err(e) => println!("error: {}", e),
                            _ => {}
                        }
                    }
                    continue;
                }

                let parts: Vec<&str> = line.split_whitespace().collect();
                let result = match parts.as_slice() {
                    ["quit"] | ["exit"] | ["q"] => break,
                    ["help"] | ["?"] => {
                        println!("commands:");
                        println!("  list                   - List all stored files");
                        println!(
                            "  put <FILE> [NAME]      - Store file (NAME defaults to filename)"
                        );
                        println!(
                            "  get <NAME> [OUTPUT]    - Retrieve file (OUTPUT defaults to NAME, - for stdout)"
                        );
                        println!("  cat <NAME>             - Print file to stdout");
                        println!("  gethash <HASH> <OUTPUT> - Retrieve by hash (- for stdout)");
                        println!("  delete <NAME>          - Delete a file (alias: rm)");
                        println!("  rename <FROM> <TO>     - Rename a file");
                        println!("  copy <FROM> <TO>       - Copy a file (alias: cp)");
                        println!(
                            "  find <QUERY> [--name]  - Find files (exact/prefix/contains match)"
                        );
                        println!(
                            "  search <QUERY> [--name] - List all matches (no selection prompt)"
                        );
                        println!("  !<cmd>                 - Run shell command");
                        println!("  help                   - Show this help");
                        println!("  quit                   - Exit repl");
                        Ok(())
                    }
                    ["list"] | ["ls"] => ctx.list().await,
                    ["put", path] => ctx.put(path, None).await,
                    ["put", path, name] => ctx.put(path, Some(name)).await,
                    ["get", name] => ctx.get(name, None).await,
                    ["get", name, output] => ctx.get(name, Some(output)).await,
                    ["cat", name] => ctx.get(name, Some("-")).await,
                    ["gethash", hash, output] => ctx.gethash(hash, output).await,
                    ["delete", name] | ["rm", name] => ctx.delete(name).await,
                    ["rename", from, to] => ctx.rename(from, to).await,
                    ["copy", from, to] | ["cp", from, to] => ctx.copy(from, to).await,
                    ["find", query, rest @ ..] => {
                        let prefer_name = rest.contains(&"--name");
                        match ctx.find(query, prefer_name).await {
                            Ok(matches) if matches.is_empty() => {
                                println!("no matches found for: {}", query);
                                Ok(())
                            }
                            Ok(matches) if matches.len() == 1 => {
                                let m = &matches[0];
                                let kind_str = match m.kind {
                                    MatchKind::Exact => "exact",
                                    MatchKind::Prefix => "prefix",
                                    MatchKind::Contains => "contains",
                                };
                                let match_type = if m.is_hash_match { "hash" } else { "name" };
                                println!("{}\t{}", m.hash, m.name);
                                println!("({} {} match)", kind_str, match_type);
                                Ok(())
                            }
                            Ok(matches) => {
                                println!("found {} matches:", matches.len());
                                for (i, m) in matches.iter().enumerate() {
                                    let kind_str = match m.kind {
                                        MatchKind::Exact => "exact",
                                        MatchKind::Prefix => "prefix",
                                        MatchKind::Contains => "contains",
                                    };
                                    let match_type = if m.is_hash_match { "hash" } else { "name" };
                                    println!(
                                        "  [{}] {}\t{} ({} {})",
                                        i + 1,
                                        m.hash,
                                        m.name,
                                        kind_str,
                                        match_type
                                    );
                                }
                                println!("select [1-{}] or press enter to cancel:", matches.len());
                                match rl.readline("? ") {
                                    Ok(sel) => {
                                        let sel = sel.trim();
                                        if sel.is_empty() {
                                            println!("cancelled");
                                        } else if let Ok(n) = sel.parse::<usize>() {
                                            if n >= 1 && n <= matches.len() {
                                                let m = &matches[n - 1];
                                                println!("selected: {}\t{}", m.hash, m.name);
                                            } else {
                                                println!("invalid selection");
                                            }
                                        } else {
                                            println!("invalid selection");
                                        }
                                    }
                                    _ => println!("cancelled"),
                                }
                                Ok(())
                            }
                            Err(e) => Err(e),
                        }
                    }
                    ["search", query, rest @ ..] => {
                        let prefer_name = rest.contains(&"--name");
                        match ctx.find(query, prefer_name).await {
                            Ok(matches) if matches.is_empty() => {
                                println!("no matches found for: {}", query);
                                Ok(())
                            }
                            Ok(matches) => {
                                for m in &matches {
                                    let kind_str = match m.kind {
                                        MatchKind::Exact => "exact",
                                        MatchKind::Prefix => "prefix",
                                        MatchKind::Contains => "contains",
                                    };
                                    let match_type = if m.is_hash_match { "hash" } else { "name" };
                                    println!(
                                        "{}\t{}\t({} {})",
                                        m.hash, m.name, kind_str, match_type
                                    );
                                }
                                Ok(())
                            }
                            Err(e) => Err(e),
                        }
                    }
                    _ => {
                        println!("unknown command: {}", line);
                        println!("type 'help' for available commands");
                        Ok(())
                    }
                };

                if let Err(e) = result {
                    println!("error: {}", e);
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("^C");
                break;
            }
            Err(ReadlineError::Eof) => {
                break;
            }
            Err(e) => {
                println!("readline error: {}", e);
                break;
            }
        }
    }

    ctx.shutdown().await?;
    Ok(())
}

fn print_usage() {
    eprintln!(
        r#"usage:
  id serve [--ephemeral] [--no-relay]      Start server (accepts put/get from peers)
  id repl                                  Interactive REPL (auto-detects local/remote)
  id put <FILE>                            Store file (works with serve running)
  id put - <NAME>                          Store from stdin with given name
  id put <NODE_ID> <FILE> [--no-relay]     Upload file to remote node
  id put <NODE_ID> - <NAME> [--no-relay]   Upload from stdin to remote node
  id get <NAME> [OUTPUT]                   Retrieve file (OUTPUT defaults to NAME, use - for stdout)
  id get <NODE_ID> <NAME> [OUTPUT] [--no-relay]  Download from remote (use - for stdout)
  id gethash <HASH> <OUTPUT>               Retrieve by hash (use - for stdout)
  id list                                  List all stored files
  id id                                    Print node ID

Options:
  --ephemeral    Use in-memory storage for serve (default: persistent .iroh-store)
  --no-relay     Disable relay servers (direct connections only)

Notes:
  - put/get/list/repl work while serve is running (connects via network)
  - Use - for stdin (put) or stdout (get)"#
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    let args: Vec<String> = std::env::args().skip(1).collect();

    // Check for help before filtering flags
    if args
        .iter()
        .any(|a| a == "--help" || a == "-h" || a == "help")
    {
        print_usage();
        return Ok(());
    }

    let ephemeral = args.iter().any(|a| a == "--ephemeral");
    let no_relay = args.iter().any(|a| a == "--no-relay");
    let args: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(|s| s.as_str())
        .collect();

    let key = load_or_create_keypair(KEY_FILE).await?;
    let node_id: EndpointId = key.public().into();

    match args.as_slice() {
        [] => run_repl().await,

        ["id"] => {
            println!("{}", node_id);
            Ok(())
        }

        ["repl"] | ["shell"] => run_repl().await,

        ["serve"] => {
            info!("serve: {}", node_id);
            let store = open_store(ephemeral).await?;
            let store_handle = store.as_store();

            let mut builder = Endpoint::builder()
                .secret_key(key.clone())
                .address_lookup(PkarrPublisher::n0_dns())
                .address_lookup(DnsAddressLookup::n0_dns());
            if no_relay {
                builder = builder.relay_mode(RelayMode::Disabled);
            }
            let endpoint = builder.bind().await?;

            let meta = MetaProtocol::new(&store_handle);
            let blobs = BlobsProtocol::new(&store_handle, None);

            let router = Router::builder(endpoint)
                .accept(META_ALPN, meta)
                .accept(BLOBS_ALPN, blobs)
                .spawn();

            let serve_node_id = router.endpoint().id();
            let bound_addrs = router.endpoint().bound_sockets();
            // Convert wildcard addresses (0.0.0.0, [::]) to localhost for client connections
            let local_addrs: Vec<SocketAddr> = bound_addrs
                .iter()
                .map(|addr| match addr {
                    SocketAddr::V4(v4) if v4.ip().is_unspecified() => {
                        SocketAddr::new(Ipv4Addr::LOCALHOST.into(), v4.port())
                    }
                    SocketAddr::V6(v6) if v6.ip().is_unspecified() => {
                        SocketAddr::new(Ipv6Addr::LOCALHOST.into(), v6.port())
                    }
                    other => *other,
                })
                .collect();
            create_serve_lock(&serve_node_id, &local_addrs).await?;

            println!("node: {}", serve_node_id);
            if ephemeral {
                println!("mode: ephemeral (in-memory)");
            } else {
                println!("mode: persistent ({})", STORE_PATH);
            }
            if no_relay {
                println!("relay: disabled");
            }

            tokio::signal::ctrl_c().await?;
            remove_serve_lock().await?;
            router.shutdown().await?;
            store.shutdown().await?;
            Ok(())
        }

        ["put", path] => {
            // Local put - check if serve is running
            if *path == "-" {
                bail!("stdin requires a name: put - <NAME>");
            }
            let path = PathBuf::from(path);
            let filename = path
                .file_name()
                .context("invalid filename")?
                .to_string_lossy()
                .to_string();
            let data = afs::read(&path).await?;

            if let Some(serve_info) = get_serve_info().await {
                // Serve is running - use remote protocol
                let store = open_store(true).await?;
                let store_handle = store.as_store();

                let added = store_handle
                    .add_bytes_with_opts(AddBytesOptions {
                        data: data.into(),
                        format: BlobFormat::Raw,
                    })
                    .await?;
                let hash = added.hash;

                let (endpoint, endpoint_addr) = create_local_client_endpoint(&serve_info).await?;

                let meta_conn = endpoint.connect(endpoint_addr.clone(), META_ALPN).await?;
                let (mut send, mut recv) = meta_conn.open_bi().await?;
                let req = postcard::to_allocvec(&MetaRequest::Put {
                    filename: filename.clone(),
                    hash,
                })?;
                send.write_all(&req).await?;
                send.finish()?;
                let resp_buf = recv.read_to_end(64 * 1024).await?;
                let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
                meta_conn.close(0u32.into(), b"done");

                match resp {
                    MetaResponse::Put { success: true } => {
                        let blobs_conn =
                            endpoint.connect(endpoint_addr.clone(), BLOBS_ALPN).await?;
                        let push_request = PushRequest::new(
                            hash,
                            ChunkRangesSeq::from_ranges([ChunkRanges::all()]),
                        );
                        store_handle
                            .remote()
                            .execute_push(blobs_conn.clone(), push_request)
                            .await?;
                        blobs_conn.close(0u32.into(), b"done");
                        eprintln!("stored: {} -> {}", filename, hash);
                        store.shutdown().await?;
                        Ok(())
                    }
                    MetaResponse::Put { success: false } => bail!("server rejected"),
                    _ => bail!("unexpected response"),
                }
            } else {
                // No serve running - direct store access
                let store = open_store(false).await?;
                let store_handle = store.as_store();

                let added = store_handle
                    .add_bytes_with_opts(AddBytesOptions {
                        data: data.into(),
                        format: BlobFormat::Raw,
                    })
                    .await?;

                store_handle.tags().set(&filename, added.hash).await?;
                eprintln!("stored: {} -> {}", filename, added.hash);
                store.shutdown().await?;
                Ok(())
            }
        }

        ["put", "-", name] => {
            // Local put from stdin with name
            let data = read_input("-").await?;

            if let Some(serve_info) = get_serve_info().await {
                // Serve is running - use remote protocol
                let store = open_store(true).await?;
                let store_handle = store.as_store();

                let added = store_handle
                    .add_bytes_with_opts(AddBytesOptions {
                        data: data.into(),
                        format: BlobFormat::Raw,
                    })
                    .await?;
                let hash = added.hash;

                let (endpoint, endpoint_addr) = create_local_client_endpoint(&serve_info).await?;

                let meta_conn = endpoint.connect(endpoint_addr.clone(), META_ALPN).await?;
                let (mut send, mut recv) = meta_conn.open_bi().await?;
                let req = postcard::to_allocvec(&MetaRequest::Put {
                    filename: name.to_string(),
                    hash,
                })?;
                send.write_all(&req).await?;
                send.finish()?;
                let resp_buf = recv.read_to_end(64 * 1024).await?;
                let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
                meta_conn.close(0u32.into(), b"done");

                match resp {
                    MetaResponse::Put { success: true } => {
                        let blobs_conn =
                            endpoint.connect(endpoint_addr.clone(), BLOBS_ALPN).await?;
                        let push_request = PushRequest::new(
                            hash,
                            ChunkRangesSeq::from_ranges([ChunkRanges::all()]),
                        );
                        store_handle
                            .remote()
                            .execute_push(blobs_conn.clone(), push_request)
                            .await?;
                        blobs_conn.close(0u32.into(), b"done");
                        eprintln!("stored: {} -> {}", name, hash);
                        store.shutdown().await?;
                        Ok(())
                    }
                    MetaResponse::Put { success: false } => bail!("server rejected"),
                    _ => bail!("unexpected response"),
                }
            } else {
                // No serve running - direct store access
                let store = open_store(false).await?;
                let store_handle = store.as_store();

                let added = store_handle
                    .add_bytes_with_opts(AddBytesOptions {
                        data: data.into(),
                        format: BlobFormat::Raw,
                    })
                    .await?;

                store_handle.tags().set(*name, added.hash).await?;
                eprintln!("stored: {} -> {}", name, added.hash);
                store.shutdown().await?;
                Ok(())
            }
        }

        ["put", server_node_id, path] => {
            let server_node_id: EndpointId = server_node_id.parse()?;
            let path = PathBuf::from(path);
            let filename = path
                .file_name()
                .context("invalid filename")?
                .to_string_lossy()
                .to_string();

            // Use ephemeral store for remote push (no local copy)
            let store = open_store(true).await?;
            let store_handle = store.as_store();

            // Read and hash the file
            let data = afs::read(&path).await?;
            let added = store_handle
                .add_bytes_with_opts(AddBytesOptions {
                    data: data.into(),
                    format: BlobFormat::Raw,
                })
                .await?;
            let hash = added.hash;

            // Remote put - connect to server (use separate client key to allow self-connect)
            let client_key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
            let mut builder = Endpoint::builder()
                .secret_key(client_key)
                .address_lookup(PkarrPublisher::n0_dns())
                .address_lookup(DnsAddressLookup::n0_dns());
            if no_relay {
                builder = builder.relay_mode(RelayMode::Disabled);
            }
            let endpoint = builder.bind().await?;

            let meta_conn = endpoint.connect(server_node_id, META_ALPN).await?;
            let (mut send, mut recv) = meta_conn.open_bi().await?;
            let req = postcard::to_allocvec(&MetaRequest::Put {
                filename: filename.clone(),
                hash,
            })?;
            send.write_all(&req).await?;
            send.finish()?;
            let resp_buf = recv.read_to_end(64 * 1024).await?;
            let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
            meta_conn.close(0u32.into(), b"done");

            match resp {
                MetaResponse::Put { success: true } => {
                    let blobs_conn = endpoint.connect(server_node_id, BLOBS_ALPN).await?;
                    let push_request =
                        PushRequest::new(hash, ChunkRangesSeq::from_ranges([ChunkRanges::all()]));
                    store_handle
                        .remote()
                        .execute_push(blobs_conn.clone(), push_request)
                        .await?;
                    blobs_conn.close(0u32.into(), b"done");
                    println!("uploaded: {} -> {}", filename, hash);
                    store.shutdown().await?;
                    Ok(())
                }
                MetaResponse::Put { success: false } => bail!("server rejected"),
                _ => bail!("unexpected response"),
            }
        }

        ["put", server_node_id, "-", name] => {
            // Remote put from stdin: put <NODE_ID> - <NAME>
            let server_node_id: EndpointId = server_node_id.parse()?;

            let store = open_store(true).await?;
            let store_handle = store.as_store();

            let data = read_input("-").await?;
            let added = store_handle
                .add_bytes_with_opts(AddBytesOptions {
                    data: data.into(),
                    format: BlobFormat::Raw,
                })
                .await?;
            let hash = added.hash;

            let client_key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
            let mut builder = Endpoint::builder()
                .secret_key(client_key)
                .address_lookup(PkarrPublisher::n0_dns())
                .address_lookup(DnsAddressLookup::n0_dns());
            if no_relay {
                builder = builder.relay_mode(RelayMode::Disabled);
            }
            let endpoint = builder.bind().await?;

            let meta_conn = endpoint.connect(server_node_id, META_ALPN).await?;
            let (mut send, mut recv) = meta_conn.open_bi().await?;
            let req = postcard::to_allocvec(&MetaRequest::Put {
                filename: name.to_string(),
                hash,
            })?;
            send.write_all(&req).await?;
            send.finish()?;
            let resp_buf = recv.read_to_end(64 * 1024).await?;
            let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
            meta_conn.close(0u32.into(), b"done");

            match resp {
                MetaResponse::Put { success: true } => {
                    let blobs_conn = endpoint.connect(server_node_id, BLOBS_ALPN).await?;
                    let push_request =
                        PushRequest::new(hash, ChunkRangesSeq::from_ranges([ChunkRanges::all()]));
                    store_handle
                        .remote()
                        .execute_push(blobs_conn.clone(), push_request)
                        .await?;
                    blobs_conn.close(0u32.into(), b"done");
                    eprintln!("uploaded: {} -> {}", name, hash);
                    store.shutdown().await?;
                    Ok(())
                }
                MetaResponse::Put { success: false } => bail!("server rejected"),
                _ => bail!("unexpected response"),
            }
        }

        ["get", name] => {
            // Local get by name - check if serve is running
            if let Some(serve_info) = get_serve_info().await {
                // Serve is running - use remote protocol
                let store = open_store(true).await?;
                let store_handle = store.as_store();

                let (endpoint, endpoint_addr) = create_local_client_endpoint(&serve_info).await?;

                let meta_conn = endpoint.connect(endpoint_addr.clone(), META_ALPN).await?;
                let (mut send, mut recv) = meta_conn.open_bi().await?;
                let req = postcard::to_allocvec(&MetaRequest::Get {
                    filename: name.to_string(),
                })?;
                send.write_all(&req).await?;
                send.finish()?;
                let resp_buf = recv.read_to_end(64 * 1024).await?;
                let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
                meta_conn.close(0u32.into(), b"done");

                let hash = match resp {
                    MetaResponse::Get { hash: Some(h) } => h,
                    MetaResponse::Get { hash: None } => bail!("file not found"),
                    _ => bail!("unexpected response"),
                };

                let blobs_conn = endpoint.connect(endpoint_addr.clone(), BLOBS_ALPN).await?;
                store_handle
                    .remote()
                    .fetch(blobs_conn.clone(), hash)
                    .await?;
                blobs_conn.close(0u32.into(), b"done");

                export_blob(&store_handle, hash, name).await?;
                store.shutdown().await?;
                Ok(())
            } else {
                // No serve running - direct store access
                let store = open_store(false).await?;
                let store_handle = store.as_store();

                let tag = store_handle
                    .tags()
                    .get(name)
                    .await?
                    .context("file not found")?;

                export_blob(&store_handle, tag.hash, name).await?;
                store.shutdown().await?;
                Ok(())
            }
        }

        ["get", first, second] => {
            // Could be: get <NAME> <OUTPUT> or get <NODE_ID> <NAME>
            // Detect node ID by checking if it's 64 hex chars
            let is_node_id = first.len() == 64 && first.chars().all(|c| c.is_ascii_hexdigit());

            if is_node_id {
                // Remote get: get <NODE_ID> <NAME>
                let server_node_id: EndpointId = first.parse()?;
                let name = *second;

                let store = open_store(true).await?;
                let store_handle = store.as_store();

                let client_key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
                let mut builder = Endpoint::builder()
                    .secret_key(client_key)
                    .address_lookup(PkarrPublisher::n0_dns())
                    .address_lookup(DnsAddressLookup::n0_dns());
                if no_relay {
                    builder = builder.relay_mode(RelayMode::Disabled);
                }
                let endpoint = builder.bind().await?;

                let meta_conn = endpoint.connect(server_node_id, META_ALPN).await?;
                let (mut send, mut recv) = meta_conn.open_bi().await?;
                let req = postcard::to_allocvec(&MetaRequest::Get {
                    filename: name.to_string(),
                })?;
                send.write_all(&req).await?;
                send.finish()?;
                let resp_buf = recv.read_to_end(64 * 1024).await?;
                let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
                meta_conn.close(0u32.into(), b"done");

                let hash = match resp {
                    MetaResponse::Get { hash: Some(h) } => h,
                    MetaResponse::Get { hash: None } => bail!("file not found on remote"),
                    _ => bail!("unexpected response"),
                };

                let blobs_conn = endpoint.connect(server_node_id, BLOBS_ALPN).await?;
                store_handle
                    .remote()
                    .fetch(blobs_conn.clone(), hash)
                    .await?;
                blobs_conn.close(0u32.into(), b"done");

                export_blob(&store_handle, hash, name).await?;
                store.shutdown().await?;
                Ok(())
            } else {
                // Local get with output: get <NAME> <OUTPUT>
                let name = *first;
                let output = *second;

                if let Some(serve_info) = get_serve_info().await {
                    // Serve is running - use remote protocol
                    let store = open_store(true).await?;
                    let store_handle = store.as_store();

                    let (endpoint, endpoint_addr) =
                        create_local_client_endpoint(&serve_info).await?;

                    let meta_conn = endpoint.connect(endpoint_addr.clone(), META_ALPN).await?;
                    let (mut send, mut recv) = meta_conn.open_bi().await?;
                    let req = postcard::to_allocvec(&MetaRequest::Get {
                        filename: name.to_string(),
                    })?;
                    send.write_all(&req).await?;
                    send.finish()?;
                    let resp_buf = recv.read_to_end(64 * 1024).await?;
                    let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
                    meta_conn.close(0u32.into(), b"done");

                    let hash = match resp {
                        MetaResponse::Get { hash: Some(h) } => h,
                        MetaResponse::Get { hash: None } => bail!("file not found"),
                        _ => bail!("unexpected response"),
                    };

                    let blobs_conn = endpoint.connect(endpoint_addr.clone(), BLOBS_ALPN).await?;
                    store_handle
                        .remote()
                        .fetch(blobs_conn.clone(), hash)
                        .await?;
                    blobs_conn.close(0u32.into(), b"done");

                    export_blob(&store_handle, hash, output).await?;
                    store.shutdown().await?;
                    Ok(())
                } else {
                    // No serve running - direct store access
                    let store = open_store(false).await?;
                    let store_handle = store.as_store();

                    let tag = store_handle
                        .tags()
                        .get(name)
                        .await?
                        .context("file not found")?;

                    export_blob(&store_handle, tag.hash, output).await?;
                    store.shutdown().await?;
                    Ok(())
                }
            }
        }

        ["get", node_id, name, output] => {
            // Remote get with output: get <NODE_ID> <NAME> <OUTPUT>
            let server_node_id: EndpointId = node_id.parse()?;

            let store = open_store(true).await?;
            let store_handle = store.as_store();

            let client_key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
            let mut builder = Endpoint::builder()
                .secret_key(client_key)
                .address_lookup(PkarrPublisher::n0_dns())
                .address_lookup(DnsAddressLookup::n0_dns());
            if no_relay {
                builder = builder.relay_mode(RelayMode::Disabled);
            }
            let endpoint = builder.bind().await?;

            let meta_conn = endpoint.connect(server_node_id, META_ALPN).await?;
            let (mut send, mut recv) = meta_conn.open_bi().await?;
            let req = postcard::to_allocvec(&MetaRequest::Get {
                filename: name.to_string(),
            })?;
            send.write_all(&req).await?;
            send.finish()?;
            let resp_buf = recv.read_to_end(64 * 1024).await?;
            let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
            meta_conn.close(0u32.into(), b"done");

            let hash = match resp {
                MetaResponse::Get { hash: Some(h) } => h,
                MetaResponse::Get { hash: None } => bail!("file not found on remote"),
                _ => bail!("unexpected response"),
            };

            let blobs_conn = endpoint.connect(server_node_id, BLOBS_ALPN).await?;
            store_handle
                .remote()
                .fetch(blobs_conn.clone(), hash)
                .await?;
            blobs_conn.close(0u32.into(), b"done");

            export_blob(&store_handle, hash, output).await?;
            store.shutdown().await?;
            Ok(())
        }

        ["list"] => {
            if let Some(serve_info) = get_serve_info().await {
                // Serve is running - use remote protocol
                let (endpoint, endpoint_addr) = create_local_client_endpoint(&serve_info).await?;

                let meta_conn = endpoint.connect(endpoint_addr.clone(), META_ALPN).await?;
                let (mut send, mut recv) = meta_conn.open_bi().await?;
                let req = postcard::to_allocvec(&MetaRequest::List)?;
                send.write_all(&req).await?;
                send.finish()?;
                let resp_buf = recv.read_to_end(1024 * 1024).await?;
                let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
                meta_conn.close(0u32.into(), b"done");

                match resp {
                    MetaResponse::List { items } => {
                        if items.is_empty() {
                            println!("(no files stored)");
                        } else {
                            for (hash, name) in items {
                                println!("{}\t{}", hash, name);
                            }
                        }
                    }
                    _ => bail!("unexpected response"),
                }
                Ok(())
            } else {
                let store = open_store(false).await?;
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
                store.shutdown().await?;
                Ok(())
            }
        }

        ["gethash", hash_str, output] => {
            // Get by hash: gethash <HASH> <OUTPUT>
            let hash: Hash = hash_str.parse().context("invalid hash")?;

            if let Some(serve_info) = get_serve_info().await {
                // Serve is running - fetch from serve
                let store = open_store(true).await?;
                let store_handle = store.as_store();

                let (endpoint, endpoint_addr) = create_local_client_endpoint(&serve_info).await?;

                let blobs_conn = endpoint.connect(endpoint_addr.clone(), BLOBS_ALPN).await?;
                store_handle
                    .remote()
                    .fetch(blobs_conn.clone(), hash)
                    .await?;
                blobs_conn.close(0u32.into(), b"done");

                export_blob(&store_handle, hash, output).await?;
                store.shutdown().await?;
                Ok(())
            } else {
                let store = open_store(false).await?;
                let store_handle = store.as_store();

                export_blob(&store_handle, hash, output).await?;
                store.shutdown().await?;
                Ok(())
            }
        }

        _ => {
            print_usage();
            std::process::exit(1);
        }
    }
}
