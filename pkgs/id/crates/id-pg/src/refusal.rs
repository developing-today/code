//! Refusals: stable codes and feature names (design §3.2, §6).
//!
//! A refusal's grantee-visible body depends only on its code and
//! feature name — never on a column name, a value, or on whether a
//! column was withheld or simply absent. That is what makes an
//! undeclared column and a withheld column produce byte-identical
//! error bodies (§6.2), which the §10 model tests assert.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The stable error code (design §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefusalCode {
    /// The query shape is outside the §3.1 supported set. Shown as it
    /// is: it depends only on the query text.
    QueryShapeRefused,
    /// The requested limit exceeds the collection's declared cap.
    /// Refused, never clamped (§3.2, §6.2).
    LimitExceeded,
    /// The collapsed outcome for anything that depends on the data or
    /// the grant: unknown relation, unlisted relation (§6.2).
    NotAvailable,
}

impl RefusalCode {
    /// The stable wire name of the code.
    pub fn name(self) -> &'static str {
        match self {
            RefusalCode::QueryShapeRefused => "query_shape_refused",
            RefusalCode::LimitExceeded => "limit_exceeded",
            RefusalCode::NotAvailable => "not_available",
        }
    }
}

/// A refused query shape, named by its stable feature name (§3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Feature {
    /// Joins and PostgREST embeds.
    JoinUnsupported,
    /// OR, NOT, or nested boolean trees.
    BooleanUnsupported,
    /// `like`, `ilike`, `match`, and the full-text operators.
    TextPatternUnsupported,
    /// Offset or page-number pagination.
    OffsetUnsupported,
    /// Exact or planned counts.
    CountUnsupported,
    /// RPC (`/rpc/*`).
    RpcUnsupported,
    /// Writes (`POST`, `PATCH`, `PUT`, `DELETE`, upsert).
    WriteUnsupported,
    /// Schema or OpenAPI discovery (`GET /`).
    DiscoveryUnsupported,
    /// Ordering, filtering, or projecting on an undeclared column —
    /// the same name for a withheld column (§6.1).
    FieldUnsupported,
    /// Raw SQL, or any `/query` endpoint.
    SqlUnsupported,
    /// A limit above the declared cap.
    LimitExceeded,
}

impl Feature {
    /// The stable feature name as it appears in the error (§3.2).
    pub fn name(self) -> &'static str {
        match self {
            Feature::JoinUnsupported => "join_unsupported",
            Feature::BooleanUnsupported => "boolean_unsupported",
            Feature::TextPatternUnsupported => "text_pattern_unsupported",
            Feature::OffsetUnsupported => "offset_unsupported",
            Feature::CountUnsupported => "count_unsupported",
            Feature::RpcUnsupported => "rpc_unsupported",
            Feature::WriteUnsupported => "write_unsupported",
            Feature::DiscoveryUnsupported => "discovery_unsupported",
            Feature::FieldUnsupported => "field_unsupported",
            Feature::SqlUnsupported => "sql_unsupported",
            Feature::LimitExceeded => "limit_exceeded",
        }
    }
}

/// A query refused before any network call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    /// The stable error code.
    pub code: RefusalCode,
    /// The refused feature, when the code carries one. `not_available`
    /// carries none: naming a feature would distinguish outcomes the
    /// collapse rule merges.
    pub feature: Option<Feature>,
}

impl Refusal {
    /// A `query_shape_refused` refusal naming `feature`.
    pub fn shape(feature: Feature) -> Self {
        Refusal {
            code: RefusalCode::QueryShapeRefused,
            feature: Some(feature),
        }
    }

    /// The `limit_exceeded` refusal (§3.2, §6.2).
    pub fn limit_exceeded() -> Self {
        Refusal {
            code: RefusalCode::LimitExceeded,
            feature: Some(Feature::LimitExceeded),
        }
    }

    /// The collapsed `not_available` refusal.
    pub fn not_available() -> Self {
        Refusal {
            code: RefusalCode::NotAvailable,
            feature: None,
        }
    }

    /// The stable grantee-visible error body. It is a function of the
    /// code and feature name only, so two refusals with the same code
    /// and feature are byte-identical no matter what the query named.
    pub fn body(&self) -> String {
        match self.feature {
            Some(feature) => format!(
                "{{\"code\":\"{}\",\"feature\":\"{}\"}}",
                self.code.name(),
                feature.name()
            ),
            None => format!("{{\"code\":\"{}\"}}", self.code.name()),
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.body())
    }
}

impl std::error::Error for Refusal {}
