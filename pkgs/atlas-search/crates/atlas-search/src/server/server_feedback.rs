//! Feedback HTTP DTOs for the Atlas search service.
//! Source of the verbatim copy in pkgs/fleet's fleet-search crate,
//! split from server.rs in #10446 to satisfy the production-line
//! limit: mirror every change there. Drift guard:
//! .github/scripts/check-fleet-search-parity.sh.

use atlas_core::{FeedbackInput, Vote};
use serde::Deserialize;

/// One feedback submission (design section 6.1): a thumbs vote on
/// `(query_id, chunk_id)` — `chunk_id` omitted or null is a
/// per-query vote — with an optional verbatim comment and the
/// surface's `opened` report (section 6.4).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FeedbackRequest {
    pub(crate) query_id: String,
    pub(crate) chunk_id: Option<i64>,
    pub(crate) vote: String,
    pub(crate) comment: Option<String>,
    pub(crate) opened: Option<bool>,
    pub(crate) requester: Option<FeedbackRequester>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FeedbackRequester {
    pub(crate) kind: Option<String>,
    pub(crate) id: Option<String>,
}

impl FeedbackRequest {
    pub(crate) fn into_input(self) -> Result<FeedbackInput, String> {
        let vote = Vote::parse(&self.vote).map_err(|e| e.to_string())?;
        let (kind, id) = match self.requester {
            Some(r) => (
                r.kind.unwrap_or_else(|| "anonymous".to_string()),
                r.id.unwrap_or_default(),
            ),
            None => ("anonymous".to_string(), String::new()),
        };
        let input = FeedbackInput {
            query_id: self.query_id,
            chunk_id: self.chunk_id,
            vote,
            comment: self.comment,
            requester_kind: kind,
            requester_id: id,
            opened: self.opened.unwrap_or(false),
        };
        input.validate().map_err(|e| e.to_string())?;
        Ok(input)
    }
}
