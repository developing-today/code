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

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

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

/// One HTTP connection to a laptop service, over a tunnel stream.
pub struct HttpConn {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    reusable: bool,
}

impl HttpConn {
    pub fn new(stream: TcpStream) -> std::io::Result<HttpConn> {
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

        // Body.
        let mut data = Vec::new();
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
            // Close-delimited body: still peer-sized, so read with
            // the same cap (the connection is dead either way).
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
        // No length, no chunking, keep-alive: the body is empty by
        // the only reading HTTP/1.1 allows here.
        if close_after {
            self.reusable = false;
        }
        Ok((status, data))
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
                loop {
                    let mut trailer = String::new();
                    let n = self.read_line_bounded(&mut trailer)?;
                    if n == 0 || trailer.trim_end().is_empty() {
                        break;
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
}
