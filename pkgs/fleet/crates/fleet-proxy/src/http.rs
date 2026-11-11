//! A minimal HTTP/1.1 client connection over a tunnel stream —
//! just what the broker needs to speak to the laptop's services,
//! with the pooling semantics of Python's http.client made
//! explicit: a connection is either reusable after a response
//! (Content-Length or chunked body, no `Connection: close`) or it
//! is dead and the pool must not see it again.
//!
//! Errors distinguish the two cases the broker's retry rule cares
//! about: [`HttpError::Stale`] — nothing was read, the pooled
//! connection had silently died, retry fresh is safe; anything
//! else means bytes may have flowed and the request's fate is the
//! caller's problem (the broker mirrors the Python: one retry on a
//! fresh connection regardless, because these requests are the
//! idempotent-shaped local RPCs the Python broker already retries).
//!
//! Peer-declared sizes are never trusted (#5579): a malformed or
//! conflicting `Content-Length` is a [`HttpError::Malformed`] that
//! marks the connection non-reusable (treating it as "no body"
//! would leave the real body bytes in the stream and desync the
//! next response parsed on the pooled connection), declared body
//! sizes are checked against [`MAX_RESPONSE_BODY_BYTES`] *before*
//! any allocation for them, and status/header/chunk-size lines are
//! read with the [`MAX_LINE_BYTES`] bound instead of accumulating
//! without limit.
//!
//! Repetition is bounded as well as size (#6509): a peer can send
//! units that are each individually legal forever — interim 1xx
//! heads, header lines, trailer lines — and each successful read
//! resets the per-read tunnel timeout, so the exchange (and its
//! pooled tunnel) hangs indefinitely. The count bounds
//! [`MAX_INTERIM_HEADS`], [`MAX_HEADER_LINES`], and
//! [`MAX_TRAILER_LINES`] cap the repetition; exceeding one is the
//! same failure shape as exceeding a size cap —
//! [`HttpError::Malformed`], connection non-reusable.
//!
//! Body existence is decided by the request method and the status
//! class before the framing headers are consulted (#5921): a
//! response to a HEAD request and a 101, 204, or 304 response
//! never carry a body, whatever Content-Length or
//! Transfer-Encoding claim, and a 1xx interim response (except
//! 101) is consumed and discarded — the response that follows
//! it is the final one. Reading a body that by definition never
//! arrives would stall the exchange until the tunnel read
//! timeout; returning a 1xx as final would leave the real
//! response in the stream to desync the pooled connection.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;

/// Largest response body this client will accept, on either the
/// Content-Length or the chunked path (cumulative for chunked).
/// Matches the fleet-proxy-proto frame ceiling
/// (`MAX_FRAME_BYTES`, 64 MiB): a peer declaring more than this is
/// buggy or hostile, and the error is raised before allocating
/// anything sized by the declaration.
pub const MAX_RESPONSE_BODY_BYTES: usize = 64 * 1024 * 1024;

/// Largest single line (status line, one header line, one chunk
/// size line, one trailer line) this client will read. Matches the
/// tunnel CONNECT response cap precedent (65536 bytes): legitimate
/// lines from the laptop's local services are nowhere near this.
pub const MAX_LINE_BYTES: usize = 64 * 1024;

/// Largest number of interim 1xx heads (excluding 101) this client
/// will consume before the final response head. A legitimate
/// laptop service sends at most one or two (100 Continue, 103
/// Early Hints); an unbounded stream of them is a hang, not a
/// conversation (#6509).
pub const MAX_INTERIM_HEADS: usize = 16;

/// Largest number of header lines in one response head. The
/// laptop's local services send a handful; hundreds of distinct
/// lines is a peer burning the exchange, not metadata (#6509).
pub const MAX_HEADER_LINES: usize = 256;

/// Largest number of trailer lines after a chunked body's final
/// chunk. Trailers mirror header counts in legitimate traffic;
/// the same generous ceiling applies (#6509).
pub const MAX_TRAILER_LINES: usize = 256;

#[derive(Debug)]
pub enum HttpError {
    /// EOF before a single response byte: the connection was stale.
    Stale,
    /// Transport failure mid-exchange.
    Io(std::io::Error),
    /// The peer said something that is not HTTP/1.x.
    Malformed(String),
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::Stale => write!(f, "connection closed before any response"),
            HttpError::Io(e) => write!(f, "{e}"),
            HttpError::Malformed(why) => write!(f, "malformed HTTP response: {why}"),
        }
    }
}

impl From<std::io::Error> for HttpError {
    fn from(e: std::io::Error) -> Self {
        HttpError::Io(e)
    }
}

/// Whether `method` is a valid HTTP method token (RFC 9110
/// `token`): non-empty, `tchar`s only. Anything else — whitespace,
/// a control character, a separator — cannot appear in a request
/// line this client emits (#5750).
pub(crate) fn valid_method(method: &str) -> bool {
    !method.is_empty()
        && method.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

/// Whether `path` may appear in a request line: non-empty,
/// origin-form (starts with `/`), and free of spaces, control
/// characters, and DEL — the bytes that would break the line or
/// forge a header (#5750). Query strings and percent-encoded
/// bytes pass unchanged.
pub(crate) fn valid_path(path: &str) -> bool {
    path.starts_with('/') && path.bytes().all(|b| b > 0x20 && b != 0x7f)
}

/// The byte stream under an [`HttpConn`]: a TCP tunnel stream, or
/// a colocated service's Unix socket (#6219). Both are plain
/// Read+Write byte streams once connected; the HTTP exchange does
/// not care which it rides.
pub enum ConnStream {
    Tcp(TcpStream),
    Unix(UnixStream),
}

impl ConnStream {
    fn try_clone(&self) -> std::io::Result<ConnStream> {
        Ok(match self {
            ConnStream::Tcp(s) => ConnStream::Tcp(s.try_clone()?),
            ConnStream::Unix(s) => ConnStream::Unix(s.try_clone()?),
        })
    }
}

impl Read for ConnStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            ConnStream::Tcp(s) => s.read(buf),
            ConnStream::Unix(s) => s.read(buf),
        }
    }
}

impl Write for ConnStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            ConnStream::Tcp(s) => s.write(buf),
            ConnStream::Unix(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            ConnStream::Tcp(s) => s.flush(),
            ConnStream::Unix(s) => s.flush(),
        }
    }
}

/// One HTTP connection to a laptop service, over a tunnel stream
/// or a colocated Unix socket (#6219).
pub struct HttpConn {
    reader: BufReader<ConnStream>,
    writer: ConnStream,
    reusable: bool,
}

impl HttpConn {
    pub fn new(stream: TcpStream) -> std::io::Result<HttpConn> {
        Self::from_stream(ConnStream::Tcp(stream))
    }

    /// An HTTP connection over a colocated service's Unix socket
    /// (#6219): the same exchange, dialled locally instead of via
    /// TCP loopback.
    pub fn from_unix(stream: UnixStream) -> std::io::Result<HttpConn> {
        Self::from_stream(ConnStream::Unix(stream))
    }

    fn from_stream(stream: ConnStream) -> std::io::Result<HttpConn> {
        let writer = stream.try_clone()?;
        Ok(HttpConn {
            reader: BufReader::with_capacity(16 * 1024, stream),
            writer,
            reusable: true,
        })
    }

    /// Whether the connection may return to the pool.
    pub fn reusable(&self) -> bool {
        self.reusable
    }

    /// Read one line with the [`MAX_LINE_BYTES`] bound. Returns the
    /// number of bytes read (0 at EOF), like `BufRead::read_line`;
    /// a line longer than the bound is [`HttpError::Malformed`] and
    /// marks the connection non-reusable.
    fn read_line_bounded(&mut self, out: &mut String) -> Result<usize, HttpError> {
        let mut bytes = Vec::new();
        let n = self
            .reader
            .by_ref()
            .take((MAX_LINE_BYTES + 1) as u64)
            .read_until(b'\n', &mut bytes)?;
        if bytes.len() > MAX_LINE_BYTES {
            self.reusable = false;
            return Err(HttpError::Malformed(format!(
                "response line exceeds {MAX_LINE_BYTES} bytes"
            )));
        }
        match String::from_utf8(bytes) {
            Ok(s) => {
                out.push_str(&s);
                Ok(n)
            }
            Err(e) => Err(HttpError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                e,
            ))),
        }
    }

    /// One request/response exchange. `body` is the already-encoded
    /// JSON payload, or None for a bodyless request (no
    /// Content-Length is sent then — http.client's shape).
    /// Returns the status and the raw response body.
    pub fn request(
        &mut self,
        method: &str,
        path: &str,
        host_header: &str,
        token: &str,
        body: Option<&[u8]>,
    ) -> Result<(u16, Vec<u8>), HttpError> {
        // The method and path are interpolated into the request
        // line below: a control character in either is a CRLF
        // injection that forges headers or smuggles a whole second
        // request onto this pooled connection (#5750). Reject
        // before a single byte is written. (The Python broker got
        // this from http.client, which validates both.)
        if !valid_method(method) || !valid_path(path) {
            self.reusable = false;
            return Err(HttpError::Malformed(format!(
                "invalid request method or path (method {method:?})"
            )));
        }
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\n\
             Host: {host_header}\r\n\
             X-Agent-Token: {token}\r\n\
             Content-Type: application/json\r\n\
             Accept-Encoding: identity\r\n"
        );
        if let Some(b) = body {
            head.push_str(&format!("Content-Length: {}\r\n", b.len()));
        }
        head.push_str("\r\n");
        self.writer.write_all(head.as_bytes())?;
        if let Some(b) = body {
            self.writer.write_all(b)?;
        }
        self.writer.flush()?;

        // Response head(s). A 1xx interim response (except 101)
        // is not the answer: its head is consumed and the reader
        // continues to the final response (#5921). Returning the
        // interim head as final would leave the real response in
        // the stream, to be parsed as the next exchange's response
        // on this pooled connection.
        let mut interim_heads = 0usize;
        let (status, content_length, chunked, close_after) = loop {
            let head = self.read_response_head()?;
            if (100..200).contains(&head.0) && head.0 != 101 {
                interim_heads += 1;
                if interim_heads > MAX_INTERIM_HEADS {
                    self.reusable = false;
                    return Err(HttpError::Malformed(format!(
                        "more than {MAX_INTERIM_HEADS} interim 1xx heads"
                    )));
                }
                continue;
            }
            break head;
        };

        // Body existence is decided before the framing headers
        // (#5921, RFC 9112 section 6.3): a response to a HEAD request
        // and a 101, 204, or 304 response never carry a body,
        // whatever Content-Length or Transfer-Encoding claim —
        // a 304 may even carry the Content-Length of the cached
        // 200. Reading for those bytes would stall until the read
        // timeout and kill the connection; skipping them leaves
        // the stream exactly at the next response, so a keep-alive
        // exchange stays reusable.
        // The method comparison is exact: HTTP methods are
        // case-sensitive, so a lowercase `head` is a different
        // method whose response does carry a body.
        let no_body = method == "HEAD" || matches!(status, 101 | 204 | 304);

        // Body.
        let mut data = Vec::new();
        if !no_body {
            if chunked {
                self.read_chunked(&mut data)?;
            } else if let Some(len) = content_length {
                // len <= MAX_RESPONSE_BODY_BYTES was enforced while
                // parsing the header, before this allocation.
                data.resize(len, 0);
                self.reader.read_exact(&mut data).map_err(|e| {
                    self.reusable = false;
                    HttpError::Io(e)
                })?;
            } else if close_after {
                // Close-delimited body: still peer-sized, so read
                // with the same cap (the connection is dead either
                // way).
                self.reader
                    .by_ref()
                    .take((MAX_RESPONSE_BODY_BYTES + 1) as u64)
                    .read_to_end(&mut data)
                    .map_err(HttpError::Io)?;
                if data.len() > MAX_RESPONSE_BODY_BYTES {
                    self.reusable = false;
                    return Err(HttpError::Malformed(format!(
                        "close-delimited body exceeds {MAX_RESPONSE_BODY_BYTES}-byte response body cap"
                    )));
                }
                self.reusable = false;
                return Ok((status, data));
            }
        }
        // No length, no chunking, keep-alive: the body is empty by
        // the only reading HTTP/1.1 allows here.
        if close_after {
            self.reusable = false;
        }
        Ok((status, data))
    }

    /// Read one response head: the status line and the header
    /// block. Returns the status, the validated Content-Length
    /// (if any), whether the body is chunked, and whether the
    /// peer sent `Connection: close`. Zero bytes before the
    /// status line means the pooled connection was stale.
    fn read_response_head(&mut self) -> Result<(u16, Option<usize>, bool, bool), HttpError> {
        // Status line. Zero bytes read = stale pooled connection.
        let mut status_line = String::new();
        let n = self.read_line_bounded(&mut status_line)?;
        if n == 0 {
            self.reusable = false;
            return Err(HttpError::Stale);
        }
        let status: u16 = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .ok_or_else(|| {
                self.reusable = false;
                HttpError::Malformed(format!("status line {status_line:?}"))
            })?;

        // Headers.
        let mut content_length: Option<usize> = None;
        let mut chunked = false;
        let mut close_after = false;
        let mut header_lines = 0usize;
        loop {
            let mut line = String::new();
            let n = self.read_line_bounded(&mut line)?;
            if n == 0 {
                self.reusable = false;
                return Err(HttpError::Malformed("EOF inside headers".into()));
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            header_lines += 1;
            if header_lines > MAX_HEADER_LINES {
                self.reusable = false;
                return Err(HttpError::Malformed(format!(
                    "more than {MAX_HEADER_LINES} header lines"
                )));
            }
            let Some((name, value)) = line.split_once(':') else {
                self.reusable = false;
                return Err(HttpError::Malformed(format!("header line {line:?}")));
            };
            let value = value.trim();
            match name.trim().to_ascii_lowercase().as_str() {
                "content-length" => {
                    // Strict: an unparsable value is NOT "no body"
                    // (that reading desyncs the pooled connection),
                    // duplicates must agree, and the declared size
                    // is capped before it is ever allocated for.
                    let parsed: usize = value.parse().map_err(|_| {
                        self.reusable = false;
                        HttpError::Malformed(format!("Content-Length {value:?}"))
                    })?;
                    if parsed > MAX_RESPONSE_BODY_BYTES {
                        self.reusable = false;
                        return Err(HttpError::Malformed(format!(
                            "Content-Length {parsed} exceeds {MAX_RESPONSE_BODY_BYTES}-byte response body cap"
                        )));
                    }
                    if let Some(prev) = content_length {
                        if prev != parsed {
                            self.reusable = false;
                            return Err(HttpError::Malformed(format!(
                                "conflicting Content-Length headers {prev} and {parsed}"
                            )));
                        }
                    }
                    content_length = Some(parsed);
                }
                "transfer-encoding" => chunked = value.to_ascii_lowercase().contains("chunked"),
                "connection" => {
                    close_after = value.to_ascii_lowercase().contains("close");
                }
                _ => {}
            }
        }
        Ok((status, content_length, chunked, close_after))
    }

    fn read_chunked(&mut self, out: &mut Vec<u8>) -> Result<(), HttpError> {
        loop {
            let mut size_line = String::new();
            let n = self.read_line_bounded(&mut size_line)?;
            if n == 0 {
                self.reusable = false;
                return Err(HttpError::Malformed("EOF inside chunked body".into()));
            }
            let size_str = size_line.trim().split(';').next().unwrap_or("");
            let size = usize::from_str_radix(size_str, 16).map_err(|_| {
                self.reusable = false;
                HttpError::Malformed(format!("chunk size {size_line:?}"))
            })?;
            if size == 0 {
                // Trailer part: header lines until the blank line.
                let mut trailer_lines = 0usize;
                loop {
                    let mut trailer = String::new();
                    let n = self.read_line_bounded(&mut trailer)?;
                    if n == 0 || trailer.trim_end().is_empty() {
                        break;
                    }
                    trailer_lines += 1;
                    if trailer_lines > MAX_TRAILER_LINES {
                        self.reusable = false;
                        return Err(HttpError::Malformed(format!(
                            "more than {MAX_TRAILER_LINES} trailer lines"
                        )));
                    }
                }
                return Ok(());
            }
            // Cap each chunk AND the cumulative body before
            // allocating anything sized by the declaration.
            match out.len().checked_add(size) {
                Some(total) if total <= MAX_RESPONSE_BODY_BYTES => {}
                _ => {
                    self.reusable = false;
                    return Err(HttpError::Malformed(format!(
                        "chunked body exceeds {MAX_RESPONSE_BODY_BYTES}-byte response body cap"
                    )));
                }
            }
            let start = out.len();
            out.resize(start + size, 0);
            self.reader.read_exact(&mut out[start..]).map_err(|e| {
                self.reusable = false;
                HttpError::Io(e)
            })?;
            // Trailing CRLF after each chunk.
            let mut crlf = [0u8; 2];
            self.reader.read_exact(&mut crlf).map_err(|e| {
                self.reusable = false;
                HttpError::Io(e)
            })?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    // The reader machinery is exercised against canned responses by
    // way of a loopback pair; the error-path pins live here, next to
    // the reader they pin (#5579).

    #[test]
    fn stale_is_distinct_from_malformed() {
        assert!(matches!(HttpError::Stale, HttpError::Stale));
        let m = HttpError::Malformed("x".into());
        assert!(m.to_string().contains("malformed"));
    }

    /// An HttpConn whose peer answers each request (head read up to
    /// the blank line) with the next canned response, in order.
    fn canned_conn(responses: Vec<Vec<u8>>) -> (HttpConn, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let stream = TcpStream::connect(addr).unwrap();
        let (mut peer, _) = listener.accept().unwrap();
        let server = std::thread::spawn(move || {
            for response in &responses {
                let mut buf = [0u8; 1024];
                let mut seen = Vec::new();
                while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                    match peer.read(&mut buf) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => seen.extend_from_slice(&buf[..n]),
                    }
                }
                if peer.write_all(response).is_err() {
                    return;
                }
                let _ = peer.flush();
            }
        });
        (HttpConn::new(stream).unwrap(), server)
    }

    fn get(conn: &mut HttpConn) -> Result<(u16, Vec<u8>), HttpError> {
        conn.request("GET", "/x", "127.0.0.1", "token", None)
    }

    #[test]
    fn crlf_in_path_is_malformed_before_any_write() {
        // #5750: the crafted path that smuggled a second request
        // in the probe must now fail at validation; the canned
        // peer never sees a request (no response is queued).
        let (mut conn, _server) = canned_conn(vec![]);
        let err = conn
            .request(
                "GET",
                "/health\r\nX-Injected: yes\r\nContent-Length: 0\r\n\r\nGET /admin HTTP/1.1",
                "127.0.0.1",
                "token",
                None,
            )
            .unwrap_err();
        assert!(matches!(err, HttpError::Malformed(_)), "{err}");
        assert!(!conn.reusable());
    }

    #[test]
    fn crlf_in_method_and_space_in_path_are_malformed() {
        let (mut conn, _server) = canned_conn(vec![]);
        let err = conn
            .request("GET\r\nX-Injected: 1", "/x", "127.0.0.1", "token", None)
            .unwrap_err();
        assert!(matches!(err, HttpError::Malformed(_)), "{err}");
        let (mut conn, _server) = canned_conn(vec![]);
        let err = conn
            .request("GET", "/x y", "127.0.0.1", "token", None)
            .unwrap_err();
        assert!(matches!(err, HttpError::Malformed(_)), "{err}");
        let (mut conn, _server) = canned_conn(vec![]);
        let err = conn
            .request("GET", "", "127.0.0.1", "token", None)
            .unwrap_err();
        assert!(matches!(err, HttpError::Malformed(_)), "{err}");
    }

    #[test]
    fn method_and_path_validators_accept_ordinary_traffic() {
        assert!(valid_method("GET"));
        assert!(valid_method("PROPFIND"));
        assert!(!valid_method(""));
        assert!(!valid_method("GE T"));
        assert!(valid_path("/health"));
        assert!(valid_path("/queue/list?state=queued&n=5"));
        assert!(valid_path("/files/a%20b"));
        assert!(!valid_path(""));
        assert!(!valid_path("health"));
        assert!(!valid_path("/x\u{7f}"));
    }

    #[test]
    fn well_formed_content_length_round_trips_and_stays_reusable() {
        let (mut conn, server) = canned_conn(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n[]".to_vec(),
        ]);
        let (status, body) = get(&mut conn).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, b"{}");
        assert!(conn.reusable());
        // A second exchange on the same pooled connection parses
        // cleanly — the first body was consumed exactly.
        let (status, body) = get(&mut conn).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, b"[]");
        assert!(conn.reusable());
        server.join().unwrap();
    }

    #[test]
    fn well_formed_chunked_round_trips_and_stays_reusable() {
        let (mut conn, server) = canned_conn(vec![
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n"
                .to_vec(),
        ]);
        let (status, body) = get(&mut conn).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, b"hello");
        assert!(conn.reusable());
        server.join().unwrap();
    }

    #[test]
    fn single_interim_head_then_final_response_succeeds() {
        // Control for the 1xx bound (#6509): one interim head is
        // ordinary traffic and the final response parses normally.
        let (mut conn, server) = canned_conn(vec![
            b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
        ]);
        let (status, body) = get(&mut conn).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, b"{}");
        assert!(conn.reusable());
        server.join().unwrap();
    }

    #[test]
    fn interim_head_flood_is_malformed_and_not_reusable() {
        // #6509: an endless stream of individually legal 1xx heads
        // must fail at the count bound, not hang the exchange.
        let mut flood = Vec::new();
        for _ in 0..(MAX_INTERIM_HEADS + 5) {
            flood.extend_from_slice(b"HTTP/1.1 100 Continue\r\n\r\n");
        }
        flood.extend_from_slice(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        let (mut conn, _server) = canned_conn(vec![flood]);
        let err = get(&mut conn).unwrap_err();
        assert!(matches!(err, HttpError::Malformed(_)), "{err}");
        assert!(!conn.reusable());
    }

    #[test]
    fn header_line_flood_is_malformed_and_not_reusable() {
        // #6509: header lines are each under MAX_LINE_BYTES; only
        // the count bound stops the flood.
        let mut flood = Vec::new();
        flood.extend_from_slice(b"HTTP/1.1 200 OK\r\n");
        for i in 0..(MAX_HEADER_LINES + 5) {
            flood.extend_from_slice(format!("X-Flood-{i}: v\r\n").as_bytes());
        }
        flood.extend_from_slice(b"Content-Length: 0\r\n\r\n");
        let (mut conn, _server) = canned_conn(vec![flood]);
        let err = get(&mut conn).unwrap_err();
        assert!(matches!(err, HttpError::Malformed(_)), "{err}");
        assert!(!conn.reusable());
    }

    #[test]
    fn chunked_with_trailers_round_trips_and_stays_reusable() {
        // Control for the trailer bound (#6509): a couple of
        // trailer lines after the final chunk are ordinary.
        let (mut conn, server) = canned_conn(vec![
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\nX-Checksum: abc\r\nX-Trailer: yes\r\n\r\n"
                .to_vec(),
        ]);
        let (status, body) = get(&mut conn).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, b"hello");
        assert!(conn.reusable());
        server.join().unwrap();
    }

    #[test]
    fn trailer_line_flood_is_malformed_and_not_reusable() {
        // #6509: the trailer loop after the final chunk gets the
        // same count bound as the header loop.
        let mut flood = Vec::new();
        flood.extend_from_slice(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1\r\na\r\n0\r\n",
        );
        for i in 0..(MAX_TRAILER_LINES + 5) {
            flood.extend_from_slice(format!("X-Flood-{i}: v\r\n").as_bytes());
        }
        flood.extend_from_slice(b"\r\n");
        let (mut conn, _server) = canned_conn(vec![flood]);
        let err = get(&mut conn).unwrap_err();
        assert!(matches!(err, HttpError::Malformed(_)), "{err}");
        assert!(!conn.reusable());
    }

    #[test]
    fn agreeing_duplicate_content_length_is_accepted() {
        let (mut conn, server) = canned_conn(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
        ]);
        let (status, body) = get(&mut conn).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, b"{}");
        assert!(conn.reusable());
        server.join().unwrap();
    }

    #[test]
    fn invalid_content_length_is_malformed_and_not_reusable() {
        let (mut conn, _server) = canned_conn(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: abc\r\n\r\nhello".to_vec(),
        ]);
        let err = get(&mut conn).unwrap_err();
        assert!(matches!(err, HttpError::Malformed(_)), "{err}");
        assert!(!conn.reusable());
    }

    #[test]
    fn conflicting_duplicate_content_length_is_malformed_and_not_reusable() {
        let (mut conn, _server) = canned_conn(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Length: 0\r\n\r\nhello".to_vec(),
        ]);
        let err = get(&mut conn).unwrap_err();
        assert!(matches!(err, HttpError::Malformed(_)), "{err}");
        assert!(!conn.reusable());
    }

    #[test]
    fn oversize_declared_content_length_errors_before_the_body() {
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
            MAX_RESPONSE_BODY_BYTES + 1
        );
        let (mut conn, _server) = canned_conn(vec![response.into_bytes()]);
        // No body bytes are ever sent: the error must come from the
        // header alone, before any allocation sized by it.
        let err = get(&mut conn).unwrap_err();
        assert!(matches!(err, HttpError::Malformed(_)), "{err}");
        assert!(!conn.reusable());
    }

    #[test]
    fn oversize_chunk_errors_before_the_chunk_body() {
        let response = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n",
            MAX_RESPONSE_BODY_BYTES + 1
        );
        let (mut conn, _server) = canned_conn(vec![response.into_bytes()]);
        let err = get(&mut conn).unwrap_err();
        assert!(matches!(err, HttpError::Malformed(_)), "{err}");
        assert!(!conn.reusable());
    }

    #[test]
    fn cumulative_chunked_body_over_the_cap_errors() {
        // First chunk is small and fully sent; the second chunk is
        // exactly at the cap on its own, so only the cumulative
        // check can reject it — before its body is read or sized.
        let mut response =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n".to_vec();
        response.extend_from_slice(format!("{:x}\r\n", MAX_RESPONSE_BODY_BYTES).as_bytes());
        let (mut conn, _server) = canned_conn(vec![response]);
        let err = get(&mut conn).unwrap_err();
        assert!(matches!(err, HttpError::Malformed(_)), "{err}");
        assert!(!conn.reusable());
    }

    #[test]
    fn overlong_status_line_is_malformed_and_not_reusable() {
        let mut response = b"HTTP/1.1 200 OK ".to_vec();
        response.extend_from_slice("x".repeat(MAX_LINE_BYTES + 100).as_bytes());
        response.extend_from_slice(b"\r\n\r\n");
        let (mut conn, _server) = canned_conn(vec![response]);
        let err = get(&mut conn).unwrap_err();
        assert!(matches!(err, HttpError::Malformed(_)), "{err}");
        assert!(!conn.reusable());
    }
    #[test]
    fn head_response_with_content_length_is_bodyless_and_stays_reusable() {
        // #5921: a HEAD response carries the Content-Length of the
        // GET it mirrors, but no body bytes ever follow. The reader
        // must not wait for them, and the pooled connection must
        // parse the next exchange cleanly.
        let (mut conn, server) = canned_conn(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
        ]);
        let (status, body) = conn
            .request("HEAD", "/x", "127.0.0.1", "token", None)
            .unwrap();
        assert_eq!(status, 200);
        assert!(body.is_empty());
        assert!(conn.reusable());
        let (status, body) = get(&mut conn).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, b"{}");
        assert!(conn.reusable());
        server.join().unwrap();
    }

    #[test]
    fn no_content_204_with_content_length_is_bodyless_and_stays_reusable() {
        // #5921: a 204 never carries a body, whatever its headers
        // claim; the next pooled exchange must parse cleanly.
        let (mut conn, server) = canned_conn(vec![
            b"HTTP/1.1 204 No Content\r\nContent-Length: 5\r\n\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
        ]);
        let (status, body) = get(&mut conn).unwrap();
        assert_eq!(status, 204);
        assert!(body.is_empty());
        assert!(conn.reusable());
        let (status, body) = get(&mut conn).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, b"{}");
        assert!(conn.reusable());
        server.join().unwrap();
    }

    #[test]
    fn not_modified_304_with_cached_content_length_is_bodyless() {
        // #5921: a 304 may legitimately carry the Content-Length
        // of the cached 200 response (RFC 9110 section 15.4.5) and
        // still has no body of its own.
        let (mut conn, server) = canned_conn(vec![
            b"HTTP/1.1 304 Not Modified\r\nContent-Length: 5\r\n\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
        ]);
        let (status, body) = get(&mut conn).unwrap();
        assert_eq!(status, 304);
        assert!(body.is_empty());
        assert!(conn.reusable());
        let (status, body) = get(&mut conn).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, b"{}");
        assert!(conn.reusable());
        server.join().unwrap();
    }

    #[test]
    fn no_body_status_with_chunked_transfer_encoding_is_bodyless() {
        // #5921: Transfer-Encoding must not resurrect a body on a
        // no-body status either — no chunks follow a 204.
        let (mut conn, server) = canned_conn(vec![
            b"HTTP/1.1 204 No Content\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
        ]);
        let (status, body) = get(&mut conn).unwrap();
        assert_eq!(status, 204);
        assert!(body.is_empty());
        assert!(conn.reusable());
        let (status, body) = get(&mut conn).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, b"{}");
        assert!(conn.reusable());
        server.join().unwrap();
    }

    #[test]
    fn interim_103_is_consumed_and_the_final_response_is_returned() {
        // #5921: a 1xx interim response is not the final answer.
        // Returning it would leave the real response in the stream
        // to desync the next exchange on the pooled connection.
        let (mut conn, server) = canned_conn(vec![
            b"HTTP/1.1 103 Early Hints\r\nLink: </style.css>; rel=preload\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello"
                .to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
        ]);
        let (status, body) = get(&mut conn).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, b"hello");
        assert!(conn.reusable());
        // Nothing of the final response remains in the stream:
        // the next pooled exchange parses cleanly.
        let (status, body) = get(&mut conn).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, b"{}");
        assert!(conn.reusable());
        server.join().unwrap();
    }
    #[test]
    fn lowercase_head_is_a_different_method_and_reads_its_body() {
        // HTTP methods are case-sensitive: only an actual HEAD
        // suppresses the body read. A lowercase `head` request is
        // an ordinary method token whose response body must be
        // consumed, or it desyncs the pooled connection.
        let (mut conn, server) = canned_conn(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_vec(),
        ]);
        let (status, body) = conn
            .request("head", "/x", "127.0.0.1", "token", None)
            .unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, b"hello");
        assert!(conn.reusable());
        server.join().unwrap();
    }

    #[test]
    fn unix_socket_round_trips_and_stays_reusable() {
        // #6219: an HttpConn over a UnixStream speaks the same
        // HTTP exchange as over TCP (the colocated dial path).
        use std::os::unix::net::UnixListener;
        let dir =
            std::env::temp_dir().join(format!("fleet-proxy-http-uds-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("svc.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let stream = UnixStream::connect(&path).unwrap();
        let (mut peer, _) = listener.accept().unwrap();
        let server = std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            let mut seen = Vec::new();
            while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                match peer.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => seen.extend_from_slice(&buf[..n]),
                }
            }
            let _ = peer.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}");
            let _ = peer.flush();
        });
        let mut conn = HttpConn::from_unix(stream).unwrap();
        let (status, body) = get(&mut conn).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, b"{}");
        assert!(conn.reusable());
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
