//! uds_http — a minimal HTTP/1.1 server over a Unix domain socket,
//! shared by the fleet brokers' dual-bind listeners (#6216, from the
//! adopted #5699 local-transports design).
//!
//! tiny_http is TCP-only, so the brokers' request handlers are
//! transport-agnostic functions of (method, target, token header,
//! body) -> (status, JSON body); the TCP loops adapt tiny_http
//! requests to them, and this module adapts Unix-socket requests to
//! the same handlers. The wire surface, routes, and auth semantics
//! are therefore identical on both transports by construction.
//!
//! Socket lifecycle mirrors fleet-proxy's serve.rs: the parent dir
//! is created and chmod 0700, an existing socket file is
//! connect-probed — a live owner means `Ok(None)` (the caller logs
//! and serves TCP only), a dead file is unlinked as stale — and the
//! bound socket is chmod 0600.
//!
//! The parser is deliberately small: request line, headers, and a
//! Content-Length body, one request per connection
//! (`Connection: close`). That covers the brokers' whole surface:
//! queue-broker is POST-with-JSON, secrets-broker is GET/POST with
//! no meaningful request body.

use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;

const MAX_HEAD_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

/// One parsed HTTP request, transport-agnostic.
pub struct HttpRequest {
    pub method: String,
    /// Request target exactly as sent (path plus any query string).
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    /// First header value with this (case-insensitive) name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Bind `path` as a Unix listener with the fleet-proxy stale-socket
/// discipline. `Ok(None)` means a live process already owns the
/// socket (connect probe succeeded) — not an error.
pub fn bind_socket(path: &Path) -> std::io::Result<Option<UnixListener>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
    }
    if path.exists() {
        match UnixStream::connect(path) {
            Ok(_) => return Ok(None),
            Err(_) => {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    let listener = UnixListener::bind(path)?;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    Ok(Some(listener))
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "OK",
    }
}

fn read_request(stream: &mut UnixStream) -> Option<HttpRequest> {
    // Read until the header terminator, capping the head size.
    let mut buf: Vec<u8> = Vec::with_capacity(8192);
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        if let Some(pos) = find(&buf, b"\r\n\r\n") {
            break pos + 4;
        }
        if buf.len() > MAX_HEAD_BYTES {
            return None;
        }
        match stream.read(&mut chunk) {
            Ok(0) => return None, // probe connects and closes: no request
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => return None,
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.lines();
    let mut req_line = lines.next()?.split_whitespace();
    let method = req_line.next()?.to_string();
    let target = req_line.next()?.to_string();
    let mut headers = Vec::new();
    // Content-Length follows the fleet strict doctrine (the
    // fleet-proxy http.rs oracle, #6527): an unparsable value is
    // malformed — never "no body", which would dispatch the request
    // with its body silently dropped — and duplicate headers must
    // agree. Rejection is `None`: no request, no handler dispatch.
    // The oversized-length clamp below is unchanged (#6515 owns the
    // clamp semantics); duplicates are compared before clamping.
    let mut declared_length: Option<usize> = None;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let (k, v) = (k.trim().to_string(), v.trim().to_string());
            if k.eq_ignore_ascii_case("content-length") {
                let Ok(parsed) = v.parse::<usize>() else {
                    return None;
                };
                if let Some(prev) = declared_length {
                    if prev != parsed {
                        return None;
                    }
                }
                declared_length = Some(parsed);
            }
            headers.push((k, v));
        }
    }
    let content_length = declared_length.map(|n| n.min(MAX_BODY_BYTES)).unwrap_or(0);
    let mut body = buf[head_end..].to_vec();
    while body.len() < content_length {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
    }
    body.truncate(content_length);
    Some(HttpRequest {
        method,
        target,
        headers,
        body,
    })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Serve `listener` forever: one thread per connection, one request
/// per connection. `handler` returns (status, JSON body bytes) — the
/// same handler the broker's TCP loop adapts tiny_http requests to.
pub fn serve<F>(listener: UnixListener, handler: F)
where
    F: Fn(&HttpRequest) -> (u16, Vec<u8>) + Send + Sync + 'static,
{
    let handler = Arc::new(handler);
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let handler = Arc::clone(&handler);
        std::thread::spawn(move || {
            if let Some(req) = read_request(&mut stream) {
                let (status, body) = handler(&req);
                let resp = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    status,
                    reason(status),
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{bind_socket, serve};
    use std::io::{Read, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixStream;

    fn temp_socket(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("fleet-uds-test-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("broker.sock")
    }

    #[test]
    fn stale_socket_file_does_not_block_bind() {
        let path = temp_socket("stale");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // A regular file where the socket would go: the connect probe
        // fails, the file is unlinked, and the bind succeeds.
        std::fs::write(&path, b"not a socket").unwrap();
        let listener = bind_socket(&path).unwrap();
        assert!(listener.is_some());
        // Socket 0600, parent dir 0700.
        let sock_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        let dir_mode = std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(sock_mode, 0o600);
        assert_eq!(dir_mode, 0o700);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn live_socket_is_reported_not_stolen() {
        let path = temp_socket("live");
        let first = bind_socket(&path).unwrap().unwrap();
        // A second bind while the first listener lives: the probe
        // connects, so bind_socket reports the live owner.
        let second = bind_socket(&path).unwrap();
        assert!(second.is_none());
        drop(first);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn serves_the_handler_over_the_socket() {
        let path = temp_socket("serve");
        let listener = bind_socket(&path).unwrap().unwrap();
        std::thread::spawn(move || {
            serve(listener, |req| {
                let body = format!(
                    "{{\"method\":\"{}\",\"target\":\"{}\",\"token\":\"{}\"}}",
                    req.method,
                    req.target,
                    req.header("X-Agent-Token").unwrap_or("")
                );
                (200, body.into_bytes())
            });
        });
        let mut stream = UnixStream::connect(&path).unwrap();
        stream
            .write_all(
                b"POST /healthz HTTP/1.1\r\nHost: uds\r\nX-Agent-Token: t0ken\r\nContent-Length: 2\r\n\r\n{}",
            )
            .unwrap();
        let mut resp = String::new();
        stream.read_to_string(&mut resp).unwrap();
        assert!(resp.starts_with("HTTP/1.1 200"), "resp: {resp}");
        assert!(resp.contains("\"method\":\"POST\""), "resp: {resp}");
        assert!(resp.contains("\"target\":\"/healthz\""), "resp: {resp}");
        assert!(resp.contains("\"token\":\"t0ken\""), "resp: {resp}");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    // #6527: Content-Length validation. The oracle is the response
    // (or its absence) plus the handler invocation count: a rejected
    // shape must never reach the handler, so no response is written
    // and the connection closes with an empty read.
    fn serve_counting(
        tag: &str,
    ) -> (
        std::path::PathBuf,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let path = temp_socket(tag);
        let listener = bind_socket(&path).unwrap().unwrap();
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = std::sync::Arc::clone(&calls);
        std::thread::spawn(move || {
            serve(listener, move |req| {
                counter.fetch_add(1, Ordering::SeqCst);
                (200, req.body.clone())
            });
        });
        (path, calls)
    }

    fn roundtrip(path: &std::path::Path, request: &[u8]) -> String {
        let mut stream = UnixStream::connect(path).unwrap();
        stream.write_all(request).unwrap();
        let mut resp = String::new();
        stream.read_to_string(&mut resp).unwrap();
        resp
    }

    #[test]
    fn unparsable_content_length_is_rejected_before_the_handler() {
        use std::sync::atomic::Ordering;
        for (tag, value) in [("cl-alpha", "abc"), ("cl-negative", "-5"), ("cl-empty", "")] {
            let (path, calls) = serve_counting(tag);
            let request = format!(
                "POST /healthz HTTP/1.1\r\nHost: uds\r\nContent-Length: {value}\r\n\r\n{{}}"
            );
            let resp = roundtrip(&path, request.as_bytes());
            assert_eq!(
                resp, "",
                "value {value:?} must get no response, got: {resp}"
            );
            assert_eq!(
                calls.load(Ordering::SeqCst),
                0,
                "handler must not run for Content-Length: {value:?}"
            );
            let _ = std::fs::remove_dir_all(path.parent().unwrap());
        }
    }

    #[test]
    fn conflicting_duplicate_content_lengths_are_rejected_before_the_handler() {
        use std::sync::atomic::Ordering;
        let (path, calls) = serve_counting("cl-conflict");
        let resp = roundtrip(
            &path,
            b"POST /healthz HTTP/1.1\r\nHost: uds\r\nContent-Length: 2\r\nContent-Length: 3\r\n\r\n{}",
        );
        assert_eq!(
            resp, "",
            "conflicting duplicates must get no response, got: {resp}"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn identical_duplicate_content_lengths_are_accepted() {
        use std::sync::atomic::Ordering;
        let (path, calls) = serve_counting("cl-agree");
        let resp = roundtrip(
            &path,
            b"POST /healthz HTTP/1.1\r\nHost: uds\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n{}",
        );
        assert!(resp.starts_with("HTTP/1.1 200"), "resp: {resp}");
        assert!(
            resp.ends_with("{}"),
            "handler must receive the body, resp: {resp}"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
