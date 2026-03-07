use anyhow::{Context, Result, anyhow, bail};
use futures_lite::StreamExt;
use iroh::{
    discovery::dns::DnsDiscovery,
    endpoint::{Connection, Endpoint, RelayMode},
    protocol::{AcceptError, ProtocolHandler, Router},
};
use iroh_base::{EndpointId, SecretKey};
use iroh_blobs::{
    ALPN as BLOBS_ALPN, BlobFormat, BlobsProtocol, Hash,
    api::{Store, blobs::AddBytesOptions},
    protocol::{ChunkRanges, ChunkRangesSeq, PushRequest},
    store::{fs::FsStore, mem::MemStore},
};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
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

#[derive(Debug, Serialize, Deserialize)]
enum MetaRequest {
    Put { filename: String, hash: Hash },
    Get { filename: String },
    List,
}

#[derive(Debug, Serialize, Deserialize)]
enum MetaResponse {
    Put { success: bool },
    Get { hash: Option<Hash> },
    List { items: Vec<(Hash, String)> },
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
}

impl ProtocolHandler for MetaProtocol {
    async fn accept(&self, conn: Connection) -> std::result::Result<(), AcceptError> {
        let (mut send, mut recv) = conn.accept_bi().await?;
        let buf = recv
            .read_to_end(64 * 1024)
            .await
            .map_err(AcceptError::from_err)?;
        let req: MetaRequest = postcard::from_bytes(&buf).map_err(AcceptError::from_err)?;
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
        }
        conn.closed().await;
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

/// Check if serve is running by reading the lock file
async fn get_serve_node_id() -> Option<EndpointId> {
    if let Ok(contents) = afs::read_to_string(SERVE_LOCK).await {
        contents.trim().parse().ok()
    } else {
        None
    }
}

/// Create serve lock file with node ID
async fn create_serve_lock(node_id: &EndpointId) -> Result<()> {
    afs::write(SERVE_LOCK, node_id.to_string()).await?;
    Ok(())
}

/// Remove serve lock file
async fn remove_serve_lock() -> Result<()> {
    let _ = afs::remove_file(SERVE_LOCK).await;
    Ok(())
}

fn print_usage() {
    eprintln!(
        r#"usage:
  id serve [--ephemeral] [--no-relay]      Start server (accepts put/get from peers)
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
  - put/get/list work while serve is running (connects via network)
  - Use - for stdin (put) or stdout (get)"#
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    let args: Vec<String> = std::env::args().skip(1).collect();
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
        ["id"] => {
            println!("{}", node_id);
            Ok(())
        }

        ["serve"] => {
            info!("serve: {}", node_id);
            let store = open_store(ephemeral).await?;
            let store_handle = store.as_store();

            let mut builder = Endpoint::builder()
                .secret_key(key.clone())
                .discovery(DnsDiscovery::n0_dns());
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
            create_serve_lock(&serve_node_id).await?;

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

            if let Some(server_node_id) = get_serve_node_id().await {
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

                let client_key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
                let endpoint = Endpoint::builder()
                    .secret_key(client_key)
                    .discovery(DnsDiscovery::n0_dns())
                    .relay_mode(RelayMode::Disabled)
                    .bind()
                    .await?;

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
                        endpoint.close().await;
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

            if let Some(server_node_id) = get_serve_node_id().await {
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

                let client_key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
                let endpoint = Endpoint::builder()
                    .secret_key(client_key)
                    .discovery(DnsDiscovery::n0_dns())
                    .relay_mode(RelayMode::Disabled)
                    .bind()
                    .await?;

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
                        endpoint.close().await;
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
                .discovery(DnsDiscovery::n0_dns());
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
                    endpoint.close().await;
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
                .discovery(DnsDiscovery::n0_dns());
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
                    endpoint.close().await;
                    store.shutdown().await?;
                    Ok(())
                }
                MetaResponse::Put { success: false } => bail!("server rejected"),
                _ => bail!("unexpected response"),
            }
        }

        ["get", name] => {
            // Local get by name - check if serve is running
            if let Some(server_node_id) = get_serve_node_id().await {
                // Serve is running - use remote protocol
                let store = open_store(true).await?;
                let store_handle = store.as_store();

                let client_key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
                let endpoint = Endpoint::builder()
                    .secret_key(client_key)
                    .discovery(DnsDiscovery::n0_dns())
                    .relay_mode(RelayMode::Disabled)
                    .bind()
                    .await?;

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
                    MetaResponse::Get { hash: None } => bail!("file not found"),
                    _ => bail!("unexpected response"),
                };

                let blobs_conn = endpoint.connect(server_node_id, BLOBS_ALPN).await?;
                store_handle
                    .remote()
                    .fetch(blobs_conn.clone(), hash)
                    .await?;
                blobs_conn.close(0u32.into(), b"done");

                export_blob(&store_handle, hash, name).await?;
                endpoint.close().await;
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
                    .discovery(DnsDiscovery::n0_dns());
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
                endpoint.close().await;
                store.shutdown().await?;
                Ok(())
            } else {
                // Local get with output: get <NAME> <OUTPUT>
                let name = *first;
                let output = *second;

                if let Some(server_node_id) = get_serve_node_id().await {
                    // Serve is running - use remote protocol
                    let store = open_store(true).await?;
                    let store_handle = store.as_store();

                    let client_key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
                    let endpoint = Endpoint::builder()
                        .secret_key(client_key)
                        .discovery(DnsDiscovery::n0_dns())
                        .relay_mode(RelayMode::Disabled)
                        .bind()
                        .await?;

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
                        MetaResponse::Get { hash: None } => bail!("file not found"),
                        _ => bail!("unexpected response"),
                    };

                    let blobs_conn = endpoint.connect(server_node_id, BLOBS_ALPN).await?;
                    store_handle
                        .remote()
                        .fetch(blobs_conn.clone(), hash)
                        .await?;
                    blobs_conn.close(0u32.into(), b"done");

                    export_blob(&store_handle, hash, output).await?;
                    endpoint.close().await;
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
                .discovery(DnsDiscovery::n0_dns());
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
            endpoint.close().await;
            store.shutdown().await?;
            Ok(())
        }

        ["list"] => {
            if let Some(server_node_id) = get_serve_node_id().await {
                // Serve is running - use remote protocol
                let client_key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
                let endpoint = Endpoint::builder()
                    .secret_key(client_key)
                    .discovery(DnsDiscovery::n0_dns())
                    .relay_mode(RelayMode::Disabled)
                    .bind()
                    .await?;

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
                endpoint.close().await;
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

            if let Some(server_node_id) = get_serve_node_id().await {
                // Serve is running - fetch from serve
                let store = open_store(true).await?;
                let store_handle = store.as_store();

                let client_key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
                let endpoint = Endpoint::builder()
                    .secret_key(client_key)
                    .discovery(DnsDiscovery::n0_dns())
                    .relay_mode(RelayMode::Disabled)
                    .bind()
                    .await?;

                let blobs_conn = endpoint.connect(server_node_id, BLOBS_ALPN).await?;
                store_handle
                    .remote()
                    .fetch(blobs_conn.clone(), hash)
                    .await?;
                blobs_conn.close(0u32.into(), b"done");

                export_blob(&store_handle, hash, output).await?;
                endpoint.close().await;
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
