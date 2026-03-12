use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
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
    Put {
        /// File paths to store (use path:name to rename, e.g. file.txt:stored.txt)
        /// If first arg is a 64-char hex NODE_ID, remaining args are sent to that remote node
        #[arg(required = false)]
        files: Vec<String>,
        /// Read content from stdin instead of file paths (requires one name argument)
        #[arg(long, conflicts_with = "stdin")]
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

/// Parse items from stdin, splitting on newline, tab, or comma
fn parse_stdin_items() -> Result<Vec<String>> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    Ok(input
        .split(|c| c == '\n' || c == '\t' || c == ',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect())
}

/// Execute a shell command and return its stdout
fn shell_capture(cmd: &str) -> Result<String> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .context("failed to execute shell command")?;
    if !output.status.success() {
        bail!(
            "command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Result of preprocessing a REPL line
enum ReplInput {
    /// Ready to execute with this line (possibly modified)
    Ready(String),
    /// Need more input - heredoc mode with delimiter
    NeedMore {
        delimiter: String,
        lines: Vec<String>,
        original_line: String,
    },
    /// Empty/whitespace only
    Empty,
}

/// Preprocess a REPL line, handling:
/// - $(...) and `...` command substitution
/// - <<< here-string
/// - <<DELIM heredoc start
/// - |> pipe operator (cmd |> put - name)
fn preprocess_repl_line(line: &str) -> Result<ReplInput> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(ReplInput::Empty);
    }

    // Check for heredoc: put - name <<EOF
    if let Some(heredoc_start) = line.find("<<") {
        let after = &line[heredoc_start + 2..];
        // Check it's not <<< (here-string)
        if !after.starts_with('<') {
            let delimiter = after.trim().to_string();
            if !delimiter.is_empty() {
                let original_line = line[..heredoc_start].trim().to_string();
                return Ok(ReplInput::NeedMore {
                    delimiter,
                    lines: Vec::new(),
                    original_line,
                });
            }
        }
    }

    let mut result = line.to_string();

    // Process here-string: <<< 'content' or <<< "content" or <<< content
    while let Some(pos) = result.find("<<<") {
        let before = &result[..pos];
        let after = &result[pos + 3..].trim_start();

        // Extract the content (quoted or unquoted)
        let (content, rest) = if after.starts_with('\'') {
            // Single-quoted
            if let Some(end) = after[1..].find('\'') {
                (&after[1..end + 1], &after[end + 2..])
            } else {
                bail!("unterminated single quote in here-string");
            }
        } else if after.starts_with('"') {
            // Double-quoted
            if let Some(end) = after[1..].find('"') {
                (&after[1..end + 1], &after[end + 2..])
            } else {
                bail!("unterminated double quote in here-string");
            }
        } else {
            // Unquoted - take until end (rest of line is content)
            (after.trim(), "")
        };

        // Replace - with content marker in the command
        // The pattern is: put - name <<< content
        let before_str = before.trim();
        let new_before = before_str
            .replace(" - ", &format!(" __STDIN_CONTENT__:{} ", content))
            .replace(" -$", &format!(" __STDIN_CONTENT__:{}", content));
        result = format!("{}{}", new_before, rest);
    }

    // Process $(...) command substitution - for put commands, treat as content
    while let Some(start) = result.find("$(") {
        let mut depth = 1;
        let mut end = start + 2;
        for (i, c) in result[start + 2..].chars().enumerate() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = start + 2 + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        if depth != 0 {
            bail!("unterminated $(...) in command");
        }
        let cmd = &result[start + 2..end];
        let output = shell_capture(cmd)?;

        // Check if this $() is the first arg to put - if so, treat as content
        let before = result[..start].trim();
        if before == "put" || before.ends_with(" put") {
            result = format!(
                "{}__STDIN_CONTENT__:{}{}",
                &result[..start],
                output,
                &result[end + 1..]
            );
        } else {
            result = format!("{}{}{}", &result[..start], output, &result[end + 1..]);
        }
    }

    // Process `...` backtick substitution - for put commands, treat as content
    while let Some(start) = result.find('`') {
        if let Some(end) = result[start + 1..].find('`') {
            let cmd = &result[start + 1..start + 1 + end];
            let output = shell_capture(cmd)?;

            // Check if this `` is the first arg to put - if so, treat as content
            let before = result[..start].trim();
            if before == "put" || before.ends_with(" put") {
                result = format!(
                    "{}__STDIN_CONTENT__:{}{}",
                    &result[..start],
                    output,
                    &result[start + 2 + end..]
                );
            } else {
                result = format!(
                    "{}{}{}",
                    &result[..start],
                    output,
                    &result[start + 2 + end..]
                );
            }
        } else {
            bail!("unterminated backtick in command");
        }
    }

    // Process |> pipe operator: echo hello |> put - name
    if let Some(pos) = result.find("|>") {
        let left = result[..pos].trim().to_string();
        let right = result[pos + 2..].trim().to_string();

        // Execute left side as shell command
        let output = shell_capture(&left)?;

        // Replace - in right side with stdin content marker
        let mut new_result = right
            .replace(" - ", &format!(" __STDIN_CONTENT__:{} ", output))
            .replace(" -\n", &format!(" __STDIN_CONTENT__:{}\n", output))
            .replace(" -$", &format!(" __STDIN_CONTENT__:{}", output));

        // If no - found, might be at end
        if !new_result.contains("__STDIN_CONTENT__") {
            // Append content as argument
            new_result = format!("{} __STDIN_CONTENT__:{}", right, output);
        }
        result = new_result;
    }

    Ok(ReplInput::Ready(result))
}

/// Continue reading heredoc lines until delimiter is found
fn continue_heredoc(
    rl: &mut DefaultEditor,
    delimiter: &str,
    lines: &mut Vec<String>,
) -> Result<Option<String>> {
    println!(
        "(heredoc: type '{}' on its own line to end, Ctrl+C to cancel)",
        delimiter
    );

    loop {
        match rl.readline(".. ") {
            Ok(line) => {
                if line.trim() == delimiter {
                    return Ok(Some(lines.join("\n")));
                }
                lines.push(line);
            }
            Err(ReadlineError::Interrupted) => {
                println!("^C (heredoc cancelled)");
                return Ok(None);
            }
            Err(ReadlineError::Eof) => {
                return Ok(None);
            }
            Err(e) => {
                bail!("readline error: {}", e);
            }
        }
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
    // Enable relay and DNS lookup so @NODE_ID targeting works for remote peers
    let endpoint = Endpoint::builder()
        .secret_key(client_key)
        .address_lookup(PkarrPublisher::n0_dns())
        .address_lookup(DnsAddressLookup::n0_dns())
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

    loop {
        match rl.readline("> ") {
            Ok(raw_line) => {
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
                                "  find <QUERY> [--name]  - Find files (exact/prefix/contains match)"
                            );
                            println!(
                                "  search <QUERY> [--name] - List all matches (no selection prompt)"
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
                        (None, ["put", path]) => ctx.put(path, None).await,
                        (None, ["put", path, name]) => ctx.put(path, Some(name)).await,
                        (None, ["get", name]) => ctx.get(name, None).await,
                        (None, ["get", name, output]) => ctx.get(name, Some(output)).await,
                        (None, ["cat", name]) => ctx.get(name, Some("-")).await,
                        (None, ["gethash", hash, output]) => ctx.gethash(hash, output).await,
                        (None, ["delete", name]) | (None, ["rm", name]) => ctx.delete(name).await,
                        (None, ["rename", from, to]) => ctx.rename(from, to).await,
                        (None, ["copy", from, to]) | (None, ["cp", from, to]) => {
                            ctx.copy(from, to).await
                        }
                        (None, ["find", query, rest @ ..]) => {
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
                                        let match_type =
                                            if m.is_hash_match { "hash" } else { "name" };
                                        println!(
                                            "  [{}] {}\t{} ({} {})",
                                            i + 1,
                                            m.hash,
                                            m.name,
                                            kind_str,
                                            match_type
                                        );
                                    }
                                    println!(
                                        "select [1-{}] or press enter to cancel:",
                                        matches.len()
                                    );
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
                        (None, ["search", query, rest @ ..]) => {
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
                                        let match_type =
                                            if m.is_hash_match { "hash" } else { "name" };
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
                    }
                };

                if let Err(e) = result {
                    println!("error: {}", e);
                }
            }
            Err(ReadlineError::Interrupted) => {
                // Ctrl+C - just print ^C and continue (user can type 'quit' or Ctrl+D to exit)
                println!("^C (type 'quit' or Ctrl+D to exit)");
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

/// Check if a string looks like a node ID (64 hex chars)
fn is_node_id(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Put multiple files - local or to a remote node
/// If first argument is a NODE_ID, remaining files are sent to that remote
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
    }
}
