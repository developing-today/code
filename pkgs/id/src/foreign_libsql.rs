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
//! - A slow or dead foreign store never blocks native ID operations:
//!   calls are isolated behind per-alias circuit breakers with bounded
//!   retries for reads only; auth failure is fatal and never retried.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
fn validate_fragment(fragment: &str, arg_count: usize) -> Result<(), AdapterError> {
    if fragment.contains(';') || fragment.contains("--") || fragment.contains("/*") {
        return Err(AdapterError::QueryRefused(
            "predicate fragment is not a single expression".to_owned(),
        ));
    }
    for word in fragment.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
        if WRITE_KEYWORDS.contains(&word.to_ascii_uppercase().as_str()) {
            return Err(AdapterError::WriteRefused);
        }
    }
    if fragment.matches('?').count() != arg_count {
        return Err(AdapterError::QueryRefused(
            "predicate placeholder count does not match argument count".to_owned(),
        ));
    }
    Ok(())
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
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.eq_ignore_ascii_case("transfer-encoding") {
            continue;
        }
        transfer_encoding_seen = true;
        for coding in value.split(',') {
            if !coding.trim().eq_ignore_ascii_case("chunked") || chunked {
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
            BreakerState::Closed { .. } | BreakerState::HalfOpen => true,
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
            // The token is resolved per attempt and never stored.
            let token = self.credentials.resolve(&alias_config.credential_ref)?;
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
                    return Err(AdapterError::Auth {
                        credential_ref: alias_config.credential_ref.clone(),
                    });
                }
                Err(error) => return Err(error),
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
    let parsed: Json = serde_json::from_str(body).map_err(|_| AdapterError::MalformedResponse)?;
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
            assert!(rows.is_empty());
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
        assert!(rows.is_empty());
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

    /// Oracle: record item 7 — the breaker opens at the failure
    /// threshold, refuses while open, half-opens after the timer, and
    /// closes on a successful probe.
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
        assert!(breaker.allow(t0 + BREAKER_HALF_OPEN_AFTER));
        breaker.record_success();
        assert!(breaker.allow(t0));
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
}
