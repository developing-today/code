//! Client side of the meta protocol, with an in-process fallback.
//!
//! Commands that talk to the meta protocol (tags, and anything else expressed
//! as a [`MetaRequest`]) are written once against [`MetaRequester`]. Two
//! implementations exist:
//!
//! - [`MetaClient::Remote`]: a QUIC connection to a running `id serve`
//!   (local, found via the lock file) or to any peer by node ID.
//! - [`MetaClient::Local`]: no server is running, so the request is answered
//!   **in-process by the very same [`MetaProtocol::handle`]** over a
//!   [`LocalNode`]. Offline and online therefore behave identically — there is
//!   no separate "legacy" code path to drift out of sync.

use std::future::Future;

use anyhow::{Context, Result, bail};
use iroh::{
    EndpointAddr,
    endpoint::{Connection, Endpoint, RelayMode, presets},
};
use iroh_base::EndpointId;

use crate::commands::client::create_local_client_endpoint;
use crate::commands::serve::get_serve_info;
use crate::local::LocalNode;
use crate::protocol::{MetaRequest, MetaResponse};
use crate::{CLIENT_KEY_FILE, META_ALPN, load_or_create_keypair};

/// Maximum accepted size of a single response (64 MiB).
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

/// Something that can answer a [`MetaRequest`].
pub trait MetaRequester {
    /// Send `req` and return the node's response.
    ///
    /// A [`MetaResponse::Error`] is returned as `Ok`; use [`expect_ok`] to turn
    /// it into an `Err`.
    fn request(&mut self, req: MetaRequest) -> impl Future<Output = Result<MetaResponse>>;
}

/// Turn a [`MetaResponse::Error`] into an `Err`, passing other responses through.
pub fn expect_ok(resp: MetaResponse) -> Result<MetaResponse> {
    match resp {
        MetaResponse::Error { message } => bail!("{message}"),
        other => Ok(other),
    }
}

/// Send one request over an open connection.
pub async fn request_over(conn: &Connection, req: &MetaRequest) -> Result<MetaResponse> {
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(&postcard::to_allocvec(req)?).await?;
    send.finish()?;
    let buf = recv.read_to_end(MAX_RESPONSE_BYTES).await?;
    postcard::from_bytes(&buf).context("decoding meta response")
}

/// A meta-protocol client: remote over QUIC, or local in-process.
#[derive(Debug)]
pub enum MetaClient {
    /// Connected to a server (the local `id serve` or a remote peer).
    Remote {
        /// Endpoint the connection was made from (kept alive for the connection).
        endpoint: Endpoint,
        /// The meta-protocol connection.
        conn: Connection,
    },
    /// No server: operate directly on the local stores.
    Local(Box<LocalNode>),
}

impl MetaClient {
    /// Connect to the local `id serve` if one is running, else open the local
    /// stores directly.
    pub async fn open() -> Result<Self> {
        if let Some(info) = get_serve_info().await {
            let (endpoint, addr) = create_local_client_endpoint(&info).await?;
            let conn = endpoint.connect(addr, META_ALPN).await?;
            Ok(Self::Remote { endpoint, conn })
        } else {
            Ok(Self::Local(Box::new(LocalNode::open(false).await?)))
        }
    }

    /// Connect to a specific peer by node ID.
    pub async fn connect_node(node: EndpointId, no_relay: bool) -> Result<Self> {
        let key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
        let mut builder = Endpoint::builder(presets::N0).secret_key(key);
        if no_relay {
            builder = builder.relay_mode(RelayMode::Disabled);
        }
        let endpoint = builder.bind().await?;
        let conn = endpoint
            .connect(EndpointAddr::from(node), META_ALPN)
            .await?;
        Ok(Self::Remote { endpoint, conn })
    }

    /// Close the connection / shut down local stores.
    pub async fn close(self) -> Result<()> {
        match self {
            Self::Remote { endpoint, conn } => {
                conn.close(0u32.into(), b"done");
                endpoint.close().await;
                Ok(())
            }
            Self::Local(node) => node.shutdown().await,
        }
    }
}

impl MetaRequester for MetaClient {
    async fn request(&mut self, req: MetaRequest) -> Result<MetaResponse> {
        match self {
            Self::Remote { conn, .. } => request_over(conn, &req).await,
            Self::Local(node) => Ok(node.request(req).await),
        }
    }
}
