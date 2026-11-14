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
use std::time::Duration;

use anyhow::{Context, Result, bail};
use iroh::{
    EndpointAddr,
    endpoint::{Connection, Endpoint, ReadToEndError, RecvStream},
};
use iroh_base::EndpointId;

use iroh_blobs::{
    ALPN as BLOBS_ALPN, Hash,
    api::Store,
    protocol::{ChunkRanges, ChunkRangesSeq, PushRequest},
};

use crate::protocol::{META_ALPN, MetaRequest, MetaResponse};

/// Maximum accepted size of a single response (64 MiB).
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

/// Maximum time allowed for one remote meta request and its response.
pub const META_RESPONSE_TIMEOUT: Duration = Duration::from_secs(45);

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

/// Read one response stream to its end, bounded by [`MAX_RESPONSE_BYTES`]
/// and [`META_RESPONSE_TIMEOUT`].
///
/// Hand-rolled request paths (remote find in the CLI and the REPL) use this
/// instead of a per-call-site literal bound, so every response read shares
/// the one named cap and cannot drift back to a smaller size: the find
/// handler returns every match unbounded, so a response past ~64 KiB is
/// ordinary for a large store (#1617).
pub async fn read_capped_response(recv: &mut RecvStream) -> Result<Vec<u8>> {
    read_capped_response_with_timeout(recv, META_RESPONSE_TIMEOUT).await
}

async fn read_capped_response_with_timeout(
    recv: &mut RecvStream,
    timeout: Duration,
) -> Result<Vec<u8>> {
    id_core::connectivity::within(
        timeout,
        "reading a meta response",
        read_capped_response_unbounded(recv),
    )
    .await
}

async fn read_capped_response_unbounded(recv: &mut RecvStream) -> Result<Vec<u8>> {
    recv.read_to_end(MAX_RESPONSE_BYTES)
        .await
        .map_err(capped_response_error)
}

/// Send one request over an open connection.
pub async fn request_over(conn: &Connection, req: &MetaRequest) -> Result<MetaResponse> {
    request_over_with_timeout(conn, req, META_RESPONSE_TIMEOUT).await
}

async fn request_over_with_timeout(
    conn: &Connection,
    req: &MetaRequest,
    timeout: Duration,
) -> Result<MetaResponse> {
    id_core::connectivity::within(timeout, "waiting for a meta response", async {
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(&postcard::to_allocvec(req)?).await?;
        send.finish()?;
        let buf = read_capped_response_unbounded(&mut recv).await?;
        crate::protocol::decode_meta_exact(&buf).context("decoding meta response")
    })
    .await
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::protocol::{FindMatch, ListEntry, MatchKind};
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

    /// Accept and read one request, then leave the response stream open.
    fn serve_silently(server: Endpoint) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let incoming = server.accept().await.expect("incoming");
            let conn = incoming.await.expect("connecting");
            let (_send, mut recv) = conn.accept_bi().await.expect("accept_bi");
            let _req = recv.read_to_end(1024).await.expect("read request");
            tokio::time::sleep(Duration::from_secs(5)).await;
        })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn silent_peer_response_times_out() {
        let server = endpoint(vec![TEST_ALPN.to_vec()]).await.unwrap();
        let client = endpoint(vec![]).await.unwrap();
        let silent = serve_silently(server.clone());
        let conn = client
            .connect(loopback_addr(&server), TEST_ALPN)
            .await
            .unwrap();

        let err = request_over_with_timeout(&conn, &MetaRequest::List, Duration::from_millis(100))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("timed out after"));
        assert!(err.to_string().contains("waiting for a meta response"));

        conn.close(0u32.into(), b"done");
        silent.abort();
        client.close().await;
        server.close().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn silent_peer_capped_read_times_out() {
        let server = endpoint(vec![TEST_ALPN.to_vec()]).await.unwrap();
        let client = endpoint(vec![]).await.unwrap();
        let silent = serve_silently(server.clone());
        let conn = client
            .connect(loopback_addr(&server), TEST_ALPN)
            .await
            .unwrap();
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        send.write_all(&postcard::to_allocvec(&MetaRequest::List).unwrap())
            .await
            .unwrap();
        send.finish().unwrap();

        let err = read_capped_response_with_timeout(&mut recv, Duration::from_millis(100))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("timed out after"));
        assert!(err.to_string().contains("reading a meta response"));

        conn.close(0u32.into(), b"done");
        silent.abort();
        client.close().await;
        server.close().await;
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

    /// #1761: a List response past the hand-rolled 1 MiB bound the CLI,
    /// REPL, peers and serve-probe paths used to apply must read through
    /// the shared cap intact — 30,000 items at ~50 wire bytes each is
    /// ~1.5 MiB, past the old bound and under the shared one. This is
    /// the shape of store (15–25k items) that could not be listed at all.
    #[tokio::test(flavor = "multi_thread")]
    async fn list_response_larger_than_1_mib_reads_through() {
        let items: Vec<ListEntry> = (0..30_000)
            .map(|i| ListEntry {
                hash: Hash::from_bytes([9u8; 32]),
                name: format!("file-{i:05}.txt"),
                origin: None,
                uploaded_at: None,
            })
            .collect();
        let payload = postcard::to_allocvec(&MetaResponse::List { items }).unwrap();
        assert!(
            payload.len() > 1024 * 1024,
            "payload {} bytes must exceed the old 1 MiB bound",
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
            MetaResponse::List { items } => {
                assert_eq!(items.len(), 30_000);
                assert_eq!(items[29_999].name, "file-29999.txt");
            }
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

const PUT_FINALIZATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2 * 60);

/// Upload `hash` to `target`, **then** register `name` for it.
///
/// Two ordering rules make this safe:
///
/// 1. **Bytes before name.** The server verifies the blob is present before it
///    creates a name, so a failed upload can never leave a dangling name.
/// 2. **Keep the blobs connection open until the server confirms.** A push is
///    one-way — the server sends no acknowledgement — and closing the QUIC
///    connection right after the last write can abort the server's import of
///    data it has buffered but not yet processed. The `Put` request is the
///    acknowledgement: the server answers it once the blob is complete, so we
///    close the blobs connection only after that.
///
/// A `Put` for a blob that is still being finalized is retried for up to two
/// minutes (large blobs take a moment to finish on the server).
pub async fn push_and_name(
    store: &Store,
    endpoint: &Endpoint,
    target: EndpointAddr,
    name: &str,
    hash: Hash,
) -> Result<()> {
    let blobs_conn = id_core::connectivity::connect(endpoint, target.clone(), BLOBS_ALPN).await?;
    let push_request = PushRequest::new(hash, ChunkRangesSeq::from_ranges([ChunkRanges::all()]));
    store
        .remote()
        .execute_push(blobs_conn.clone(), push_request)
        .await
        .context(
            "blob push rejected: the server only accepts writes from allowed nodes \
             (run `id id --client` on this machine and add the printed ID to the \
             server's `--allow-node` or `.iroh-allowed`)",
        )?;

    let meta_conn = id_core::connectivity::connect(endpoint, target, META_ALPN).await?;
    let req = MetaRequest::Put {
        filename: name.to_owned(),
        hash,
    };
    let outcome = id_core::connectivity::within(
        PUT_FINALIZATION_TIMEOUT,
        "registering the blob name",
        async {
            loop {
                match request_over(&meta_conn, &req).await? {
                    MetaResponse::Error { message }
                        if crate::fileops::is_blob_missing_message(&message) =>
                    {
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    }
                    response => return Ok(response),
                }
            }
        },
    )
    .await;
    meta_conn.close(0u32.into(), b"done");
    blobs_conn.close(0u32.into(), b"done");
    match expect_ok(outcome?)? {
        MetaResponse::Put { success: true } => Ok(()),
        MetaResponse::Put { success: false } => bail!("server rejected"),
        _ => bail!("unexpected response"),
    }
}
