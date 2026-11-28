//! HTTP-facing search error type.
//!
//! Split from [`super`] (#16255) to keep `server` under the
//! production line limit.

use atlas_core::Error as CoreError;
use atlas_proto::ErrorResponse;

// ------------------------------------------------------------- error ----

#[derive(Debug, Clone)]
pub struct ServerError {
    pub status: u16,
    pub body: ErrorResponse,
}

impl ServerError {
    pub fn new(status: u16, code: &str, message: impl Into<String>) -> ServerError {
        ServerError {
            status,
            body: ErrorResponse::new(code, message.into()),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> ServerError {
        ServerError::new(400, "bad_request", message)
    }

    pub fn unauthorized(message: impl Into<String>) -> ServerError {
        ServerError::new(401, "unauthorized", message)
    }

    pub fn forbidden(message: impl Into<String>) -> ServerError {
        ServerError::new(403, "forbidden", message)
    }

    pub fn job_expired() -> ServerError {
        ServerError::new(
            404,
            "job_expired_or_unknown",
            "job is expired or unknown; resubmit the search",
        )
    }

    pub fn job_gone() -> ServerError {
        ServerError::new(
            410,
            "job_expired",
            "job expired before its results were read; resubmit the search",
        )
    }

    pub fn rate_limited(retry_after_seconds: u64, queue_depth: usize) -> ServerError {
        ServerError {
            status: 429,
            body: ErrorResponse::rate_limited(
                "atlas service is saturated; retry after the indicated delay",
                retry_after_seconds,
                queue_depth,
            ),
        }
    }

    pub fn from_core(e: &CoreError) -> ServerError {
        match e {
            CoreError::Fatal(m) => ServerError::new(400, "search_rejected", m.clone()),
            CoreError::Backend(m) | CoreError::Embed(m) => {
                ServerError::new(500, "search_failed", m.clone())
            }
        }
    }
}
