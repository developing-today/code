//! Client side of the meta protocol: the composition half.
//!
//! Step 3b of the pkgs/id crate split (#4406): the wire half of this
//! module — [`MetaRequester`], `request_over`, the capped response
//! read — lives in `id_sync::meta_client` and is re-exported here.
//! What stays in the facade is the composition: [`MetaClient`] chooses
//! between a running `id serve` (found through the CLI layer's serve
//! lock) and the in-process [`LocalNode`], both of which are facade
//! concerns until step 7 extracts the CLI layer.

use anyhow::Result;
use iroh::{
    EndpointAddr,
    endpoint::{Connection, Endpoint},
};
use iroh_base::EndpointId;

use crate::commands::client::create_local_client_endpoint;
use crate::commands::serve::get_serve_info;
use crate::local::LocalNode;
use crate::protocol::{MetaRequest, MetaResponse};
use crate::{CLIENT_KEY_FILE, META_ALPN, load_or_create_keypair};

pub use id_sync::meta_client::{
    MAX_RESPONSE_BYTES, META_RESPONSE_TIMEOUT, MetaRequester, capped_response_error, expect_ok,
    read_capped_response, request_over,
};

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
            let conn = id_core::connectivity::connect(&endpoint, addr, META_ALPN).await?;
            Ok(Self::Remote { endpoint, conn })
        } else {
            Ok(Self::Local(Box::new(LocalNode::open(false).await?)))
        }
    }

    /// Connect to a specific peer by node ID.
    pub async fn connect_node(node: EndpointId, no_relay: bool) -> Result<Self> {
        let key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
        let endpoint = id_core::connectivity::public_builder(
            id_core::connectivity::RelayPolicy::from_no_relay(no_relay),
        )
        .secret_key(key)
        .bind()
        .await?;
        let conn =
            id_core::connectivity::connect(&endpoint, EndpointAddr::from(node), META_ALPN).await?;
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

