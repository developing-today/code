//! The via-service client seam (design section 1): *prefer
//! service, fall back to standalone loudly*.
//!
//! Step 3 (#5353) ships the contract, not the protocol: the job
//! protocol client proper lands in step 4 (#5354) against
//! atlas-proto, behind this same function. Today `attempt` probes
//! the configured base URL with a short timeout and reports what
//! it found; the caller then always serves standalone, printing
//! the warning on stderr and recording the note in the envelope
//! (matching how the Python reports backend fallbacks in
//! `notes`). No outcome of the probe — refused, timeout, an HTTP
//! answer from something that is not yet the service — can hang
//! or silently change a result.

use std::time::Duration;

/// What the probe found, in one clause, for the warning + note.
pub struct Attempt {
    pub detail: String,
}

/// The default service base URL (loopback bind per design
/// section 3; the port is proposed there pending the packaging
/// step's port-inventory check, #5355).
pub const DEFAULT_SERVICE_URL: &str = "http://127.0.0.1:18760";

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

/// Probe the service endpoint. Step 4 replaces this probe with
/// the real submit/page client; the fallback contract around it
/// does not change.
pub fn attempt(base: &str) -> Attempt {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(2)))
        .timeout_connect(Some(Duration::from_secs(1)))
        .http_status_as_error(false)
        .build()
        .into();
    let detail = match agent.get(base).call() {
        Ok(resp) => format!(
            "service at {base} answered HTTP {} but the job-protocol client lands in step 4 (#5354)",
            resp.status().as_u16()
        ),
        Err(e) => format!("service at {base} unreachable ({e})"),
    };
    Attempt { detail }
}
