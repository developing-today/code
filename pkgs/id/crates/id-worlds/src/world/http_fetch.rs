//! The network half of the `http.fetch` provider (#1062, design
//! `doc/2026-11-22_design_id1062-http-fetch-provider.md`).
//!
//! Policy lives elsewhere: the actor's `execute` runs the
//! grant/ceiling decision, and `WorldCore::check_http_fetch` runs
//! the URL, allowlist, and rate checks before this module is
//! called. What lives here is the fetch itself, under the bounds
//! policy cannot express: the total timeout spent down across
//! hops, redirects followed manually with every hop re-validated
//! and re-checked against the allowlist (no redirects off the
//! list), and the response body read only up to its cap.
//!
//! The blocking client is only ever constructed and used on a
//! dedicated OS thread per fetch: the `reqwest` blocking API
//! panics inside an async runtime, and the actor's runtime is
//! never asked to block. One thread per fetch is cheap next to a
//! network round trip, and the rate bound caps how many exist.

use std::collections::BTreeSet;
use std::io::Read;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::world_caps::{
    CapError, MAX_HTTP_FETCH_REDIRECTS, MAX_HTTP_FETCH_RESPONSE_BYTES, MAX_HTTP_FETCH_TIMEOUT_MS,
    http_fetch_entry_admits, validate_http_fetch_url,
};

/// One completed fetch: the URL that produced the response (after
/// any followed redirects), its status, and its body bytes.
#[derive(Debug)]
pub(crate) struct HttpFetchOutcome {
    /// Final URL, after redirects.
    pub url: String,
    /// HTTP status code.
    pub status: u16,
    /// Response body, at most `MAX_HTTP_FETCH_RESPONSE_BYTES`.
    pub body: Vec<u8>,
}

/// The process-wide blocking client: redirects off (the executor
/// follows them manually, checking each hop), no proxy (a fetch
/// goes to the allowlisted host, not through whatever proxy the
/// environment names).
fn client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        match reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
        {
            Ok(client) => client,
            Err(error) => {
                tracing::error!("http fetch client build failed: {error:#}");
                reqwest::blocking::Client::new()
            }
        }
    })
}

/// Fetch `url` (already validated and allowlist-checked by the
/// caller) on a dedicated thread, joined before returning.
/// `allowed_hosts` is the admin allowlist snapshot, consulted
/// again for every redirect hop.
///
/// # Errors
///
/// A [`CapError`] naming the failure: `fetch_timeout`,
/// `fetch_failed`, `response_too_large`, `too_many_redirects`,
/// or `host_not_allowed` for a redirect off the list.
pub(crate) fn fetch(
    url: &str,
    allowed_hosts: &BTreeSet<String>,
) -> Result<HttpFetchOutcome, CapError> {
    let url = url.to_owned();
    let allowed_hosts = allowed_hosts.clone();
    let handle = std::thread::spawn(move || perform(&url, &allowed_hosts));
    match handle.join() {
        Ok(outcome) => outcome,
        Err(_) => Err(CapError::failed_with("fetch_failed")),
    }
}

/// The fetch loop, running on the fetch thread.
fn perform(url: &str, allowed_hosts: &BTreeSet<String>) -> Result<HttpFetchOutcome, CapError> {
    let deadline = Instant::now() + Duration::from_millis(MAX_HTTP_FETCH_TIMEOUT_MS);
    let mut current =
        reqwest::Url::parse(url).map_err(|_| CapError::bad_args_with("malformed_url"))?;
    let mut hops = 0usize;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(CapError::failed_with("fetch_timeout"));
        }
        let mut response = client()
            .get(current.clone())
            .timeout(remaining)
            .send()
            .map_err(|error| map_reqwest_error(&error))?;
        let status = response.status();
        if status.is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            if let Some(location) = location {
                hops += 1;
                if hops > MAX_HTTP_FETCH_REDIRECTS {
                    return Err(CapError::failed_with("too_many_redirects"));
                }
                // A redirect is a fresh request to a new host and
                // port: it passes the same validation and allowlist
                // check before any connection to it is made.
                let next = current
                    .join(&location)
                    .map_err(|_| CapError::bad_args_with("host_not_allowed"))?;
                validate_http_fetch_url(next.as_str())
                    .map_err(|_| CapError::bad_args_with("host_not_allowed"))?;
                if !allowed_hosts
                    .iter()
                    .any(|listed| http_fetch_entry_admits(listed, next.as_str()))
                {
                    return Err(CapError::bad_args_with("host_not_allowed"));
                }
                current = next;
                continue;
            }
        }
        // Read at most the cap plus one byte: a longer body is a
        // refusal, and an unbounded stream costs bounded memory.
        let mut body = Vec::new();
        response
            .by_ref()
            .take((MAX_HTTP_FETCH_RESPONSE_BYTES + 1) as u64)
            .read_to_end(&mut body)
            .map_err(|_| CapError::failed_with("fetch_failed"))?;
        if body.len() > MAX_HTTP_FETCH_RESPONSE_BYTES {
            return Err(CapError::failed_with("response_too_large"));
        }
        return Ok(HttpFetchOutcome {
            url: current.to_string(),
            status: status.as_u16(),
            body,
        });
    }
}

/// Map a transport failure onto the provider's stable reasons.
fn map_reqwest_error(error: &reqwest::Error) -> CapError {
    if error.is_timeout() {
        CapError::failed_with("fetch_timeout")
    } else {
        CapError::failed_with("fetch_failed")
    }
}
