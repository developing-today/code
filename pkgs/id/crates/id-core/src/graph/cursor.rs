//! Snapshot query cursors (slice A-GRAPH-9126-01, #10221; design part A
//! §5.1, statements S5.8, S5.10, S5.11).
//!
//! A cursor is opaque to callers. Inside, it is the canonical CBOR array
//! `[1, frontier: bstr(32), query: bstr(32), after: uint]`: the snapshot
//! frontier the first page was read at, a digest of the query it
//! continues, and how many rows of the stable total order were already
//! returned. A cursor is honoured only at the same frontier
//! ([`CursorError::Expired`] otherwise) and for the same query
//! ([`CursorError::Mismatch`] otherwise, including any bytes that are
//! not a cursor). Because the order is total and the snapshot is fixed,
//! consecutive pages concatenate without omission or duplication.

use crate::canonical_cbor::{Value, decode_canonical, encode_canonical};

/// Cursor format version.
pub const CURSOR_VERSION: u64 = 1;

/// Identity of a fixed snapshot of assertions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Frontier(pub [u8; 32]);

/// An opaque continuation token for the next page of one query.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Cursor(Vec<u8>);

/// Why a cursor was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorError {
    /// The cursor belongs to another snapshot frontier.
    Expired,
    /// The cursor belongs to another query, or is not a cursor.
    Mismatch,
}

impl Cursor {
    /// A cursor resuming `query` at `frontier` after `after` rows.
    #[must_use]
    pub fn new(frontier: Frontier, query: [u8; 32], after: u64) -> Self {
        let value = Value::Array(vec![
            Value::UInt(CURSOR_VERSION),
            Value::Bytes(frontier.0.to_vec()),
            Value::Bytes(query.to_vec()),
            Value::UInt(after),
        ]);
        Self(encode_canonical(&value).unwrap_or_default())
    }

    /// Wraps cursor bytes received from a caller.
    #[must_use]
    pub const fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The opaque bytes to hand to a caller.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The row offset this cursor resumes after, if it continues `query`
    /// at `frontier`.
    ///
    /// # Errors
    ///
    /// [`CursorError`] when the cursor is malformed, for another query,
    /// or for another frontier.
    pub fn resume(&self, frontier: Frontier, query: [u8; 32]) -> Result<u64, CursorError> {
        let Ok(Value::Array(items)) = decode_canonical(&self.0) else {
            return Err(CursorError::Mismatch);
        };
        let [
            Value::UInt(CURSOR_VERSION),
            Value::Bytes(at),
            Value::Bytes(for_query),
            Value::UInt(after),
        ] = items.as_slice()
        else {
            return Err(CursorError::Mismatch);
        };
        if for_query.as_slice() != query.as_slice() {
            return Err(CursorError::Mismatch);
        }
        if at.as_slice() != frontier.0.as_slice() {
            return Err(CursorError::Expired);
        }
        Ok(*after)
    }
}
