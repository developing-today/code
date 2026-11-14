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
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const MAX_HEAD_BYTES: usize = 64 * 1024;
/// Largest request body either broker accepts, on any transport
/// (#6515). A declared Content-Length above this is rejected before
/// the body is read — never clamped, which would dispatch a silently
/// truncated request (PR #6514 thread 4492609738).
pub(crate) const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;
/// Per-connection read/write timeout (#6515, PR #6514 thread
/// 4492609760): a peer that stalls mid-request loses its connection
/// instead of holding a serve thread forever.
pub(crate) const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// Bound on simultaneous UDS connections (#6515, thread 4492609760):
/// past this, new connections are dropped until a slot frees.
const MAX_CONNECTIONS: usize = 128;
/// How long the stale-socket probe waits for a connect to resolve
/// before assuming a live owner (#6515, thread 4492609644). A unix
/// connect to a live listener completes immediately; a full backlog
/// can block indefinitely, and an uncertain probe must never steal
/// the socket — timeout reads as "live".
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

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

/// RAII guard for the bind lock file: flock held until drop.
struct BindLock(std::fs::File);

fn bind_lock(path: &Path) -> std::io::Result<BindLock> {
    // Serialize probe/unlink/bind across concurrent starters (#6515,
    // PR #6514 thread 4492609657): without this, two starters can both
    // probe a stale socket, and the second one's unlink can remove
    // the socket the first one just bound live.
    let lock_path = path.with_file_name(format!(
        "{}.bind-lock",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;
    let rc = unsafe { libc::flock(std::os::unix::io::AsRawFd::as_raw_fd(&file), libc::LOCK_EX) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(BindLock(file))
}

fn probe_live(path: &Path) -> bool {
    // Bound the live-owner probe (#6515, thread 4492609644): run the
    // connect on a side thread and wait at most PROBE_TIMEOUT. A
    // timeout (or a dead probe thread) reads as a live owner — the
    // safe direction, since stealing is the destructive outcome.
    let (tx, rx) = std::sync::mpsc::channel();
    let probe_path = path.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(UnixStream::connect(&probe_path).is_ok());
    });
    rx.recv_timeout(PROBE_TIMEOUT).unwrap_or(true)
}

/// Bind `path` as a Unix listener with the fleet-proxy stale-socket
/// discipline. `Ok(None)` means a live process already owns the
/// socket (connect probe succeeded) — not an error.
pub fn bind_socket(path: &Path) -> std::io::Result<Option<UnixListener>> {
    if let Some(parent) = path.parent() {
        let parent_existed = parent.exists();
        std::fs::create_dir_all(parent)?;
        if !parent_existed {
            // Only a directory this call created gets its mode set
            // (#6515, PR #6514 thread 4492609622): chmod on a
            // pre-existing, possibly shared directory would silently
            // rewrite the owner's chosen permissions.
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
    }
    let _lock = bind_lock(path)?;
    if path.exists() {
        if probe_live(path) {
            return Ok(None);
        }
        // Stale: unlink only an actual socket file (#6515, PR #6514
        // thread 4492609635). A non-socket file at the socket path is
        // someone else's data (e.g. a mispointed override) — refuse,
        // never delete it.
        match std::fs::symlink_metadata(path) {
            Ok(md) if md.file_type().is_socket() => {
                std::fs::remove_file(path)?;
            }
            Ok(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "{} exists and is not a socket; refusing to remove it",
                        path.display()
                    ),
                ));
            }
            Err(e) => return Err(e),
        }
    }
    let listener = UnixListener::bind(path)?;
    // A socket we cannot restrict to owner-only must not be served
    // (#6515, PR #6514 thread 4492609673): fail the bind (the caller
    // degrades to TCP only) instead of ignoring the chmod error.
    if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        let _ = std::fs::remove_file(path);
        return Err(e);
    }
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
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "OK",
    }
}

/// Read and parse only the request head (request line + headers).
/// Returns the request — with any body bytes that arrived in the
/// same reads parked in `body` — and the declared Content-Length.
/// Splitting head from body is what lets `serve_with_precheck`
/// refuse a request (e.g. unauthenticated, #6515) before a single
/// body byte is consumed.
fn read_head(stream: &mut UnixStream) -> Option<(HttpRequest, usize)> {
    // Read until the header terminator, capping the head size. The
    // cap applies to the complete header (#6515, PR #6514 thread
    // 4492609689): a terminator that only appears past the cap does
    // not rescue an oversized head.
    let mut buf: Vec<u8> = Vec::with_capacity(8192);
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        if let Some(pos) = find(&buf, b"\r\n\r\n") {
            if pos + 4 > MAX_HEAD_BYTES {
                return None;
            }
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
    let mut declared_length: Option<usize> = None;
    let mut has_transfer_encoding = false;
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
            if k.eq_ignore_ascii_case("transfer-encoding") {
                has_transfer_encoding = true;
            }
            headers.push((k, v));
        }
    }
    // Chunked (or any) Transfer-Encoding is not decoded on this
    // transport (#6515, PR #6514 thread 4492609728): the brokers'
    // clients always send Content-Length, so an encoded body is
    // rejected outright instead of being misparsed as length-less.
    if has_transfer_encoding {
        return None;
    }
    let content_length = declared_length.unwrap_or(0);
    // An oversized body is rejected from its declared length, before
    // a byte of it is read (#6515, thread 4492609738) — the previous
    // clamp dispatched the handler with a silently truncated body.
    if content_length > MAX_BODY_BYTES {
        return None;
    }
    let body = buf[head_end..].to_vec();
    Some((
        HttpRequest {
            method,
            target,
            headers,
            body,
        },
        content_length,
    ))
}

/// Read the rest of the body declared by `read_head`. An incomplete
/// body — the peer closes before the declared length arrives — is
/// rejected, never dispatched truncated (#6515, PR #6514 thread
/// 4492609747).
fn read_body(
    stream: &mut UnixStream,
    mut req: HttpRequest,
    content_length: usize,
) -> Option<HttpRequest> {
    let mut chunk = [0u8; 8192];
    while req.body.len() < content_length {
        match stream.read(&mut chunk) {
            Ok(0) => return None,
            Ok(n) => req.body.extend_from_slice(&chunk[..n]),
            Err(_) => return None,
        }
    }
    req.body.truncate(content_length);
    Some(req)
}

fn read_request(stream: &mut UnixStream) -> Option<HttpRequest> {
    let (req, content_length) = read_head(stream)?;
    read_body(stream, req, content_length)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn write_response(stream: &mut UnixStream, status: u16, body: &[u8]) {
    let resp = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status,
        reason(status),
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

/// Serve `listener` forever: one thread per connection (bounded by
/// MAX_CONNECTIONS), one request per connection. `handler` returns
/// (status, JSON body bytes) — the same handler the broker's TCP
/// loop adapts tiny_http requests to.
pub fn serve<F>(listener: UnixListener, handler: F)
where
    F: Fn(&HttpRequest) -> (u16, Vec<u8>) + Send + Sync + 'static,
{
    serve_inner(
        listener,
        None::<Arc<dyn Fn(&HttpRequest) -> Option<(u16, Vec<u8>)> + Send + Sync>>,
        handler,
    );
}

/// Like `serve`, but `precheck` runs on the parsed head — method,
/// target, headers, no body yet — before the body is read (#6515).
/// `Some((status, body))` from the precheck is the response, sent
/// without consuming the request body; `None` proceeds to body read
/// and the handler. This is how the brokers reject unauthorized
/// UDS requests before reading their bodies.
pub fn serve_with_precheck<F, G>(listener: UnixListener, precheck: G, handler: F)
where
    F: Fn(&HttpRequest) -> (u16, Vec<u8>) + Send + Sync + 'static,
    G: Fn(&HttpRequest) -> Option<(u16, Vec<u8>)> + Send + Sync + 'static,
{
    serve_inner(
        listener,
        Some(Arc::new(precheck)
            as Arc<
                dyn Fn(&HttpRequest) -> Option<(u16, Vec<u8>)> + Send + Sync,
            >),
        handler,
    );
}

fn serve_inner<F>(
    listener: UnixListener,
    precheck: Option<Arc<dyn Fn(&HttpRequest) -> Option<(u16, Vec<u8>)> + Send + Sync>>,
    handler: F,
) where
    F: Fn(&HttpRequest) -> (u16, Vec<u8>) + Send + Sync + 'static,
{
    let handler = Arc::new(handler);
    let in_flight = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        // Bound connection resources (#6515, thread 4492609760):
        // past the cap the connection is dropped unaccepted-work
        // style — closed immediately, no thread spawned.
        if in_flight.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            in_flight.fetch_sub(1, Ordering::SeqCst);
            continue;
        }
        let handler = Arc::clone(&handler);
        let precheck = precheck.clone();
        let in_flight = Arc::clone(&in_flight);
        std::thread::spawn(move || {
            let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
            let _ = stream.set_write_timeout(Some(READ_TIMEOUT));
            let outcome = (|| -> Option<(u16, Vec<u8>)> {
                let (head, content_length) = read_head(&mut stream)?;
                if let Some(pre) = &precheck {
                    if let Some(resp) = pre(&head) {
                        return Some(resp);
                    }
                }
                let req = read_body(&mut stream, head, content_length)?;
                Some(handler(&req))
            })();
            if let Some((status, body)) = outcome {
                write_response(&mut stream, status, &body);
            }
            in_flight.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{bind_socket, serve, serve_with_precheck};
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
        // A dead socket file where the socket would go: bind once
        // (which also creates the parent dir), drop the listener —
        // the file remains, the connect probe fails, the file is
        // unlinked, and the re-bind succeeds.
        let first = bind_socket(&path).unwrap().unwrap();
        drop(first);
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
    fn non_socket_file_is_preserved_and_bind_refused() {
        // #6515 (PR #6514 thread 4492609635): a regular file at the
        // socket path is someone else's data. Oracle: after the bind
        // attempt the file's contents are byte-identical and the
        // bind failed instead of deleting it.
        let path = temp_socket("not-a-socket");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"not a socket").unwrap();
        let result = bind_socket(&path);
        assert!(result.is_err(), "bind over a regular file must fail");
        assert_eq!(std::fs::read(&path).unwrap(), b"not a socket");
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
    fn conflicting_duplicate_content_lengths_reversed_are_rejected() {
        use std::sync::atomic::Ordering;
        let (path, calls) = serve_counting("cl-conflict-rev");
        let resp = roundtrip(
            &path,
            b"POST /healthz HTTP/1.1\r\nHost: uds\r\nContent-Length: 3\r\nContent-Length: 2\r\n\r\n{}",
        );
        assert_eq!(
            resp, "",
            "conflicting duplicates (reverse order) must get no response, got: {resp}"
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

    // #6515 hardening (PR #6514 threads 4492609738 / 4492609747 /
    // 4492609728). Oracle, same shape as the #6527 tests: a rejected
    // request gets no response and the handler runs zero times.

    #[test]
    fn oversized_content_length_is_rejected_before_the_handler() {
        use std::sync::atomic::Ordering;
        let (path, calls) = serve_counting("cl-oversized");
        let request = format!(
            "POST /healthz HTTP/1.1\r\nHost: uds\r\nContent-Length: {}\r\n\r\n",
            super::MAX_BODY_BYTES + 1
        );
        let resp = roundtrip(&path, request.as_bytes());
        assert_eq!(resp, "", "oversized body must get no response, got: {resp}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn incomplete_body_is_rejected_before_the_handler() {
        use std::sync::atomic::Ordering;
        let (path, calls) = serve_counting("cl-incomplete");
        // Declare 10 body bytes, send 2, then close the write side:
        // the server sees EOF mid-body and must not dispatch the
        // truncated request.
        let mut stream = UnixStream::connect(&path).unwrap();
        stream
            .write_all(b"POST /healthz HTTP/1.1\r\nHost: uds\r\nContent-Length: 10\r\n\r\n{}")
            .unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        let mut resp = String::new();
        stream.read_to_string(&mut resp).unwrap();
        assert_eq!(
            resp, "",
            "incomplete body must get no response, got: {resp}"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn chunked_transfer_encoding_is_rejected_before_the_handler() {
        use std::sync::atomic::Ordering;
        let (path, calls) = serve_counting("te-chunked");
        let resp = roundtrip(
            &path,
            b"POST /healthz HTTP/1.1\r\nHost: uds\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n",
        );
        assert_eq!(resp, "", "chunked body must get no response, got: {resp}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn precheck_refuses_before_the_body_is_consumed() {
        // #6515, the TCP finding's UDS twin (thread 4492609604).
        // Oracle: the client sends headers declaring a 100-byte body
        // and then sends NOTHING — a 401 response still arrives
        // promptly, which is only possible if the refusal was made
        // from the head alone, before the body was read.
        use std::sync::atomic::{AtomicUsize, Ordering};
        let path = temp_socket("precheck-auth");
        let listener = bind_socket(&path).unwrap().unwrap();
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = std::sync::Arc::clone(&calls);
        std::thread::spawn(move || {
            serve_with_precheck(
                listener,
                |head| {
                    if head.header("X-Agent-Token") == Some("t0ken") {
                        None
                    } else {
                        Some((401, br#"{"error":"unauthorized"}"#.to_vec()))
                    }
                },
                move |req| {
                    counter.fetch_add(1, Ordering::SeqCst);
                    (200, req.body.clone())
                },
            );
        });
        let mut stream = UnixStream::connect(&path).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        stream
            .write_all(b"POST /claim HTTP/1.1\r\nHost: uds\r\nContent-Length: 100\r\n\r\n")
            .unwrap();
        // No body bytes are ever sent. The 401 must arrive anyway.
        let mut buf = [0u8; 512];
        let n = stream.read(&mut buf).expect("401 arrives without a body");
        let resp = String::from_utf8_lossy(&buf[..n]).into_owned();
        assert!(resp.starts_with("HTTP/1.1 401"), "resp: {resp}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    // Property-based coverage of the #6527 Content-Length invariants
    // (Codex review on PR #6529; the oracle is the invariant itself,
    // per the repo testing doctrine — the fixed examples above stay
    // as smoke coverage). Oracle: a request shape the strict doctrine
    // rejects must never reach the handler (no response, zero calls);
    // a shape it accepts must dispatch exactly once with the declared
    // body intact.
    fn prop_socket() -> (
        std::path::PathBuf,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        serve_counting(&format!("cl-prop-{n}"))
    }

    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]

        #[test]
        fn unparsable_content_length_never_dispatches(
            value in prop_oneof![
                "[a-z]{1,6}",
                "-[0-9]{1,4}",
                "[0-9]{1,4}[a-z]",
                "[0-9]\\.[0-9]{1,3}",
            ],
        ) {
            use std::sync::atomic::Ordering;
            // The strategies above can never spell a non-negative
            // integer, so the oracle is decidable for every case.
            prop_assume!(value.parse::<usize>().is_err());
            let (path, calls) = prop_socket();
            let request = format!(
                "POST /healthz HTTP/1.1\r\nHost: uds\r\nContent-Length: {value}\r\n\r\n{{}}"
            );
            let resp = roundtrip(&path, request.as_bytes());
            prop_assert_eq!(resp, "", "value {:?} must get no response", value);
            prop_assert_eq!(
                calls.load(Ordering::SeqCst),
                0,
                "handler must not run for Content-Length: {:?}",
                value
            );
            let _ = std::fs::remove_dir_all(path.parent().unwrap());
        }

        #[test]
        fn conflicting_duplicate_content_lengths_never_dispatch_in_either_order(
            a in 0..1000usize,
            b in 0..1000usize,
            swap in any::<bool>(),
        ) {
            use std::sync::atomic::Ordering;
            prop_assume!(a != b);
            let (first, second) = if swap { (b, a) } else { (a, b) };
            let (path, calls) = prop_socket();
            let request = format!(
                "POST /healthz HTTP/1.1\r\nHost: uds\r\nContent-Length: {first}\r\nContent-Length: {second}\r\n\r\n{{}}"
            );
            let resp = roundtrip(&path, request.as_bytes());
            prop_assert_eq!(resp, "", "conflicting {}/{} must get no response", first, second);
            prop_assert_eq!(calls.load(Ordering::SeqCst), 0);
            let _ = std::fs::remove_dir_all(path.parent().unwrap());
        }

        #[test]
        fn agreeing_duplicate_content_lengths_dispatch_with_body_intact(
            body_text in "[a-z]{0,32}",
        ) {
            use std::sync::atomic::Ordering;
            let n = body_text.len();
            let (path, calls) = prop_socket();
            let request = format!(
                "POST /healthz HTTP/1.1\r\nHost: uds\r\nContent-Length: {n}\r\nContent-Length: {n}\r\n\r\n{body_text}"
            );
            let resp = roundtrip(&path, request.as_bytes());
            prop_assert!(resp.starts_with("HTTP/1.1 200"), "resp: {}", resp);
            prop_assert!(resp.ends_with(&body_text), "body must arrive intact, resp: {}", resp);
            prop_assert_eq!(calls.load(Ordering::SeqCst), 1);
            let _ = std::fs::remove_dir_all(path.parent().unwrap());
        }
    }
}
