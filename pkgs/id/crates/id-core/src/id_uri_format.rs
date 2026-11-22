//! Canonical formatter for the internal `id://` V1 URI syntax, and the
//! caller-local resolver hint check.
//!
//! Slice A9117-uri-format (issue #10273), design
//! `part-a-addressing/versioned-design.md` A-9117.1, statements
//! S4.11–S4.18. The executable reference is
//! `part-a-addressing/uri_codec_reference.py` (`format_array`,
//! `format_content`, `resolve_context`).
//!
//! Every admitted value has exactly one text form:
//!
//! - the scheme and the resolver alias are lowercase;
//! - namespace and slot octets in the RFC 3986 unreserved set stay
//!   literal and every other octet is an uppercase `%HH` escape;
//! - a null slot is `~` and a bytes slot is `b.` plus its octets;
//! - a content ID is `content/raw-blake3-256.` plus 64 lowercase hex.
//!
//! [`crate::id_uri_parse::parse`] accepts only this spelling for slots
//! and digests, so `parse(format(value)) == value` and
//! `format(parse(canonical)) == canonical`. The resolver alias in a URI
//! is only a hint: [`resolve_context`] requires it to equal the resolver
//! the caller already selected and never selects one from it.

use crate::array_address::ArrayAddress;
use crate::id_shape::ContentId;
use crate::id_uri_parse::{
    CONTENT_PROFILE, IdUriV1, SCHEME, UriParseError, UriTarget, VERSION, check_alias,
};

const HEX_LOWER: &[u8; 16] = b"0123456789abcdef";
const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

/// RFC 3986 unreserved: `ALPHA / DIGIT / "-" / "." / "_" / "~"`.
pub(crate) const fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

/// Append `bytes` in their one canonical escaped form.
pub(crate) fn push_pct_encoded(out: &mut String, bytes: &[u8]) {
    for &byte in bytes {
        if is_unreserved(byte) {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(char::from(HEX_UPPER[usize::from(byte >> 4)]));
            out.push(char::from(HEX_UPPER[usize::from(byte & 0x0F)]));
        }
    }
}

/// The one canonical text form of `value`. Formatting cannot fail:
/// [`IdUriV1::new`] and the parser admit only values that can be written.
pub fn format(value: &IdUriV1) -> String {
    let mut out = String::new();
    out.push_str(SCHEME);
    out.push_str("://");
    out.push_str(value.resolver_hint());
    out.push('/');
    push_pct_encoded(&mut out, value.namespace().as_bytes());
    out.push('/');
    out.push_str(VERSION);
    match value.target() {
        UriTarget::Array(address) => {
            out.push_str("/array");
            for slot in address.slots() {
                out.push('/');
                match slot {
                    None => out.push('~'),
                    Some(bytes) => {
                        out.push_str("b.");
                        push_pct_encoded(&mut out, bytes);
                    }
                }
            }
        }
        UriTarget::Content(id) => {
            out.push_str("/content/");
            out.push_str(CONTENT_PROFILE);
            out.push('.');
            for byte in id.digest() {
                out.push(char::from(HEX_LOWER[usize::from(byte >> 4)]));
                out.push(char::from(HEX_LOWER[usize::from(byte & 0x0F)]));
            }
        }
    }
    out
}

/// Format an array address. Fails as `invalid_resolver_alias`,
/// `empty_namespace` or `dot_segment_namespace` when the value cannot be
/// written.
pub fn format_array(
    resolver_hint: &str,
    namespace: &str,
    address: &ArrayAddress,
) -> Result<String, UriParseError> {
    let value = IdUriV1::new(resolver_hint, namespace, UriTarget::Array(address.clone()))?;
    Ok(format(&value))
}

/// Format a content ID. Fails as for [`format_array`], and as
/// `unsupported_hash_profile` for any profile but `raw-blake3-256`.
pub fn format_content(
    resolver_hint: &str,
    namespace: &str,
    id: &ContentId,
) -> Result<String, UriParseError> {
    let value = IdUriV1::new(resolver_hint, namespace, UriTarget::Content(*id))?;
    Ok(format(&value))
}

/// Check the URI's resolver hint against the resolver the caller has
/// already selected. The hint is never trusted to pick one: a mismatch
/// is `resolver_context_mismatch`, and both must be valid aliases.
pub fn resolve_context<'a>(
    uri_hint: &str,
    selected_local_resolver: &'a str,
) -> Result<&'a str, UriParseError> {
    check_alias(uri_hint)?;
    check_alias(selected_local_resolver)?;
    if uri_hint != selected_local_resolver {
        return Err(UriParseError::ResolverContextMismatch);
    }
    Ok(selected_local_resolver)
}
