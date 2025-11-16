use anyhow::{bail, Context, Result};
use iroh::endpoint::{Connection, Endpoint, EndpointId, RecvStream, SecretKey, SendStream};
use iroh_blobs::{
    api::blobs::Blobs,
    store::{fs, mem},
    BlobFormat, Hash, ALPN as BLOBS_ALPN,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::{fs as afs, io::AsyncWriteExt, sync::Mutex};
use tracing::{error, info};

const KEY_FILE: &str = ".iroh-key";
const BLOB_DIR: &str = ".iroh-blobs";
const MANIFEST_FILE: &str = ".iroh-manifest";
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

async fn load_or_create_keypair(path: &str) -> Result<SecretKey> {
    match afs::read(path).await {
        Ok(bytes) => {
            let bytes: [u8; 32] =
                bytes.try_into().map_err(|_| anyhow::anyhow!("invalid key length"))?;
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

async fn load_manifest(path: &str) -> Result<HashMap<String, Hash>> {
    match afs::read(path).await {
        Ok(data) => postcard::from_bytes(&data).context("decode manifest"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(e) => Err(e.into()),
    }
}

async fn save_manifest(path: &str, manifest: &HashMap<String, Hash>) -> Result<()> {
    let data = postcard::to_allocvec(manifest)?;
    afs::write(path, data).await?;
    Ok(())
}

async fn read_postcard(recv: &mut RecvStream, limit: usize) -> Result<Vec<u8>> {
    let buf = recv.read_to_end(limit).await?;
    Ok(buf)
}

async fn write_all(send: &mut SendStream, bytes: &[u8]) -> Result<()> {
    send.write_all(bytes).await?;
    send.finish()?;
    Ok(())
}

async fn handle_meta(
    mut conn: Connection,
    manifest: Arc<Mutex<HashMap<String, Hash>>>,
    manifest_path: &'static str,
) -> Result<()> {
    let (mut send, mut recv) = conn.accept_bi().await?;
    let req_buf = read_postcard(&mut recv, 64 * 1024).await?;
    let req: MetaRequest = postcard::from_bytes(&req_buf)?;
    match req {
        MetaRequest::Put { filename, hash } => {
            let mut m = manifest.lock().await;
            m.insert(filename, hash);
            save_manifest(manifest_path, &m).await?;
            let resp = postcard::to_allocvec(&MetaResponse::Put { success: true })?;
            write_all(&mut send, &resp).await?;
        }
        MetaRequest::Get { filename } => {
            let m = manifest.lock().await;
            let hash = m.get(&filename).copied();
            let resp = postcard::to_allocvec(&MetaResponse::Get { hash })?;
            write_all(&mut send, &resp).await?;
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    let key = load_or_create_keypair(KEY_FILE).await?;

    match args.as_slice() {
        ["serve"] => {
            let endpoint = Endpoint::builder()
                .alpns(vec![META_ALPN.to_vec(), BLOBS_ALPN.to_vec()])
                .secret_key(key.clone())
                .bind()
                .await?;

            let my_id: EndpointId = endpoint.id();
            info!("serve: {}", my_id);

            let fs_store = fs::open(fs::Config {
                path: PathBuf::from(BLOB_DIR),
                create: true,
                ..Default::default()
            })
            .await?;
            let blobs = Blobs::new(fs_store);

            let manifest = Arc::new(Mutex::new(
                load_manifest(MANIFEST_FILE).await.context("load manifest")?,
            ));

            loop {
                match endpoint.accept().await {
                    None => break,
                    Some(incoming) => {
                        let alpn = incoming.protocol();
                        if alpn == META_ALPN {
                            let m = manifest.clone();
                            tokio::spawn(async move {
                                let accepting = incoming.accept();
                                match accepting.into_connection() {
                                    Ok(conn) => {
                                        if let Err(e) = handle_meta(conn, m, MANIFEST_FILE).await {
                                            error!("meta: {}", e);
                                        }
                                    }
                                    Err(e) => error!("meta accept: {}", e),
                                }
                            });
                        } else if alpn == BLOBS_ALPN {
                            let b = blobs.clone();
                            tokio::spawn(async move {
                                if let Err(e) = b.handle_incoming(incoming).await {
                                    error!("blobs: {}", e);
                                }
                            });
                        }
                    }
                }
            }
            Ok(())
        }

        ["put", server_id, path] => {
            let server_id: EndpointId = server_id.parse()?;
            let path = PathBuf::from(path);
            let filename = path
                .file_name()
                .context("invalid filename")?
                .to_string_lossy()
                .to_string();

            let endpoint = Endpoint::builder()
                .secret_key(key.clone())
                .alpns(vec![META_ALPN.to_vec(), BLOBS_ALPN.to_vec()])
                .bind()
                .await?;

            let mem_store = mem::create();
            let blobs = Blobs::new(mem_store);

            let data = afs::read(&path).await?;
            let hash = blobs.add_bytes(data, BlobFormat::Raw).await?;

            let mut conn = endpoint.connect(server_id, META_ALPN).await?;
            let (mut send, mut recv) = conn.open_bi().await?;
            let req_buf = postcard::to_allocvec(&MetaRequest::Put { filename, hash })?;
            write_all(&mut send, &req_buf).await?;
            let resp_buf = read_postcard(&mut recv, 64 * 1024).await?;
            let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
            match resp {
                MetaResponse::Put { success: true } => {
                    blobs.upload(hash, &[server_id]).await?;
                    Ok(())
                }
                MetaResponse::Put { success: false } => bail!("server rejected"),
                _ => bail!("unexpected response"),
            }
        }

        ["get", server_id, path] => {
            let server_id: EndpointId = server_id.parse()?;
            let path = PathBuf::from(path);
            let filename = path
                .file_name()
                .context("invalid filename")?
                .to_string_lossy()
                .to_string();

            let endpoint = Endpoint::builder()
                .secret_key(key.clone())
                .alpns(vec![META_ALPN.to_vec(), BLOBS_ALPN.to_vec()])
                .bind()
                .await?;

            let mem_store = mem::create();
            let blobs = Blobs::new(mem_store);

            let mut conn = endpoint.connect(server_id, META_ALPN).await?;
            let (mut send, mut recv) = conn.open_bi().await?;
            let req_buf = postcard::to_allocvec(&MetaRequest::Get { filename })?;
            write_all(&mut send, &req_buf).await?;
            let resp_buf = read_postcard(&mut recv, 64 * 1024).await?;
            let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
            let hash = match resp {
                MetaResponse::Get { hash: Some(h) } => h,
                MetaResponse::Get { hash: None } => bail!("file not found"),
                _ => bail!("unexpected response"),
            };

            blobs.download(hash, &[server_id]).await?;
            blobs.export(hash, path).await?;
            Ok(())
        }

        _ => {
            eprintln!("usage:\n  <bin> serve\n  <bin> put <ENDPOINT_ID> <FILE>\n  <bin> get <ENDPOINT_ID> <FILE>");
            std::process::exit(1);
        }
    }
}
