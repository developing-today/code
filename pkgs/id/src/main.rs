use anyhow::{anyhow, bail, Context, Result};
use iroh::{
    discovery::pkarr::PkarrResolver,
    endpoint::{Connection, Endpoint},
    protocol::{AcceptError, ProtocolHandler, Router},
};
use iroh_base::{EndpointId, SecretKey};
use iroh_blobs::{
    api::{blobs::AddBytesOptions, Store},
    store::{fs, mem},
    BlobFormat, Hash, BlobsProtocol, ALPN as BLOBS_ALPN,
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use tokio::{fs as afs, io::AsyncWriteExt};
use tracing::{error, info};

const KEY_FILE: &str = ".iroh-key";
const META_ALPN: &[u8] = b"/iroh-meta/1";

#[derive(Debug, Serialize, Deserialize)]
enum MetaRequest {
    Put { filename: String, hash: Hash },
    Get { filename: String },
}

#[derive(Debug, Serialize, Deserialize)]
enum MetaResponse {
    Put { success: bool },
    Get { hash: Option<Hash> },
}

#[derive(Clone)]
struct MetaProtocol {
    store: Store,
}

impl MetaProtocol {
    fn new(store: &Store) -> Arc<Self> {
        Arc::new(Self { store: store.clone() })
    }
}

impl ProtocolHandler for MetaProtocol {
    async fn accept(&self, conn: Connection) -> std::result::Result<(), AcceptError> {
        let (mut send, mut recv) = conn.accept_bi().await?;
        let buf = recv.read_to_end(64 * 1024).await.map_err(AcceptError::from_err)?;
        let req: MetaRequest =
            postcard::from_bytes(&buf).map_err(|e| AcceptError::from_err(anyhow!(e)))?;
        match req {
            MetaRequest::Put { filename, hash } => {
                self.store
                    .tags()
                    .set(&filename, hash)
                    .await
                    .map_err(AcceptError::from_err)?;
                let resp =
                    postcard::to_allocvec(&MetaResponse::Put { success: true }).map_err(|e| {
                        AcceptError::from_err(anyhow!(e))
                    })?;
                send.write_all(&resp)
                    .await
                    .map_err(AcceptError::from_err)?;
                send.finish()?;
            }
            MetaRequest::Get { filename } => {
                let mut found: Option<Hash> = None;
                // prefer direct lookup if available
                if let Ok(Some(tag)) = self.store.tags().get(&filename).await {
                    found = Some(tag.hash);
                } else {
                    // fallback scan if direct get not supported
                    if let Ok(mut list) = self.store.tags().list().await {
                        while let Some(item) = list.next().await {
                            let item = item.map_err(AcceptError::from_err)?;
                            if item.name.as_str() == filename {
                                found = Some(item.hash);
                                break;
                            }
                        }
                    }
                }
                let resp =
                    postcard::to_allocvec(&MetaResponse::Get { hash: found }).map_err(|e| {
                        AcceptError::from_err(anyhow!(e))
                    })?;
                send.write_all(&resp)
                    .await
                    .map_err(AcceptError::from_err)?;
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
            let key = SecretKey::generate();
            afs::write(path, key.to_bytes()).await?;
            Ok(key)
        }
        Err(e) => Err(e.into()),
    }
}

async fn read_postcard(conn: &mut Connection, limit: usize) -> Result<Vec<u8>> {
    let (_s, mut r) = conn.open_bi().await?;
    let buf = r.read_to_end(limit).await?;
    Ok(buf)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    let key = load_or_create_keypair(KEY_FILE).await?;
    let node_id: EndpointId = key.public().into();
    let node_id_str = node_id.to_string();

    match args.as_slice() {
        ["serve"] => {
            info!("serve: {}", node_id_str);
            let endpoint = Endpoint::builder().secret_key(key.clone()).bind().await?;

            let store = mem::MemStore::new();
            let meta = MetaProtocol::new(&store);
            let blobs = BlobsProtocol::new(&store, None);

            let router = Router::builder(endpoint)
                .accept(META_ALPN, meta)
                .accept(BLOBS_ALPN, blobs)
                .spawn();

            println!("node: {}", router.endpoint().id());

            tokio::signal::ctrl_c().await?;
            router.shutdown().await?;
            store.shutdown().await?;
            Ok(())
        }

        ["put", server_node_id, path] => {
            let server_node_id: EndpointId = server_node_id.parse()?;
            let path = PathBuf::from(path);
            let filename = path
                .file_name()
                .context("invalid filename")?
                .to_string_lossy()
                .to_string();

            let endpoint = Endpoint::empty_builder(iroh::RelayMode::Default)
                .discovery(PkarrResolver::n0_dns())
                .secret_key(key.clone())
                .bind()
                .await?;

            let store = mem::MemStore::new();
            let data = afs::read(&path).await?;
            let added = store
                .add_bytes_with_opts(AddBytesOptions {
                    data: data.into(),
                    format: BlobFormat::Raw,
                })
                .await?;
            let hash = added.hash;

            let mut meta_conn = endpoint.connect(server_node_id, META_ALPN).await?;
            let (mut send, mut recv) = meta_conn.open_bi().await?;
            let req = postcard::to_allocvec(&MetaRequest::Put { filename, hash })?;
            send.write_all(&req).await?;
            send.finish()?;
            let resp_buf = recv.read_to_end(64 * 1024).await?;
            let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
            meta_conn.close(0u32.into(), b"done");

            match resp {
                MetaResponse::Put { success: true } => {
                    let blobs_conn = endpoint.connect(server_node_id, BLOBS_ALPN).await?;
                    store.remote().push(blobs_conn.clone(), hash).await?;
                    blobs_conn.close(0u32.into(), b"done");
                    endpoint.close().await;
                    store.shutdown().await?;
                    Ok(())
                }
                MetaResponse::Put { success: false } => bail!("server rejected"),
                _ => bail!("unexpected response"),
            }
        }

        ["get", server_node_id, path] => {
            let server_node_id: EndpointId = server_node_id.parse()?;
            let path = PathBuf::from(path);
            let filename = path
                .file_name()
                .context("invalid filename")?
                .to_string_lossy()
                .to_string();

            let endpoint = Endpoint::empty_builder(iroh::RelayMode::Default)
                .discovery(PkarrResolver::n0_dns())
                .secret_key(key.clone())
                .bind()
                .await?;

            let store = mem::MemStore::new();

            let mut meta_conn = endpoint.connect(server_node_id, META_ALPN).await?;
            let (mut send, mut recv) = meta_conn.open_bi().await?;
            let req = postcard::to_allocvec(&MetaRequest::Get { filename })?;
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
            store.remote().fetch(blobs_conn.clone(), hash).await?;
            blobs_conn.close(0u32.into(), b"done");

            store.blobs().export(hash, path).await?;
            endpoint.close().await;
            store.shutdown().await?;
            Ok(())
        }

        _ => {
            eprintln!(
                "usage:\n  <bin> serve\n  <bin> put <NODE_ID> <FILE>\n  <bin> get <NODE_ID> <FILE>"
            );
            std::process::exit(1);
        }
    }
}
