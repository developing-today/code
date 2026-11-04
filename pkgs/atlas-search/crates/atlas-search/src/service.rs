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
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("~"))
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

/// Run one search through the service. On success returns the
/// reconstructed envelope; on any failure returns a one-clause
/// detail for the caller's loud fallback.
pub fn search(base: &str, request: &SearchRequest) -> Result<Json, String> {
    let token = client_token()?;
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
    loop {
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
        results.extend(page.results.iter().cloned());
        if results.len() >= request.k || page.next_cursor.is_none() {
            break;
        }
        cursor = page.next_cursor;
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
