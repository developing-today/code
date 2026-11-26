//! The central plane of `fleet kv` (Phase 2, #13316): schema, per-key CAS
//! with atomic idempotency receipts, append-only history and conflicts,
//! and namespace definitions.
//!
//! Every central operation is a list of SQL statements that the primary runs
//! in order inside one transaction ([`Pipeline::transaction`]). The same
//! statements run on the shared libSQL primary over its HTTP pipeline and on
//! [`SqlitePrimary`], the in-process primary the tests and the model use, so
//! the CAS protocol has one implementation.
//!
//! The CAS transaction (design §5) never branches on the client:
//!
//! 1. it inserts the idempotency receipt, deciding `committed` or `conflict`
//!    from the head as it stands inside the transaction, stamped with a fresh
//!    per-attempt nonce (`INSERT OR IGNORE`, so an existing receipt for the
//!    same `(writer, host incarnation, outbox id)` is kept as it is);
//! 2. it appends the conflict record, the version row and the new head, each
//!    only if the receipt carries this attempt's nonce and the matching
//!    outcome;
//! 3. it reads the receipt back.
//!
//! A receipt with another nonce means this request was already decided: the
//! same fingerprint replays the recorded result, and a different fingerprint
//! is `idempotency_key_reuse`. So a retransmit, even after the head moved on,
//! never applies twice and never turns into a spurious conflict.

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::types::{ToSqlOutput, ValueRef};

use crate::blob::Partition;
use rusqlite::{Connection, TransactionBehavior};

/// The registry handle of the distributed fleet registry (design §2).
pub const REGISTRY_ID: &str = "fleet-kv-central";
/// Default per-namespace inline threshold in bytes (design §8, D009).
pub const DEFAULT_BLOB_THRESHOLD: u64 = 256 * 1024;
/// At most this many attempts per CAS (design §5, default in effect).
pub const MAX_ATTEMPTS: u32 = 5;
/// No backoff between attempts is longer than this (design §5).
pub const BACKOFF_CAP: Duration = Duration::from_secs(1);

/// The numbered migrations, oldest first.
pub const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../migrations/0001_central.sql")),
    (2, include_str!("../migrations/0002_archive.sql")),
    (3, include_str!("../migrations/0003_blobs.sql")),
    (4, include_str!("../migrations/0004_placement.sql")),
    (5, include_str!("../migrations/0005_envelope.sql")),
    (6, include_str!("../migrations/0006_gc.sql")),
    (7, include_str!("../migrations/0007_purge.sql")),
];

/// A bound SQL value, in the shapes both the libSQL pipeline and SQLite use.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Value {
    pub(crate) fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(*value),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value),
            _ => None,
        }
    }

    pub(crate) fn into_bytes(self) -> Option<Vec<u8>> {
        match self {
            Self::Blob(value) => Some(value),
            Self::Text(value) => Some(value.into_bytes()),
            _ => None,
        }
    }
}

impl rusqlite::ToSql for Value {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(match self {
            Self::Null => ToSqlOutput::Owned(rusqlite::types::Value::Null),
            Self::Integer(value) => ToSqlOutput::Owned(rusqlite::types::Value::Integer(*value)),
            Self::Real(value) => ToSqlOutput::Owned(rusqlite::types::Value::Real(*value)),
            Self::Text(value) => ToSqlOutput::Borrowed(ValueRef::Text(value.as_bytes())),
            Self::Blob(value) => ToSqlOutput::Borrowed(ValueRef::Blob(value)),
        })
    }
}

pub(crate) fn text(value: &str) -> Value {
    Value::Text(value.to_string())
}

pub(crate) fn integer(value: u64) -> Value {
    Value::Integer(i64::try_from(value).unwrap_or(i64::MAX))
}

/// One statement with its positional arguments (`?1`, `?2`, ...).
#[derive(Clone, Debug)]
pub struct Statement {
    pub sql: String,
    pub args: Vec<Value>,
}

impl Statement {
    /// A statement bound to the leading arguments it numbers: SQLite counts
    /// a statement's parameters up to its highest `?N`, so the shared
    /// argument list is cut to that length.
    pub(crate) fn numbered(sql: &str, args: &[Value]) -> Self {
        let highest = highest_parameter(sql);
        Self {
            sql: sql.to_string(),
            args: args.iter().take(highest).cloned().collect(),
        }
    }
}

pub(crate) fn highest_parameter(sql: &str) -> usize {
    let bytes = sql.as_bytes();
    let mut highest = 0;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'?' {
            let start = index + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if let Ok(number) = sql[start..end].parse::<usize>() {
                highest = highest.max(number);
            }
            index = end;
        } else {
            index += 1;
        }
    }
    highest
}

/// The rows one statement returned (empty for writes).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

#[derive(Debug)]
pub enum CentralError {
    /// The primary could not be reached or did not answer; the request may
    /// or may not have committed, and is safe to retransmit.
    Unavailable(String),
    /// The primary answered with an error.
    Store(String),
    /// The same idempotency key arrived with a different request body.
    IdempotencyKeyReuse {
        recorded_fingerprint: String,
        fingerprint: String,
    },
    /// A namespace or key does not satisfy the design's syntax rules.
    Malformed(&'static str),
    /// No definition for the namespace in the named registry.
    UnknownNamespace {
        registry_id: String,
        namespace: String,
    },
    /// No central endpoint, or no usable credential for it, is configured.
    NotConfigured(String),
    /// The endpoint refused the request (`unauthorized`, `forbidden`,
    /// `data_plane_credential_refused`, or `central_refused`).
    Refused { code: &'static str, message: String },
}

impl CentralError {
    /// Stable typed refusal code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unavailable(_) => "unavailable",
            Self::Store(_) => "central_store",
            Self::IdempotencyKeyReuse { .. } => "idempotency_key_reuse",
            Self::Malformed(code) => code,
            Self::UnknownNamespace { .. } => "unknown_namespace",
            Self::NotConfigured(_) => "central_not_configured",
            Self::Refused { code, .. } => code,
        }
    }

    /// Whether retransmitting the same request may succeed.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }
}

impl std::fmt::Display for CentralError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(message) => write!(f, "central primary unavailable: {message}"),
            Self::Store(message) => write!(f, "central primary refused the request: {message}"),
            Self::IdempotencyKeyReuse {
                recorded_fingerprint,
                fingerprint,
            } => write!(
                f,
                "idempotency key reused with a different request \
                 (recorded {recorded_fingerprint}, got {fingerprint})"
            ),
            Self::Malformed(code) => write!(f, "{code}"),
            Self::UnknownNamespace {
                registry_id,
                namespace,
            } => write!(f, "unknown namespace {namespace} in registry {registry_id}"),
            Self::NotConfigured(message) => write!(f, "central plane not configured: {message}"),
            Self::Refused { code, message } => write!(f, "{code}: {message}"),
        }
    }
}

impl std::error::Error for CentralError {}

pub type Result<T> = std::result::Result<T, CentralError>;

/// The central primary: runs statements in order inside one transaction.
pub trait Pipeline {
    /// Runs every statement, in order, inside one transaction, and returns
    /// each statement's rows. Either every statement commits or none does.
    fn transaction(&self, statements: &[Statement]) -> Result<Vec<Rows>>;
}

/// An in-process primary on a SQLite file or memory: the tests' primary and
/// the model the libSQL pipeline is checked against. Several
/// `SqlitePrimary` values may open the same file; `BEGIN IMMEDIATE`
/// serializes their transactions the way the shared primary does.
pub struct SqlitePrimary {
    connection: Mutex<Connection>,
}

impl SqlitePrimary {
    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory().map_err(store)?)
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let connection = Connection::open(path).map_err(store)?;
        connection
            .busy_timeout(Duration::from_secs(30))
            .map_err(store)?;
        Self::from_connection(connection)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
        let primary = Self {
            connection: Mutex::new(connection),
        };
        migrate(&primary)?;
        Ok(primary)
    }
}

fn store(error: rusqlite::Error) -> CentralError {
    CentralError::Store(error.to_string())
}

fn from_sqlite(value: rusqlite::types::Value) -> Value {
    match value {
        rusqlite::types::Value::Null => Value::Null,
        rusqlite::types::Value::Integer(value) => Value::Integer(value),
        rusqlite::types::Value::Real(value) => Value::Real(value),
        rusqlite::types::Value::Text(value) => Value::Text(value),
        rusqlite::types::Value::Blob(value) => Value::Blob(value),
    }
}

impl SqlitePrimary {
    /// Runs statements that are not in the [`crate::catalog`], such as a
    /// test fixture's bulk seed. Everything else goes through
    /// [`Pipeline::transaction`], which in tests refuses unlisted SQL so the
    /// hosted door's catalog cannot fall behind the client.
    #[cfg(test)]
    pub(crate) fn transaction_unlisted(&self, statements: &[Statement]) -> Result<Vec<Rows>> {
        self.run(statements)
    }

    fn run(&self, statements: &[Statement]) -> Result<Vec<Rows>> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| CentralError::Store("primary connection poisoned".into()))?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(store)?;
        let mut results = Vec::with_capacity(statements.len());
        for statement in statements {
            let mut prepared = transaction.prepare(&statement.sql).map_err(store)?;
            let columns: Vec<String> = prepared.column_names().map(str::to_string).collect();
            let mut rows = prepared
                .query(rusqlite::params_from_iter(statement.args.iter()))
                .map_err(store)?;
            let mut collected = Vec::new();
            while let Some(row) = rows.next().map_err(store)? {
                let mut cells = Vec::with_capacity(columns.len());
                for index in 0..columns.len() {
                    let cell: rusqlite::types::Value = row.get(index).map_err(store)?;
                    cells.push(from_sqlite(cell));
                }
                collected.push(cells);
            }
            results.push(Rows {
                columns,
                rows: collected,
            });
        }
        transaction.commit().map_err(store)?;
        Ok(results)
    }
}

impl Pipeline for SqlitePrimary {
    fn transaction(&self, statements: &[Statement]) -> Result<Vec<Rows>> {
        if cfg!(test) {
            for statement in statements {
                assert!(
                    crate::catalog::is_known(&statement.sql),
                    "a statement the hosted kv door would refuse, missing from catalog.rs: {}",
                    statement.sql
                );
            }
        }
        self.run(statements)
    }
}

/// The statements of one migration file, in order.
pub fn migration_statements(source: &str) -> Vec<String> {
    let without_comments: String = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    without_comments
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        .map(str::to_string)
        .collect()
}

/// Applies every migration the primary has not recorded, each in its own
/// transaction with its `kv_central_migrations` row. Re-running is a no-op.
pub fn migrate(primary: &dyn Pipeline) -> Result<()> {
    let applied = primary.transaction(&[
        Statement::numbered(MIGRATIONS_TABLE, &[]),
        Statement::numbered(MIGRATIONS_READ, &[]),
    ])?;
    let applied: Vec<i64> = applied
        .get(1)
        .map(|rows| rows.rows.iter().filter_map(|row| row[0].as_i64()).collect())
        .unwrap_or_default();
    for (version, source) in MIGRATIONS {
        if applied.contains(version) {
            continue;
        }
        let mut statements: Vec<Statement> = migration_statements(source)
            .iter()
            .map(|sql| Statement::numbered(sql, &[]))
            .collect();
        statements.push(Statement::numbered(
            MIGRATION_RECORD,
            &[Value::Integer(*version), Value::Integer(now_ms())],
        ));
        primary.transaction(&statements)?;
    }
    Ok(())
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// A central key: `(scope_id, registry_id, namespace, key)` (design §1).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Address {
    pub scope_id: String,
    pub registry_id: String,
    pub namespace: String,
    pub key: String,
}

impl Address {
    pub fn new(scope_id: &str, namespace: &str, key: &str) -> Self {
        Self {
            scope_id: scope_id.to_string(),
            registry_id: REGISTRY_ID.to_string(),
            namespace: namespace.to_string(),
            key: key.to_string(),
        }
    }
}

/// What a write does to a head.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Set(Vec<u8>),
    /// A masking tombstone: the key reads absent in this scope.
    Tombstone,
}

impl Op {
    pub(crate) fn state(&self) -> &'static str {
        match self {
            Self::Set(_) => "live",
            Self::Tombstone => "tombstone",
        }
    }

    pub(crate) fn value(&self) -> &[u8] {
        match self {
            Self::Set(value) => value,
            Self::Tombstone => &[],
        }
    }
}

/// The idempotency key of one write: `(authenticated writer, host
/// incarnation, outbox id)` (design §5).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IdempotencyKey {
    pub writer: String,
    pub host_incarnation: String,
    pub outbox_id: String,
}

/// One CAS write: replace the head at `expected_version` (0 = absent).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Intent {
    pub address: Address,
    pub expected_version: u64,
    pub op: Op,
    pub idempotency: IdempotencyKey,
}

impl Intent {
    /// The immutable hash of the intent body: scope, registry, namespace,
    /// key, expected version, op and value. The writer and idempotency key
    /// are not part of the body.
    pub fn fingerprint(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        for field in [
            self.address.scope_id.as_bytes(),
            self.address.registry_id.as_bytes(),
            self.address.namespace.as_bytes(),
            self.address.key.as_bytes(),
            self.op.state().as_bytes(),
            self.op.value(),
        ] {
            hasher.update(&(field.len() as u64).to_le_bytes());
            hasher.update(field);
        }
        hasher.update(&self.expected_version.to_le_bytes());
        format!("blake3-256:{}", hasher.finalize().to_hex())
    }
}

/// How a CAS was decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The head moved from `expected_version` to `version`.
    Committed { version: u64 },
    /// The head was at `actual`, not the expected version; the proposed body
    /// is kept as conflict `conflict_id`.
    Conflict {
        expected: u64,
        actual: u64,
        conflict_id: i64,
    },
}

/// The result of a CAS request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub outcome: Outcome,
    /// True when an earlier transmission of the same request had already
    /// been decided and this is its recorded result.
    pub replayed: bool,
}

/// The receipt insert as a literal, so the publish-CAS can extend it.
macro_rules! receipt_insert {
    () => {
        "INSERT OR IGNORE INTO idem_receipts (
        writer, host_incarnation, outbox_id, request_fingerprint, attempt,
        scope_id, registry_id, namespace, key, expected_version,
        outcome, version, recorded_at_ms)
    SELECT ?8, ?9, ?10, ?11, ?12, ?1, ?2, ?3, ?4, ?5,
        CASE WHEN head.version = ?5 THEN 'committed' ELSE 'conflict' END,
        CASE WHEN head.version = ?5 THEN ?5 + 1 ELSE head.version END,
        ?13
    FROM (SELECT COALESCE((SELECT version FROM kv_heads
        WHERE scope_id = ?1 AND registry_id = ?2 AND namespace = ?3 AND key = ?4), 0) AS version) AS head"
    };
}

pub(crate) const RECEIPT_INSERT: &str = receipt_insert!();

/// The publish-CAS receipt (#13318, design §8): decided only while the
/// blob it names is published in its partition (`?14` jurisdiction, `?15`
/// tier, `?16` content id). With no published row no receipt is written,
/// so nothing else in the transaction moves.
pub(crate) const BLOB_RECEIPT_INSERT: &str = concat!(
    receipt_insert!(),
    "\n    WHERE EXISTS (SELECT 1 FROM kv_blobs AS blob\n        WHERE blob.jurisdiction = ?14 AND blob.tier = ?15 AND blob.content_id = ?16)"
);

macro_rules! this_attempt {
    () => {
        "receipt.writer = ?8 AND receipt.host_incarnation = ?9 AND receipt.outbox_id = ?10 AND receipt.attempt = ?12"
    };
}

pub(crate) const CONFLICT_INSERT: &str = concat!(
    "INSERT INTO kv_conflicts (
            scope_id, registry_id, namespace, key, expected_version, actual_version,
            proposed_state, proposed_value, writer, host_incarnation, outbox_id, written_at_ms)
        SELECT ?1, ?2, ?3, ?4, ?5, receipt.version, ?6, ?7, ?8, ?9, ?10, ?13
        FROM idem_receipts AS receipt
        WHERE ",
    this_attempt!(),
    " AND receipt.outcome = 'conflict'"
);

pub(crate) const VERSION_INSERT: &str = concat!(
    "INSERT INTO kv_versions (
            scope_id, registry_id, namespace, key, version, parent_version,
            state, value, writer, host_incarnation, outbox_id, written_at_ms)
        SELECT ?1, ?2, ?3, ?4, receipt.version, ?5, ?6, ?7, ?8, ?9, ?10, ?13
        FROM idem_receipts AS receipt
        WHERE ",
    this_attempt!(),
    " AND receipt.outcome = 'committed'"
);

/// The publish-CAS conflict body: as [`CONFLICT_INSERT`], proposing a
/// blob reference (design §11).
pub(crate) const BLOB_CONFLICT_INSERT: &str = concat!(
    "INSERT INTO kv_conflicts (
            scope_id, registry_id, namespace, key, expected_version, actual_version,
            proposed_state, proposed_value, writer, host_incarnation, outbox_id, written_at_ms,
            proposed_envelope)
        SELECT ?1, ?2, ?3, ?4, ?5, receipt.version, ?6, ?7, ?8, ?9, ?10, ?13, 'kv:ref-v1'
        FROM idem_receipts AS receipt
        WHERE ",
    this_attempt!(),
    " AND receipt.outcome = 'conflict'"
);

/// The publish-CAS version: as [`VERSION_INSERT`], whose value is a blob
/// reference (design §11). The head the same transaction writes takes its
/// envelope from this row.
pub(crate) const BLOB_VERSION_INSERT: &str = concat!(
    "INSERT INTO kv_versions (
            scope_id, registry_id, namespace, key, version, parent_version,
            state, value, writer, host_incarnation, outbox_id, written_at_ms, envelope)
        SELECT ?1, ?2, ?3, ?4, receipt.version, ?5, ?6, ?7, ?8, ?9, ?10, ?13, 'kv:ref-v1'
        FROM idem_receipts AS receipt
        WHERE ",
    this_attempt!(),
    " AND receipt.outcome = 'committed'"
);

pub(crate) const HEAD_UPSERT: &str = concat!(
    "INSERT INTO kv_heads (
            scope_id, registry_id, namespace, key, version, state, value, writer, written_at_ms)
        SELECT ?1, ?2, ?3, ?4, receipt.version, ?6, ?7, ?8, ?13
        FROM idem_receipts AS receipt
        WHERE ",
    this_attempt!(),
    " AND receipt.outcome = 'committed'
        ON CONFLICT (scope_id, registry_id, namespace, key) DO UPDATE SET
            version = excluded.version,
            state = excluded.state,
            value = excluded.value,
            writer = excluded.writer,
            written_at_ms = excluded.written_at_ms
        WHERE kv_heads.version = ?5"
);

pub(crate) const RECEIPT_READ: &str =
    "SELECT receipt.request_fingerprint, receipt.attempt, receipt.outcome,
        receipt.expected_version, receipt.version,
        (SELECT conflict.id FROM kv_conflicts AS conflict
         WHERE conflict.writer = ?8 AND conflict.host_incarnation = ?9
           AND conflict.outbox_id = ?10)
    FROM idem_receipts AS receipt
    WHERE receipt.writer = ?8 AND receipt.host_incarnation = ?9 AND receipt.outbox_id = ?10";

/// A nonce for one transaction attempt; unique per process and call.
pub(crate) fn attempt_nonce() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut hasher = blake3::Hasher::new();
    hasher.update(&std::process::id().to_le_bytes());
    hasher.update(&now_ms().to_le_bytes());
    hasher.update(&COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos())
        .unwrap_or(0);
    hasher.update(&nanos.to_le_bytes());
    let hex = hasher.finalize().to_hex();
    hex[..32].to_string()
}

/// The statements of one CAS transaction, for `attempt`.
pub fn cas_statements(intent: &Intent, attempt: &str) -> Vec<Statement> {
    let args = cas_args(intent, attempt);
    vec![
        Statement::numbered(RECEIPT_INSERT, &args),
        Statement::numbered(CONFLICT_INSERT, &args),
        Statement::numbered(VERSION_INSERT, &args),
        Statement::numbered(HEAD_UPSERT, &args),
        Statement::numbered(RECEIPT_READ, &args),
    ]
}

/// The thirteen positional arguments every CAS statement shares.
pub(crate) fn cas_args(intent: &Intent, attempt: &str) -> Vec<Value> {
    vec![
        text(&intent.address.scope_id),
        text(&intent.address.registry_id),
        text(&intent.address.namespace),
        text(&intent.address.key),
        integer(intent.expected_version),
        text(intent.op.state()),
        Value::Blob(intent.op.value().to_vec()),
        text(&intent.idempotency.writer),
        text(&intent.idempotency.host_incarnation),
        text(&intent.idempotency.outbox_id),
        text(&intent.fingerprint()),
        text(attempt),
        Value::Integer(now_ms()),
    ]
}

/// One CAS attempt: a single transaction on the primary.
pub fn cas(primary: &dyn Pipeline, intent: &Intent) -> Result<Receipt> {
    validate(&intent.address)?;
    let attempt = attempt_nonce();
    decide(primary, intent, &cas_statements(intent, &attempt), &attempt)?
        .ok_or_else(|| CentralError::Store("the CAS transaction returned no receipt".into()))
}

/// Run one CAS-shaped transaction whose last statement reads the receipt,
/// and decode it. `None` when no receipt was recorded for the key.
pub(crate) fn decide(
    primary: &dyn Pipeline,
    intent: &Intent,
    statements: &[Statement],
    attempt: &str,
) -> Result<Option<Receipt>> {
    let results = primary.transaction(statements)?;
    let Some(row) = results.last().and_then(|rows| rows.rows.first()) else {
        return Ok(None);
    };
    let field = |index: usize| row.get(index).cloned().unwrap_or(Value::Null);
    let recorded_fingerprint = field(0).as_str().unwrap_or_default().to_string();
    let fingerprint = intent.fingerprint();
    if recorded_fingerprint != fingerprint {
        return Err(CentralError::IdempotencyKeyReuse {
            recorded_fingerprint,
            fingerprint,
        });
    }
    let replayed = field(1).as_str() != Some(attempt);
    let expected = field(3).as_i64().unwrap_or(0).max(0) as u64;
    let version = field(4).as_i64().unwrap_or(0).max(0) as u64;
    let outcome = match field(2).as_str() {
        Some("committed") => Outcome::Committed { version },
        Some("conflict") => Outcome::Conflict {
            expected,
            actual: version,
            conflict_id: field(5).as_i64().ok_or_else(|| {
                CentralError::Store("a conflict receipt has no conflict record".into())
            })?,
        },
        other => {
            return Err(CentralError::Store(format!(
                "a receipt has an unknown outcome {other:?}"
            )));
        }
    };
    Ok(Some(Receipt { outcome, replayed }))
}

/// Bounded retries for one CAS (design §5): at most [`MAX_ATTEMPTS`]
/// transmissions of the same request, retrying only when the primary was
/// unreachable, with jittered exponential backoff capped at
/// [`BACKOFF_CAP`]. Every transmission carries the same idempotency key, so
/// a request that committed before its answer was lost replays its receipt.
pub fn cas_with_retry(
    primary: &dyn Pipeline,
    intent: &Intent,
    sleep: &mut dyn FnMut(Duration),
) -> Result<Receipt> {
    with_retry(intent, sleep, || cas(primary, intent))
}

/// Run `attempt` until it succeeds, fails for good, or [`MAX_ATTEMPTS`]
/// attempts have met an unavailable primary, with full-jitter backoff
/// seeded by `intent`'s fingerprint between them.
pub(crate) fn with_retry<T>(
    intent: &Intent,
    sleep: &mut dyn FnMut(Duration),
    mut attempt_once: impl FnMut() -> Result<T>,
) -> Result<T> {
    let mut jitter = Jitter::seeded(&intent.fingerprint());
    let mut attempt = 1;
    loop {
        match attempt_once() {
            Err(error) if error.is_retryable() && attempt < MAX_ATTEMPTS => {
                sleep(jitter.backoff(attempt));
                attempt += 1;
            }
            other => return other,
        }
    }
}

/// Full-jitter exponential backoff from a small deterministic generator.
struct Jitter(u64);

impl Jitter {
    fn seeded(seed: &str) -> Self {
        let digest = blake3::hash(seed.as_bytes());
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&digest.as_bytes()[..8]);
        Self((u64::from_le_bytes(bytes) ^ (now_ms() as u64)) | 1)
    }

    fn next(&mut self) -> u64 {
        // xorshift64
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A delay in `[0, min(cap, 50 ms * 2^(attempt-1))]`.
    fn backoff(&mut self, attempt: u32) -> Duration {
        let ceiling_ms =
            (50u64 << attempt.saturating_sub(1).min(10)).min(BACKOFF_CAP.as_millis() as u64);
        Duration::from_millis(self.next() % (ceiling_ms + 1))
    }
}

/// A key's current head.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Head {
    pub version: u64,
    pub live: bool,
    /// The stored bytes: the value itself, or for a `kv:ref-v1` head the
    /// encoded blob reference.
    pub value: Vec<u8>,
    pub writer: String,
    pub written_at_ms: i64,
    /// The row envelope version (design §11): `kv:raw-v1` or `kv:ref-v1`.
    pub envelope: String,
}

impl Head {
    /// The head's value under its envelope. An envelope this client does
    /// not know is refused, never read as bytes.
    pub fn envelope(&self) -> std::result::Result<crate::blob::Envelope, crate::blob::BlobError> {
        crate::blob::Envelope::from_row(&self.envelope, &self.value)
    }
}

/// One row of a key's history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub version: u64,
    pub parent_version: u64,
    pub live: bool,
    pub value: Vec<u8>,
    pub writer: String,
    pub idempotency: IdempotencyKey,
}

/// One failed CAS, with the body it proposed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub id: i64,
    pub expected_version: u64,
    pub actual_version: u64,
    pub proposed: Op,
    pub idempotency: IdempotencyKey,
}

macro_rules! address_filter {
    () => {
        "scope_id = ?1 AND registry_id = ?2 AND namespace = ?3 AND key = ?4"
    };
}

pub(crate) const HEAD_READ: &str = concat!(
    "SELECT version, state, value, writer, written_at_ms FROM kv_heads WHERE ",
    address_filter!()
);

pub(crate) const HISTORY_READ: &str = concat!(
    "SELECT version, parent_version, state, value, writer, host_incarnation, outbox_id
             FROM kv_versions WHERE ",
    address_filter!(),
    " ORDER BY version"
);

pub(crate) const CONFLICTS_READ: &str = concat!(
    "SELECT id, expected_version, actual_version, proposed_state, proposed_value,
                    writer, host_incarnation, outbox_id
             FROM kv_conflicts WHERE ",
    address_filter!(),
    " ORDER BY id"
);

/// The envelope of a head: its version row's, inline bytes when that row
/// is absent (a head written before history was kept).
macro_rules! head_envelope {
    () => {
        "COALESCE((SELECT version_row.envelope FROM kv_versions AS version_row
            WHERE version_row.scope_id = head.scope_id AND version_row.registry_id = head.registry_id
              AND version_row.namespace = head.namespace AND version_row.key = head.key
              AND version_row.version = head.version), 'kv:raw-v1')"
    };
}

/// [`HEAD_READ`] with the head's envelope (design §11).
pub(crate) const HEAD_ENVELOPED_READ: &str = concat!(
    "SELECT head.version, head.state, head.value, head.writer, head.written_at_ms, ",
    head_envelope!(),
    "
         FROM kv_heads AS head
         WHERE head.scope_id = ?1 AND head.registry_id = ?2 AND head.namespace = ?3 AND head.key = ?4"
);

/// [`HEADS_READ`] with each head's envelope (design §11).
pub(crate) const HEADS_ENVELOPED_READ: &str = concat!(
    "SELECT head.key, head.version, head.state, head.value, head.writer, head.written_at_ms, ",
    head_envelope!(),
    "
         FROM kv_heads AS head
         WHERE head.scope_id = ?1 AND head.registry_id = ?2 AND head.namespace = ?3
         ORDER BY head.key"
);

pub(crate) const HEADS_READ: &str =
    "SELECT key, version, state, value, writer, written_at_ms FROM kv_heads
         WHERE scope_id = ?1 AND registry_id = ?2 AND namespace = ?3 ORDER BY key";

pub(crate) const LISTING_READ: &str = "SELECT key, version, state FROM kv_heads
         WHERE scope_id = ?1 AND registry_id = ?2 AND namespace = ?3 ORDER BY key";

pub(crate) const NAMESPACE_UPSERT: &str = "INSERT INTO kv_namespaces (
                 registry_id, namespace, scope_kind, shared_scope_id, core_binding,
                 global_flag, blob_threshold, policy_epoch, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT (registry_id, namespace) DO UPDATE SET
                 scope_kind = excluded.scope_kind,
                 shared_scope_id = excluded.shared_scope_id,
                 core_binding = excluded.core_binding,
                 global_flag = excluded.global_flag,
                 blob_threshold = excluded.blob_threshold,
                 policy_epoch = MAX(kv_namespaces.policy_epoch + 1, excluded.policy_epoch),
                 updated_at_ms = excluded.updated_at_ms";

pub(crate) const NAMESPACE_EPOCH_READ: &str =
    "SELECT policy_epoch FROM kv_namespaces WHERE registry_id = ?1 AND namespace = ?2";

pub(crate) const NAMESPACES_READ: &str =
    "SELECT registry_id, namespace, scope_kind, shared_scope_id, core_binding,
                global_flag, blob_threshold, policy_epoch
         FROM kv_namespaces WHERE registry_id = ?1 ORDER BY namespace";

/// Set a namespace's blob placement, only beside its definition: `?1`
/// registry, `?2` namespace, `?3` jurisdiction, `?4` tier, `?5` now.
pub(crate) const PLACEMENT_UPSERT: &str = "INSERT INTO kv_namespace_placement (
                 registry_id, namespace, jurisdiction, tier, updated_at_ms)
             SELECT ?1, ?2, ?3, ?4, ?5
             WHERE EXISTS (SELECT 1 FROM kv_namespaces WHERE registry_id = ?1 AND namespace = ?2)
             ON CONFLICT (registry_id, namespace) DO UPDATE SET
                 jurisdiction = excluded.jurisdiction,
                 tier = excluded.tier,
                 updated_at_ms = excluded.updated_at_ms";

/// Drop a namespace's blob placement: `?1` registry, `?2` namespace.
pub(crate) const PLACEMENT_DELETE: &str =
    "DELETE FROM kv_namespace_placement WHERE registry_id = ?1 AND namespace = ?2";

/// Every placement in a registry: `?1` registry.
pub(crate) const PLACEMENTS_READ: &str =
    "SELECT namespace, jurisdiction, tier FROM kv_namespace_placement
         WHERE registry_id = ?1 ORDER BY namespace";

pub(crate) const MIGRATIONS_TABLE: &str = "CREATE TABLE IF NOT EXISTS kv_central_migrations (
                 version INTEGER PRIMARY KEY,
                 applied_at_ms INTEGER NOT NULL
             )";

pub(crate) const MIGRATIONS_READ: &str = "SELECT version FROM kv_central_migrations";

pub(crate) const MIGRATION_RECORD: &str =
    "INSERT OR IGNORE INTO kv_central_migrations (version, applied_at_ms) VALUES (?1, ?2)";

fn address_args(address: &Address) -> [Value; 4] {
    [
        text(&address.scope_id),
        text(&address.registry_id),
        text(&address.namespace),
        text(&address.key),
    ]
}

pub(crate) fn read(primary: &dyn Pipeline, sql: &str, args: &[Value]) -> Result<Vec<Vec<Value>>> {
    Ok(primary
        .transaction(&[Statement::numbered(sql, args)])?
        .into_iter()
        .next()
        .map(|rows| rows.rows)
        .unwrap_or_default())
}

pub(crate) fn unsigned(value: &Value) -> u64 {
    value.as_i64().unwrap_or(0).max(0) as u64
}

pub(crate) fn string(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}

fn op_of(state: &Value, value: Value) -> Op {
    if state.as_str() == Some("tombstone") {
        Op::Tombstone
    } else {
        Op::Set(value.into_bytes().unwrap_or_default())
    }
}

/// The authoritative head of `address`, tombstones included.
pub fn head(primary: &dyn Pipeline, address: &Address) -> Result<Option<Head>> {
    let rows = read(primary, HEAD_ENVELOPED_READ, &address_args(address))?;
    Ok(rows.into_iter().next().map(|mut row| {
        let value = std::mem::replace(&mut row[2], Value::Null);
        Head {
            version: unsigned(&row[0]),
            live: row[1].as_str() == Some("live"),
            value: value.into_bytes().unwrap_or_default(),
            writer: string(&row[3]),
            written_at_ms: row[4].as_i64().unwrap_or(0),
            envelope: string(&row[5]),
        }
    }))
}

/// Every head in one scope of one namespace, by key, tombstones included.
pub fn heads(
    primary: &dyn Pipeline,
    scope_id: &str,
    registry_id: &str,
    namespace: &str,
) -> Result<Vec<(String, Head)>> {
    let rows = read(
        primary,
        HEADS_ENVELOPED_READ,
        &[text(scope_id), text(registry_id), text(namespace)],
    )?;
    Ok(rows
        .into_iter()
        .map(|mut row| {
            let value = std::mem::replace(&mut row[3], Value::Null);
            (
                string(&row[0]),
                Head {
                    version: unsigned(&row[1]),
                    live: row[2].as_str() == Some("live"),
                    value: value.into_bytes().unwrap_or_default(),
                    writer: string(&row[4]),
                    written_at_ms: row[5].as_i64().unwrap_or(0),
                    envelope: string(&row[6]),
                },
            )
        })
        .collect())
}

/// One key of a scope's listing: its head version and whether it is live.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub key: String,
    pub version: u64,
    pub live: bool,
}

/// The heads in one scope of one namespace, by key, without their values.
pub fn listing(
    primary: &dyn Pipeline,
    scope_id: &str,
    registry_id: &str,
    namespace: &str,
) -> Result<Vec<Listed>> {
    let rows = read(
        primary,
        LISTING_READ,
        &[text(scope_id), text(registry_id), text(namespace)],
    )?;
    Ok(rows
        .into_iter()
        .map(|row| Listed {
            key: string(&row[0]),
            version: unsigned(&row[1]),
            live: row[2].as_str() == Some("live"),
        })
        .collect())
}

/// Every committed version of `address`, oldest first.
pub fn history(primary: &dyn Pipeline, address: &Address) -> Result<Vec<Version>> {
    let rows = read(primary, HISTORY_READ, &address_args(address))?;
    Ok(rows
        .into_iter()
        .map(|mut row| {
            let value = std::mem::replace(&mut row[3], Value::Null);
            let live = row[2].as_str() == Some("live");
            Version {
                version: unsigned(&row[0]),
                parent_version: unsigned(&row[1]),
                live,
                value: value.into_bytes().unwrap_or_default(),
                writer: string(&row[4]),
                idempotency: IdempotencyKey {
                    writer: string(&row[4]),
                    host_incarnation: string(&row[5]),
                    outbox_id: string(&row[6]),
                },
            }
        })
        .collect())
}

/// Every failed CAS on `address`, oldest first.
pub fn conflicts(primary: &dyn Pipeline, address: &Address) -> Result<Vec<Conflict>> {
    let rows = read(primary, CONFLICTS_READ, &address_args(address))?;
    Ok(rows
        .into_iter()
        .map(|mut row| {
            let value = std::mem::replace(&mut row[4], Value::Null);
            Conflict {
                id: row[0].as_i64().unwrap_or(0),
                expected_version: unsigned(&row[1]),
                actual_version: unsigned(&row[2]),
                proposed: op_of(&row[3], value),
                idempotency: IdempotencyKey {
                    writer: string(&row[5]),
                    host_incarnation: string(&row[6]),
                    outbox_id: string(&row[7]),
                },
            }
        })
        .collect())
}

/// How a namespace selects its central scope (design §1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// One shared scope for every authorized agent.
    Shared(String),
    /// A private scope per `(owner, context)`.
    Private,
}

/// A namespace definition in a registry (design §2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Namespace {
    pub registry_id: String,
    pub namespace: String,
    pub scope: Scope,
    pub core_binding: Option<String>,
    /// Designation only (D003): never changes how reads resolve.
    pub global_flag: bool,
    pub blob_threshold: u64,
    pub policy_epoch: u64,
    /// The partition every blob of this namespace goes to; `None` places
    /// none (no default partition, acceptance item 6).
    pub placement: Option<Partition>,
}

impl Namespace {
    /// A shared namespace in the fleet registry with the default threshold.
    pub fn shared(namespace: &str, scope_id: &str) -> Self {
        Self {
            registry_id: REGISTRY_ID.to_string(),
            namespace: namespace.to_string(),
            scope: Scope::Shared(scope_id.to_string()),
            core_binding: None,
            global_flag: false,
            blob_threshold: DEFAULT_BLOB_THRESHOLD,
            policy_epoch: 1,
            placement: None,
        }
    }

    /// A private namespace in the fleet registry with the default threshold.
    pub fn private(namespace: &str) -> Self {
        Self {
            scope: Scope::Private,
            ..Self::shared(namespace, "")
        }
    }

    /// The one central scope this namespace selects for a caller: the
    /// shared scope, or the caller's private scope. There is no second
    /// fallthrough scope.
    pub fn scope_for(&self, owner_user_id: &str, context_id: &str) -> String {
        match &self.scope {
            Scope::Shared(scope_id) => scope_id.clone(),
            Scope::Private => private_scope_id(owner_user_id, context_id),
        }
    }
}

/// The private scope of `(owner, context)`: `private:`, the owner's id in
/// lowercase hex, `:`, then a hash of both. The hash means no choice of names
/// can make two identities share a scope or collide with a shared scope id;
/// the hex owner lets the hosted door check that a private scope belongs to
/// the authenticated caller without knowing the context.
pub fn private_scope_id(owner_user_id: &str, context_id: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    for field in [owner_user_id, context_id] {
        hasher.update(&(field.len() as u64).to_le_bytes());
        hasher.update(field.as_bytes());
    }
    format!(
        "{}{}",
        private_scope_prefix(owner_user_id),
        hasher.finalize().to_hex()
    )
}

/// The prefix every private scope of `owner_user_id` starts with.
pub fn private_scope_prefix(owner_user_id: &str) -> String {
    let mut prefix = String::with_capacity(9 + owner_user_id.len() * 2);
    prefix.push_str("private:");
    for byte in owner_user_id.bytes() {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        prefix.push(char::from(HEX[usize::from(byte >> 4)]));
        prefix.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    prefix.push(':');
    prefix
}

/// Writes a namespace definition. The stored policy epoch becomes one more
/// than the previous definition's, or the given epoch if that is larger, so
/// every change is visible to hosts that cached the old one.
pub fn define_namespace(primary: &dyn Pipeline, definition: &Namespace) -> Result<u64> {
    crate::validate_namespace(&definition.namespace)
        .map_err(|_| CentralError::Malformed("malformed_namespace"))?;
    let (kind, shared) = match &definition.scope {
        Scope::Shared(scope_id) if !scope_id.is_empty() => ("shared", text(scope_id)),
        Scope::Shared(_) => return Err(CentralError::Malformed("malformed_scope")),
        Scope::Private => ("private", Value::Null),
    };
    let args = [
        text(&definition.registry_id),
        text(&definition.namespace),
        text(kind),
        shared,
        definition
            .core_binding
            .as_deref()
            .map(text)
            .unwrap_or(Value::Null),
        Value::Integer(i64::from(definition.global_flag)),
        integer(definition.blob_threshold),
        integer(definition.policy_epoch.max(1)),
        Value::Integer(now_ms()),
    ];
    let placement = match &definition.placement {
        Some(partition) => {
            if !crate::blob::valid_label(&partition.jurisdiction)
                || !crate::blob::valid_label(&partition.tier)
            {
                return Err(CentralError::Malformed("malformed_partition"));
            }
            Statement::numbered(
                PLACEMENT_UPSERT,
                &[
                    text(&definition.registry_id),
                    text(&definition.namespace),
                    text(&partition.jurisdiction),
                    text(&partition.tier),
                    Value::Integer(now_ms()),
                ],
            )
        }
        None => Statement::numbered(
            PLACEMENT_DELETE,
            &[text(&definition.registry_id), text(&definition.namespace)],
        ),
    };
    let results = primary.transaction(&[
        Statement::numbered(NAMESPACE_UPSERT, &args),
        placement,
        Statement::numbered(NAMESPACE_EPOCH_READ, &args),
    ])?;
    results
        .last()
        .and_then(|rows| rows.rows.first())
        .map(|row| unsigned(&row[0]))
        .ok_or_else(|| CentralError::Store("the namespace definition was not stored".into()))
}

/// Every namespace defined in `registry_id`, by name.
pub fn namespaces(primary: &dyn Pipeline, registry_id: &str) -> Result<Vec<Namespace>> {
    let registry = [text(registry_id)];
    let mut results = primary.transaction(&[
        Statement::numbered(NAMESPACES_READ, &registry),
        Statement::numbered(PLACEMENTS_READ, &registry),
    ])?;
    if results.len() != 2 {
        return Err(CentralError::Store(
            "the namespace read answered the wrong number of results".into(),
        ));
    }
    let placements: std::collections::HashMap<String, Partition> = results
        .pop()
        .map(|rows| rows.rows)
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            (
                string(&row[0]),
                Partition::new(&string(&row[1]), &string(&row[2])),
            )
        })
        .collect();
    let rows = results.pop().map(|rows| rows.rows).unwrap_or_default();
    Ok(rows
        .into_iter()
        .map(|row| Namespace {
            registry_id: string(&row[0]),
            namespace: string(&row[1]),
            scope: if row[2].as_str() == Some("shared") {
                Scope::Shared(string(&row[3]))
            } else {
                Scope::Private
            },
            core_binding: row[4].as_str().map(str::to_string),
            global_flag: row[5].as_i64() == Some(1),
            blob_threshold: unsigned(&row[6]),
            policy_epoch: unsigned(&row[7]),
            placement: placements.get(&string(&row[1])).cloned(),
        })
        .collect())
}

/// The definition of `namespace` in `registry_id`, or `unknown_namespace`
/// naming the registry. Resolution never falls back to another registry.
pub fn resolve(primary: &dyn Pipeline, registry_id: &str, namespace: &str) -> Result<Namespace> {
    crate::validate_namespace(namespace)
        .map_err(|_| CentralError::Malformed("malformed_namespace"))?;
    namespaces(primary, registry_id)?
        .into_iter()
        .find(|definition| definition.namespace == namespace)
        .ok_or_else(|| CentralError::UnknownNamespace {
            registry_id: registry_id.to_string(),
            namespace: namespace.to_string(),
        })
}

pub(crate) fn validate(address: &Address) -> Result<()> {
    crate::validate_namespace(&address.namespace)
        .map_err(|_| CentralError::Malformed("malformed_namespace"))?;
    crate::validate_key(&address.key).map_err(|_| CentralError::Malformed("malformed_key"))?;
    if address.scope_id.is_empty() || address.registry_id.is_empty() {
        return Err(CentralError::Malformed("malformed_scope"));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::HashMap;

    fn key(writer: &str, outbox: &str) -> IdempotencyKey {
        IdempotencyKey {
            writer: writer.to_string(),
            host_incarnation: format!("{writer}-host"),
            outbox_id: outbox.to_string(),
        }
    }

    fn intent(address: &Address, expected: u64, op: Op, writer: &str, outbox: &str) -> Intent {
        Intent {
            address: address.clone(),
            expected_version: expected,
            op,
            idempotency: key(writer, outbox),
        }
    }

    fn set(value: &str) -> Op {
        Op::Set(value.as_bytes().to_vec())
    }

    fn status() -> Address {
        Address::new("shared:team", "status", "agents/one")
    }

    fn committed(receipt: &Receipt) -> u64 {
        match receipt.outcome {
            Outcome::Committed { version } => version,
            Outcome::Conflict { .. } => panic!("expected a commit, got {receipt:?}"),
        }
    }

    #[test]
    fn central_migrations_split_and_rerun() {
        let statements = migration_statements(MIGRATIONS[0].1);
        assert!(statements.len() >= 6, "{statements:?}");
        assert!(statements.iter().all(|sql| !sql.contains(';')));
        let primary = SqlitePrimary::in_memory().expect("primary");
        migrate(&primary).expect("second run is a no-op");
        let recorded = primary
            .transaction_unlisted(&[Statement::numbered(
                "SELECT COUNT(*) FROM kv_central_migrations",
                &[],
            )])
            .unwrap();
        assert_eq!(
            recorded[0].rows[0][0],
            Value::Integer(MIGRATIONS.len() as i64)
        );
    }

    #[test]
    fn central_parameter_counting() {
        assert_eq!(highest_parameter("SELECT ?1, ?13, ?2"), 13);
        assert_eq!(highest_parameter("SELECT 1"), 0);
        for statement in cas_statements(&intent(&status(), 0, set("x"), "a", "1"), "nonce") {
            assert_eq!(statement.args.len(), highest_parameter(&statement.sql));
        }
    }

    #[test]
    fn central_first_write_commits_version_one() {
        let primary = SqlitePrimary::in_memory().unwrap();
        let receipt = cas(&primary, &intent(&status(), 0, set("ready"), "a", "1")).unwrap();
        assert_eq!(
            receipt,
            Receipt {
                outcome: Outcome::Committed { version: 1 },
                replayed: false
            }
        );
        let head = head(&primary, &status()).unwrap().unwrap();
        assert_eq!(
            (head.version, head.live, head.value),
            (1, true, b"ready".to_vec())
        );
        assert_eq!(head.writer, "a");
        let history = history(&primary, &status()).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].parent_version, 0);
    }

    #[test]
    fn central_stale_write_is_a_retrievable_conflict() {
        let primary = SqlitePrimary::in_memory().unwrap();
        committed(&cas(&primary, &intent(&status(), 0, set("first"), "a", "1")).unwrap());
        let loser = cas(&primary, &intent(&status(), 0, set("second"), "b", "1")).unwrap();
        let Outcome::Conflict {
            expected,
            actual,
            conflict_id,
        } = loser.outcome
        else {
            panic!("expected a conflict, got {loser:?}");
        };
        assert_eq!((expected, actual), (0, 1));
        // Never last-writer-wins: the head is the winner's full value.
        assert_eq!(head(&primary, &status()).unwrap().unwrap().value, b"first");
        // Both versions are retrievable.
        let conflicts = conflicts(&primary, &status()).unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].id, conflict_id);
        assert_eq!(conflicts[0].proposed, set("second"));
        assert_eq!(history(&primary, &status()).unwrap().len(), 1);
    }

    #[test]
    fn central_retransmit_replays_even_after_the_head_moved() {
        let primary = SqlitePrimary::in_memory().unwrap();
        let first = intent(&status(), 0, set("one"), "a", "1");
        assert_eq!(committed(&cas(&primary, &first).unwrap()), 1);
        committed(&cas(&primary, &intent(&status(), 1, set("two"), "b", "7")).unwrap());
        let again = cas(&primary, &first).unwrap();
        assert_eq!(
            again,
            Receipt {
                outcome: Outcome::Committed { version: 1 },
                replayed: true
            }
        );
        assert_eq!(head(&primary, &status()).unwrap().unwrap().version, 2);
        assert!(conflicts(&primary, &status()).unwrap().is_empty());
        assert_eq!(history(&primary, &status()).unwrap().len(), 2);
    }

    #[test]
    fn central_conflict_replays_as_the_same_conflict() {
        let primary = SqlitePrimary::in_memory().unwrap();
        committed(&cas(&primary, &intent(&status(), 0, set("one"), "a", "1")).unwrap());
        let late = intent(&status(), 0, set("late"), "b", "1");
        let first = cas(&primary, &late).unwrap();
        let second = cas(&primary, &late).unwrap();
        assert!(!first.replayed);
        assert!(second.replayed);
        assert_eq!(first.outcome, second.outcome);
        assert_eq!(conflicts(&primary, &status()).unwrap().len(), 1);
    }

    #[test]
    fn central_same_id_different_body_is_refused() {
        let primary = SqlitePrimary::in_memory().unwrap();
        committed(&cas(&primary, &intent(&status(), 0, set("one"), "a", "1")).unwrap());
        let error = cas(&primary, &intent(&status(), 0, set("other"), "a", "1")).unwrap_err();
        assert_eq!(error.code(), "idempotency_key_reuse");
        assert_eq!(head(&primary, &status()).unwrap().unwrap().value, b"one");
    }

    #[test]
    fn central_scopes_are_isolated() {
        let primary = SqlitePrimary::in_memory().unwrap();
        let mine = Address::new(&private_scope_id("drew", "laptop"), "notes", "todo");
        let theirs = Address::new(&private_scope_id("felix", "laptop"), "notes", "todo");
        assert_ne!(mine.scope_id, theirs.scope_id);
        committed(&cas(&primary, &intent(&mine, 0, set("mine"), "drew", "1")).unwrap());
        assert_eq!(head(&primary, &theirs).unwrap(), None);
        committed(&cas(&primary, &intent(&theirs, 0, set("theirs"), "felix", "1")).unwrap());
        assert_eq!(head(&primary, &mine).unwrap().unwrap().value, b"mine");
        // A tombstone in one scope leaves the other scope's head alone.
        committed(&cas(&primary, &intent(&mine, 1, Op::Tombstone, "drew", "2")).unwrap());
        assert!(head(&primary, &theirs).unwrap().unwrap().live);
    }

    #[test]
    fn central_private_scope_ids_do_not_collide_by_concatenation() {
        assert_ne!(private_scope_id("ab", "c"), private_scope_id("a", "bc"));
        assert!(private_scope_id("a", "b").starts_with("private:"));
        assert!(private_scope_id("drew", "laptop").starts_with("private:64726577:"));
        assert_eq!(private_scope_prefix("drew"), "private:64726577:");
    }

    #[test]
    fn central_tombstone_sweeps_through_cas_and_never_resurrects_on_replay() {
        let primary = SqlitePrimary::in_memory().unwrap();
        let write = intent(&status(), 0, set("alive"), "a", "1");
        committed(&cas(&primary, &write).unwrap());
        let mask = intent(&status(), 1, Op::Tombstone, "b", "1");
        assert_eq!(committed(&cas(&primary, &mask).unwrap()), 2);
        // Replaying the original write must not bring the value back.
        assert!(cas(&primary, &write).unwrap().replayed);
        let current = head(&primary, &status()).unwrap().unwrap();
        assert_eq!((current.version, current.live), (2, false));
        // A delete racing a write: the stale delete is a conflict.
        let stale_mask = intent(&status(), 1, Op::Tombstone, "c", "1");
        assert!(matches!(
            cas(&primary, &stale_mask).unwrap().outcome,
            Outcome::Conflict { actual: 2, .. }
        ));
    }

    #[test]
    fn central_malformed_addresses_are_typed() {
        let primary = SqlitePrimary::in_memory().unwrap();
        let bad_key = Address::new("shared:team", "status", "/leading");
        let bad_ns = Address::new("shared:team", "9status", "k");
        assert_eq!(
            cas(&primary, &intent(&bad_key, 0, set("x"), "a", "1"))
                .unwrap_err()
                .code(),
            "malformed_key"
        );
        assert_eq!(
            cas(&primary, &intent(&bad_ns, 0, set("x"), "a", "1"))
                .unwrap_err()
                .code(),
            "malformed_namespace"
        );
    }

    #[test]
    fn central_namespace_placement_is_stored_read_and_dropped() {
        let primary = SqlitePrimary::in_memory().unwrap();
        let mut status = Namespace::shared("status", "shared:team");
        assert_eq!(resolve_after(&primary, &status).placement, None);
        status.placement = Some(Partition::new("eu", "standard"));
        assert_eq!(
            resolve_after(&primary, &status).placement,
            Some(Partition::new("eu", "standard"))
        );
        define_namespace(&primary, &Namespace::private("notes")).unwrap();
        let listed = namespaces(&primary, REGISTRY_ID).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].placement, None);
        assert_eq!(listed[1].placement, Some(Partition::new("eu", "standard")));
        // Moving or dropping the placement is a redefinition like any other.
        status.placement = Some(Partition::new("us", "cold"));
        let moved = resolve_after(&primary, &status);
        assert_eq!(moved.placement, Some(Partition::new("us", "cold")));
        assert_eq!(moved.policy_epoch, 3);
        status.placement = None;
        assert_eq!(resolve_after(&primary, &status).placement, None);
        // A malformed label is refused before anything is written.
        status.placement = Some(Partition::new("EU", "standard"));
        assert_eq!(
            define_namespace(&primary, &status).unwrap_err().code(),
            "malformed_partition"
        );
        assert_eq!(
            resolve(&primary, REGISTRY_ID, "status")
                .unwrap()
                .policy_epoch,
            4
        );
    }

    fn resolve_after(primary: &SqlitePrimary, definition: &Namespace) -> Namespace {
        define_namespace(primary, definition).unwrap();
        resolve(primary, REGISTRY_ID, &definition.namespace).unwrap()
    }

    #[test]
    fn central_namespaces_resolve_in_their_registry_only() {
        let primary = SqlitePrimary::in_memory().unwrap();
        assert_eq!(
            define_namespace(&primary, &Namespace::shared("status", "shared:team")).unwrap(),
            1
        );
        assert_eq!(
            define_namespace(&primary, &Namespace::private("notes")).unwrap(),
            1
        );
        // A redefinition bumps the epoch.
        let mut wider = Namespace::shared("status", "shared:team");
        wider.global_flag = true;
        assert_eq!(define_namespace(&primary, &wider).unwrap(), 2);
        let status = resolve(&primary, REGISTRY_ID, "status").unwrap();
        assert!(status.global_flag);
        assert_eq!(status.scope_for("drew", "laptop"), "shared:team");
        let notes = resolve(&primary, REGISTRY_ID, "notes").unwrap();
        assert_eq!(
            notes.scope_for("drew", "laptop"),
            private_scope_id("drew", "laptop")
        );
        let missing = resolve(&primary, REGISTRY_ID, "absent").unwrap_err();
        assert_eq!(missing.code(), "unknown_namespace");
        assert!(missing.to_string().contains(REGISTRY_ID));
        // No fallback into another registry.
        assert_eq!(
            resolve(&primary, "server:host-a", "status")
                .unwrap_err()
                .code(),
            "unknown_namespace"
        );
        assert_eq!(namespaces(&primary, REGISTRY_ID).unwrap().len(), 2);
    }

    /// A primary that loses the first `failures` answers after committing.
    struct LossyPrimary {
        inner: SqlitePrimary,
        failures: Mutex<u32>,
    }

    impl Pipeline for LossyPrimary {
        fn transaction(&self, statements: &[Statement]) -> Result<Vec<Rows>> {
            let result = self.inner.transaction(statements)?;
            let mut failures = self.failures.lock().unwrap();
            if *failures > 0 {
                *failures -= 1;
                return Err(CentralError::Unavailable("answer lost".into()));
            }
            Ok(result)
        }
    }

    #[test]
    fn central_lost_answer_retries_into_the_recorded_result() {
        let primary = LossyPrimary {
            inner: SqlitePrimary::in_memory().unwrap(),
            failures: Mutex::new(2),
        };
        let mut sleeps = Vec::new();
        let receipt = cas_with_retry(
            &primary,
            &intent(&status(), 0, set("once"), "a", "1"),
            &mut |delay| sleeps.push(delay),
        )
        .unwrap();
        assert_eq!(receipt.outcome, Outcome::Committed { version: 1 });
        assert!(receipt.replayed, "the first transmission committed");
        assert_eq!(sleeps.len(), 2);
        assert!(sleeps.iter().all(|delay| *delay <= BACKOFF_CAP));
        assert!(conflicts(&primary.inner, &status()).unwrap().is_empty());
        assert_eq!(history(&primary.inner, &status()).unwrap().len(), 1);
    }

    #[test]
    fn central_retries_stop_after_five_attempts() {
        let primary = LossyPrimary {
            inner: SqlitePrimary::in_memory().unwrap(),
            failures: Mutex::new(100),
        };
        let mut sleeps = 0;
        let error = cas_with_retry(
            &primary,
            &intent(&status(), 0, set("x"), "a", "1"),
            &mut |_| sleeps += 1,
        )
        .unwrap_err();
        assert_eq!(error.code(), "unavailable");
        assert_eq!(sleeps, MAX_ATTEMPTS - 1);
    }

    #[test]
    fn central_backoff_is_capped() {
        let mut jitter = Jitter::seeded("seed");
        for attempt in 1..=20 {
            assert!(jitter.backoff(attempt) <= BACKOFF_CAP);
        }
    }

    #[test]
    fn central_concurrent_writers_serialize_per_key() {
        let path = std::env::temp_dir().join(format!(
            "fleet-kv-central-{}-{}.db",
            std::process::id(),
            attempt_nonce()
        ));
        SqlitePrimary::open(&path).unwrap();
        let address = status();
        let writers = 5;
        let commits_each = 8;
        let handles: Vec<_> = (0..writers)
            .map(|writer| {
                let path = path.clone();
                let address = address.clone();
                std::thread::spawn(move || {
                    let primary = SqlitePrimary::open(&path).unwrap();
                    let mut commits = 0;
                    let mut outbox: u64 = 0;
                    while commits < commits_each {
                        let expected = head(&primary, &address).unwrap().map_or(0, |h| h.version);
                        outbox += 1;
                        let write = intent(
                            &address,
                            expected,
                            set(&format!("w{writer}-{outbox}")),
                            &format!("w{writer}"),
                            &outbox.to_string(),
                        );
                        if let Outcome::Committed { .. } = cas(&primary, &write).unwrap().outcome {
                            commits += 1;
                        }
                    }
                    outbox
                })
            })
            .collect();
        let attempts: u64 = handles.into_iter().map(|h| h.join().unwrap()).sum();
        let primary = SqlitePrimary::open(&path).unwrap();
        let history = history(&primary, &address).unwrap();
        let total = (writers * commits_each) as u64;
        assert_eq!(history.len() as u64, total);
        for (index, version) in history.iter().enumerate() {
            assert_eq!(version.version, index as u64 + 1);
            assert_eq!(version.parent_version, index as u64);
        }
        assert_eq!(head(&primary, &address).unwrap().unwrap().version, total);
        assert_eq!(
            conflicts(&primary, &address).unwrap().len() as u64,
            attempts - total
        );
        drop(primary);
        let _ = std::fs::remove_file(&path);
    }

    /// One step of the model: a writer proposes a body against the version
    /// it last observed, possibly stale, possibly a retransmit.
    #[derive(Clone, Debug)]
    enum Step {
        Write {
            writer: u8,
            staleness: u8,
            tombstone: bool,
        },
        Retransmit {
            index: usize,
        },
    }

    fn step() -> impl Strategy<Value = Step> {
        prop_oneof![
            3 => (0u8..5, 0u8..3, any::<bool>()).prop_map(|(writer, staleness, tombstone)| Step::Write {
                writer,
                staleness,
                tombstone,
            }),
            1 => (0usize..64).prop_map(|index| Step::Retransmit { index }),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]

        /// CAS linearity and sweep idempotence against a sequential oracle:
        /// the head always equals one writer's full intent, every loser gets
        /// a typed conflict naming the head it lost to, and retransmits
        /// change nothing.
        #[test]
        fn central_cas_matches_a_sequential_oracle(steps in prop::collection::vec(step(), 1..40)) {
            let primary = SqlitePrimary::in_memory().unwrap();
            let address = status();
            // Oracle: the head version and body, and every decided request.
            let mut version = 0u64;
            let mut body: Option<Op> = None;
            let mut decided: Vec<(Intent, Outcome)> = Vec::new();
            let mut counters: HashMap<u8, u32> = HashMap::new();
            for step in steps {
                match step {
                    Step::Write { writer, staleness, tombstone } => {
                        let outbox = counters.entry(writer).or_default();
                        *outbox += 1;
                        let expected = version.saturating_sub(u64::from(staleness));
                        let op = if tombstone { Op::Tombstone } else {
                            Op::Set(format!("w{writer}-{outbox}").into_bytes())
                        };
                        let write = intent(&address, expected, op.clone(), &format!("w{writer}"), &outbox.to_string());
                        let receipt = cas(&primary, &write).unwrap();
                        prop_assert!(!receipt.replayed);
                        if expected == version {
                            version += 1;
                            body = Some(op);
                            prop_assert_eq!(&receipt.outcome, &Outcome::Committed { version });
                        } else {
                            let Outcome::Conflict { expected: e, actual, .. } = receipt.outcome.clone() else {
                                return Err(TestCaseError::fail("expected a conflict"));
                            };
                            prop_assert_eq!((e, actual), (expected, version));
                        }
                        decided.push((write, receipt.outcome));
                    }
                    Step::Retransmit { index } => {
                        if decided.is_empty() { continue; }
                        let (write, outcome) = decided[index % decided.len()].clone();
                        let receipt = cas(&primary, &write).unwrap();
                        prop_assert!(receipt.replayed);
                        prop_assert_eq!(receipt.outcome, outcome);
                    }
                }
                let current = head(&primary, &address).unwrap();
                match &body {
                    None => prop_assert_eq!(current, None),
                    Some(op) => {
                        let current = current.unwrap();
                        prop_assert_eq!(current.version, version);
                        prop_assert_eq!(current.live, matches!(op, Op::Set(_)));
                        prop_assert_eq!(&current.value, &op.value().to_vec());
                    }
                }
            }
            let commits = decided.iter().filter(|(_, o)| matches!(o, Outcome::Committed { .. })).count();
            prop_assert_eq!(history(&primary, &address).unwrap().len(), commits);
            prop_assert_eq!(conflicts(&primary, &address).unwrap().len(), decided.len() - commits);
        }
    }
}
