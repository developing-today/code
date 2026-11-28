//! Turso/libsql HTTP adapter v1 — read-only foreign data source for ID.
//!
//! Implements the decision record on issue #5806 (B3 under the #5672 Part B
//! foreign data-store adapter contract) for issue #7577:
//!
//! - Raw libsql HTTP protocol (v2 pipeline, JSON) implemented here — not the
//!   libsql crate, not Hrana-over-WebSocket. No SQLite-family dependency
//!   enters the shipped graph; the guard test
//!   `test_the_lock_has_no_sqlite_with_logsync_wired` in
//!   [`crate::store_traits`] keeps passing unmodified, and `FUEL_BUDGETS`
//!   is untouched.
//! - Read-only in v1. Queries are built from a structured [`GrantedQuery`],
//!   so a write shape cannot be expressed; predicate fragments carrying
//!   write keywords or statement separators are refused.
//! - Live pass-through only. There is no materialized cache and no silent
//!   fallback to stale data.
//! - A foreign store is addressed as `id://foreign/libsql/<alias>`. The
//!   alias resolves only in local operator config ([`ForeignConfig`]) to an
//!   HTTPS URL and a credential *reference*. The token is fetched through
//!   the reference at call time; neither the URL nor any token ever enters
//!   ID data, payloads, or logs, and errors name the credential reference,
//!   never the token.
//! - Grant filtering is pushed into the remote query itself. The adapter
//!   never fetches ungranted rows to filter locally, exposes only the
//!   granted view schema, and returns identical results for not-found and
//!   not-granted (an empty row set), including through its error classes:
//!   server response bodies are never copied into errors.
//! - Public reads of published Turso collections (decision record #5802
//!   D4) go through [`LibsqlAdapter::query_public`]: the publication
//!   record's explicit collection set ([`PublicationScope`]) gates the
//!   read before any fetch, each collection is served in its declared
//!   mode (live, or a materialized public replica store), and readers
//!   see only the two named outcomes of [`PublicReadError`] — refused
//!   or unavailable — never a credential, a locator, or stale data
//!   served as current.
//! - Materialized replicas are maintained by [`ReplicaMaintainer`],
//!   a separate owner-side type: the read adapter above stays
//!   read-only by construction, and every write the maintainer
//!   makes is built from validated identifiers, the source's own
//!   schema, and wire values — never caller-supplied SQL
//!   (decision record #5802 D4/D5).
//! - A slow or dead foreign store never blocks native ID operations:
//!   calls are isolated behind per-alias circuit breakers with bounded
//!   retries for reads only; auth failure is fatal and never retried.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use id_grants::{PublicationRecord, Scope};
use id_sync::serving_limits::{
    FetchCap, FetchPage, OwnerAggregateLimits, ServingGate, ServingRefusal, ShareServingLimits,
};
use serde_json::{Value as Json, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Default per-statement deadline (record #5806 item 7).
pub const DEFAULT_STATEMENT_TIMEOUT: Duration = Duration::from_secs(30);
/// Connect timeout (record #5806 item 7).
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Bounded read retries: total attempts per query, first try included.
pub const MAX_ATTEMPTS: u32 = 3;
/// Consecutive transient failures that open a per-alias circuit breaker.
pub const BREAKER_FAILURE_THRESHOLD: u32 = 5;
/// How long an open breaker waits before a half-open probe is allowed.
pub const BREAKER_HALF_OPEN_AFTER: Duration = Duration::from_secs(30);
/// Largest single BLOB the adapter will send: servers reject very large
/// BLOBs, so an oversize payload is refused with a named size error
/// instead of being truncated (record #5806 item 5).
pub const MAX_BLOB_BYTES: usize = 1_000_000;
/// Largest pipeline response body the transport will buffer.
const MAX_RESPONSE_BYTES: usize = 16 * 1_024 * 1_024;
/// URI prefix for libsql foreign stores (record #5806 item 4).
pub const FOREIGN_LIBSQL_PREFIX: &str = "id://foreign/libsql/";

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why an adapter call failed. The class is the contract: deterministic
/// classes surface immediately, transient classes are retried (reads only)
/// and feed the circuit breaker, and auth is fatal. No variant carries a
/// server response body, a token, or a resolved URL.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AdapterError {
    /// The alias is not present in local operator config.
    #[error("unknown foreign-store alias: {0}")]
    UnknownAlias(String),
    /// The foreign URI or local config is malformed.
    #[error("foreign-store config error: {0}")]
    Config(String),
    /// Auth failed (or the credential could not be resolved). Fatal, never
    /// retried. Names the credential reference, never the token.
    #[error("authentication failed for credential reference {credential_ref}")]
    Auth {
        /// The credential reference that failed (never the token itself).
        credential_ref: String,
    },
    /// A write was attempted. v1 is read-only; writes are refused
    /// explicitly, never degraded (record #5806 item 2).
    #[error("writes to a foreign store are refused in adapter v1")]
    WriteRefused,
    /// The query shape cannot be enforced without leaking ungranted data,
    /// or is not a single read-only shape (record #5806 item 8).
    #[error("query refused: {0}")]
    QueryRefused(String),
    /// A value crossed the boundary with the wrong type. A type mismatch
    /// is an error, never a coercion (record #5806 item 5).
    #[error("type mismatch on column {column}: expected {expected}, found {found}")]
    TypeMismatch {
        /// Column whose value mismatched (an alias-side schema name only).
        column: String,
        /// Contract type name.
        expected: &'static str,
        /// Wire type actually returned.
        found: &'static str,
    },
    /// A BLOB exceeded the size the adapter will send.
    #[error("blob of {size} bytes exceeds the {max} byte adapter limit")]
    SizeRefused {
        /// Actual size in bytes.
        size: usize,
        /// Configured maximum in bytes.
        max: usize,
    },
    /// A REAL value was NaN or infinite and cannot cross as JSON.
    #[error("non-finite REAL value cannot cross the libsql boundary")]
    NonFiniteReal,
    /// The remote answered with a deterministic statement error (4xx other
    /// than auth, or a pipeline error result). Body text is not copied.
    #[error("remote statement failed with status {status}")]
    RemoteStatement {
        /// HTTP status, or 0 for a pipeline-level error result.
        status: u16,
    },
    /// The response was not a well-formed libsql v2 pipeline response.
    #[error("malformed libsql pipeline response")]
    MalformedResponse,
    /// Transient network or 5xx failure; retried with bounded backoff and
    /// counted by the circuit breaker.
    #[error("transient foreign-store failure: {0}")]
    Transient(String),
    /// The per-statement deadline elapsed.
    #[error("foreign-store statement deadline elapsed")]
    Timeout,
    /// The per-alias circuit breaker is open.
    #[error("circuit breaker open for alias {0}")]
    CircuitOpen(String),
}

impl AdapterError {
    /// Whether this class is transient (retryable for reads, breaker food).
    #[must_use]
    pub const fn is_transient(&self) -> bool {
        matches!(self, Self::Transient(_) | Self::Timeout)
    }
}

// ---------------------------------------------------------------------------
// Values: the wire surface and the typed contract surface
// ---------------------------------------------------------------------------

/// A value exactly as it crosses the libsql wire: the five libsql types.
/// Wire JSON form (v2 pipeline): `{"type":"integer","value":"42"}` —
/// integers are strings, blobs are base64, floats are JSON numbers.
#[derive(Debug, Clone, PartialEq)]
pub enum WireValue {
    /// SQL NULL.
    Null,
    /// libsql INTEGER (i64).
    Integer(i64),
    /// libsql REAL (f64).
    Real(f64),
    /// libsql TEXT (valid UTF-8 by construction).
    Text(String),
    /// libsql BLOB (raw bytes).
    Blob(Vec<u8>),
}

impl WireValue {
    /// The wire type name, for error messages.
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Integer(_) => "integer",
            Self::Real(_) => "real",
            Self::Text(_) => "text",
            Self::Blob(_) => "blob",
        }
    }

    /// Serialize to the libsql v2 pipeline JSON value form.
    #[must_use]
    pub fn to_json(&self) -> Json {
        match self {
            Self::Null => json!({"type": "null"}),
            Self::Integer(v) => json!({"type": "integer", "value": v.to_string()}),
            Self::Real(v) => json!({"type": "float", "value": v}),
            Self::Text(v) => json!({"type": "text", "value": v}),
            Self::Blob(v) => {
                json!({"type": "blob", "value": base64_encode(v)})
            }
        }
    }

    /// Parse the libsql v2 pipeline JSON value form. This validates the
    /// wire envelope only; the column contract is checked in
    /// [`IdValue::decode`].
    pub fn from_json(value: &Json) -> Result<Self, AdapterError> {
        let kind = value
            .get("type")
            .and_then(Json::as_str)
            .ok_or(AdapterError::MalformedResponse)?;
        match kind {
            "null" => Ok(Self::Null),
            "integer" => {
                let raw = value.get("value").ok_or(AdapterError::MalformedResponse)?;
                let parsed = match raw {
                    Json::String(s) => s
                        .parse::<i64>()
                        .map_err(|_| AdapterError::MalformedResponse),
                    Json::Number(n) => n.as_i64().ok_or(AdapterError::MalformedResponse),
                    _ => Err(AdapterError::MalformedResponse),
                };
                parsed.map(Self::Integer)
            }
            "float" => {
                let raw = value.get("value").ok_or(AdapterError::MalformedResponse)?;
                let parsed = match raw {
                    Json::Number(n) => n.as_f64(),
                    Json::String(s) => s.parse::<f64>().ok(),
                    _ => None,
                };
                parsed
                    .map(Self::Real)
                    .ok_or(AdapterError::MalformedResponse)
            }
            "text" => value
                .get("value")
                .and_then(Json::as_str)
                .map(|s| Self::Text(s.to_owned()))
                .ok_or(AdapterError::MalformedResponse),
            "blob" => {
                let raw = value
                    .get("value")
                    .and_then(Json::as_str)
                    .ok_or(AdapterError::MalformedResponse)?;
                base64_decode(raw)
                    .map(Self::Blob)
                    .ok_or(AdapterError::MalformedResponse)
            }
            _ => Err(AdapterError::MalformedResponse),
        }
    }
}

/// The contract type of one granted column (record #5806 item 5). Every
/// returned value is validated against it; affinity coercion by the server
/// surfaces as [`AdapterError::TypeMismatch`], never a silent conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnType {
    /// Raw bytes (hashes, signatures, keys, CBOR/MessagePack payloads).
    Blob,
    /// Signed 64-bit integer.
    Integer,
    /// Unsigned 64-bit: INTEGER up to `i64::MAX`, above that an 8-byte
    /// big-endian BLOB. Never REAL.
    Unsigned,
    /// Boolean carried as INTEGER 0/1.
    Bool,
    /// Human-readable valid UTF-8 text.
    Text,
    /// Floating point. Never used for IDs, hashes, counters, timestamps.
    Real,
    /// Epoch-microsecond timestamp carried as INTEGER.
    TimestampMicros,
}

impl ColumnType {
    /// Contract type name, for error messages.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Blob => "blob",
            Self::Integer => "integer",
            Self::Unsigned => "unsigned",
            Self::Bool => "bool",
            Self::Text => "text",
            Self::Real => "real",
            Self::TimestampMicros => "timestamp_micros",
        }
    }
}

/// A typed ID-side value. Encoding and decoding go through the mapping in
/// the record, item 5, in both directions — there is no other path across
/// the boundary.
#[derive(Debug, Clone, PartialEq)]
pub enum IdValue {
    /// Absence. Distinct from empty TEXT and empty BLOB in both directions.
    Null,
    /// Signed integer.
    Integer(i64),
    /// Unsigned 64-bit integer (see [`ColumnType::Unsigned`]).
    Unsigned(u64),
    /// Floating point.
    Real(f64),
    /// Valid UTF-8 text.
    Text(String),
    /// Raw bytes.
    Blob(Vec<u8>),
    /// Boolean.
    Bool(bool),
    /// Epoch-microsecond timestamp.
    TimestampMicros(i64),
}

impl IdValue {
    /// Build a TEXT value from raw bytes, rejecting non-UTF-8 input
    /// instead of lossy-replacing it (record #5806 item 5).
    pub fn text_from_bytes(bytes: Vec<u8>) -> Result<Self, AdapterError> {
        String::from_utf8(bytes)
            .map(Self::Text)
            .map_err(|_| AdapterError::TypeMismatch {
                column: String::new(),
                expected: "text",
                found: "blob (non-utf8)",
            })
    }

    /// Encode to the wire surface (record #5806 item 5).
    pub fn encode(&self, max_blob_bytes: usize) -> Result<WireValue, AdapterError> {
        match self {
            Self::Null => Ok(WireValue::Null),
            Self::Integer(v) | Self::TimestampMicros(v) => Ok(WireValue::Integer(*v)),
            Self::Unsigned(v) => {
                if let Ok(signed) = i64::try_from(*v) {
                    Ok(WireValue::Integer(signed))
                } else {
                    // Above i64::MAX: 8-byte big-endian BLOB. Never REAL.
                    Ok(WireValue::Blob(v.to_be_bytes().to_vec()))
                }
            }
            Self::Real(v) => {
                if v.is_finite() {
                    Ok(WireValue::Real(*v))
                } else {
                    Err(AdapterError::NonFiniteReal)
                }
            }
            Self::Text(v) => Ok(WireValue::Text(v.clone())),
            Self::Blob(v) => {
                if v.len() > max_blob_bytes {
                    Err(AdapterError::SizeRefused {
                        size: v.len(),
                        max: max_blob_bytes,
                    })
                } else {
                    Ok(WireValue::Blob(v.clone()))
                }
            }
            Self::Bool(v) => Ok(WireValue::Integer(i64::from(*v))),
        }
    }

    /// Decode a wire value under a column contract. NULL decodes to
    /// [`IdValue::Null`] for every column type (absence is distinct from
    /// empty values). Any other type mismatch is an error.
    pub fn decode(
        wire: WireValue,
        column: &str,
        contract: ColumnType,
    ) -> Result<Self, AdapterError> {
        let mismatch = |found: &'static str| AdapterError::TypeMismatch {
            column: column.to_owned(),
            expected: contract.name(),
            found,
        };
        match (wire, contract) {
            (WireValue::Null, _) => Ok(Self::Null),
            (WireValue::Blob(v), ColumnType::Blob) => Ok(Self::Blob(v)),
            (WireValue::Integer(v), ColumnType::Integer) => Ok(Self::Integer(v)),
            (WireValue::Integer(v), ColumnType::TimestampMicros) => Ok(Self::TimestampMicros(v)),
            (WireValue::Integer(v), ColumnType::Unsigned) => u64::try_from(v)
                .map(Self::Unsigned)
                .map_err(|_| mismatch("integer (negative)")),
            (WireValue::Blob(v), ColumnType::Unsigned) => {
                let bytes: [u8; 8] = v
                    .as_slice()
                    .try_into()
                    .map_err(|_| mismatch("blob (not 8 bytes)"))?;
                Ok(Self::Unsigned(u64::from_be_bytes(bytes)))
            }
            (WireValue::Integer(0), ColumnType::Bool) => Ok(Self::Bool(false)),
            (WireValue::Integer(1), ColumnType::Bool) => Ok(Self::Bool(true)),
            (WireValue::Integer(_), ColumnType::Bool) => Err(mismatch("integer (not 0/1)")),
            (WireValue::Text(v), ColumnType::Text) => Ok(Self::Text(v)),
            (WireValue::Real(v), ColumnType::Real) => Ok(Self::Real(v)),
            (wire, _) => Err(mismatch(wire.type_name())),
        }
    }
}

// ---------------------------------------------------------------------------
// Queries: structured, grant-pushed, read-only by construction
// ---------------------------------------------------------------------------

/// A read-only query against the granted view of one table. The grant
/// predicate is mandatory and is ANDed into the generated SQL, so grant
/// filtering happens in the remote query itself; the adapter never sees
/// ungranted rows. A caller asking for a row it is not granted gets the
/// same empty result as a caller asking for a row that does not exist.
#[derive(Debug, Clone, PartialEq)]
pub struct GrantedQuery {
    /// Table (or view) name; validated as a bare SQL identifier.
    pub table: String,
    /// The granted view schema: (column name, contract type) pairs. This
    /// is also all the schema the adapter ever exposes for the store.
    pub columns: Vec<(String, ColumnType)>,
    /// Grant predicate SQL fragment with `?` placeholders, e.g.
    /// `grantee = ?`. Required, never empty.
    pub grant_sql: String,
    /// Arguments for `grant_sql`, in order.
    pub grant_args: Vec<IdValue>,
    /// Additional caller filter fragment with `?` placeholders.
    pub filter_sql: Option<String>,
    /// Arguments for `filter_sql`, in order.
    pub filter_args: Vec<IdValue>,
    /// Optional row cap, applied as a literal LIMIT.
    pub limit: Option<u32>,
}

/// One decoded result row, aligned with [`GrantedQuery::columns`].
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Column names, in contract order.
    pub columns: Vec<String>,
    /// Decoded values, in contract order.
    pub values: Vec<IdValue>,
}

impl Row {
    /// Look up a decoded value by column name.
    #[must_use]
    pub fn get(&self, column: &str) -> Option<&IdValue> {
        self.columns
            .iter()
            .position(|name| name == column)
            .and_then(|idx| self.values.get(idx))
    }
}

const WRITE_KEYWORDS: [&str; 12] = [
    "INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "CREATE", "REPLACE", "ATTACH", "DETACH",
    "PRAGMA", "VACUUM", "TRUNCATE",
];

/// Write keywords that also name SQLite scalar functions. A token match on
/// one of these is a function call — not a write statement — only when the
/// token is immediately followed (modulo whitespace) by an argument list
/// opener. Every other shape, e.g. `REPLACE INTO ...`, stays refused
/// (#13896).
const FUNCTION_KEYWORDS: [&str; 1] = ["REPLACE"];

fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A predicate fragment must be a single boolean expression: no statement
/// separators, no comment openers, no write keywords, and exactly as many
/// `?` placeholders as supplied arguments.
///
/// The keyword scan is position-aware: a write keyword that also names a
/// SQLite scalar function (see [`FUNCTION_KEYWORDS`]) is accepted when it
/// opens a call — `replace(label, '-', '')` — and refused in every other
/// shape, so `REPLACE INTO ...` stays refused (#13896).
fn validate_fragment(fragment: &str, arg_count: usize) -> Result<(), AdapterError> {
    if fragment.contains(';') || fragment.contains("--") || fragment.contains("/*") {
        return Err(AdapterError::QueryRefused(
            "predicate fragment is not a single expression".to_owned(),
        ));
    }
    let bytes = fragment.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_' {
            let start = index;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            let word = &fragment[start..index];
            if WRITE_KEYWORDS.contains(&word.to_ascii_uppercase().as_str())
                && !is_function_call(fragment, word, index)
            {
                return Err(AdapterError::WriteRefused);
            }
        } else {
            index += 1;
        }
    }
    if fragment.matches('?').count() != arg_count {
        return Err(AdapterError::QueryRefused(
            "predicate placeholder count does not match argument count".to_owned(),
        ));
    }
    Ok(())
}

/// True when the keyword token `word`, ending at byte `end`, opens a scalar
/// function call: the token names a real SQLite function and the next
/// non-whitespace character is `(`.
fn is_function_call(fragment: &str, word: &str, end: usize) -> bool {
    if !FUNCTION_KEYWORDS.contains(&word.to_ascii_uppercase().as_str()) {
        return false;
    }
    fragment[end..]
        .trim_start_matches(|c: char| c.is_whitespace())
        .starts_with('(')
}

impl GrantedQuery {
    /// The granted view schema this query exposes — and the only schema
    /// the adapter exposes for the store (record #5806 item 8).
    #[must_use]
    pub fn granted_schema(&self) -> &[(String, ColumnType)] {
        &self.columns
    }

    /// Build the single SELECT statement and its wire arguments. The
    /// grant predicate is always present and always first in the WHERE.
    pub fn build_statement(
        &self,
        max_blob_bytes: usize,
    ) -> Result<(String, Vec<WireValue>), AdapterError> {
        if !is_identifier(&self.table) {
            return Err(AdapterError::QueryRefused(
                "table is not a bare identifier".to_owned(),
            ));
        }
        if self.columns.is_empty() {
            return Err(AdapterError::QueryRefused(
                "granted view has no columns".to_owned(),
            ));
        }
        for (name, _) in &self.columns {
            if !is_identifier(name) {
                return Err(AdapterError::QueryRefused(
                    "column is not a bare identifier".to_owned(),
                ));
            }
        }
        if self.grant_sql.trim().is_empty() {
            return Err(AdapterError::QueryRefused(
                "a grant predicate is required; ungranted reads are refused".to_owned(),
            ));
        }
        validate_fragment(&self.grant_sql, self.grant_args.len())?;
        if let Some(filter) = &self.filter_sql {
            validate_fragment(filter, self.filter_args.len())?;
        } else if !self.filter_args.is_empty() {
            return Err(AdapterError::QueryRefused(
                "filter arguments without a filter fragment".to_owned(),
            ));
        }

        let column_list = self
            .columns
            .iter()
            .map(|(name, _)| format!("\"{name}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let mut sql = format!(
            "SELECT {column_list} FROM \"{}\" WHERE ({})",
            self.table, self.grant_sql
        );
        if let Some(filter) = &self.filter_sql {
            sql.push_str(&format!(" AND ({filter})"));
        }
        if let Some(limit) = self.limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }

        let mut args = Vec::with_capacity(self.grant_args.len() + self.filter_args.len());
        for value in self.grant_args.iter().chain(self.filter_args.iter()) {
            args.push(value.encode(max_blob_bytes)?);
        }
        Ok((sql, args))
    }
}

// ---------------------------------------------------------------------------
// Public reads: published Turso collections (decision record #5802 D4)
// ---------------------------------------------------------------------------

/// How one published collection is served (decision record #5802, D4):
/// live through the adapter's read path, or from a materialized public
/// replica the owner's side maintains. The mode is the owner's declared
/// choice per collection; it is never inferred, and a live collection
/// never falls back to a replica (or to any stale snapshot) when its
/// live read fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicServingMode {
    /// Live reads against the source store. The credential the adapter
    /// resolves for the alias is a read-only credential held on the
    /// serving side; it never reaches the reader (invariant 5).
    Live,
    /// Reads against a materialized public replica store, named by its
    /// own alias in [`ForeignConfig`]. The replica holds only published
    /// collections; its maintenance, and its deletion on unpublish, are
    /// the owner's side's work, driven by the publish/unpublish actions
    /// (the record's slice, #14781) — this adapter only reads it.
    MaterializedReplica,
}

/// One collection named by a publication record, with its declared
/// serving mode (decision record #5802, D4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedCollection {
    /// Collection (table) name, exactly as the publication record
    /// names it.
    pub name: String,
    /// The owner's declared serving mode for this collection.
    pub mode: PublicServingMode,
    /// Alias of the materialized replica store. Required when `mode`
    /// is [`PublicServingMode::MaterializedReplica`]; must be absent
    /// for [`PublicServingMode::Live`].
    pub replica_alias: Option<String>,
}

/// The published set of one foreign store: exactly the collections a
/// publication record names, each with its declared serving mode
/// (decision record #5802, D4). The serving layer builds this from the
/// publication record; the adapter treats it as complete and closed —
/// a collection outside the set is not published, and a table created
/// after publication is never added implicitly. Unpublish is expressed
/// by the serving layer no longer presenting the collection in the
/// scope (the public grant revoked with it), so an unpublished
/// collection reads exactly like a never-published one here: refused.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PublicationScope {
    collections: BTreeMap<String, PublishedCollection>,
}

impl PublicationScope {
    /// Build a scope from an explicit enumeration of published
    /// collections. Names must be bare SQL identifiers (the same rule
    /// the query builder enforces), a replica-served collection must
    /// name its replica alias, a live-served collection must not, and
    /// a name may appear only once: a malformed scope is a config
    /// error, refused loudly rather than partially applied
    /// (invariant 8).
    pub fn new(
        collections: impl IntoIterator<Item = PublishedCollection>,
    ) -> Result<Self, AdapterError> {
        let mut map = BTreeMap::new();
        for collection in collections {
            if !is_identifier(&collection.name) {
                return Err(AdapterError::Config(format!(
                    "published collection name is not a bare identifier: {}",
                    collection.name
                )));
            }
            match (collection.mode, &collection.replica_alias) {
                (PublicServingMode::MaterializedReplica, None) => {
                    return Err(AdapterError::Config(format!(
                        "published collection {} is replica-served but names no replica alias",
                        collection.name
                    )));
                }
                (PublicServingMode::Live, Some(_)) => {
                    return Err(AdapterError::Config(format!(
                        "published collection {} is live-served but names a replica alias",
                        collection.name
                    )));
                }
                _ => {}
            }
            if map.contains_key(&collection.name) {
                return Err(AdapterError::Config(format!(
                    "published collection named twice: {}",
                    collection.name
                )));
            }
            map.insert(collection.name.clone(), collection);
        }
        Ok(Self { collections: map })
    }

    /// Build the scope from the real publication record (#14781,
    /// [`PublicationRecord`]) plus the serving side's per-collection
    /// serving declarations.
    ///
    /// The record is the authority for WHAT is published: its scope's
    /// collection units are exactly the published set (D4). The
    /// declarations are the authority for HOW each collection is
    /// served — the owner's per-collection live/materialized choice
    /// (#5803), which the record does not carry — so the serving side
    /// supplies them here, as [`PublishedCollection`] entries.
    ///
    /// The two must agree exactly, and every disagreement is a config
    /// error, refused loudly rather than partially applied
    /// (invariant 8):
    ///
    /// - a retired record (`unpublished_at` set) builds no scope:
    ///   unpublish retires the whole share at once (D5);
    /// - a record unit that is an object, not a collection, is
    ///   refused: this read path serves collections, and silently
    ///   dropping part of the published set would serve less than
    ///   the record promises with nobody told;
    /// - a collection the record names with no serving declaration,
    ///   and a declaration naming a collection the record does not
    ///   name, are both refused: the scope is never wider or narrower
    ///   than the record, and a table the record does not name (one
    ///   created later, say) is not published.
    ///
    /// The caller passes a record that is live at build time — the
    /// grants layer's liveness check (grant lineage, via
    /// `Publications::resolve`) is what vouches for that. The scope
    /// is then a snapshot of the published set; unpublish is
    /// expressed by the serving layer no longer presenting it, as
    /// [`PublicationScope`] documents.
    pub fn from_record(
        record: &PublicationRecord,
        declared: impl IntoIterator<Item = PublishedCollection>,
    ) -> Result<Self, AdapterError> {
        if record.unpublished_at.is_some() {
            return Err(AdapterError::Config(format!(
                "publication record for share {} is retired; a retired share has no public scope",
                record.share
            )));
        }
        let mut named = BTreeSet::new();
        for unit in &record.scope {
            match unit {
                Scope::Collection(name) => {
                    named.insert(name.as_str());
                }
                Scope::Object(id) => {
                    return Err(AdapterError::Config(format!(
                        "publication record for share {} names object {id}, which the \
                         adapter's collection read path cannot serve",
                        record.share
                    )));
                }
            }
        }
        let declared: Vec<PublishedCollection> = declared.into_iter().collect();
        let declared_names: BTreeSet<&str> = declared
            .iter()
            .map(|collection| collection.name.as_str())
            .collect();
        for name in &named {
            if !declared_names.contains(name) {
                return Err(AdapterError::Config(format!(
                    "publication record for share {} publishes collection {name}, \
                     which has no serving declaration",
                    record.share
                )));
            }
        }
        for collection in &declared {
            if !named.contains(collection.name.as_str()) {
                return Err(AdapterError::Config(format!(
                    "serving declaration names collection {}, which the publication \
                     record for share {} does not publish",
                    collection.name, record.share
                )));
            }
        }
        Self::new(declared)
    }

    /// The published collection named `name`, if the record names it.
    #[must_use]
    pub fn collection(&self, name: &str) -> Option<&PublishedCollection> {
        self.collections.get(name)
    }

    /// Whether the record names `name` as published.
    #[must_use]
    pub fn is_published(&self, name: &str) -> bool {
        self.collections.contains_key(name)
    }

    /// Every published collection in the scope, in name order. The
    /// replica maintainer's drivers walk this set; the read path
    /// looks collections up by name instead.
    pub fn collections(&self) -> impl Iterator<Item = &PublishedCollection> {
        self.collections.values()
    }
}

/// Why a public read did not serve rows. The public outcome vocabulary
/// (decision record #5802, D4 and invariant 8): two stable, named
/// outcomes and nothing else. Neither variant carries a credential, a
/// credential reference, a resolved URL, a server response body, or
/// any adapter-internal detail, so the value is safe to surface in any
/// public artifact, error, or log (invariant 5). The serving layer
/// keeps the adapter's own classified error for its operator side and
/// exposes only this outcome to readers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PublicReadError {
    /// The query names a collection the publication record does not
    /// name, or its predicate fragments reach beyond the one named
    /// collection (a subquery or join shape), or it attempts a write:
    /// refused at the source, before any fetch. "Not published" and
    /// "no such collection" are this same outcome (invariant 3).
    #[error("public read refused")]
    Refused,
    /// The declared source could not be read. A failed live read is
    /// unavailable, never a stale snapshot served as current; a
    /// replica-served collection whose replica cannot be read is
    /// unavailable rather than silently served from the live source.
    #[error("public read unavailable")]
    Unavailable,
}

/// Fragment keywords that would let a predicate reach beyond the
/// single collection a public query names: a subquery (`SELECT` /
/// `FROM` / `WITH`), a join, or a set operation. The public read
/// surface is the adapter's structured single-collection read; the
/// adapter cannot enumerate the tables an arbitrary subquery touches,
/// so the shape is refused rather than fetched wide and filtered
/// afterwards (decision record #5802, D4).
const PUBLIC_FRAGMENT_FORBIDDEN: [&str; 7] = [
    "SELECT",
    "FROM",
    "JOIN",
    "UNION",
    "EXCEPT",
    "INTERSECT",
    "WITH",
];

/// A predicate fragment on the public path must stay inside the one
/// named collection: no subquery, join, or set-operation keyword as a
/// whole word. The scan is lexical, matching the base fragment rules —
/// a forbidden word inside a string literal is refused too, which is
/// conservative and the safe direction. (The base rules — no statement
/// separators, no write keywords — are enforced again by statement
/// construction.)
fn validate_public_fragment(fragment: &str) -> Result<(), PublicReadError> {
    let bytes = fragment.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_' {
            let start = index;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            let word = &fragment[start..index];
            if PUBLIC_FRAGMENT_FORBIDDEN.contains(&word.to_ascii_uppercase().as_str()) {
                return Err(PublicReadError::Refused);
            }
        } else {
            index += 1;
        }
    }
    Ok(())
}

impl<T: Transport, C: CredentialResolver> LibsqlAdapter<T, C> {
    /// Serve one public read of a published Turso collection
    /// (decision record #5802, D4; issue #14784).
    ///
    /// The read is scoped twice at the source: `scope` — the published
    /// set from the publication record — decides whether the named
    /// collection may be served at all and from where, and the
    /// [`GrantedQuery`]'s grant predicate is pushed into the single
    /// SELECT exactly as on the private path (invariant 1). A query
    /// naming an unpublished collection, or reaching beyond it through
    /// a fragment subquery or join, is refused before any fetch. The
    /// outcome vocabulary is [`PublicReadError`]: a failed read is
    /// `Unavailable`, never stale data served as current.
    pub async fn query_public(
        &self,
        alias: &str,
        scope: &PublicationScope,
        query: &GrantedQuery,
    ) -> Result<Vec<Row>, PublicReadError> {
        let published = scope
            .collection(&query.table)
            .ok_or(PublicReadError::Refused)?;
        validate_public_fragment(&query.grant_sql)?;
        if let Some(filter) = &query.filter_sql {
            validate_public_fragment(filter)?;
        }
        let target_alias = match published.mode {
            PublicServingMode::Live => alias,
            PublicServingMode::MaterializedReplica => published
                .replica_alias
                .as_deref()
                .ok_or(PublicReadError::Unavailable)?,
        };
        match self.query(target_alias, query).await {
            Ok(rows) => Ok(rows),
            Err(AdapterError::QueryRefused(_)) | Err(AdapterError::WriteRefused) => {
                Err(PublicReadError::Refused)
            }
            Err(_) => Err(PublicReadError::Unavailable),
        }
    }
}

// ---------------------------------------------------------------------------
// Serving limits at the public-read boundary (#5802 D3 item 4, #14783)
// ---------------------------------------------------------------------------

/// Why a served public read did not serve rows: the serving layer's
/// outcome vocabulary. The adapter's two outcomes
/// ([`PublicReadError`]) are preserved as the first two variants; a
/// fetch stopped by the share's serving limits names the limit
/// instead — the stable serving-limit outcome the #5802 record
/// requires (D3 item 4, invariant 8: caps fail loudly). Like
/// [`PublicReadError`], no variant carries a credential, a credential
/// reference, a resolved URL, a server response body, or any
/// adapter-internal detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PublicServingError {
    /// The read was refused at the source — unpublished collection,
    /// reach-through fragment, or write shape — before any fetch.
    #[error("public read refused")]
    Refused,
    /// The declared source could not be read. Never stale data
    /// served as current.
    #[error("public read unavailable")]
    Unavailable,
    /// A serving limit refused the fetch: the reader is over the
    /// share's request-rate cap, or the owner's aggregate budget or
    /// concurrent-fetch ceiling is spent. The refusal's stable wire
    /// name is [`ServingRefusal::name`].
    #[error("public read refused by a serving limit")]
    Limit(ServingRefusal),
    /// The fetched rows crossed a per-fetch serving cap (objects or
    /// bytes) and were not served. This boundary serves a fetch
    /// whole or not at all: the [`GrantedQuery`] surface carries a
    /// literal `LIMIT` but no offset, so there is no resume position
    /// to hand back, and serving the partial result as if complete
    /// would be the silent truncation invariant 8 forbids.
    /// Page-and-resume under #5800's chunk-resume rule binds where
    /// snapshot fetches are served. The cap's stable wire name is
    /// [`FetchCap::name`].
    #[error("public read crossed a per-fetch serving cap")]
    Capped(FetchCap),
}

impl PublicServingError {
    /// The stable name of this outcome: the refusal's or cap's own
    /// wire name for the limit outcomes.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Refused => "refused",
            Self::Unavailable => "unavailable",
            Self::Limit(refusal) => refusal.name(),
            Self::Capped(FetchCap::Objects) => "fetch_capped_objects",
            Self::Capped(FetchCap::Bytes) => "fetch_capped_bytes",
        }
    }
}

impl From<PublicReadError> for PublicServingError {
    fn from(error: PublicReadError) -> Self {
        match error {
            PublicReadError::Refused => Self::Refused,
            PublicReadError::Unavailable => Self::Unavailable,
        }
    }
}

/// The serving context of one public read: the opaque ids the gate
/// keys on, the owner's tuned limits for the share, and the
/// caller's clock.
#[derive(Debug, Clone, Copy)]
pub struct PublicReadContext<'a> {
    /// The share's owner (the aggregate budget and the concurrency
    /// ceiling are charged to the owner, across all their shares).
    pub owner: &'a str,
    /// The share being read (the rate window is per share and
    /// reader).
    pub share: &'a str,
    /// The reader, by identity-or-address.
    pub reader: &'a str,
    /// The owner's tuned limits for this share, if any; the limits
    /// in force clamp these to the platform ceilings.
    pub owner_tuned_limits: Option<&'a ShareServingLimits>,
    /// Milliseconds on the caller's monotonic clock, as in the gate.
    pub now_ms: u64,
}

/// The bytes one fetched row is charged as on the serving layers:
/// the payload bytes of its values — text and blob payloads at
/// their length, fixed-width values at their width. Column names
/// are contract, not payload, and are not charged.
fn row_serving_bytes(row: &Row) -> u64 {
    row.values
        .iter()
        .map(|value| match value {
            IdValue::Null => 0,
            IdValue::Bool(_) => 1,
            IdValue::Integer(_)
            | IdValue::Unsigned(_)
            | IdValue::Real(_)
            | IdValue::TimestampMicros(_) => 8,
            IdValue::Text(text) => u64::try_from(text.len()).unwrap_or(u64::MAX),
            IdValue::Blob(blob) => u64::try_from(blob.len()).unwrap_or(u64::MAX),
        })
        .fold(0u64, u64::saturating_add)
}

/// The serving layer for public reads: one [`ServingGate`] held
/// across calls, wiring the mandatory serving limits of the #5802
/// record (D3 item 4) into the adapter's public-read path
/// ([`LibsqlAdapter::query_public`], #14784). The serving layer
/// that presents public shares owns one `PublicServing`: the gate's
/// rate windows, aggregate budgets, and concurrency counts are
/// meaningful only held across reads, so they live here, not in
/// the stateless adapter.
///
/// This is the one public-read boundary that exists today. The
/// #5800 snapshot fetch path — share snapshots served to readers
/// with page-and-resume — does not exist yet; when a slice builds
/// it, it calls the same gate per reader fetch, as the gate's own
/// docs prescribe.
#[derive(Debug)]
pub struct PublicServing {
    gate: Mutex<ServingGate>,
    platform_ceilings: ShareServingLimits,
}

impl PublicServing {
    /// A serving layer enforcing the deployment's per-owner
    /// aggregate limits under the platform's per-share ceilings.
    #[must_use]
    pub fn new(owner_limits: OwnerAggregateLimits, platform_ceilings: ShareServingLimits) -> Self {
        Self {
            gate: Mutex::new(ServingGate::new(owner_limits)),
            platform_ceilings,
        }
    }

    /// Serve one public read of a published collection under the
    /// share's serving limits.
    ///
    /// The layers run in the gate's own order, around the adapter
    /// read: the owner's concurrency slot is taken first and held
    /// across the read; the fetched rows then run through
    /// [`ServingGate::serve_fetch`] — rate check, per-fetch cap
    /// plan over the rows' sizes, aggregate charge. At this
    /// boundary the row sizes exist only once the read returns, so
    /// the gate runs on the fetched result: a fetch the gate
    /// refuses, and a fetch whose rows cross a per-fetch cap, serve
    /// nothing to the reader. The aggregate charge stands for a
    /// capped fetch — the source read happened and the owner's
    /// serving path bore it; what the cap withholds is delivery,
    /// not the read.
    pub async fn query_public<T: Transport, C: CredentialResolver>(
        &self,
        adapter: &LibsqlAdapter<T, C>,
        alias: &str,
        scope: &PublicationScope,
        query: &GrantedQuery,
        context: &PublicReadContext<'_>,
    ) -> Result<Vec<Row>, PublicServingError> {
        let limits =
            ShareServingLimits::effective(context.owner_tuned_limits, &self.platform_ceilings);
        {
            let mut gate = self.gate.lock().unwrap_or_else(|p| p.into_inner());
            if let Err(refusal) = gate.begin_owner_fetch(context.owner) {
                return Err(PublicServingError::Limit(refusal));
            }
        }
        let result = self
            .serve_gated(adapter, alias, scope, query, context, &limits)
            .await;
        {
            let mut gate = self.gate.lock().unwrap_or_else(|p| p.into_inner());
            gate.end_owner_fetch(context.owner);
        }
        result
    }

    /// Plan and charge one page of a public snapshot fetch under the
    /// same share and owner limits as public reads. A capped page carries
    /// its stable cap name and next object index; callers resume the same
    /// snapshot cut from `resume_from` rather than treating it as complete.
    ///
    /// The snapshot-serving transport supplies the cut's ordered object
    /// sizes and the prior page's resume position. This method is the
    /// serving-boundary seam for #5800 page-and-resume; it does not fetch
    /// or serialize the snapshot itself.
    pub fn serve_snapshot_page(
        &self,
        object_sizes: &[u64],
        from: u64,
        context: &PublicReadContext<'_>,
    ) -> Result<FetchPage, PublicServingError> {
        let limits =
            ShareServingLimits::effective(context.owner_tuned_limits, &self.platform_ceilings);
        let mut gate = self.gate.lock().unwrap_or_else(|p| p.into_inner());
        gate.serve_fetch(
            context.owner,
            context.share,
            context.reader,
            object_sizes,
            from,
            &limits,
            context.now_ms,
        )
        .map_err(PublicServingError::Limit)
    }

    async fn serve_gated<T: Transport, C: CredentialResolver>(
        &self,
        adapter: &LibsqlAdapter<T, C>,
        alias: &str,
        scope: &PublicationScope,
        query: &GrantedQuery,
        context: &PublicReadContext<'_>,
        limits: &ShareServingLimits,
    ) -> Result<Vec<Row>, PublicServingError> {
        let rows = adapter.query_public(alias, scope, query).await?;
        let sizes: Vec<u64> = rows.iter().map(row_serving_bytes).collect();
        let page = {
            let mut gate = self.gate.lock().unwrap_or_else(|p| p.into_inner());
            gate.serve_fetch(
                context.owner,
                context.share,
                context.reader,
                &sizes,
                0,
                limits,
                context.now_ms,
            )
        }
        .map_err(PublicServingError::Limit)?;
        if let Some(cap) = page.capped_by {
            return Err(PublicServingError::Capped(cap));
        }
        Ok(rows)
    }

    /// Prune expired rate and aggregate windows. This bounds
    /// retained state and changes no admission decision.
    pub fn prune(&self, now_ms: u64) {
        let mut gate = self.gate.lock().unwrap_or_else(|p| p.into_inner());
        gate.prune(now_ms);
    }
}

// ---------------------------------------------------------------------------
// Replica maintenance: the owner-side write seam (decision record #5802
// D4/D5; issue #14784)
// ---------------------------------------------------------------------------

/// Source-read page size for a materialization copy: rows are pulled
/// from the source in pages of this many rows, so a large collection
/// is never held whole on either side of the copy.
pub const REPLICA_PAGE_ROWS: u32 = 500;

/// Rows per INSERT statement in a materialization copy.
pub const REPLICA_INSERT_ROWS_PER_STATEMENT: usize = 100;

/// Statements per pipeline call in a materialization copy. A call is
/// the unit the remote executes in order; chunking bounds request
/// size without changing the copy's outcome.
pub const REPLICA_STATEMENTS_PER_CALL: usize = 25;

/// The copy/maintenance policy for materialized replicas: batching
/// only. What triggers a copy is the driver's business (see
/// [`ReplicaMaintainer::publish_effects`]); these numbers shape how a
/// copy crosses the wire, never what it contains.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplicaPolicy {
    /// Source rows fetched per read page.
    pub page_rows: u32,
    /// Rows carried by one INSERT statement.
    pub insert_rows_per_statement: usize,
    /// Statements carried by one pipeline call.
    pub statements_per_call: usize,
}

impl Default for ReplicaPolicy {
    fn default() -> Self {
        Self {
            page_rows: REPLICA_PAGE_ROWS,
            insert_rows_per_statement: REPLICA_INSERT_ROWS_PER_STATEMENT,
            statements_per_call: REPLICA_STATEMENTS_PER_CALL,
        }
    }
}

/// The record of one completed materialization: the cut a manifest
/// states for a replica-served collection (decision record #5802 D4 —
/// a materialized replica is a declared mode, and its manifest states
/// its cut/epoch). Carries names and counts only: no credential, no
/// credential reference, no resolved URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Materialization {
    /// The collection that was copied.
    pub collection: String,
    /// Alias of the replica store now holding the copy.
    pub replica_alias: String,
    /// Rows copied into the replica.
    pub rows: u64,
    /// The publication epoch this copy belongs to, as supplied by the
    /// caller (the manifest epoch the publish action advanced).
    pub epoch: u64,
    /// When the copy completed (Unix seconds, UTC), as supplied by
    /// the caller.
    pub completed_at: u64,
}

/// One owner-side statement: SQL built by this module from validated
/// identifiers, the source store's own schema text, and wire values —
/// never caller-supplied SQL text.
#[derive(Debug, Clone, PartialEq)]
struct OwnerStatement {
    sql: String,
    args: Vec<WireValue>,
}

/// One owner-side statement result, at the wire level: column names
/// and wire values, with no contract decode. The maintainer copies
/// values source-to-replica exactly as they crossed, so no column
/// contract is needed — or invented — on this path.
#[derive(Debug, Clone, PartialEq)]
struct RawResult {
    cols: Vec<String>,
    rows: Vec<Vec<WireValue>>,
}

/// The shadow table a materialization builds before the atomic swap:
/// `<collection>__id_materializing`. Readers never see this name in a
/// scope; it exists only inside one materialization.
fn shadow_name(collection: &str) -> String {
    format!("{collection}__id_materializing")
}

/// The owner-side maintainer for materialized public replicas
/// (decision record #5802 D4/D5; issue #14784).
///
/// This is the write seam the read adapter deliberately does not
/// have: [`LibsqlAdapter`]'s statement path stays read-only by
/// construction, and no method here is reachable from it. The
/// maintainer is a separate type, built from the same operator config
/// and the same transport/credential plumbing, and it is the only
/// writer in this module. Its rules:
///
/// - It never accepts caller-supplied SQL. Every statement is built
///   from a validated bare identifier, the source store's own schema
///   text (renamed token-by-token and verified, never trusted
///   blindly), and wire values copied from the source.
/// - It writes only to replica aliases. A materialization whose
///   replica alias equals its source alias is a config error, refused
///   before any I/O: the source store is never a write target.
/// - It touches only collections a [`PublicationScope`] names as
///   replica-served. Live-served collections are refused here (their
///   serving path is the read adapter), and a collection outside the
///   scope is not the maintainer's business at all.
/// - Every operation is idempotent as a whole, so a failed operation
///   is retried by calling it again, never resumed mid-way: a
///   materialization builds a shadow table and swaps it in with one
///   rename only after the copy completes, so a failed copy leaves
///   the previous replica — or its absence — exactly as it was, and
///   readers never see a partial copy. A deletion drops if present.
/// - No call is retried internally and no circuit breaker is kept:
///   maintenance is owner-driven, not reader-driven, and a failure
///   surfaces to the owner as the classified [`AdapterError`]
///   immediately. Errors name aliases and credential references only.
///
/// Maintenance policy (what triggers what):
///
/// - **Publish** (and any amendment that changes the published set):
///   [`publish_effects`](Self::publish_effects) materializes every
///   replica-served collection in the scope. The publish/unpublish
///   actions (#14781) own the record and the trigger; these methods
///   are the effect they drive.
/// - **Refresh**: re-running
///   [`materialize`](Self::materialize) for a collection is the
///   maintenance operation — a fresh atomic copy under a new cut.
///   There is no scheduler and no change tracking in this crate: the
///   owner refreshes when it wants a newer cut, and the returned
///   [`Materialization`] is the record of the cut it got. The copy is
///   a paged read pass over the source, not a cross-request snapshot
///   transaction; the cut's epoch and completion time state honestly
///   when the copy was taken.
/// - **Unpublish**: [`unpublish_effects`](Self::unpublish_effects)
///   deletes the replica of every replica-served collection in the
///   scope (D4: a materialized public replica is deleted on
///   unpublish). Call it with the scope as built while the record was
///   live — unpublish retires the record, and a retired record builds
///   no scope ([`PublicationScope::from_record`]).
pub struct ReplicaMaintainer<T: Transport, C: CredentialResolver> {
    config: ForeignConfig,
    transport: T,
    credentials: C,
}

impl ReplicaMaintainer<HttpTransport, EnvCredentialResolver> {
    /// The production maintainer: HTTP transport, environment
    /// credentials.
    #[must_use]
    pub fn new(config: ForeignConfig) -> Self {
        Self::with_parts(config, HttpTransport, EnvCredentialResolver)
    }
}

impl<T: Transport, C: CredentialResolver> ReplicaMaintainer<T, C> {
    /// Build a maintainer from explicit parts (tests, alternate
    /// resolvers).
    #[must_use]
    pub fn with_parts(config: ForeignConfig, transport: T, credentials: C) -> Self {
        Self {
            config,
            transport,
            credentials,
        }
    }

    /// Run one batch of owner statements against `alias`, in order,
    /// in a single pipeline call. Every statement must report an `ok`
    /// result; the first error result fails the whole call with
    /// [`AdapterError::RemoteStatement`] (status 0), and its message
    /// is not copied out — the same no-leak classification as the read
    /// path.
    async fn run(
        &self,
        alias: &str,
        statements: &[OwnerStatement],
    ) -> Result<Vec<RawResult>, AdapterError> {
        let alias_config = self.config.alias(alias)?.clone();
        // The token is resolved per call and never stored.
        let token = self.credentials.resolve(&alias_config.credential_ref)?;
        let body = multi_pipeline_body(statements);
        let outcome = self
            .transport
            .post_pipeline(
                &alias_config.url,
                &token,
                &body,
                CONNECT_TIMEOUT,
                alias_config.statement_timeout,
            )
            .await;
        let response = match outcome {
            Ok(response) => response,
            Err(TransportError::Timeout) => return Err(AdapterError::Timeout),
            Err(other) => return Err(AdapterError::Transient(other.to_string())),
        };
        match response.status {
            200 => parse_owner_results(&response.body, statements.len()),
            401 | 403 => Err(AdapterError::Auth {
                credential_ref: alias_config.credential_ref.clone(),
            }),
            400..=499 => Err(AdapterError::RemoteStatement {
                status: response.status,
            }),
            _ => Err(AdapterError::Transient(format!(
                "remote status {}",
                response.status
            ))),
        }
    }

    /// Run owner statements in pipeline calls of at most
    /// `statements_per_call`, discarding results.
    async fn run_chunked(
        &self,
        alias: &str,
        statements: &[OwnerStatement],
        statements_per_call: usize,
    ) -> Result<(), AdapterError> {
        for chunk in statements.chunks(statements_per_call.max(1)) {
            self.run(alias, chunk).await?;
        }
        Ok(())
    }

    /// The replica alias a published collection is maintained under,
    /// with the maintainer's guards: the collection must be
    /// replica-served and name its replica alias, and the name must be
    /// a bare identifier. Live-served collections are refused — they
    /// have no replica to maintain.
    fn replica_alias(published: &PublishedCollection) -> Result<String, AdapterError> {
        if !is_identifier(&published.name) {
            return Err(AdapterError::Config(format!(
                "published collection name is not a bare identifier: {}",
                published.name
            )));
        }
        match (published.mode, &published.replica_alias) {
            (PublicServingMode::MaterializedReplica, Some(alias)) => Ok(alias.clone()),
            (PublicServingMode::MaterializedReplica, None) => Err(AdapterError::Config(format!(
                "published collection {} is replica-served but names no replica alias",
                published.name
            ))),
            (PublicServingMode::Live, _) => Err(AdapterError::Config(format!(
                "published collection {} is live-served; only replica-served \
                 collections are maintained",
                published.name
            ))),
        }
    }

    /// Materialize one replica-served collection: copy its schema and
    /// rows from the source store into its replica store, as one
    /// atomic swap (see the type's policy). Re-running this method is
    /// the refresh operation. Returns the cut record.
    pub async fn materialize(
        &self,
        source_alias: &str,
        published: &PublishedCollection,
        policy: &ReplicaPolicy,
        epoch: u64,
        now: u64,
    ) -> Result<Materialization, AdapterError> {
        let replica_alias = Self::replica_alias(published)?;
        if replica_alias == source_alias {
            return Err(AdapterError::Config(format!(
                "published collection {} names its source alias {source_alias} as its \
                 replica alias; the source store is never a write target",
                published.name
            )));
        }
        if policy.page_rows == 0 || policy.insert_rows_per_statement == 0 {
            return Err(AdapterError::Config(
                "replica policy batch sizes must be non-zero".to_owned(),
            ));
        }
        let name = published.name.as_str();
        let shadow = shadow_name(name);

        // Plan first: read the source's own schema for this collection
        // and build every DDL statement before any write happens, so
        // a schema this module cannot rename faithfully refuses the
        // whole materialization with the replica untouched.
        let schema = self
            .run(
                source_alias,
                &[OwnerStatement {
                    sql: "SELECT \"type\", \"name\", \"sql\" FROM \"sqlite_master\" \
                          WHERE \"tbl_name\" = ? AND \"sql\" IS NOT NULL ORDER BY \"rowid\""
                        .to_owned(),
                    args: vec![WireValue::Text(name.to_owned())],
                }],
            )
            .await?;
        let (table_ddl, index_ddls) = plan_replica_schema(&schema[0], name, &shadow)?;

        // Phase 1: (re)build the shadow table under the source schema.
        let mut build = vec![OwnerStatement {
            sql: format!("DROP TABLE IF EXISTS \"{shadow}\""),
            args: Vec::new(),
        }];
        build.push(OwnerStatement {
            sql: table_ddl,
            args: Vec::new(),
        });
        build.extend(index_ddls.into_iter().map(|sql| OwnerStatement {
            sql,
            args: Vec::new(),
        }));
        self.run_chunked(&replica_alias, &build, policy.statements_per_call)
            .await?;

        // Phase 2: copy the rows, page by page, values exactly as
        // they crossed the wire.
        let replica_max_blob = self.config.alias(&replica_alias)?.max_blob_bytes;
        let mut total: u64 = 0;
        let mut offset: u64 = 0;
        loop {
            let page = self
                .run(
                    source_alias,
                    &[OwnerStatement {
                        sql: format!("SELECT * FROM \"{name}\" LIMIT ? OFFSET ?"),
                        args: vec![
                            WireValue::Integer(i64::from(policy.page_rows)),
                            WireValue::Integer(i64::try_from(offset).map_err(|_| {
                                AdapterError::Config(
                                    "replica copy offset exceeds the wire integer range".to_owned(),
                                )
                            })?),
                        ],
                    }],
                )
                .await?;
            let result = &page[0];
            if result.rows.is_empty() {
                break;
            }
            let mut inserts = Vec::new();
            for chunk in result.rows.chunks(policy.insert_rows_per_statement) {
                let mut sql = format!("INSERT INTO \"{shadow}\" VALUES ");
                let mut args = Vec::with_capacity(chunk.len() * result.cols.len());
                for (row_index, row) in chunk.iter().enumerate() {
                    if row.len() != result.cols.len() {
                        return Err(AdapterError::MalformedResponse);
                    }
                    if row_index > 0 {
                        sql.push(',');
                    }
                    sql.push('(');
                    for (cell_index, cell) in row.iter().enumerate() {
                        if let WireValue::Blob(bytes) = cell {
                            if bytes.len() > replica_max_blob {
                                return Err(AdapterError::SizeRefused {
                                    size: bytes.len(),
                                    max: replica_max_blob,
                                });
                            }
                        }
                        if cell_index > 0 {
                            sql.push(',');
                        }
                        sql.push('?');
                        args.push(cell.clone());
                    }
                    sql.push(')');
                }
                inserts.push(OwnerStatement { sql, args });
            }
            self.run_chunked(&replica_alias, &inserts, policy.statements_per_call)
                .await?;
            total += result.rows.len() as u64;
            if (result.rows.len() as u64) < u64::from(policy.page_rows) {
                break;
            }
            offset += u64::from(policy.page_rows);
        }

        // Phase 3: the swap. One call, in order: drop the previous
        // replica, rename the shadow in. Until this call the previous
        // replica (or its absence) is exactly what readers see.
        self.run(
            &replica_alias,
            &[
                OwnerStatement {
                    sql: format!("DROP TABLE IF EXISTS \"{name}\""),
                    args: Vec::new(),
                },
                OwnerStatement {
                    sql: format!("ALTER TABLE \"{shadow}\" RENAME TO \"{name}\""),
                    args: Vec::new(),
                },
            ],
        )
        .await?;

        Ok(Materialization {
            collection: name.to_owned(),
            replica_alias,
            rows: total,
            epoch,
            completed_at: now,
        })
    }

    /// Delete one collection's materialized replica (decision record
    /// #5802 D4: a materialized public replica is deleted on
    /// unpublish). Idempotent: returns whether a replica table was
    /// present. A leftover shadow from an interrupted materialization
    /// is dropped with it.
    pub async fn delete_replica(
        &self,
        published: &PublishedCollection,
    ) -> Result<bool, AdapterError> {
        let replica_alias = Self::replica_alias(published)?;
        let name = published.name.as_str();
        let present = self
            .run(
                &replica_alias,
                &[OwnerStatement {
                    sql: "SELECT \"name\" FROM \"sqlite_master\" \
                          WHERE \"type\" = 'table' AND \"name\" = ?"
                        .to_owned(),
                    args: vec![WireValue::Text(name.to_owned())],
                }],
            )
            .await?;
        let existed = !present[0].rows.is_empty();
        self.run(
            &replica_alias,
            &[
                OwnerStatement {
                    sql: format!("DROP TABLE IF EXISTS \"{}\"", shadow_name(name)),
                    args: Vec::new(),
                },
                OwnerStatement {
                    sql: format!("DROP TABLE IF EXISTS \"{name}\""),
                    args: Vec::new(),
                },
            ],
        )
        .await?;
        Ok(existed)
    }

    /// The publish effect: materialize every replica-served
    /// collection in `scope` from `source_alias`. Live-served
    /// collections are skipped — they have no replica. Driven by the
    /// publish action (#14781), which owns the record and the
    /// trigger; call it after the record is live.
    pub async fn publish_effects(
        &self,
        source_alias: &str,
        scope: &PublicationScope,
        policy: &ReplicaPolicy,
        epoch: u64,
        now: u64,
    ) -> Result<Vec<Materialization>, AdapterError> {
        let mut done = Vec::new();
        for published in scope.collections() {
            if published.mode == PublicServingMode::MaterializedReplica {
                done.push(
                    self.materialize(source_alias, published, policy, epoch, now)
                        .await?,
                );
            }
        }
        Ok(done)
    }

    /// The unpublish effect: delete the replica of every
    /// replica-served collection in `scope` (decision record #5802
    /// D4/D5). Returns the names of the collections whose replicas
    /// were deleted. Call it with the scope as built while the record
    /// was live — unpublish retires the record, and a retired record
    /// builds no scope.
    pub async fn unpublish_effects(
        &self,
        scope: &PublicationScope,
    ) -> Result<Vec<String>, AdapterError> {
        let mut deleted = Vec::new();
        for published in scope.collections() {
            if published.mode == PublicServingMode::MaterializedReplica
                && self.delete_replica(published).await?
            {
                deleted.push(published.name.clone());
            }
        }
        Ok(deleted)
    }
}

/// The pipeline body for a batch of owner statements: one `execute`
/// request per statement, then `close` — the same v2 pipeline shape
/// as [`pipeline_body`], with more than one statement in it.
fn multi_pipeline_body(statements: &[OwnerStatement]) -> String {
    let requests: Vec<Json> = statements
        .iter()
        .map(|statement| {
            let args_json: Vec<Json> = statement.args.iter().map(WireValue::to_json).collect();
            json!({"type": "execute", "stmt": {"sql": statement.sql, "args": args_json}})
        })
        .chain(std::iter::once(json!({"type": "close"})))
        .collect();
    json!({"baton": null, "requests": requests}).to_string()
}

/// Parse a pipeline response for [`ReplicaMaintainer::run`]: the
/// first `expected` results must all be `ok` execute results. Column
/// names and wire values are returned uninterpreted; a row whose
/// width disagrees with its columns is malformed.
fn parse_owner_results(body: &str, expected: usize) -> Result<Vec<RawResult>, AdapterError> {
    let parsed: Json = id_core::json_unique::parse_json_without_duplicate_keys(body)
        .map_err(|_| AdapterError::MalformedResponse)?;
    let results = parsed
        .get("results")
        .and_then(Json::as_array)
        .ok_or(AdapterError::MalformedResponse)?;
    if results.len() < expected {
        return Err(AdapterError::MalformedResponse);
    }
    let mut out = Vec::with_capacity(expected);
    for entry in &results[..expected] {
        if entry.get("type").and_then(Json::as_str) != Some("ok") {
            // A pipeline-level error result is a deterministic
            // statement failure; its message is deliberately not
            // copied out.
            return Err(AdapterError::RemoteStatement { status: 0 });
        }
        let result = entry
            .get("response")
            .and_then(|r| r.get("result"))
            .ok_or(AdapterError::MalformedResponse)?;
        let cols_json = result
            .get("cols")
            .and_then(Json::as_array)
            .ok_or(AdapterError::MalformedResponse)?;
        let mut cols = Vec::with_capacity(cols_json.len());
        for col in cols_json {
            cols.push(
                col.get("name")
                    .and_then(Json::as_str)
                    .ok_or(AdapterError::MalformedResponse)?
                    .to_owned(),
            );
        }
        let raw_rows = result
            .get("rows")
            .and_then(Json::as_array)
            .ok_or(AdapterError::MalformedResponse)?;
        let mut rows = Vec::with_capacity(raw_rows.len());
        for raw_row in raw_rows {
            let cells = raw_row.as_array().ok_or(AdapterError::MalformedResponse)?;
            if cells.len() != cols.len() {
                return Err(AdapterError::MalformedResponse);
            }
            let mut values = Vec::with_capacity(cells.len());
            for cell in cells {
                values.push(WireValue::from_json(cell)?);
            }
            rows.push(values);
        }
        out.push(RawResult { cols, rows });
    }
    Ok(out)
}

/// Build the replica DDL from the source's schema rows for one
/// collection: the table's own CREATE statement renamed onto the
/// shadow table, plus each index's CREATE statement renamed onto the
/// shadow (index names are prefixed with the shadow name, so a
/// refresh never collides with the live replica's indexes). Every
/// rename is verified against the name the schema row claims; a
/// statement this module cannot rename faithfully is a config error,
/// refused before any write (invariant 8).
fn plan_replica_schema(
    schema: &RawResult,
    collection: &str,
    shadow: &str,
) -> Result<(String, Vec<String>), AdapterError> {
    let mut table_ddl = None;
    let mut index_ddls = Vec::new();
    for row in &schema.rows {
        let (kind, obj_name, sql) = match row.as_slice() {
            [
                WireValue::Text(kind),
                WireValue::Text(obj_name),
                WireValue::Text(sql),
            ] => (kind.as_str(), obj_name.as_str(), sql.as_str()),
            _ => return Err(AdapterError::MalformedResponse),
        };
        match kind {
            "table" if obj_name == collection => {
                table_ddl = Some(rename_table_ddl(sql, collection, shadow)?);
            }
            "index" => {
                let renamed_index = format!("{shadow}__{obj_name}");
                index_ddls.push(rename_index_ddl(
                    sql,
                    obj_name,
                    &renamed_index,
                    collection,
                    shadow,
                )?);
            }
            _ => {}
        }
    }
    let table_ddl = table_ddl.ok_or_else(|| {
        AdapterError::Config(format!(
            "source store has no table named {collection} to materialize"
        ))
    })?;
    Ok((table_ddl, index_ddls))
}

/// One name token in DDL text: byte range and its unquoted value. The
/// token may be bare or quoted with `"`, `` ` ``, or `[`.
fn ddl_name_token(sql: &str, start: usize) -> Option<(usize, usize, String)> {
    let bytes = sql.as_bytes();
    let mut i = start;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let token_start = i;
    match bytes[i] {
        b'"' | b'`' => {
            let quote = bytes[i];
            i += 1;
            let mut raw = Vec::new();
            while i < bytes.len() {
                if bytes[i] == quote {
                    // A doubled quote inside a quoted name is an
                    // escaped quote, not the end of the token.
                    if quote == b'"' && bytes.get(i + 1) == Some(&b'"') {
                        raw.push(b'"');
                        i += 2;
                        continue;
                    }
                    return String::from_utf8(raw)
                        .ok()
                        .map(|value| (token_start, i + 1, value));
                }
                raw.push(bytes[i]);
                i += 1;
            }
            None
        }
        b'[' => {
            i += 1;
            let value_start = i;
            while i < bytes.len() && bytes[i] != b']' {
                i += 1;
            }
            if i < bytes.len() {
                Some((token_start, i + 1, sql[value_start..i].to_owned()))
            } else {
                None
            }
        }
        _ => {
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'$')
            {
                i += 1;
            }
            if i == token_start {
                None
            } else {
                Some((token_start, i, sql[token_start..i].to_owned()))
            }
        }
    }
}

/// Skip whitespace, then consume the bare word at the cursor if it is
/// `word` (case-insensitive). Returns the cursor past the word.
fn ddl_expect_word(sql: &str, start: usize, word: &str) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut i = start;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let word_start = i;
    while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
        i += 1;
    }
    if i > word_start && sql[word_start..i].eq_ignore_ascii_case(word) {
        Some(i)
    } else {
        None
    }
}

/// Consume an optional `IF NOT EXISTS` clause at the cursor.
fn ddl_skip_if_not_exists(sql: &str, start: usize) -> usize {
    if let Some(after_if) = ddl_expect_word(sql, start, "IF") {
        if let Some(after_not) = ddl_expect_word(sql, after_if, "NOT") {
            if let Some(after_exists) = ddl_expect_word(sql, after_not, "EXISTS") {
                return after_exists;
            }
        }
    }
    start
}

/// Replace the name token at `token` with `replacement`, quoted.
fn ddl_splice_name(sql: &str, token: (usize, usize, String), replacement: &str) -> String {
    let mut out = String::with_capacity(sql.len() + replacement.len());
    out.push_str(&sql[..token.0]);
    out.push('"');
    out.push_str(replacement);
    out.push('"');
    out.push_str(&sql[token.1..]);
    out
}

/// Rename a `CREATE TABLE` statement onto the shadow table: the name
/// token after `CREATE TABLE [IF NOT EXISTS]` must be the collection
/// name; anything else is refused rather than guessed at.
fn rename_table_ddl(sql: &str, collection: &str, shadow: &str) -> Result<String, AdapterError> {
    let refused = || {
        AdapterError::Config(format!(
            "source schema for collection {collection} is not a renamable CREATE TABLE statement"
        ))
    };
    let after_create = ddl_expect_word(sql, 0, "CREATE").ok_or_else(refused)?;
    let after_table = ddl_expect_word(sql, after_create, "TABLE").ok_or_else(refused)?;
    let name_at = ddl_skip_if_not_exists(sql, after_table);
    let token = ddl_name_token(sql, name_at).ok_or_else(refused)?;
    if !token.2.eq_ignore_ascii_case(collection) {
        return Err(refused());
    }
    Ok(ddl_splice_name(sql, token, shadow))
}

/// Rename a `CREATE INDEX` statement onto the shadow table: the
/// index name token is replaced with `renamed_index`, and the table
/// token after `ON` — which must be the collection name — with the
/// shadow name.
fn rename_index_ddl(
    sql: &str,
    index_name: &str,
    renamed_index: &str,
    collection: &str,
    shadow: &str,
) -> Result<String, AdapterError> {
    let refused = || {
        AdapterError::Config(format!(
            "source index {index_name} on collection {collection} is not a renamable \
             CREATE INDEX statement"
        ))
    };
    let mut at = ddl_expect_word(sql, 0, "CREATE").ok_or_else(refused)?;
    if let Some(after_unique) = ddl_expect_word(sql, at, "UNIQUE") {
        at = after_unique;
    }
    let after_index = ddl_expect_word(sql, at, "INDEX").ok_or_else(refused)?;
    let name_at = ddl_skip_if_not_exists(sql, after_index);
    let index_token = ddl_name_token(sql, name_at).ok_or_else(refused)?;
    if !index_token.2.eq_ignore_ascii_case(index_name) {
        return Err(refused());
    }
    let after_on = ddl_expect_word(sql, index_token.1, "ON").ok_or_else(refused)?;
    let table_token = ddl_name_token(sql, after_on).ok_or_else(refused)?;
    if !table_token.2.eq_ignore_ascii_case(collection) {
        return Err(refused());
    }
    // Splice the later token first so the earlier offset stays valid.
    let renamed = ddl_splice_name(sql, table_token, shadow);
    // The table token came after the index token, and both splices
    // before it leave its own range untouched.
    Ok(ddl_splice_name(&renamed, index_token, renamed_index))
}

// ---------------------------------------------------------------------------
// Replica maintenance tests. Every test names its oracle: the #5802
// decision record (D4/D5) or the #14784 checkpoint that fixes the
// expected behaviour.
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod replica_tests {
    use std::collections::{BTreeMap, VecDeque};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::Duration;

    use serde_json::{Value as Json, json};

    use super::*;

    /// Transport routed by base URL: each URL has its own scripted
    /// outcome queue; every request is captured for inspection.
    #[derive(Debug, Clone, Default)]
    struct RouterTransport {
        outcomes:
            Arc<StdMutex<BTreeMap<String, VecDeque<Result<TransportResponse, TransportError>>>>>,
        captured: Arc<StdMutex<Vec<(String, String)>>>,
    }

    impl RouterTransport {
        fn push(&self, url: &str, outcome: Result<TransportResponse, TransportError>) {
            self.outcomes
                .lock()
                .unwrap()
                .entry(url.to_owned())
                .or_default()
                .push_back(outcome);
        }

        /// Every SQL string sent to `url`, in send order.
        fn sql_sent_to(&self, url: &str) -> Vec<String> {
            let mut sql = Vec::new();
            for (captured_url, body) in self.captured.lock().unwrap().iter() {
                if captured_url != url {
                    continue;
                }
                let parsed: Json = serde_json::from_str(body).unwrap();
                for request in parsed["requests"].as_array().unwrap() {
                    if let Some(stmt) = request.get("stmt") {
                        sql.push(stmt["sql"].as_str().unwrap().to_owned());
                    }
                }
            }
            sql
        }

        /// Every request body sent to `url`, in send order.
        fn bodies_sent_to(&self, url: &str) -> Vec<String> {
            self.captured
                .lock()
                .unwrap()
                .iter()
                .filter(|(captured_url, _)| captured_url == url)
                .map(|(_, body)| body.clone())
                .collect()
        }
    }

    impl Transport for RouterTransport {
        fn post_pipeline<'a>(
            &'a self,
            base_url: &'a str,
            _token: &'a str,
            body: &'a str,
            _connect_timeout: Duration,
            _deadline: Duration,
        ) -> Pin<Box<dyn Future<Output = Result<TransportResponse, TransportError>> + Send + 'a>>
        {
            self.captured
                .lock()
                .unwrap()
                .push((base_url.to_owned(), body.to_owned()));
            let outcome = self
                .outcomes
                .lock()
                .unwrap()
                .get_mut(base_url)
                .and_then(VecDeque::pop_front);
            Box::pin(async move {
                outcome.unwrap_or_else(|| {
                    Ok(TransportResponse {
                        status: 500,
                        body: String::new(),
                    })
                })
            })
        }
    }

    #[derive(Debug)]
    struct MapCredentials;

    impl CredentialResolver for MapCredentials {
        fn resolve(&self, reference: &str) -> Result<String, AdapterError> {
            match reference {
                "env:SRC_TOKEN" => Ok("source-token-value".to_owned()),
                "env:REP_TOKEN" => Ok("replica-token-value".to_owned()),
                _ => Err(AdapterError::Auth {
                    credential_ref: reference.to_owned(),
                }),
            }
        }
    }

    const SOURCE_URL: &str = "https://source.invalid";
    const REPLICA_URL: &str = "https://replica.invalid";

    fn maintainer_config() -> ForeignConfig {
        let mut aliases = BTreeMap::new();
        aliases.insert(
            "src".to_owned(),
            AliasConfig {
                url: SOURCE_URL.to_owned(),
                credential_ref: "env:SRC_TOKEN".to_owned(),
                ..AliasConfig::default()
            },
        );
        aliases.insert(
            "rep".to_owned(),
            AliasConfig {
                url: REPLICA_URL.to_owned(),
                credential_ref: "env:REP_TOKEN".to_owned(),
                ..AliasConfig::default()
            },
        );
        ForeignConfig { aliases }
    }

    fn maintainer(
        transport: RouterTransport,
    ) -> ReplicaMaintainer<RouterTransport, MapCredentials> {
        ReplicaMaintainer::with_parts(maintainer_config(), transport, MapCredentials)
    }

    fn replica_collection() -> PublishedCollection {
        PublishedCollection {
            name: "objects".to_owned(),
            mode: PublicServingMode::MaterializedReplica,
            replica_alias: Some("rep".to_owned()),
        }
    }

    fn text_cell(value: &str) -> Json {
        json!({"type": "text", "value": value})
    }

    fn int_cell(value: i64) -> Json {
        json!({"type": "integer", "value": value.to_string()})
    }

    fn exec_result(cols: &[&str], rows: Json) -> Json {
        json!({"type": "ok", "response": {"type": "execute", "result": {
            "cols": cols.iter().map(|name| json!({"name": name, "decltype": null})).collect::<Vec<_>>(),
            "rows": rows,
            "affected_row_count": 0,
            "last_insert_rowid": null
        }}})
    }

    fn pipeline_ok(results: Vec<Json>) -> Result<TransportResponse, TransportError> {
        let mut results = results;
        results.push(json!({"type": "ok", "response": {"type": "close"}}));
        Ok(TransportResponse {
            status: 200,
            body: json!({"baton": null, "base_url": null, "results": results}).to_string(),
        })
    }

    fn empty_execs(count: usize) -> Result<TransportResponse, TransportError> {
        pipeline_ok((0..count).map(|_| exec_result(&[], json!([]))).collect())
    }

    /// The source's schema answer for the `objects` collection: its
    /// CREATE TABLE plus one index.
    fn schema_outcome() -> Result<TransportResponse, TransportError> {
        pipeline_ok(vec![exec_result(
            &["type", "name", "sql"],
            json!([
                [
                    text_cell("table"),
                    text_cell("objects"),
                    text_cell(
                        "CREATE TABLE \"objects\" (\"id\" INTEGER PRIMARY KEY, \"label\" TEXT)"
                    )
                ],
                [
                    text_cell("index"),
                    text_cell("idx_objects_label"),
                    text_cell("CREATE INDEX \"idx_objects_label\" ON \"objects\" (\"label\")")
                ]
            ]),
        )])
    }

    fn page_outcome(rows: Json) -> Result<TransportResponse, TransportError> {
        pipeline_ok(vec![exec_result(&["id", "label"], rows)])
    }

    fn test_policy() -> ReplicaPolicy {
        ReplicaPolicy {
            page_rows: 2,
            insert_rows_per_statement: 2,
            statements_per_call: 25,
        }
    }

    /// Script a full successful materialization of a 3-row `objects`
    /// collection (pages of 2 + 1 under `test_policy`).
    fn script_successful_copy(transport: &RouterTransport) {
        transport.push(SOURCE_URL, schema_outcome());
        transport.push(
            SOURCE_URL,
            page_outcome(json!([
                [int_cell(1), text_cell("a")],
                [int_cell(2), text_cell("b")]
            ])),
        );
        transport.push(
            SOURCE_URL,
            page_outcome(json!([[int_cell(3), text_cell("c")]])),
        );
        transport.push(REPLICA_URL, empty_execs(3)); // build: drop + create + index
        transport.push(REPLICA_URL, empty_execs(1)); // insert page 1
        transport.push(REPLICA_URL, empty_execs(1)); // insert page 2
        transport.push(REPLICA_URL, empty_execs(2)); // swap: drop + rename
    }

    /// Oracle: decision record #5802 D4 — a materialized replica is a
    /// copy of exactly the published collection (its schema and rows),
    /// built on the owner's side and swapped in atomically; the cut
    /// record states rows, epoch, and completion time.
    #[tokio::test]
    async fn test_materialize_copies_schema_and_rows_then_swaps() {
        let transport = RouterTransport::default();
        script_successful_copy(&transport);
        let maintainer = maintainer(transport.clone());

        let cut = maintainer
            .materialize(
                "src",
                &replica_collection(),
                &test_policy(),
                7,
                1_700_000_000,
            )
            .await
            .unwrap();
        assert_eq!(
            cut,
            Materialization {
                collection: "objects".to_owned(),
                replica_alias: "rep".to_owned(),
                rows: 3,
                epoch: 7,
                completed_at: 1_700_000_000,
            }
        );

        // The source saw the schema read and two paged row reads —
        // and nothing else.
        let source_sql = transport.sql_sent_to(SOURCE_URL);
        assert_eq!(source_sql.len(), 3);
        assert!(source_sql[0].contains("sqlite_master"));
        assert_eq!(source_sql[1], "SELECT * FROM \"objects\" LIMIT ? OFFSET ?");
        assert_eq!(source_sql[2], "SELECT * FROM \"objects\" LIMIT ? OFFSET ?");

        // The replica saw: shadow build under the renamed schema,
        // the row inserts, and only then the swap.
        assert_eq!(
            transport.sql_sent_to(REPLICA_URL),
            vec![
                "DROP TABLE IF EXISTS \"objects__id_materializing\"",
                "CREATE TABLE \"objects__id_materializing\" (\"id\" INTEGER PRIMARY KEY, \"label\" TEXT)",
                "CREATE INDEX \"objects__id_materializing__idx_objects_label\" ON \"objects__id_materializing\" (\"label\")",
                "INSERT INTO \"objects__id_materializing\" VALUES (?,?),(?,?)",
                "INSERT INTO \"objects__id_materializing\" VALUES (?,?)",
                "DROP TABLE IF EXISTS \"objects\"",
                "ALTER TABLE \"objects__id_materializing\" RENAME TO \"objects\"",
            ]
        );

        // Values crossed exactly as wire values, in row order.
        let replica_bodies = transport.bodies_sent_to(REPLICA_URL);
        let insert_body: Json = serde_json::from_str(&replica_bodies[1]).unwrap();
        assert_eq!(
            insert_body["requests"][0]["stmt"]["args"],
            json!([
                {"type": "integer", "value": "1"},
                {"type": "text", "value": "a"},
                {"type": "integer", "value": "2"},
                {"type": "text", "value": "b"}
            ])
        );
    }

    /// Oracle: #14784 checkpoint — the maintainer's seam is for
    /// replica-served collections only; a live-served collection is
    /// refused before any I/O.
    #[tokio::test]
    async fn test_materialize_refuses_live_collection() {
        let transport = RouterTransport::default();
        let maintainer = maintainer(transport.clone());
        let live = PublishedCollection {
            name: "objects".to_owned(),
            mode: PublicServingMode::Live,
            replica_alias: None,
        };
        let error = maintainer
            .materialize("src", &live, &test_policy(), 1, 1)
            .await
            .unwrap_err();
        assert!(matches!(error, AdapterError::Config(_)), "{error:?}");
        assert_eq!(transport.sql_sent_to(SOURCE_URL), [] as [String; 0]);
        assert_eq!(transport.sql_sent_to(REPLICA_URL), [] as [String; 0]);
    }

    /// Oracle: D4 + the seam's guard — the source store is never a
    /// write target, so a replica alias equal to the source alias is
    /// refused before any I/O.
    #[tokio::test]
    async fn test_materialize_refuses_replica_alias_equal_to_source() {
        let transport = RouterTransport::default();
        let maintainer = maintainer(transport.clone());
        let mut collection = replica_collection();
        collection.replica_alias = Some("src".to_owned());
        let error = maintainer
            .materialize("src", &collection, &test_policy(), 1, 1)
            .await
            .unwrap_err();
        assert!(matches!(error, AdapterError::Config(_)), "{error:?}");
        assert_eq!(transport.sql_sent_to(SOURCE_URL), [] as [String; 0]);
    }

    /// Oracle: invariant 8 — a collection the source does not have is
    /// a loud config error, and the replica is never touched.
    #[tokio::test]
    async fn test_materialize_missing_source_table_is_config_error() {
        let transport = RouterTransport::default();
        transport.push(
            SOURCE_URL,
            pipeline_ok(vec![exec_result(&["type", "name", "sql"], json!([]))]),
        );
        let maintainer = maintainer(transport.clone());
        let error = maintainer
            .materialize("src", &replica_collection(), &test_policy(), 1, 1)
            .await
            .unwrap_err();
        assert!(matches!(error, AdapterError::Config(_)), "{error:?}");
        assert_eq!(transport.sql_sent_to(REPLICA_URL), [] as [String; 0]);
    }

    /// Oracle: the atomic-swap policy — a copy that fails mid-way
    /// never swaps, so readers keep the previous replica (or its
    /// absence); the failure is the classified statement error, with
    /// no server message copied out.
    #[tokio::test]
    async fn test_failed_copy_never_swaps() {
        let transport = RouterTransport::default();
        transport.push(SOURCE_URL, schema_outcome());
        transport.push(
            SOURCE_URL,
            page_outcome(json!([[int_cell(1), text_cell("a")]])),
        );
        transport.push(REPLICA_URL, empty_execs(3));
        transport.push(
            REPLICA_URL,
            pipeline_ok(vec![json!({
                "type": "error",
                "error": {"message": "constraint failed in secret detail", "code": "SQLITE_CONSTRAINT"}
            })]),
        );
        let maintainer = maintainer(transport.clone());
        let error = maintainer
            .materialize("src", &replica_collection(), &test_policy(), 1, 1)
            .await
            .unwrap_err();
        assert_eq!(error, AdapterError::RemoteStatement { status: 0 });
        assert!(!error.to_string().contains("secret detail"));
        let replica_sql = transport.sql_sent_to(REPLICA_URL);
        assert!(
            !replica_sql.iter().any(|sql| sql.contains("RENAME")),
            "{replica_sql:?}"
        );
    }

    /// Oracle: invariant 5 — an auth failure on the replica names the
    /// credential reference, never a token value.
    #[tokio::test]
    async fn test_replica_auth_error_names_reference_not_token() {
        let transport = RouterTransport::default();
        transport.push(SOURCE_URL, schema_outcome());
        let mut config = maintainer_config();
        config.aliases.get_mut("rep").unwrap().credential_ref = "env:MISSING_TOKEN".to_owned();
        let maintainer = ReplicaMaintainer::with_parts(config, transport.clone(), MapCredentials);
        let error = maintainer
            .materialize("src", &replica_collection(), &test_policy(), 1, 1)
            .await
            .unwrap_err();
        assert_eq!(
            error,
            AdapterError::Auth {
                credential_ref: "env:MISSING_TOKEN".to_owned()
            }
        );
        assert!(!error.to_string().contains("token-value"));
    }

    /// Oracle: decision record #5802 D4 — unpublish deletes the
    /// materialized replica; deletion is idempotent and also clears
    /// any leftover shadow.
    #[tokio::test]
    async fn test_delete_replica_drops_table_and_shadow() {
        let transport = RouterTransport::default();
        transport.push(
            REPLICA_URL,
            pipeline_ok(vec![exec_result(
                &["name"],
                json!([[text_cell("objects")]]),
            )]),
        );
        transport.push(REPLICA_URL, empty_execs(2));
        let maintainer = maintainer(transport.clone());
        assert!(
            maintainer
                .delete_replica(&replica_collection())
                .await
                .unwrap()
        );
        assert_eq!(
            transport.sql_sent_to(REPLICA_URL)[1..],
            [
                "DROP TABLE IF EXISTS \"objects__id_materializing\"",
                "DROP TABLE IF EXISTS \"objects\"",
            ]
        );
    }

    /// Oracle: D4/D5 — deleting an absent replica is a no-op outcome,
    /// not an error (unpublish of a share whose replica never
    /// materialized still completes).
    #[tokio::test]
    async fn test_delete_replica_absent_is_false_not_error() {
        let transport = RouterTransport::default();
        transport.push(
            REPLICA_URL,
            pipeline_ok(vec![exec_result(&["name"], json!([]))]),
        );
        transport.push(REPLICA_URL, empty_execs(2));
        let maintainer = maintainer(transport.clone());
        assert!(
            !maintainer
                .delete_replica(&replica_collection())
                .await
                .unwrap()
        );
    }

    /// Oracle: D4/D5 drivers — publish materializes only the
    /// replica-served collections of the scope; unpublish deletes
    /// only those, and live-served collections are untouched by both.
    #[tokio::test]
    async fn test_publish_and_unpublish_effects_touch_only_replica_collections() {
        let scope = PublicationScope::new([
            PublishedCollection {
                name: "notes".to_owned(),
                mode: PublicServingMode::Live,
                replica_alias: None,
            },
            replica_collection(),
        ])
        .unwrap();

        let transport = RouterTransport::default();
        script_successful_copy(&transport);
        transport.push(
            REPLICA_URL,
            pipeline_ok(vec![exec_result(
                &["name"],
                json!([[text_cell("objects")]]),
            )]),
        );
        transport.push(REPLICA_URL, empty_execs(2));
        let maintainer = maintainer(transport.clone());

        let cuts = maintainer
            .publish_effects("src", &scope, &test_policy(), 3, 42)
            .await
            .unwrap();
        assert_eq!(cuts.len(), 1);
        assert_eq!(cuts[0].collection, "objects");
        assert_eq!(cuts[0].epoch, 3);

        let deleted = maintainer.unpublish_effects(&scope).await.unwrap();
        assert_eq!(deleted, vec!["objects".to_owned()]);
        // No statement ever named the live-served collection.
        assert!(
            !transport
                .sql_sent_to(REPLICA_URL)
                .iter()
                .any(|sql| sql.contains("notes"))
        );
        assert!(
            !transport
                .sql_sent_to(SOURCE_URL)
                .iter()
                .any(|sql| sql.contains("notes"))
        );
    }

    /// Oracle: the DDL rename is verified, never guessed — a CREATE
    /// TABLE naming a different table, or a shape that is not a
    /// CREATE TABLE at all, is refused.
    #[test]
    fn test_rename_table_ddl_is_verified() {
        assert_eq!(
            rename_table_ddl(
                "CREATE TABLE objects (id INTEGER PRIMARY KEY)",
                "objects",
                "objects__id_materializing",
            )
            .unwrap(),
            "CREATE TABLE \"objects__id_materializing\" (id INTEGER PRIMARY KEY)"
        );
        assert_eq!(
            rename_table_ddl(
                "create table if not exists \"Objects\" (id integer)",
                "objects",
                "shadow",
            )
            .unwrap(),
            "create table if not exists \"shadow\" (id integer)"
        );
        assert!(rename_table_ddl("CREATE TABLE other (id INTEGER)", "objects", "s").is_err());
        assert!(rename_table_ddl("DROP TABLE objects", "objects", "s").is_err());
    }

    /// Oracle: the index rename moves both the index name and the
    /// table it is built on, and refuses an index over another table.
    #[test]
    fn test_rename_index_ddl_is_verified() {
        assert_eq!(
            rename_index_ddl(
                "CREATE UNIQUE INDEX idx ON objects (label)",
                "idx",
                "shadow__idx",
                "objects",
                "shadow",
            )
            .unwrap(),
            "CREATE UNIQUE INDEX \"shadow__idx\" ON \"shadow\" (label)"
        );
        assert!(
            rename_index_ddl(
                "CREATE INDEX idx ON other (label)",
                "idx",
                "shadow__idx",
                "objects",
                "shadow",
            )
            .is_err()
        );
    }
}

// ---------------------------------------------------------------------------
// Config and credentials
// ---------------------------------------------------------------------------

/// Local operator config for one alias. The URL and the credential
/// reference live only here — never in ID data (record #5806 item 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasConfig {
    /// Base URL of the libsql server (https; http for a local server).
    pub url: String,
    /// Credential reference resolved at call time, e.g. `env:TURSO_TOKEN`.
    pub credential_ref: String,
    /// Per-statement deadline for this alias.
    pub statement_timeout: Duration,
    /// Largest single BLOB argument for this alias.
    pub max_blob_bytes: usize,
}

impl Default for AliasConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            credential_ref: String::new(),
            statement_timeout: DEFAULT_STATEMENT_TIMEOUT,
            max_blob_bytes: MAX_BLOB_BYTES,
        }
    }
}

/// All locally configured foreign libsql stores, by alias.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForeignConfig {
    /// Alias → config. Aliases are the only part of this that ID data
    /// may name.
    pub aliases: BTreeMap<String, AliasConfig>,
}

impl ForeignConfig {
    /// Look up one alias.
    pub fn alias(&self, alias: &str) -> Result<&AliasConfig, AdapterError> {
        self.aliases
            .get(alias)
            .ok_or_else(|| AdapterError::UnknownAlias(alias.to_owned()))
    }
}

/// Parse `id://foreign/libsql/<alias>` to its alias. Anything else is a
/// config error; the alias is the only part an ID object may carry.
pub fn parse_foreign_uri(uri: &str) -> Result<String, AdapterError> {
    let alias = uri
        .strip_prefix(FOREIGN_LIBSQL_PREFIX)
        .ok_or_else(|| AdapterError::Config(format!("not a libsql foreign URI: {uri}")))?;
    if alias.is_empty()
        || !alias
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(AdapterError::Config(format!(
            "invalid foreign-store alias in URI: {uri}"
        )));
    }
    Ok(alias.to_owned())
}

/// Resolves a credential reference to a token at call time. The token is
/// used for one call's Authorization header and is never stored by the
/// adapter, logged, or placed in ID data.
pub trait CredentialResolver: Send + Sync {
    /// Resolve `reference` (e.g. `env:NAME`) to a token.
    fn resolve(&self, reference: &str) -> Result<String, AdapterError>;
}

/// Resolves `env:VAR` references from the process environment. Other
/// reference schemes belong to resolvers backed by the deployment's
/// secret store; they plug in through [`CredentialResolver`].
#[derive(Debug, Default, Clone, Copy)]
pub struct EnvCredentialResolver;

impl CredentialResolver for EnvCredentialResolver {
    fn resolve(&self, reference: &str) -> Result<String, AdapterError> {
        let var = reference.strip_prefix("env:").ok_or_else(|| {
            AdapterError::Config(format!(
                "unsupported credential reference scheme: {reference}"
            ))
        })?;
        std::env::var(var).map_err(|_| AdapterError::Auth {
            credential_ref: reference.to_owned(),
        })
    }
}

// ---------------------------------------------------------------------------
// Transport: raw HTTP/1.1, HTTPS via rustls when the URL asks for it
// ---------------------------------------------------------------------------

/// One HTTP response from the transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response body as text (the pipeline protocol is JSON).
    pub body: String,
}

/// Transport-level failure classes. Messages carry no URL and no token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// TCP connect failed.
    Connect(String),
    /// TLS setup failed.
    Tls(String),
    /// Read/write failed mid-request.
    Io(String),
    /// The overall deadline elapsed.
    Timeout,
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect(msg) => write!(f, "connect failed: {msg}"),
            Self::Tls(msg) => write!(f, "tls failed: {msg}"),
            Self::Io(msg) => write!(f, "io failed: {msg}"),
            Self::Timeout => write!(f, "deadline elapsed"),
        }
    }
}

impl std::error::Error for TransportError {}

/// The HTTP transport seam. The production implementation is
/// [`HttpTransport`]; tests substitute scripted transports so the
/// protocol, retry, and privacy behaviour have deterministic oracles.
pub trait Transport: Send + Sync {
    /// POST `body` to `{base_url}/v2/pipeline` with a bearer token.
    fn post_pipeline<'a>(
        &'a self,
        base_url: &'a str,
        token: &'a str,
        body: &'a str,
        connect_timeout: Duration,
        deadline: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<TransportResponse, TransportError>> + Send + 'a>>;
}

struct ParsedUrl {
    https: bool,
    host: String,
    port: u16,
    path_prefix: String,
}

fn parse_base_url(base_url: &str) -> Result<ParsedUrl, TransportError> {
    let (https, rest) = if let Some(rest) = base_url.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = base_url.strip_prefix("http://") {
        (false, rest)
    } else {
        return Err(TransportError::Connect(
            "base URL must be http:// or https://".to_owned(),
        ));
    };
    let (authority, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, ""),
    };
    let (host, port) = match authority.rfind(':') {
        Some(idx) if authority[idx + 1..].chars().all(|c| c.is_ascii_digit()) => (
            authority[..idx].to_owned(),
            authority[idx + 1..]
                .parse::<u16>()
                .map_err(|_| TransportError::Connect("bad port".to_owned()))?,
        ),
        _ => (authority.to_owned(), if https { 443 } else { 80 }),
    };
    if host.is_empty() {
        return Err(TransportError::Connect("empty host".to_owned()));
    }
    Ok(ParsedUrl {
        https,
        host,
        port,
        path_prefix: path.trim_end_matches('/').to_owned(),
    })
}

/// Production transport: HTTP/1.1 over TCP, TLS via rustls + WebPKI roots
/// for https URLs. One request per connection (`Connection: close`), the
/// shape the v2 pipeline endpoint expects.
#[derive(Debug, Default, Clone, Copy)]
pub struct HttpTransport;

impl HttpTransport {
    async fn round_trip(
        base_url: &str,
        token: &str,
        body: &str,
        connect_timeout: Duration,
    ) -> Result<TransportResponse, TransportError> {
        let url = parse_base_url(base_url)?;
        let connect = tokio::net::TcpStream::connect((url.host.as_str(), url.port));
        let tcp = tokio::time::timeout(connect_timeout, connect)
            .await
            .map_err(|_| TransportError::Timeout)?
            .map_err(|e| TransportError::Connect(e.to_string()))?;

        let path = format!("{}/v2/pipeline", url.path_prefix);
        let request = format!(
            "POST {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {token}\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            url.host,
            body.len(),
        );

        let raw = if url.https {
            let stream = Self::wrap_tls(&url, tcp).await?;
            Self::exchange(stream, request.as_bytes()).await?
        } else {
            Self::exchange(tcp, request.as_bytes()).await?
        };
        parse_http_response(&raw)
    }

    async fn wrap_tls(
        url: &ParsedUrl,
        tcp: tokio::net::TcpStream,
    ) -> Result<tokio_rustls::client::TlsStream<tokio::net::TcpStream>, TransportError> {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
        let server_name = rustls::pki_types::ServerName::try_from(url.host.clone())
            .map_err(|_| TransportError::Tls("invalid server name".to_owned()))?;
        connector
            .connect(server_name, tcp)
            .await
            .map_err(|e| TransportError::Tls(e.to_string()))
    }

    async fn exchange<S>(mut stream: S, request: &[u8]) -> Result<Vec<u8>, TransportError>
    where
        S: AsyncReadExt + AsyncWriteExt + Unpin,
    {
        stream
            .write_all(request)
            .await
            .map_err(|e| TransportError::Io(e.to_string()))?;
        let mut raw = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let read = stream
                .read(&mut chunk)
                .await
                .map_err(|e| TransportError::Io(e.to_string()))?;
            if read == 0 {
                break;
            }
            raw.extend_from_slice(&chunk[..read]);
            if raw.len() > MAX_RESPONSE_BYTES {
                return Err(TransportError::Io("response too large".to_owned()));
            }
        }
        Ok(raw)
    }
}

impl Transport for HttpTransport {
    fn post_pipeline<'a>(
        &'a self,
        base_url: &'a str,
        token: &'a str,
        body: &'a str,
        connect_timeout: Duration,
        deadline: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<TransportResponse, TransportError>> + Send + 'a>> {
        Box::pin(async move {
            tokio::time::timeout(
                deadline,
                Self::round_trip(base_url, token, body, connect_timeout),
            )
            .await
            .map_err(|_| TransportError::Timeout)?
        })
    }
}

// #8504: chunk framing is parsed on byte slices and the chunk size is a
// byte count — never a string boundary. The old code lossy-decoded the whole
// response first, so a multi-byte UTF-8 sequence split across chunks became
// replacement characters and slicing at the byte count panicked. Only the
// fully assembled body is decoded (lossy, the prior body contract).
fn parse_http_response(raw: &[u8]) -> Result<TransportResponse, TransportError> {
    let head_end = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| TransportError::Io("response has no header block".to_owned()))?;
    let head = String::from_utf8_lossy(&raw[..head_end]);
    let body_bytes = &raw[head_end + 4..];
    let status_line = head.lines().next().unwrap_or_default();
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| TransportError::Io("response has no status code".to_owned()))?;
    let mut transfer_encoding_seen = false;
    let mut chunked = false;
    for line in head.lines().skip(1) {
        // RFC 9112 section 5.2: obs-fold is deprecated and a folded
        // continuation would otherwise be skipped here, letting the
        // unfolded field value smuggle codings past the check below
        // (e.g. "Transfer-Encoding: chunked" folded with ", gzip").
        // Reject any folded header line outright.
        if line.starts_with([' ', '\t']) {
            return Err(TransportError::Io("folded header field".to_owned()));
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.eq_ignore_ascii_case("transfer-encoding") {
            continue;
        }
        transfer_encoding_seen = true;
        for coding in value.split(',') {
            // Only HTTP optional whitespace (SP / HTAB) may surround a
            // transfer coding; str::trim would also strip e.g. VT/FF and
            // accept a malformed field.
            let coding = coding.trim_matches(|c| matches!(c, ' ' | '\t'));
            if !coding.eq_ignore_ascii_case("chunked") || chunked {
                return Err(TransportError::Io(
                    "unsupported transfer encoding".to_owned(),
                ));
            }
            chunked = true;
        }
    }
    if transfer_encoding_seen && !chunked {
        return Err(TransportError::Io(
            "unsupported transfer encoding".to_owned(),
        ));
    }
    let body_bytes = if chunked {
        dechunk(body_bytes)?
    } else {
        body_bytes.to_vec()
    };
    let body = String::from_utf8_lossy(&body_bytes).into_owned();
    Ok(TransportResponse { status, body })
}

fn is_http_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'..=b'\'' | b'*' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
        )
}

fn skip_optional_whitespace(mut bytes: &[u8]) -> &[u8] {
    while matches!(bytes.first(), Some(b' ' | b'\t')) {
        bytes = &bytes[1..];
    }
    bytes
}

fn quoted_extension_value_len(bytes: &[u8]) -> Option<usize> {
    if !bytes.starts_with(b"\"") {
        return None;
    }
    let mut index = 1;
    while let Some(&byte) = bytes.get(index) {
        match byte {
            b'"' => return Some(index + 1),
            b'\\' => {
                let escaped = *bytes.get(index + 1)?;
                if !(matches!(escaped, b'\t' | b' ')
                    || (0x21..=0x7e).contains(&escaped)
                    || escaped >= 0x80)
                {
                    return None;
                }
                index += 2;
            }
            b'\t' | b' ' | b'!' | b'#'..=b'[' | b']'..=b'~' => index += 1,
            0x80..=0xff => index += 1,
            _ => return None,
        }
    }
    None
}

fn valid_chunk_extensions(mut bytes: &[u8]) -> bool {
    while !bytes.is_empty() {
        bytes = skip_optional_whitespace(bytes);
        let Some(after_semicolon) = bytes.strip_prefix(b";") else {
            return false;
        };
        bytes = skip_optional_whitespace(after_semicolon);
        let name_len = bytes
            .iter()
            .take_while(|byte| is_http_token_byte(**byte))
            .count();
        if name_len == 0 {
            return false;
        }
        let after_name = &bytes[name_len..];
        if let Some(after_equals) = skip_optional_whitespace(after_name).strip_prefix(b"=") {
            bytes = skip_optional_whitespace(after_equals);
            let value_len = if bytes.starts_with(b"\"") {
                let Some(value_len) = quoted_extension_value_len(bytes) else {
                    return false;
                };
                value_len
            } else {
                bytes
                    .iter()
                    .take_while(|byte| is_http_token_byte(**byte))
                    .count()
            };
            if value_len == 0 {
                return false;
            }
            bytes = &bytes[value_len..];
        } else {
            bytes = after_name;
        }
    }
    true
}

fn valid_trailer_field(line: &[u8]) -> bool {
    let Some(colon) = line.iter().position(|byte| *byte == b':') else {
        return false;
    };
    !line[..colon].is_empty()
        && line[..colon].iter().copied().all(is_http_token_byte)
        && line[colon + 1..].iter().all(|byte| {
            matches!(byte, b'\t' | b' ') || (0x21..=0x7e).contains(byte) || *byte >= 0x80
        })
}

fn dechunk(body: &[u8]) -> Result<Vec<u8>, TransportError> {
    let mut out = Vec::new();
    let mut rest = body;
    loop {
        let line_end = rest
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| TransportError::Io("bad chunk framing".to_owned()))?;
        let size_line = &rest[..line_end];
        let size_end = size_line
            .iter()
            .position(|byte| !byte.is_ascii_hexdigit())
            .unwrap_or(size_line.len());
        let size_bytes = &size_line[..size_end];
        let size_text = std::str::from_utf8(size_bytes)
            .map_err(|_| TransportError::Io("bad chunk size".to_owned()))?;
        if size_text.is_empty() {
            return Err(TransportError::Io("bad chunk size".to_owned()));
        }
        if !valid_chunk_extensions(&size_line[size_end..]) {
            return Err(TransportError::Io("bad chunk extension".to_owned()));
        }
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| TransportError::Io("bad chunk size".to_owned()))?;
        rest = &rest[line_end + 2..];
        if size == 0 {
            loop {
                let trailer_end = rest
                    .windows(2)
                    .position(|window| window == b"\r\n")
                    .ok_or_else(|| TransportError::Io("bad chunk framing".to_owned()))?;
                if trailer_end == 0 {
                    rest = &rest[2..];
                    break;
                }
                if !valid_trailer_field(&rest[..trailer_end]) {
                    return Err(TransportError::Io("bad chunk trailer".to_owned()));
                }
                rest = &rest[trailer_end + 2..];
            }
            if !rest.is_empty() {
                return Err(TransportError::Io(
                    "extra data after chunked body".to_owned(),
                ));
            }
            return Ok(out);
        }
        if rest.len() < size {
            return Err(TransportError::Io("short chunk".to_owned()));
        }
        out.extend_from_slice(&rest[..size]);
        rest = rest[size..]
            .strip_prefix(b"\r\n")
            .ok_or_else(|| TransportError::Io("bad chunk framing".to_owned()))?;
    }
}

// ---------------------------------------------------------------------------
// Circuit breaker (per alias)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BreakerState {
    Closed { failures: u32 },
    Open { since: Instant },
    HalfOpen,
}

/// Per-alias circuit breaker (record #5806 item 7): opens after repeated
/// transient failures, half-opens on a timer, closes on a successful
/// probe. Time is injected so tests are deterministic.
#[derive(Debug)]
pub struct CircuitBreaker {
    state: Mutex<BreakerState>,
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new()
    }
}

impl CircuitBreaker {
    /// A closed breaker with no recorded failures.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Mutex::new(BreakerState::Closed { failures: 0 }),
        }
    }

    /// Whether a call may proceed at `now`; transitions Open → HalfOpen
    /// once the half-open delay has elapsed.
    pub fn allow(&self, now: Instant) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        match *state {
            BreakerState::Closed { .. } => true,
            BreakerState::HalfOpen => false,
            BreakerState::Open { since } => {
                if now.duration_since(since) >= BREAKER_HALF_OPEN_AFTER {
                    *state = BreakerState::HalfOpen;
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Record a successful call: the breaker closes.
    pub fn record_success(&self) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        *state = BreakerState::Closed { failures: 0 };
    }

    /// Record a transient failure: increments the counter, or re-opens a
    /// half-open breaker, opening the breaker at the threshold.
    pub fn record_transient_failure(&self, now: Instant) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        *state = match *state {
            BreakerState::Closed { failures } => {
                let failures = failures + 1;
                if failures >= BREAKER_FAILURE_THRESHOLD {
                    BreakerState::Open { since: now }
                } else {
                    BreakerState::Closed { failures }
                }
            }
            BreakerState::HalfOpen | BreakerState::Open { .. } => BreakerState::Open { since: now },
        };
    }

    /// Re-open after a half-open probe cannot establish recovery. Other
    /// non-transient failures leave a closed breaker unchanged.
    fn record_probe_failure(&self, now: Instant) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if matches!(*state, BreakerState::HalfOpen) {
            *state = BreakerState::Open { since: now };
        }
    }
}

/// Releases a half-open probe reservation when the admitted call ends
/// without recording an outcome — above all when its future is
/// cancelled while awaiting the transport: HalfOpen has no timed exit,
/// so a dropped probe would refuse every later call for the alias
/// forever. The guard re-opens the breaker on drop, which is a no-op
/// unless the state is still HalfOpen, so calls that did record an
/// outcome are unaffected.
struct HalfOpenProbeGuard<'a> {
    breaker: &'a CircuitBreaker,
}

impl Drop for HalfOpenProbeGuard<'_> {
    fn drop(&mut self) {
        self.breaker.record_probe_failure(Instant::now());
    }
}

// ---------------------------------------------------------------------------
// The adapter
// ---------------------------------------------------------------------------

/// The libsql foreign-store adapter. Parameterized over transport and
/// credential resolver so tests can script both deterministically.
#[derive(Debug)]
pub struct LibsqlAdapter<T: Transport, C: CredentialResolver> {
    config: ForeignConfig,
    transport: T,
    credentials: C,
    breakers: Mutex<BTreeMap<String, Arc<CircuitBreaker>>>,
    retry_backoff: Duration,
}

impl LibsqlAdapter<HttpTransport, EnvCredentialResolver> {
    /// The production adapter: HTTP transport, environment credentials.
    #[must_use]
    pub fn new(config: ForeignConfig) -> Self {
        Self::with_parts(config, HttpTransport, EnvCredentialResolver)
    }
}

impl<T: Transport, C: CredentialResolver> LibsqlAdapter<T, C> {
    /// Build an adapter from explicit parts (tests, alternate resolvers).
    #[must_use]
    pub fn with_parts(config: ForeignConfig, transport: T, credentials: C) -> Self {
        Self {
            config,
            transport,
            credentials,
            breakers: Mutex::new(BTreeMap::new()),
            retry_backoff: Duration::from_millis(100),
        }
    }

    /// Backoff base between read retries (zero in tests).
    pub fn set_retry_backoff(&mut self, backoff: Duration) {
        self.retry_backoff = backoff;
    }

    fn breaker_for(&self, alias: &str) -> Arc<CircuitBreaker> {
        let mut breakers = self.breakers.lock().unwrap_or_else(|p| p.into_inner());
        breakers
            .entry(alias.to_owned())
            .or_insert_with(|| Arc::new(CircuitBreaker::new()))
            .clone()
    }

    /// Run one granted read query against `id://foreign/libsql/<alias>`.
    pub async fn query_uri(
        &self,
        uri: &str,
        query: &GrantedQuery,
    ) -> Result<Vec<Row>, AdapterError> {
        let alias = parse_foreign_uri(uri)?;
        self.query(&alias, query).await
    }

    /// Run one granted read query against the named alias.
    ///
    /// Behaviour (record #5806 items 5, 7, 8): the grant predicate is
    /// pushed into the single SELECT; transient failures retry up to
    /// [`MAX_ATTEMPTS`] total attempts with bounded backoff; auth failure
    /// is fatal; an open circuit breaker refuses before any network I/O;
    /// not-found and not-granted are the same empty result.
    pub async fn query(&self, alias: &str, query: &GrantedQuery) -> Result<Vec<Row>, AdapterError> {
        let alias_config = self.config.alias(alias)?.clone();
        let (sql, args) = query.build_statement(alias_config.max_blob_bytes)?;
        let body = pipeline_body(&sql, &args);

        let breaker = self.breaker_for(alias);
        let mut attempt: u32 = 0;
        loop {
            attempt += 1;
            if !breaker.allow(Instant::now()) {
                return Err(AdapterError::CircuitOpen(alias.to_owned()));
            }
            // Held for the rest of the attempt: if this future is
            // dropped before the attempt records an outcome (caller
            // cancellation while awaiting the transport), the guard
            // re-opens the breaker instead of stranding the half-open
            // probe reservation.
            let _probe_guard = HalfOpenProbeGuard { breaker: &breaker };
            // The token is resolved per attempt and never stored.
            let token = match self.credentials.resolve(&alias_config.credential_ref) {
                Ok(token) => token,
                Err(error) => {
                    breaker.record_probe_failure(Instant::now());
                    return Err(error);
                }
            };
            let outcome = self
                .transport
                .post_pipeline(
                    &alias_config.url,
                    &token,
                    &body,
                    CONNECT_TIMEOUT,
                    alias_config.statement_timeout,
                )
                .await;
            let result = match outcome {
                Ok(response) => classify_response(&response, &alias_config, query),
                Err(TransportError::Timeout) => Err(AdapterError::Timeout),
                Err(other) => Err(AdapterError::Transient(other.to_string())),
            };
            match result {
                Ok(rows) => {
                    breaker.record_success();
                    return Ok(rows);
                }
                Err(error) if error.is_transient() => {
                    breaker.record_transient_failure(Instant::now());
                    if attempt >= MAX_ATTEMPTS {
                        return Err(error);
                    }
                    if !self.retry_backoff.is_zero() {
                        tokio::time::sleep(self.retry_backoff * attempt).await;
                    }
                }
                Err(AdapterError::Auth { .. }) => {
                    // Fatal and never retried; the token is not in the error.
                    breaker.record_probe_failure(Instant::now());
                    return Err(AdapterError::Auth {
                        credential_ref: alias_config.credential_ref.clone(),
                    });
                }
                Err(error) => {
                    breaker.record_probe_failure(Instant::now());
                    return Err(error);
                }
            }
        }
    }
}

fn pipeline_body(sql: &str, args: &[WireValue]) -> String {
    let args_json: Vec<Json> = args.iter().map(WireValue::to_json).collect();
    json!({
        "baton": null,
        "requests": [
            {"type": "execute", "stmt": {"sql": sql, "args": args_json}},
            {"type": "close"}
        ]
    })
    .to_string()
}

/// Map one HTTP response onto rows or a classified error. Server body
/// text is used only inside the success parse; error paths classify by
/// status/class alone so ungranted metadata cannot leak through errors.
fn classify_response(
    response: &TransportResponse,
    alias_config: &AliasConfig,
    query: &GrantedQuery,
) -> Result<Vec<Row>, AdapterError> {
    match response.status {
        200 => parse_pipeline_rows(&response.body, query),
        401 | 403 => Err(AdapterError::Auth {
            credential_ref: alias_config.credential_ref.clone(),
        }),
        400..=499 => Err(AdapterError::RemoteStatement {
            status: response.status,
        }),
        _ => Err(AdapterError::Transient(format!(
            "remote status {}",
            response.status
        ))),
    }
}

fn parse_pipeline_rows(body: &str, query: &GrantedQuery) -> Result<Vec<Row>, AdapterError> {
    let parsed: Json = id_core::json_unique::parse_json_without_duplicate_keys(body)
        .map_err(|_| AdapterError::MalformedResponse)?;
    let results = parsed
        .get("results")
        .and_then(Json::as_array)
        .ok_or(AdapterError::MalformedResponse)?;
    let first = results.first().ok_or(AdapterError::MalformedResponse)?;
    if first.get("type").and_then(Json::as_str) != Some("ok") {
        // A pipeline-level error result is a deterministic statement
        // failure; its message is deliberately not copied out.
        return Err(AdapterError::RemoteStatement { status: 0 });
    }
    let result = first
        .get("response")
        .and_then(|r| r.get("result"))
        .ok_or(AdapterError::MalformedResponse)?;
    let cols = result
        .get("cols")
        .and_then(Json::as_array)
        .ok_or(AdapterError::MalformedResponse)?;
    let mut names = Vec::with_capacity(cols.len());
    for col in cols {
        names.push(
            col.get("name")
                .and_then(Json::as_str)
                .ok_or(AdapterError::MalformedResponse)?
                .to_owned(),
        );
    }
    // The returned schema must be exactly the granted view schema: any
    // deviation means the remote did not run the query as contracted.
    let expected: Vec<String> = query.columns.iter().map(|(n, _)| n.clone()).collect();
    if names != expected {
        return Err(AdapterError::MalformedResponse);
    }
    let raw_rows = result
        .get("rows")
        .and_then(Json::as_array)
        .ok_or(AdapterError::MalformedResponse)?;
    let mut rows = Vec::with_capacity(raw_rows.len());
    for raw_row in raw_rows {
        let cells = raw_row.as_array().ok_or(AdapterError::MalformedResponse)?;
        if cells.len() != query.columns.len() {
            return Err(AdapterError::MalformedResponse);
        }
        let mut values = Vec::with_capacity(cells.len());
        for (cell, (name, contract)) in cells.iter().zip(query.columns.iter()) {
            let wire = WireValue::from_json(cell)?;
            values.push(IdValue::decode(wire, name, *contract)?);
        }
        rows.push(Row {
            columns: names.clone(),
            values,
        });
    }
    Ok(rows)
}

// ---------------------------------------------------------------------------
// base64 (standard alphabet, padded) — the blob wire encoding
// ---------------------------------------------------------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(*chunk.get(1).unwrap_or(&0));
        let b2 = u32::from(*chunk.get(2).unwrap_or(&0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut sextets = Vec::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' => sextets.push(byte - b'A'),
            b'a'..=b'z' => sextets.push(byte - b'a' + 26),
            b'0'..=b'9' => sextets.push(byte - b'0' + 52),
            b'+' => sextets.push(62),
            b'/' => sextets.push(63),
            b'=' => break,
            _ => return None,
        }
    }
    let mut out = Vec::with_capacity(sextets.len() * 3 / 4);
    for chunk in sextets.chunks(4) {
        if chunk.len() == 1 {
            return None;
        }
        let n = chunk.iter().fold(0u32, |acc, s| (acc << 6) | u32::from(*s));
        let n = n << (6 * (4 - chunk.len()));
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Tests. Every test names its oracle: the #5806 decision record item, or
// the libsql v2 pipeline wire format, that fixes the expected behaviour.
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex as StdMutex;

    use super::*;

    /// Oracle: #8504 - chunk sizes are byte counts, so framing is parsed
    /// on byte slices and UTF-8 is decoded only after the body is fully
    /// assembled: a multi-byte sequence split across chunks round-trips,
    /// and malformed framing returns a transport error, never a panic.
    #[test]
    fn test_chunked_body_decodes_utf8_split_across_chunks() {
        let mut raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        // U+00E9 is 0xC3 0xA9 in UTF-8; ship it as two one-byte chunks.
        raw.extend_from_slice(b"1\r\n\xc3\r\n1\r\n\xa9\r\n0\r\n\r\n");
        let response = parse_http_response(&raw).expect("split-UTF8 chunked body parses");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, "\u{e9}");
    }

    #[test]
    fn test_chunked_body_ordinary_chunks_round_trip() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n";
        let response = parse_http_response(raw).expect("ordinary chunked body parses");
        assert_eq!(response.body, "Wikipedia");
    }

    #[test]
    fn test_chunked_body_accepts_chunk_extensions_and_trailers() {
        // Oracle: RFC 9112 chunked coding; extensions decorate the size and
        // trailer fields follow the zero-sized last chunk.
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4 ; foo = bar ; quoted=\"hello world\"\r\nWiki\r\n0\r\nX-Checksum: yes\r\n\r\n";
        let response = parse_http_response(raw).expect("valid extensions and trailers parse");
        assert_eq!(response.body, "Wiki");
    }

    #[test]
    fn test_unsupported_transfer_codings_are_rejected() {
        for coding in ["notchunked", "gzip, chunked", "chunked, gzip", ""] {
            let raw = format!(
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: {coding}\r\n\r\n4\r\nWiki\r\n0\r\n\r\n"
            );
            assert!(
                parse_http_response(raw.as_bytes()).is_err(),
                "unsupported transfer coding must error: {coding:?}"
            );
        }
    }

    #[test]
    fn test_folded_header_fields_are_rejected() {
        // A folded continuation line must not let an unsupported coding
        // ride past the Transfer-Encoding check.
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n , gzip\r\n\r\n4\r\nWiki\r\n0\r\n\r\n";
        assert!(parse_http_response(raw).is_err());
        let raw = b"HTTP/1.1 200 OK\r\nX-Note: a\r\n b\r\n\r\nhello";
        assert!(parse_http_response(raw).is_err());
    }

    #[test]
    fn test_transfer_coding_whitespace_is_ows_only() {
        // Vertical tab is not HTTP optional whitespace: the field is
        // malformed and must be rejected, not trimmed into "chunked".
        let raw =
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: \u{b}chunked\r\n\r\n4\r\nWiki\r\n0\r\n\r\n";
        assert!(parse_http_response(raw.as_bytes()).is_err());
        // SP / HTAB around the coding remain fine.
        let raw =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding:\t chunked \t\r\n\r\n4\r\nWiki\r\n0\r\n\r\n";
        let response = parse_http_response(raw).expect("OWS-wrapped chunked parses");
        assert_eq!(response.body, "Wiki");
    }

    #[test]
    fn test_chunked_body_malformed_framing_errors_without_panic() {
        let head = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";
        for body in [
            &b"zz\r\nabc\r\n0\r\n\r\n"[..],
            b"4\r\nWi",
            b"4",
            b"4\r\nWiki0\r\n\r\n",
            b"0\r\n",
            b"0\r\nX-Checksum: yes\r\n",
            b"4;;bad\r\nWiki\r\n0\r\n\r\n",
            b"0\r\nnot-a-field\r\n\r\n",
            b"4 \r\nWiki\r\n0\r\n\r\n",
            b"4;foo=bar \r\nWiki\r\n0\r\n\r\n",
        ] {
            let mut raw = head.to_vec();
            raw.extend_from_slice(body);
            assert!(
                parse_http_response(&raw).is_err(),
                "malformed framing must error: {body:?}"
            );
        }
    }

    proptest::proptest! {
        /// Oracle: for every byte sequence, the chunk decoder either returns
        /// decoded bytes or a transport error; malformed wire data never panics.
        #[test]
        fn test_dechunk_never_panics_for_arbitrary_bytes(body in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..512)) {
            let _ = dechunk(&body);
        }
    }

    #[test]
    fn test_non_chunked_body_keeps_utf8_intact() {
        let mut raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\n".to_vec();
        raw.extend_from_slice("h\u{e9}llo".as_bytes());
        let response = parse_http_response(&raw).expect("plain body parses");
        assert_eq!(response.body, "h\u{e9}llo");
    }

    fn sample_query() -> GrantedQuery {
        GrantedQuery {
            table: "objects".to_owned(),
            columns: vec![
                ("id".to_owned(), ColumnType::Unsigned),
                ("digest".to_owned(), ColumnType::Blob),
                ("label".to_owned(), ColumnType::Text),
                ("visible".to_owned(), ColumnType::Bool),
            ],
            grant_sql: "grantee = ?".to_owned(),
            grant_args: vec![IdValue::Text("alice".to_owned())],
            filter_sql: Some("label = ?".to_owned()),
            filter_args: vec![IdValue::Text("report".to_owned())],
            limit: Some(10),
        }
    }

    fn test_config(url: &str) -> ForeignConfig {
        let mut aliases = BTreeMap::new();
        aliases.insert(
            "main".to_owned(),
            AliasConfig {
                url: url.to_owned(),
                credential_ref: "env:TEST_LIBSQL_TOKEN".to_owned(),
                statement_timeout: Duration::from_secs(5),
                max_blob_bytes: MAX_BLOB_BYTES,
            },
        );
        ForeignConfig { aliases }
    }

    /// Oracle: record item 5 — unsigned above i64::MAX is an 8-byte
    /// big-endian BLOB, never REAL.
    #[test]
    fn test_unsigned_above_i64_max_encodes_as_big_endian_blob() {
        let value = IdValue::Unsigned(u64::MAX);
        let wire = value.encode(MAX_BLOB_BYTES).unwrap();
        assert_eq!(wire, WireValue::Blob(u64::MAX.to_be_bytes().to_vec()));
        let back = IdValue::decode(wire, "id", ColumnType::Unsigned).unwrap();
        assert_eq!(back, value);

        let just_above = IdValue::Unsigned(i64::MAX as u64 + 1);
        assert!(matches!(
            just_above.encode(MAX_BLOB_BYTES).unwrap(),
            WireValue::Blob(_)
        ));
    }

    /// Oracle: record item 5 — unsigned at i64::MAX still fits INTEGER.
    #[test]
    fn test_unsigned_at_i64_max_encodes_as_integer() {
        let wire = IdValue::Unsigned(i64::MAX as u64)
            .encode(MAX_BLOB_BYTES)
            .unwrap();
        assert_eq!(wire, WireValue::Integer(i64::MAX));
    }

    /// Oracle: record item 5 — booleans are INTEGER 0/1, and a bool
    /// column rejects any other integer instead of coercing truthiness.
    #[test]
    fn test_bool_encodes_as_integer_and_decodes_strictly() {
        assert_eq!(
            IdValue::Bool(true).encode(MAX_BLOB_BYTES).unwrap(),
            WireValue::Integer(1)
        );
        assert_eq!(
            IdValue::Bool(false).encode(MAX_BLOB_BYTES).unwrap(),
            WireValue::Integer(0)
        );
        assert_eq!(
            IdValue::decode(WireValue::Integer(1), "visible", ColumnType::Bool).unwrap(),
            IdValue::Bool(true)
        );
        assert!(matches!(
            IdValue::decode(WireValue::Integer(2), "visible", ColumnType::Bool),
            Err(AdapterError::TypeMismatch { .. })
        ));
    }

    /// Oracle: record item 5 — NULL is absence, distinct from empty
    /// TEXT and empty BLOB in both directions.
    #[test]
    fn test_null_is_distinct_from_empty_text_and_empty_blob() {
        let null = IdValue::Null.encode(MAX_BLOB_BYTES).unwrap();
        let empty_text = IdValue::Text(String::new()).encode(MAX_BLOB_BYTES).unwrap();
        let empty_blob = IdValue::Blob(Vec::new()).encode(MAX_BLOB_BYTES).unwrap();
        assert_eq!(null, WireValue::Null);
        assert_ne!(null, empty_text);
        assert_ne!(null, empty_blob);
        assert_ne!(empty_text, empty_blob);
        assert_eq!(
            IdValue::decode(WireValue::Null, "label", ColumnType::Text).unwrap(),
            IdValue::Null
        );
        assert_eq!(
            IdValue::decode(WireValue::Text(String::new()), "label", ColumnType::Text).unwrap(),
            IdValue::Text(String::new())
        );
    }

    /// Oracle: record item 5 — TEXT rejects non-UTF-8 bytes rather than
    /// replacing them, and a BLOB in a TEXT column is a type error, not
    /// a coercion.
    #[test]
    fn test_non_utf8_and_blob_in_text_column_are_errors() {
        assert!(IdValue::text_from_bytes(vec![0xff, 0xfe]).is_err());
        assert!(matches!(
            IdValue::decode(WireValue::Blob(b"abc".to_vec()), "label", ColumnType::Text),
            Err(AdapterError::TypeMismatch { .. })
        ));
    }

    /// Oracle: record item 5 — REAL loses integer precision above 2^53,
    /// so a REAL in an integer-family column is a type error; an
    /// overflowing negative INTEGER in an unsigned column is too.
    #[test]
    fn test_real_in_integer_column_is_a_type_error() {
        let big = 9_007_199_254_740_993.0_f64; // 2^53 + 1, not representable
        assert!(matches!(
            IdValue::decode(WireValue::Real(big), "n", ColumnType::Integer),
            Err(AdapterError::TypeMismatch { .. })
        ));
        assert!(matches!(
            IdValue::decode(WireValue::Integer(-1), "id", ColumnType::Unsigned),
            Err(AdapterError::TypeMismatch { .. })
        ));
        assert!(matches!(
            IdValue::Real(f64::NAN).encode(MAX_BLOB_BYTES),
            Err(AdapterError::NonFiniteReal)
        ));
    }

    /// Oracle: record item 5 — oversize payloads are refused with a
    /// named size error, never truncated.
    #[test]
    fn test_oversize_blob_is_refused_not_truncated() {
        let big = IdValue::Blob(vec![0u8; 11]);
        assert!(matches!(
            big.encode(10),
            Err(AdapterError::SizeRefused { size: 11, max: 10 })
        ));
    }

    /// Oracle: record items 2 + 8 — write shapes are refused, and a
    /// query without a grant predicate is refused outright.
    #[test]
    fn test_write_shapes_and_missing_grant_are_refused() {
        let mut query = sample_query();
        query.grant_sql = "grantee = ?; DELETE FROM objects".to_owned();
        assert!(query.build_statement(MAX_BLOB_BYTES).is_err());

        let mut query = sample_query();
        query.filter_sql = Some("label = ? AND visible = ?".to_owned());
        query.filter_args = vec![IdValue::Text("report".to_owned())];
        // Placeholder mismatch is a refusal, not a silent fix-up.
        assert!(query.build_statement(MAX_BLOB_BYTES).is_err());

        let mut query = sample_query();
        query.grant_sql = "  ".to_owned();
        query.grant_args = vec![];
        assert!(matches!(
            query.build_statement(MAX_BLOB_BYTES),
            Err(AdapterError::QueryRefused(_))
        ));
    }

    /// Oracle: #13896 — a `replace()` call in a read predicate is a
    /// scalar function call, not a write statement: it is accepted and
    /// its arguments stay parameterized, while `REPLACE INTO ...`
    /// (write-shaped, no argument list) is still refused.
    #[test]
    fn test_replace_function_call_is_accepted_and_replace_into_is_refused() {
        let mut query = sample_query();
        query.filter_sql = Some("replace(label, '-', '') = ?".to_owned());
        query.filter_args = vec![IdValue::Text("report".to_owned())];
        let (sql, args) = query.build_statement(MAX_BLOB_BYTES).unwrap();
        assert_eq!(
            sql,
            "SELECT \"id\", \"digest\", \"label\", \"visible\" FROM \"objects\" \
             WHERE (grantee = ?) AND (replace(label, '-', '') = ?) LIMIT 10"
        );
        assert_eq!(
            args,
            vec![
                WireValue::Text("alice".to_owned()),
                WireValue::Text("report".to_owned()),
            ]
        );

        let mut query = sample_query();
        query.filter_sql = Some("REPLACE INTO objects (label) VALUES ('x')".to_owned());
        query.filter_args = vec![];
        assert!(matches!(
            query.build_statement(MAX_BLOB_BYTES),
            Err(AdapterError::WriteRefused)
        ));

        let mut query = sample_query();
        query.filter_sql = Some("label = replace".to_owned());
        query.filter_args = vec![];
        // A bare keyword with no call opener is not a function call.
        assert!(matches!(
            query.build_statement(MAX_BLOB_BYTES),
            Err(AdapterError::WriteRefused)
        ));
    }

    /// Oracle: record item 8 — the grant predicate is pushed into the
    /// remote query, first in the WHERE, ahead of the caller filter,
    /// with arguments in grant-then-filter order.
    #[test]
    fn test_grant_predicate_is_pushed_into_the_sql() {
        let (sql, args) = sample_query().build_statement(MAX_BLOB_BYTES).unwrap();
        assert_eq!(
            sql,
            "SELECT \"id\", \"digest\", \"label\", \"visible\" FROM \"objects\" \
             WHERE (grantee = ?) AND (label = ?) LIMIT 10"
        );
        assert_eq!(
            args,
            vec![
                WireValue::Text("alice".to_owned()),
                WireValue::Text("report".to_owned()),
            ]
        );
    }

    /// Oracle: record item 4 — only `id://foreign/libsql/<alias>` URIs
    /// parse, and only the alias comes out.
    #[test]
    fn test_parse_foreign_uri() {
        assert_eq!(
            parse_foreign_uri("id://foreign/libsql/main").unwrap(),
            "main"
        );
        assert!(parse_foreign_uri("id://foreign/redb/main").is_err());
        assert!(parse_foreign_uri("id://foreign/libsql/").is_err());
        assert!(parse_foreign_uri("id://foreign/libsql/a/b").is_err());
    }

    /// Scripted transport: replays queued outcomes, captures requests.
    #[derive(Debug, Default)]
    struct ScriptTransport {
        outcomes: StdMutex<VecDeque<Result<TransportResponse, TransportError>>>,
        captured: StdMutex<Vec<(String, String)>>,
    }

    impl ScriptTransport {
        fn push(&self, outcome: Result<TransportResponse, TransportError>) {
            self.outcomes.lock().unwrap().push_back(outcome);
        }
        fn attempts(&self) -> usize {
            self.captured.lock().unwrap().len()
        }
    }

    impl Transport for ScriptTransport {
        fn post_pipeline<'a>(
            &'a self,
            base_url: &'a str,
            token: &'a str,
            body: &'a str,
            _connect_timeout: Duration,
            _deadline: Duration,
        ) -> Pin<Box<dyn Future<Output = Result<TransportResponse, TransportError>> + Send + 'a>>
        {
            self.captured
                .lock()
                .unwrap()
                .push((base_url.to_owned(), body.to_owned()));
            let _ = token;
            let outcome = self.outcomes.lock().unwrap().pop_front();
            Box::pin(async move {
                outcome.unwrap_or_else(|| {
                    Ok(TransportResponse {
                        status: 500,
                        body: String::new(),
                    })
                })
            })
        }
    }

    #[derive(Debug)]
    struct StaticCredentials;

    impl CredentialResolver for StaticCredentials {
        fn resolve(&self, reference: &str) -> Result<String, AdapterError> {
            if reference == "env:TEST_LIBSQL_TOKEN" {
                Ok("test-token-value".to_owned())
            } else {
                Err(AdapterError::Auth {
                    credential_ref: reference.to_owned(),
                })
            }
        }
    }

    fn ok_pipeline_body(rows_json: &str) -> String {
        let rows: Json = serde_json::from_str(&format!("[{rows_json}]")).unwrap();
        json!({
            "baton": "b1",
            "base_url": null,
            "results": [
                {"type": "ok", "response": {"type": "execute", "result": {
                    "cols": [
                        {"name": "id", "decltype": null},
                        {"name": "digest", "decltype": null},
                        {"name": "label", "decltype": null},
                        {"name": "visible", "decltype": null}
                    ],
                    "rows": rows,
                    "affected_row_count": 0,
                    "last_insert_rowid": null
                }}},
                {"type": "ok", "response": {"type": "close"}}
            ]
        })
        .to_string()
    }

    fn scripted_adapter(
        transport: ScriptTransport,
    ) -> LibsqlAdapter<ScriptTransport, StaticCredentials> {
        let mut adapter = LibsqlAdapter::with_parts(
            test_config("https://example.invalid"),
            transport,
            StaticCredentials,
        );
        adapter.set_retry_backoff(Duration::ZERO);
        adapter
    }

    /// Oracle: the libsql v2 pipeline wire format + record item 5 —
    /// the request carries the grant-pushed SQL with wire-encoded args
    /// (integer as string, blob as base64), and the response rows are
    /// decoded under the column contract, including a u64 above
    /// i64::MAX arriving as an 8-byte blob.
    #[tokio::test]
    async fn test_pipeline_round_trip_decodes_under_contract() {
        let transport = ScriptTransport::default();
        let row = format!(
            "[{{\"type\":\"integer\",\"value\":\"7\"}},{{\"type\":\"blob\",\"value\":\"{}\"}},\
             {{\"type\":\"text\",\"value\":\"report\"}},{{\"type\":\"integer\",\"value\":\"1\"}}]",
            base64_encode(b"hash-bytes")
        );
        let big_row = format!(
            "[{{\"type\":\"blob\",\"value\":\"{}\"}},{{\"type\":\"blob\",\"value\":\"{}\"}},\
             {{\"type\":\"null\"}},{{\"type\":\"integer\",\"value\":\"0\"}}]",
            base64_encode(&u64::MAX.to_be_bytes()),
            base64_encode(b"")
        );
        transport.push(Ok(TransportResponse {
            status: 200,
            body: ok_pipeline_body(&format!("{row},{big_row}")),
        }));
        let adapter = scripted_adapter(transport);
        let rows = adapter.query("main", &sample_query()).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("id"), Some(&IdValue::Unsigned(7)));
        assert_eq!(
            rows[0].get("digest"),
            Some(&IdValue::Blob(b"hash-bytes".to_vec()))
        );
        assert_eq!(rows[0].get("visible"), Some(&IdValue::Bool(true)));
        assert_eq!(rows[1].get("id"), Some(&IdValue::Unsigned(u64::MAX)));
        assert_eq!(rows[1].get("label"), Some(&IdValue::Null));

        let captured = adapter.transport.captured.lock().unwrap();
        let sent: Json = serde_json::from_str(&captured[0].1).unwrap();
        assert_eq!(sent["requests"][0]["type"], json!("execute"));
        let stmt = &sent["requests"][0]["stmt"];
        assert!(
            stmt["sql"]
                .as_str()
                .unwrap()
                .contains("WHERE (grantee = ?) AND (label = ?)")
        );
        assert_eq!(stmt["args"][0], json!({"type": "text", "value": "alice"}));
    }

    /// Oracle: record item 8 — not-found and not-granted are the same
    /// result: an empty row set, no error, no distinguishing signal.
    #[tokio::test]
    async fn test_not_found_and_not_granted_are_indistinguishable() {
        for _ in 0..2 {
            let transport = ScriptTransport::default();
            transport.push(Ok(TransportResponse {
                status: 200,
                body: ok_pipeline_body(""),
            }));
            let adapter = scripted_adapter(transport);
            let rows = adapter.query("main", &sample_query()).await.unwrap();
            assert_eq!(rows, [] as [Row; 0]);
        }
    }

    /// Oracle: record item 7 — auth failure is fatal and never
    /// retried, and the error names the credential reference, never
    /// the token.
    #[tokio::test]
    async fn test_auth_failure_is_fatal_and_names_the_reference_only() {
        let transport = ScriptTransport::default();
        transport.push(Ok(TransportResponse {
            status: 401,
            body: "unauthorized".to_owned(),
        }));
        let adapter = scripted_adapter(transport);
        let error = adapter.query("main", &sample_query()).await.unwrap_err();
        assert_eq!(
            error,
            AdapterError::Auth {
                credential_ref: "env:TEST_LIBSQL_TOKEN".to_owned()
            }
        );
        assert!(!error.to_string().contains("test-token-value"));
        assert_eq!(adapter.transport.attempts(), 1);
    }

    /// Oracle: record item 7 — transient 5xx failures retry with
    /// bounded attempts for reads and then succeed.
    #[tokio::test]
    async fn test_transient_5xx_is_retried_then_succeeds() {
        let transport = ScriptTransport::default();
        transport.push(Ok(TransportResponse {
            status: 500,
            body: String::new(),
        }));
        transport.push(Ok(TransportResponse {
            status: 502,
            body: String::new(),
        }));
        transport.push(Ok(TransportResponse {
            status: 200,
            body: ok_pipeline_body(""),
        }));
        let adapter = scripted_adapter(transport);
        let rows = adapter.query("main", &sample_query()).await.unwrap();
        assert_eq!(rows, [] as [Row; 0]);
        assert_eq!(adapter.transport.attempts(), 3);
    }

    /// Oracle: record item 7 — malformed responses are deterministic:
    /// surfaced immediately, exactly one attempt.
    #[tokio::test]
    async fn test_malformed_response_is_deterministic() {
        let transport = ScriptTransport::default();
        transport.push(Ok(TransportResponse {
            status: 200,
            body: "not json".to_owned(),
        }));
        let adapter = scripted_adapter(transport);
        let error = adapter.query("main", &sample_query()).await.unwrap_err();
        assert_eq!(error, AdapterError::MalformedResponse);
        assert_eq!(adapter.transport.attempts(), 1);
    }

    /// A duplicated key in a response is malformed, not read as its last
    /// value (#8794).
    #[tokio::test]
    async fn test_a_response_with_a_duplicated_key_is_malformed() {
        let body = ok_pipeline_body("");
        let rest = body.strip_prefix('{').unwrap();
        let transport = ScriptTransport::default();
        transport.push(Ok(TransportResponse {
            status: 200,
            body: format!("{{\"baton\":\"b0\",{rest}"),
        }));
        let adapter = scripted_adapter(transport);
        let error = adapter.query("main", &sample_query()).await.unwrap_err();
        assert_eq!(error, AdapterError::MalformedResponse);
    }

    /// Oracle: record item 7 — the breaker opens at the failure
    /// threshold, refuses while open, grants only one half-open probe,
    /// reopens after a failed probe, and closes on a successful probe.
    #[test]
    fn test_circuit_breaker_opens_half_opens_and_closes() {
        let breaker = CircuitBreaker::new();
        let t0 = Instant::now();
        assert!(breaker.allow(t0));
        for _ in 0..BREAKER_FAILURE_THRESHOLD {
            breaker.record_transient_failure(t0);
        }
        assert!(!breaker.allow(t0));
        assert!(!breaker.allow(t0 + BREAKER_HALF_OPEN_AFTER - Duration::from_secs(1)));
        let probe_at = t0 + BREAKER_HALF_OPEN_AFTER;
        assert!(breaker.allow(probe_at));
        assert!(!breaker.allow(probe_at));
        breaker.record_transient_failure(probe_at);
        assert!(!breaker.allow(probe_at + BREAKER_HALF_OPEN_AFTER - Duration::from_secs(1)));
        let retry_probe_at = probe_at + BREAKER_HALF_OPEN_AFTER;
        assert!(breaker.allow(retry_probe_at));
        assert!(!breaker.allow(retry_probe_at));
        breaker.record_probe_failure(retry_probe_at);
        assert!(!breaker.allow(retry_probe_at + BREAKER_HALF_OPEN_AFTER - Duration::from_secs(1)));
        assert!(breaker.allow(retry_probe_at + BREAKER_HALF_OPEN_AFTER));
        breaker.record_success();
        assert!(breaker.allow(t0));
    }

    /// Oracle: PR #8848 review finding (Codex and CodeRabbit) — a
    /// half-open probe dropped before it records an outcome (a
    /// cancelled query future) must not strand the breaker in
    /// HalfOpen, which has no timed exit: the probe guard re-opens
    /// the breaker and the restarted cooldown admits a later probe.
    #[test]
    fn test_half_open_probe_guard_reopens_on_drop() {
        let breaker = CircuitBreaker::new();
        let t0 = Instant::now();
        for _ in 0..BREAKER_FAILURE_THRESHOLD {
            breaker.record_transient_failure(t0);
        }
        let probe_at = t0 + BREAKER_HALF_OPEN_AFTER;
        assert!(breaker.allow(probe_at));
        {
            let _guard = HalfOpenProbeGuard { breaker: &breaker };
            // Dropped here with no outcome recorded, exactly as a
            // cancelled query future drops it.
        }
        // The guard re-opened the breaker at drop time (about t0):
        // the restarted cooldown refuses a probe until a full delay
        // after the drop, then admits one.
        assert!(!breaker.allow(probe_at - Duration::from_millis(1)));
        assert!(breaker.allow(probe_at + BREAKER_HALF_OPEN_AFTER));
        breaker.record_success();
        // A guard dropped after an outcome was recorded is a no-op:
        // the closed breaker stays closed.
        drop(HalfOpenProbeGuard { breaker: &breaker });
        assert!(breaker.allow(t0));
    }

    /// Transport whose pipeline call never completes, so a test can
    /// cancel a query while it is awaiting the transport.
    #[derive(Debug, Default)]
    struct PendingTransport {
        attempts: StdMutex<usize>,
    }

    impl PendingTransport {
        fn attempts(&self) -> usize {
            *self.attempts.lock().unwrap()
        }
    }

    impl Transport for PendingTransport {
        fn post_pipeline<'a>(
            &'a self,
            base_url: &'a str,
            token: &'a str,
            body: &'a str,
            _connect_timeout: Duration,
            _deadline: Duration,
        ) -> Pin<Box<dyn Future<Output = Result<TransportResponse, TransportError>> + Send + 'a>>
        {
            *self.attempts.lock().unwrap() += 1;
            let _ = (base_url, token, body);
            Box::pin(std::future::pending())
        }
    }

    /// Oracle: PR #8848 review finding at the adapter level —
    /// cancelling the admitted half-open probe while it awaits the
    /// transport re-opens the breaker (restarted cooldown) instead of
    /// stranding it in HalfOpen forever.
    #[tokio::test]
    async fn test_cancelled_half_open_probe_reopens_breaker() {
        let adapter = LibsqlAdapter::with_parts(
            test_config("https://example.invalid"),
            PendingTransport::default(),
            StaticCredentials,
        );
        // Open the breaker with its cooldown already elapsed, so the
        // first query is admitted as the half-open probe.
        let breaker = adapter.breaker_for("main");
        let past = Instant::now() - BREAKER_HALF_OPEN_AFTER - Duration::from_secs(1);
        for _ in 0..BREAKER_FAILURE_THRESHOLD {
            breaker.record_transient_failure(past);
        }
        let adapter = Arc::new(adapter);
        let probe = {
            let adapter = Arc::clone(&adapter);
            tokio::spawn(async move { adapter.query("main", &sample_query()).await })
        };
        // Wait until the probe is inside the transport, then cancel
        // it the way a dropped caller future cancels it.
        for _ in 0..10_000 {
            if adapter.transport.attempts() > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(adapter.transport.attempts(), 1);
        probe.abort();
        let _ = probe.await;
        // The guard re-opened the breaker: Open with a restarted
        // cooldown, not HalfOpen with a stranded probe reservation.
        {
            let state = breaker.state.lock().unwrap();
            assert!(matches!(*state, BreakerState::Open { .. }));
        }
        let error = adapter.query("main", &sample_query()).await.unwrap_err();
        assert_eq!(error, AdapterError::CircuitOpen("main".to_owned()));
    }

    /// Oracle: record acceptance — the adapter queries a remote libsql
    /// database over live HTTP. A local TCP listener plays the sqld
    /// endpoint; the full stack (HttpTransport → pipeline → contract
    /// decode) runs against it, and the listener verifies the request
    /// shape: POST /v2/pipeline with a bearer header and the
    /// grant-pushed SQL.
    #[tokio::test]
    async fn test_live_local_http_round_trip() {
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let body = ok_pipeline_body(
            "[{\"type\":\"integer\",\"value\":\"3\"},{\"type\":\"blob\",\"value\":\"aGk=\"},\
             {\"type\":\"text\",\"value\":\"report\"},{\"type\":\"integer\",\"value\":\"1\"}]",
        );
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 16384];
            let read = socket.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..read]).into_owned();
            assert!(request.starts_with("POST /v2/pipeline HTTP/1.1"));
            assert!(request.contains("Authorization: Bearer test-token-value"));
            assert!(request.contains("grantee = ?"));
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            request
        });

        let mut config = test_config(&format!("http://{addr}"));
        config.aliases.get_mut("main").unwrap().credential_ref = "env:TEST_LIBSQL_TOKEN".to_owned();
        // SAFETY: test-only variable written once in this test; no other
        // test reads or writes TEST_LIBSQL_TOKEN concurrently for a
        // different value (the scripted tests use a static resolver).
        unsafe {
            std::env::set_var("TEST_LIBSQL_TOKEN", "test-token-value");
        }
        let adapter = LibsqlAdapter::new(config);
        let rows = adapter
            .query_uri("id://foreign/libsql/main", &sample_query())
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("id"), Some(&IdValue::Unsigned(3)));
        assert_eq!(rows[0].get("digest"), Some(&IdValue::Blob(b"hi".to_vec())));
        server.await.unwrap();
    }

    fn public_scope() -> PublicationScope {
        PublicationScope::new([PublishedCollection {
            name: "objects".to_owned(),
            mode: PublicServingMode::Live,
            replica_alias: None,
        }])
        .unwrap()
    }

    /// Oracle: #14784 done check — a published collection is served
    /// live through the adapter's read path, grant predicate pushed
    /// into the single SELECT as on the private path.
    #[tokio::test]
    async fn test_public_read_serves_published_collection_live() {
        let transport = ScriptTransport::default();
        let row = format!(
            "[{{\"type\":\"integer\",\"value\":\"7\"}},{{\"type\":\"blob\",\"value\":\"{}\"}},\
             {{\"type\":\"text\",\"value\":\"report\"}},{{\"type\":\"integer\",\"value\":\"1\"}}]",
            base64_encode(b"hash")
        );
        transport.push(Ok(TransportResponse {
            status: 200,
            body: ok_pipeline_body(&row),
        }));
        let adapter = scripted_adapter(transport);
        let rows = adapter
            .query_public("main", &public_scope(), &sample_query())
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("id"), Some(&IdValue::Unsigned(7)));
        assert_eq!(
            rows[0].get("label"),
            Some(&IdValue::Text("report".to_owned()))
        );
    }

    /// Oracle: #14784 done check — a query for an unpublished table is
    /// refused before any fetch, not fetched wide and filtered after
    /// the fact.
    #[tokio::test]
    async fn test_public_read_refuses_unpublished_collection_before_any_fetch() {
        let transport = ScriptTransport::default();
        let adapter = scripted_adapter(transport);
        let mut query = sample_query();
        query.table = "secrets".to_owned();
        let error = adapter
            .query_public("main", &public_scope(), &query)
            .await
            .unwrap_err();
        assert_eq!(error, PublicReadError::Refused);
        assert_eq!(adapter.transport.attempts(), 0);
    }

    /// Oracle: #14784 done check — reaching beyond the named
    /// collection through a subquery or join shape in a predicate
    /// fragment is refused before any fetch, even when the subquery
    /// names only the published table (the adapter cannot scope an
    /// arbitrary subquery, so the shape is refused, per D4).
    #[tokio::test]
    async fn test_public_read_refuses_fragment_reach_through() {
        for fragment in [
            "id IN (SELECT id FROM secrets)",
            "id IN (SELECT id FROM objects)",
            "visible = 1 JOIN secrets ON secrets.id = objects.id",
        ] {
            let transport = ScriptTransport::default();
            let adapter = scripted_adapter(transport);
            let mut query = sample_query();
            query.filter_sql = Some(fragment.to_owned());
            query.filter_args = Vec::new();
            let error = adapter
                .query_public("main", &public_scope(), &query)
                .await
                .unwrap_err();
            assert_eq!(error, PublicReadError::Refused, "fragment: {fragment}");
            assert_eq!(adapter.transport.attempts(), 0, "fragment: {fragment}");
        }
        // The grant fragment is held to the same rule.
        let transport = ScriptTransport::default();
        let adapter = scripted_adapter(transport);
        let mut query = sample_query();
        query.grant_sql = "grantee IN (SELECT name FROM secrets)".to_owned();
        query.grant_args = Vec::new();
        let error = adapter
            .query_public("main", &public_scope(), &query)
            .await
            .unwrap_err();
        assert_eq!(error, PublicReadError::Refused);
        assert_eq!(adapter.transport.attempts(), 0);
    }

    /// Oracle: #14784 / D4 — a write keyword in a fragment reaches the
    /// adapter's own refusal during statement construction and maps to
    /// the public `Refused` outcome; nothing is executed.
    #[tokio::test]
    async fn test_public_read_maps_adapter_refusals_to_refused() {
        let transport = ScriptTransport::default();
        let adapter = scripted_adapter(transport);
        let mut query = sample_query();
        query.filter_sql = Some("label = ? OR DELETE".to_owned());
        query.filter_args = vec![IdValue::Text("report".to_owned())];
        let error = adapter
            .query_public("main", &public_scope(), &query)
            .await
            .unwrap_err();
        assert_eq!(error, PublicReadError::Refused);
        assert_eq!(adapter.transport.attempts(), 0);
    }

    /// Oracle: #14784 done check — a failed live read is the
    /// `Unavailable` outcome; no stale rows are served as current.
    #[tokio::test]
    async fn test_public_read_failed_live_read_is_unavailable() {
        // No scripted outcomes: the scripted transport answers 500,
        // which the adapter retries and then exhausts as transient.
        let transport = ScriptTransport::default();
        let adapter = scripted_adapter(transport);
        let error = adapter
            .query_public("main", &public_scope(), &sample_query())
            .await
            .unwrap_err();
        assert_eq!(error, PublicReadError::Unavailable);
        assert!(adapter.transport.attempts() > 0);
    }

    /// Oracle: #14784 / D4 — a collection the owner declared
    /// replica-served is read from the replica store's alias, not from
    /// the live source.
    #[tokio::test]
    async fn test_public_read_replica_mode_reads_the_replica_store() {
        let mut aliases = BTreeMap::new();
        for (alias, url) in [
            ("main", "https://live.example.invalid"),
            ("replica", "https://replica.example.invalid"),
        ] {
            aliases.insert(
                alias.to_owned(),
                AliasConfig {
                    url: url.to_owned(),
                    credential_ref: "env:TEST_LIBSQL_TOKEN".to_owned(),
                    statement_timeout: Duration::from_secs(5),
                    max_blob_bytes: MAX_BLOB_BYTES,
                },
            );
        }
        let mut adapter = LibsqlAdapter::with_parts(
            ForeignConfig { aliases },
            ScriptTransport::default(),
            StaticCredentials,
        );
        adapter.set_retry_backoff(Duration::ZERO);
        let row = format!(
            "[{{\"type\":\"integer\",\"value\":\"7\"}},{{\"type\":\"blob\",\"value\":\"{}\"}},\
             {{\"type\":\"text\",\"value\":\"report\"}},{{\"type\":\"integer\",\"value\":\"1\"}}]",
            base64_encode(b"hash")
        );
        adapter.transport.push(Ok(TransportResponse {
            status: 200,
            body: ok_pipeline_body(&row),
        }));
        let scope = PublicationScope::new([PublishedCollection {
            name: "objects".to_owned(),
            mode: PublicServingMode::MaterializedReplica,
            replica_alias: Some("replica".to_owned()),
        }])
        .unwrap();
        let rows = adapter
            .query_public("main", &scope, &sample_query())
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let captured = adapter.transport.captured.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].0, "https://replica.example.invalid");
    }

    /// Oracle: D4 + invariant 8 — a replica-served collection whose
    /// replica store is not configured fails loudly as `Unavailable`;
    /// it is never silently served from the live source instead.
    #[tokio::test]
    async fn test_public_read_missing_replica_store_is_unavailable_not_live() {
        let transport = ScriptTransport::default();
        // Script a success: if the read fell through to the live alias
        // it would serve rows; the outcome must be Unavailable instead.
        transport.push(Ok(TransportResponse {
            status: 200,
            body: ok_pipeline_body(""),
        }));
        let adapter = scripted_adapter(transport);
        let scope = PublicationScope::new([PublishedCollection {
            name: "objects".to_owned(),
            mode: PublicServingMode::MaterializedReplica,
            replica_alias: Some("replica".to_owned()),
        }])
        .unwrap();
        let error = adapter
            .query_public("main", &scope, &sample_query())
            .await
            .unwrap_err();
        assert_eq!(error, PublicReadError::Unavailable);
        assert_eq!(adapter.transport.attempts(), 0);
    }

    /// Oracle: #14784 done check / invariant 5 — the public outcome
    /// text names no credential, credential reference, token, or URL,
    /// even when the underlying failure is an auth failure against a
    /// configured alias.
    #[tokio::test]
    async fn test_public_read_outcomes_name_no_credential_or_locator() {
        let transport = ScriptTransport::default();
        transport.push(Ok(TransportResponse {
            status: 401,
            body: "unauthorized".to_owned(),
        }));
        let adapter = scripted_adapter(transport);
        let error = adapter
            .query_public("main", &public_scope(), &sample_query())
            .await
            .unwrap_err();
        assert_eq!(error, PublicReadError::Unavailable);
        let text = error.to_string();
        assert!(
            !text.contains("TEST_LIBSQL_TOKEN"),
            "outcome leaked a credential reference: {text}"
        );
        assert!(
            !text.contains("test-token-value"),
            "outcome leaked a token: {text}"
        );
        assert!(
            !text.contains("example.invalid"),
            "outcome leaked a locator: {text}"
        );
    }

    /// One scripted served row: values 7 / "hash" / "report" / 1,
    /// charged as 8 + 4 + 6 + 1 = 19 serving bytes.
    fn served_row_json() -> String {
        format!(
            "[{{\"type\":\"integer\",\"value\":\"7\"}},{{\"type\":\"blob\",\"value\":\"{}\"}},\
             {{\"type\":\"text\",\"value\":\"report\"}},{{\"type\":\"integer\",\"value\":\"1\"}}]",
            base64_encode(b"hash")
        )
    }

    fn push_served_rows(transport: &ScriptTransport, rows: usize) {
        let row = served_row_json();
        let body = ok_pipeline_body(&vec![row; rows].join(", "));
        transport.push(Ok(TransportResponse { status: 200, body }));
    }

    fn generous_owner_limits() -> OwnerAggregateLimits {
        OwnerAggregateLimits {
            max_objects_per_window: 1_000_000,
            max_bytes_per_window: 1_000_000_000,
            window_ms: 60_000,
            max_concurrent_fetches: 16,
        }
    }

    fn read_context<'a>(
        owner_tuned_limits: Option<&'a ShareServingLimits>,
        now_ms: u64,
    ) -> PublicReadContext<'a> {
        PublicReadContext {
            owner: "owner-1",
            share: "share-1",
            reader: "reader-1",
            owner_tuned_limits,
            now_ms,
        }
    }

    /// Oracle: the #5800 chunk-resume contract is the ordered partition of
    /// the snapshot cut: each page reports the next index iff objects remain,
    /// and concatenating pages serves every object exactly once.
    #[test]
    fn test_public_snapshot_page_reports_cap_and_resume_position() {
        let serving = PublicServing::new(
            generous_owner_limits(),
            ShareServingLimits::v1_defaults(),
        );
        let limits = ShareServingLimits {
            max_objects_per_fetch: 2,
            max_bytes_per_fetch: 1_000,
            max_requests_per_minute: 60,
        };
        let context = read_context(Some(&limits), 0);
        let sizes = [10, 20, 30];

        let first = serving
            .serve_snapshot_page(&sizes, 0, &context)
            .expect("first snapshot page is admitted");
        assert_eq!(first.objects, 2);
        assert_eq!(first.capped_by, Some(FetchCap::Objects));
        assert_eq!(first.resume_from, Some(2));

        let second = serving
            .serve_snapshot_page(&sizes, first.resume_from.unwrap(), &context)
            .expect("resumed snapshot page is admitted");
        assert_eq!(second.objects, 1);
        assert_eq!(second.capped_by, None);
        assert_eq!(second.resume_from, None);
    }

    /// Oracle: #14783 wiring — a public read inside every limit is
    /// served through the gate unchanged.
    #[tokio::test]
    async fn test_served_public_read_within_limits_serves_rows() {
        let transport = ScriptTransport::default();
        push_served_rows(&transport, 1);
        let adapter = scripted_adapter(transport);
        let serving =
            PublicServing::new(generous_owner_limits(), ShareServingLimits::v1_defaults());
        let rows = serving
            .query_public(
                &adapter,
                "main",
                &public_scope(),
                &sample_query(),
                &read_context(None, 0),
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("id"), Some(&IdValue::Unsigned(7)));
    }

    /// Oracle: #14783 done check at the wired boundary — the
    /// (share, reader) rate cap refuses with the gate's stable
    /// named outcome once the reader's window is spent.
    #[tokio::test]
    async fn test_served_public_read_rate_cap_refuses_with_named_outcome() {
        let tuned = ShareServingLimits {
            max_objects_per_fetch: 10_000,
            max_bytes_per_fetch: 100 * 1024 * 1024,
            max_requests_per_minute: 1,
        };
        let transport = ScriptTransport::default();
        push_served_rows(&transport, 1);
        push_served_rows(&transport, 1);
        let adapter = scripted_adapter(transport);
        let serving =
            PublicServing::new(generous_owner_limits(), ShareServingLimits::v1_defaults());
        serving
            .query_public(
                &adapter,
                "main",
                &public_scope(),
                &sample_query(),
                &read_context(Some(&tuned), 0),
            )
            .await
            .unwrap();
        let error = serving
            .query_public(
                &adapter,
                "main",
                &public_scope(),
                &sample_query(),
                &read_context(Some(&tuned), 1_000),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            PublicServingError::Limit(ServingRefusal::RateLimited { .. })
        ));
        assert_eq!(error.name(), "rate_limited");
    }

    /// Oracle: #14783 done check at the wired boundary — a fetch
    /// whose rows cross the per-fetch object cap serves nothing and
    /// names the cap; it is never silently truncated to the cap.
    #[tokio::test]
    async fn test_served_public_read_over_object_cap_serves_nothing() {
        let tuned = ShareServingLimits {
            max_objects_per_fetch: 1,
            max_bytes_per_fetch: 100 * 1024 * 1024,
            max_requests_per_minute: 60,
        };
        let transport = ScriptTransport::default();
        push_served_rows(&transport, 2);
        let adapter = scripted_adapter(transport);
        let serving =
            PublicServing::new(generous_owner_limits(), ShareServingLimits::v1_defaults());
        let error = serving
            .query_public(
                &adapter,
                "main",
                &public_scope(),
                &sample_query(),
                &read_context(Some(&tuned), 0),
            )
            .await
            .unwrap_err();
        assert_eq!(error, PublicServingError::Capped(FetchCap::Objects));
        assert_eq!(error.name(), "fetch_capped_objects");
    }

    /// Oracle: #14783 done check at the wired boundary — the
    /// per-fetch byte cap is charged on the rows' payload bytes
    /// (one served row is 19 bytes; a second identical row crosses
    /// a 19-byte cap).
    #[tokio::test]
    async fn test_served_public_read_over_byte_cap_serves_nothing() {
        let tuned = ShareServingLimits {
            max_objects_per_fetch: 10_000,
            max_bytes_per_fetch: 19,
            max_requests_per_minute: 60,
        };
        let transport = ScriptTransport::default();
        push_served_rows(&transport, 2);
        let adapter = scripted_adapter(transport);
        let serving =
            PublicServing::new(generous_owner_limits(), ShareServingLimits::v1_defaults());
        let error = serving
            .query_public(
                &adapter,
                "main",
                &public_scope(),
                &sample_query(),
                &read_context(Some(&tuned), 0),
            )
            .await
            .unwrap_err();
        assert_eq!(error, PublicServingError::Capped(FetchCap::Bytes));
        assert_eq!(error.name(), "fetch_capped_bytes");
    }

    /// Oracle: #14783 wiring — the owner's aggregate budget is
    /// charged across reads: a budget of one row refuses the second
    /// read with the aggregate outcome, applying nothing.
    #[tokio::test]
    async fn test_served_public_read_owner_aggregate_budget_binds_across_reads() {
        let owner_limits = OwnerAggregateLimits {
            max_objects_per_window: 1,
            max_bytes_per_window: 1_000_000_000,
            window_ms: 60_000,
            max_concurrent_fetches: 16,
        };
        let transport = ScriptTransport::default();
        push_served_rows(&transport, 1);
        push_served_rows(&transport, 1);
        let adapter = scripted_adapter(transport);
        let serving = PublicServing::new(owner_limits, ShareServingLimits::v1_defaults());
        serving
            .query_public(
                &adapter,
                "main",
                &public_scope(),
                &sample_query(),
                &read_context(None, 0),
            )
            .await
            .unwrap();
        let error = serving
            .query_public(
                &adapter,
                "main",
                &public_scope(),
                &sample_query(),
                &read_context(None, 1_000),
            )
            .await
            .unwrap_err();
        assert_eq!(
            error,
            PublicServingError::Limit(ServingRefusal::OwnerAggregateExceeded)
        );
        assert_eq!(error.name(), "owner_aggregate_exceeded");
    }

    /// Oracle: #14783 wiring — the adapter's own outcomes pass
    /// through the serving layer unchanged in kind: an unpublished
    /// collection is refused before any fetch, limits or not.
    #[tokio::test]
    async fn test_served_public_read_unpublished_collection_is_refused() {
        let transport = ScriptTransport::default();
        let adapter = scripted_adapter(transport);
        let serving =
            PublicServing::new(generous_owner_limits(), ShareServingLimits::v1_defaults());
        let mut query = sample_query();
        query.table = "secrets".to_owned();
        let error = serving
            .query_public(
                &adapter,
                "main",
                &public_scope(),
                &query,
                &read_context(None, 0),
            )
            .await
            .unwrap_err();
        assert_eq!(error, PublicServingError::Refused);
        assert_eq!(adapter.transport.attempts(), 0);
    }

    /// Oracle: invariant 5 at the wired boundary — the serving
    /// layer's outcomes name no credential, credential reference,
    /// or locator either.
    #[tokio::test]
    async fn test_served_public_read_outcomes_name_no_credential_or_locator() {
        let transport = ScriptTransport::default();
        transport.push(Ok(TransportResponse {
            status: 401,
            body: "unauthorized".to_owned(),
        }));
        let adapter = scripted_adapter(transport);
        let serving =
            PublicServing::new(generous_owner_limits(), ShareServingLimits::v1_defaults());
        let error = serving
            .query_public(
                &adapter,
                "main",
                &public_scope(),
                &sample_query(),
                &read_context(None, 0),
            )
            .await
            .unwrap_err();
        assert_eq!(error, PublicServingError::Unavailable);
        let text = error.to_string();
        assert!(
            !text.contains("TEST_LIBSQL_TOKEN"),
            "leaked a credential reference: {text}"
        );
        assert!(!text.contains("test-token-value"), "leaked a token: {text}");
        assert!(
            !text.contains("example.invalid"),
            "leaked a locator: {text}"
        );
    }

    /// Oracle: D4 + invariant 8 — a malformed published set is a
    /// config error at construction: replica-served without a replica
    /// alias, live-served with one, a duplicate name, or a name that
    /// is not a bare identifier.
    #[test]
    fn test_publication_scope_rejects_malformed_sets() {
        let live = |name: &str| PublishedCollection {
            name: name.to_owned(),
            mode: PublicServingMode::Live,
            replica_alias: None,
        };
        assert!(matches!(
            PublicationScope::new([PublishedCollection {
                name: "objects".to_owned(),
                mode: PublicServingMode::MaterializedReplica,
                replica_alias: None,
            }]),
            Err(AdapterError::Config(_))
        ));
        assert!(matches!(
            PublicationScope::new([PublishedCollection {
                name: "objects".to_owned(),
                mode: PublicServingMode::Live,
                replica_alias: Some("replica".to_owned()),
            }]),
            Err(AdapterError::Config(_))
        ));
        assert!(matches!(
            PublicationScope::new([live("objects"), live("objects")]),
            Err(AdapterError::Config(_))
        ));
        assert!(matches!(
            PublicationScope::new([live("not a table")]),
            Err(AdapterError::Config(_))
        ));
        let scope = PublicationScope::new([live("objects")]).unwrap();
        assert!(scope.is_published("objects"));
        assert!(!scope.is_published("secrets"));
        assert_eq!(scope.collection("objects"), Some(&live("objects")));
    }

    /// Build a real publication record through the real publish
    /// action (#14781), over the named collection units.
    fn published_record(collections: &[&str]) -> PublicationRecord {
        use id_grants::{
            ClassLevel, DiscoveryMode, Duration as GrantDuration, GrantSet, Publications,
            PublishRequest, ShareId,
        };
        let mut grants = GrantSet::new();
        let mut book = Publications::new("owner");
        let request = PublishRequest {
            scope: collections
                .iter()
                .map(|name| (Scope::Collection((*name).to_owned()), ClassLevel(0)))
                .collect::<BTreeMap<_, _>>(),
            subscribe: false,
            duration: GrantDuration::Standing,
            discovery: DiscoveryMode::Listed,
            ceiling: ClassLevel(1),
            owner_ceiling: ClassLevel(2),
            title: "Published tables".to_owned(),
            description: "Tables the owner published".to_owned(),
            now: 1_800_000_000,
        };
        book.publish(&mut grants, ShareId::mint([7; 32]), request)
            .unwrap()
            .clone()
    }

    fn live(name: &str) -> PublishedCollection {
        PublishedCollection {
            name: name.to_owned(),
            mode: PublicServingMode::Live,
            replica_alias: None,
        }
    }

    /// Oracle: #14784 checkpoint — the scope built from the real
    /// publication record names exactly the record's collections,
    /// each in its declared mode; a table created later (absent from
    /// the record) is not published.
    #[test]
    fn test_publication_scope_from_record_matches_the_record() {
        let record = published_record(&["objects", "events"]);
        let scope = PublicationScope::from_record(
            &record,
            [
                live("objects"),
                PublishedCollection {
                    name: "events".to_owned(),
                    mode: PublicServingMode::MaterializedReplica,
                    replica_alias: Some("events-replica".to_owned()),
                },
            ],
        )
        .unwrap();
        assert!(scope.is_published("objects"));
        assert!(scope.is_published("events"));
        assert_eq!(
            scope.collection("events").map(|c| c.mode),
            Some(PublicServingMode::MaterializedReplica)
        );
        assert!(!scope.is_published("created-later"));
    }

    /// Oracle: #14784 checkpoint — a scope built from the real
    /// record serves through the public read path like a hand-built
    /// one (the record's live collection answers; the record's set
    /// is the gate).
    #[tokio::test]
    async fn test_public_read_serves_scope_built_from_record() {
        let record = published_record(&["objects"]);
        let scope = PublicationScope::from_record(&record, [live("objects")]).unwrap();
        let transport = ScriptTransport::default();
        let row = format!(
            "[{{\"type\":\"integer\",\"value\":\"7\"}},{{\"type\":\"blob\",\"value\":\"{}\"}},\
             {{\"type\":\"text\",\"value\":\"report\"}},{{\"type\":\"integer\",\"value\":\"1\"}}]",
            base64_encode(b"hash")
        );
        transport.push(Ok(TransportResponse {
            status: 200,
            body: ok_pipeline_body(&row),
        }));
        let adapter = scripted_adapter(transport);
        let rows = adapter
            .query_public("main", &scope, &sample_query())
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
    }

    /// Oracle: #14784 checkpoint / invariant 8 — building from the
    /// record refuses every disagreement loudly: a published
    /// collection with no serving declaration, a declaration naming
    /// a collection the record does not publish, an object unit in
    /// the record's scope, and a retired record.
    #[test]
    fn test_publication_scope_from_record_refuses_disagreement() {
        let record = published_record(&["objects", "events"]);
        // A record collection with no serving declaration.
        assert!(matches!(
            PublicationScope::from_record(&record, [live("objects")]),
            Err(AdapterError::Config(_))
        ));
        // A declaration wider than the record.
        assert!(matches!(
            PublicationScope::from_record(
                &record,
                [live("objects"), live("events"), live("secrets")]
            ),
            Err(AdapterError::Config(_))
        ));
        // An object unit in the record's scope: not servable on the
        // collection read path, refused rather than dropped.
        let mut object_record = published_record(&["objects"]);
        object_record
            .scope
            .insert(Scope::Object("note-1".to_owned()));
        assert!(matches!(
            PublicationScope::from_record(&object_record, [live("objects")]),
            Err(AdapterError::Config(_))
        ));
        // A retired record builds no scope.
        let mut retired = published_record(&["objects"]);
        retired.unpublished_at = Some(1_800_000_100);
        assert!(matches!(
            PublicationScope::from_record(&retired, [live("objects")]),
            Err(AdapterError::Config(_))
        ));
    }
}
