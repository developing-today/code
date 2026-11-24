//! Minimal HTTP/1.1 request reader and response writer for the Atlas search service.
//! Source of the verbatim copy in pkgs/fleet's fleet-search crate,
//! split from server.rs in #12671 to satisfy the production-line
//! limit: mirror every change there. Drift guard:
//! .github/scripts/check-fleet-search-parity.sh.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::{Value as Json, json};

use super::ServerError;

pub(super) struct HttpRequest {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) query: HashMap<String, String>,
    pub(super) headers: HashMap<String, String>,
    pub(super) body: Vec<u8>,
}

impl HttpRequest {
    pub(super) fn bearer(&self) -> Option<&str> {
        let h = self.headers.get("authorization")?;
        h.strip_prefix("Bearer ")
            .or_else(|| h.strip_prefix("bearer "))
    }

    pub(super) fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, ServerError> {
        serde_json::from_slice(&self.body)
            .map_err(|e| ServerError::bad_request(format!("invalid JSON body: {e}")))
    }
}

pub(super) struct HttpResponse {
    pub(super) status: u16,
    pub(super) body: Json,
    pub(super) retry_after_seconds: Option<u64>,
}

impl HttpResponse {
    pub(super) fn json(status: u16, body: Json) -> HttpResponse {
        HttpResponse {
            status,
            body,
            retry_after_seconds: None,
        }
    }

    pub(super) fn from_error(e: &ServerError) -> HttpResponse {
        HttpResponse {
            status: e.status,
            body: serde_json::to_value(&e.body).unwrap_or_else(
                |_| json!({"error": {"code": "internal", "message": "unserializable error"}}),
            ),
            retry_after_seconds: e.body.error.retry_after_seconds,
        }
    }
}

pub(super) fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        410 => "Gone",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        504 => "Gateway Timeout",
        _ => "Response",
    }
}

pub(super) fn url_decode(s: &str) -> String {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub(super) fn read_request(stream: &mut TcpStream) -> Result<Option<HttpRequest>, ServerError> {
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .map_err(|e| ServerError::new(500, "internal", e.to_string()))?;
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let header_end = loop {
        if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
            break pos + 4;
        }
        if buf.len() > 64 * 1024 {
            return Err(ServerError::bad_request("request headers too large"));
        }
        let n = stream
            .read(&mut chunk)
            .map_err(|e| ServerError::new(400, "bad_request", format!("read failed: {e}")))?;
        if n == 0 {
            if buf.is_empty() {
                return Ok(None);
            }
            return Err(ServerError::bad_request("connection closed mid-request"));
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    if method.is_empty() || target.is_empty() {
        return Err(ServerError::bad_request("malformed request line"));
    }
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    if headers
        .get("expect")
        .map(|v| v.to_ascii_lowercase().contains("100-continue"))
        .unwrap_or(false)
    {
        let _ = stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
    }
    let content_length: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if content_length > 4 * 1024 * 1024 {
        return Err(ServerError::bad_request("request body too large"));
    }
    let mut body: Vec<u8> = buf[header_end..].to_vec();
    while body.len() < content_length {
        let n = stream
            .read(&mut chunk)
            .map_err(|e| ServerError::new(400, "bad_request", format!("read failed: {e}")))?;
        if n == 0 {
            return Err(ServerError::bad_request("connection closed mid-body"));
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(content_length);
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => {
            let mut map = HashMap::new();
            for pair in q.split('&').filter(|s| !s.is_empty()) {
                let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                map.insert(url_decode(k), url_decode(v));
            }
            (url_decode(p), map)
        }
        None => (url_decode(&target), HashMap::new()),
    };
    Ok(Some(HttpRequest {
        method,
        path,
        query,
        headers,
        body,
    }))
}

pub(super) fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

pub(super) fn write_response(stream: &mut TcpStream, resp: &HttpResponse) {
    let body = serde_json::to_string(&resp.body).unwrap_or_else(|_| "{}".to_string());
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        resp.status,
        reason(resp.status),
        body.len()
    );
    if resp.status == 401 {
        head.push_str("WWW-Authenticate: Bearer\r\n");
    }
    if let Some(secs) = resp.retry_after_seconds {
        head.push_str(&format!("Retry-After: {secs}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
}
