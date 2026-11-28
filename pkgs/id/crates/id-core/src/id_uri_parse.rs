//! Strict parser and formatter for the internal `id://` V1 URI syntax.
//!
//! Slice A9117-uri-parser (issue #10260), design
//! `part-a-addressing/versioned-design.md` A-9117.1, statements
//! S4.11–S4.18. The executable reference is
//! `part-a-addressing/uri_codec_reference.py`; this module follows it
//! case for case, including its error class names.
//!
//! ```text
//! id://<resolver-hint>/<namespace>/v1/array(/~ | /b.<pct-octets>)*
//! id://<resolver-hint>/<namespace>/v1/content/raw-blake3-256.<64 lowercase hex>
//! ```
//!
//! Parsing is an RFC 3986 component split followed by exactly one
//! field-specific decode and an exact grammar check:
//!
//! - The whole URI is visible ASCII (`0x21..=0x7E`). Query and fragment
//!   delimiters are not part of V1, even when empty.
//! - The authority is a resolver *hint*, `[a-z0-9][a-z0-9._-]{0,62}`. It
//!   never selects a resolver by itself: [`resolve_context`] requires it
//!   to equal the resolver the caller already selected.
//! - The path is split on `/` while escapes are still encoded, so an
//!   escaped `%2F` is data and never a separator. Empty segments (a
//!   doubled or trailing slash) are rejected. There is no dot-segment
//!   normalization and no second decode.
//! - The namespace segment is percent-decoded once and must be non-empty
//!   UTF-8 other than `.` and `..`.
//! - The version segment is critical: anything but `v1` is rejected.
//! - The shape segment is critical: anything but `array` or `content` is
//!   rejected.
//! - A bytes slot keeps the RFC 3986 unreserved octets literal and
//!   escapes every other octet as uppercase `%HH`; any other spelling of
//!   the same bytes is rejected, so a slot has exactly one text form.
//! - A content token names the `raw-blake3-256` profile and a lowercase
//!   64-digit hex digest.
//!
//! The canonical formatter and the resolver-context check live in
//! [`crate::id_uri_format`] (slice A9117-uri-format, #10273); this module
//! borrows its octet encoder so a slot's one canonical spelling is
//! defined in exactly one place.

use std::fmt;

use crate::array_address::ArrayAddress;
use crate::id_shape::{ContentId, DIGEST_BYTES, HashProfile};
use crate::id_uri_format::{is_unreserved, push_pct_encoded};

/// The URI scheme, written lowercase by the formatter.
pub const SCHEME: &str = "id";

/// The only critical version segment V1 admits.
pub const VERSION: &str = "v1";

/// The only content profile token V1 admits in a URI.
pub const CONTENT_PROFILE: &str = "raw-blake3-256";

/// Longest admitted resolver alias, in bytes.
pub const MAX_ALIAS_LEN: usize = 63;

/// Why a string is not a V1 `id://` URI, or a value cannot be written
/// as one. [`UriParseError::class`] gives the stable class name the
/// reference implementation uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UriParseError {
    /// A character outside visible ASCII `0x21..=0x7E`.
    RawControlSpaceOrNonAscii,
    /// The generic URI split itself failed: the authority holds an IP
    /// literal bracket.
    UriParse,
    /// The scheme is not `id`, or there is no `//` authority.
    InvalidSchemeOrMissingAuthority,
    /// A `?` or `#` appears anywhere.
    QueryOrFragmentNotInV1,
    /// The authority is not a lowercase resolver alias.
    InvalidResolverAlias,
    /// There is no path after the authority.
    PathMustBeAbsolute,
    /// Fewer than three path segments, or an empty one.
    EmptyOrMissingPathComponent,
    /// A `%` not followed by two hex digits.
    MalformedPercentEscape,
    /// A raw octet in a component that is neither unreserved nor part of
    /// an escape.
    RawNonUnreservedComponentByte,
    /// The namespace decodes to bytes that are not UTF-8.
    InvalidNamespaceUtf8,
    /// The namespace decodes to the empty string, `.` or `..`.
    InvalidNamespaceComponent,
    /// The version segment is not `v1`.
    UnsupportedCriticalVersion,
    /// The shape segment is neither `array` nor `content`, or `content`
    /// is not followed by exactly one token.
    UnknownCriticalShape,
    /// An array slot token is neither `~` nor `b.` plus octets.
    InvalidArraySlotToken,
    /// A bytes slot is not in its one canonical escaped form.
    NoncanonicalByteSlot,
    /// The content token has no `.` between profile and digest.
    InvalidContentToken,
    /// The content profile is not `raw-blake3-256`.
    UnsupportedHashProfile,
    /// The digest is not 64 lowercase hex digits.
    InvalidDigestText,
    /// Formatting: the namespace is empty.
    EmptyNamespace,
    /// Formatting: the namespace is `.` or `..`.
    DotSegmentNamespace,
    /// The URI's resolver hint differs from the resolver already
    /// selected.
    ResolverContextMismatch,
}

impl UriParseError {
    /// The stable class name, as in the reference implementation.
    pub const fn class(&self) -> &'static str {
        match self {
            Self::RawControlSpaceOrNonAscii => "raw_control_space_or_nonascii",
            Self::UriParse => "uri_parse_error",
            Self::InvalidSchemeOrMissingAuthority => "invalid_scheme_or_missing_authority",
            Self::QueryOrFragmentNotInV1 => "query_or_fragment_not_in_v1",
            Self::InvalidResolverAlias => "invalid_resolver_alias",
            Self::PathMustBeAbsolute => "path_must_be_absolute",
            Self::EmptyOrMissingPathComponent => "empty_or_missing_path_component",
            Self::MalformedPercentEscape => "malformed_percent_escape",
            Self::RawNonUnreservedComponentByte => "raw_non_unreserved_component_byte",
            Self::InvalidNamespaceUtf8 => "invalid_namespace_utf8",
            Self::InvalidNamespaceComponent => "invalid_namespace_component",
            Self::UnsupportedCriticalVersion => "unsupported_critical_version",
            Self::UnknownCriticalShape => "unknown_critical_shape",
            Self::InvalidArraySlotToken => "invalid_array_slot_token",
            Self::NoncanonicalByteSlot => "noncanonical_byte_slot",
            Self::InvalidContentToken => "invalid_content_token",
            Self::UnsupportedHashProfile => "unsupported_hash_profile",
            Self::InvalidDigestText => "invalid_digest_text",
            Self::EmptyNamespace => "empty_namespace",
            Self::DotSegmentNamespace => "dot_segment_namespace",
            Self::ResolverContextMismatch => "resolver_context_mismatch",
        }
    }
}

impl fmt::Display for UriParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.class())
    }
}

impl std::error::Error for UriParseError {}

/// What a V1 URI names: an array address or a content ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UriTarget {
    /// `/array` followed by zero or more slot tokens.
    Array(ArrayAddress),
    /// `/content/raw-blake3-256.<digest>`.
    Content(ContentId),
}

/// A parsed V1 `id://` URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdUriV1 {
    resolver_hint: String,
    namespace: String,
    target: UriTarget,
}

impl IdUriV1 {
    /// Build a URI value. The alias and namespace are checked here so
    /// that every value can be formatted.
    pub fn new(
        resolver_hint: impl Into<String>,
        namespace: impl Into<String>,
        target: UriTarget,
    ) -> Result<Self, UriParseError> {
        let resolver_hint = resolver_hint.into();
        let namespace = namespace.into();
        check_alias(&resolver_hint)?;
        check_namespace(&namespace)?;
        if let UriTarget::Content(id) = &target {
            check_content_profile(id)?;
        }
        Ok(Self {
            resolver_hint,
            namespace,
            target,
        })
    }

    /// Parse `uri` under the V1 grammar.
    pub fn parse(uri: &str) -> Result<Self, UriParseError> {
        parse(uri)
    }

    /// The authority, a resolver hint that never selects a resolver on
    /// its own; see [`resolve_context`].
    pub fn resolver_hint(&self) -> &str {
        &self.resolver_hint
    }

    /// The decoded namespace text.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// What the URI names.
    pub const fn target(&self) -> &UriTarget {
        &self.target
    }

    /// The one canonical text form of this value; see
    /// [`crate::id_uri_format::format`].
    pub fn format(&self) -> String {
        crate::id_uri_format::format(self)
    }
}

impl fmt::Display for IdUriV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.format())
    }
}

/// Parse `uri` under the V1 grammar.
pub fn parse(uri: &str) -> Result<IdUriV1, UriParseError> {
    let (alias, namespace, tail) = split(uri)?;
    let target = match tail.as_slice() {
        ["array", slots @ ..] => {
            let mut out = Vec::with_capacity(slots.len());
            for token in slots {
                if *token == "~" {
                    out.push(None);
                } else if let Some(payload) = token.strip_prefix("b.") {
                    out.push(Some(decode_octets(payload)?));
                } else {
                    return Err(UriParseError::InvalidArraySlotToken);
                }
            }
            UriTarget::Array(ArrayAddress::new(out))
        }
        ["content", token] => UriTarget::Content(parse_content_token(token)?),
        _ => return Err(UriParseError::UnknownCriticalShape),
    };
    Ok(IdUriV1 {
        resolver_hint: alias.to_owned(),
        namespace,
        target,
    })
}

/// RFC 3986 scheme: `ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )`.
fn is_scheme(text: &str) -> bool {
    let mut bytes = text.bytes();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
}

const fn is_alias_tail(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
}

pub(crate) fn check_alias(alias: &str) -> Result<(), UriParseError> {
    let bytes = alias.as_bytes();
    let Some((&first, rest)) = bytes.split_first() else {
        return Err(UriParseError::InvalidResolverAlias);
    };
    if bytes.len() > MAX_ALIAS_LEN
        || !(first.is_ascii_lowercase() || first.is_ascii_digit())
        || !rest.iter().copied().all(is_alias_tail)
    {
        return Err(UriParseError::InvalidResolverAlias);
    }
    Ok(())
}

fn check_namespace(namespace: &str) -> Result<(), UriParseError> {
    if namespace.is_empty() {
        return Err(UriParseError::EmptyNamespace);
    }
    if namespace == "." || namespace == ".." {
        return Err(UriParseError::DotSegmentNamespace);
    }
    Ok(())
}

fn check_content_profile(id: &ContentId) -> Result<(), UriParseError> {
    if id.profile() == HashProfile::RawBlake3_256 {
        Ok(())
    } else {
        Err(UriParseError::UnsupportedHashProfile)
    }
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Decode one path component, already split out, exactly once.
fn pct_decode(component: &str) -> Result<Vec<u8>, UriParseError> {
    let bytes = component.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        if byte == b'%' {
            let high = bytes.get(index + 1).copied().and_then(hex_value);
            let low = bytes.get(index + 2).copied().and_then(hex_value);
            let (Some(high), Some(low)) = (high, low) else {
                return Err(UriParseError::MalformedPercentEscape);
            };
            out.push((high << 4) | low);
            index += 3;
        } else if is_unreserved(byte) {
            out.push(byte);
            index += 1;
        } else {
            return Err(UriParseError::RawNonUnreservedComponentByte);
        }
    }
    Ok(out)
}

/// Decode a bytes slot and require its one canonical spelling.
fn decode_octets(component: &str) -> Result<Vec<u8>, UriParseError> {
    let decoded = pct_decode(component)?;
    let mut canonical = String::with_capacity(component.len());
    push_pct_encoded(&mut canonical, &decoded);
    if canonical != component {
        return Err(UriParseError::NoncanonicalByteSlot);
    }
    Ok(decoded)
}

fn namespace_text(segment: &str) -> Result<String, UriParseError> {
    let raw = pct_decode(segment)?;
    let Ok(value) = String::from_utf8(raw) else {
        return Err(UriParseError::InvalidNamespaceUtf8);
    };
    if value.is_empty() || value == "." || value == ".." {
        return Err(UriParseError::InvalidNamespaceComponent);
    }
    Ok(value)
}

fn parse_content_token(token: &str) -> Result<ContentId, UriParseError> {
    let (profile, digest_text) = token
        .split_once('.')
        .ok_or(UriParseError::InvalidContentToken)?;
    if profile != CONTENT_PROFILE {
        return Err(UriParseError::UnsupportedHashProfile);
    }
    let text = digest_text.as_bytes();
    if text.len() != DIGEST_BYTES * 2 {
        return Err(UriParseError::InvalidDigestText);
    }
    let mut digest = [0_u8; DIGEST_BYTES];
    for (slot, pair) in digest.iter_mut().zip(text.chunks_exact(2)) {
        let nibble = |byte: u8| match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            _ => None,
        };
        let (Some(high), Some(low)) = (nibble(pair[0]), nibble(pair[1])) else {
            return Err(UriParseError::InvalidDigestText);
        };
        *slot = (high << 4) | low;
    }
    Ok(ContentId::new(HashProfile::RawBlake3_256, digest))
}

/// The RFC 3986 component split: scheme, authority, path segments. The
/// path is split while its escapes are still encoded.
fn split(uri: &str) -> Result<(&str, String, Vec<&str>), UriParseError> {
    if !uri.bytes().all(|byte| (0x21..=0x7E).contains(&byte)) {
        return Err(UriParseError::RawControlSpaceOrNonAscii);
    }
    // Generic split first, as RFC 3986 appendix B does for any scheme:
    // an optional scheme, then an optional `//` authority that ends at
    // the first `/`, `?` or `#`.
    let (scheme, rest) = match uri.split_once(':') {
        Some((scheme, rest)) if is_scheme(scheme) => (Some(scheme), rest),
        _ => (None, uri),
    };
    let authority = rest.strip_prefix("//").map(|hier| {
        let end = hier.find(['/', '?', '#']).unwrap_or(hier.len());
        hier.split_at(end)
    });
    // Brackets only ever introduce an IP literal, which is never a
    // resolver alias; the reference's generic splitter fails on them.
    if let Some((authority, _)) = authority
        && authority.contains(['[', ']'])
    {
        return Err(UriParseError::UriParse);
    }
    // The scheme is case-insensitive (RFC 3986 section 3.1); the
    // formatter always writes it lowercase.
    let (authority, after) = match (scheme, authority) {
        (Some(scheme), Some((authority, after)))
            if scheme.eq_ignore_ascii_case(SCHEME) && !authority.is_empty() =>
        {
            (authority, after)
        }
        _ => return Err(UriParseError::InvalidSchemeOrMissingAuthority),
    };
    if uri.contains(['?', '#']) {
        return Err(UriParseError::QueryOrFragmentNotInV1);
    }
    check_alias(authority)?;
    let Some(path) = after.strip_prefix('/') else {
        return Err(UriParseError::PathMustBeAbsolute);
    };
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() < 3 || segments.iter().any(|segment| segment.is_empty()) {
        return Err(UriParseError::EmptyOrMissingPathComponent);
    }
    let namespace = namespace_text(segments[0])?;
    if segments[1] != VERSION {
        return Err(UriParseError::UnsupportedCriticalVersion);
    }
    let tail = segments[2..].to_vec();
    Ok((authority, namespace, tail))
}
