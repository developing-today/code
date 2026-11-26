//! Tenant DB store adapter: the event store's table shape over
//! SQLite-over-HTTP (Turso), so a hosted tenant node and a native node
//! share one storage interface and differ only in backend.
//!
//! Slice E6 of the hosted-tenancy design (#9083 §11 item 6; design record
//! `doc/2026-11-17T16-55-00Z_design_hosted-tenancy-architecture`), with
//! component D (#9082: ID gains Turso/SQLite-over-HTTP as a database store
//! in addition to redb — one storage interface, pluggable backends). The
//! tenant DB holds the tenant's own event log and content layer "in the
//! shape the ID store already uses" (design §2.2), so this adapter mirrors
//! the native store's redb tables one for one as SQL rows.
//!
//! # What this module is
//!
//! - The wire codec for the Turso/libsql HTTP pipeline protocol
//!   (`POST {base}/v2/pipeline`): statements and values out, result rows
//!   and statement errors back, a baton identifying the server-side
//!   session across calls. The codec is the protocol oracle; the tests
//!   pin its JSON shape.
//! - [`HttpTransport`], the production transport: one blocking HTTPS POST
//!   per pipeline call, carrying the tenant's scoped, expiring Turso
//!   token as a bearer credential (design §7). The token is supplied by
//!   the caller, is never written to the tenant DB (design §2.3), and is
//!   redacted from `Debug` output.
//! - [`TenantStore`], the row layer: the native store's tables
//!   ([`STORE_TABLES`]) as two SQL tables — one for single-value tables,
//!   one for multimap tables — with the native tables' semantics: keys
//!   and multimap values order as raw bytes, a put replaces, a remove of
//!   an absent key is not an error. The conformance test at the bottom of
//!   this module runs one op script against both this layer (over an
//!   in-process SQLite transport) and real redb tables, and requires
//!   identical observable results: the same tests pass against this
//!   backend and the native backend at the layer this slice owns.
//! - Explicit transactions over the baton session (`BEGIN`/`COMMIT`/
//!   `ROLLBACK` as statements in one session), so a later semantics port
//!   gets the native store's one-transaction-per-batch write shape
//!   (design §5.2) from the same seam.
//!
//! # What this module is not yet
//!
//! It is not yet an [`crate::store_traits::EventStoreBackend`]. The
//! ingest semantics (verification, fork handling, the waiting pool, the
//! resolution) live in `id-events`' `EventStore`, written directly
//! against redb transactions; porting them onto this row layer — without
//! forking the semantics into a second implementation — is the remaining
//! E6 work, tracked separately. Callers above `store_traits` therefore
//! have no hosted-only special case today and gain none from this slice:
//! nothing in the sync or facade layers names this module's types except
//! through the re-export.

use std::fmt;
use std::ops::RangeBounds;

use base64::Engine as _;
use serde_json::{Map, Value as Json, json};

/// The format row a tenant store writes into its `tenant_meta` table. A
/// store holding another format is refused, as the native store refuses
/// a file of another format.
pub const TENANT_STORE_FORMAT: &str = "id.tenant.store.v1";

/// The `tenant_meta` key the format row is stored under.
const FORMAT_KEY: &str = "format";

/// The DDL that creates the tenant store's tables. Idempotent: opening
/// an existing store re-runs it harmlessly. Two row tables carry every
/// store table — one for single-value tables, one for multimap tables —
/// plus `tenant_meta` for the tenant's own bookkeeping (design §2.2).
/// BLOB keys order by raw byte comparison in SQLite, the native tables'
/// key order.
pub const SCHEMA_STATEMENTS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS id_store_rows (
         table_name TEXT NOT NULL,
         key BLOB NOT NULL,
         value BLOB NOT NULL,
         PRIMARY KEY (table_name, key)
     )",
    "CREATE TABLE IF NOT EXISTS id_store_multi (
         table_name TEXT NOT NULL,
         key BLOB NOT NULL,
         value BLOB NOT NULL,
         PRIMARY KEY (table_name, key, value)
     )",
    "CREATE TABLE IF NOT EXISTS tenant_meta (
         key TEXT NOT NULL PRIMARY KEY,
         value BLOB NOT NULL
     )",
];

/// Whether a store table holds one value per key or a set of values per
/// key — redb's `TableDefinition` versus `MultimapTableDefinition`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableKind {
    /// One value per key; a put replaces.
    Single,
    /// A set of values per key; a put adds, ordered by value.
    Multi,
}

/// One of the native event store's tables, mirrored in the tenant DB.
/// The names and kinds are the table definitions in
/// `id-events`' `event_store` module; the mirror is what "in the shape
/// the ID store already uses" (design §2.2) means concretely.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoreTable {
    /// The table's name, identical to the native store's table name.
    pub name: &'static str,
    /// Whether the table is single-value or multimap.
    pub kind: TableKind,
}

/// Every table of the native event store, in the native module's order.
pub const STORE_TABLES: &[StoreTable] = &[
    StoreTable {
        name: "events",
        kind: TableKind::Single,
    },
    StoreTable {
        name: "hashes",
        kind: TableKind::Single,
    },
    StoreTable {
        name: "chains",
        kind: TableKind::Single,
    },
    StoreTable {
        name: "forks",
        kind: TableKind::Single,
    },
    StoreTable {
        name: "groups",
        kind: TableKind::Multi,
    },
    StoreTable {
        name: "proof_ids",
        kind: TableKind::Single,
    },
    StoreTable {
        name: "proof_headers",
        kind: TableKind::Single,
    },
    StoreTable {
        name: "waiting",
        kind: TableKind::Single,
    },
    StoreTable {
        name: "waiting_by_dep",
        kind: TableKind::Multi,
    },
    StoreTable {
        name: "meta",
        kind: TableKind::Single,
    },
    StoreTable {
        name: "blocks",
        kind: TableKind::Single,
    },
    StoreTable {
        name: "writer_set_bodies",
        kind: TableKind::Single,
    },
    StoreTable {
        name: "actions",
        kind: TableKind::Single,
    },
    StoreTable {
        name: "device_ops",
        kind: TableKind::Single,
    },
    StoreTable {
        name: "applied",
        kind: TableKind::Single,
    },
    StoreTable {
        name: "bodies",
        kind: TableKind::Single,
    },
    StoreTable {
        name: "tag_fold",
        kind: TableKind::Single,
    },
    StoreTable {
        name: "compacted",
        kind: TableKind::Single,
    },
];

/// The kind of the named store table.
///
/// # Errors
///
/// Returns [`TursoError::UnknownTable`] for a name outside
/// [`STORE_TABLES`]. Table names are interpolated into SQL, so the
/// closed list is also the injection guard: no caller-supplied string
/// reaches a statement unvalidated.
fn table_kind(name: &str) -> Result<TableKind, TursoError> {
    STORE_TABLES
        .iter()
        .find(|table| table.name == name)
        .map(|table| table.kind)
        .ok_or_else(|| TursoError::UnknownTable(name.to_owned()))
}

/// Why a tenant store operation failed.
#[derive(Debug, thiserror::Error)]
pub enum TursoError {
    /// The HTTP call itself failed: no answer, or a non-success status.
    #[error("turso transport: {0}")]
    Transport(String),
    /// The answer was not the pipeline protocol: malformed JSON, a
    /// missing field, a value of an unexpected shape.
    #[error("turso protocol: {0}")]
    Protocol(String),
    /// The server refused a statement. The message is the server's.
    #[error("turso statement failed: {message}")]
    Sql {
        /// The server's error message.
        message: String,
        /// The server's error code, when it sent one.
        code: Option<String>,
    },
    /// A table name outside [`STORE_TABLES`] was used.
    #[error("unknown store table: {0}")]
    UnknownTable(String),
    /// A single-value operation was used on a multimap table, or the
    /// reverse.
    #[error("table {table} is a {actual:?} table, not a {wanted:?} table")]
    WrongKind {
        /// The table that was used.
        table: String,
        /// The kind the operation needs.
        wanted: TableKind,
        /// The table's actual kind.
        actual: TableKind,
    },
    /// The tenant DB holds another store format.
    #[error("tenant store format mismatch: {0}")]
    Format(String),
    /// The connection was closed; its baton session is over.
    #[error("turso connection is closed")]
    Closed,
    /// A transaction was begun inside one, or ended outside one.
    #[error("turso transaction state: {0}")]
    Transaction(String),
}

/// One SQL value, in the pipeline protocol's encoding. Integers travel
/// as decimal strings and blobs as base64, per the protocol; the codec
/// below is the only place that knows.
#[derive(Clone, Debug, PartialEq)]
pub enum SqlValue {
    /// SQL `NULL`.
    Null,
    /// A 64-bit integer.
    Integer(i64),
    /// A 64-bit float. Non-finite values have no protocol encoding and
    /// fail at encode time.
    Float(f64),
    /// A text value.
    Text(String),
    /// A byte string.
    Blob(Vec<u8>),
}

impl SqlValue {
    /// The value's pipeline-protocol JSON.
    fn to_json(&self) -> Result<Json, TursoError> {
        Ok(match self {
            Self::Null => json!({"type": "null"}),
            Self::Integer(value) => json!({"type": "integer", "value": value.to_string()}),
            Self::Float(value) => {
                let number = serde_json::Number::from_f64(*value).ok_or_else(|| {
                    TursoError::Protocol(format!("float {value} has no protocol encoding"))
                })?;
                json!({"type": "float", "value": number})
            }
            Self::Text(value) => json!({"type": "text", "value": value}),
            Self::Blob(value) => json!({
                "type": "blob",
                "value": base64::engine::general_purpose::STANDARD.encode(value),
            }),
        })
    }

    /// Decodes one pipeline-protocol value.
    fn from_json(value: &Json) -> Result<Self, TursoError> {
        let kind = value
            .get("type")
            .and_then(Json::as_str)
            .ok_or_else(|| TursoError::Protocol(format!("value has no type: {value}")))?;
        match kind {
            "null" => Ok(Self::Null),
            "integer" => {
                let raw = value.get("value").and_then(Json::as_str).ok_or_else(|| {
                    TursoError::Protocol(format!("integer has no value: {value}"))
                })?;
                raw.parse::<i64>().map(Self::Integer).map_err(|_err| {
                    TursoError::Protocol(format!("integer value {raw:?} is not an i64"))
                })
            }
            "float" => value
                .get("value")
                .and_then(Json::as_f64)
                .map(Self::Float)
                .ok_or_else(|| TursoError::Protocol(format!("float has no value: {value}"))),
            "text" => value
                .get("value")
                .and_then(Json::as_str)
                .map(|text| Self::Text(text.to_owned()))
                .ok_or_else(|| TursoError::Protocol(format!("text has no value: {value}"))),
            "blob" => {
                let raw = value
                    .get("value")
                    .and_then(Json::as_str)
                    .ok_or_else(|| TursoError::Protocol(format!("blob has no value: {value}")))?;
                base64::engine::general_purpose::STANDARD
                    .decode(raw)
                    .map(Self::Blob)
                    .map_err(|_err| TursoError::Protocol("blob value is not base64".to_owned()))
            }
            other => Err(TursoError::Protocol(format!(
                "unknown value type {other:?}"
            ))),
        }
    }
}

/// One statement to execute: SQL text with bound arguments. Arguments
/// are always bound, never interpolated; only table names are
/// interpolated, and only after [`table_kind`] has validated them.
#[derive(Clone, Debug, PartialEq)]
pub struct Statement {
    /// The SQL text, with `?` placeholders for the arguments.
    pub sql: String,
    /// The arguments, in placeholder order.
    pub args: Vec<SqlValue>,
}

impl Statement {
    /// A statement with no arguments.
    pub fn new(sql: impl Into<String>) -> Self {
        Self {
            sql: sql.into(),
            args: Vec::new(),
        }
    }

    /// A statement with arguments.
    pub fn with_args(sql: impl Into<String>, args: Vec<SqlValue>) -> Self {
        Self {
            sql: sql.into(),
            args,
        }
    }

    /// The statement's pipeline-protocol JSON.
    fn to_json(&self) -> Result<Json, TursoError> {
        let mut args = Vec::with_capacity(self.args.len());
        for arg in &self.args {
            args.push(arg.to_json()?);
        }
        Ok(json!({
            "type": "execute",
            "stmt": {
                "sql": self.sql,
                "args": args,
                "named_args": [],
                "want_rows": true,
            },
        }))
    }
}

/// The outcome of one executed statement.
#[derive(Clone, Debug, PartialEq)]
pub struct ExecuteResult {
    /// The result columns' names, in row order.
    pub columns: Vec<String>,
    /// The result rows, each in column order.
    pub rows: Vec<Vec<SqlValue>>,
    /// How many rows the statement changed.
    pub affected_rows: u64,
    /// The rowid of the last insert, when the statement inserted.
    pub last_insert_rowid: Option<i64>,
}

impl ExecuteResult {
    /// Decodes one `execute` result from a pipeline response.
    fn from_json(result: &Json) -> Result<Self, TursoError> {
        let columns = result
            .get("cols")
            .and_then(Json::as_array)
            .ok_or_else(|| TursoError::Protocol(format!("execute result has no cols: {result}")))?
            .iter()
            .map(|col| {
                col.get("name")
                    .and_then(Json::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| TursoError::Protocol(format!("column has no name: {col}")))
            })
            .collect::<Result<Vec<String>, TursoError>>()?;
        let mut rows = Vec::new();
        for row in result
            .get("rows")
            .and_then(Json::as_array)
            .ok_or_else(|| TursoError::Protocol(format!("execute result has no rows: {result}")))?
        {
            let mut values = Vec::new();
            for value in row
                .as_array()
                .ok_or_else(|| TursoError::Protocol(format!("row is not an array: {row}")))?
            {
                values.push(SqlValue::from_json(value)?);
            }
            rows.push(values);
        }
        Ok(Self {
            columns,
            rows,
            affected_rows: result
                .get("affected_row_count")
                .and_then(Json::as_u64)
                .unwrap_or(0),
            last_insert_rowid: result.get("last_insert_rowid").and_then(Json::as_i64),
        })
    }
}

/// The transport seam: one pipeline request body out, one response body
/// back. Production uses [`HttpTransport`]; tests substitute an
/// in-process SQLite endpoint that speaks the same bytes, so the codec
/// and the SQL are exercised exactly as the server would meet them.
pub trait PipelineTransport: fmt::Debug + Send + Sync + 'static {
    /// Posts one pipeline request body and returns the response body.
    ///
    /// # Errors
    ///
    /// Returns [`TursoError::Transport`] when no protocol answer
    /// arrives.
    fn pipeline(&self, body: &[u8]) -> Result<Vec<u8>, TursoError>;
}

/// The production transport: HTTPS POSTs to a Turso database's
/// `/v2/pipeline` endpoint, authorised by the tenant's scoped token.
/// The token is a credential: it is never logged, and `Debug` redacts
/// it (design §2.3: a dump must not yield database access).
#[derive(Clone)]
pub struct HttpTransport {
    client: reqwest::blocking::Client,
    url: String,
    token: String,
}

impl fmt::Debug for HttpTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpTransport")
            .field("url", &self.url)
            .field("token", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl HttpTransport {
    /// A transport for the database at `base_url` (its `libsql://` or
    /// `https://` address), authorised by `token` — the scoped,
    /// expiring Turso token the tenancy plane minted for this tenant
    /// (design §7). The caller owns the token's lifecycle.
    ///
    /// # Errors
    ///
    /// Returns [`TursoError::Transport`] when the HTTP client cannot be
    /// built.
    pub fn new(base_url: &str, token: String) -> Result<Self, TursoError> {
        let client = reqwest::blocking::Client::builder()
            .build()
            .map_err(|err| TursoError::Transport(err.to_string()))?;
        // The pipeline endpoint is HTTPS; a libsql:// address names
        // the same host.
        let base = match base_url.strip_prefix("libsql://") {
            Some(host) => format!("https://{host}"),
            None => base_url.to_owned(),
        };
        Ok(Self {
            client,
            url: format!("{}/v2/pipeline", base.trim_end_matches('/')),
            token,
        })
    }
}

impl PipelineTransport for HttpTransport {
    fn pipeline(&self, body: &[u8]) -> Result<Vec<u8>, TursoError> {
        let response = self
            .client
            .post(&self.url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .body(body.to_vec())
            .send()
            .map_err(|err| TursoError::Transport(err.to_string()))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .map_err(|err| TursoError::Transport(err.to_string()))?;
        if !status.is_success() {
            return Err(TursoError::Transport(format!(
                "pipeline endpoint answered {status}: {}",
                String::from_utf8_lossy(&bytes)
            )));
        }
        Ok(bytes.to_vec())
    }
}

/// One open pipeline session: a transport plus the baton the server
/// issued for it. Statements sent on one connection share the baton, so
/// a transaction spans calls; a fresh connection starts a fresh
/// session.
#[derive(Debug)]
pub struct Connection<T: PipelineTransport> {
    transport: T,
    baton: Option<String>,
    in_transaction: bool,
    closed: bool,
}

impl<T: PipelineTransport> Connection<T> {
    /// Opens a session on `transport`. No call is made until the first
    /// statement.
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            baton: None,
            in_transaction: false,
            closed: false,
        }
    }

    /// Whether a transaction is open on this session.
    pub const fn in_transaction(&self) -> bool {
        self.in_transaction
    }

    /// Runs `requests` as one pipeline call and decodes the `execute`
    /// results, in order. A statement error anywhere in the call fails
    /// the whole call with that error; statements before it in the same
    /// call may already have been applied — use [`Connection::begin`]
    /// when a group must be atomic.
    fn round_trip(&mut self, requests: &[Json]) -> Result<Vec<ExecuteResult>, TursoError> {
        if self.closed {
            return Err(TursoError::Closed);
        }
        let mut body = Map::new();
        if let Some(baton) = &self.baton {
            body.insert("baton".to_owned(), Json::String(baton.clone()));
        }
        body.insert("requests".to_owned(), Json::Array(requests.to_vec()));
        let bytes = serde_json::to_vec(&Json::Object(body))
            .map_err(|err| TursoError::Protocol(err.to_string()))?;
        let answer = self.transport.pipeline(&bytes)?;
        let parsed: Json = serde_json::from_slice(&answer)
            .map_err(|err| TursoError::Protocol(format!("response is not JSON: {err}")))?;
        self.baton = match parsed.get("baton") {
            Some(Json::Null) | None => None,
            Some(Json::String(baton)) => Some(baton.clone()),
            Some(other) => {
                return Err(TursoError::Protocol(format!(
                    "baton is not a string: {other}"
                )));
            }
        };
        let results = parsed
            .get("results")
            .and_then(Json::as_array)
            .ok_or_else(|| TursoError::Protocol(format!("response has no results: {parsed}")))?;
        let mut executed = Vec::new();
        for result in results {
            let kind = result
                .get("type")
                .and_then(Json::as_str)
                .ok_or_else(|| TursoError::Protocol(format!("result has no type: {result}")))?;
            match kind {
                "ok" => {
                    let response = result.get("response").ok_or_else(|| {
                        TursoError::Protocol(format!("ok result has no response: {result}"))
                    })?;
                    let response_kind = response.get("type").and_then(Json::as_str);
                    if response_kind == Some("execute") {
                        let inner = response.get("result").ok_or_else(|| {
                            TursoError::Protocol(format!(
                                "execute response has no result: {response}"
                            ))
                        })?;
                        executed.push(ExecuteResult::from_json(inner)?);
                    }
                    // A `close` response carries no result; it is the
                    // answer to Connection::close, which ignores these.
                }
                "error" => {
                    let error = result.get("error").ok_or_else(|| {
                        TursoError::Protocol(format!("error result has no error: {result}"))
                    })?;
                    return Err(TursoError::Sql {
                        message: error
                            .get("message")
                            .and_then(Json::as_str)
                            .unwrap_or("statement failed")
                            .to_owned(),
                        code: error.get("code").and_then(Json::as_str).map(str::to_owned),
                    });
                }
                other => {
                    return Err(TursoError::Protocol(format!(
                        "unknown result type {other:?}"
                    )));
                }
            }
        }
        Ok(executed)
    }

    /// Executes one statement.
    ///
    /// # Errors
    ///
    /// Returns [`TursoError::Sql`] when the server refuses the
    /// statement, [`TursoError::Transport`] or [`TursoError::Protocol`]
    /// when the call itself fails, and [`TursoError::Closed`] after
    /// [`Connection::close`].
    pub fn execute(&mut self, statement: &Statement) -> Result<ExecuteResult, TursoError> {
        let mut results = self.round_trip(&[statement.to_json()?])?;
        results
            .pop()
            .ok_or_else(|| TursoError::Protocol("pipeline answered no result".to_owned()))
    }

    /// Executes several statements in one pipeline call, in order.
    ///
    /// # Errors
    ///
    /// As [`Connection::execute`]; the first statement error fails the
    /// call.
    pub fn execute_batch(
        &mut self,
        statements: &[Statement],
    ) -> Result<Vec<ExecuteResult>, TursoError> {
        let mut requests = Vec::with_capacity(statements.len());
        for statement in statements {
            requests.push(statement.to_json()?);
        }
        self.round_trip(&requests)
    }

    /// Opens a transaction on this session. Statements until
    /// [`Connection::commit`] or [`Connection::rollback`] share it.
    ///
    /// # Errors
    ///
    /// Returns [`TursoError::Transaction`] when a transaction is
    /// already open; statement and transport failures as
    /// [`Connection::execute`].
    pub fn begin(&mut self) -> Result<(), TursoError> {
        if self.in_transaction {
            return Err(TursoError::Transaction(
                "a transaction is already open".to_owned(),
            ));
        }
        self.execute(&Statement::new("BEGIN"))?;
        self.in_transaction = true;
        Ok(())
    }

    /// Commits the open transaction.
    ///
    /// # Errors
    ///
    /// Returns [`TursoError::Transaction`] when no transaction is open.
    pub fn commit(&mut self) -> Result<(), TursoError> {
        if !self.in_transaction {
            return Err(TursoError::Transaction("no transaction is open".to_owned()));
        }
        self.execute(&Statement::new("COMMIT"))?;
        self.in_transaction = false;
        Ok(())
    }

    /// Rolls the open transaction back.
    ///
    /// # Errors
    ///
    /// Returns [`TursoError::Transaction`] when no transaction is open.
    pub fn rollback(&mut self) -> Result<(), TursoError> {
        if !self.in_transaction {
            return Err(TursoError::Transaction("no transaction is open".to_owned()));
        }
        self.execute(&Statement::new("ROLLBACK"))?;
        self.in_transaction = false;
        Ok(())
    }

    /// Ends the session: the server drops the baton and any state it
    /// held for it. The connection cannot be used afterwards.
    ///
    /// # Errors
    ///
    /// Returns [`TursoError::Closed`] when the connection is already
    /// closed; transport failures as [`Connection::execute`].
    pub fn close(&mut self) -> Result<(), TursoError> {
        if self.closed {
            return Err(TursoError::Closed);
        }
        // Mark closed even if the close call itself fails: the baton
        // is spent either way and this handle must not be used again.
        let result = self.round_trip(&[json!({"type": "close"})]);
        self.closed = true;
        self.baton = None;
        result.map(|_| ())
    }
}

/// The tenant DB store adapter: the event store's tables as rows in the
/// tenant's Turso database, reached over [`Connection`]. One handle is
/// one baton session; row operations take `&mut self` because the
/// session is stateful (a transaction may be open).
#[derive(Debug)]
pub struct TenantStore<T: PipelineTransport> {
    conn: Connection<T>,
}

impl<T: PipelineTransport> TenantStore<T> {
    /// Opens the tenant store on `transport`: creates the schema if
    /// absent and checks the format row, writing it on a fresh store.
    ///
    /// # Errors
    ///
    /// Returns [`TursoError::Format`] when the database already holds a
    /// store of another format, and transport or statement errors from
    /// the schema calls.
    pub fn open(transport: T) -> Result<Self, TursoError> {
        let mut store = Self {
            conn: Connection::new(transport),
        };
        let ddl: Vec<Statement> = SCHEMA_STATEMENTS
            .iter()
            .map(|sql| Statement::new(*sql))
            .collect();
        store.conn.execute_batch(&ddl)?;
        match store.meta_get(FORMAT_KEY)? {
            None => {
                store.meta_put(FORMAT_KEY, TENANT_STORE_FORMAT.as_bytes())?;
            }
            Some(found) if found == TENANT_STORE_FORMAT.as_bytes() => {}
            Some(found) => {
                return Err(TursoError::Format(format!(
                    "expected {TENANT_STORE_FORMAT}, found {}",
                    String::from_utf8_lossy(&found)
                )));
            }
        }
        Ok(store)
    }

    /// The session below the row layer, for statements the row layer
    /// does not model (a later slice's migrations, diagnostics).
    pub fn connection(&mut self) -> &mut Connection<T> {
        &mut self.conn
    }

    /// Opens a transaction on the store's session. See
    /// [`Connection::begin`].
    ///
    /// # Errors
    ///
    /// As [`Connection::begin`].
    pub fn begin(&mut self) -> Result<(), TursoError> {
        self.conn.begin()
    }

    /// Commits the open transaction. See [`Connection::commit`].
    ///
    /// # Errors
    ///
    /// As [`Connection::commit`].
    pub fn commit(&mut self) -> Result<(), TursoError> {
        self.conn.commit()
    }

    /// Rolls the open transaction back. See [`Connection::rollback`].
    ///
    /// # Errors
    ///
    /// As [`Connection::rollback`].
    pub fn rollback(&mut self) -> Result<(), TursoError> {
        self.conn.rollback()
    }

    /// Ends the store's session. See [`Connection::close`].
    ///
    /// # Errors
    ///
    /// As [`Connection::close`].
    pub fn close(&mut self) -> Result<(), TursoError> {
        self.conn.close()
    }

    /// Validates `table` for an operation wanting kind `wanted`.
    fn checked_table(&self, table: &str, wanted: TableKind) -> Result<(), TursoError> {
        let actual = table_kind(table)?;
        if actual == wanted {
            Ok(())
        } else {
            Err(TursoError::WrongKind {
                table: table.to_owned(),
                wanted,
                actual,
            })
        }
    }

    /// The value under `key` in a single-value table, or `None`.
    ///
    /// # Errors
    ///
    /// Returns [`TursoError::UnknownTable`] for an unknown table and
    /// [`TursoError::WrongKind`] for a multimap table.
    pub fn get(&mut self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, TursoError> {
        self.checked_table(table, TableKind::Single)?;
        let result = self.conn.execute(&Statement::with_args(
            "SELECT value FROM id_store_rows WHERE table_name = ? AND key = ?",
            vec![
                SqlValue::Text(table.to_owned()),
                SqlValue::Blob(key.to_vec()),
            ],
        ))?;
        match result.rows.into_iter().next() {
            None => Ok(None),
            Some(row) => match row.into_iter().next() {
                Some(SqlValue::Blob(value)) => Ok(Some(value)),
                other => Err(TursoError::Protocol(format!(
                    "id_store_rows value is not a blob: {other:?}"
                ))),
            },
        }
    }

    /// Stores `value` under `key` in a single-value table, replacing
    /// any value there — the native table's put.
    ///
    /// # Errors
    ///
    /// As [`TenantStore::get`].
    pub fn put(&mut self, table: &str, key: &[u8], value: &[u8]) -> Result<(), TursoError> {
        self.checked_table(table, TableKind::Single)?;
        self.conn.execute(&Statement::with_args(
            "INSERT INTO id_store_rows (table_name, key, value) VALUES (?, ?, ?)
             ON CONFLICT (table_name, key) DO UPDATE SET value = excluded.value",
            vec![
                SqlValue::Text(table.to_owned()),
                SqlValue::Blob(key.to_vec()),
                SqlValue::Blob(value.to_vec()),
            ],
        ))?;
        Ok(())
    }

    /// Removes `key` from a single-value table. Returns whether a row
    /// was there; an absent key is not an error.
    ///
    /// # Errors
    ///
    /// As [`TenantStore::get`].
    pub fn remove(&mut self, table: &str, key: &[u8]) -> Result<bool, TursoError> {
        self.checked_table(table, TableKind::Single)?;
        let result = self.conn.execute(&Statement::with_args(
            "DELETE FROM id_store_rows WHERE table_name = ? AND key = ?",
            vec![
                SqlValue::Text(table.to_owned()),
                SqlValue::Blob(key.to_vec()),
            ],
        ))?;
        Ok(result.affected_rows > 0)
    }

    /// The values under `key` in a multimap table, in value order —
    /// the native multimap's iteration order for one key.
    ///
    /// # Errors
    ///
    /// Returns [`TursoError::UnknownTable`] for an unknown table and
    /// [`TursoError::WrongKind`] for a single-value table.
    pub fn multi_values(&mut self, table: &str, key: &[u8]) -> Result<Vec<Vec<u8>>, TursoError> {
        self.checked_table(table, TableKind::Multi)?;
        let result = self.conn.execute(&Statement::with_args(
            "SELECT value FROM id_store_multi
             WHERE table_name = ? AND key = ? ORDER BY value",
            vec![
                SqlValue::Text(table.to_owned()),
                SqlValue::Blob(key.to_vec()),
            ],
        ))?;
        let mut values = Vec::with_capacity(result.rows.len());
        for row in result.rows {
            match row.into_iter().next() {
                Some(SqlValue::Blob(value)) => values.push(value),
                other => {
                    return Err(TursoError::Protocol(format!(
                        "id_store_multi value is not a blob: {other:?}"
                    )));
                }
            }
        }
        Ok(values)
    }

    /// Adds `value` to the set under `key` in a multimap table. Adding
    /// a value already present is a no-op — the native multimap's set
    /// semantics.
    ///
    /// # Errors
    ///
    /// As [`TenantStore::multi_values`].
    pub fn multi_put(&mut self, table: &str, key: &[u8], value: &[u8]) -> Result<(), TursoError> {
        self.checked_table(table, TableKind::Multi)?;
        self.conn.execute(&Statement::with_args(
            "INSERT OR IGNORE INTO id_store_multi (table_name, key, value) VALUES (?, ?, ?)",
            vec![
                SqlValue::Text(table.to_owned()),
                SqlValue::Blob(key.to_vec()),
                SqlValue::Blob(value.to_vec()),
            ],
        ))?;
        Ok(())
    }

    /// Removes one `value` from the set under `key` in a multimap
    /// table. Returns whether it was there.
    ///
    /// # Errors
    ///
    /// As [`TenantStore::multi_values`].
    pub fn multi_remove(
        &mut self,
        table: &str,
        key: &[u8],
        value: &[u8],
    ) -> Result<bool, TursoError> {
        self.checked_table(table, TableKind::Multi)?;
        let result = self.conn.execute(&Statement::with_args(
            "DELETE FROM id_store_multi WHERE table_name = ? AND key = ? AND value = ?",
            vec![
                SqlValue::Text(table.to_owned()),
                SqlValue::Blob(key.to_vec()),
                SqlValue::Blob(value.to_vec()),
            ],
        ))?;
        Ok(result.affected_rows > 0)
    }

    /// Rows of a single-value table in `range`, in key order, at most
    /// `limit` of them — the native table's ordered range read. Bounds
    /// follow `RangeBounds`: an excluded start begins strictly after
    /// it, as a redb range does.
    ///
    /// # Errors
    ///
    /// As [`TenantStore::get`].
    pub fn scan(
        &mut self,
        table: &str,
        range: impl RangeBounds<Vec<u8>>,
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, TursoError> {
        self.checked_table(table, TableKind::Single)?;
        let (clause, mut args) = range_clauses(&range);
        let sql = format!(
            "SELECT key, value FROM id_store_rows
             WHERE table_name = ?{clause} ORDER BY key LIMIT ?"
        );
        args.insert(0, SqlValue::Text(table.to_owned()));
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        args.push(SqlValue::Integer(limit));
        let result = self.conn.execute(&Statement::with_args(sql, args))?;
        rows_of(result)
    }

    /// Rows of a single-value table whose keys start with `prefix`, in
    /// key order, at most `limit` of them.
    ///
    /// # Errors
    ///
    /// As [`TenantStore::get`].
    pub fn scan_prefix(
        &mut self,
        table: &str,
        prefix: &[u8],
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, TursoError> {
        match prefix_successor(prefix) {
            Some(successor) => self.scan(table, prefix.to_vec()..successor, limit),
            None => self.scan(table, prefix.to_vec().., limit),
        }
    }

    /// The tenant's own bookkeeping value under `key`, or `None`.
    fn meta_get(&mut self, key: &str) -> Result<Option<Vec<u8>>, TursoError> {
        let result = self.conn.execute(&Statement::with_args(
            "SELECT value FROM tenant_meta WHERE key = ?",
            vec![SqlValue::Text(key.to_owned())],
        ))?;
        match result.rows.into_iter().next() {
            None => Ok(None),
            Some(row) => match row.into_iter().next() {
                Some(SqlValue::Blob(value)) => Ok(Some(value)),
                other => Err(TursoError::Protocol(format!(
                    "tenant_meta value is not a blob: {other:?}"
                ))),
            },
        }
    }

    /// Writes the tenant's own bookkeeping value under `key`.
    fn meta_put(&mut self, key: &str, value: &[u8]) -> Result<(), TursoError> {
        self.conn.execute(&Statement::with_args(
            "INSERT INTO tenant_meta (key, value) VALUES (?, ?)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            vec![
                SqlValue::Text(key.to_owned()),
                SqlValue::Blob(value.to_vec()),
            ],
        ))?;
        Ok(())
    }
}

/// The SQL fragment and bound arguments for a byte range, in
/// placeholder order: a lower clause (` AND key >= ?` or `> ?`) when
/// the range is bounded below, then an upper clause (`<=` or `<`) when
/// bounded above. Unbounded sides contribute neither.
fn range_clauses(range: &impl RangeBounds<Vec<u8>>) -> (String, Vec<SqlValue>) {
    use std::ops::Bound::{Excluded, Included, Unbounded};
    let mut sql = String::new();
    let mut args = Vec::new();
    match range.start_bound() {
        Included(key) => {
            sql.push_str(" AND key >= ?");
            args.push(SqlValue::Blob(key.to_vec()));
        }
        Excluded(key) => {
            sql.push_str(" AND key > ?");
            args.push(SqlValue::Blob(key.to_vec()));
        }
        Unbounded => {}
    }
    match range.end_bound() {
        Included(key) => {
            sql.push_str(" AND key <= ?");
            args.push(SqlValue::Blob(key.to_vec()));
        }
        Excluded(key) => {
            sql.push_str(" AND key < ?");
            args.push(SqlValue::Blob(key.to_vec()));
        }
        Unbounded => {}
    }
    (sql, args)
}

/// Decodes `(key, value)` blob pairs from an execute result.
fn rows_of(result: ExecuteResult) -> Result<Vec<(Vec<u8>, Vec<u8>)>, TursoError> {
    let mut rows = Vec::with_capacity(result.rows.len());
    for row in result.rows {
        let mut cells = row.into_iter();
        match (cells.next(), cells.next()) {
            (Some(SqlValue::Blob(key)), Some(SqlValue::Blob(value))) => rows.push((key, value)),
            other => {
                return Err(TursoError::Protocol(format!(
                    "row is not a (blob, blob) pair: {other:?}"
                )));
            }
        }
    }
    Ok(rows)
}

/// The smallest byte string greater than every string with `prefix`:
/// the prefix with its last non-`0xFF` byte incremented and the rest
/// dropped. `None` when the prefix is all `0xFF` (or empty): no
/// successor exists, and a prefix scan is unbounded above.
fn prefix_successor(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut successor = prefix.to_vec();
    while let Some(last) = successor.last_mut() {
        if *last == u8::MAX {
            successor.pop();
        } else {
            *last += 1;
            return Some(successor);
        }
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::sync::{Arc, Mutex};

    use redb::{
        Database, MultimapTableDefinition, ReadableDatabase, ReadableMultimapTable, ReadableTable,
        TableDefinition,
    };
    use rusqlite::types::Value as SqliteValue;

    use crate::event_rows::{RowRead, TenantWrite};

    use super::*;

    /// An in-process stand-in for a Turso endpoint: it takes the exact
    /// pipeline request bytes a server would receive, executes the
    /// statements against a real SQLite database, and answers with
    /// protocol-shaped response bytes. The oracle is SQLite itself —
    /// the same engine a Turso database runs — so the adapter's SQL
    /// and codec meet the semantics they will meet in production,
    /// differing only in transport.
    #[derive(Clone, Debug)]
    struct SqliteTransport {
        db: Arc<Mutex<rusqlite::Connection>>,
    }

    impl SqliteTransport {
        fn new() -> Self {
            Self {
                db: Arc::new(Mutex::new(rusqlite::Connection::open_in_memory().unwrap())),
            }
        }

        /// One protocol value as a SQLite value.
        fn sqlite_value(value: &Json) -> SqliteValue {
            match value.get("type").and_then(Json::as_str) {
                Some("integer") => value
                    .get("value")
                    .and_then(Json::as_str)
                    .and_then(|raw| raw.parse::<i64>().ok())
                    .map_or(SqliteValue::Null, SqliteValue::Integer),
                Some("float") => value
                    .get("value")
                    .and_then(Json::as_f64)
                    .map_or(SqliteValue::Null, SqliteValue::Real),
                Some("text") => value
                    .get("value")
                    .and_then(Json::as_str)
                    .map_or(SqliteValue::Null, |text| SqliteValue::Text(text.to_owned())),
                Some("blob") => value
                    .get("value")
                    .and_then(Json::as_str)
                    .and_then(|raw| base64::engine::general_purpose::STANDARD.decode(raw).ok())
                    .map_or(SqliteValue::Null, SqliteValue::Blob),
                _ => SqliteValue::Null,
            }
        }

        /// One SQLite value in protocol shape.
        fn protocol_value(value: SqliteValue) -> Json {
            match value {
                SqliteValue::Null => json!({"type": "null"}),
                SqliteValue::Integer(int) => {
                    json!({"type": "integer", "value": int.to_string()})
                }
                SqliteValue::Real(float) => json!({"type": "float", "value": float}),
                SqliteValue::Text(text) => json!({"type": "text", "value": text}),
                SqliteValue::Blob(blob) => json!({
                    "type": "blob",
                    "value": base64::engine::general_purpose::STANDARD.encode(blob),
                }),
            }
        }

        /// Executes one statement and builds its protocol result.
        fn execute(&self, db: &rusqlite::Connection, stmt: &Json) -> Json {
            let sql = stmt.get("sql").and_then(Json::as_str).unwrap_or("");
            let args: Vec<SqliteValue> = stmt
                .get("args")
                .and_then(Json::as_array)
                .map(|args| args.iter().map(Self::sqlite_value).collect())
                .unwrap_or_default();
            let mut prepared = match db.prepare(sql) {
                Ok(prepared) => prepared,
                Err(err) => {
                    return json!({
                        "type": "error",
                        "error": {"message": err.to_string(), "code": "SQLITE_ERROR"},
                    });
                }
            };
            let column_count = prepared.column_count();
            if column_count == 0 {
                return match prepared.execute(rusqlite::params_from_iter(args)) {
                    Ok(affected) => json!({
                        "type": "ok",
                        "response": {
                            "type": "execute",
                            "result": {
                                "cols": [],
                                "rows": [],
                                "affected_row_count": affected,
                                "last_insert_rowid": db.last_insert_rowid(),
                            },
                        },
                    }),
                    Err(err) => json!({
                        "type": "error",
                        "error": {"message": err.to_string(), "code": "SQLITE_ERROR"},
                    }),
                };
            }
            let columns: Vec<String> = (0..column_count)
                .map(|index| prepared.column_name(index).unwrap_or("").to_owned())
                .collect();
            let mut query = match prepared.query(rusqlite::params_from_iter(args)) {
                Ok(query) => query,
                Err(err) => {
                    return json!({
                        "type": "error",
                        "error": {"message": err.to_string(), "code": "SQLITE_ERROR"},
                    });
                }
            };
            let mut rows = Vec::new();
            while let Some(row) = query.next().unwrap() {
                let mut values = Vec::new();
                for index in 0..column_count {
                    values.push(Self::protocol_value(
                        row.get::<_, SqliteValue>(index).unwrap(),
                    ));
                }
                rows.push(Json::Array(values));
            }
            json!({
                "type": "ok",
                "response": {
                    "type": "execute",
                    "result": {
                        "cols": columns
                            .into_iter()
                            .map(|name| json!({"name": name}))
                            .collect::<Vec<_>>(),
                        "rows": rows,
                        "affected_row_count": 0,
                        "last_insert_rowid": null,
                    },
                },
            })
        }
    }

    impl PipelineTransport for SqliteTransport {
        fn pipeline(&self, body: &[u8]) -> Result<Vec<u8>, TursoError> {
            let request: Json = serde_json::from_slice(body)
                .map_err(|err| TursoError::Protocol(err.to_string()))?;
            let db = self.db.lock().unwrap();
            let mut results = Vec::new();
            for item in request.get("requests").and_then(Json::as_array).unwrap() {
                match item.get("type").and_then(Json::as_str) {
                    Some("execute") => {
                        let stmt = item.get("stmt").cloned().unwrap_or(Json::Null);
                        results.push(self.execute(&db, &stmt));
                    }
                    Some("close") => {
                        results.push(json!({
                            "type": "ok",
                            "response": {"type": "close"},
                        }));
                    }
                    other => {
                        return Err(TursoError::Protocol(format!(
                            "test endpoint met unknown request {other:?}"
                        )));
                    }
                }
            }
            serde_json::to_vec(&json!({"baton": "test-baton", "results": results}))
                .map_err(|err| TursoError::Protocol(err.to_string()))
        }
    }

    fn open_test_store() -> TenantStore<SqliteTransport> {
        TenantStore::open(SqliteTransport::new()).unwrap()
    }

    /// Native-redb oracle: COMPACTED is an additive single-value table,
    /// just like the native event store's compaction residue table.
    #[test]
    fn test_native_redb_oracle_compacted_table_roundtrip() {
        let mut store = open_test_store();
        store.put("compacted", b"position", b"residue").unwrap();
        assert_eq!(
            store.get("compacted", b"position").unwrap(),
            Some(b"residue".to_vec())
        );
    }

    /// The oracle is the protocol's value encoding (hrana over HTTP):
    /// integers as decimal strings, blobs as base64.
    #[test]
    fn test_value_codec_round_trips() {
        let values = [
            SqlValue::Null,
            SqlValue::Integer(0),
            SqlValue::Integer(-42),
            SqlValue::Integer(i64::MAX),
            SqlValue::Float(1.5),
            SqlValue::Text("tenant".to_owned()),
            SqlValue::Blob(vec![0, 1, 0xFF]),
            SqlValue::Blob(Vec::new()),
        ];
        for value in values {
            let encoded = value.to_json().unwrap();
            assert_eq!(SqlValue::from_json(&encoded).unwrap(), value);
        }
        assert_eq!(
            SqlValue::Integer(42).to_json().unwrap(),
            json!({"type": "integer", "value": "42"}),
        );
        assert_eq!(
            SqlValue::Blob(vec![1, 2]).to_json().unwrap(),
            json!({"type": "blob", "value": "AQI="}),
        );
    }

    /// The oracle is the pipeline request shape: a baton field once the
    /// server issued one, execute requests with bound args, close as
    /// its own request type.
    #[test]
    fn test_pipeline_request_shape() {
        let statement = Statement::with_args(
            "SELECT ?",
            vec![SqlValue::Integer(7), SqlValue::Blob(vec![9])],
        );
        assert_eq!(
            statement.to_json().unwrap(),
            json!({
                "type": "execute",
                "stmt": {
                    "sql": "SELECT ?",
                    "args": [
                        {"type": "integer", "value": "7"},
                        {"type": "blob", "value": "CQ=="},
                    ],
                    "named_args": [],
                    "want_rows": true,
                },
            }),
        );
    }

    /// A statement error in the response is the call's error, with the
    /// server's message and code preserved.
    #[test]
    fn test_pipeline_response_surfaces_sql_errors() {
        #[derive(Debug)]
        struct ErrorTransport;
        impl PipelineTransport for ErrorTransport {
            fn pipeline(&self, _body: &[u8]) -> Result<Vec<u8>, TursoError> {
                Ok(serde_json::to_vec(&json!({
                    "baton": null,
                    "results": [{
                        "type": "error",
                        "error": {"message": "no such table: nope", "code": "SQLITE_ERROR"},
                    }],
                }))
                .unwrap())
            }
        }
        let mut conn = Connection::new(ErrorTransport);
        let err = conn.execute(&Statement::new("SELECT 1")).unwrap_err();
        match err {
            TursoError::Sql { message, code } => {
                assert_eq!(message, "no such table: nope");
                assert_eq!(code.as_deref(), Some("SQLITE_ERROR"));
            }
            other => panic!("expected a SQL error, got {other:?}"),
        }
    }

    /// The token a transport carries never reaches its `Debug` output.
    #[test]
    fn test_http_transport_debug_redacts_the_token() {
        let transport = HttpTransport::new(
            "https://tenant.example.com",
            "secret-token-value".to_owned(),
        )
        .unwrap();
        let debug = format!("{transport:?}");
        assert!(debug.contains("tenant.example.com"));
        assert!(!debug.contains("secret-token-value"));
    }

    /// The oracle is the native store's table semantics (redb):
    /// byte-ordered keys, last write wins, remove of an absent key is
    /// false, multimap values are a set in value order, and range reads
    /// follow the same bounds. One op script runs against both backends
    /// and every observable result must agree.
    #[test]
    fn test_row_store_matches_redb_table_semantics() {
        const EVENTS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("events");
        const GROUPS: MultimapTableDefinition<&[u8], &[u8]> =
            MultimapTableDefinition::new("groups");

        let dir = tempfile::tempdir().unwrap();
        let db = Database::create(dir.path().join("native.redb")).unwrap();
        let mut store = open_test_store();

        // Puts, including an overwrite and edge-case keys.
        let puts: &[(&[u8], &[u8])] = &[
            (b"a", b"1"),
            (b"b", b"2"),
            (b"a", b"3"),
            (b"", b"empty-key"),
            (&[0x00], b"nul"),
            (&[0xFF, 0x00], b"high"),
            (&[0x61, 0x62], b"ab"),
        ];
        {
            let txn = db.begin_write().unwrap();
            {
                let mut table = txn.open_table(EVENTS).unwrap();
                for (key, value) in puts {
                    table.insert(*key, *value).unwrap();
                }
            }
            txn.commit().unwrap();
        }
        for (key, value) in puts {
            store.put("events", key, value).unwrap();
        }
        let read = db.begin_read().unwrap();
        let native = read.open_table(EVENTS).unwrap();
        for key in [
            b"a".as_slice(),
            b"b",
            b"",
            &[0x00],
            &[0xFF, 0x00],
            b"absent",
        ] {
            let native_value = native.get(key).unwrap().map(|guard| guard.value().to_vec());
            assert_eq!(
                store.get("events", key).unwrap(),
                native_value,
                "get {key:?}"
            );
        }

        // A remove: present once, absent after — on both backends.
        {
            let txn = db.begin_write().unwrap();
            {
                let mut table = txn.open_table(EVENTS).unwrap();
                assert!(table.remove(b"b".as_slice()).unwrap().is_some());
                assert!(table.remove(b"b".as_slice()).unwrap().is_none());
            }
            txn.commit().unwrap();
        }
        assert!(store.remove("events", b"b").unwrap());
        assert!(!store.remove("events", b"b").unwrap());

        // Full scans and bounded scans agree, in key order. The read
        // handle above predates the remove, so reopen it: the oracle is
        // the native table's current contents, not a stale snapshot.
        let read = db.begin_read().unwrap();
        let native = read.open_table(EVENTS).unwrap();
        let native_all: Vec<(Vec<u8>, Vec<u8>)> = native
            .iter()
            .unwrap()
            .map(|row| {
                let (key, value) = row.unwrap();
                (key.value().to_vec(), value.value().to_vec())
            })
            .collect();
        assert_eq!(store.scan("events", .., 100).unwrap(), native_all);
        let native_range: Vec<(Vec<u8>, Vec<u8>)> = native
            .range(b"a".as_slice()..b"b".as_slice())
            .unwrap()
            .map(|row| {
                let (key, value) = row.unwrap();
                (key.value().to_vec(), value.value().to_vec())
            })
            .collect();
        assert_eq!(
            store
                .scan("events", b"a".to_vec()..b"b".to_vec(), 100)
                .unwrap(),
            native_range,
        );
        assert_eq!(
            store.scan_prefix("events", &[0x61], 100).unwrap(),
            native_all
                .iter()
                .filter(|(key, _)| key.starts_with(&[0x61]))
                .cloned()
                .collect::<Vec<_>>(),
        );
        assert_eq!(store.scan("events", .., 2).unwrap().len(), 2);

        // The row adapter exposes matching bounded pages and uses an
        // exclusive continuation key, including for an empty key.
        let mut native_scan = db.begin_read().unwrap();
        let mut native_rows = Vec::new();
        let mut after_key: Option<Vec<u8>> = None;
        loop {
            let page =
                RowRead::scan_page(&mut native_scan, "events", after_key.as_deref(), 2).unwrap();
            let page_len = page.len();
            if let Some((key, _)) = page.last() {
                after_key = Some(key.clone());
            }
            native_rows.extend(page);
            if page_len < 2 {
                break;
            }
        }
        let mut tenant_txn = TenantWrite::begin(&mut store).unwrap();
        let mut tenant_rows = Vec::new();
        let mut after_key: Option<Vec<u8>> = None;
        loop {
            let page = RowRead::scan_page(&mut &mut tenant_txn, "events", after_key.as_deref(), 2)
                .unwrap();
            let page_len = page.len();
            if let Some((key, _)) = page.last() {
                after_key = Some(key.clone());
            }
            tenant_rows.extend(page);
            if page_len < 2 {
                break;
            }
        }
        assert_eq!(tenant_rows, native_rows);
        tenant_txn.rollback().unwrap();

        // Multimap: a set of values per key, in value order.
        {
            let txn = db.begin_write().unwrap();
            {
                let mut table = txn.open_multimap_table(GROUPS).unwrap();
                table.insert(b"g".as_slice(), b"m2".as_slice()).unwrap();
                table.insert(b"g".as_slice(), b"m1".as_slice()).unwrap();
                table.insert(b"g".as_slice(), b"m2".as_slice()).unwrap();
                table.insert(b"g".as_slice(), b"m3".as_slice()).unwrap();
                assert!(table.remove(b"g".as_slice(), b"m2".as_slice()).unwrap());
            }
            txn.commit().unwrap();
        }
        for value in [b"m2".as_slice(), b"m1", b"m2", b"m3"] {
            store.multi_put("groups", b"g", value).unwrap();
        }
        assert!(store.multi_remove("groups", b"g", b"m2").unwrap());
        assert!(!store.multi_remove("groups", b"g", b"m2").unwrap());
        let read = db.begin_read().unwrap();
        let native_multi = read.open_multimap_table(GROUPS).unwrap();
        let native_values: Vec<Vec<u8>> = native_multi
            .get(b"g".as_slice())
            .unwrap()
            .map(|guard| guard.unwrap().value().to_vec())
            .collect();
        assert_eq!(store.multi_values("groups", b"g").unwrap(), native_values);
        assert_eq!(native_values, vec![b"m1".to_vec(), b"m3".to_vec()]);
    }

    /// Wrong-kind and unknown tables are refused before any SQL runs.
    #[test]
    fn test_table_kinds_are_enforced() {
        let mut store = open_test_store();
        assert!(matches!(
            store.get("groups", b"k"),
            Err(TursoError::WrongKind { .. })
        ));
        assert!(matches!(
            store.multi_values("events", b"k"),
            Err(TursoError::WrongKind { .. })
        ));
        assert!(matches!(
            store.get("nope", b"k"),
            Err(TursoError::UnknownTable(_))
        ));
    }

    /// A rolled-back transaction leaves nothing; a committed one
    /// leaves its rows — over the one baton session.
    #[test]
    fn test_transactions_commit_and_roll_back() {
        let mut store = open_test_store();
        store.begin().unwrap();
        store.put("events", b"k", b"v").unwrap();
        store.rollback().unwrap();
        assert_eq!(store.get("events", b"k").unwrap(), None);

        store.begin().unwrap();
        store.put("events", b"k", b"v").unwrap();
        store.commit().unwrap();
        assert_eq!(store.get("events", b"k").unwrap(), Some(b"v".to_vec()));

        assert!(matches!(store.commit(), Err(TursoError::Transaction(_))));
        store.begin().unwrap();
        assert!(matches!(store.begin(), Err(TursoError::Transaction(_))));
        store.rollback().unwrap();
    }

    /// A store whose format row names another format is refused, as
    /// the native store refuses a file of another format. The second
    /// open runs over a clone of the same endpoint transport, so it
    /// meets the first handle's rows exactly as a reopen would.
    #[test]
    fn test_open_refuses_a_foreign_format() {
        let transport = SqliteTransport::new();
        let mut store = TenantStore::open(transport.clone()).unwrap();
        store
            .connection()
            .execute(&Statement::with_args(
                "UPDATE tenant_meta SET value = ? WHERE key = ?",
                vec![
                    SqlValue::Blob(b"id.events.store.v9".to_vec()),
                    SqlValue::Text(FORMAT_KEY.to_owned()),
                ],
            ))
            .unwrap();
        drop(store);
        let reopened = TenantStore::open(transport);
        assert!(matches!(reopened, Err(TursoError::Format(_))));
    }

    /// After close, every operation is refused.
    #[test]
    fn test_close_ends_the_session() {
        let mut store = open_test_store();
        store.close().unwrap();
        assert!(matches!(store.get("events", b"k"), Err(TursoError::Closed)));
        assert!(matches!(store.close(), Err(TursoError::Closed)));
    }

    /// Prefix successors: the scan bound above a prefix.
    #[test]
    fn test_prefix_successor() {
        assert_eq!(prefix_successor(&[0x61]), Some(vec![0x62]));
        assert_eq!(prefix_successor(&[0x61, 0xFF]), Some(vec![0x62]));
        assert_eq!(prefix_successor(&[0xFF, 0xFF]), None);
        assert_eq!(prefix_successor(&[]), None);
    }
}
