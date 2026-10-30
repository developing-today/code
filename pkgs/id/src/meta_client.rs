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
    endpoint::{Connection, Endpoint, ReadToEndError, RecvStream},
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

/// Map a capped response-read failure to an error worth showing a user.
///
/// `ReadToEndError::TooLong` displays as a bare "stream too long", which
/// sends diagnosis toward the transport; name the actual cause and the
/// limit instead. Any other read failure passes through unchanged.
pub fn capped_response_error(err: ReadToEndError) -> anyhow::Error {
    match err {
        ReadToEndError::TooLong => {
            anyhow::anyhow!("response too large: exceeds the {MAX_RESPONSE_BYTES}-byte limit")
        }
        other => other.into(),
    }
}

/// Read one response stream to its end, bounded by [`MAX_RESPONSE_BYTES`].
///
/// Hand-rolled request paths (remote find in the CLI and the REPL) use this
/// instead of a per-call-site literal bound, so every response read shares
/// the one named cap and cannot drift back to a smaller size: the find
/// handler returns every match unbounded, so a response past ~64 KiB is
/// ordinary for a large store (#1617).
pub async fn read_capped_response(recv: &mut RecvStream) -> Result<Vec<u8>> {
    recv.read_to_end(MAX_RESPONSE_BYTES)
        .await
        .map_err(capped_response_error)
}

/// Send one request over an open connection.
pub async fn request_over(conn: &Connection, req: &MetaRequest) -> Result<MetaResponse> {
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(&postcard::to_allocvec(req)?).await?;
    send.finish()?;
    let buf = read_capped_response(&mut recv).await?;
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
            let conn = crate::connectivity::connect(&endpoint, addr, META_ALPN).await?;
            Ok(Self::Remote { endpoint, conn })
        } else {
            Ok(Self::Local(Box::new(LocalNode::open(false).await?)))
        }
    }

    /// Connect to a specific peer by node ID.
    pub async fn connect_node(node: EndpointId, no_relay: bool) -> Result<Self> {
        let key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
        let endpoint = crate::connectivity::public_builder(
            crate::connectivity::RelayPolicy::from_no_relay(no_relay),
        )
        .secret_key(key)
        .bind()
        .await?;
        let conn =
            crate::connectivity::connect(&endpoint, EndpointAddr::from(node), META_ALPN).await?;
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::MatchKind;
    use crate::protocol::FindMatch;
    use iroh::{RelayMode, TransportAddr, endpoint::presets};
    use iroh_blobs::Hash;
    use std::net::Ipv4Addr;

    const TEST_ALPN: &[u8] = b"id-capped-response-test";

    /// An endpoint on localhost, with no relay and no address lookup.
    async fn endpoint(alpns: Vec<Vec<u8>>) -> Result<Endpoint> {
        Ok(Endpoint::builder(presets::Minimal)
            .alpns(alpns)
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await?)
    }

    /// The address of `ep` on localhost: every bound socket, with an
    /// unspecified IP replaced by the loopback address.
    fn loopback_addr(ep: &Endpoint) -> EndpointAddr {
        EndpointAddr::from_parts(
            ep.id(),
            ep.bound_sockets().iter().map(|addr| {
                let ip = if addr.ip().is_unspecified() {
                    Ipv4Addr::LOCALHOST.into()
                } else {
                    addr.ip()
                };
                TransportAddr::Ip(std::net::SocketAddr::new(ip, addr.port()))
            }),
        )
    }

    /// Serve `payload` as the response on the first stream `server` accepts.
    fn serve_once(server: Endpoint, payload: Vec<u8>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let incoming = server.accept().await.expect("incoming");
            let conn = incoming.await.expect("connecting");
            let (mut send, mut recv) = conn.accept_bi().await.expect("accept_bi");
            let _req = recv.read_to_end(1024).await.expect("read request");
            send.write_all(&payload).await.expect("write payload");
            send.finish().expect("finish");
            conn.closed().await;
        })
    }

    /// Send one request to `server` and read the response with
    /// [`read_capped_response`], exactly as the remote-find paths do.
    async fn round_trip(server: &Endpoint, client: &Endpoint) -> Result<Vec<u8>> {
        let conn = client.connect(loopback_addr(server), TEST_ALPN).await?;
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(b"find").await?;
        send.finish()?;
        let buf = read_capped_response(&mut recv).await;
        conn.close(0u32.into(), b"done");
        buf
    }

    /// #1617: a find response past the old 64 KiB per-site bound must read
    /// through the shared cap intact — 2,000 matches at ~45 wire bytes each
    /// is ~90 KiB, comfortably past the old bound and under the shared one.
    #[tokio::test(flavor = "multi_thread")]
    async fn find_response_larger_than_64_kib_reads_through() {
        let matches: Vec<FindMatch> = (0..2_000)
            .map(|i| FindMatch {
                hash: Hash::from_bytes([7u8; 32]),
                name: format!("found-{i:04}.txt"),
                kind: MatchKind::Contains,
                is_hash_match: false,
            })
            .collect();
        let payload = postcard::to_allocvec(&MetaResponse::Find { matches }).unwrap();
        assert!(
            payload.len() > 64 * 1024,
            "payload {} bytes must exceed the old 64 KiB bound",
            payload.len()
        );

        let server = endpoint(vec![TEST_ALPN.to_vec()]).await.unwrap();
        let client = endpoint(vec![]).await.unwrap();
        let served = serve_once(server.clone(), payload);
        let buf = round_trip(&server, &client).await.unwrap();
        served.await.unwrap();
        client.close().await;
        server.close().await;

        let resp: MetaResponse = postcard::from_bytes(&buf).unwrap();
        match resp {
            MetaResponse::Find { matches } => assert_eq!(matches.len(), 2_000),
            _ => panic!("unexpected response"),
        }
    }

    /// A response past the shared cap must fail by name — the limit and its
    /// size — not as noq's bare "stream too long".
    #[test]
    fn over_cap_error_names_the_limit() {
        let err = capped_response_error(ReadToEndError::TooLong);
        let msg = format!("{err}");
        assert_eq!(
            msg,
            format!("response too large: exceeds the {MAX_RESPONSE_BYTES}-byte limit"),
            "the message must be exactly the named limit, with no leftover placeholder text"
        );
    }
}
