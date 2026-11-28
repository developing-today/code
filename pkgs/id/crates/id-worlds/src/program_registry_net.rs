//! The program registry over iroh (#8281; design of record
//! `doc/2026-11-10T21-24-00Z_design_id_program_ecosystem/`, decision D6).
//!
//! [`RegistryProtocol`] serves a [`Gallery`] (its registry, with the
//! operator's admission applied) on [`REGISTRY_ALPN`], and the client
//! functions here ([`list`], [`fetch`], [`publish`], [`unpublish`]) read and
//! write any registry that speaks it: a self-hosted one, a big hosted one, or
//! a test. A client trusts no registry: every listed record is checked with
//! [`BundleRecord::verify`], and a fetched bundle is decoded with
//! [`ProgramBundle::decode`] (the sideload verification path) and must be
//! the content that was asked for.
//!
//! One request per bidirectional stream. The client writes a JSON request
//! frame, plus the bundle bytes for a publish; the server answers with a
//! JSON response frame, then one frame per record for a list, or the bundle
//! bytes for a fetch.
//!
//! Publishing over the network is the operator's choice
//! ([`RegistryProtocol::accepting_publish`]); reading and author-signed
//! unpublish are always served.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::Context as _;
use iroh::{
    EndpointAddr,
    endpoint::{Connection, RecvStream, SendStream},
    protocol::{AcceptError, ProtocolHandler},
};
use serde::{Deserialize, Serialize};
use tokio::task::JoinSet;

use crate::frames::{read_frame, write_frame};
use crate::program_bundle::{BundleRecord, MAX_MANIFEST_BYTES, ProgramBundle};
use crate::program_gallery::Gallery;
use crate::program_gallery_admission::TransparencyEntry;
use crate::program_registry::ListQuery;

/// ALPN for the program registry protocol.
pub const REGISTRY_ALPN: &[u8] = b"/id-program-registry/1";

/// Largest bundle file sent either way: the world module limit plus room
/// for the manifest, the seal and the framing.
pub const MAX_BUNDLE_FILE_BYTES: usize =
    id_caps::world_limits::MAX_WORLD_MODULE_BYTES + MAX_MANIFEST_BYTES + 8 * 1024;

/// Most records a client accepts from one list.
pub const MAX_LIST_RECORDS: usize = 100_000;

/// A gallery shared between the serving protocols and whatever administers
/// it.
pub type SharedGallery = Arc<RwLock<Gallery>>;

/// Bound on concurrent request streams from one connection.
const MAX_STREAMS_PER_CONNECTION: usize = 8;

/// Bound on waiting for a request frame or a response header.
const FRAME_TIMEOUT: Duration = Duration::from_secs(30);

/// Bound on moving one bundle body.
const BODY_TIMEOUT: Duration = Duration::from_secs(120);

/// One request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum RegistryRequest {
    /// The listed records, optionally narrowed by author and name.
    List {
        /// Only this author's records (lower-case hex key).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        author: Option<String>,
        /// Only records with this name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// The bundle file with this content hash.
    Fetch {
        /// The bundle's content hash.
        content_hash: String,
    },
    /// Publish the bundle file of `length` bytes that follows this frame.
    Publish {
        /// The bundle file's length in bytes.
        length: u64,
    },
    /// Withdraw a bundle on its author's signed request.
    Unpublish {
        /// The bundle's content hash.
        content_hash: String,
        /// The author's signature (see
        /// [`crate::program_registry::sign_unpublish`]).
        signature: String,
    },
}

/// One response header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum RegistryResponse {
    /// `count` record frames follow.
    Records {
        /// How many record frames follow.
        count: u64,
    },
    /// The bundle file of `length` bytes follows.
    Bundle {
        /// The bundle file's length in bytes.
        length: u64,
    },
    /// The bundle is published; `listed` is false while it waits for the
    /// operator's review.
    Published {
        /// The published record.
        record: BundleRecord,
        /// Whether this registry lists it now.
        listed: bool,
    },
    /// The bundle is withdrawn.
    Unpublished {
        /// The withdrawn record.
        record: BundleRecord,
    },
    /// This registry holds the bundle but stopped listing it, publicly.
    Unlisted {
        /// The removal or refusal, as it stands on the transparency list.
        unlisted_by: TransparencyEntry,
    },
    /// The request was refused.
    Error {
        /// A stable code (the registry's or the admission's).
        code: String,
        /// The reason, for a person.
        message: String,
    },
}

/// Why a registry request did not succeed.
#[derive(Debug, thiserror::Error)]
pub enum RegistryNetError {
    /// The registry refused the request.
    #[error("the registry refused: {code}: {message}")]
    Refused {
        /// The registry's stable code.
        code: String,
        /// The registry's reason.
        message: String,
    },
    /// The registry holds the bundle but stopped listing it.
    #[error("the registry stopped listing {}: {:?}", .0.record.content_hash, .0.reason)]
    Unlisted(Box<TransparencyEntry>),
    /// The registry's answer does not verify or is not what was asked for.
    #[error("the registry's answer does not verify: {0}")]
    Invalid(String),
    /// The connection or stream failed.
    #[error("registry transport: {0:#}")]
    Transport(anyhow::Error),
}

impl From<anyhow::Error> for RegistryNetError {
    fn from(error: anyhow::Error) -> Self {
        Self::Transport(error)
    }
}

/// Serves a gallery's registry operations on [`REGISTRY_ALPN`].
#[derive(Clone, Debug)]
pub struct RegistryProtocol {
    gallery: SharedGallery,
    accept_publish: bool,
}

impl RegistryProtocol {
    /// Serve `gallery` read-only: list, fetch and author-signed unpublish.
    #[must_use]
    pub const fn new(gallery: SharedGallery) -> Self {
        Self {
            gallery,
            accept_publish: false,
        }
    }

    /// Also accept publishes from any peer. Each one is verified and then
    /// goes through the operator's admission like any other.
    #[must_use]
    pub const fn accepting_publish(mut self) -> Self {
        self.accept_publish = true;
        self
    }
}

impl ProtocolHandler for RegistryProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let mut streams = JoinSet::new();
        while let Ok((mut send, mut recv)) = conn.accept_bi().await {
            while streams.try_join_next().is_some() {}
            if streams.len() >= MAX_STREAMS_PER_CONNECTION {
                let _ = send.reset(1_u32.into());
                let _ = recv.stop(1_u32.into());
                continue;
            }
            let protocol = self.clone();
            streams.spawn(async move {
                if let Err(error) = protocol.serve_stream(send, recv).await {
                    tracing::debug!("registry stream ended: {error:#}");
                }
            });
        }
        streams.shutdown().await;
        Ok(())
    }
}

impl RegistryProtocol {
    async fn serve_stream(&self, mut send: SendStream, mut recv: RecvStream) -> anyhow::Result<()> {
        let request = tokio::time::timeout(FRAME_TIMEOUT, read_frame(&mut recv))
            .await
            .context("timed out waiting for a registry request")??
            .context("registry stream closed before a request")?;
        let request: RegistryRequest =
            serde_json::from_slice(&request).context("decode registry request")?;
        match request {
            RegistryRequest::List { author, name } => {
                let _ = recv.stop(0_u32.into());
                let records = self.read()?.list(&ListQuery { author, name });
                respond(
                    &mut send,
                    &RegistryResponse::Records {
                        count: records.len() as u64,
                    },
                )
                .await?;
                for record in &records {
                    write_frame(&mut send, &serde_json::to_vec(record)?).await?;
                }
            }
            RegistryRequest::Fetch { content_hash } => {
                let _ = recv.stop(0_u32.into());
                let (bytes, unlisted_by) = {
                    let gallery = self.read()?;
                    (
                        gallery.fetch(&content_hash),
                        gallery.unlisted_by(&content_hash).cloned(),
                    )
                };
                match (bytes, unlisted_by) {
                    (Some(bytes), _) => {
                        respond(
                            &mut send,
                            &RegistryResponse::Bundle {
                                length: bytes.len() as u64,
                            },
                        )
                        .await?;
                        tokio::time::timeout(BODY_TIMEOUT, send.write_all(&bytes))
                            .await
                            .context("timed out sending a bundle")??;
                    }
                    (None, Some(unlisted_by)) => {
                        respond(&mut send, &RegistryResponse::Unlisted { unlisted_by }).await?;
                    }
                    (None, None) => {
                        respond(&mut send, &refused("not_found", "no such bundle is listed"))
                            .await?;
                    }
                }
            }
            RegistryRequest::Publish { length } => {
                let response = if !self.accept_publish {
                    let _ = recv.stop(0_u32.into());
                    refused("publish_closed", "this registry does not accept publishes")
                } else if usize::try_from(length).map_or(true, |n| n > MAX_BUNDLE_FILE_BYTES) {
                    let _ = recv.stop(0_u32.into());
                    refused(
                        "too_large",
                        &format!("a bundle file is at most {MAX_BUNDLE_FILE_BYTES} bytes"),
                    )
                } else {
                    let mut bytes = vec![0_u8; usize::try_from(length).unwrap_or_default()];
                    tokio::time::timeout(BODY_TIMEOUT, recv.read_exact(&mut bytes))
                        .await
                        .context("timed out receiving a bundle")??;
                    let mut gallery = self.write()?;
                    match gallery.publish(&bytes) {
                        Ok(record) => {
                            let listed = record.verify().is_ok_and(|manifest| {
                                gallery.admission().lists(&manifest.content_hash)
                            });
                            RegistryResponse::Published { record, listed }
                        }
                        Err(error) => refused(error.code(), &error.to_string()),
                    }
                };
                respond(&mut send, &response).await?;
            }
            RegistryRequest::Unpublish {
                content_hash,
                signature,
            } => {
                let _ = recv.stop(0_u32.into());
                let outcome = self.write()?.unpublish(&content_hash, &signature);
                let response = match outcome {
                    Ok(record) => RegistryResponse::Unpublished { record },
                    Err(error) => refused(error.code(), &error.to_string()),
                };
                respond(&mut send, &response).await?;
            }
        }
        send.finish()?;
        Ok(())
    }

    fn read(&self) -> anyhow::Result<std::sync::RwLockReadGuard<'_, Gallery>> {
        self.gallery
            .read()
            .map_err(|error| anyhow::anyhow!("the gallery lock is poisoned: {error}"))
    }

    fn write(&self) -> anyhow::Result<std::sync::RwLockWriteGuard<'_, Gallery>> {
        self.gallery
            .write()
            .map_err(|error| anyhow::anyhow!("the gallery lock is poisoned: {error}"))
    }
}

fn refused(code: &str, message: &str) -> RegistryResponse {
    RegistryResponse::Error {
        code: code.to_owned(),
        message: message.to_owned(),
    }
}

async fn respond(send: &mut SendStream, response: &RegistryResponse) -> anyhow::Result<()> {
    write_frame(send, &serde_json::to_vec(response)?).await
}

/// Open one request stream to the registry at `addr` and read its response
/// header.
async fn request(
    endpoint: &iroh::Endpoint,
    addr: EndpointAddr,
    request: &RegistryRequest,
    body: Option<&[u8]>,
) -> Result<(RegistryResponse, RecvStream), RegistryNetError> {
    let connection = endpoint
        .connect(addr, REGISTRY_ALPN)
        .await
        .context("connect to the registry")?;
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .context("open a registry stream")?;
    write_frame(
        &mut send,
        &serde_json::to_vec(request).context("encode request")?,
    )
    .await?;
    if let Some(body) = body {
        tokio::time::timeout(BODY_TIMEOUT, send.write_all(body))
            .await
            .context("timed out sending a bundle")?
            .context("send a bundle")?;
    }
    send.finish().context("finish the request")?;
    let header = tokio::time::timeout(FRAME_TIMEOUT, read_frame(&mut recv))
        .await
        .context("timed out waiting for the registry")??
        .context("the registry closed the stream without answering")?;
    let response: RegistryResponse = serde_json::from_slice(&header)
        .map_err(|e| RegistryNetError::Invalid(format!("response header: {e}")))?;
    match response {
        RegistryResponse::Error { code, message } => {
            Err(RegistryNetError::Refused { code, message })
        }
        RegistryResponse::Unlisted { unlisted_by } => {
            Err(RegistryNetError::Unlisted(Box::new(unlisted_by)))
        }
        response => Ok((response, recv)),
    }
}

fn unexpected(response: &RegistryResponse) -> RegistryNetError {
    RegistryNetError::Invalid(format!("unexpected answer {response:?}"))
}

/// The records the registry at `addr` lists for `query`, each verified.
///
/// # Errors
///
/// The registry refuses or cannot be reached, or sends a record that does
/// not verify or does not match the query.
pub async fn list(
    endpoint: &iroh::Endpoint,
    addr: EndpointAddr,
    query: &ListQuery,
) -> Result<Vec<BundleRecord>, RegistryNetError> {
    let (response, mut recv) = request(
        endpoint,
        addr,
        &RegistryRequest::List {
            author: query.author.clone(),
            name: query.name.clone(),
        },
        None,
    )
    .await?;
    let RegistryResponse::Records { count } = response else {
        return Err(unexpected(&response));
    };
    let count = usize::try_from(count)
        .ok()
        .filter(|&count| count <= MAX_LIST_RECORDS)
        .ok_or_else(|| RegistryNetError::Invalid(format!("{count} records is too many")))?;
    let mut records = Vec::with_capacity(count.min(1024));
    for _ in 0..count {
        let frame = tokio::time::timeout(FRAME_TIMEOUT, read_frame(&mut recv))
            .await
            .context("timed out reading a record")??
            .ok_or_else(|| RegistryNetError::Invalid("the list ended early".to_owned()))?;
        let record: BundleRecord = serde_json::from_slice(&frame)
            .map_err(|e| RegistryNetError::Invalid(format!("record: {e}")))?;
        let manifest = record
            .verify()
            .map_err(|e| RegistryNetError::Invalid(e.to_string()))?;
        if query.author.as_ref().is_some_and(|a| *a != manifest.author)
            || query.name.as_ref().is_some_and(|n| *n != manifest.name)
        {
            return Err(RegistryNetError::Invalid(format!(
                "{} does not match the query",
                manifest.content_hash
            )));
        }
        records.push(record);
    }
    Ok(records)
}

/// Fetch and verify the bundle with `content_hash` from the registry at
/// `addr`.
///
/// The bytes are decoded exactly as a sideloaded file is, and must be the
/// content asked for, so they can go straight to an install.
///
/// # Errors
///
/// The registry refuses, stopped listing it, or cannot be reached, or the
/// bundle does not verify or is other content.
pub async fn fetch(
    endpoint: &iroh::Endpoint,
    addr: EndpointAddr,
    content_hash: &str,
) -> Result<ProgramBundle, RegistryNetError> {
    let (response, mut recv) = request(
        endpoint,
        addr,
        &RegistryRequest::Fetch {
            content_hash: content_hash.to_owned(),
        },
        None,
    )
    .await?;
    let RegistryResponse::Bundle { length } = response else {
        return Err(unexpected(&response));
    };
    let length = usize::try_from(length)
        .ok()
        .filter(|&length| length <= MAX_BUNDLE_FILE_BYTES)
        .ok_or_else(|| RegistryNetError::Invalid(format!("a {length}-byte bundle is too large")))?;
    let mut bytes = vec![0_u8; length];
    tokio::time::timeout(BODY_TIMEOUT, recv.read_exact(&mut bytes))
        .await
        .context("timed out receiving the bundle")?
        .context("receive the bundle")?;
    let bundle =
        ProgramBundle::decode(&bytes).map_err(|e| RegistryNetError::Invalid(e.to_string()))?;
    if bundle.manifest().content_hash != content_hash {
        return Err(RegistryNetError::Invalid(format!(
            "asked for {content_hash}, got {}",
            bundle.manifest().content_hash
        )));
    }
    Ok(bundle)
}

/// Publish a bundle file to the registry at `addr`. Returns the record and
/// whether the registry lists it now (false while it waits for review).
///
/// # Errors
///
/// The file is too large, or the registry refuses or cannot be reached.
pub async fn publish(
    endpoint: &iroh::Endpoint,
    addr: EndpointAddr,
    bundle: &[u8],
) -> Result<(BundleRecord, bool), RegistryNetError> {
    if bundle.len() > MAX_BUNDLE_FILE_BYTES {
        return Err(RegistryNetError::Invalid(format!(
            "a bundle file is at most {MAX_BUNDLE_FILE_BYTES} bytes"
        )));
    }
    let (response, _) = request(
        endpoint,
        addr,
        &RegistryRequest::Publish {
            length: bundle.len() as u64,
        },
        Some(bundle),
    )
    .await?;
    match response {
        RegistryResponse::Published { record, listed } => Ok((record, listed)),
        response => Err(unexpected(&response)),
    }
}

/// Withdraw a bundle from the registry at `addr` on its author's signed
/// request.
///
/// # Errors
///
/// The registry refuses or cannot be reached.
pub async fn unpublish(
    endpoint: &iroh::Endpoint,
    addr: EndpointAddr,
    content_hash: &str,
    signature: &str,
) -> Result<BundleRecord, RegistryNetError> {
    let (response, _) = request(
        endpoint,
        addr,
        &RegistryRequest::Unpublish {
            content_hash: content_hash.to_owned(),
            signature: signature.to_owned(),
        },
        None,
    )
    .await?;
    match response {
        RegistryResponse::Unpublished { record } => Ok(record),
        response => Err(unexpected(&response)),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use ed25519_dalek::SigningKey;
    use iroh::{Endpoint, RelayMode, TransportAddr, endpoint::presets, protocol::Router};

    use super::*;
    use crate::program_bundle::{ProgramSpec, export};
    use crate::program_gallery_admission::ReasonClass;
    use crate::program_registry::{AdmitAll, sign_unpublish};

    async fn endpoint() -> Endpoint {
        Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap()
    }

    fn endpoint_addr(endpoint: &Endpoint) -> EndpointAddr {
        EndpointAddr::from_parts(
            endpoint.id(),
            endpoint.bound_sockets().iter().map(|addr| {
                let ip = if addr.ip().is_unspecified() {
                    std::net::Ipv4Addr::LOCALHOST.into()
                } else {
                    addr.ip()
                };
                TransportAddr::Ip(std::net::SocketAddr::new(ip, addr.port()))
            }),
        )
    }

    fn bundle(name: &str, author: &SigningKey) -> ProgramBundle {
        export(
            format!("{name} payload").into_bytes(),
            ProgramSpec {
                name,
                version: 1,
                licence: None,
                description: Some("A test program."),
                categories: &["test"],
            },
            author,
        )
        .unwrap()
    }

    async fn serve(
        protocol: impl FnOnce(SharedGallery) -> RegistryProtocol,
    ) -> (Router, EndpointAddr, SharedGallery) {
        let dir = tempfile::tempdir().unwrap().keep();
        let gallery: SharedGallery =
            Arc::new(RwLock::new(Gallery::open_dir(dir, AdmitAll).unwrap()));
        let server = endpoint().await;
        let addr = endpoint_addr(&server);
        let router = Router::builder(server)
            .accept(REGISTRY_ALPN, protocol(Arc::clone(&gallery)))
            .spawn();
        (router, addr, gallery)
    }

    #[tokio::test]
    async fn a_second_host_lists_fetches_and_verifies_what_was_published() {
        let author = SigningKey::from_bytes(&[5; 32]);
        let (_router, addr, gallery) =
            serve(|g| RegistryProtocol::new(g).accepting_publish()).await;
        let client = endpoint().await;
        let chess = bundle("chess", &author);

        let (record, listed) = publish(&client, addr.clone(), &chess.to_bytes())
            .await
            .unwrap();
        assert_eq!(record, chess.record());
        assert!(listed);
        assert_eq!(gallery.read().unwrap().registry().len(), 1);

        let other = endpoint().await;
        let records = list(&other, addr.clone(), &ListQuery::default())
            .await
            .unwrap();
        assert_eq!(records, vec![chess.record()]);
        let none = list(
            &other,
            addr.clone(),
            &ListQuery {
                author: None,
                name: Some("go".to_owned()),
            },
        )
        .await
        .unwrap();
        assert!(none.is_empty());

        let hash = chess.manifest().content_hash.clone();
        let fetched = fetch(&other, addr.clone(), &hash).await.unwrap();
        assert_eq!(fetched.to_bytes(), chess.to_bytes());
        assert!(chess.record().describes(&fetched));

        match fetch(&other, addr.clone(), &"0".repeat(64)).await {
            Err(RegistryNetError::Refused { code, .. }) => assert_eq!(code, "not_found"),
            other => panic!("expected not_found, got {other:?}"),
        }

        let record = unpublish(&other, addr.clone(), &hash, &sign_unpublish(&hash, &author))
            .await
            .unwrap();
        assert_eq!(record, chess.record());
        assert!(
            list(&other, addr.clone(), &ListQuery::default())
                .await
                .unwrap()
                .is_empty()
        );
        match publish(&client, addr, &chess.to_bytes()).await {
            Err(RegistryNetError::Refused { code, .. }) => assert_eq!(code, "withdrawn"),
            other => panic!("a withdrawn bundle stays withdrawn here, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn publishing_is_the_operators_choice_and_removals_say_why() {
        let author = SigningKey::from_bytes(&[6; 32]);
        let (_router, addr, gallery) = serve(RegistryProtocol::new).await;
        let client = endpoint().await;
        let chess = bundle("chess", &author);
        let hash = chess.manifest().content_hash.clone();

        match publish(&client, addr.clone(), &chess.to_bytes()).await {
            Err(RegistryNetError::Refused { code, .. }) => assert_eq!(code, "publish_closed"),
            other => panic!("expected publish_closed, got {other:?}"),
        }
        {
            let mut gallery = gallery.write().unwrap();
            gallery.publish(&chess.to_bytes()).unwrap();
            gallery
                .remove(&hash, ReasonClass::Malware, 1, None)
                .unwrap();
        }
        match fetch(&client, addr.clone(), &hash).await {
            Err(RegistryNetError::Unlisted(entry)) => {
                assert_eq!(entry.record, chess.identity());
                assert_eq!(entry.reason, ReasonClass::Malware);
            }
            other => panic!("expected the removal, got {other:?}"),
        }
        assert!(
            list(&client, addr, &ListQuery::default())
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn requests_and_responses_have_a_stable_wire_form() {
        assert_eq!(
            serde_json::to_string(&RegistryRequest::Fetch {
                content_hash: "ab".to_owned()
            })
            .unwrap(),
            r#"{"op":"fetch","content_hash":"ab"}"#
        );
        assert_eq!(
            serde_json::to_string(&RegistryRequest::List {
                author: None,
                name: None
            })
            .unwrap(),
            r#"{"op":"list"}"#
        );
        assert!(serde_json::from_str::<RegistryRequest>(r#"{"op":"list","extra":1}"#).is_err());
        assert_eq!(
            serde_json::to_string(&RegistryResponse::Bundle { length: 3 }).unwrap(),
            r#"{"status":"bundle","length":3}"#
        );
    }
}
