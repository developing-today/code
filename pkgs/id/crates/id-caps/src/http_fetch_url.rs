//! `http.fetch` URL checks and the admin allowlist match.
//!
//! Split out of [`crate::world_caps`] (#1062, #12659) to keep that
//! module under the production line limit. `world_caps` re-exports
//! every item here, so callers keep their `world_caps::` paths.

use anyhow::{Result, ensure};

use crate::world_caps::MAX_HTTP_FETCH_URL_BYTES;

/// The canonical spelling of an `http.fetch` host for allowlist
/// comparison: lowercase, with one trailing dot (the DNS root in
/// a fully-qualified name) stripped. Applied to URL hosts and to
/// allowlist entries alike, so the comparison is between like
/// spellings.
#[must_use]
pub fn normalize_http_fetch_host(host: &str) -> String {
    let mut normalized = host.to_ascii_lowercase();
    if normalized.ends_with('.') {
        normalized.pop();
    }
    normalized
}

/// Check an `http.fetch` URL and return its normalized host: the
/// spelling the admin allowlist is matched against. Only `http`
/// and `https` URLs with a host and no userinfo pass — other
/// schemes are not fetches, and credentials in a URL would leak
/// into logs and audit digests.
///
/// # Errors
///
/// Names the rule the URL breaks.
pub fn validate_http_fetch_url(url: &str) -> Result<String> {
    ensure!(
        !url.is_empty() && url.len() <= MAX_HTTP_FETCH_URL_BYTES,
        "http fetch url must be 1 to {MAX_HTTP_FETCH_URL_BYTES} bytes"
    );
    let parsed =
        url::Url::parse(url).map_err(|_| anyhow::anyhow!("http fetch url is malformed"))?;
    ensure!(
        matches!(parsed.scheme(), "http" | "https"),
        "http fetch url scheme must be http or https"
    );
    ensure!(
        parsed.username().is_empty() && parsed.password().is_none(),
        "http fetch url must not carry credentials"
    );
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("http fetch url has no host"))?;
    Ok(normalize_http_fetch_host(host))
}

/// Whether one admin allowlist entry admits an `http.fetch` URL
/// (#1062). An entry is a host, optionally followed by `:port`,
/// spelled as a URL spells it (an IPv6 address in brackets). The
/// host must equal the URL's host after
/// [`normalize_http_fetch_host`], exactly: no wildcards, no implied
/// subdomains. A bare host admits only the default port of the
/// URL's scheme (80 for `http`, 443 for `https`), so listing
/// `example.com` does not open `example.com:8443`; an entry with a
/// port admits that port only, under either scheme. A URL that
/// fails [`validate_http_fetch_url`] is admitted by no entry, and
/// an entry whose port is not a valid port number admits nothing.
#[must_use]
pub fn http_fetch_entry_admits(entry: &str, url: &str) -> bool {
    let Ok(host) = validate_http_fetch_url(url) else {
        return false;
    };
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    let (entry_host, entry_port) = split_http_fetch_entry(entry);
    if normalize_http_fetch_host(entry_host) != host {
        return false;
    }
    match entry_port {
        // `Url::port` is `None` exactly when the URL uses its
        // scheme's default port, spelled out or not.
        None => parsed.port().is_none(),
        Some(port) => parsed.port_or_known_default() == Some(port),
    }
}

/// Split an allowlist entry into its host and optional port. Only
/// a trailing `:digits` is a port, and only when the part before it
/// is a bracketed IPv6 address or holds no other colon, so a bare
/// IPv6 spelling is never cut in the middle.
fn split_http_fetch_entry(entry: &str) -> (&str, Option<u16>) {
    if let Some((host, port)) = entry.rsplit_once(':') {
        let port_shaped = !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit());
        let host_shaped = !host.is_empty() && (host.ends_with(']') || !host.contains(':'));
        if port_shaped && host_shaped {
            // An out-of-range port keeps the whole entry as the
            // host, which matches no URL: the entry fails closed.
            if let Ok(port) = port.parse::<u16>() {
                return (host, Some(port));
            }
        }
    }
    (entry, None)
}
