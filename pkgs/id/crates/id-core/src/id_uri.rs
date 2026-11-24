//! The `id://` URL: parser and canonical serializer (#10980; design
//! #9117, `doc/2026-11-17T04-55-00Z_design_id9117_id_uri_scheme.md`).
//!
//! ```text
//! id-URI       = "id://" authority path-abempty [ "?" query ] [ "#" fragment ]
//! authority    = host [ ":" port ]              ; no userinfo
//! path-abempty = "/" namespace *( "/" segment )
//! namespace    = ALPHA *( ALPHA / DIGIT / "-" ) ; lowercase after normalisation
//! segment      = *pchar
//! ```
//!
//! This module is the URL layer only (§2–§3 of the design). It knows
//! nothing about which namespaces a server declares or how a namespace
//! encodes its paths; selecting a namespace and applying its encoding is
//! the server-side registry's job (#10981). What it guarantees:
//!
//! - **Normalisation.** The scheme and namespace are lowercased. The host
//!   is lowercased and IDNA names become punycode. Percent-encoded
//!   unreserved characters are decoded; every other percent-encoding is
//!   kept with uppercase hex. Path case is preserved, and a trailing
//!   slash is significant (an empty final segment is a segment).
//! - **Length.** Input longer than [`MAX_ID_URI_LEN`] (8 KiB) is rejected
//!   with [`IdUriError::TooLong`], never truncated.
//! - **Named errors.** Every rejection class has its own
//!   [`IdUriError`] variant.
//! - **Round trip.** `parse(s).to_string()` is the canonical form, and
//!   parsing the canonical form yields an equal [`IdUri`].
//!
//! Two choices this module makes that the design leaves implicit:
//!
//! - Characters outside RFC 3986 (spaces, raw non-ASCII in the path,
//!   query or fragment) are rejected as [`IdUriError::InvalidCharacter`]
//!   rather than silently percent-encoded, so a parsed URL is always
//!   exactly the text a resolver will see. Non-ASCII is accepted in the
//!   host only, where IDNA defines its meaning.
//! - The dot segments `.` and `..` (including `%2E` spellings) are
//!   rejected as [`IdUriError::DotSegment`]. Generic URI tooling removes
//!   them (RFC 3986 §5.2.4), which would let the same text name a
//!   different namespace path, or a different namespace, depending on
//!   who handled it first.
//!
//! The fragment is parsed and kept for client use, but it is never part
//! of resolution: [`IdUri::without_fragment`] is the form a client sends.

use std::fmt;
use std::net::Ipv6Addr;
use std::str::FromStr;

/// The longest `id://` URL accepted, in bytes (design §3, Q4). A
/// namespace may declare a lower limit; none may raise it.
pub const MAX_ID_URI_LEN: usize = 8 * 1024;

const SCHEME: &str = "id://";

/// The part of a URL a parse error was found in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Component {
    /// The host.
    Host,
    /// The namespace (the first path segment).
    Namespace,
    /// A namespace-path segment.
    Path,
    /// The query.
    Query,
    /// The fragment.
    Fragment,
}

impl fmt::Display for Component {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Host => "host",
            Self::Namespace => "namespace",
            Self::Path => "path",
            Self::Query => "query",
            Self::Fragment => "fragment",
        })
    }
}

/// Why an `id://` URL was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdUriError {
    /// Longer than [`MAX_ID_URI_LEN`]; never truncated.
    #[error("id URL is {len} bytes, over the {max}-byte limit")]
    TooLong {
        /// The input length in bytes.
        len: usize,
        /// The limit it exceeded.
        max: usize,
    },
    /// Does not start with `id://`.
    #[error("not an id:// URL")]
    NotIdScheme,
    /// The authority carries userinfo; credentials never ride URLs.
    #[error("id URL authority carries userinfo, which is forbidden")]
    UserinfoPresent,
    /// The authority names no host.
    #[error("id URL has an empty host")]
    EmptyHost,
    /// The host is not a valid DNS name, IPv4 address or IPv6 literal.
    #[error("id URL host {host:?} is invalid")]
    BadHost {
        /// The host as written.
        host: String,
    },
    /// The port is not a decimal number from 0 to 65535.
    #[error("id URL port {port:?} is invalid")]
    BadPort {
        /// The port as written.
        port: String,
    },
    /// The URL has no path, so it names no namespace.
    #[error("id URL names no namespace")]
    MissingNamespace,
    /// The first path segment is not a namespace name
    /// (`ALPHA *( ALPHA / DIGIT / "-" )`).
    #[error("id URL namespace {segment:?} is invalid")]
    BadNamespace {
        /// The segment as written.
        segment: String,
    },
    /// A `%` not followed by two hex digits.
    #[error("malformed percent-encoding in the id URL {component} at byte {at}")]
    MalformedPercentEncoding {
        /// Where it was found.
        component: Component,
        /// Its byte offset within that component.
        at: usize,
    },
    /// A character RFC 3986 does not allow in that component.
    #[error("character {ch:?} is not allowed in the id URL {component}")]
    InvalidCharacter {
        /// Where it was found.
        component: Component,
        /// The offending character.
        ch: char,
    },
    /// A `.` or `..` namespace-path segment.
    #[error("dot segment {segment:?} is not allowed in an id URL path")]
    DotSegment {
        /// The segment as written.
        segment: String,
    },
    /// A namespace-path segment is not valid UTF-8 once percent-decoded,
    /// so it cannot be read under the `text` encoding.
    #[error("id URL path segment {index} is not valid UTF-8 once decoded")]
    InvalidUtf8 {
        /// The segment's index among the namespace-path segments.
        index: usize,
    },
}

/// A parsed, normalised `id://` URL.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdUri {
    host: String,
    port: Option<u16>,
    namespace: String,
    segments: Vec<String>,
    query: Option<String>,
    fragment: Option<String>,
}

impl IdUri {
    /// Parses and normalises an `id://` URL.
    pub fn parse(input: &str) -> Result<Self, IdUriError> {
        if input.len() > MAX_ID_URI_LEN {
            return Err(IdUriError::TooLong {
                len: input.len(),
                max: MAX_ID_URI_LEN,
            });
        }
        let rest = input
            .get(..SCHEME.len())
            .filter(|prefix| prefix.eq_ignore_ascii_case(SCHEME))
            .and_then(|_| input.get(SCHEME.len()..))
            .ok_or(IdUriError::NotIdScheme)?;

        let (rest, fragment) = match rest.split_once('#') {
            Some((before, fragment)) => (before, Some(fragment)),
            None => (rest, None),
        };
        let (rest, query) = match rest.split_once('?') {
            Some((before, query)) => (before, Some(query)),
            None => (rest, None),
        };
        let (authority, path) = match rest.find('/') {
            Some(slash) => rest.split_at(slash),
            None => (rest, ""),
        };

        let (host, port) = parse_authority(authority)?;
        let mut parts = path.split('/');
        // `path` is empty or starts with '/': the first piece is empty.
        let _ = parts.next();
        let namespace = parts.next().ok_or(IdUriError::MissingNamespace)?;
        let namespace = parse_namespace(namespace)?;
        let segments = parts
            .map(|segment| {
                let canonical = normalise(segment, Component::Path, is_pchar)?;
                if canonical == "." || canonical == ".." {
                    return Err(IdUriError::DotSegment {
                        segment: segment.to_owned(),
                    });
                }
                Ok(canonical)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let query = query
            .map(|q| normalise(q, Component::Query, is_query_char))
            .transpose()?;
        let fragment = fragment
            .map(|f| normalise(f, Component::Fragment, is_query_char))
            .transpose()?;

        Ok(Self {
            host,
            port,
            namespace,
            segments,
            query,
            fragment,
        })
    }

    /// The host: lowercase, IDNA names as punycode, IPv6 in brackets.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The port, if the URL names one.
    #[must_use]
    pub const fn port(&self) -> Option<u16> {
        self.port
    }

    /// The namespace name, lowercase.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// The namespace-path segments in canonical percent-encoded form.
    /// A trailing slash shows as a final empty segment.
    #[must_use]
    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    /// The namespace-path segments percent-decoded to bytes (for the
    /// `b64` encoding and other binary cores).
    #[must_use]
    pub fn segment_bytes(&self) -> Vec<Vec<u8>> {
        self.segments.iter().map(|s| percent_decode(s)).collect()
    }

    /// The namespace-path segments percent-decoded as UTF-8 text (the
    /// `text` encoding). A segment that is not valid UTF-8 is an
    /// [`IdUriError::InvalidUtf8`], never a lossy substitution.
    pub fn text_segments(&self) -> Result<Vec<String>, IdUriError> {
        self.segment_bytes()
            .into_iter()
            .enumerate()
            .map(|(index, bytes)| {
                String::from_utf8(bytes).map_err(|_err| IdUriError::InvalidUtf8 { index })
            })
            .collect()
    }

    /// The query in canonical form, without the `?`.
    #[must_use]
    pub fn query(&self) -> Option<&str> {
        self.query.as_deref()
    }

    /// The fragment in canonical form, without the `#`. Client-side
    /// only: it is never sent to a resolver.
    #[must_use]
    pub fn fragment(&self) -> Option<&str> {
        self.fragment.as_deref()
    }

    /// The same URL with its fragment stripped: the form a resolver sees.
    #[must_use]
    pub fn without_fragment(&self) -> Self {
        Self {
            fragment: None,
            ..self.clone()
        }
    }
}

impl FromStr for IdUri {
    type Err = IdUriError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl fmt::Display for IdUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{SCHEME}{}", self.host)?;
        if let Some(port) = self.port {
            write!(f, ":{port}")?;
        }
        write!(f, "/{}", self.namespace)?;
        for segment in &self.segments {
            write!(f, "/{segment}")?;
        }
        if let Some(query) = &self.query {
            write!(f, "?{query}")?;
        }
        if let Some(fragment) = &self.fragment {
            write!(f, "#{fragment}")?;
        }
        Ok(())
    }
}

fn parse_authority(authority: &str) -> Result<(String, Option<u16>), IdUriError> {
    if authority.contains('@') {
        return Err(IdUriError::UserinfoPresent);
    }
    let (host, port) = if authority.starts_with('[') {
        let close = authority.find(']').ok_or_else(|| IdUriError::BadHost {
            host: authority.to_owned(),
        })?;
        let (literal, after) = authority.split_at(close + 1);
        let port = match after {
            "" => None,
            _ => Some(after.strip_prefix(':').ok_or_else(|| IdUriError::BadHost {
                host: authority.to_owned(),
            })?),
        };
        (parse_ipv6_literal(literal)?, port)
    } else {
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        };
        (parse_reg_name(host)?, port)
    };
    let port = match port {
        // RFC 3986 §6.2.3: an empty port is the same as no port.
        None | Some("") => None,
        Some(digits) if digits.bytes().all(|b| b.is_ascii_digit()) => {
            Some(digits.parse::<u16>().map_err(|_err| IdUriError::BadPort {
                port: digits.to_owned(),
            })?)
        }
        Some(other) => {
            return Err(IdUriError::BadPort {
                port: other.to_owned(),
            });
        }
    };
    Ok((host, port))
}

fn parse_ipv6_literal(literal: &str) -> Result<String, IdUriError> {
    let bad = || IdUriError::BadHost {
        host: literal.to_owned(),
    };
    let inner = literal
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(bad)?;
    let addr: Ipv6Addr = inner.parse().map_err(|_err| bad())?;
    Ok(format!("[{addr}]"))
}

fn parse_reg_name(host: &str) -> Result<String, IdUriError> {
    if host.is_empty() {
        return Err(IdUriError::EmptyHost);
    }
    // A reg-name may be percent-encoded (RFC 3986 §3.2.2); decode it so
    // IDNA sees the name itself.
    for (at, ch) in host.char_indices() {
        if ch == '%' {
            if !has_hex_pair(host, at) {
                return Err(IdUriError::MalformedPercentEncoding {
                    component: Component::Host,
                    at,
                });
            }
        } else if ch.is_ascii() && !is_unreserved(ch) && !is_sub_delim(ch) {
            return Err(IdUriError::InvalidCharacter {
                component: Component::Host,
                ch,
            });
        }
    }
    let decoded = String::from_utf8(percent_decode(host)).map_err(|_err| IdUriError::BadHost {
        host: host.to_owned(),
    })?;
    let ascii = idna::domain_to_ascii(&decoded).map_err(|_err| IdUriError::BadHost {
        host: host.to_owned(),
    })?;
    if ascii.is_empty() {
        return Err(IdUriError::EmptyHost);
    }
    Ok(ascii.to_ascii_lowercase())
}

fn parse_namespace(raw: &str) -> Result<String, IdUriError> {
    let bad = || IdUriError::BadNamespace {
        segment: raw.to_owned(),
    };
    let canonical = normalise(raw, Component::Namespace, is_pchar)?.to_ascii_lowercase();
    let mut bytes = canonical.bytes();
    let first_is_alpha = bytes.next().is_some_and(|b| b.is_ascii_alphabetic());
    if !first_is_alpha || !bytes.all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err(bad());
    }
    Ok(canonical)
}

/// Validates `raw` against `allowed`, decodes percent-encoded unreserved
/// characters and uppercases the hex of every other percent-encoding.
fn normalise(
    raw: &str,
    component: Component,
    allowed: fn(char) -> bool,
) -> Result<String, IdUriError> {
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.char_indices();
    while let Some((at, ch)) = chars.next() {
        if ch == '%' {
            let pair = bytes
                .get(at + 1..at + 3)
                .filter(|pair| pair.iter().all(u8::is_ascii_hexdigit))
                .ok_or(IdUriError::MalformedPercentEncoding { component, at })?;
            let value = hex_value(pair);
            let decoded = char::from(value);
            if value.is_ascii() && is_unreserved(decoded) {
                out.push(decoded);
            } else {
                out.push('%');
                for b in pair {
                    out.push(char::from(b.to_ascii_uppercase()));
                }
            }
            let _ = chars.next();
            let _ = chars.next();
        } else if allowed(ch) {
            out.push(ch);
        } else {
            return Err(IdUriError::InvalidCharacter { component, ch });
        }
    }
    Ok(out)
}

fn percent_decode(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        match bytes.get(i + 1..i + 3) {
            Some(pair) if b == b'%' && pair.iter().all(u8::is_ascii_hexdigit) => {
                out.push(hex_value(pair));
                i += 3;
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

fn has_hex_pair(s: &str, at: usize) -> bool {
    s.as_bytes()
        .get(at + 1..at + 3)
        .is_some_and(|pair| pair.iter().all(u8::is_ascii_hexdigit))
}

fn hex_value(pair: &[u8]) -> u8 {
    pair.iter().fold(0u8, |acc, &b| {
        let digit = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => 0,
        };
        acc.wrapping_mul(16).wrapping_add(digit)
    })
}

const fn is_unreserved(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | '_' | '~')
}

const fn is_sub_delim(ch: char) -> bool {
    matches!(
        ch,
        '!' | '$' | '&' | '\'' | '(' | ')' | '*' | '+' | ',' | ';' | '='
    )
}

const fn is_pchar(ch: char) -> bool {
    is_unreserved(ch) || is_sub_delim(ch) || matches!(ch, ':' | '@')
}

const fn is_query_char(ch: char) -> bool {
    is_pchar(ch) || matches!(ch, '/' | '?')
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn canonical(s: &str) -> String {
        IdUri::parse(s).unwrap().to_string()
    }

    // Oracle: the design record's grammar (§2) and normalisation rules
    // (§3), each example written out by hand from the record's text.
    #[test]
    fn parses_the_grammar_into_its_parts() {
        let uri = IdUri::parse("id://example.com:8443/docs/a/b%2Fc?rev=3#sec-2").unwrap();
        assert_eq!(uri.host(), "example.com");
        assert_eq!(uri.port(), Some(8443));
        assert_eq!(uri.namespace(), "docs");
        assert_eq!(uri.segments(), ["a", "b%2Fc"]);
        assert_eq!(uri.text_segments().unwrap(), ["a", "b/c"]);
        assert_eq!(uri.query(), Some("rev=3"));
        assert_eq!(uri.fragment(), Some("sec-2"));
    }

    #[test]
    fn normalises_case_where_the_record_says_and_only_there() {
        assert_eq!(
            canonical("ID://Example.COM/Docs/Mixed/Case"),
            "id://example.com/docs/Mixed/Case"
        );
    }

    #[test]
    fn idna_hosts_become_punycode() {
        assert_eq!(
            canonical("id://bücher.example/docs/x"),
            "id://xn--bcher-kva.example/docs/x"
        );
        assert_eq!(
            canonical("id://b%C3%BCcher.example/docs/x"),
            "id://xn--bcher-kva.example/docs/x"
        );
    }

    #[test]
    fn decodes_unreserved_and_uppercases_other_percent_hex() {
        assert_eq!(
            canonical("id://h/ns/%61%7e%2d/%2f%c3%a9"),
            "id://h/ns/a~-/%2F%C3%A9"
        );
        assert_eq!(canonical("id://h/ns/x?q=%3d%41"), "id://h/ns/x?q=%3DA");
    }

    #[test]
    fn trailing_slash_is_significant() {
        let without = IdUri::parse("id://h/ns/a").unwrap();
        let with = IdUri::parse("id://h/ns/a/").unwrap();
        assert_ne!(without, with);
        assert_eq!(with.segments(), ["a", ""]);
        assert_eq!(with.to_string(), "id://h/ns/a/");
        assert_eq!(IdUri::parse("id://h/ns").unwrap().segments(), [] as [String; 0]);
        assert_eq!(IdUri::parse("id://h/ns/").unwrap().segments(), [""]);
    }

    #[test]
    fn ports_and_ipv6_literals() {
        assert_eq!(canonical("id://h:/ns"), "id://h/ns");
        assert_eq!(
            canonical("id://[2001:DB8::1]:7/ns"),
            "id://[2001:db8::1]:7/ns"
        );
        assert_eq!(canonical("id://127.0.0.1/ns"), "id://127.0.0.1/ns");
        assert_eq!(
            IdUri::parse("id://h:65536/ns"),
            Err(IdUriError::BadPort {
                port: "65536".into()
            })
        );
        assert_eq!(
            IdUri::parse("id://h:8x/ns"),
            Err(IdUriError::BadPort { port: "8x".into() })
        );
        assert!(matches!(
            IdUri::parse("id://[zz]/ns"),
            Err(IdUriError::BadHost { .. })
        ));
    }

    #[test]
    fn fragment_is_client_side_only() {
        let uri = IdUri::parse("id://h/ns/a#here").unwrap();
        assert_eq!(uri.without_fragment().to_string(), "id://h/ns/a");
        assert_eq!(uri.without_fragment().fragment(), None);
    }

    #[test]
    fn each_rejection_class_has_its_named_error() {
        let cases: [(&str, IdUriError); 9] = [
            ("https://h/ns", IdUriError::NotIdScheme),
            ("id://user@h/ns", IdUriError::UserinfoPresent),
            ("id://user:pw@h/ns", IdUriError::UserinfoPresent),
            ("id:///ns", IdUriError::EmptyHost),
            ("id://h", IdUriError::MissingNamespace),
            (
                "id://h/",
                IdUriError::BadNamespace {
                    segment: String::new(),
                },
            ),
            (
                "id://h/9lives",
                IdUriError::BadNamespace {
                    segment: "9lives".into(),
                },
            ),
            (
                "id://h/ns/a%2",
                IdUriError::MalformedPercentEncoding {
                    component: Component::Path,
                    at: 1,
                },
            ),
            (
                "id://h/ns/a b",
                IdUriError::InvalidCharacter {
                    component: Component::Path,
                    ch: ' ',
                },
            ),
        ];
        for (input, want) in cases {
            assert_eq!(IdUri::parse(input), Err(want), "{input}");
        }
        assert_eq!(
            IdUri::parse("id://h/ns/%2e%2E"),
            Err(IdUriError::DotSegment {
                segment: "%2e%2E".into()
            })
        );
        assert_eq!(
            IdUri::parse("id://h/ns/."),
            Err(IdUriError::DotSegment {
                segment: ".".into()
            })
        );
        assert_eq!(
            IdUri::parse("id://h/n_s"),
            Err(IdUriError::BadNamespace {
                segment: "n_s".into()
            })
        );
        assert_eq!(
            IdUri::parse("id://h/ns/x?a=%zz"),
            Err(IdUriError::MalformedPercentEncoding {
                component: Component::Query,
                at: 2
            })
        );
        assert_eq!(
            IdUri::parse("id://h/ns/é"),
            Err(IdUriError::InvalidCharacter {
                component: Component::Path,
                ch: 'é'
            })
        );
    }

    #[test]
    fn invalid_utf8_is_an_error_under_text_never_a_substitution() {
        let uri = IdUri::parse("id://h/ns/ok/%FF%FE").unwrap();
        assert_eq!(
            uri.text_segments(),
            Err(IdUriError::InvalidUtf8 { index: 1 })
        );
        assert_eq!(uri.segment_bytes(), [b"ok".to_vec(), vec![0xFF, 0xFE]]);
    }

    #[test]
    fn over_length_is_rejected_never_truncated() {
        let base = "id://h/ns/";
        let at_limit = format!("{base}{}", "a".repeat(MAX_ID_URI_LEN - base.len()));
        assert!(IdUri::parse(&at_limit).is_ok());
        let over = format!("{at_limit}a");
        assert_eq!(
            IdUri::parse(&over),
            Err(IdUriError::TooLong {
                len: MAX_ID_URI_LEN + 1,
                max: MAX_ID_URI_LEN
            })
        );
    }

    // Oracle: round-trip identity. Canonical serialisation of a parse
    // parses back to an equal value and serialises to the same text.
    #[test]
    fn canonical_form_round_trips() {
        for input in [
            "ID://Example.com:443/Docs/a/b%2fc/?x=%41#Frag",
            "id://[::1]/n/%7Euser/",
            "id://h/ns",
            "id://h/ns/a:b@c!$&'()*+,;=",
            "id://xn--bcher-kva.example/text/%E2%9C%93",
        ] {
            let first = IdUri::parse(input).unwrap();
            let text = first.to_string();
            let second = IdUri::parse(&text).unwrap();
            assert_eq!(first, second, "{input}");
            assert_eq!(second.to_string(), text, "{input}");
        }
    }

    mod properties {
        use super::super::*;
        use proptest::prelude::*;

        fn segment() -> impl Strategy<Value = String> {
            proptest::collection::vec(any::<u8>(), 0..12).prop_map(|bytes| {
                bytes
                    .iter()
                    .map(|b| {
                        let ch = char::from(*b);
                        if b.is_ascii() && is_unreserved(ch) && ch != '.' {
                            ch.to_string()
                        } else {
                            format!("%{b:02x}")
                        }
                    })
                    .collect()
            })
        }

        proptest! {
            // Invariant: any URL built from valid parts parses, its
            // canonical form is a fixed point, and the decoded segment
            // bytes survive the round trip.
            #[test]
            fn canonical_is_a_fixed_point(
                ns in "[a-zA-Z][a-zA-Z0-9-]{0,8}",
                segments in proptest::collection::vec(segment(), 0..5),
                port in proptest::option::of(any::<u16>()),
            ) {
                let mut input = String::from("id://Host.Example");
                if let Some(port) = port {
                    input.push(':');
                    input.push_str(&port.to_string());
                }
                input.push('/');
                input.push_str(&ns);
                for s in &segments {
                    input.push('/');
                    input.push_str(s);
                }
                let parsed = match IdUri::parse(&input) {
                    Ok(parsed) => parsed,
                    // "%2e"/"%2E" alone, or ".." spelled with escapes, is a dot segment.
                    Err(IdUriError::DotSegment { .. }) => return Ok(()),
                    Err(other) => panic!("{input}: {other}"),
                };
                let text = parsed.to_string();
                let again = IdUri::parse(&text).unwrap();
                prop_assert_eq!(&again, &parsed);
                prop_assert_eq!(again.to_string(), text);
                let want: Vec<Vec<u8>> = segments.iter().map(|s| percent_decode(s)).collect();
                prop_assert_eq!(parsed.segment_bytes(), want);
            }
        }
    }
}
