//! The `fleet id` initiator byte streaming, build slice 4 of design #10391
//! (pass 2, §5 slice 4; issue #15436): byte streams over the slice-3
//! postcard-native initiator transport (§4.3).

//! Design §4.3 contract implemented here:
//!
//! - **One logical exchange per bidirectional QUIC stream:** the
//!   caller sends request bytes, finishes the send side, then reads
//!   the response bytes. Fleet multiplexes concurrent exchanges over
//!   one connection — each [`exchange`] call owns its stream, so N
//!   concurrent calls prove the multiplexing.
//! - **stdin/stdout/stderr-style mapping:** [`StreamRole`] names the
//!   three caller-facing byte flows (request in, response out,
//!   diagnostics out), and [`pipe_exchange`] wires them to caller
//!   streams the way the `fleet id stream` verb will wire stdio:
//!   request bytes in, response bytes out, diagnostics never mixed
//!   into the response.
//! - **Per-stream caps, configurable per call:** the default is the
//!   slice-3 64 KiB bound ([`DEFAULT_STREAM_CAP`]); callers moving
//!   larger fixtures pass [`StreamCaps::with_max`]. Over-cap streams
//!   fail closed with [`StreamError::TooLarge`] — never truncated.
//! - **Backpressure is QUIC backpressure:** [`send_bytes`] writes
//!   straight to the QUIC stream and blocks on the connection's flow
//!   control when the peer reads slowly. Fleet keeps no second
//!   buffer and no second flow-control layer; the slow-consumer test
//!   proves the sender blocks with bounded peer consumption.
//! - **Clean cancel:** [`cancel_send`] / [`cancel_recv`] reset/stop a
//!   stream with [`STREAM_CANCEL_CODE`]; the peer observes
//!   [`StreamError::PeerReset`] promptly and the connection stays
//!   usable for later exchanges.
//!
//! The live dial rides on the slice-2/3 connection setup (relay-aware
//! dial, `/iroh-meta/3` ALPN); this slice is the stream layer plus
//! its tests, independently revertible.

/// The ALPN byte streams ride on: the same postcard-native initiator
/// transport slice 3 framed ([`crate::id_postcard::POSTCARD_ALPN`]).
/// Streaming changes the stream contents, not the connection.
pub const STREAM_ALPN: &str = crate::id_postcard::POSTCARD_ALPN;

/// Default per-stream read cap: 64 KiB, inherited from the meta
/// protocol's `read_to_end` bound (design §4.3). Same bound slice 3
/// enforces on value frames; byte streams raise it per call via
/// [`StreamCaps::with_max`].
pub const DEFAULT_STREAM_CAP: usize = crate::id_postcard::PER_STREAM_CAP;

/// Read/write quantum for stream copies: small enough that a large
/// transfer never needs a large contiguous fleet-side buffer, large
/// enough to keep QUIC fed. This is a copy granularity, not a flow
/// control layer — backpressure stays QUIC's (design §4.3).
pub const IO_CHUNK: usize = 32 * 1024;

/// Application error code a fleet cancel puts on the wire
/// ([`cancel_send`] / [`cancel_recv`]). Any `u32` below 2^62 is a
/// legal QUIC application code; the value is arbitrary and stable so
/// both sides (and tests) can match on it.
pub const STREAM_CANCEL_CODE: u32 = 0xCA4C_E1;

/// Which caller-facing byte flow a stream carries (design §4.3:
/// stdin feeds request streams, stdout/stderr carry response bytes
/// and diagnostics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamRole {
    /// Request bytes in (stdin or a `--in` file/socket).
    Request,
    /// Response bytes out (stdout). Never mixed with diagnostics.
    Response,
    /// Diagnostics out (stderr). Never mixed into the response.
    Diagnostic,
}

impl StreamRole {
    /// The stdio name this role maps to.
    pub fn label(self) -> &'static str {
        match self {
            StreamRole::Request => "stdin",
            StreamRole::Response => "stdout",
            StreamRole::Diagnostic => "stderr",
        }
    }

    /// True for the request (input) direction.
    pub fn is_input(self) -> bool {
        matches!(self, StreamRole::Request)
    }

    /// True for the response/diagnostic (output) directions.
    pub fn is_output(self) -> bool {
        !self.is_input()
    }
}

/// Per-call caps for one byte-stream exchange. The default is the
/// §4.3 64 KiB bound; fixture transfers configure larger maxima
/// explicitly so the raise is always a caller decision, never a
/// silent growth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamCaps {
    /// Maximum bytes accepted on one stream direction. Both the
    /// request read (for [`pipe_exchange`]) and the response read
    /// (for [`exchange`]) enforce this; exceeding it fails the
    /// exchange with [`StreamError::TooLarge`].
    pub max_stream_bytes: usize,
}

impl Default for StreamCaps {
    fn default() -> Self {
        StreamCaps {
            max_stream_bytes: DEFAULT_STREAM_CAP,
        }
    }
}

impl StreamCaps {
    /// Caps allowing up to `max_stream_bytes` per direction.
    pub fn with_max(max_stream_bytes: usize) -> Self {
        StreamCaps { max_stream_bytes }
    }
}

/// Why a byte-stream exchange failed. Every variant is a hard error:
/// silent truncation or best-effort delivery is never attempted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamError {
    /// The stream carried more than the configured cap. `seen` is
    /// the byte count at which the cap tripped (always `> cap`).
    TooLarge { seen: usize, cap: usize },
    /// The peer reset or stopped the stream mid-exchange — the
    /// clean-cancel observation ([`cancel_send`]/[`cancel_recv`]
    /// from either side). Carries the wire application code.
    PeerReset { code: u64 },
    /// Any other transport failure (connection lost, closed
    /// stream, local I/O on the piped caller streams).
    Transport(String),
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StreamError::TooLarge { seen, cap } => write!(
                f,
                "stream exceeds cap: saw {seen} bytes, per-call maximum is {cap} (raise StreamCaps explicitly, never silently)"
            ),
            StreamError::PeerReset { code } => write!(
                f,
                "stream reset by peer (application code {code:#x}); exchange cancelled, connection still usable"
            ),
            StreamError::Transport(msg) => write!(f, "stream transport failed: {msg}"),
        }
    }
}

impl std::error::Error for StreamError {}

/// Enforce a byte cap on a running count. Pure, so the rule is
/// unit-testable without a network: any count above `cap` fails
/// closed instead of truncating.
pub fn ensure_within_cap(seen: usize, cap: usize) -> Result<(), StreamError> {
    if seen > cap {
        return Err(StreamError::TooLarge { seen, cap });
    }
    Ok(())
}

fn map_read_error(error: iroh::endpoint::ReadError) -> StreamError {
    match error {
        iroh::endpoint::ReadError::Reset(code) => StreamError::PeerReset {
            code: code.into_inner(),
        },
        other => StreamError::Transport(format!("{other}")),
    }
}

/// Write `bytes` to the stream and finish it (end-of-request). The
/// writes go straight to the QUIC stream: when the peer reads
/// slowly this call blocks on QUIC flow control instead of
/// buffering — that blocking is the §4.3 backpressure mechanism,
/// and fleet adds no layer of its own.
pub async fn send_bytes(
    send: &mut iroh::endpoint::SendStream,
    bytes: &[u8],
) -> Result<(), StreamError> {
    for chunk in bytes.chunks(IO_CHUNK) {
        send.write_all(chunk)
            .await
            .map_err(|e| StreamError::Transport(format!("request write: {e}")))?;
    }
    send.finish()
        .map_err(|e| StreamError::Transport(format!("request finish: {e:?}")))?;
    Ok(())
}

/// Read one direction to end, enforcing `cap` on the running total.
/// Allocation stays cap-bounded ([`IO_CHUNK`] quanta); the first
/// byte past the cap fails the read with [`StreamError::TooLarge`].
pub async fn recv_bytes(
    recv: &mut iroh::endpoint::RecvStream,
    cap: usize,
) -> Result<Vec<u8>, StreamError> {
    let mut out = Vec::new();
    loop {
        match recv.read_chunk(IO_CHUNK).await {
            Ok(Some(chunk)) => {
                out.extend_from_slice(&chunk);
                ensure_within_cap(out.len(), cap)?;
            }
            Ok(None) => return Ok(out),
            Err(error) => return Err(map_read_error(error)),
        }
    }
}

/// One logical request/response exchange over one bidirectional
/// stream (design §4.3): open a stream, send the request, finish,
/// read the response to the caps limit. Each call owns its stream,
/// so concurrent calls over one connection are the multiplexing —
/// no cross-stream state is shared here.
pub async fn exchange(
    conn: &iroh::endpoint::Connection,
    request: &[u8],
    caps: &StreamCaps,
) -> Result<Vec<u8>, StreamError> {
    let (mut send, mut recv) = conn
        .open_bi()
        .await
        .map_err(|e| StreamError::Transport(format!("open stream: {e:?}")))?;
    send_bytes(&mut send, request).await?;
    recv_bytes(&mut recv, caps.max_stream_bytes).await
}

/// Cancel our send side with [`STREAM_CANCEL_CODE`]. Idempotent: a
/// stream that already finished or closed needs no cancel, so the
/// result is dropped and the call always succeeds.
pub fn cancel_send(send: &mut iroh::endpoint::SendStream) {
    let _ = send.reset(STREAM_CANCEL_CODE.into());
}

/// Cancel our receive side with [`STREAM_CANCEL_CODE`]. Idempotent
/// like [`cancel_send`]: the result is dropped.
pub fn cancel_recv(recv: &mut iroh::endpoint::RecvStream) {
    let _ = recv.stop(STREAM_CANCEL_CODE.into());
}

/// Outcome of one [`pipe_exchange`]: the byte counts each way, for
/// the diagnostic line and caller accounting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipeStats {
    /// Request bytes moved from the input to the stream.
    pub sent: u64,
    /// Response bytes moved from the stream to the output.
    pub received: u64,
}

/// The stdin/stdout/stderr-style mapping as a function: read the
/// whole request from `input` ([`StreamRole::Request`], stdin-style,
/// cap-enforced), run one [`exchange`], write the response to
/// `output` ([`StreamRole::Response`], stdout-style), and write a
/// one-line summary to `diag` ([`StreamRole::Diagnostic`],
/// stderr-style — never into the response).
pub async fn pipe_exchange<R, W, D>(
    conn: &iroh::endpoint::Connection,
    input: R,
    mut output: W,
    mut diag: D,
    caps: &StreamCaps,
) -> Result<PipeStats, StreamError>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
    D: tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let mut request = Vec::new();
    input
        .take(caps.max_stream_bytes as u64 + 1)
        .read_to_end(&mut request)
        .await
        .map_err(|e| StreamError::Transport(format!("stdin read: {e}")))?;
    ensure_within_cap(request.len(), caps.max_stream_bytes)?;
    let response = exchange(conn, &request, caps).await?;
    output
        .write_all(&response)
        .await
        .map_err(|e| StreamError::Transport(format!("stdout write: {e}")))?;
    output
        .flush()
        .await
        .map_err(|e| StreamError::Transport(format!("stdout flush: {e}")))?;
    let stats = PipeStats {
        sent: request.len() as u64,
        received: response.len() as u64,
    };
    let line = format!(
        "id-stream: role={} sent={} received={}\n",
        StreamRole::Request.label(),
        stats.sent,
        stats.received
    );
    diag.write_all(line.as_bytes())
        .await
        .map_err(|e| StreamError::Transport(format!("stderr write: {e}")))?;
    Ok(stats)
}

/// SHA-256 hex digest of `bytes`: the integrity check the
/// sha-identical acceptance criteria compare. Both directions of an
/// exchange hash equal digests if and only if the bytes survived
/// identical.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    let digest = sha2::Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    /// Deterministic byte stream (splitmix64) — the fixture's
    /// oracle is byte identity (compared as SHA-256 hex), not the
    /// generator.
    fn fixture_bytes(n: usize, seed: u64) -> Vec<u8> {
        let mut state = seed;
        (0..n)
            .map(|_| {
                state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
                let mut z = state;
                z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                (z ^ (z >> 31)) as u8
            })
            .collect()
    }

    #[test]
    fn roles_map_to_stdio_names() {
        assert_eq!(StreamRole::Request.label(), "stdin");
        assert_eq!(StreamRole::Response.label(), "stdout");
        assert_eq!(StreamRole::Diagnostic.label(), "stderr");
        assert!(StreamRole::Request.is_input());
        assert!(!StreamRole::Request.is_output());
        assert!(StreamRole::Response.is_output());
        assert!(StreamRole::Diagnostic.is_output());
        assert!(!StreamRole::Response.is_input());
    }

    #[test]
    fn default_caps_match_the_postcard_per_stream_bound() {
        assert_eq!(DEFAULT_STREAM_CAP, crate::id_postcard::PER_STREAM_CAP);
        assert_eq!(DEFAULT_STREAM_CAP, 64 * 1024);
        assert_eq!(
            StreamCaps::default().max_stream_bytes,
            DEFAULT_STREAM_CAP
        );
        assert_eq!(
            StreamCaps::with_max(16 << 20).max_stream_bytes,
            16 << 20
        );
    }

    #[test]
    fn cap_rule_fails_closed_above_the_cap_only() {
        assert_eq!(ensure_within_cap(0, 64), Ok(()));
        assert_eq!(ensure_within_cap(64, 64), Ok(()));
        assert_eq!(
            ensure_within_cap(65, 64),
            Err(StreamError::TooLarge { seen: 65, cap: 64 })
        );
    }

    #[test]
    fn stream_alpn_is_the_postcard_initiator_alpn() {
        assert_eq!(STREAM_ALPN, crate::id_postcard::POSTCARD_ALPN);
        assert_eq!(STREAM_ALPN, "/iroh-meta/3");
    }

    #[test]
    fn sha256_hex_is_stable_and_sensitive() {
        let a = fixture_bytes(1024, 7);
        let mut b = a.clone();
        assert_eq!(sha256_hex(&a), sha256_hex(&a));
        assert_eq!(sha256_hex(&a).len(), 64);
        b[512] ^= 0x01;
        assert_ne!(sha256_hex(&a), sha256_hex(&b));
    }

    // --- Live transport: relay-fixture round-trips. Acceptance for
    // #15436: 4 MiB and 10 MiB fixtures transfer sha-identical in
    // both directions through the relay fixture (the same
    // `run_relay_server` local relay the slice-2 mode-2 test dials
    // through, with the CONNECT-shim dial path untouched — this
    // slice rides the established relay connection with byte
    // streams). Both directions are checked independently: the
    // server hashes what it received (client -> server) and the
    // client hashes the echo (server -> client).

    /// Echo server over an established connection: `streams`
    /// request/response exchanges, recording the SHA-256 of each
    /// request received. Holds the connection open until the peer
    /// closes it (same shape as slice 3's `serve_probes`).
    fn serve_echo(
        conn: iroh::endpoint::Connection,
        streams: usize,
        received: Arc<std::sync::Mutex<Vec<String>>>,
    ) -> tokio::task::JoinHandle<Result<(), String>> {
        tokio::spawn(async move {
            for i in 0..streams {
                let (mut send, mut recv) = conn
                    .accept_bi()
                    .await
                    .map_err(|e| format!("accept_bi {i}: {e:?}"))?;
                // The server side is generous: the CAP UNDER TEST is
                // the caller's, enforced client-side. The server
                // just needs headroom above the largest fixture.
                let request = recv_bytes(&mut recv, 32 << 20)
                    .await
                    .map_err(|e| format!("read request {i}: {e}"))?;
                received
                    .lock()
                    .expect("received log")
                    .push(sha256_hex(&request));
                send_bytes(&mut send, &request)
                    .await
                    .map_err(|e| format!("echo response {i}: {e}"))?;
            }
            conn.closed().await;
            Ok(())
        })
    }

    async fn loopback_endpoint(alpns: Vec<Vec<u8>>) -> iroh::Endpoint {
        iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
            .alpns(alpns)
            .relay_mode(iroh::RelayMode::Disabled)
            .bind()
            .await
            .expect("bind loopback endpoint")
    }

    fn loopback_addr(ep: &iroh::Endpoint) -> iroh::EndpointAddr {
        iroh::EndpointAddr::from_parts(
            ep.id(),
            ep.bound_sockets().iter().map(|addr| {
                let ip = if addr.ip().is_unspecified() {
                    Ipv4Addr::LOCALHOST.into()
                } else {
                    addr.ip()
                };
                iroh::TransportAddr::Ip(std::net::SocketAddr::new(ip, addr.port()))
            }),
        )
    }

    struct RelayFixture {
        relay_url: iroh::RelayUrl,
        relay_map: iroh::RelayMap,
        // Opaque relay-server guard: `iroh::test_utils::run_relay_server`
        // returns `iroh_relay::server::Server`, which `iroh` does not
        // re-export, so we erase the type rather than add a direct
        // `iroh-relay` dependency for one test guard.
        _relay_server: Box<dyn std::any::Any + Send>,
    }

    async fn relay_fixture() -> RelayFixture {
        let (relay_map, relay_url, relay_server) =
            iroh::test_utils::run_relay_server()
                .await
                .expect("local relay");
        RelayFixture {
            relay_url,
            relay_map,
            _relay_server: Box::new(relay_server),
        }
    }

    /// Both endpoints pinned to the relay path (no direct UDP):
    /// proves the bytes crossed the relay, not a shortcut. The
    /// server offers [`STREAM_ALPN`]; the client offers it at
    /// connect time (same shape as the slice-3 loopback tests).
    async fn relay_endpoint(fixture: &RelayFixture, alpns: Vec<Vec<u8>>) -> iroh::Endpoint {
        iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
            .alpns(alpns)
            .relay_mode(iroh::RelayMode::Custom(fixture.relay_map.clone()))
            .clear_ip_transports()
            .ca_tls_config(iroh::tls::CaTlsConfig::insecure_skip_verify())
            .bind()
            .await
            .expect("bind relay endpoint")
    }

    async fn relay_pair() -> (RelayFixture, iroh::Endpoint, iroh::Endpoint) {
        let fixture = relay_fixture().await;
        let server = relay_endpoint(&fixture, vec![STREAM_ALPN.as_bytes().to_vec()]).await;
        let client = relay_endpoint(&fixture, vec![]).await;
        // Both sides must hold a live home-relay session before the
        // dial: an early dial can miss the server's registration and
        // burn the whole connect timeout (cf. id-core connectivity
        // `test_n0_endpoints_dial_by_id`: "an early lookup can miss it").
        tokio::join!(server.online(), client.online());
        eprintln!("relay_pair: both online after {:?}", t0.elapsed());
        (fixture, server, client)
    }

    async fn connect_relay(
        client: &iroh::Endpoint,
        server: &iroh::Endpoint,
        relay_url: &iroh::RelayUrl,
    ) -> iroh::endpoint::Connection {
        let addr = iroh::EndpointAddr::new(server.id()).with_relay_url(relay_url.clone());
        client
            .connect(addr, STREAM_ALPN.as_bytes())
            .await
            .expect("relay connect")
    }

    /// One fixture through the relay fixture, checked both
    /// directions: the server hash proves client -> server, the
    /// client hash proves server -> client.
    async fn relay_fixture_round_trip(size: usize, seed: u64) {
        let (fixture, server, client) = relay_pair().await;
        let received = Arc::new(std::sync::Mutex::new(Vec::new()));
        // Accept and dial concurrently: awaiting the server accept
        // before initiating the client connect deadlocks (each side
        // waits for the other). `join!` polls both together.
        let (incoming_opt, client_conn) = tokio::join!(
            server.accept(),
            connect_relay(&client, &server, &fixture.relay_url),
        );
        let conn = incoming_opt
            .expect("incoming connection")
            .await
            .expect("authenticated relay connection");
        let serving = serve_echo(conn, 1, received.clone());
        let caps = StreamCaps::with_max(16 << 20);
        let payload = fixture_bytes(size, seed);
        let echoed = exchange(&client_conn, &payload, &caps)
            .await
            .expect("fixture exchange");
        assert_eq!(
            sha256_hex(&echoed),
            sha256_hex(&payload),
            "server -> client direction must be sha-identical"
        );
        client_conn.close(0u32.into(), b"done");
        serving
            .await
            .expect("server task")
            .expect("echo server served the stream");
        let log = received.lock().expect("received log");
        assert_eq!(log.len(), 1, "server saw exactly one request");
        assert_eq!(
            log[0],
            sha256_hex(&payload),
            "client -> server direction must be sha-identical"
        );
        client.close().await;
        server.close().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stream_4mib_relay_sha_identical_both_directions() {
        tokio::time::timeout(Duration::from_secs(60), relay_fixture_round_trip(4 << 20, 0x15436))
            .await
            .expect("4 MiB relay transfer completes within timeout");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stream_10mib_relay_sha_identical_both_directions() {
        tokio::time::timeout(
            Duration::from_secs(120),
            relay_fixture_round_trip(10 << 20, 0x15437),
        )
        .await
        .expect("10 MiB relay transfer completes within timeout");
    }

    /// Concurrent exchanges over ONE relayed connection: distinct
    /// payloads must each echo back to the right caller — the
    /// multiplexing proof (§4.3: one logical exchange per stream,
    /// fleet multiplexes over the connection).
    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_exchanges_multiplex_over_one_relay_connection() {
        tokio::time::timeout(Duration::from_secs(120), async {
            const STREAMS: usize = 8;
            let (fixture, server, client) = relay_pair().await;
            let received = Arc::new(std::sync::Mutex::new(Vec::new()));
            // Accept and dial concurrently (see relay_fixture_round_trip).
            let (incoming_opt, client_conn) = tokio::join!(
                server.accept(),
                connect_relay(&client, &server, &fixture.relay_url),
            );
            let conn = incoming_opt
                .expect("incoming connection")
                .await
                .expect("authenticated relay connection");
            let serving = serve_echo(conn, STREAMS, received.clone());
            let caps = StreamCaps::with_max(16 << 20);
            let payloads: Vec<Vec<u8>> = (0..STREAMS)
                .map(|i| fixture_bytes(1 << 20, 0x15440 + i as u64))
                .collect();
            let mut tasks = Vec::new();
            for payload in &payloads {
                let conn = client_conn.clone();
                let caps = caps;
                let payload = payload.clone();
                tasks.push(tokio::spawn(async move {
                    exchange(&conn, &payload, &caps).await
                }));
            }
            for (i, task) in tasks.into_iter().enumerate() {
                let echoed = task
                    .await
                    .expect("exchange task")
                    .expect("exchange succeeds");
                assert_eq!(
                    sha256_hex(&echoed),
                    sha256_hex(&payloads[i]),
                    "stream {i} echoed its own payload"
                );
            }
            client_conn.close(0u32.into(), b"done");
            serving
                .await
                .expect("server task")
                .expect("echo server served all streams");
            let log = received.lock().expect("received log");
            assert_eq!(log.len(), STREAMS, "server saw all {STREAMS} requests");
            let mut got: Vec<String> = log.clone();
            got.sort();
            let mut want: Vec<String> =
                payloads.iter().map(|p| sha256_hex(p)).collect();
            want.sort();
            assert_eq!(got, want, "every payload arrived exactly once");
            client.close().await;
            server.close().await;
        })
        .await
        .expect("multiplexed relay exchanges complete within timeout");
    }

    /// The default 64 KiB cap fails a larger stream closed with
    /// [`StreamError::TooLarge`] — caps are enforced, never
    /// silently grown or truncated.
    #[tokio::test(flavor = "multi_thread")]
    async fn default_cap_rejects_over_cap_stream() {
        tokio::time::timeout(Duration::from_secs(30), async {
            let server = loopback_endpoint(vec![STREAM_ALPN.as_bytes().to_vec()]).await;
            let client = loopback_endpoint(vec![]).await;
            let addr = loopback_addr(&server);
            let serving = tokio::spawn({
                let server = server.clone();
                async move {
                    let incoming = server.accept().await.expect("incoming");
                    let conn = incoming.await.expect("connecting");
                    let (mut send, mut recv) = conn.accept_bi().await.expect("accept_bi");
                    let big = fixture_bytes(65 * 1024, 0x15438);
                    send_bytes(&mut send, &big).await.expect("serve 65 KiB");
                    let _ = recv.read_chunk(1024).await;
                    conn.closed().await;
                }
            });
            let conn = client
                .connect(addr, STREAM_ALPN.as_bytes())
                .await
                .expect("connect");
            let (mut send, mut recv) = conn.open_bi().await.expect("open_bi");
            send.write_all(b"ping").await.expect("ping");
            send.finish().expect("finish ping");
            let err = recv_bytes(&mut recv, StreamCaps::default().max_stream_bytes)
                .await
                .expect_err("65 KiB must exceed the 64 KiB default cap");
            // The sender streams in chunks, so the exact over-cap
            // byte count depends on chunk boundaries: assert the
            // shape (cap bound + first-chunk overshoot), not a byte.
            match err {
                StreamError::TooLarge { seen, cap } => {
                    assert_eq!(cap, 64 * 1024, "default cap is the 64 KiB bound");
                    assert!(
                        seen > cap && seen <= 65 * 1024,
                        "seen {seen} is past the cap but within the 65 KiB stream"
                    );
                }
                other => panic!("expected TooLarge, got {other:?}"),
            }
            conn.close(0u32.into(), b"done");
            serving.await.expect("server task");
            client.close().await;
            server.close().await;
        })
        .await
        .expect("over-cap rejection completes within timeout");
    }

    /// Slow consumer: the server drains 4 KiB per 25 ms while the
    /// caller pushes 2 MiB. The send half must still be blocked two
    /// seconds in with under 512 KiB consumed — QUIC backpressure,
    /// not a fleet-side buffer (the send path holds no copy beyond
    /// the caller's slice and QUIC's own window).
    #[tokio::test(flavor = "multi_thread")]
    async fn slow_consumer_applies_backpressure_with_bounded_memory() {
        tokio::time::timeout(Duration::from_secs(90), async {
            const PAYLOAD: usize = 2 << 20;
            const CONSUMED_BOUND: usize = 512 << 10;
            let server = loopback_endpoint(vec![STREAM_ALPN.as_bytes().to_vec()]).await;
            let client = loopback_endpoint(vec![]).await;
            let addr = loopback_addr(&server);
            let consumed = Arc::new(AtomicUsize::new(0));
            let serving = tokio::spawn({
                let server = server.clone();
                let consumed = consumed.clone();
                async move {
                    let incoming = server.accept().await.expect("incoming");
                    let conn = incoming.await.expect("connecting");
                    let (mut send, mut recv) = conn.accept_bi().await.expect("accept_bi");
                    // Slow drain: small chunks, sleeps between them.
                    // ~160 KiB/s, so 2 MiB takes ~13 s end to end.
                    let mut request = Vec::new();
                    loop {
                        match recv.read_chunk(4 * 1024).await.expect("slow read") {
                            Some(chunk) => {
                                request.extend_from_slice(&chunk);
                                consumed.fetch_add(chunk.len(), Ordering::SeqCst);
                                tokio::time::sleep(Duration::from_millis(25)).await;
                            }
                            None => break,
                        }
                    }
                    send_bytes(&mut send, &request).await.expect("echo back");
                    conn.closed().await;
                }
            });
            let conn = client
                .connect(addr, STREAM_ALPN.as_bytes())
                .await
                .expect("connect");
            let (mut send, mut recv) = conn.open_bi().await.expect("open_bi");
            let payload = fixture_bytes(PAYLOAD, 0x15439);
            let payload_for_send = payload.clone();
            let send_task =
                tokio::spawn(async move { send_bytes(&mut send, &payload_for_send).await });
            // Two seconds in the send half must still be blocked —
            // and the server must have consumed only a fraction.
            tokio::time::sleep(Duration::from_secs(2)).await;
            assert!(
                !send_task.is_finished(),
                "sender must block on QUIC backpressure against a slow consumer"
            );
            let drained = consumed.load(Ordering::SeqCst);
            assert!(
                drained < CONSUMED_BOUND,
                "bounded memory: server consumed {drained} bytes, bound is {CONSUMED_BOUND}"
            );
            send_task
                .await
                .expect("send task")
                .expect("send completes once the consumer drains");
            // The finished send lets the server echo; the full
            // payload must still arrive sha-identical.
            let echoed = recv_bytes(&mut recv, 16 << 20)
                .await
                .expect("read echo");
            assert_eq!(
                sha256_hex(&echoed),
                sha256_hex(&payload),
                "payload survives backpressured transfer sha-identical"
            );
            conn.close(0u32.into(), b"done");
            serving.await.expect("server task");
            client.close().await;
            server.close().await;
        })
        .await
        .expect("backpressure transfer completes within timeout");
    }

    /// Cancel: the caller sends a 64 KiB prefix of a would-be large
    /// transfer, then cancels. The server must observe the reset
    /// promptly (not hang for the rest of the bytes), and a fresh
    /// exchange on the SAME connection must still succeed — clean
    /// teardown, connection survives.
    #[tokio::test(flavor = "multi_thread")]
    async fn cancel_tears_down_stream_cleanly_and_connection_survives() {
        tokio::time::timeout(Duration::from_secs(30), async {
            let server = loopback_endpoint(vec![STREAM_ALPN.as_bytes().to_vec()]).await;
            let client = loopback_endpoint(vec![]).await;
            let addr = loopback_addr(&server);
            let (outcome_tx, outcome_rx) = tokio::sync::oneshot::channel::<StreamError>();
            let serving = tokio::spawn({
                let server = server.clone();
                async move {
                    let incoming = server.accept().await.expect("incoming");
                    let conn = incoming.await.expect("connecting");
                    // Stream 1: the cancelled one.
                    let (_, mut recv) = conn.accept_bi().await.expect("accept_bi 1");
                    let outcome = match recv_bytes(&mut recv, 32 << 20).await {
                        Err(e) => e,
                        Ok(bytes) => StreamError::Transport(format!(
                            "expected a reset, stream ended cleanly with {} bytes",
                            bytes.len()
                        )),
                    };
                    let _ = outcome_tx.send(outcome);
                    // Stream 2: proves the connection survived.
                    let (mut send, mut recv) = conn.accept_bi().await.expect("accept_bi 2");
                    let request = recv_bytes(&mut recv, 32 << 20).await.expect("request 2");
                    send_bytes(&mut send, &request).await.expect("echo 2");
                    conn.closed().await;
                }
            });
            let conn = client
                .connect(addr, STREAM_ALPN.as_bytes())
                .await
                .expect("connect");
            let (mut send, _recv) = conn.open_bi().await.expect("open_bi 1");
            send.write_all(&fixture_bytes(64 * 1024, 0x15441))
                .await
                .expect("partial send");
            cancel_send(&mut send);
            drop(send);
            let outcome = tokio::time::timeout(Duration::from_secs(10), outcome_rx)
                .await
                .expect("server observes the cancel promptly")
                .expect("outcome sent");
            match outcome {
                StreamError::PeerReset { code } => assert_eq!(
                    code,
                    u64::from(STREAM_CANCEL_CODE),
                    "server sees fleet's cancel code"
                ),
                other => panic!("expected PeerReset, server saw: {other}"),
            }
            // Same connection, fresh stream: still fully functional.
            let caps = StreamCaps::default();
            let payload = fixture_bytes(1024, 0x15442);
            let echoed = exchange(&conn, &payload, &caps)
                .await
                .expect("exchange after cancel");
            assert_eq!(echoed, payload, "connection survives the cancel");
            conn.close(0u32.into(), b"done");
            serving.await.expect("server task");
            client.close().await;
            server.close().await;
        })
        .await
        .expect("cancel teardown completes within timeout");
    }

    /// The stdin/stdout/stderr mapping end to end over loopback:
    /// request bytes in, response bytes out, diagnostics on their
    /// own writer — never mixed into the response.
    #[tokio::test(flavor = "multi_thread")]
    async fn pipe_exchange_maps_stdio_roles() {
        tokio::time::timeout(Duration::from_secs(30), async {
            let server = loopback_endpoint(vec![STREAM_ALPN.as_bytes().to_vec()]).await;
            let client = loopback_endpoint(vec![]).await;
            let addr = loopback_addr(&server);
            let serving = tokio::spawn({
                let server = server.clone();
                async move {
                    let incoming = server.accept().await.expect("incoming");
                    let conn = incoming.await.expect("connecting");
                    let (mut send, mut recv) = conn.accept_bi().await.expect("accept_bi");
                    let request = recv_bytes(&mut recv, 32 << 20).await.expect("request");
                    let mut reversed = request.clone();
                    reversed.reverse();
                    send_bytes(&mut send, &reversed).await.expect("response");
                    conn.closed().await;
                }
            });
            let conn = client
                .connect(addr, STREAM_ALPN.as_bytes())
                .await
                .expect("connect");
            let caps = StreamCaps::default();
            let request = fixture_bytes(4096, 0x15443);
            let mut response = Vec::new();
            let mut diag = Vec::new();
            let stats = pipe_exchange(&conn, &request[..], &mut response, &mut diag, &caps)
                .await
                .expect("pipe exchange");
            assert_eq!(stats.sent, 4096);
            assert_eq!(stats.received, 4096);
            let mut want = request.clone();
            want.reverse();
            assert_eq!(response, want, "stdout carries the response bytes");
            let diag_line = String::from_utf8(diag).expect("diagnostic is text");
            assert!(
                diag_line.contains("sent=4096") && diag_line.contains("received=4096"),
                "stderr carries the summary, not response bytes: {diag_line:?}"
            );
            conn.close(0u32.into(), b"done");
            serving.await.expect("server task");
            client.close().await;
            server.close().await;
        })
        .await
        .expect("piped exchange completes within timeout");
    }
}
