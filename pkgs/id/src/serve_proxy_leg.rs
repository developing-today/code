//! Serve-proxy transport legs — slice 1 of issue #14918 (design #5792,
//! codec slice PR #7666, mux-credit slice PR #13859).
//!
//! The codec in [`super::serve_proxy`] defines the typed frames; the mux
//! in [`super::serve_proxy_mux`] owns per-leg credit accounting. Neither
//! wires a live transport. This module is the leg establishment both of
//! them wait on:
//!
//! - **CONNECT leg (primary):** a TCP connection to the broker — through
//!   an HTTP CONNECT tunnel when egress requires a proxy — upgraded to
//!   TLS 1.3. The TLS configuration accepts TLS 1.3 only; a peer that
//!   cannot speak it does not get a leg. Typed frames then travel as the
//!   codec's length-prefixed byte stream, unchanged.
//! - **WebSocket leg (fallback):** where CONNECT is denied, the same
//!   TLS 1.3 connection carries a WebSocket instead, and each typed frame
//!   travels as exactly one binary message (design §2). Text messages,
//!   fragmented multi-frame messages, and messages that do not contain
//!   exactly one complete frame are rejected at the boundary.
//!
//! Invariants carried over from #5792 and enforced here: the broker
//! forwards E2E ciphertext only and payload bytes are never logged
//! (nothing in this module logs); malformed or oversized input is
//! rejected as soon as it is visible, before it is buffered — the
//! CONNECT response header is read byte-at-a-time under a fixed cap so
//! no frame byte is ever consumed by handshake parsing, the byte-stream
//! leg decodes through the codec's bounded [`FrameDecoder`], and the
//! WebSocket leg caps message and frame size at the codec's maximum
//! wire frame. This slice deliberately does not implement mux stream
//! mapping/scheduling, keepalive, resumption, or E2E forwarding; those
//! are the later #14918 slices and compose on top of these legs.
//!
//! Testing oracle (repo testing doctrine): typed-frame byte preservation
//! — for each supported leg, a sequence of frames sent through the
//! established leg is received by the peer identical in type, stream ID,
//! sequence, and payload bytes, including a DATA frame at exactly the
//! 16 KiB cap; and boundary rejection — malformed or oversized input on
//! each leg is an error, never a delivered frame. The TLS leg's oracle
//! additionally pins the negotiated protocol version server-side to
//! TLS 1.3.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::{TlsConnector, client::TlsStream};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{WebSocketStream, client_async, tungstenite::client::IntoClientRequest};

use super::serve_proxy::{Frame, FrameDecoder, MAX_FRAME_BODY};

/// Largest wire representation of one frame (4-byte length prefix plus
/// the largest body the codec accepts). The WebSocket leg refuses
/// messages larger than this, mirroring the byte-stream leg's bound.
pub const MAX_FRAME_WIRE_BYTES: usize = 4 + MAX_FRAME_BODY;

/// Cap on the CONNECT response header. A proxy response is a status line
/// plus a handful of headers; anything larger is not a CONNECT response
/// this leg will keep reading.
const MAX_CONNECT_RESPONSE_BYTES: usize = 8 * 1024;

/// Default deadline for each establishment step (TCP connect, CONNECT
/// tunnel, TLS handshake, WebSocket handshake).
pub const DEFAULT_ESTABLISH_TIMEOUT: Duration = Duration::from_secs(30);

/// The TLS 1.3 client configuration for a broker leg, trusting the
/// WebPKI root set — the same roots the crate's other rustls client
/// (the libsql HTTP adapter) uses. TLS 1.3 is the only protocol version
/// offered: the builder names it explicitly rather than inheriting
/// rustls defaults, so a future default change cannot silently widen
/// the leg to TLS 1.2.
pub fn tls13_client_config() -> Result<Arc<rustls::ClientConfig>> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    tls13_client_config_with_roots(roots)
}

/// [`tls13_client_config`] with a caller-supplied root store (private
/// deployments, and the tests' self-signed broker).
pub fn tls13_client_config_with_roots(
    roots: rustls::RootCertStore,
) -> Result<Arc<rustls::ClientConfig>> {
    let config = rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// A field that will be interpolated into a handshake (CONNECT authority,
/// server name) must not be able to smuggle extra headers or requests:
/// non-empty, no whitespace or control characters.
fn validate_handshake_field(field: &str, what: &str) -> Result<()> {
    if field.is_empty() || field.chars().any(|c| c.is_whitespace() || c.is_control()) {
        bail!("{what} {field:?} is not a valid handshake field");
    }
    Ok(())
}

/// Establish an HTTP CONNECT tunnel over an already-connected stream
/// (typically the TCP connection to the egress proxy): writes
/// `CONNECT authority`, and returns the stream positioned at the first
/// byte after the proxy's response header once the proxy answers 200.
/// Any other status — 407 included, surfaced with its code so the caller
/// can classify it as [`super::serve_proxy::ErrorReason::ProxyAuthRequired`]
/// if it builds an ERROR frame — a malformed status line, or a header
/// over [`MAX_CONNECT_RESPONSE_BYTES`] is an error and no leg exists.
///
/// The response is read one byte at a time on purpose: the broker may
/// send its first frame the moment the tunnel is up, and a buffered
/// reader could swallow those bytes into handshake parsing. One byte at
/// a time is slow and exactly correct for a header of at most a few
/// hundred bytes.
pub async fn connect_tunnel<S>(mut stream: S, authority: &str) -> Result<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    validate_handshake_field(authority, "CONNECT authority")?;
    let request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .context("writing CONNECT request to proxy")?;
    stream.flush().await.context("flushing CONNECT request")?;

    let mut response = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    while !response.ends_with(b"\r\n\r\n") {
        if response.len() >= MAX_CONNECT_RESPONSE_BYTES {
            bail!(
                "CONNECT response header exceeds {MAX_CONNECT_RESPONSE_BYTES} bytes; refusing to keep reading"
            );
        }
        let read = stream
            .read(&mut byte)
            .await
            .context("reading CONNECT response from proxy")?;
        if read == 0 {
            bail!("proxy closed the connection during the CONNECT response");
        }
        response.push(byte[0]);
    }

    let text = std::str::from_utf8(&response).context("CONNECT response is not valid UTF-8")?;
    let status_line = text.lines().next().context("CONNECT response is empty")?;
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().context("CONNECT status line has no version")?;
    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        bail!("CONNECT response has a malformed status line: {status_line:?}");
    }
    let status: u16 = parts
        .next()
        .context("CONNECT status line has no status code")?
        .parse()
        .context("CONNECT status line has a non-numeric status code")?;
    if status != 200 {
        bail!("proxy refused CONNECT with status {status}");
    }
    Ok(stream)
}

/// One established byte-stream leg: typed frames over any
/// `AsyncRead + AsyncWrite` stream — in production the TLS 1.3 stream
/// out of [`establish_connect_leg`], in tests a tunnelled or duplex
/// stream. Receiving decodes through the codec's bounded
/// [`FrameDecoder`], so an oversized length prefix is refused as soon
/// as its four bytes arrive, exactly as the codec documents.
#[derive(Debug)]
pub struct ByteStreamLeg<S> {
    stream: S,
    decoder: FrameDecoder,
    pending: VecDeque<Frame>,
    read_buf: [u8; 16 * 1024],
}

impl<S> ByteStreamLeg<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            decoder: FrameDecoder::new(),
            pending: VecDeque::new(),
            read_buf: [0u8; 16 * 1024],
        }
    }

    /// Encode and write one frame, then flush: a leg delivers frames,
    /// not byte fragments the peer must time out on.
    pub async fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        let wire = frame.encode().context("encoding frame for leg")?;
        self.stream
            .write_all(&wire)
            .await
            .context("writing frame to leg")?;
        self.stream.flush().await.context("flushing leg")?;
        Ok(())
    }

    /// Receive the next frame. `Ok(None)` means the peer closed the leg
    /// cleanly between frames; a close mid-frame is an error.
    pub async fn recv_frame(&mut self) -> Result<Option<Frame>> {
        loop {
            if let Some(frame) = self.pending.pop_front() {
                return Ok(Some(frame));
            }
            let read = self
                .stream
                .read(&mut self.read_buf)
                .await
                .context("reading frame bytes from leg")?;
            if read == 0 {
                if self.decoder.pending_bytes() > 0 {
                    bail!("leg closed mid-frame");
                }
                return Ok(None);
            }
            let chunk = self
                .read_buf
                .get(..read)
                .context("read returned more bytes than the buffer holds")?;
            let frames = self
                .decoder
                .feed(chunk)
                .context("decoding frames from leg")?;
            self.pending.extend(frames);
        }
    }

    /// The underlying stream (for shutdown or protocol inspection).
    pub fn get_mut(&mut self) -> &mut S {
        &mut self.stream
    }
}


/// One established WebSocket leg: each typed frame travels as exactly
/// one binary message (design §2). Sending encodes one frame into one
/// message; receiving requires a binary message holding exactly one
/// complete frame — a text message, a multi-frame message, or trailing
/// bytes after the frame are boundary errors, never delivered frames.
#[derive(Debug)]
pub struct WebSocketLeg<S> {
    socket: WebSocketStream<S>,
}

impl<S> WebSocketLeg<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Wrap a completed WebSocket handshake. Prefer the `establish_*`
    /// constructors; this is for callers (and tests) that performed the
    /// handshake themselves.
    pub fn new(socket: WebSocketStream<S>) -> Self {
        Self { socket }
    }

    pub async fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        let wire = frame.encode().context("encoding frame for WebSocket leg")?;
        self.socket
            .send(Message::Binary(wire.into()))
            .await
            .context("sending frame as WebSocket binary message")?;
        Ok(())
    }

    /// Receive the next frame. `Ok(None)` means the peer sent a Close
    /// message; ping/pong control messages are handled by the WebSocket
    /// layer (answered and skipped) — serve-proxy keepalive is a later
    /// slice and travels as typed PING frames, not WebSocket pings.
    pub async fn recv_frame(&mut self) -> Result<Option<Frame>> {
        loop {
            let Some(message) = self.socket.next().await else {
                return Ok(None);
            };
            let message = message.context("receiving WebSocket message")?;
            match message {
                Message::Binary(bytes) => {
                    if bytes.len() > MAX_FRAME_WIRE_BYTES {
                        bail!(
                            "WebSocket binary message of {} bytes exceeds the {MAX_FRAME_WIRE_BYTES}-byte leg bound",
                            bytes.len()
                        );
                    }
                    let (frame, consumed) = Frame::decode_prefix(&bytes)
                        .context("decoding frame from WebSocket message")?
                        .context("WebSocket binary message holds no complete frame")?;
                    if consumed != bytes.len() {
                        bail!(
                            "WebSocket binary message holds {} trailing bytes after one frame; one frame per message is the leg contract",
                            bytes.len() - consumed
                        );
                    }
                    return Ok(Some(frame));
                }
                Message::Close(_) => return Ok(None),
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
                Message::Text(_) => {
                    bail!("text message on a serve-proxy WebSocket leg; frames are binary only");
                }
            }
        }
    }
}

/// The broker endpoint a leg is established to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerEndpoint {
    /// Host name (or IP literal) of the broker; also the TLS server name
    /// unless `server_name` overrides it.
    pub host: String,
    pub port: u16,
    /// TLS server name when it differs from `host` (e.g. dialing an IP).
    pub server_name: Option<String>,
}

impl BrokerEndpoint {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            server_name: None,
        }
    }

    fn authority(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    fn tls_server_name(&self) -> Result<rustls::pki_types::ServerName<'static>> {
        let name = self.server_name.as_deref().unwrap_or(&self.host);
        validate_handshake_field(name, "TLS server name")?;
        rustls::pki_types::ServerName::try_from(name.to_owned())
            .context("broker TLS server name is invalid")
    }
}

async fn tcp_connect(host: &str, port: u16, what: &str) -> Result<TcpStream> {
    validate_handshake_field(host, "TCP host")?;
    tokio::time::timeout(
        DEFAULT_ESTABLISH_TIMEOUT,
        TcpStream::connect((host, port)),
    )
    .await
    .with_context(|| format!("timed out connecting to {what}"))?
    .with_context(|| format!("connecting to {what}"))
}

async fn tls_connect(
    config: Arc<rustls::ClientConfig>,
    endpoint: &BrokerEndpoint,
    tcp: TcpStream,
) -> Result<TlsStream<TcpStream>> {
    let connector = TlsConnector::from(config);
    let server_name = endpoint.tls_server_name()?;
    tokio::time::timeout(
        DEFAULT_ESTABLISH_TIMEOUT,
        connector.connect(server_name, tcp),
    )
    .await
    .context("timed out during the broker TLS 1.3 handshake")?
    .context("broker TLS 1.3 handshake failed")
}

/// Establish the primary leg: TCP to the broker — tunnelled through the
/// HTTP proxy at `proxy` (`"host:port"`) via CONNECT when one is given —
/// then TLS 1.3 to the broker, end to end through any tunnel. Frames
/// then travel as the codec byte stream.
pub async fn establish_connect_leg(
    endpoint: &BrokerEndpoint,
    proxy: Option<&str>,
    tls_config: Arc<rustls::ClientConfig>,
) -> Result<ByteStreamLeg<TlsStream<TcpStream>>> {
    let tcp = match proxy {
        Some(proxy) => {
            let (proxy_host, proxy_port) = proxy
                .rsplit_once(':')
                .context("proxy must be in host:port form")?;
            let proxy_port: u16 = proxy_port.parse().context("proxy port is not a number")?;
            let to_proxy = tcp_connect(proxy_host, proxy_port, "egress proxy").await?;
            tokio::time::timeout(
                DEFAULT_ESTABLISH_TIMEOUT,
                connect_tunnel(to_proxy, &endpoint.authority()),
            )
            .await
            .context("timed out establishing the CONNECT tunnel")?
            .context("establishing the CONNECT tunnel to the broker")?
        }
        None => tcp_connect(&endpoint.host, endpoint.port, "broker").await?,
    };
    let tls = tls_connect(tls_config, endpoint, tcp).await?;
    Ok(ByteStreamLeg::new(tls))
}

/// Establish the fallback leg over an already-connected stream: a
/// WebSocket client handshake for `url` (`ws://` / `wss://` — the TLS
/// layer, when wanted, is the stream's own, established by the caller
/// as in [`establish_websocket_leg`]), bounded to one maximum wire
/// frame per message.
pub async fn establish_websocket_leg_over<S>(
    stream: S,
    url: &str,
) -> Result<WebSocketLeg<S>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let request = url
        .into_client_request()
        .context("building the WebSocket handshake request")?;
    let (socket, _response) = tokio::time::timeout(
        DEFAULT_ESTABLISH_TIMEOUT,
        client_async(request, stream),
    )
    .await
    .context("timed out during the WebSocket handshake")?
    .context("WebSocket handshake failed")?;
    Ok(WebSocketLeg::new(socket))
}

/// Establish the fallback leg: TCP + TLS 1.3 to the broker (through a
/// CONNECT tunnel when `proxy` is given, exactly as the primary leg),
/// then a WebSocket handshake; frames travel one per binary message
/// where CONNECT alone was not enough to carry the leg.
pub async fn establish_websocket_leg(
    endpoint: &BrokerEndpoint,
    proxy: Option<&str>,
    tls_config: Arc<rustls::ClientConfig>,
) -> Result<WebSocketLeg<TlsStream<TcpStream>>> {
    let tcp = match proxy {
        Some(proxy) => {
            let (proxy_host, proxy_port) = proxy
                .rsplit_once(':')
                .context("proxy must be in host:port form")?;
            let proxy_port: u16 = proxy_port.parse().context("proxy port is not a number")?;
            let to_proxy = tcp_connect(proxy_host, proxy_port, "egress proxy").await?;
            tokio::time::timeout(
                DEFAULT_ESTABLISH_TIMEOUT,
                connect_tunnel(to_proxy, &endpoint.authority()),
            )
            .await
            .context("timed out establishing the CONNECT tunnel")?
            .context("establishing the CONNECT tunnel to the broker")?
        }
        None => tcp_connect(&endpoint.host, endpoint.port, "broker").await?,
    };
    let tls = tls_connect(tls_config, endpoint, tcp).await?;
    let server_name = endpoint
        .server_name
        .clone()
        .unwrap_or_else(|| endpoint.host.clone());
    let url = format!("wss://{server_name}/serve-proxy");
    establish_websocket_leg_over(tls, &url).await
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::serve_proxy::{FrameType, MAX_DATA_PAYLOAD};
    use futures_util::{SinkExt as _, StreamExt as _};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpListener;

    /// The frame sequence every leg oracle round-trips: one frame of
    /// each shape, including a DATA frame at exactly the 16 KiB cap.
    fn oracle_frames() -> Vec<Frame> {
        vec![
            Frame::new(FrameType::Open, 1, 0, b"broker.test:ssh".to_vec()).unwrap(),
            Frame::data(1, 0, vec![0xAB; MAX_DATA_PAYLOAD]).unwrap(),
            Frame::data(1, 1, b"ciphertext".to_vec()).unwrap(),
            Frame::new(FrameType::Ping, 0, 0, Vec::new()).unwrap(),
            Frame::new(FrameType::WindowUpdate, 1, 0, 4096u32.to_be_bytes().to_vec()).unwrap(),
            Frame::new(FrameType::Close, 1, 0, Vec::new()).unwrap(),
        ]
    }

    /// Oracle driver for byte-stream legs: `server` echoes every frame
    /// back over the accepted stream; the client sends the oracle
    /// sequence through its leg and asserts byte-for-byte identity of
    /// what returns. Identity of decoded frames plus the codec's own
    /// encode identity is byte preservation of the payloads.
    async fn assert_byte_stream_leg_round_trip<S>(client_stream: S, server: tokio::task::JoinHandle<()>)
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let mut leg = ByteStreamLeg::new(client_stream);
        let frames = oracle_frames();
        for frame in &frames {
            leg.send_frame(frame).await.unwrap();
        }
        for expected in &frames {
            let received = leg.recv_frame().await.unwrap().unwrap();
            assert_eq!(&received, expected);
        }
        server.abort();
    }

    fn spawn_byte_stream_echo(stream: TcpStream) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut leg = ByteStreamLeg::new(stream);
            while let Ok(Some(frame)) = leg.recv_frame().await {
                if leg.send_frame(&frame).await.is_err() {
                    break;
                }
            }
        })
    }

    #[tokio::test]
    async fn serve_proxy_leg_connect_tunnel_round_trips_typed_frames() {
        // Fake broker: accepts one TCP connection and echoes frames.
        let broker = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let broker_addr = broker.local_addr().unwrap();
        let echo = tokio::spawn(async move {
            let (stream, _) = broker.accept().await.unwrap();
            spawn_byte_stream_echo(stream).await.unwrap();
        });

        // Fake egress proxy: answers one CONNECT with 200, then splices
        // bytes between the client and the broker in both directions.
        let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let splice = tokio::spawn(async move {
            let (mut client_side, _) = proxy.accept().await.unwrap();
            // Read the CONNECT request header (byte-at-a-time shape is
            // the client's; here a buffered read is fine, the client
            // sends nothing else before the response).
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                client_side.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
            }
            let request_text = String::from_utf8(request).unwrap();
            assert!(
                request_text.starts_with(&format!("CONNECT {broker_addr} HTTP/1.1")),
                "unexpected CONNECT request: {request_text:?}"
            );
            client_side
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await
                .unwrap();
            let mut broker_side = TcpStream::connect(broker_addr).await.unwrap();
            tokio::io::copy_bidirectional(&mut client_side, &mut broker_side)
                .await
                .unwrap();
        });

        let to_proxy = TcpStream::connect(proxy_addr).await.unwrap();
        let tunnelled = connect_tunnel(to_proxy, &broker_addr.to_string())
            .await
            .unwrap();
        assert_byte_stream_leg_round_trip(tunnelled, echo).await;
        splice.abort();
    }

    #[tokio::test]
    async fn serve_proxy_leg_connect_rejects_non_200_and_malformed() {
        for (response, needle) in [
            ("HTTP/1.1 407 Proxy Authentication Required\r\n\r\n", "407"),
            ("HTTP/1.1 403 Forbidden\r\n\r\n", "403"),
            ("GARBAGE\r\n\r\n", "malformed"),
        ] {
            let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let proxy_addr = proxy.local_addr().unwrap();
            let responder = response.to_string();
            let server = tokio::spawn(async move {
                let (mut stream, _) = proxy.accept().await.unwrap();
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).await.unwrap();
                    request.push(byte[0]);
                }
                stream.write_all(responder.as_bytes()).await.unwrap();
            });
            let to_proxy = TcpStream::connect(proxy_addr).await.unwrap();
            let err = connect_tunnel(to_proxy, "broker.test:443")
                .await
                .unwrap_err();
            assert!(
                format!("{err:#}").contains(needle),
                "expected {needle:?} in error, got {err:#}"
            );
            server.await.unwrap();
        }

        // An authority that could smuggle a second request is refused
        // before anything is written.
        let (client, _server) = tokio::io::duplex(1024);
        let err = connect_tunnel(client, "broker.test:443\r\nInjected: x")
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("handshake field"));
    }

    #[tokio::test]
    async fn serve_proxy_leg_tls13_over_connect_round_trips_and_negotiates_tls13() {
        // Self-signed broker certificate for broker.test.
        let certified = rcgen::generate_simple_self_signed(vec!["broker.test".to_string()]).unwrap();
        let cert_der = certified.cert.der().clone();
        let key_der = rustls::pki_types::PrivateKeyDer::try_from(
            certified.key_pair.serialize_der(),
        )
        .unwrap();

        let server_config = rustls::ServerConfig::builder_with_protocol_versions(&[
            &rustls::version::TLS13,
        ])
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));

        // Broker side: plain TCP accept, consume the CONNECT request,
        // answer 200, then TLS-accept and echo frames; report the
        // negotiated protocol version back over a channel.
        let broker = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let broker_addr = broker.local_addr().unwrap();
        let (version_tx, version_rx) = tokio::sync::oneshot::channel();
        let broker_task = tokio::spawn(async move {
            let (mut tcp, _) = broker.accept().await.unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                tcp.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
            }
            tcp.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
            let tls = acceptor.accept(tcp).await.unwrap();
            let version = tls.get_ref().1.protocol_version();
            let _ = version_tx.send(version);
            spawn_byte_stream_echo_tls(tls).await.unwrap();
        });

        // Client: TCP to the "proxy" (the broker listener plays both
        // roles in sequence), CONNECT, then the production establishment
        // path's TLS configuration with the self-signed root.
        let mut roots = rustls::RootCertStore::empty();
        roots.add(cert_der).unwrap();
        let config = tls13_client_config_with_roots(roots).unwrap();
        let endpoint = BrokerEndpoint::new("broker.test", broker_addr.port());
        let tcp = TcpStream::connect(broker_addr).await.unwrap();
        let tunnelled = connect_tunnel(tcp, &endpoint.authority()).await.unwrap();
        let tls = tls_connect(config, &endpoint, tunnelled).await.unwrap();
        assert_byte_stream_leg_round_trip(tls, broker_task).await;

        let negotiated = version_rx.await.unwrap();
        assert_eq!(negotiated, Some(rustls::ProtocolVersion::TLSv1_3));
    }

    fn spawn_byte_stream_echo_tls(
        stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    ) -> tokio::task::JoinHandle<()> {
        // The server-side TlsStream implements AsyncRead/AsyncWrite too,
        // so the generic byte-stream leg echoes over it unchanged.
        tokio::spawn(async move {
            let mut leg = ByteStreamLeg::new(stream);
            while let Ok(Some(frame)) = leg.recv_frame().await {
                if leg.send_frame(&frame).await.is_err() {
                    break;
                }
            }
        })
    }

    #[tokio::test]
    async fn serve_proxy_leg_websocket_round_trips_typed_frames() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let mut leg = WebSocketLeg::new(socket);
            while let Ok(Some(frame)) = leg.recv_frame().await {
                leg.send_frame(&frame).await.unwrap();
            }
        });

        let tcp = TcpStream::connect(addr).await.unwrap();
        let mut leg = establish_websocket_leg_over(tcp, &format!("ws://{addr}/serve-proxy"))
            .await
            .unwrap();
        let frames = oracle_frames();
        for frame in &frames {
            leg.send_frame(frame).await.unwrap();
        }
        for expected in &frames {
            let received = leg.recv_frame().await.unwrap().unwrap();
            assert_eq!(&received, expected);
        }
        server.abort();
    }

    #[tokio::test]
    async fn serve_proxy_leg_websocket_rejects_text_and_oversized() {
        // Text message: a hard boundary error, not a frame.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            socket.send(Message::Text("not a frame".into())).await.unwrap();
            // Keep the socket open so the client reads the text first.
            tokio::time::sleep(Duration::from_millis(200)).await;
        });
        let tcp = TcpStream::connect(addr).await.unwrap();
        let mut leg = establish_websocket_leg_over(tcp, &format!("ws://{addr}/serve-proxy"))
            .await
            .unwrap();
        let err = leg.recv_frame().await.unwrap_err();
        assert!(format!("{err:#}").contains("binary only"), "got {err:#}");
        server.abort();

        // Oversized binary message: bigger than one maximum wire frame.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let oversized = vec![0u8; MAX_FRAME_WIRE_BYTES + 1];
            let _ = socket.send(Message::Binary(oversized.into())).await;
            tokio::time::sleep(Duration::from_millis(200)).await;
        });
        let tcp = TcpStream::connect(addr).await.unwrap();
        let mut leg = establish_websocket_leg_over(tcp, &format!("ws://{addr}/serve-proxy"))
            .await
            .unwrap();
        // The rejection itself is the oracle; its wording belongs to
        // whichever layer (this leg's explicit bound check or the
        // WebSocket capacity error) trips first.
        assert!(leg.recv_frame().await.is_err());
        server.abort();

        // A binary message whose frame claims a DATA payload over the
        // 16 KiB cap is rejected by the codec at the same boundary.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let mut body = vec![2u8]; // FrameType::Data
            body.extend_from_slice(&1u32.to_be_bytes());
            body.extend_from_slice(&0u64.to_be_bytes());
            body.extend_from_slice(&vec![0u8; MAX_DATA_PAYLOAD + 1]);
            let mut wire = Vec::new();
            wire.extend_from_slice(&u32::try_from(body.len()).unwrap().to_be_bytes());
            wire.extend_from_slice(&body);
            socket.send(Message::Binary(wire.into())).await.unwrap();
            tokio::time::sleep(Duration::from_millis(200)).await;
        });
        let tcp = TcpStream::connect(addr).await.unwrap();
        let mut leg = establish_websocket_leg_over(tcp, &format!("ws://{addr}/serve-proxy"))
            .await
            .unwrap();
        assert!(leg.recv_frame().await.is_err());
        server.abort();
    }

    #[tokio::test]
    async fn serve_proxy_leg_byte_stream_rejects_oversized_prefix_and_mid_frame_close() {
        // Oversized length prefix: refused as soon as the prefix is
        // visible, without the body ever arriving.
        let (client, mut server) = tokio::io::duplex(4096);
        let mut leg = ByteStreamLeg::new(client);
        let prefix = u32::try_from(MAX_FRAME_BODY + 1).unwrap().to_be_bytes();
        server.write_all(&prefix).await.unwrap();
        assert!(leg.recv_frame().await.is_err());

        // Clean close mid-frame is an error, not a silent end of leg.
        let (client, mut server) = tokio::io::duplex(4096);
        let mut leg = ByteStreamLeg::new(client);
        let frame = Frame::data(3, 7, b"ciphertext".to_vec()).unwrap();
        let wire = frame.encode().unwrap();
        server.write_all(&wire[..6]).await.unwrap();
        drop(server);
        assert!(leg.recv_frame().await.is_err());
    }
}
