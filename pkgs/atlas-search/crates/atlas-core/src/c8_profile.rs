//! C8 report/page metadata profile and typed links — design slice
//! C8-M (developing-today/code#9576, #12919).
//!
//! This module is the seam-independent core of C8-M: the pure types
//! and derivations the merged C8 design specifies in §2 (corpus
//! metadata envelope) and §3 (link model). It deliberately contains
//! no ingestion, storage, resolution, or traversal: those integrate
//! through the Atlas platform seams owned by #9091 (ingestion) and
//! the #9106 relation projection (graph), per the design's delivery
//! table. What lives here is everything about a report/page that can
//! be computed from the page's canonical URL and body alone:
//!
//! - [`UrlParts`]: the canonical source URL exploded into scheme,
//!   host, port, path, file name, last path part, base name, and
//!   extension, with media-type derivation (media type, is-HTML and
//!   is-video flags) per the #9576 page metadata schema.
//! - [`sanitize_url`]: the canonical form of a source URL with
//!   credentials and secret query parameters discarded (design §2).
//!   Invariants: the output never contains userinfo or a secret
//!   query parameter, and sanitizing an already-canonical URL is
//!   the identity.
//! - [`C8Source`] / [`C8Page`]: the `source` and `page` objects of
//!   the design §2 envelope profile, serialized exactly as the
//!   design names them. Fields a source does not supply stay absent
//!   rather than guessed (e.g. `modified_at`).
//! - [`extract_links`]: explicit links extracted from a page body
//!   and classified into the design §3 closed vocabulary. Extraction
//!   only ever produces `evidence: explicit` links without a
//!   confidence; `generated_by` and `related_error` links are
//!   source-supplied metadata, never inferred from link shape here.

use serde::{Deserialize, Serialize};

/// Version of the C8 profile of the shared metadata envelope
/// (design §2: version always; additive-only changes).
pub const C8_PROFILE_SCHEMA_VERSION: u32 = 1;

/// Query parameter names (compared case-insensitively, with `-`
/// normalized to `_`) whose values are credentials or signatures and
/// are therefore discarded from canonical URLs (design §2:
/// "credentials and secret query parameters are discarded").
const SECRET_QUERY_KEYS: &[&str] = &[
    "access_token",
    "api_key",
    "apikey",
    "auth",
    "authorization",
    "client_secret",
    "credential",
    "credentials",
    "key",
    "passwd",
    "password",
    "private_token",
    "secret",
    "session",
    "session_id",
    "sig",
    "signature",
    "token",
    "x_api_key",
];

fn is_secret_query_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace('-', "_");
    SECRET_QUERY_KEYS.contains(&normalized.as_str())
}

/// The canonical source URL, exploded (design §2 `url_parts`, plus
/// the derived fields the #9576 page metadata schema names).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UrlParts {
    pub scheme: String,
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    pub path: String,
    /// Final path segment when the URL addresses a file-like
    /// resource (path does not end in `/`); absent for directories.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    /// Last non-empty path segment, whether file or directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_path_part: Option<String>,
    /// `file_name` without its final extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_name: Option<String>,
    /// Lowercased extension of `file_name`, without the dot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension: Option<String>,
}

impl UrlParts {
    /// Parse an absolute `scheme://authority/path` URL. Returns
    /// `None` for relative references and malformed input. Userinfo
    /// in the authority is accepted and discarded — it is never
    /// retained in the parts. Scheme and host are lowercased; the
    /// path is preserved as-is.
    pub fn parse(raw: &str) -> Option<UrlParts> {
        let (scheme, rest) = raw.split_once("://")?;
        if scheme.is_empty()
            || !scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            return None;
        }
        let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let authority = &rest[..authority_end];
        let after = &rest[authority_end..];
        // Discard any userinfo: everything up to the last '@'.
        let hostport = match authority.rfind('@') {
            Some(at) => &authority[at + 1..],
            None => authority,
        };
        if hostport.is_empty() {
            return None;
        }
        let (host, port) = parse_host_port(hostport)?;
        let path_end = after.find(['?', '#']).unwrap_or(after.len());
        let path = if after[..path_end].is_empty() {
            "/".to_string()
        } else {
            after[..path_end].to_string()
        };

        let last_path_part = path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .next_back()
            .map(str::to_string);
        let file_name = if path.ends_with('/') {
            None
        } else {
            last_path_part.clone()
        };
        let (base_name, extension) = match &file_name {
            Some(name) => match name.rfind('.') {
                Some(0) | None => (Some(name.clone()), None),
                Some(dot) => (
                    Some(name[..dot].to_string()),
                    Some(name[dot + 1..].to_ascii_lowercase()),
                ),
            },
            None => (None, None),
        };

        Some(UrlParts {
            scheme: scheme.to_ascii_lowercase(),
            host,
            port,
            path,
            file_name,
            last_path_part,
            base_name,
            extension,
        })
    }

    /// Media type derived from the file extension, when the
    /// extension is a known page/media type. This is a derivation
    /// from the URL only; a source-supplied media type (carried on
    /// [`C8Page`]) takes precedence at the envelope level.
    pub fn media_type(&self) -> Option<&'static str> {
        match self.extension.as_deref() {
            Some("html" | "htm") => Some("text/html"),
            Some("md" | "markdown") => Some("text/markdown"),
            Some("txt") => Some("text/plain"),
            Some("json") => Some("application/json"),
            Some("pdf") => Some("application/pdf"),
            Some("png") => Some("image/png"),
            Some("jpg" | "jpeg") => Some("image/jpeg"),
            Some("gif") => Some("image/gif"),
            Some("svg") => Some("image/svg+xml"),
            Some("mp4") => Some("video/mp4"),
            Some("webm") => Some("video/webm"),
            Some("mov") => Some("video/quicktime"),
            Some("mp3") => Some("audio/mpeg"),
            Some("wav") => Some("audio/wav"),
            _ => None,
        }
    }

    pub fn is_html(&self) -> bool {
        self.media_type() == Some("text/html")
    }

    pub fn is_video(&self) -> bool {
        self.media_type()
            .is_some_and(|media| media.starts_with("video/"))
    }
}

/// Split an authority's host and optional port. Handles bracketed
/// IPv6 literals. A trailing `:` segment that is not all digits is
/// treated as part of the host rather than a port.
fn parse_host_port(hostport: &str) -> Option<(String, Option<u16>)> {
    if let Some(rest) = hostport.strip_prefix('[') {
        let close = rest.find(']')?;
        let host = &rest[..close];
        let tail = &rest[close + 1..];
        let port = if tail.is_empty() {
            None
        } else {
            Some(tail.strip_prefix(':')?.parse().ok()?)
        };
        return Some((host.to_ascii_lowercase(), port));
    }
    match hostport.rsplit_once(':') {
        Some((host, digits))
            if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) =>
        {
            Some((host.to_ascii_lowercase(), Some(digits.parse().ok()?)))
        }
        _ => Some((hostport.to_ascii_lowercase(), None)),
    }
}

/// The canonical form of a source URL (design §2): scheme and host
/// lowercased, credentials discarded, secret query parameters
/// discarded, fragment dropped (it is page-local, never sent to the
/// source). Benign query parameters survive in their original order.
/// Returns `None` when `raw` is not an absolute URL.
pub fn sanitize_url(raw: &str) -> Option<String> {
    let parts = UrlParts::parse(raw)?;
    let host = if parts.host.contains(':') {
        format!("[{}]", parts.host)
    } else {
        parts.host.clone()
    };
    let mut out = format!("{}://{}", parts.scheme, host);
    if let Some(port) = parts.port {
        out.push(':');
        out.push_str(&port.to_string());
    }
    out.push_str(&parts.path);
    if let Some(query) = raw_query(raw) {
        let kept: Vec<&str> = query
            .split('&')
            .filter(|pair| {
                if pair.is_empty() {
                    return false;
                }
                let key = pair.split('=').next().unwrap_or("");
                !is_secret_query_key(key)
            })
            .collect();
        if !kept.is_empty() {
            out.push('?');
            out.push_str(&kept.join("&"));
        }
    }
    Some(out)
}

/// The raw query string of a URL (between `?` and `#`/end), if any.
fn raw_query(raw: &str) -> Option<&str> {
    let after = raw.split_once('?')?.1;
    let end = after.find('#').unwrap_or(after.len());
    Some(&after[..end])
}

/// The `source` object of the design §2 envelope profile.
/// `kind`, `id`, and `revision` are source-supplied (the design's
/// idempotency identity); `url` is retained only in sanitized
/// canonical form; `observed_at` is Atlas observation time;
/// `modified_at` is source-supplied and stays absent when the
/// source does not provide it — it is never guessed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct C8Source {
    pub kind: String,
    pub id: String,
    pub revision: String,
    pub url: String,
    pub observed_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<String>,
}

impl C8Source {
    /// Build a source record, sanitizing `raw_url` into canonical
    /// form. Returns `None` when `raw_url` is not an absolute URL.
    pub fn new(
        kind: impl Into<String>,
        id: impl Into<String>,
        revision: impl Into<String>,
        raw_url: &str,
        observed_at: impl Into<String>,
        modified_at: Option<String>,
    ) -> Option<C8Source> {
        Some(C8Source {
            kind: kind.into(),
            id: id.into(),
            revision: revision.into(),
            url: sanitize_url(raw_url)?,
            observed_at: observed_at.into(),
            modified_at,
        })
    }
}

/// Typed page kind (design §2: `page.kind` is a typed, documented
/// value, supplied by the source — it is not inferred from the URL
/// here; only media facts are derived).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageKind {
    Report,
    IncidentReport,
    Page,
    PageWithComments,
    LiveStream,
    Unknown,
}

/// The `page` object of the design §2 envelope profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct C8Page {
    pub kind: PageKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    pub url_parts: UrlParts,
    /// Scripts that generated the page, when the source provides
    /// them (design §2); never inferred.
    #[serde(default)]
    pub generated_by: Vec<String>,
    /// Graph/rendering libraries the page used, when the source
    /// provides them; never inferred.
    #[serde(default)]
    pub graph_libraries: Vec<String>,
}

impl C8Page {
    /// Build a page profile from its canonical source URL, deriving
    /// the media type from the URL when the extension is known.
    /// Returns `None` when `raw_url` is not an absolute URL.
    pub fn from_url(kind: PageKind, raw_url: &str) -> Option<C8Page> {
        let parts = UrlParts::parse(raw_url)?;
        let media_type = parts.media_type().map(str::to_string);
        Some(C8Page {
            kind,
            media_type,
            url_parts: parts,
            generated_by: Vec::new(),
            graph_libraries: Vec::new(),
        })
    }
}

/// Link kind, from the closed vocabulary of design §3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    MentionsIssue,
    MentionsPullRequest,
    MentionsCommit,
    LinksPage,
    GeneratedBy,
    RelatedError,
}

/// How a link is known (design §3: `evidence` distinguishes an
/// explicit source link from an inferred association).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkEvidence {
    Explicit,
    Inferred,
}

/// A typed source link (design §3). `target_id` is canonical in its
/// owning system — for web targets, the sanitized canonical URL.
/// Inferred links always carry a confidence; explicit links never
/// do (the constructors enforce this pairing).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct C8Link {
    pub kind: LinkKind,
    pub target_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_locator: Option<String>,
    pub evidence: LinkEvidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

impl C8Link {
    pub fn explicit(kind: LinkKind, target_id: impl Into<String>) -> C8Link {
        C8Link {
            kind,
            target_id: target_id.into(),
            source_locator: None,
            evidence: LinkEvidence::Explicit,
            confidence: None,
        }
    }

    pub fn inferred(kind: LinkKind, target_id: impl Into<String>, confidence: f32) -> C8Link {
        C8Link {
            kind,
            target_id: target_id.into(),
            source_locator: None,
            evidence: LinkEvidence::Inferred,
            confidence: Some(confidence),
        }
    }
}

/// Extract the explicit links of a page body (design §3, phase one
/// of resolution): Markdown inline links and bare `http(s)` URLs,
/// classified by target shape and deduplicated by
/// `(kind, target_id)`, in first-seen order. Every returned link is
/// `evidence: explicit` with no confidence.
///
/// GitHub issue / pull-request / commit URLs classify as
/// `mentions_issue` / `mentions_pull_request` / `mentions_commit`;
/// any other absolute URL and any relative page reference classifies
/// as `links_page`. Non-hierarchical schemes (`mailto:`, `tel:`) and
/// same-page anchors (`#…`) are not page links and are skipped.
/// Link *resolution* against Atlas records is a later C8-M step and
/// is not attempted here.
pub fn extract_links(body: &str) -> Vec<C8Link> {
    let mut candidates: Vec<(usize, String)> = Vec::new();
    candidates.extend(markdown_targets(body));
    candidates.extend(bare_urls(body));
    candidates.sort_by_key(|(position, _)| *position);

    let mut out: Vec<C8Link> = Vec::new();
    for (_, target) in candidates {
        if let Some((kind, target_id)) = classify_link(&target)
            && !out
                .iter()
                .any(|link| link.kind == kind && link.target_id == target_id)
        {
            out.push(C8Link::explicit(kind, target_id));
        }
    }
    out
}

/// Classify one extracted target. Returns the link kind and the
/// canonical target ID (sanitized URL for absolute targets, the raw
/// reference for relative ones), or `None` for targets that are not
/// page links.
fn classify_link(target: &str) -> Option<(LinkKind, String)> {
    if target.is_empty() || target.starts_with('#') {
        return None;
    }
    if let Some(sanitized) = sanitize_url(target) {
        let parts = UrlParts::parse(&sanitized)?;
        if parts.host == "github.com" {
            let segments: Vec<&str> = parts
                .path
                .split('/')
                .filter(|segment| !segment.is_empty())
                .collect();
            if segments.len() == 4 {
                let number = segments[3];
                if segments[2] == "issues"
                    && !number.is_empty()
                    && number.chars().all(|c| c.is_ascii_digit())
                {
                    return Some((LinkKind::MentionsIssue, sanitized));
                }
                if segments[2] == "pull"
                    && !number.is_empty()
                    && number.chars().all(|c| c.is_ascii_digit())
                {
                    return Some((LinkKind::MentionsPullRequest, sanitized));
                }
                if segments[2] == "commit"
                    && (7..=40).contains(&number.len())
                    && number.chars().all(|c| c.is_ascii_hexdigit())
                {
                    return Some((LinkKind::MentionsCommit, sanitized));
                }
            }
        }
        return Some((LinkKind::LinksPage, sanitized));
    }
    // Not an absolute URL: a relative page reference is a page link;
    // anything carrying a scheme (`mailto:`, `tel:`) is not.
    if has_scheme_prefix(target) {
        return None;
    }
    Some((LinkKind::LinksPage, target.to_string()))
}

/// Whether `s` begins with a URI scheme (`ALPHA *( ALPHA / DIGIT /
/// "+" / "-" / "." ) ":"`).
fn has_scheme_prefix(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    for c in chars {
        if c == ':' {
            return true;
        }
        if !(c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
            return false;
        }
    }
    false
}

/// Markdown inline-link targets (`[text](target)`), with the byte
/// position of each target. Handles `<target>` wrapping and an
/// optional `"title"` after the target.
fn markdown_targets(body: &str) -> Vec<(usize, String)> {
    let bytes = body.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b']' && bytes[i + 1] == b'(' {
            let start = i + 2;
            let mut end = start;
            while end < bytes.len() && bytes[end] != b')' {
                end += 1;
            }
            if end < bytes.len() {
                let inner = body[start..end].trim();
                let target = if let Some(rest) = inner.strip_prefix('<') {
                    rest.split('>').next().unwrap_or("")
                } else {
                    inner.split_whitespace().next().unwrap_or("")
                };
                if !target.is_empty() {
                    out.push((start, target.to_string()));
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// Bare `http://` / `https://` URLs in prose, with byte positions.
/// A URL runs until whitespace or a delimiter that cannot be part
/// of it in prose or Markdown; trailing sentence punctuation is
/// trimmed.
fn bare_urls(body: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (scheme, needle) in [(7usize, "http://"), (8usize, "https://")] {
        let mut from = 0;
        while let Some(found) = body[from..].find(needle) {
            let start = from + found;
            let mut end = start + scheme;
            for (offset, c) in body[start + scheme..].char_indices() {
                if c.is_whitespace() || matches!(c, ')' | ']' | '"' | '\'' | '<' | '>' | '`') {
                    break;
                }
                end = start + scheme + offset + c.len_utf8();
            }
            let url = body[start..end].trim_end_matches(['.', ',', ';', ':', '!', '?']);
            if url.len() > scheme {
                out.push((start, url.to_string()));
            }
            from = end.max(start + scheme);
        }
    }
    out
}
