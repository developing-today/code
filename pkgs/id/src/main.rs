use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use futures_lite::StreamExt;
use iroh::{
    address_lookup::{DnsAddressLookup, PkarrPublisher},
    endpoint::{Connection, Endpoint, RelayMode},
    protocol::Router,
    EndpointAddr,
};
use iroh_base::EndpointId;
use iroh_blobs::{
    ALPN as BLOBS_ALPN, BlobFormat, BlobsProtocol, Hash,
    api::{Store, blobs::AddBytesOptions},
    protocol::{ChunkRanges, ChunkRangesSeq, PushRequest},
};
use rustyline::{DefaultEditor, error::ReadlineError};
use std::{
    io::{IsTerminal, Read},
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    path::PathBuf,
};
use tokio::fs as afs;
use tracing::info;

// Import from library
use id::{
    FindMatch, MatchKind, MetaProtocol, MetaRequest, MetaResponse, TaggedMatch,
    StoreType, load_or_create_keypair, open_store,
    create_local_client_endpoint, create_serve_lock, get_serve_info, remove_serve_lock,
    KEY_FILE, CLIENT_KEY_FILE, STORE_PATH, META_ALPN,
    export_blob, is_node_id, parse_stdin_items, read_input,
};
use id::repl::{ReplInput, continue_heredoc, preprocess_repl_line};

/// iroh-based peer-to-peer file sharing
#[derive(Parser)]
#[command(name = "id", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Start server (accepts put/get from peers)
    Serve {
        /// Use in-memory storage (default: persistent .iroh-store)
        #[arg(long)]
        ephemeral: bool,
        /// Disable relay servers (direct connections only)
        #[arg(long)]
        no_relay: bool,
    },
    /// Interactive REPL - use 'id repl <NODE_ID>' for remote session, or @NODE_ID prefix in commands
    #[command(alias = "shell")]
    Repl {
        /// Remote node ID for session-level remote targeting (all commands target this node)
        #[arg(required = false)]
        node: Option<String>,
    },
    /// Store one or more files (supports path:name for renaming)
    /// Use "put <NODE_ID> file1 file2 ..." to put to a remote node
    #[command(aliases = ["in", "add", "store", "import"])]
    Put {
        /// File paths to store (use path:name to rename, e.g. file.txt:stored.txt)
        /// If first arg is a 64-char hex NODE_ID, remaining args are sent to that remote node
        #[arg(required = false)]
        files: Vec<String>,
        /// Read content from stdin instead of file paths (requires one name argument)
        #[arg(long, visible_alias = "data", conflicts_with = "stdin")]
        content: bool,
        /// Read additional file paths from stdin (split on newline/tab/comma)
        #[arg(long, conflicts_with = "content")]
        stdin: bool,
        /// Store by hash only, don't create named tags
        #[arg(long)]
        hash_only: bool,
        /// Disable relay servers (for remote operations)
        #[arg(long)]
        no_relay: bool,
    },
    /// Store content by hash only (no name)
    #[command(name = "put-hash")]
    PutHash {
        /// File path or "-" for stdin
        source: String,
    },
    /// Retrieve one or more files by name or hash (supports source:output for renaming)
    /// Use "get <NODE_ID> name1 name2 ..." to get from a remote node
    Get {
        /// Names or hashes to retrieve (use source:output to rename, e.g. file.txt:out.txt or hash:- for stdout)
        /// If first arg is a 64-char hex NODE_ID, remaining args are fetched from that remote node
        #[arg(required = false)]
        sources: Vec<String>,
        /// Read additional sources from stdin (split on newline/tab/comma)
        #[arg(long)]
        stdin: bool,
        /// Treat all sources as hashes (fail if not found, don't check names)
        #[arg(long, conflicts_with = "name_only")]
        hash: bool,
        /// Treat all sources as names only (don't try as hash even if 64 hex chars)
        #[arg(long, conflicts_with = "hash")]
        name_only: bool,
        /// Output all files to stdout (concatenated) - overrides per-item outputs
        #[arg(long)]
        stdout: bool,
        /// Disable relay servers (for remote operations)
        #[arg(long)]
        no_relay: bool,
    },
    /// Retrieve a file by hash (alias for get --hash)
    #[command(name = "get-hash")]
    GetHash {
        /// The blob hash
        hash: String,
        /// Output path (use "-" for stdout)
        output: String,
    },
    /// Output files to stdout (like get but defaults to stdout)
    #[command(aliases = ["output", "out"])]
    Cat {
        /// Names or hashes to retrieve
        /// If first arg is a 64-char hex NODE_ID, remaining args are fetched from that remote node
        #[arg(required = false)]
        sources: Vec<String>,
        /// Read additional sources from stdin (split on newline/tab/comma)
        #[arg(long)]
        stdin: bool,
        /// Treat all sources as hashes
        #[arg(long, conflicts_with = "name_only")]
        hash: bool,
        /// Treat all sources as names only
        #[arg(long, conflicts_with = "hash")]
        name_only: bool,
        /// Disable relay servers (for remote operations)
        #[arg(long)]
        no_relay: bool,
    },
    /// Find files by name/hash query and output to file (use --stdout for stdout)
    Find {
        /// Search queries (matches name or hash: exact > prefix > contains)
        #[arg(required = true)]
        queries: Vec<String>,
        /// Prefer name matches over hash matches
        #[arg(long)]
        name: bool,
        /// Output to stdout instead of file
        #[arg(long)]
        stdout: bool,
        /// Output all matches (to stdout, or to directory with --dir)
        #[arg(long, visible_aliases = ["out", "export", "save", "full"])]
        all: bool,
        /// Output directory for --all (each file saved by name)
        #[arg(long)]
        dir: Option<String>,
        /// Output format: tag (default), group, or union
        #[arg(long, default_value = "tag")]
        format: String,
        /// Remote node ID to search
        #[arg(long)]
        node: Option<String>,
        /// Disable relay servers
        #[arg(long)]
        no_relay: bool,
    },
    /// Search files by name/hash query and list all matches
    Search {
        /// Search queries (matches name or hash: exact > prefix > contains)
        #[arg(required = true)]
        queries: Vec<String>,
        /// Prefer name matches over hash matches
        #[arg(long)]
        name: bool,
        /// Output all matches (to stdout, or to directory with --dir)
        #[arg(long, visible_aliases = ["out", "export", "save", "full"])]
        all: bool,
        /// Output directory for --all (each file saved by name)
        #[arg(long)]
        dir: Option<String>,
        /// Output format: tag (default), group, or union
        #[arg(long, default_value = "tag")]
        format: String,
        /// Remote node ID to search
        #[arg(long)]
        node: Option<String>,
        /// Disable relay servers
        #[arg(long)]
        no_relay: bool,
    },
    /// List all stored files (local or remote)
    List {
        /// Remote node ID to list (optional - lists local if not provided)
        #[arg(required = false)]
        node: Option<String>,
        /// Disable relay servers (for remote operations)
        #[arg(long)]
        no_relay: bool,
    },
    /// Print node ID
    Id,
}



/// REPL context - holds either remote connections or local store access
struct ReplContext {
    inner: ReplContextInner,
    /// Session-level remote target (from `id repl <NODE_ID>`) - reserved for future use
    #[allow(dead_code)]
    session_target: Option<EndpointId>,
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
    /// Connected to a remote peer node
    RemoteNode {
        endpoint: Endpoint,
        node_id: EndpointId,
        meta_conn: Option<Connection>,
        blobs_conn: Option<Connection>,
        store: StoreType,
    },
}

impl ReplContext {
    async fn new(target_node: Option<String>) -> Result<Self> {
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
            return Ok(ReplContext {
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
            Ok(ReplContext {
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
            Ok(ReplContext {
                inner: ReplContextInner::Local { store },
                session_target: None,
            })
        }
    }

    fn mode_str(&self) -> String {
        match &self.inner {
            ReplContextInner::Remote { .. } => "local-serve".to_string(),
            ReplContextInner::Local { .. } => "local".to_string(),
            ReplContextInner::RemoteNode { node_id, .. } => {
                format!("remote:{}", &node_id.to_string()[..8])
            }
        }
    }

    /// Check if connected to a server (local serve or remote node)
    fn is_connected(&self) -> bool {
        matches!(
            &self.inner,
            ReplContextInner::Remote { .. } | ReplContextInner::RemoteNode { .. }
        )
    }

    /// Get store handle (works for all modes)
    fn store_handle(&self) -> Store {
        match &self.inner {
            ReplContextInner::Remote { store, .. } => store.as_store(),
            ReplContextInner::Local { store } => store.as_store(),
            ReplContextInner::RemoteNode { store, .. } => store.as_store(),
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
            ReplContextInner::RemoteNode {
                endpoint,
                node_id,
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
                let conn = endpoint.connect(*node_id, META_ALPN).await?;
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
            ReplContextInner::RemoteNode {
                endpoint,
                node_id,
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
                let conn = endpoint.connect(*node_id, BLOBS_ALPN).await?;
                *blobs_conn = Some(conn);
                Ok(blobs_conn.as_ref().unwrap())
            }
            ReplContextInner::Local { .. } => bail!("blobs_conn called in local mode"),
        }
    }

    async fn list(&mut self) -> Result<()> {
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
        // Check for content marker: __STDIN_CONTENT__:actual_content
        let (data, filename) = if let Some(content) = path.strip_prefix("__STDIN_CONTENT__:") {
            let name = name.ok_or_else(|| anyhow!("content requires a name"))?;
            (content.as_bytes().to_vec(), name.to_string())
        } else if path == "-" {
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

        if self.is_connected() {
            // Add to local ephemeral store first
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
                    let store_handle = self.store_handle();
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

    async fn get(&mut self, name: &str, output: Option<&str>) -> Result<()> {
        let output = output.unwrap_or(name);

        if self.is_connected() {
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
                    let store_handle = self.store_handle();
                    store_handle.remote().fetch(blobs_conn, hash).await?;
                    export_blob(&store_handle, hash, output).await?;
                }
                MetaResponse::Get { hash: None } => bail!("not found: {}", name),
                _ => bail!("unexpected response"),
            }
        } else {
            let store_handle = self.store_handle();
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

    async fn delete(&mut self, name: &str) -> Result<()> {
        if self.is_connected() {
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
        } else {
            let store_handle = self.store_handle();
            store_handle.tags().delete(name).await?;
            println!("deleted: {}", name);
        }
        Ok(())
    }

    async fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        if self.is_connected() {
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
        } else {
            let store_handle = self.store_handle();
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
        if self.is_connected() {
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
        } else {
            let store_handle = self.store_handle();
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
        let matches = if self.is_connected() {
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

    /// Get endpoint for creating connections (returns None for pure local mode)
    fn endpoint(&self) -> Option<&Endpoint> {
        match &self.inner {
            ReplContextInner::Remote { endpoint, .. } => Some(endpoint),
            ReplContextInner::RemoteNode { endpoint, .. } => Some(endpoint),
            ReplContextInner::Local { .. } => None,
        }
    }

    /// List files on a specific remote node (using @NODE_ID syntax)
    async fn list_on_node(&mut self, node_str: &str) -> Result<()> {
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
                        println!("{}\t{}", hash, name);
                    }
                }
            }
            _ => bail!("unexpected response"),
        }
        conn.close(0u32.into(), b"done");
        Ok(())
    }

    /// Put a file to a specific remote node (using @NODE_ID syntax)
    async fn put_on_node(&mut self, node_str: &str, path: &str, name: Option<&str>) -> Result<()> {
        let node_id: EndpointId = node_str.parse()?;
        let endpoint = self.endpoint().ok_or_else(|| {
            anyhow!("@NODE_ID requires a connected mode (use 'id repl' with a running serve)")
        })?;

        // Read data
        let (data, filename) = if let Some(content) = path.strip_prefix("__STDIN_CONTENT__:") {
            let name = name.ok_or_else(|| anyhow!("content requires a name"))?;
            (content.as_bytes().to_vec(), name.to_string())
        } else if path == "-" {
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

        // Add to local store first
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

        // Connect and request server to accept
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
                // Push blob to remote
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

    /// Get a file from a specific remote node (using @NODE_ID syntax)
    async fn get_on_node(
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

        // Get hash from remote
        let meta_conn = endpoint.connect(node_id, META_ALPN).await?;
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
                // Fetch blob from remote
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

    /// Delete a file on a specific remote node (using @NODE_ID syntax)
    async fn delete_on_node(&mut self, node_str: &str, name: &str) -> Result<()> {
        let node_id: EndpointId = node_str.parse()?;
        let endpoint = self.endpoint().ok_or_else(|| {
            anyhow!("@NODE_ID requires a connected mode (use 'id repl' with a running serve)")
        })?;

        let conn = endpoint.connect(node_id, META_ALPN).await?;
        let (mut send, mut recv) = conn.open_bi().await?;
        let req = postcard::to_allocvec(&MetaRequest::Delete {
            filename: name.to_string(),
        })?;
        send.write_all(&req).await?;
        send.finish()?;
        let resp_buf = recv.read_to_end(64 * 1024).await?;
        let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;

        match resp {
            MetaResponse::Delete { success: true } => {
                println!("deleted: {} (@{})", name, &node_str[..8])
            }
            MetaResponse::Delete { success: false } => {
                bail!("not found: {} (@{})", name, &node_str[..8])
            }
            _ => bail!("unexpected response"),
        }
        conn.close(0u32.into(), b"done");
        Ok(())
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

async fn run_repl(target_node: Option<String>) -> Result<()> {
    let mut ctx = ReplContext::new(target_node).await?;
    println!("id repl ({})", ctx.mode_str());
    println!("commands: list, put, get, cat, gethash, help, quit");
    println!("input: $(...), `...`, |>, <<<, <<EOF supported");

    let mut rl = DefaultEditor::new()?;
    let mut ctrl_c_count = 0u8;

    loop {
        match rl.readline("> ") {
            Ok(raw_line) => {
                ctrl_c_count = 0; // Reset on any input
                let raw_line = raw_line.trim();
                if raw_line.is_empty() {
                    continue;
                }
                let _ = rl.add_history_entry(raw_line);

                // Shell escape: !command (no preprocessing)
                if let Some(cmd) = raw_line.strip_prefix('!') {
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

                // Preprocess the line (handle $(), ``, |>, <<<, <<)
                let line = match preprocess_repl_line(raw_line) {
                    Ok(ReplInput::Empty) => continue,
                    Ok(ReplInput::Ready(line)) => line,
                    Ok(ReplInput::NeedMore {
                        delimiter,
                        mut lines,
                        original_line,
                    }) => {
                        // Heredoc mode - read until delimiter
                        match continue_heredoc(&mut rl, &delimiter, &mut lines) {
                            Ok(Some(content)) => {
                                // Replace - with content marker in original line
                                original_line
                                    .replace(" - ", &format!(" __STDIN_CONTENT__:{} ", content))
                                    .replace(" -$", &format!(" __STDIN_CONTENT__:{}", content))
                            }
                            Ok(None) => continue, // Cancelled
                            Err(e) => {
                                println!("error: {}", e);
                                continue;
                            }
                        }
                    }
                    Err(e) => {
                        println!("error: {}", e);
                        continue;
                    }
                };

                // Special handling for __STDIN_CONTENT__: marker
                // Format: put __STDIN_CONTENT__:content name
                let result = if line.contains("__STDIN_CONTENT__:") {
                    if let Some(start) = line.find("__STDIN_CONTENT__:") {
                        let before = line[..start].trim();
                        let after_marker = &line[start + 18..]; // 18 = len("__STDIN_CONTENT__:")

                        // Find the last whitespace-separated token (the name)
                        let after_trimmed = after_marker.trim();
                        if let Some(last_space) = after_trimmed.rfind(' ') {
                            let content = &after_trimmed[..last_space];
                            let name = &after_trimmed[last_space + 1..];

                            if before == "put" {
                                let content_marker = format!("__STDIN_CONTENT__:{}", content);
                                ctx.put(&content_marker, Some(name)).await
                            } else {
                                println!("unknown command with content: {}", before);
                                Ok(())
                            }
                        } else {
                            // No name provided - just content
                            println!("error: content requires a name (e.g., put $(cmd) name.txt)");
                            Ok(())
                        }
                    } else {
                        Ok(())
                    }
                } else {
                    let parts: Vec<&str> = line.split_whitespace().collect();

                    // Check for @NODE_ID prefix on commands
                    // Format: <cmd> @NODE_ID [args...]
                    let (target_node, cmd_parts) = if parts.len() >= 2 {
                        if let Some(node_str) = parts[1].strip_prefix('@') {
                            if is_node_id(node_str) {
                                // Reconstruct args: [cmd, arg1, arg2, ...]
                                let mut new_parts = vec![parts[0]];
                                new_parts.extend(&parts[2..]);
                                (Some(node_str), new_parts)
                            } else {
                                (None, parts.clone())
                            }
                        } else {
                            (None, parts.clone())
                        }
                    } else {
                        (None, parts.clone())
                    };

                    match (target_node, cmd_parts.as_slice()) {
                        // Commands with @NODE_ID target
                        (Some(node), ["list"]) | (Some(node), ["ls"]) => {
                            ctx.list_on_node(node).await
                        }
                        (Some(node), ["put", path]) => ctx.put_on_node(node, path, None).await,
                        (Some(node), ["put", path, name]) => {
                            ctx.put_on_node(node, path, Some(name)).await
                        }
                        (Some(node), ["get", name]) => ctx.get_on_node(node, name, None).await,
                        (Some(node), ["get", name, output]) => {
                            ctx.get_on_node(node, name, Some(output)).await
                        }
                        (Some(node), ["cat", name]) => ctx.get_on_node(node, name, Some("-")).await,
                        (Some(node), ["delete", name]) | (Some(node), ["rm", name]) => {
                            ctx.delete_on_node(node, name).await
                        }
                        (Some(_node), _) => {
                            println!("@NODE_ID not supported for this command");
                            Ok(())
                        }

                        // Regular commands (no @NODE_ID)
                        (None, ["quit"]) | (None, ["exit"]) | (None, ["q"]) => break,
                        (None, ["help"]) | (None, ["?"]) => {
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
                                "  find <QUERY> [--name] [--file|>FILE] - Find & output (stdout default)"
                            );
                            println!(
                                "  search <QUERY> [--name] [--file|>FILE] - List matches (optionally save first)"
                            );
                            println!("  !<cmd>                 - Run shell command");
                            println!("  help                   - Show this help");
                            println!("  quit                   - Exit repl");
                            println!();
                            println!("remote targeting:");
                            println!("  list @NODE_ID          - List files on remote node");
                            println!("  put @NODE_ID FILE      - Store file on remote node");
                            println!("  get @NODE_ID NAME      - Get file from remote node");
                            println!("  cat @NODE_ID NAME      - Print remote file to stdout");
                            println!("  delete @NODE_ID NAME   - Delete file on remote node");
                            println!();
                            println!("input methods:");
                            println!("  put $(cmd) name        - Store output of command");
                            println!("  put `cmd` name         - Store output of command (alt)");
                            println!("  cmd |> put - name      - Pipe command output to put");
                            println!("  put - name <<< 'text'  - Store literal text");
                            println!("  put - name <<EOF       - Start heredoc (end with EOF)");
                            Ok(())
                        }
                        (None, ["list"]) | (None, ["ls"]) => ctx.list().await,
                        (None, ["put", path]) | (None, ["in", path]) => ctx.put(path, None).await,
                        (None, ["put", path, name]) | (None, ["in", path, name]) => {
                            ctx.put(path, Some(name)).await
                        }
                        (None, ["get", name]) => ctx.get(name, None).await,
                        (None, ["get", name, output]) => ctx.get(name, Some(output)).await,
                        (None, ["cat", name])
                        | (None, ["output", name])
                        | (None, ["out", name]) => ctx.get(name, Some("-")).await,
                        (None, ["gethash", hash, output]) => ctx.gethash(hash, output).await,
                        (None, ["delete", name]) | (None, ["rm", name]) => ctx.delete(name).await,
                        (None, ["rename", from, to]) => ctx.rename(from, to).await,
                        (None, ["copy", from, to]) | (None, ["cp", from, to]) => {
                            ctx.copy(from, to).await
                        }
                        (None, ["find", rest @ ..]) => {
                            // Parse queries (args before flags) and flags
                            let mut queries: Vec<&str> = Vec::new();
                            let mut prefer_name = false;
                            let mut all = false;
                            let mut output_file: Option<&str> = None;
                            let mut dir: Option<&str> = None;
                            let mut to_file = false;
                            let mut format = "union"; // REPL default is union
                            
                            let mut i = 0;
                            while i < rest.len() {
                                let arg = rest[i];
                                if arg == "--name" {
                                    prefer_name = true;
                                } else if arg == "--all" || arg == "--out" || arg == "--export" || arg == "--save" || arg == "--full" {
                                    all = true;
                                } else if arg == "--file" {
                                    to_file = true;
                                } else if arg.starts_with('>') {
                                    output_file = Some(&arg[1..]);
                                    to_file = true;
                                } else if arg == "--dir" {
                                    if i + 1 < rest.len() {
                                        dir = Some(rest[i + 1]);
                                        i += 1;
                                    }
                                } else if arg == "--format" {
                                    if i + 1 < rest.len() {
                                        format = rest[i + 1];
                                        i += 1;
                                    }
                                } else if arg == "--tag" {
                                    format = "tag";
                                } else if arg == "--group" {
                                    format = "group";
                                } else if arg == "--union" {
                                    format = "union";
                                } else if !arg.starts_with('-') {
                                    queries.push(arg);
                                }
                                i += 1;
                            }
                            
                            if queries.is_empty() {
                                println!("usage: find <query>... [--name] [--all] [--dir <dir>] [--file] [>filename]");
                                return Ok(());
                            }

                            // Collect matches for all queries
                            let mut all_matches: Vec<(String, FindMatch)> = Vec::new();
                            for query in &queries {
                                match ctx.find(query, prefer_name).await {
                                    Ok(matches) => {
                                        for m in matches {
                                            all_matches.push((query.to_string(), m));
                                        }
                                    }
                                    Err(e) => {
                                        println!("error searching for '{}': {}", query, e);
                                    }
                                }
                            }

                            if all_matches.is_empty() {
                                println!("no matches found for: {}", queries.join(", "));
                                return Ok(());
                            }

                            // --all mode: output all matches
                            if all {
                                if let Some(dir_path) = dir {
                                    if let Err(e) = std::fs::create_dir_all(dir_path) {
                                        println!("error creating directory: {}", e);
                                        return Ok(());
                                    }
                                    let mut seen = std::collections::HashSet::new();
                                    for (query, m) in &all_matches {
                                        let key = format!("{}:{}", m.hash, m.name);
                                        if seen.insert(key) {
                                            let output_path = format!("{}/{}", dir_path, m.name);
                                            if let Err(e) = ctx.get(&m.name, Some(&output_path)).await {
                                                println!("error: {}", e);
                                            } else {
                                                print_match_repl(query, m, format);
                                            }
                                        }
                                    }
                                } else {
                                    // Output all to stdout
                                    let mut seen = std::collections::HashSet::new();
                                    for (_, m) in &all_matches {
                                        let key = format!("{}:{}", m.hash, m.name);
                                        if seen.insert(key) {
                                            if let Err(e) = ctx.get(&m.name, Some("-")).await {
                                                println!("error: {}", e);
                                            }
                                        }
                                    }
                                }
                                return Ok(());
                            }

                            // Single match
                            if all_matches.len() == 1 {
                                let (_, m) = &all_matches[0];
                                let output = if to_file {
                                    output_file.unwrap_or(&m.name)
                                } else {
                                    "-"
                                };
                                return ctx.get(&m.name, Some(output)).await;
                            }

                            // Multiple matches - show numbered list and prompt for selection
                            println!("found {} matches:", all_matches.len());
                            for (i, (query, m)) in all_matches.iter().enumerate() {
                                let kind_str = match m.kind {
                                    MatchKind::Exact => "exact",
                                    MatchKind::Prefix => "prefix",
                                    MatchKind::Contains => "contains",
                                };
                                let match_type = if m.is_hash_match { "hash" } else { "name" };
                                match format {
                                    "tag" => println!("[{}]\t{}\t{}\t{}\t({} {})", i + 1, query, m.hash, m.name, kind_str, match_type),
                                    "group" => println!("[{}]\t{}\t{}\t({} {})", i + 1, m.hash, m.name, kind_str, match_type),
                                    _ => println!("[{}]\t{}\t{}\t({} {}) [{}]", i + 1, m.hash, m.name, kind_str, match_type, query),
                                }
                            }
                            println!("select numbers (e.g., '1 3 5' or '1,2,3') or enter to cancel:");
                            
                            match rl.readline("? ") {
                                Ok(sel) => {
                                    let sel = sel.trim();
                                    if sel.is_empty() {
                                        println!("cancelled");
                                        return Ok(());
                                    }
                                    
                                    // Parse selection: split on comma and space, parse integers
                                    let selections: Vec<usize> = sel
                                        .split(|c| c == ',' || c == ' ')
                                        .filter(|s| !s.is_empty())
                                        .filter_map(|s| s.trim().parse::<usize>().ok())
                                        .filter(|&n| n >= 1 && n <= all_matches.len())
                                        .collect();
                                    
                                    if selections.is_empty() {
                                        println!("invalid selection");
                                        return Ok(());
                                    }

                                    // Determine output mode
                                    if let Some(dir_path) = dir {
                                        // Output to directory AND stdout
                                        if let Err(e) = std::fs::create_dir_all(dir_path) {
                                            println!("error creating directory: {}", e);
                                            return Ok(());
                                        }
                                        for n in &selections {
                                            let (_, m) = &all_matches[n - 1];
                                            let output_path = format!("{}/{}", dir_path, m.name);
                                            // Write to file
                                            if let Err(e) = ctx.get(&m.name, Some(&output_path)).await {
                                                println!("error: {}", e);
                                            }
                                            // Also output to stdout
                                            if let Err(e) = ctx.get(&m.name, Some("-")).await {
                                                println!("error: {}", e);
                                            }
                                        }
                                    } else if to_file {
                                        // Output to file(s)
                                        for n in &selections {
                                            let (_, m) = &all_matches[n - 1];
                                            let output = output_file.unwrap_or(&m.name);
                                            if let Err(e) = ctx.get(&m.name, Some(output)).await {
                                                println!("error: {}", e);
                                            }
                                        }
                                    } else {
                                        // Output to stdout in selection order
                                        for n in &selections {
                                            let (_, m) = &all_matches[n - 1];
                                            if let Err(e) = ctx.get(&m.name, Some("-")).await {
                                                println!("error: {}", e);
                                            }
                                        }
                                    }
                                    Ok(())
                                }
                                _ => {
                                    println!("cancelled");
                                    Ok(())
                                }
                            }
                        }
                        (None, ["search", rest @ ..]) => {
                            // Parse queries (args before flags) and flags
                            let mut queries: Vec<&str> = Vec::new();
                            let mut prefer_name = false;
                            let mut all = false;
                            let mut output_file: Option<&str> = None;
                            let mut dir: Option<&str> = None;
                            let mut to_file = false;
                            let mut format = "union"; // REPL default is union
                            
                            let mut i = 0;
                            while i < rest.len() {
                                let arg = rest[i];
                                if arg == "--name" {
                                    prefer_name = true;
                                } else if arg == "--all" || arg == "--out" || arg == "--export" || arg == "--save" || arg == "--full" {
                                    all = true;
                                } else if arg == "--file" {
                                    to_file = true;
                                } else if arg.starts_with('>') {
                                    output_file = Some(&arg[1..]);
                                    to_file = true;
                                } else if arg == "--dir" {
                                    if i + 1 < rest.len() {
                                        dir = Some(rest[i + 1]);
                                        i += 1;
                                    }
                                } else if arg == "--format" {
                                    if i + 1 < rest.len() {
                                        format = rest[i + 1];
                                        i += 1;
                                    }
                                } else if arg == "--tag" {
                                    format = "tag";
                                } else if arg == "--group" {
                                    format = "group";
                                } else if arg == "--union" {
                                    format = "union";
                                } else if !arg.starts_with('-') {
                                    queries.push(arg);
                                }
                                i += 1;
                            }
                            
                            if queries.is_empty() {
                                println!("usage: search <query>... [--name] [--all] [--dir <dir>] [--file] [>filename]");
                                return Ok(());
                            }

                            // Collect matches for all queries
                            let mut all_matches: Vec<(String, FindMatch)> = Vec::new();
                            for query in &queries {
                                match ctx.find(query, prefer_name).await {
                                    Ok(matches) => {
                                        for m in matches {
                                            all_matches.push((query.to_string(), m));
                                        }
                                    }
                                    Err(e) => {
                                        println!("error searching for '{}': {}", query, e);
                                    }
                                }
                            }

                            if all_matches.is_empty() {
                                println!("no matches found for: {}", queries.join(", "));
                                return Ok(());
                            }

                            // --all mode: output all matches to files
                            if all {
                                if let Some(dir_path) = dir {
                                    if let Err(e) = std::fs::create_dir_all(dir_path) {
                                        println!("error creating directory: {}", e);
                                        return Ok(());
                                    }
                                    let mut seen = std::collections::HashSet::new();
                                    for (query, m) in &all_matches {
                                        let key = format!("{}:{}", m.hash, m.name);
                                        if seen.insert(key) {
                                            let output_path = format!("{}/{}", dir_path, m.name);
                                            if let Err(e) = ctx.get(&m.name, Some(&output_path)).await {
                                                println!("error: {}", e);
                                            } else {
                                                print_match_repl(query, m, format);
                                            }
                                        }
                                    }
                                } else {
                                    // Output all to stdout
                                    let mut seen = std::collections::HashSet::new();
                                    for (_, m) in &all_matches {
                                        let key = format!("{}:{}", m.hash, m.name);
                                        if seen.insert(key) {
                                            if let Err(e) = ctx.get(&m.name, Some("-")).await {
                                                println!("error: {}", e);
                                            }
                                        }
                                    }
                                }
                                return Ok(());
                            }

                            // Default: list matches (union format for REPL)
                            for (query, m) in &all_matches {
                                print_match_repl(query, m, format);
                            }

                            // If --file or >filename, also output first match to file
                            if to_file {
                                let (_, m) = &all_matches[0];
                                let output = output_file.unwrap_or(&m.name);
                                ctx.get(&m.name, Some(output)).await
                            } else {
                                Ok(())
                            }
                        }
                        _ => {
                            println!("unknown command: {}", line);
                            println!("type 'help' for available commands");
                            Ok(())
                        }
                    }
                };

                if let Err(e) = result {
                    println!("error: {}", e);
                }
            }
            Err(ReadlineError::Interrupted) => {
                ctrl_c_count += 1;
                if ctrl_c_count >= 2 {
                    println!("^C");
                    break;
                }
                println!("^C (press Ctrl+C again, Ctrl+D, or type 'quit' to exit)");
                continue;
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

// ============================================================================
// Command handlers
// ============================================================================

async fn cmd_serve(ephemeral: bool, no_relay: bool) -> Result<()> {
    let key = load_or_create_keypair(KEY_FILE).await?;
    let node_id: EndpointId = key.public().into();
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

async fn cmd_id() -> Result<()> {
    let key = load_or_create_keypair(KEY_FILE).await?;
    let node_id: EndpointId = key.public().into();
    println!("{}", node_id);
    Ok(())
}

async fn cmd_list(node: Option<String>, no_relay: bool) -> Result<()> {
    // Remote list
    if let Some(node_id_str) = node {
        if !is_node_id(&node_id_str) {
            bail!("invalid node ID: must be 64 hex characters");
        }
        let server_node_id: EndpointId = node_id_str.parse()?;
        return cmd_list_remote(server_node_id, no_relay).await;
    }

    // Local list
    if let Some(serve_info) = get_serve_info().await {
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
    }
    Ok(())
}

async fn cmd_list_remote(server_node_id: EndpointId, no_relay: bool) -> Result<()> {
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
}

async fn cmd_gethash(hash_str: &str, output: &str) -> Result<()> {
    // Validate hash format before parsing (64 hex chars)
    if hash_str.len() != 64 || !hash_str.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("invalid hash: expected 64 hex characters");
    }
    let hash: Hash = hash_str.parse().context("invalid hash")?;

    if let Some(serve_info) = get_serve_info().await {
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
    } else {
        let store = open_store(false).await?;
        let store_handle = store.as_store();

        export_blob(&store_handle, hash, output).await?;
        store.shutdown().await?;
    }
    Ok(())
}

async fn cmd_put_hash(source: &str) -> Result<()> {
    let data = if source == "-" {
        read_input("-").await?
    } else {
        afs::read(source).await?
    };

    if let Some(serve_info) = get_serve_info().await {
        // Store in local ephemeral store, push blob to serve
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

        let blobs_conn = endpoint.connect(endpoint_addr.clone(), BLOBS_ALPN).await?;
        let push_request =
            PushRequest::new(hash, ChunkRangesSeq::from_ranges([ChunkRanges::all()]));
        store_handle
            .remote()
            .execute_push(blobs_conn.clone(), push_request)
            .await?;
        blobs_conn.close(0u32.into(), b"done");

        println!("{}", hash);
        store.shutdown().await?;
    } else {
        let store = open_store(false).await?;
        let store_handle = store.as_store();

        let added = store_handle
            .add_bytes_with_opts(AddBytesOptions {
                data: data.into(),
                format: BlobFormat::Raw,
            })
            .await?;

        println!("{}", added.hash);
        store.shutdown().await?;
    }
    Ok(())
}

async fn cmd_put_local_file(path: &str, custom_name: Option<String>) -> Result<()> {
    let path = PathBuf::from(path);
    let filename = custom_name.unwrap_or_else(|| {
        path.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unnamed".to_string())
    });
    let data = afs::read(&path).await?;

    if let Some(serve_info) = get_serve_info().await {
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
                let blobs_conn = endpoint.connect(endpoint_addr.clone(), BLOBS_ALPN).await?;
                let push_request =
                    PushRequest::new(hash, ChunkRangesSeq::from_ranges([ChunkRanges::all()]));
                store_handle
                    .remote()
                    .execute_push(blobs_conn.clone(), push_request)
                    .await?;
                blobs_conn.close(0u32.into(), b"done");
                eprintln!("stored: {} -> {}", filename, hash);
                store.shutdown().await?;
            }
            MetaResponse::Put { success: false } => bail!("server rejected"),
            _ => bail!("unexpected response"),
        }
    } else {
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
    }
    Ok(())
}

async fn cmd_put_local_stdin(name: &str) -> Result<()> {
    let data = read_input("-").await?;

    if let Some(serve_info) = get_serve_info().await {
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
                let blobs_conn = endpoint.connect(endpoint_addr.clone(), BLOBS_ALPN).await?;
                let push_request =
                    PushRequest::new(hash, ChunkRangesSeq::from_ranges([ChunkRanges::all()]));
                store_handle
                    .remote()
                    .execute_push(blobs_conn.clone(), push_request)
                    .await?;
                blobs_conn.close(0u32.into(), b"done");
                eprintln!("stored: {} -> {}", name, hash);
                store.shutdown().await?;
            }
            MetaResponse::Put { success: false } => bail!("server rejected"),
            _ => bail!("unexpected response"),
        }
    } else {
        let store = open_store(false).await?;
        let store_handle = store.as_store();

        let added = store_handle
            .add_bytes_with_opts(AddBytesOptions {
                data: data.into(),
                format: BlobFormat::Raw,
            })
            .await?;

        store_handle.tags().set(name, added.hash).await?;
        eprintln!("stored: {} -> {}", name, added.hash);
        store.shutdown().await?;
    }
    Ok(())
}

async fn cmd_get_local(name: &str, output: &str) -> Result<()> {
    if let Some(serve_info) = get_serve_info().await {
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

        export_blob(&store_handle, hash, output).await?;
        store.shutdown().await?;
    } else {
        let store = open_store(false).await?;
        let store_handle = store.as_store();

        let tag = store_handle
            .tags()
            .get(name)
            .await?
            .context("file not found")?;

        export_blob(&store_handle, tag.hash, output).await?;
        store.shutdown().await?;
    }
    Ok(())
}

/// Get a single item by name or hash (for multi-get)
async fn cmd_get_one(source: &str, output: &str, hash_mode: bool, name_only: bool) -> Result<()> {
    let is_valid_hash = source.len() == 64 && source.chars().all(|c| c.is_ascii_hexdigit());

    // If --hash flag, treat as hash lookup
    if hash_mode {
        return cmd_gethash(source, output).await;
    }

    // If it looks like a hash (64 hex chars) and not --name-only, try hash first
    if is_valid_hash && !name_only {
        if let Ok(hash) = source.parse::<Hash>() {
            if let Some(serve_info) = get_serve_info().await {
                let store = open_store(true).await?;
                let store_handle = store.as_store();
                let (endpoint, endpoint_addr) = create_local_client_endpoint(&serve_info).await?;
                let blobs_conn = endpoint.connect(endpoint_addr.clone(), BLOBS_ALPN).await?;

                match store_handle.remote().fetch(blobs_conn.clone(), hash).await {
                    Ok(_) => {
                        blobs_conn.close(0u32.into(), b"done");
                        export_blob(&store_handle, hash, output).await?;
                        store.shutdown().await?;
                        return Ok(());
                    }
                    Err(_) => {
                        blobs_conn.close(0u32.into(), b"done");
                    }
                }
                store.shutdown().await?;
            } else {
                let store = open_store(false).await?;
                let store_handle = store.as_store();
                if store_handle.blobs().has(hash).await? {
                    export_blob(&store_handle, hash, output).await?;
                    store.shutdown().await?;
                    return Ok(());
                }
                store.shutdown().await?;
            }
        }
    }

    // Try as name
    cmd_get_local(source, output).await
}

/// Get a single file from a remote node
async fn cmd_get_one_remote(
    server_node_id: EndpointId,
    name: &str,
    output: &str,
    no_relay: bool,
) -> Result<()> {
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

/// Get multiple items - local or from a remote node
/// If first argument is a NODE_ID, remaining items are fetched from that remote
async fn cmd_get_multi(
    sources: Vec<String>,
    from_stdin: bool,
    hash_mode: bool,
    name_only: bool,
    to_stdout: bool,
    no_relay: bool,
) -> Result<()> {
    let mut items = sources;

    // Check if first arg is a remote node ID
    let remote_node: Option<EndpointId> = if !items.is_empty() && is_node_id(&items[0]) {
        let node_id: EndpointId = items[0].parse()?;
        items.remove(0);
        Some(node_id)
    } else {
        None
    };

    if from_stdin {
        items.extend(parse_stdin_items()?);
    }

    if items.is_empty() {
        bail!("no sources provided");
    }

    let mut errors = Vec::new();
    for spec in &items {
        let (source, explicit_output) = parse_get_spec(spec);
        // Priority: --stdout flag > explicit :output > source name
        let output = if to_stdout {
            "-"
        } else if let Some(out) = explicit_output {
            out
        } else {
            source
        };
        let result = if let Some(node_id) = remote_node {
            cmd_get_one_remote(node_id, source, output, no_relay).await
        } else {
            cmd_get_one(source, output, hash_mode, name_only).await
        };
        if let Err(e) = result {
            errors.push(format!("{}: {}", source, e));
        }
    }

    if !errors.is_empty() {
        bail!("some gets failed:\n{}", errors.join("\n"));
    }
    Ok(())
}

/// Parse a put spec: "path" or "path:name"
fn parse_put_spec(spec: &str) -> (&str, Option<&str>) {
    if let Some(idx) = spec.rfind(':') {
        // Check if this looks like a Windows path (e.g., C:\path) - single letter before colon
        if idx == 1 && spec.len() > 2 {
            return (spec, None);
        }
        let (path, name) = spec.split_at(idx);
        (path, Some(&name[1..])) // skip the ':'
    } else {
        (spec, None)
    }
}

/// Parse a get spec: "source" or "source:output"
fn parse_get_spec(spec: &str) -> (&str, Option<&str>) {
    if let Some(idx) = spec.rfind(':') {
        // Check if this looks like a Windows path (e.g., C:\path) - single letter before colon
        if idx == 1 && spec.len() > 2 {
            return (spec, None);
        }
        let (source, output) = spec.split_at(idx);
        (source, Some(&output[1..])) // skip the ':'
    } else {
        (spec, None)
    }
}

/// Put a single local file with optional custom name (for multi-put)
async fn cmd_put_one(path: &str, name: Option<&str>, hash_only: bool) -> Result<()> {
    if hash_only {
        cmd_put_hash(path).await
    } else {
        cmd_put_local_file(path, name.map(|s| s.to_string())).await
    }
}

/// Put a single file to a remote node
async fn cmd_put_one_remote(
    server_node_id: EndpointId,
    path: &str,
    name: Option<&str>,
    no_relay: bool,
) -> Result<()> {
    let path_buf = PathBuf::from(path);
    let filename = if let Some(n) = name {
        n.to_string()
    } else {
        path_buf
            .file_name()
            .context("invalid filename")?
            .to_string_lossy()
            .to_string()
    };

    let store = open_store(true).await?;
    let store_handle = store.as_store();

    let data = afs::read(&path_buf).await?;
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
        }
        MetaResponse::Put { success: false } => bail!("server rejected"),
        _ => bail!("unexpected response"),
    }
    Ok(())
}


/// Put multiple files - local or to a remote node
/// If first argument is a NODE_ID, remaining files are sent to that remote
/// Auto-detects stdin content if no files provided and stdin is piped
async fn cmd_put_multi(
    files: Vec<String>,
    content_mode: bool,
    from_stdin: bool,
    hash_only: bool,
    no_relay: bool,
) -> Result<()> {
    // Content mode: read stdin as content, store with given name
    if content_mode {
        if files.len() != 1 {
            bail!("--content requires exactly one name argument");
        }
        let name = &files[0];
        if hash_only {
            return cmd_put_hash("-").await;
        } else {
            return cmd_put_local_stdin(name).await;
        }
    }

    let mut items = files;

    // Check if first arg is a remote node ID
    let remote_node: Option<EndpointId> = if !items.is_empty() && is_node_id(&items[0]) {
        let node_id: EndpointId = items[0].parse()?;
        items.remove(0);
        Some(node_id)
    } else {
        None
    };

    if from_stdin {
        items.extend(parse_stdin_items()?);
    }

    // Auto-detect stdin content: if exactly one arg (the name) and stdin is piped
    if items.len() == 1 && !std::io::stdin().is_terminal() && !from_stdin {
        // Check if the item looks like a file path that exists
        let path = PathBuf::from(&items[0]);
        if !path.exists() {
            // Doesn't exist as a file, treat as name and read content from stdin
            let name = &items[0];
            if hash_only {
                return cmd_put_hash("-").await;
            } else {
                return cmd_put_local_stdin(name).await;
            }
        }
    }

    if items.is_empty() {
        bail!("no files provided");
    }

    let mut errors = Vec::new();
    for spec in &items {
        let (path, name) = parse_put_spec(spec);
        let result = if let Some(node_id) = remote_node {
            cmd_put_one_remote(node_id, path, name, no_relay).await
        } else {
            cmd_put_one(path, name, hash_only).await
        };
        if let Err(e) = result {
            errors.push(format!("{}: {}", spec, e));
        }
    }

    if !errors.is_empty() {
        bail!("some puts failed:\n{}", errors.join("\n"));
    }
    Ok(())
}

/// Find files matching queries - CLI version (defaults to file output)
/// Supports multiple queries with format options: tag, group, union
async fn cmd_find(
    queries: Vec<String>,
    prefer_name: bool,
    to_stdout: bool,
    all: bool,
    dir: Option<String>,
    format: &str,
    node: Option<String>,
    no_relay: bool,
) -> Result<()> {
    // Collect matches for all queries
    let mut all_matches: Vec<TaggedMatch> = Vec::new();
    for query in &queries {
        let matches = cmd_find_matches(query, prefer_name, node.clone(), no_relay).await?;
        for m in matches {
            all_matches.push(TaggedMatch {
                query: query.clone(),
                hash: m.hash,
                name: m.name,
                kind: m.kind,
                is_hash_match: m.is_hash_match,
            });
        }
    }

    if all_matches.is_empty() {
        bail!("no matches found for: {}", queries.join(", "));
    }

    // --all mode: output all matches
    if all {
        if let Some(ref dir_path) = dir {
            std::fs::create_dir_all(dir_path)?;
            // Deduplicate by hash+name for file output
            let mut seen = std::collections::HashSet::new();
            for m in &all_matches {
                let key = format!("{}:{}", m.hash, m.name);
                if seen.insert(key) {
                    let output_path = format!("{}/{}", dir_path, m.name);
                    if let Some(ref node_str) = node {
                        let node_id: EndpointId = node_str.parse()?;
                        cmd_get_one_remote(node_id, &m.name, &output_path, no_relay).await?;
                    } else {
                        cmd_get_one(&m.name, &output_path, false, false).await?;
                    }
                    print_match_cli(m, format);
                }
            }
        } else {
            // Output all to stdout (concatenated)
            let mut seen = std::collections::HashSet::new();
            for m in &all_matches {
                let key = format!("{}:{}", m.hash, m.name);
                if seen.insert(key) {
                    if let Some(ref node_str) = node {
                        let node_id: EndpointId = node_str.parse()?;
                        cmd_get_one_remote(node_id, &m.name, "-", no_relay).await?;
                    } else {
                        cmd_get_one(&m.name, "-", false, false).await?;
                    }
                }
            }
        }
        return Ok(());
    }

    // Single match or first match mode
    if all_matches.len() == 1 {
        let m = &all_matches[0];
        let output = if to_stdout { "-" } else { &m.name };
        if node.is_some() {
            let node_id: EndpointId = node.unwrap().parse()?;
            cmd_get_one_remote(node_id, &m.name, output, no_relay).await?;
        } else {
            cmd_get_one(&m.name, output, false, false).await?;
        }
    } else {
        // Multiple matches - print them and use first one
        eprintln!("found {} matches (using first):", all_matches.len());
        print_matches_cli(&all_matches, format);
        let m = &all_matches[0];
        let output = if to_stdout { "-" } else { &m.name };
        if let Some(node_str) = node {
            let node_id: EndpointId = node_str.parse()?;
            cmd_get_one_remote(node_id, &m.name, output, no_relay).await?;
        } else {
            cmd_get_one(&m.name, output, false, false).await?;
        }
    }
    Ok(())
}

/// Search files matching queries - CLI version (list only, or --all to output files)
/// Supports multiple queries with format options: tag, group, union
async fn cmd_search(
    queries: Vec<String>,
    prefer_name: bool,
    all: bool,
    dir: Option<String>,
    format: &str,
    node: Option<String>,
    no_relay: bool,
) -> Result<()> {
    // Collect matches for all queries
    let mut all_matches: Vec<TaggedMatch> = Vec::new();
    for query in &queries {
        let matches = cmd_find_matches(query, prefer_name, node.clone(), no_relay).await?;
        for m in matches {
            all_matches.push(TaggedMatch {
                query: query.clone(),
                hash: m.hash,
                name: m.name,
                kind: m.kind,
                is_hash_match: m.is_hash_match,
            });
        }
    }

    if all_matches.is_empty() {
        println!("no matches found for: {}", queries.join(", "));
        return Ok(());
    }

    // --all mode: output all files (like find --all)
    if all {
        if let Some(ref dir_path) = dir {
            std::fs::create_dir_all(dir_path)?;
            let mut seen = std::collections::HashSet::new();
            for m in &all_matches {
                let key = format!("{}:{}", m.hash, m.name);
                if seen.insert(key) {
                    let output_path = format!("{}/{}", dir_path, m.name);
                    if let Some(ref node_str) = node {
                        let node_id: EndpointId = node_str.parse()?;
                        cmd_get_one_remote(node_id, &m.name, &output_path, no_relay).await?;
                    } else {
                        cmd_get_one(&m.name, &output_path, false, false).await?;
                    }
                    print_match_cli(m, format);
                }
            }
        } else {
            // Output all to stdout (concatenated)
            let mut seen = std::collections::HashSet::new();
            for m in &all_matches {
                let key = format!("{}:{}", m.hash, m.name);
                if seen.insert(key) {
                    if let Some(ref node_str) = node {
                        let node_id: EndpointId = node_str.parse()?;
                        cmd_get_one_remote(node_id, &m.name, "-", no_relay).await?;
                    } else {
                        cmd_get_one(&m.name, "-", false, false).await?;
                    }
                }
            }
        }
        return Ok(());
    }

    // Default: just list matches
    print_matches_cli(&all_matches, format);
    Ok(())
}

/// Print a single match in CLI format
fn print_match_cli(m: &TaggedMatch, format: &str) {
    let kind_str = match m.kind {
        MatchKind::Exact => "exact",
        MatchKind::Prefix => "prefix",
        MatchKind::Contains => "contains",
    };
    let match_type = if m.is_hash_match { "hash" } else { "name" };
    
    match format {
        "tag" => println!("{}\t{}\t{}\t({} {})", m.query, m.hash, m.name, kind_str, match_type),
        "union" => println!("{}\t{}\t({} {}) [{}]", m.hash, m.name, kind_str, match_type, m.query),
        _ => println!("{}\t{}\t({} {})", m.hash, m.name, kind_str, match_type), // group or default
    }
}

/// Print matches in CLI format based on format option
fn print_matches_cli(matches: &[TaggedMatch], format: &str) {
    match format {
        "group" => {
            // Group by query
            let mut current_query: Option<&str> = None;
            for m in matches {
                if current_query != Some(&m.query) {
                    eprintln!("=== {} ===", m.query);
                    current_query = Some(&m.query);
                }
                let kind_str = match m.kind {
                    MatchKind::Exact => "exact",
                    MatchKind::Prefix => "prefix",
                    MatchKind::Contains => "contains",
                };
                let match_type = if m.is_hash_match { "hash" } else { "name" };
                println!("{}\t{}\t({} {})", m.hash, m.name, kind_str, match_type);
            }
        }
        "union" => {
            for m in matches {
                let kind_str = match m.kind {
                    MatchKind::Exact => "exact",
                    MatchKind::Prefix => "prefix",
                    MatchKind::Contains => "contains",
                };
                let match_type = if m.is_hash_match { "hash" } else { "name" };
                println!("{}\t{}\t({} {}) [{}]", m.hash, m.name, kind_str, match_type, m.query);
            }
        }
        _ => {
            // "tag" format (default): query as first column
            for m in matches {
                let kind_str = match m.kind {
                    MatchKind::Exact => "exact",
                    MatchKind::Prefix => "prefix",
                    MatchKind::Contains => "contains",
                };
                let match_type = if m.is_hash_match { "hash" } else { "name" };
                println!("{}\t{}\t{}\t({} {})", m.query, m.hash, m.name, kind_str, match_type);
            }
        }
    }
}

/// Print a single match in REPL format (used by REPL find/search)
fn print_match_repl(query: &str, m: &FindMatch, format: &str) {
    let kind_str = match m.kind {
        MatchKind::Exact => "exact",
        MatchKind::Prefix => "prefix",
        MatchKind::Contains => "contains",
    };
    let match_type = if m.is_hash_match { "hash" } else { "name" };
    
    match format {
        "tag" => println!("{}\t{}\t{}\t({} {})", query, m.hash, m.name, kind_str, match_type),
        "group" => println!("{}\t{}\t({} {})", m.hash, m.name, kind_str, match_type),
        _ => println!("{}\t{}\t({} {}) [{}]", m.hash, m.name, kind_str, match_type, query), // union (default for REPL)
    }
}

/// Get find matches (shared by cmd_find and cmd_search)
async fn cmd_find_matches(
    query: &str,
    prefer_name: bool,
    node: Option<String>,
    no_relay: bool,
) -> Result<Vec<FindMatch>> {
    if let Some(node_str) = node {
        let node_id: EndpointId = node_str.parse()?;
        let client_key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
        let mut builder = Endpoint::builder()
            .secret_key(client_key)
            .address_lookup(PkarrPublisher::n0_dns())
            .address_lookup(DnsAddressLookup::n0_dns());
        if no_relay {
            builder = builder.relay_mode(RelayMode::Disabled);
        }
        let endpoint = builder.bind().await?;

        let meta_conn = endpoint.connect(node_id, META_ALPN).await?;
        let (mut send, mut recv) = meta_conn.open_bi().await?;
        let req = postcard::to_allocvec(&MetaRequest::Find {
            query: query.to_string(),
            prefer_name,
        })?;
        send.write_all(&req).await?;
        send.finish()?;
        let resp_buf = recv.read_to_end(64 * 1024).await?;
        let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
        meta_conn.close(0u32.into(), b"done");

        match resp {
            MetaResponse::Find { matches } => Ok(matches),
            _ => bail!("unexpected response"),
        }
    } else {
        // Local search
        let store = open_store(false).await?;
        let store_handle = store.as_store();
        let mut matches = Vec::new();
        let query_lower = query.to_lowercase();

        if let Ok(mut list) = store_handle.tags().list().await {
            while let Some(item) = list.next().await {
                if let Ok(item) = item {
                    let name = String::from_utf8_lossy(item.name.as_ref()).to_string();
                    let hash_str = item.hash.to_string();
                    let name_lower = name.to_lowercase();

                    if let Some(kind) = match_kind(&name_lower, &query_lower) {
                        matches.push(FindMatch {
                            hash: item.hash,
                            name: name.clone(),
                            kind,
                            is_hash_match: false,
                        });
                    } else if let Some(kind) = match_kind(&hash_str, &query_lower) {
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

        store.shutdown().await?;
        Ok(matches)
    }
}

/// Helper function for matching (used by cmd_find_matches)
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

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    let cli = Cli::parse();

    match cli.command {
        None => run_repl(None).await,
        Some(Command::Repl { node }) => run_repl(node).await,
        Some(Command::Serve {
            ephemeral,
            no_relay,
        }) => cmd_serve(ephemeral, no_relay).await,
        Some(Command::Id) => cmd_id().await,
        Some(Command::List { node, no_relay }) => cmd_list(node, no_relay).await,
        Some(Command::GetHash { hash, output }) => cmd_gethash(&hash, &output).await,
        Some(Command::Put {
            files,
            content,
            stdin,
            hash_only,
            no_relay,
        }) => cmd_put_multi(files, content, stdin, hash_only, no_relay).await,
        Some(Command::PutHash { source }) => cmd_put_hash(&source).await,
        Some(Command::Get {
            sources,
            stdin,
            hash,
            name_only,
            stdout,
            no_relay,
        }) => cmd_get_multi(sources, stdin, hash, name_only, stdout, no_relay).await,
        Some(Command::Cat {
            sources,
            stdin,
            hash,
            name_only,
            no_relay,
        }) => cmd_get_multi(sources, stdin, hash, name_only, true, no_relay).await,
        Some(Command::Find {
            queries,
            name,
            stdout,
            all,
            dir,
            format,
            node,
            no_relay,
        }) => cmd_find(queries, name, stdout, all, dir, &format, node, no_relay).await,
        Some(Command::Search {
            queries,
            name,
            all,
            dir,
            format,
            node,
            no_relay,
        }) => cmd_search(queries, name, all, dir, &format, node, no_relay).await,
    }
}
