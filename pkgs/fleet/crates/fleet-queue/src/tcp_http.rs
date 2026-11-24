//! Shared broker TCP admission and request dispatch (#14330).
//!
//! Admission happens on raw sockets, before parsing or spawning. tiny_http's
//! Request destructor drains unread bodies and its internal connection pool
//! grows without a cap, so a request-level limiter cannot safely shed load.
//! Overload closes the socket, just like UDS: no response write, drain, queue,
//! detached rejection worker, or retained Request. Each admitted connection
//! serves one request with Connection: close and bounded I/O timeouts.

use crate::uds_http::{self, HttpRequest, InFlightSlot, MAX_BODY_BYTES, READ_TIMEOUT};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

pub(crate) fn serve<F, G>(listener: TcpListener, precheck: G, handler: F)
where
    F: Fn(&HttpRequest) -> (u16, Vec<u8>) + Send + Sync + 'static,
    G: Fn(&HttpRequest) -> Option<(u16, Vec<u8>)> + Send + Sync + 'static,
{
    serve_connections(listener.incoming(), precheck, handler);
}

fn serve_connections<F, G>(
    connections: impl Iterator<Item = std::io::Result<TcpStream>>,
    precheck: G,
    handler: F,
) where
    F: Fn(&HttpRequest) -> (u16, Vec<u8>) + Send + Sync + 'static,
    G: Fn(&HttpRequest) -> Option<(u16, Vec<u8>)> + Send + Sync + 'static,
{
    let precheck = Arc::new(precheck);
    let handler = Arc::new(handler);
    let in_flight = Arc::new(AtomicUsize::new(0));
    for stream in connections {
        let Ok(mut stream) = stream else { continue };
        let Some(slot) = InFlightSlot::try_acquire(&in_flight) else {
            continue;
        };
        let precheck = Arc::clone(&precheck);
        let handler = Arc::clone(&handler);
        // Builder::spawn failure drops the closure, stream and slot as well.
        let _ = std::thread::Builder::new()
            .name("broker-tcp".into())
            .spawn(move || {
                let _slot = slot;
                // If timeout installation fails, fail closed rather than serving
                // an unbounded blocking read/write.
                if stream.set_read_timeout(Some(READ_TIMEOUT)).is_err()
                    || stream.set_write_timeout(Some(READ_TIMEOUT)).is_err()
                {
                    return;
                }
                if let Some((status, body)) = respond(&mut stream, &*precheck, &*handler) {
                    uds_http::write_response(&mut stream, status, &body);
                }
                // Drop the raw socket before releasing the admission slot.
                drop(stream);
            });
    }
}

fn respond(
    stream: &mut TcpStream,
    precheck: &impl Fn(&HttpRequest) -> Option<(u16, Vec<u8>)>,
    handler: &impl Fn(&HttpRequest) -> (u16, Vec<u8>),
) -> Option<(u16, Vec<u8>)> {
    let (mut head, length) = uds_http::read_tcp_head(stream)?;
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
        _ => return Some((400, b"{\"error\":\"invalid body framing\"}".to_vec())),
    };
    if let Some(response) = precheck(&head) {
        return Some(response);
    }
    if length > MAX_BODY_BYTES {
        return Some((413, b"{\"error\":\"request body too large\"}".to_vec()));
    }
    if let Some(expect) = head.header("Expect") {
        if !expect.eq_ignore_ascii_case("100-continue") {
            return Some((417, b"{\"error\":\"unsupported expectation\"}".to_vec()));
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
            return Some((413, b"{\"error\":\"request body too large\"}".to_vec()));
        }
        head.body = body;
        head
    } else {
        uds_http::read_body(stream, head, length)?
    };
    Some(handler(&req))
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
        let mut all_closed = true;
        for peer in rejected.iter_mut() {
            let mut response = Vec::new();
            match peer.read_to_end(&mut response) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
                Err(_) => all_closed = false,
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
            all_closed,
            "saturation must close without draining withheld bodies"
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
}
