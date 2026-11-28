//! Shared broker TCP admission and request dispatch (#14330).
//!
//! Admission happens on raw sockets, before parsing or spawning. tiny_http's
//! Request destructor drains unread bodies and its internal connection pool
//! grows without a cap, so a request-level limiter cannot safely shed load.
//! Overload closes the socket, just like UDS: no response write, drain, queue,
//! detached rejection worker, or retained Request. Each admitted connection
//! serves one request with Connection: close and bounded I/O timeouts.

use crate::uds_http::{
    self, HttpRequest, InFlightSlot, MAX_BODY_BYTES, READ_TIMEOUT, RequestTransport,
    RequestTransportKind,
};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

trait RequestTransportStream: Read + Write {
    fn request_transport(&self) -> RequestTransport;
    fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()>;
}

impl RequestTransportStream for TcpStream {
    fn request_transport(&self) -> RequestTransport {
        RequestTransport {
            kind: RequestTransportKind::TcpPlain,
            peer_ip: self.peer_addr().ok().map(|address| address.ip()),
        }
    }
    fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        TcpStream::set_read_timeout(self, timeout)
    }
}

#[cfg(feature = "queue-broker")]
impl RequestTransportStream for crate::broker_tls::BrokerStream {
    fn request_transport(&self) -> RequestTransport {
        crate::broker_tls::BrokerStream::request_transport(self)
    }
    fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        match self {
            crate::broker_tls::BrokerStream::Plain(stream) => {
                TcpStream::set_read_timeout(stream, timeout)
            }
            crate::broker_tls::BrokerStream::Tls(stream) => {
                TcpStream::set_read_timeout(&stream.sock, timeout)
            }
        }
    }
}

pub(crate) fn serve<F, G>(listener: TcpListener, precheck: G, handler: F)
where
    F: Fn(&HttpRequest) -> (u16, Vec<u8>) + Send + Sync + 'static,
    G: Fn(&HttpRequest) -> Option<(u16, Vec<u8>)> + Send + Sync + 'static,
{
    serve_connections(
        listener.incoming(),
        |stream: TcpStream| prepare(stream),
        precheck,
        handler,
    );
}

/// BT-3 (#14401): the same admission and dispatch with the
/// broker's TLS server wrapped around each accepted connection
/// (dual-accept: TLS handshakes are terminated, plaintext
/// Phase 0 connections are served as before — design rule M2).
#[cfg(feature = "queue-broker")]
pub(crate) fn serve_tls<F, G>(
    listener: TcpListener,
    tls: Arc<crate::broker_tls::BrokerTlsServer>,
    precheck: G,
    handler: F,
) where
    F: Fn(&HttpRequest) -> (u16, Vec<u8>) + Send + Sync + 'static,
    G: Fn(&HttpRequest) -> Option<(u16, Vec<u8>)> + Send + Sync + 'static,
{
    serve_connections(
        listener.incoming(),
        move |stream: TcpStream| {
            let stream = prepare(stream)?;
            tls.accept(stream)
        },
        precheck,
        handler,
    );
}

/// Install the bounded I/O timeouts every served connection
/// runs under. If installation fails, fail closed (drop the
/// connection) rather than serving an unbounded blocking
/// read/write.
fn prepare(stream: TcpStream) -> Option<TcpStream> {
    if stream.set_read_timeout(Some(READ_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(READ_TIMEOUT)).is_err()
    {
        return None;
    }
    Some(stream)
}

fn serve_connections<S, W, F, G>(
    connections: impl Iterator<Item = std::io::Result<TcpStream>>,
    wrap: W,
    precheck: G,
    handler: F,
) where
    S: RequestTransportStream + Send + 'static,
    W: Fn(TcpStream) -> Option<S> + Send + Sync + 'static,
    F: Fn(&HttpRequest) -> (u16, Vec<u8>) + Send + Sync + 'static,
    G: Fn(&HttpRequest) -> Option<(u16, Vec<u8>)> + Send + Sync + 'static,
{
    let wrap = Arc::new(wrap);
    let precheck = Arc::new(precheck);
    let handler = Arc::new(handler);
    let in_flight = Arc::new(AtomicUsize::new(0));
    for stream in connections {
        let Ok(stream) = stream else { continue };
        let Some(slot) = InFlightSlot::try_acquire(&in_flight) else {
            continue;
        };
        let wrap = Arc::clone(&wrap);
        let precheck = Arc::clone(&precheck);
        let handler = Arc::clone(&handler);
        // Builder::spawn failure drops the closure, stream and slot as well.
        let _ = std::thread::Builder::new()
            .name("broker-tcp".into())
            .spawn(move || {
                let _slot = slot;
                let Some(mut stream) = wrap(stream) else {
                    return;
                };
                if let Some((status, body, body_consumed)) =
                    respond(&mut stream, &*precheck, &*handler)
                {
                    uds_http::write_response(&mut stream, status, &body);
                    if !body_consumed {
                        drain_unread(&mut stream);
                    }
                }
                // Drop the connection before releasing the admission slot.
                drop(stream);
            });
    }
}

fn respond<S: RequestTransportStream>(
    stream: &mut S,
    precheck: &impl Fn(&HttpRequest) -> Option<(u16, Vec<u8>)>,
    handler: &impl Fn(&HttpRequest) -> (u16, Vec<u8>),
) -> Option<(u16, Vec<u8>, bool)> {
    let (mut head, length) = uds_http::read_tcp_head(stream)?;
    head.transport = stream.request_transport();
    // Reject ambiguous or unsupported framing, including duplicate TE.
    let encodings: Vec<_> = head
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("Transfer-Encoding"))
        .map(|(_, value)| value.as_str())
        .collect();
    let chunked = match encodings.as_slice() {
        [] => false,
        [value]
            if value.eq_ignore_ascii_case("chunked") && head.header("Content-Length").is_none() =>
        {
            true
        }
        _ => return Some((400, b"{\"error\":\"invalid body framing\"}".to_vec(), false)),
    };
    if let Some((status, body)) = precheck(&head) {
        return Some((status, body, false));
    }
    if length > MAX_BODY_BYTES {
        return Some((
            413,
            b"{\"error\":\"request body too large\"}".to_vec(),
            false,
        ));
    }
    if let Some(expect) = head.header("Expect") {
        if !expect.eq_ignore_ascii_case("100-continue") {
            return Some((
                417,
                b"{\"error\":\"unsupported expectation\"}".to_vec(),
                false,
            ));
        }
        stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").ok()?;
    }
    let req = if chunked {
        let prefix = std::io::Cursor::new(std::mem::take(&mut head.body));
        let mut body = Vec::new();
        chunked_transfer::Decoder::new(prefix.chain(&mut *stream))
            .take((MAX_BODY_BYTES + 1) as u64)
            .read_to_end(&mut body)
            .ok()?;
        if body.len() > MAX_BODY_BYTES {
            // Match the prior tiny_http adapter's post-read refusal body.
            return Some((413, b"{}".to_vec(), true));
        }
        head.body = body;
        head
    } else {
        uds_http::read_body(stream, head, length)?
    };
    let (status, body) = handler(&req);
    Some((status, body, true))
}

/// How long the lingering close below waits for one more chunk of
/// the body a refused request pipelined with its head. The body was
/// written before the response, so it is already in flight; this is
/// a grace window for draining it, not a read deadline for peers.
const LINGER_CLOSE_TIMEOUT: Duration = Duration::from_millis(100);

/// Lingering close for a response sent before the request body was
/// consumed (precheck, framing and size refusals — #16583): closing
/// a TCP socket with the pipelined body still unread makes the
/// kernel reset the connection, and the reset can discard the
/// response before the peer reads it — the intermittent
/// `Connection reset by peer` on the broker_tls wrong-token
/// roundtrip. Discard whatever the peer already sent, bounded in
/// bytes and by one short timeout per read, so the close that
/// follows is a clean FIN / TLS close_notify. A peer withholding
/// its body delays only this connection's close, by one timeout;
/// the response is already on the wire either way.
fn drain_unread<S: RequestTransportStream>(stream: &mut S) {
    if stream.set_read_timeout(Some(LINGER_CLOSE_TIMEOUT)).is_err() {
        return;
    }
    let mut chunk = [0u8; 8192];
    let mut drained = 0usize;
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                drained += n;
                if drained > MAX_BODY_BYTES {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uds_http::MAX_CONNECTIONS;
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpStream};
    use std::sync::atomic::Ordering;
    use std::sync::{Condvar, Mutex, mpsc};
    use std::time::Duration;

    #[test]
    fn unauthenticated_withheld_body_burst_is_bounded_and_recovers() {
        // Oracle: at most MAX_CONNECTIONS worker prechecks may be entered;
        // overload sockets close promptly, and a healthy request completes
        // after capacity frees while rejected peers still hold their sockets.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let entered = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (tx, rx) = mpsc::channel();
        let worker_entered = Arc::clone(&entered);
        let worker_gate = Arc::clone(&gate);
        std::thread::spawn(move || {
            serve(
                listener,
                move |head| {
                    if head.target == "/hold" {
                        worker_entered.fetch_add(1, Ordering::SeqCst);
                        tx.send(()).unwrap();
                        let (lock, cv) = &*worker_gate;
                        let _released = cv
                            .wait_while(lock.lock().unwrap(), |released| !*released)
                            .unwrap();
                        return Some((401, b"{}".to_vec()));
                    }
                    None
                },
                |_| (200, b"ok".to_vec()),
            );
        });
        let connect = |target: &str| {
            let mut stream = TcpStream::connect(addr).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            write!(
                stream,
                "POST {target} HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2048\r\n\r\n"
            )
            .unwrap();
            stream
        };
        let held: Vec<_> = (0..MAX_CONNECTIONS).map(|_| connect("/hold")).collect();
        for _ in 0..MAX_CONNECTIONS {
            rx.recv_timeout(Duration::from_secs(10))
                .expect("worker admitted");
        }
        let mut rejected: Vec<_> = (0..32)
            .map(|_| {
                let mut stream = TcpStream::connect(addr).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                // Overload closes the raw socket on accept: the RST can
                // arrive before these headers flush, so a write failure
                // here is itself saturation shedding, not a test error.
                let _ = write!(
                    stream,
                    "POST /hold HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2048\r\n\r\n"
                );
                stream
            })
            .collect();
        // Shed means close with no response bytes: any payload (even a
        // 503) proves the server wrote instead of shedding the socket.
        let mut shed_cleanly = true;
        for peer in rejected.iter_mut() {
            let mut response = Vec::new();
            match peer.read_to_end(&mut response) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
                Err(_) => {
                    shed_cleanly = false;
                    continue;
                }
            }
            if !response.is_empty() {
                shed_cleanly = false;
            }
        }
        // Always release test workers even on failure, to avoid hanging the
        // suite on the very regression this test detects.
        let (lock, cv) = &*gate;
        *lock.lock().unwrap() = true;
        cv.notify_all();
        for peer in held {
            let _ = peer.shutdown(Shutdown::Both);
        }
        assert!(
            shed_cleanly,
            "saturation must close with no response bytes, without draining withheld bodies"
        );
        assert_eq!(entered.load(Ordering::SeqCst), MAX_CONNECTIONS);
        let mut healthy = TcpStream::connect(addr).unwrap();
        healthy
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        healthy
            .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        healthy.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        // Keep these alive through the recovery assertion.
        drop(rejected);
    }

    #[test]
    fn chunked_expect_and_oversize_parity() {
        // Oracle: the TCP adapter keeps tiny_http's wire support —
        // chunked bodies decode, Expect: 100-continue is honoured, and
        // an oversized declared length is refused with 413.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            serve(listener, |_| None, |req| (200, req.body.clone()));
        });
        let roundtrip = |request: &[u8], body: Option<&[u8]>| -> String {
            let mut stream = TcpStream::connect(addr).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream.write_all(request).unwrap();
            if let Some(body) = body {
                // The server must have sent the interim 100 first.
                let mut interim = [0u8; 25];
                stream.read_exact(&mut interim).unwrap();
                assert_eq!(&interim, b"HTTP/1.1 100 Continue\r\n\r\n");
                stream.write_all(body).unwrap();
            }
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        };
        let chunked = roundtrip(
            b"POST /echo HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\nwiki\r\n5\r\npedia\r\n0\r\n\r\n",
            None,
        );
        assert!(chunked.starts_with("HTTP/1.1 200"), "{chunked}");
        assert!(chunked.ends_with("wikipedia"), "{chunked}");
        let continued = roundtrip(
            b"POST /echo HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\nExpect: 100-continue\r\nConnection: close\r\n\r\n",
            Some(b"{}"),
        );
        assert!(continued.starts_with("HTTP/1.1 200"), "{continued}");
        assert!(continued.ends_with("{}"), "{continued}");
        let oversize = format!(
            "POST /echo HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            super::super::uds_http::MAX_BODY_BYTES + 1
        );
        let big = roundtrip(oversize.as_bytes(), None);
        assert!(big.starts_with("HTTP/1.1 413"), "{big}");
    }

    #[test]
    fn precheck_refusal_with_pipelined_body_delivers_response() {
        // Oracle (#16583): a precheck refusal for a request whose body
        // was pipelined with its head must reach the client intact on
        // every roundtrip — never discarded by a connection reset
        // caused by closing the socket with the body still unread.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            serve(
                listener,
                |head| {
                    if head.header("X-Agent-Token") == Some("secret") {
                        None
                    } else {
                        Some((401, b"{\"error\":\"bad token\"}".to_vec()))
                    }
                },
                |_| (200, b"ok".to_vec()),
            );
        });
        for _ in 0..50 {
            let mut stream = TcpStream::connect(addr).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .write_all(
                    b"POST /healthz HTTP/1.1\r\nHost: localhost\r\n\
                      X-Agent-Token: wrong\r\nContent-Length: 2\r\n\
                      Connection: close\r\n\r\n",
                )
                .unwrap();
            stream.write_all(b"{}").unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 401"), "{response}");
            assert!(
                response.ends_with("{\"error\":\"bad token\"}"),
                "{response}"
            );
        }
    }
}
