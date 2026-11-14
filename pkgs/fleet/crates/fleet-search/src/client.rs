//! The via-service client for the Atlas job protocol (fleet step 5,
//! #5433) — a port of atlas-search's own via-service client
//! (pkgs/atlas-search/crates/atlas-search/src/service.rs), so the
//! fleet verb and `atlas-search --via-service` speak to the service
//! identically: submit → poll status → page the immutable ranking
//! until the requested result count is satisfied (or the ranking
//! ends). The cursor guards from code#5424 ride along unchanged: a
//! non-advancing cursor, or an empty page that still claims more
//! results, is a malformed response and an error — never a loop.
//!
//! The fallback contract lives in the caller (lib.rs): any failure
//! here — unreachable, an HTTP error, a failed/expired job, a
//! malformed response — comes back as a one-clause detail string
//! for the loud standalone fallback. Token values are never
//! printed, logged, or included in an error string.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::Value as Json;

use crate::proto::{JobCreated, JobStatus, JobStatusResponse, SearchPage, SearchRequest};

/// The default service base URL (the service binds loopback on the
/// laptop; the sandbox reaches that loopback through the forward
/// described in [`ensure_sandbox_forward`]).
pub const DEFAULT_SERVICE_URL: &str = "http://127.0.0.1:18760";

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const CLIENT_DEADLINE: Duration = Duration::from_secs(330);
const PAGE_LIMIT: usize = 50;

/// Resolve the base URL: flag, then `ATLAS_SEARCH_URL`, then the
/// default — atlas-search's own resolution order.
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
/// when no flag or config says otherwise — atlas-search's rule.
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
        _ => panic!(
            "HOME is not set or is empty: fleet-search requires a non-empty HOME to resolve its state/config paths (issue #5640)"
        ),
    }
}

/// The service config path whose *existence* marks the serving
/// host: `ATLAS_SEARCH_SERVICE_CONFIG`, else
/// `~/.local/state/atlas-search/service.json` (written by
/// `atlas-search init` on the laptop; the sandbox never has one).
/// This is the seat signal for the Q3 host default (laptop seat →
/// standalone, sandbox seat → via-service).
pub fn serving_host_marker() -> PathBuf {
    match std::env::var("ATLAS_SEARCH_SERVICE_CONFIG") {
        Ok(p) if !p.is_empty() => PathBuf::from(p),
        _ => home_dir().join(".local/state/atlas-search/service.json"),
    }
}

/// Resolve this client's bearer token, atlas-search's order:
/// `ATLAS_SEARCH_TOKEN`, then the JSON file at
/// `ATLAS_SEARCH_CLIENT_CONFIG`, then
/// `~/.config/atlas-search/client.json` (`{"token": "..."}`).
/// The value is a secret: read from the environment or the 0600
/// client config, never printed or embedded in an error.
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
    ureq::Agent::config_builder()
        .timeout_global(Some(CLIENT_DEADLINE))
        .timeout_connect(Some(Duration::from_secs(5)))
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

/// A short-timeout unauthenticated health probe (`GET /health`
/// carries no corpus data in the protocol).
pub fn health_ok(base: &str) -> bool {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(2)))
        .timeout_connect(Some(Duration::from_secs(1)))
        .http_status_as_error(false)
        .build()
        .into();
    match agent.get(format!("{base}/health")).call() {
        Ok(resp) => resp.status().as_u16() == 200,
        Err(_) => false,
    }
}

/// The sandbox seat's reachability step, following the deployed
/// atlas-search wrapper's established pattern exactly (code#5356;
/// the via-service client is proxy-blind, #5597 — this mirrors the
/// wrapper's mechanism, it does not invent a second one):
///
/// 1. bootstrap the shared laptop-ssh control connection
///    (`~/tools/laptop-ssh.sh true`, idempotent);
/// 2. ask the control master to forward loopback:<port> to the
///    laptop's service (`ssh -O forward -L ...` — a nonzero exit
///    also covers "already forwarded", so health is the real test);
/// 3. the caller health-checks and proceeds.
///
/// Sandbox-seat marker: the wrapper tooling exists under $HOME
/// (`~/tools/laptop-ssh.sh`). On the laptop seat the service binds
/// the same loopback directly and this is never called. Every step
/// is best-effort: a failure here just means the service attempt
/// that follows fails and the loud fallback engages.
pub fn ensure_sandbox_forward(base: &str) {
    let home = home_dir();
    if !home.join("tools/laptop-ssh.sh").exists() {
        return;
    }
    if health_ok(base) {
        return;
    }
    let port = base
        .rsplit(':')
        .next()
        .and_then(|p| p.trim_end_matches('/').parse::<u16>().ok())
        .unwrap_or(18760);
    let ssh_opts: Vec<String> = [
        "-o",
        &format!(
            "ProxyCommand=python3 {}",
            home.join("tools/laptop-proxy-connect.py").display()
        ),
        "-o",
        "StrictHostKeyChecking=yes",
        "-o",
        &format!(
            "UserKnownHostsFile={}",
            home.join(".ssh/known_hosts_laptop").display()
        ),
        "-o",
        "IdentitiesOnly=yes",
        "-i",
        &home.join(".ssh/id_ed25519").display().to_string(),
        "-o",
        "ControlMaster=auto",
        "-o",
        &format!("ControlPath={}", home.join(".ssh/cm-%r@%h:%p").display()),
        "-o",
        "ControlPersist=3600",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let target = "user@100.99.113.103";
    let _ = std::process::Command::new(home.join("tools/laptop-ssh.sh"))
        .arg("true")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    let mut cmd = std::process::Command::new("ssh");
    cmd.args(&ssh_opts)
        .args(["-O", "forward", "-L"])
        .arg(format!("{port}:127.0.0.1:{port}"))
        .arg(target)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let _ = cmd.status();
}

/// Run one search through the service. On success returns the
/// reconstructed envelope (the service's envelope with the paged
/// results spliced in, field-for-field what standalone renders
/// for the same request); on any failure returns a one-clause
/// detail for the caller's loud fallback.
pub fn search(base: &str, request: &SearchRequest) -> Result<Json, String> {
    let token = client_token()?;
    search_with_token(base, request, &token)
}

fn search_with_token(base: &str, request: &SearchRequest, token: &str) -> Result<Json, String> {
    let agent = agent();
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
        let remaining = request.k.saturating_sub(results.len());
        results.extend(page.results.iter().take(remaining).cloned());
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
/// the service only to shape the fallback detail honestly (the
/// caller serves get-chunk standalone either way) — atlas-search's
/// own shape for this verb.
pub fn get_chunk_fallback_detail(base: &str) -> String {
    if health_ok(base) {
        format!(
            "service at {base} is up, but get-chunk is job-scoped in the protocol and this invocation has no job"
        )
    } else {
        format!("service at {base} unreachable; get-chunk served standalone")
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
        OversizedPage,
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
    /// The stub is the service-parity oracle for these tests: the
    /// client's job is to reproduce its ranking exactly.
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
                        StubMode::OversizedPage => {
                            (r#"[{"id":1},{"id":2},{"id":3}]"#, r#""more-cursor""#)
                        }
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
                        r#"{"job_id":"job-1","status":"done","branch_states":{"fts":"done","vector":"done"},"effective_ceiling":"personal","pool":100,"submitted_at_ms":0,"expires_at_ms":999999999}"#,
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
    fn search_multi_page_happy_path_reproduces_the_stub_ranking() {
        // Service-parity oracle: the client's spliced ranking is
        // the stub service's ranking, in order — [1, 2].
        let base = start_stub(StubMode::HappyPath);
        let out = search_with_token(&base, &request_k(10), "test-token").unwrap();
        let results = out
            .get("results")
            .and_then(Json::as_array)
            .expect("envelope carries a results array");
        let ids: Vec<i64> = results
            .iter()
            .filter_map(|r| r.get("id")?.as_i64())
            .collect();
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn search_multi_page_stops_at_requested_k() {
        let base = start_stub(StubMode::HappyPath);
        let out = search_with_token(&base, &request_k(2), "test-token").unwrap();
        let results = out
            .get("results")
            .and_then(Json::as_array)
            .expect("envelope carries a results array");
        let ids: Vec<i64> = results
            .iter()
            .filter_map(|r| r.get("id")?.as_i64())
            .collect();
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn search_truncates_oversized_single_page_to_requested_k() {
        let base = start_stub(StubMode::OversizedPage);
        let out = search_with_token(&base, &request_k(2), "test-token").unwrap();
        let results = out
            .get("results")
            .and_then(Json::as_array)
            .expect("envelope carries a results array");
        let ids: Vec<i64> = results
            .iter()
            .filter_map(|r| r.get("id")?.as_i64())
            .collect();
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn base_url_resolution_order() {
        assert_eq!(base_url(Some("http://x:1/")), "http://x:1");
        // The env/default legs are process-global state; the flag
        // leg above is the one tests may pin hermetically.
        assert!(base_url(None).starts_with("http"));
    }
}
