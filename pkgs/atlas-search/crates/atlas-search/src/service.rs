//! The via-service client (design sections 1-3): *prefer
//! service, fall back to standalone loudly*.
//!
//! Step 4 (#5354) replaces step 3's probe with the real job
//! protocol client against atlas-proto: submit → poll status →
//! page the immutable ranking until the requested result count is
//! satisfied (or the ranking ends). The fallback contract around
//! it is unchanged from step 3: any failure — unreachable, an HTTP
//! error, a failed/expired job, a malformed response — is reported
//! by the caller on stderr and recorded in the envelope's notes,
//! and standalone serves the query. A successful service search
//! returns the service's envelope with paged results spliced in,
//! field-for-field what standalone renders for the same request.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use atlas_proto::{JobCreated, JobStatus, JobStatusResponse, SearchPage, SearchRequest};
use serde_json::Value as Json;

/// The default service base URL (loopback bind per design
/// section 3; the port is proposed there pending the packaging
/// step's port-inventory check, #5355).
pub const DEFAULT_SERVICE_URL: &str = "http://127.0.0.1:18760";

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const CLIENT_DEADLINE: Duration = Duration::from_secs(330);
const PAGE_LIMIT: usize = 50;

/// Resolve the base URL: flag, then `ATLAS_SEARCH_URL`, then the
/// default.
pub fn base_url(flag: Option<&str>) -> String {
    if let Some(u) = flag {
        return u.trim_end_matches('/').to_string();
    }
    if let Ok(u) = std::env::var("ATLAS_SEARCH_URL")
        && !u.is_empty()
    {
        return u.trim_end_matches('/').to_string();
    }
    DEFAULT_SERVICE_URL.to_string()
}

/// Whether via-service was requested by env (`ATLAS_SEARCH_VIA=service`)
/// when the flag was not passed.
pub fn via_env() -> bool {
    std::env::var("ATLAS_SEARCH_VIA").is_ok_and(|v| v == "service")
}

fn home_dir() -> PathBuf {
    // Home-resolution contract (issues #5640 / #5762): a missing or
    // empty HOME is an error naming HOME, never a substitute
    // directory - the old fallback to a literal "~" silently
    // resolved state/config/token paths relative to the CWD.
    match std::env::var_os("HOME") {
        Some(home) if !home.is_empty() => PathBuf::from(home),
        _ => panic!("HOME is not set or is empty: atlas-search requires a non-empty HOME to resolve its state/config paths (issue #5640)"),
    }
}

/// Resolve this client's bearer token. The value is a secret: it
/// is read from the environment or the 0600 client config and is
/// never printed, logged, or included in an error string.
///
/// Order: `ATLAS_SEARCH_TOKEN`, then the JSON file at
/// `ATLAS_SEARCH_CLIENT_CONFIG`, then
/// `~/.config/atlas-search/client.json` (`{"token": "..."}`).
pub fn client_token() -> Result<String, String> {
    if let Ok(t) = std::env::var("ATLAS_SEARCH_TOKEN")
        && !t.is_empty()
    {
        return Ok(t);
    }
    let path = match std::env::var("ATLAS_SEARCH_CLIENT_CONFIG") {
        Ok(p) if !p.is_empty() => PathBuf::from(p),
        _ => home_dir().join(".config/atlas-search/client.json"),
    };
    let text = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "no service client token (set ATLAS_SEARCH_TOKEN or create {}): {e}",
            path.display()
        )
    })?;
    let doc: Json = serde_json::from_str(&text)
        .map_err(|e| format!("bad client config {}: {e}", path.display()))?;
    doc.get("token")
        .and_then(Json::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("client config {} has no \"token\" string", path.display()))
}

fn agent() -> ureq::Agent {
    agent_with_proxy(ureq::Proxy::try_from_env())
}

/// Build the via-service agent. `proxy` comes from the process
/// environment in production (`Proxy::try_from_env`: ALL_PROXY /
/// HTTPS_PROXY / http_proxy and friends, with NO_PROXY bypasses),
/// so hosts that can only reach the tailnet through an HTTP CONNECT
/// proxy (sandbox, codespaces, CI) can use the service path instead
/// of always falling back to standalone (#5597). With no proxy set
/// the agent connects directly, exactly as before. Tests inject an
/// explicit proxy to run against a stub CONNECT proxy.
fn agent_with_proxy(proxy: Option<ureq::Proxy>) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(CLIENT_DEADLINE))
        .timeout_connect(Some(Duration::from_secs(5)))
        .proxy(proxy)
        .http_status_as_error(false)
        .build()
        .into()
}

fn error_detail(status: u16, body: &str) -> String {
    if let Ok(doc) = serde_json::from_str::<Json>(body)
        && let Some(err) = doc.get("error")
    {
        let code = err.get("code").and_then(Json::as_str).unwrap_or("error");
        let message = err.get("message").and_then(Json::as_str).unwrap_or("");
        return format!("HTTP {status} ({code}: {message})");
    }
    let snippet: String = body.chars().take(160).collect();
    format!("HTTP {status}: {snippet}")
}

fn read_body(resp: &mut ureq::http::Response<ureq::Body>) -> Result<String, String> {
    resp.body_mut()
        .read_to_string()
        .map_err(|e| format!("cannot read service response: {e}"))
}

/// Run one search through the service. On success returns the
/// reconstructed envelope; on any failure returns a one-clause
/// detail for the caller's loud fallback.
pub fn search(base: &str, request: &SearchRequest) -> Result<Json, String> {
    let token = client_token()?;
    search_with_token(base, request, &token)
}

fn search_with_token(base: &str, request: &SearchRequest, token: &str) -> Result<Json, String> {
    search_with_agent(base, request, token, agent())
}

fn search_with_agent(
    base: &str,
    request: &SearchRequest,
    token: &str,
    agent: ureq::Agent,
) -> Result<Json, String> {
    let auth = format!("Bearer {token}");

    // Submit.
    let body = serde_json::to_string(request)
        .map_err(|e| format!("cannot serialize the search request: {e}"))?;
    let mut resp = agent
        .post(format!("{base}/v1/search/jobs"))
        .header("Authorization", &auth)
        .header("Content-Type", "application/json")
        .send(body.as_str())
        .map_err(|e| format!("service at {base} unreachable ({e})"))?;
    let status = resp.status().as_u16();
    let body = read_body(&mut resp)?;
    if status != 202 {
        return Err(format!(
            "service at {base} rejected the job submit: {}",
            error_detail(status, &body)
        ));
    }
    let created: JobCreated = serde_json::from_str(&body)
        .map_err(|e| format!("service at {base} sent a malformed job response: {e}"))?;
    let job_url = format!("{base}/v1/search/jobs/{}", created.job_id);

    // Poll until the job completes. The job capability token is
    // sufficient for status and results; use it rather than the
    // client credential where the protocol allows either.
    let job_auth = format!("Bearer {}", created.job_token);
    let deadline = Instant::now() + CLIENT_DEADLINE;
    loop {
        let mut resp = agent
            .get(&job_url)
            .header("Authorization", &job_auth)
            .call()
            .map_err(|e| format!("service at {base} unreachable while polling ({e})"))?;
        let status = resp.status().as_u16();
        let body = read_body(&mut resp)?;
        if status != 200 {
            return Err(format!(
                "service at {base} lost the job: {}",
                error_detail(status, &body)
            ));
        }
        let job: JobStatusResponse = serde_json::from_str(&body)
            .map_err(|e| format!("service at {base} sent malformed job status: {e}"))?;
        match job.status {
            JobStatus::Done => break,
            JobStatus::Failed => {
                return Err(format!(
                    "service job failed: {}",
                    job.error.unwrap_or_else(|| "unknown error".to_string())
                ));
            }
            JobStatus::Expired => {
                return Err("service job expired before completion".to_string());
            }
            JobStatus::Queued | JobStatus::Running => {
                if Instant::now() >= deadline {
                    return Err("service job did not complete within 330 seconds".to_string());
                }
                std::thread::sleep(POLL_INTERVAL);
            }
        }
    }

    // Page the immutable ranking until the requested result count
    // is satisfied or the ranking ends.
    let mut envelope: Option<Json> = None;
    let mut results: Vec<Json> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut seen_cursors: std::collections::HashSet<String> = std::collections::HashSet::new();
    loop {
        if Instant::now() >= deadline {
            return Err("service job results did not finish paging within 330 seconds".to_string());
        }
        let mut call = agent
            .get(format!("{job_url}/results"))
            .header("Authorization", &job_auth)
            .query("limit", PAGE_LIMIT.to_string());
        if let Some(c) = &cursor {
            call = call.query("cursor", c.clone());
        }
        let mut resp = call
            .call()
            .map_err(|e| format!("service at {base} unreachable while paging ({e})"))?;
        let status = resp.status().as_u16();
        let body = read_body(&mut resp)?;
        if status != 200 {
            return Err(format!(
                "service at {base} failed a results page: {}",
                error_detail(status, &body)
            ));
        }
        let page: SearchPage = serde_json::from_str(&body)
            .map_err(|e| format!("service at {base} sent a malformed results page: {e}"))?;
        if page.job_id != created.job_id {
            return Err("service returned a page for a different job".to_string());
        }
        if envelope.is_none() {
            envelope = Some(page.envelope.clone());
        }
        let page_len = page.results.len();
        results.extend(page.results.iter().cloned());
        if results.len() >= request.k || page.next_cursor.is_none() {
            break;
        }
        // Another iteration would follow `next_cursor`: it must
        // actually advance the ranking, or this loop never ends
        // (issue #5424). A non-advancing cursor, or an empty page
        // that still claims more results, is a malformed response:
        // return Err so the caller falls back to standalone loudly.
        let next_cursor = page.next_cursor.clone().unwrap();
        if page_len == 0 {
            return Err(format!(
                "service at {base} sent a malformed results page: empty page still returns a next_cursor"
            ));
        }
        if cursor.as_deref() == Some(next_cursor.as_str()) {
            return Err(format!(
                "service at {base} sent a malformed results page: next_cursor did not advance (repeats the cursor just used)"
            ));
        }
        // A cycle with period >= 2 (A -> B -> A) passes the immediate-repeat
        // check above on every page: the ranking still never advances, so
        // any cursor already followed in this session is equally malformed
        // (#5654).
        if !seen_cursors.insert(next_cursor.clone()) {
            return Err(format!(
                "service at {base} sent a malformed results page: next_cursor did not advance (repeats a cursor already seen in this paging session)"
            ));
        }
        cursor = Some(next_cursor);
    }
    let mut out = envelope.ok_or("service returned no results envelope")?;
    if let Some(obj) = out.as_object_mut() {
        obj.insert("results".to_string(), Json::Array(results));
    }
    Ok(out)
}

/// The job protocol scopes get-chunk to an existing job, so a
/// standalone get-chunk invocation has no job to page from. Probe
/// the service only to shape the fallback detail honestly: the
/// caller warns loudly and serves standalone either way.
pub fn get_chunk_fallback_detail(base: &str) -> String {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(2)))
        .timeout_connect(Some(Duration::from_secs(1)))
        .proxy(ureq::Proxy::try_from_env())
        .http_status_as_error(false)
        .build()
        .into();
    match agent.get(format!("{base}/health")).call() {
        Ok(resp) if resp.status().as_u16() == 200 => format!(
            "service at {base} is up, but get-chunk is job-scoped in the protocol and this invocation has no job"
        ),
        Ok(resp) => format!(
            "service at {base} answered HTTP {} on /health; get-chunk served standalone",
            resp.status().as_u16()
        ),
        Err(e) => format!("service at {base} unreachable ({e})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    #[derive(Clone, Copy, PartialEq)]
    enum StubMode {
        CyclingCursor,
        RepeatingCursor,
        EmptyPage,
        HappyPath,
    }

    fn respond(stream: &mut TcpStream, status: &str, body: &str) {
        let resp = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(resp.as_bytes());
    }

    fn read_request(stream: &mut TcpStream) -> String {
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = stream.read(&mut chunk).unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..pos]).to_string();
                let mut content_length = 0usize;
                for line in head.lines() {
                    if let Some((k, v)) = line.split_once(':')
                        && k.trim().eq_ignore_ascii_case("content-length")
                    {
                        content_length = v.trim().parse().unwrap_or(0);
                    }
                }
                if buf.len() >= pos + 4 + content_length {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&buf).to_string()
    }

    /// A stub atlas service: submit -> 202, status -> done, and a
    /// results endpoint whose paging behaviour is set by `mode`.
    fn start_stub(mode: StubMode) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let req = read_request(&mut stream);
                let first = req.lines().next().unwrap_or("").to_string();
                if first.starts_with("POST /v1/search/jobs") {
                    respond(
                        &mut stream,
                        "202 Accepted",
                        r#"{"job_id":"job-1","job_token":"job-token","status":"queued","effective_ceiling":"personal","queue_depth":0,"submitted_at_ms":0,"expires_at_ms":999999999}"#,
                    );
                } else if first.contains("/results") {
                    let has_cursor = first.contains("cursor=");
                    let (results, next): (&str, &str) = match mode {
                        StubMode::CyclingCursor => {
                            if first.contains("cursor=cursor-B") {
                                (r#"[{"id":1}]"#, r#""cursor-A""#)
                            } else {
                                (r#"[{"id":1}]"#, r#""cursor-B""#)
                            }
                        }
                        StubMode::RepeatingCursor => (r#"[{"id":1}]"#, r#""same-cursor""#),
                        StubMode::EmptyPage => (r#"[]"#, r#""some-cursor""#),
                        StubMode::HappyPath => {
                            if has_cursor {
                                (r#"[{"id":2}]"#, "null")
                            } else {
                                (r#"[{"id":1}]"#, r#""cursor-2""#)
                            }
                        }
                    };
                    let body = format!(
                        r#"{{"job_id":"job-1","status":"done","effective_ceiling":"personal","envelope":{{"results":[]}},"results":{results},"next_cursor":{next},"total_results":99}}"#
                    );
                    respond(&mut stream, "200 OK", &body);
                } else if first.starts_with("GET /v1/search/jobs/") {
                    respond(
                        &mut stream,
                        "200 OK",
                        r#"{"job_id":"job-1","status":"done","branch_states":{"fts":"done","vector":"done"},"effective_ceiling":"personal","pool":100,"notes":[],"submitted_at_ms":0,"expires_at_ms":999999999}"#,
                    );
                } else {
                    respond(
                        &mut stream,
                        "404 Not Found",
                        r#"{"error":{"code":"not_found","message":"no such route"}}"#,
                    );
                }
            }
        });
        format!("http://{addr}")
    }

    fn request_k(k: usize) -> SearchRequest {
        let mut req = SearchRequest::new("stub query");
        req.k = k;
        req
    }

    #[test]
    fn search_repeating_cursor_returns_err_promptly() {
        let base = start_stub(StubMode::RepeatingCursor);
        let started = Instant::now();
        let err = search_with_token(&base, &request_k(10), "test-token").unwrap_err();
        assert!(err.contains("did not advance"), "unexpected error: {err}");
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "repeating cursor was not rejected promptly"
        );
    }

    #[test]
    fn search_cycling_cursor_returns_err_promptly() {
        // #5654: a period-2 cycle (A -> B -> A) passes the immediate-repeat
        // check on every page; the seen-cursor set must reject it promptly.
        let base = start_stub(StubMode::CyclingCursor);
        let started = Instant::now();
        let err = search_with_token(&base, &request_k(100_000), "test-token").unwrap_err();
        assert!(err.contains("already seen"), "unexpected error: {err}");
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "cycling cursor was not rejected promptly"
        );
    }

    #[test]
    fn search_empty_page_with_cursor_returns_err() {
        let base = start_stub(StubMode::EmptyPage);
        let err = search_with_token(&base, &request_k(10), "test-token").unwrap_err();
        assert!(err.contains("empty page"), "unexpected error: {err}");
    }

    #[test]
    fn search_multi_page_happy_path_unchanged() {
        let base = start_stub(StubMode::HappyPath);
        let out = search_with_token(&base, &request_k(10), "test-token").unwrap();
        let results = out
            .get("results")
            .and_then(Json::as_array)
            .expect("envelope carries a results array");
        assert_eq!(results.len(), 2);
    }

    /// A stub HTTP CONNECT proxy: records every CONNECT target it
    /// is asked for (the test's oracle), answers 200, then relays
    /// bytes between the client and the real target both ways.
    fn start_connect_proxy() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::sync::{Arc, Mutex};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let connects: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&connects);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut client) = stream else { continue };
                let recorded = Arc::clone(&recorded);
                thread::spawn(move || {
                    // Read the CONNECT request head.
                    let mut head: Vec<u8> = Vec::new();
                    let mut chunk = [0u8; 1024];
                    loop {
                        let n = client.read(&mut chunk).unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        head.extend_from_slice(&chunk[..n]);
                        if head.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let head = String::from_utf8_lossy(&head).to_string();
                    let target = head
                        .lines()
                        .next()
                        .and_then(|l| l.strip_prefix("CONNECT "))
                        .and_then(|l| l.split_whitespace().next())
                        .unwrap_or("")
                        .to_string();
                    recorded.lock().unwrap().push(target.clone());
                    let Ok(mut upstream) = TcpStream::connect(&target) else {
                        let _ = client
                            .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n");
                        return;
                    };
                    let _ = client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n");
                    let _ = client.set_nodelay(true);
                    let _ = upstream.set_nodelay(true);
                    let mut up_read = upstream.try_clone().unwrap();
                    let mut client_read = client.try_clone().unwrap();
                    let relay = thread::spawn(move || {
                        let _ = std::io::copy(&mut up_read, &mut client);
                    });
                    let _ = std::io::copy(&mut client_read, &mut upstream);
                    let _ = relay.join();
                });
            }
        });
        (format!("http://{addr}"), connects)
    }

    #[test]
    fn search_via_connect_proxy_tunnels_to_service() {
        // Oracle: the stub proxy's CONNECT log. If the client ignored
        // the configured proxy and connected directly, the search
        // would still succeed (the stub service is on loopback) but
        // no CONNECT would ever be recorded.
        let base = start_stub(StubMode::HappyPath);
        let (proxy_url, connects) = start_connect_proxy();
        let proxy = ureq::Proxy::new(&proxy_url).unwrap();
        let agent = agent_with_proxy(Some(proxy));
        let out = search_with_agent(&base, &request_k(10), "test-token", agent).unwrap();
        let results = out
            .get("results")
            .and_then(Json::as_array)
            .expect("envelope carries a results array");
        assert_eq!(results.len(), 2);
        let authority = base.strip_prefix("http://").unwrap();
        let recorded = connects.lock().unwrap();
        assert!(
            !recorded.is_empty() && recorded.iter().all(|t| t == authority),
            "expected every request tunnelled via CONNECT to {authority}, got {recorded:?}"
        );
    }

    #[test]
    fn agent_with_no_proxy_connects_directly() {
        // No proxy configured: the pre-#5597 behaviour, unchanged.
        let base = start_stub(StubMode::HappyPath);
        let agent = agent_with_proxy(None);
        let out = search_with_agent(&base, &request_k(10), "test-token", agent).unwrap();
        assert!(out.get("results").and_then(Json::as_array).is_some());
    }
}
