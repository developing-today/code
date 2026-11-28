//! Error mapping and the grantee collapse (design #5805, §6; slice
//! #13762).
//!
//! Every failure the adapter can meet — a refusal before any network call,
//! an HTTP status from `PostgREST`, a body that is not JSON, a deadline, a
//! dropped connection, a local capacity limit, or a value outside the
//! typed mapping — is a [`Failure`]. [`Failure::to_error`] turns it into
//! an [`AdapterError`] with a stable [`ErrorCode`] (§6.1) for one
//! [`Audience`]:
//!
//! - Outcomes that depend only on the query text (`query_shape_refused`,
//!   `limit_exceeded`) or on load (`rate_limited`, `timeout`,
//!   `adapter_unavailable`) are shown to a grantee as they are.
//! - Anything that depends on the data or the grant — a 401 or 403, a 404,
//!   any other 4xx — is collapsed to `not_available` for a grantee (§6.2),
//!   with a body byte-identical to the pre-network `not_available` for an
//!   unlisted relation. The owner sees the real code (`credential_unavailable`
//!   for a 401 or 403) and a short diagnostic.
//!
//! Diagnostics are fixed strings plus, at most, an HTTP status number. No
//! error, for either audience, carries the source's response text, a
//! header, a value, or the credential (§2, §9 invariants 4 and 6).

use std::fmt::{self, Write as _};

use serde::{Deserialize, Serialize};

use crate::refusal::{Feature, Refusal, RefusalCode};

/// The stable error codes the adapter emits (design §6.1, from #5803 §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    /// The query shape is outside the §3.1 supported set.
    QueryShapeRefused,
    /// The requested limit exceeds the collection's declared cap.
    LimitExceeded,
    /// The collapsed outcome for anything that depends on the data or the
    /// grant.
    NotAvailable,
    /// The source refused the credential (owner view of a 401 or 403).
    CredentialUnavailable,
    /// A 429 from the source, or a configured capacity limit.
    RateLimited,
    /// The request deadline passed. Never an empty result.
    Timeout,
    /// A connection failure, a 5xx, a redirect, or a non-JSON body.
    AdapterUnavailable,
    /// A value that fails its declared type.
    TypeMappingFailed,
}

impl ErrorCode {
    /// The stable wire name of the code.
    pub const fn name(self) -> &'static str {
        match self {
            Self::QueryShapeRefused => "query_shape_refused",
            Self::LimitExceeded => "limit_exceeded",
            Self::NotAvailable => "not_available",
            Self::CredentialUnavailable => "credential_unavailable",
            Self::RateLimited => "rate_limited",
            Self::Timeout => "timeout",
            Self::AdapterUnavailable => "adapter_unavailable",
            Self::TypeMappingFailed => "type_mapping_failed",
        }
    }
}

impl From<RefusalCode> for ErrorCode {
    fn from(code: RefusalCode) -> Self {
        match code {
            RefusalCode::QueryShapeRefused => Self::QueryShapeRefused,
            RefusalCode::LimitExceeded => Self::LimitExceeded,
            RefusalCode::NotAvailable => Self::NotAvailable,
        }
    }
}

/// Who the error is shown to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Audience {
    /// The endpoint's owner, who may see diagnostics about the source.
    Owner,
    /// A grantee, who sees the §6.2 collapse.
    Grantee,
}

/// Something that stopped a read, before or after the network call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The translator refused the query before any network call.
    Refused(Refusal),
    /// `PostgREST` answered with a status outside 2xx.
    Status(u16),
    /// `PostgREST` answered 2xx, but the body is not JSON.
    NonJsonBody,
    /// The per-request deadline passed.
    Deadline,
    /// The connection failed (DNS, TCP, TLS, or a reset).
    Connection,
    /// The endpoint's configured concurrency or capacity limit was hit.
    CapacityLimit,
    /// A value failed its declared type. The value itself is never kept.
    TypeMapping {
        /// The declared column.
        column: String,
        /// The column's declared type.
        declared_type: String,
    },
}

/// Classify a `PostgREST` response before its body is used: a 2xx with a
/// JSON body is a success; anything else is a [`Failure`]. A non-JSON
/// 200 is an error, never an empty result (§6.1, §9 invariant 8).
pub const fn classify_response(status: u16, body_is_json: bool) -> Result<(), Failure> {
    match status {
        200..=299 if body_is_json => Ok(()),
        200..=299 => Err(Failure::NonJsonBody),
        _ => Err(Failure::Status(status)),
    }
}

/// An error as shown to one audience.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterError {
    /// The stable code.
    pub code: ErrorCode,
    /// The refused feature, for `query_shape_refused` and
    /// `limit_exceeded`.
    pub feature: Option<Feature>,
    /// For `type_mapping_failed`: the column and its declared type. The
    /// value is never included.
    pub type_mapping: Option<(String, String)>,
    /// A short diagnostic for the owner. Always `None` for a grantee.
    pub detail: Option<String>,
}

impl AdapterError {
    const fn of(code: ErrorCode) -> Self {
        Self {
            code,
            feature: None,
            type_mapping: None,
            detail: None,
        }
    }

    fn with_detail(mut self, audience: Audience, detail: String) -> Self {
        if audience == Audience::Owner {
            self.detail = Some(detail);
        }
        self
    }

    /// The stable error body. For a grantee it depends only on the code,
    /// the feature name, and (for `type_mapping_failed`) the declared
    /// column and type — so two errors that the collapse rule merges are
    /// byte-identical.
    pub fn body(&self) -> String {
        let mut body = format!("{{\"code\":{}", json_string(self.code.name()));
        if let Some(feature) = self.feature {
            let _ = write!(body, ",\"feature\":{}", json_string(feature.name()));
        }
        if let Some((column, declared_type)) = &self.type_mapping {
            let _ = write!(
                body,
                ",\"column\":{},\"type\":{}",
                json_string(column),
                json_string(declared_type)
            );
        }
        if let Some(detail) = &self.detail {
            let _ = write!(body, ",\"detail\":{}", json_string(detail));
        }
        body.push('}');
        body
    }
}

impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.body())
    }
}

impl std::error::Error for AdapterError {}

impl Failure {
    /// Map this failure to the error `audience` sees (§6.1, §6.2).
    pub fn to_error(&self, audience: Audience) -> AdapterError {
        match self {
            Self::Refused(refusal) => AdapterError {
                feature: refusal.feature,
                ..AdapterError::of(refusal.code.into())
            },
            Self::Status(status @ (401 | 403)) => match audience {
                Audience::Owner => AdapterError::of(ErrorCode::CredentialUnavailable)
                    .with_detail(audience, format!("source returned HTTP {status}")),
                Audience::Grantee => AdapterError::of(ErrorCode::NotAvailable),
            },
            Self::Status(429) => AdapterError::of(ErrorCode::RateLimited)
                .with_detail(audience, "source returned HTTP 429".to_owned()),
            Self::Status(status @ 500..=599) => AdapterError::of(ErrorCode::AdapterUnavailable)
                .with_detail(audience, format!("source returned HTTP {status}")),
            Self::Status(status @ 300..=399) => AdapterError::of(ErrorCode::AdapterUnavailable)
                .with_detail(
                    audience,
                    format!("source returned HTTP {status}; redirects are refused"),
                ),
            Self::Status(status @ 400..=499) => AdapterError::of(ErrorCode::NotAvailable)
                .with_detail(audience, format!("source returned HTTP {status}")),
            Self::Status(status) => AdapterError::of(ErrorCode::AdapterUnavailable).with_detail(
                audience,
                format!("source returned unexpected HTTP {status}"),
            ),
            Self::NonJsonBody => AdapterError::of(ErrorCode::AdapterUnavailable)
                .with_detail(audience, "source returned a non-JSON body".to_owned()),
            Self::Deadline => AdapterError::of(ErrorCode::Timeout)
                .with_detail(audience, "request deadline exceeded".to_owned()),
            Self::Connection => AdapterError::of(ErrorCode::AdapterUnavailable)
                .with_detail(audience, "connection to the source failed".to_owned()),
            Self::CapacityLimit => AdapterError::of(ErrorCode::RateLimited)
                .with_detail(audience, "endpoint capacity limit reached".to_owned()),
            Self::TypeMapping {
                column,
                declared_type,
            } => AdapterError {
                type_mapping: Some((column.clone(), declared_type.clone())),
                ..AdapterError::of(ErrorCode::TypeMappingFailed)
            },
        }
    }
}

/// Encode `s` as a JSON string literal.
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn json_string_escapes_quotes_and_controls() {
        assert_eq!(json_string("a\"b\\c\n\u{1}"), "\"a\\\"b\\\\c\\n\\u0001\"");
    }
}
