//! Identity-scoped local storage primitives for the `fleet kv` domain.
//!
//! The local store keeps three kinds of state for every complete identity
//! tuple `(owner_user_id, context_id, partition_id, namespace, key)`:
//!
//! - the **overlay** (`kv_local`): the current local head, either a live value
//!   or a masking tombstone;
//! - the **version history** (`kv_versions_local`): every local version,
//!   append-only;
//! - the **outbox** (`kv_outbox`): unsent intents for the disconnect sweep,
//!   fenced by a per-tuple generation (`kv_fence`).
//!
//! `clear-override`, masking `delete` and history `revert` are the three
//! delete-related states from the fleet-kv design (D005) and stay distinct.

pub mod cli;
pub mod launch;

use rusqlite::{Connection, ErrorCode, OptionalExtension, Transaction, params};

const SCHEMA_VERSION: i64 = 2;
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Debug)]
pub enum KvError {
    MalformedKey,
    MalformedNamespace,
    /// `revert --to` named a version that is not in this tuple's history.
    UnknownVersion,
    /// `revert --to` named a masking tombstone; `delete` is the masking verb.
    RevertTargetIsTombstone,
    /// `clear-override` could not take the per-key lock within the bounded
    /// wait; the fence was not touched.
    ClearBusyTryAgain,
    Sql(rusqlite::Error),
}

impl KvError {
    /// Stable typed refusal code printed by the CLI.
    pub fn code(&self) -> &'static str {
        match self {
            Self::MalformedKey => "malformed_key",
            Self::MalformedNamespace => "malformed_namespace",
            Self::UnknownVersion => "unknown_version",
            Self::RevertTargetIsTombstone => "revert_target_is_tombstone",
            Self::ClearBusyTryAgain => "clear_busy_try_again",
            Self::Sql(_) => "store_error",
        }
    }
}

impl std::fmt::Display for KvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MalformedKey => f.write_str(
                "keys are 1-512 bytes of printable ASCII; `/` separates non-empty segments",
            ),
            Self::MalformedNamespace => f.write_str(
                "namespaces start with a letter, then letters, digits or hyphens, at most 63 bytes",
            ),
            Self::UnknownVersion => f.write_str("no such version in this key's local history"),
            Self::RevertTargetIsTombstone => {
                f.write_str("that version is a masking tombstone; use `delete` to mask the key")
            }
            Self::ClearBusyTryAgain => {
                f.write_str("another writer holds this key's lock; nothing was changed, try again")
            }
            Self::Sql(error) => write!(f, "local store error: {error}"),
        }
    }
}

impl std::error::Error for KvError {}

fn sql(error: rusqlite::Error) -> KvError {
    KvError::Sql(error)
}

pub struct Identity<'a> {
    owner_user_id: &'a str,
    context_id: &'a str,
    partition_id: &'a str,
}

impl<'a> Identity<'a> {
    pub fn new(owner_user_id: &'a str, context_id: &'a str, partition_id: &'a str) -> Self {
        Self {
            owner_user_id,
            context_id,
            partition_id,
        }
    }
}

/// Where a listed row came from. Phase 1 has no central plane, so every row
/// is local; a masking tombstone lists as `Masked`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Local,
    Masked,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Masked => "masked",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListEntry {
    pub key: String,
    pub version: u64,
    pub origin: Origin,
}

/// One sweep intent in the local outbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxIntent {
    pub id: i64,
    pub key: String,
    pub version: u64,
    pub op: String,
    pub generation: u64,
    pub status: String,
}

pub struct LocalStore {
    connection: Connection,
}

impl LocalStore {
    pub fn in_memory() -> Result<Self, KvError> {
        let connection = Connection::open_in_memory().map_err(sql)?;
        Self::from_connection(connection)
    }

    /// Open (or create) the durable host-local SQLite database.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, KvError> {
        let connection = Connection::open(path).map_err(sql)?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(sql)?;
        Self::from_connection(connection)
    }

    fn from_connection(connection: Connection) -> Result<Self, KvError> {
        connection.busy_timeout(BUSY_TIMEOUT).map_err(sql)?;
        connection
            .execute_batch(
                "PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS kv_local (
                 owner_user_id TEXT NOT NULL,
                 context_id TEXT NOT NULL,
                 partition_id TEXT NOT NULL,
                 namespace TEXT NOT NULL,
                 key TEXT NOT NULL,
                 value BLOB NOT NULL,
                 PRIMARY KEY(owner_user_id, context_id, partition_id, namespace, key)
             ) WITHOUT ROWID;",
            )
            .map_err(sql)?;
        migrate(&connection)?;
        Ok(Self { connection })
    }

    /// Store arbitrary bytes under the caller's complete identity tuple.
    /// Appends a version, moves the overlay to it and enqueues a sweep intent,
    /// all in one transaction. Returns the new version number.
    pub fn set(
        &self,
        identity: &Identity<'_>,
        namespace: &str,
        key: &str,
        value: &[u8],
    ) -> Result<u64, KvError> {
        validate_namespace(namespace)?;
        validate_key(key)?;
        let tx = self.connection.unchecked_transaction().map_err(sql)?;
        let version = append_head(&tx, identity, namespace, key, value, STATE_LIVE, "set")?;
        tx.commit().map_err(sql)?;
        Ok(version)
    }

    /// Read only a row owned by the caller's complete identity tuple. A
    /// masking tombstone reads as absent.
    pub fn get(
        &self,
        identity: &Identity<'_>,
        namespace: &str,
        key: &str,
    ) -> Result<Option<Vec<u8>>, KvError> {
        validate_namespace(namespace)?;
        validate_key(key)?;
        self.connection
            .query_row(
                "SELECT value FROM kv_local
             WHERE owner_user_id=?1 AND context_id=?2 AND partition_id=?3 AND namespace=?4 AND key=?5
               AND state='live'",
                params![
                    identity.owner_user_id,
                    identity.context_id,
                    identity.partition_id,
                    namespace,
                    key
                ],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql)
    }

    /// List the caller's keys in one namespace, sorted by key. Masking
    /// tombstones are hidden unless `include_tombstones` is set.
    pub fn list(
        &self,
        identity: &Identity<'_>,
        namespace: &str,
        include_tombstones: bool,
    ) -> Result<Vec<ListEntry>, KvError> {
        validate_namespace(namespace)?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT key, version, state FROM kv_local
             WHERE owner_user_id=?1 AND context_id=?2 AND partition_id=?3 AND namespace=?4
               AND (?5 OR state='live')
             ORDER BY key",
            )
            .map_err(sql)?;
        let rows = statement
            .query_map(
                params![
                    identity.owner_user_id,
                    identity.context_id,
                    identity.partition_id,
                    namespace,
                    include_tombstones
                ],
                |row| {
                    let state: String = row.get(2)?;
                    Ok(ListEntry {
                        key: row.get(0)?,
                        version: unsigned(row.get(1)?),
                        origin: if state == STATE_TOMBSTONE {
                            Origin::Masked
                        } else {
                            Origin::Local
                        },
                    })
                },
            )
            .map_err(sql)?;
        rows.collect::<Result<_, _>>().map_err(sql)
    }

    /// Write a masking tombstone: the key reads absent locally and the
    /// tombstone sweeps as a mask. Returns the tombstone's version number.
    pub fn delete(
        &self,
        identity: &Identity<'_>,
        namespace: &str,
        key: &str,
    ) -> Result<u64, KvError> {
        validate_namespace(namespace)?;
        validate_key(key)?;
        let tx = self.connection.unchecked_transaction().map_err(sql)?;
        let version = append_head(
            &tx,
            identity,
            namespace,
            key,
            &[],
            STATE_TOMBSTONE,
            "tombstone",
        )?;
        tx.commit().map_err(sql)?;
        Ok(version)
    }

    /// Drop the local overlay row, advance the tuple's generation fence and
    /// cancel every unsent intent for it. History is kept. Returns whether an
    /// overlay row existed.
    ///
    /// The clear takes the database write lock (shared with any sweeper's
    /// submission path) with a bounded 5 s wait; on timeout it returns
    /// [`KvError::ClearBusyTryAgain`] without touching the fence.
    pub fn clear_override(
        &self,
        identity: &Identity<'_>,
        namespace: &str,
        key: &str,
    ) -> Result<bool, KvError> {
        validate_namespace(namespace)?;
        validate_key(key)?;
        let tx =
            Transaction::new_unchecked(&self.connection, rusqlite::TransactionBehavior::Immediate)
                .map_err(busy_or_sql)?;
        let tuple = params![
            identity.owner_user_id,
            identity.context_id,
            identity.partition_id,
            namespace,
            key
        ];
        let removed = tx
            .execute(
                "DELETE FROM kv_local
             WHERE owner_user_id=?1 AND context_id=?2 AND partition_id=?3 AND namespace=?4 AND key=?5",
                tuple,
            )
            .map_err(busy_or_sql)?;
        tx.execute(
            "INSERT INTO kv_fence(owner_user_id,context_id,partition_id,namespace,key,generation)
             VALUES (?1,?2,?3,?4,?5,1)
             ON CONFLICT(owner_user_id,context_id,partition_id,namespace,key)
             DO UPDATE SET generation=generation+1",
            tuple,
        )
        .map_err(busy_or_sql)?;
        tx.execute(
            "UPDATE kv_outbox SET status='cancelled'
             WHERE owner_user_id=?1 AND context_id=?2 AND partition_id=?3 AND namespace=?4 AND key=?5
               AND status='pending'",
            tuple,
        )
        .map_err(busy_or_sql)?;
        tx.commit().map_err(busy_or_sql)?;
        Ok(removed > 0)
    }

    /// Append a new version whose value equals version `to` of this tuple's
    /// history, and make it the local head. Returns the new version number.
    pub fn revert(
        &self,
        identity: &Identity<'_>,
        namespace: &str,
        key: &str,
        to: u64,
    ) -> Result<u64, KvError> {
        validate_namespace(namespace)?;
        validate_key(key)?;
        let tx = self.connection.unchecked_transaction().map_err(sql)?;
        let target: Option<(Vec<u8>, String)> = tx
            .query_row(
                "SELECT value, state FROM kv_versions_local
             WHERE owner_user_id=?1 AND context_id=?2 AND partition_id=?3 AND namespace=?4 AND key=?5
               AND version=?6",
                params![
                    identity.owner_user_id,
                    identity.context_id,
                    identity.partition_id,
                    namespace,
                    key,
                    signed(to)
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(sql)?;
        let Some((value, state)) = target else {
            return Err(KvError::UnknownVersion);
        };
        if state == STATE_TOMBSTONE {
            return Err(KvError::RevertTargetIsTombstone);
        }
        let version = append_head(&tx, identity, namespace, key, &value, STATE_LIVE, "revert")?;
        tx.commit().map_err(sql)?;
        Ok(version)
    }

    /// The caller's sweep intents for one key, oldest first.
    pub fn outbox(
        &self,
        identity: &Identity<'_>,
        namespace: &str,
        key: &str,
    ) -> Result<Vec<OutboxIntent>, KvError> {
        validate_namespace(namespace)?;
        validate_key(key)?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, key, version, op, generation, status FROM kv_outbox
             WHERE owner_user_id=?1 AND context_id=?2 AND partition_id=?3 AND namespace=?4 AND key=?5
             ORDER BY id",
            )
            .map_err(sql)?;
        let rows = statement
            .query_map(
                params![
                    identity.owner_user_id,
                    identity.context_id,
                    identity.partition_id,
                    namespace,
                    key
                ],
                |row| {
                    Ok(OutboxIntent {
                        id: row.get(0)?,
                        key: row.get(1)?,
                        version: unsigned(row.get(2)?),
                        op: row.get(3)?,
                        generation: unsigned(row.get(4)?),
                        status: row.get(5)?,
                    })
                },
            )
            .map_err(sql)?;
        rows.collect::<Result<_, _>>().map_err(sql)
    }
}

const STATE_LIVE: &str = "live";
const STATE_TOMBSTONE: &str = "tombstone";

fn busy_or_sql(error: rusqlite::Error) -> KvError {
    match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => KvError::ClearBusyTryAgain,
        _ => KvError::Sql(error),
    }
}

/// Schema migration keyed on `PRAGMA user_version`. Version 1 is the
/// original value-only `kv_local`; version 2 adds versions, tombstones, the
/// outbox and the generation fence. Existing overlay rows become version 1.
fn migrate(connection: &Connection) -> Result<(), KvError> {
    let current: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(sql)?;
    if current >= SCHEMA_VERSION {
        return Ok(());
    }
    let tx = connection.unchecked_transaction().map_err(sql)?;
    let has_version: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('kv_local') WHERE name='version'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(sql)?
        > 0;
    if !has_version {
        tx.execute_batch(
            "ALTER TABLE kv_local ADD COLUMN version INTEGER NOT NULL DEFAULT 1;
             ALTER TABLE kv_local ADD COLUMN state TEXT NOT NULL DEFAULT 'live';",
        )
        .map_err(sql)?;
    }
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS kv_versions_local (
             owner_user_id TEXT NOT NULL,
             context_id TEXT NOT NULL,
             partition_id TEXT NOT NULL,
             namespace TEXT NOT NULL,
             key TEXT NOT NULL,
             version INTEGER NOT NULL,
             parent_version INTEGER,
             value BLOB NOT NULL,
             state TEXT NOT NULL CHECK(state IN ('live','tombstone')),
             op TEXT NOT NULL,
             written_at_ms INTEGER NOT NULL,
             PRIMARY KEY(owner_user_id, context_id, partition_id, namespace, key, version)
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS kv_fence (
             owner_user_id TEXT NOT NULL,
             context_id TEXT NOT NULL,
             partition_id TEXT NOT NULL,
             namespace TEXT NOT NULL,
             key TEXT NOT NULL,
             generation INTEGER NOT NULL,
             PRIMARY KEY(owner_user_id, context_id, partition_id, namespace, key)
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS kv_outbox (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             owner_user_id TEXT NOT NULL,
             context_id TEXT NOT NULL,
             partition_id TEXT NOT NULL,
             namespace TEXT NOT NULL,
             key TEXT NOT NULL,
             version INTEGER NOT NULL,
             op TEXT NOT NULL,
             generation INTEGER NOT NULL,
             status TEXT NOT NULL CHECK(status IN ('pending','cancelled','acked'))
         );
         CREATE INDEX IF NOT EXISTS kv_outbox_pending
             ON kv_outbox(owner_user_id, context_id, partition_id, namespace, key, status);
         INSERT OR IGNORE INTO kv_versions_local(
             owner_user_id, context_id, partition_id, namespace, key,
             version, parent_version, value, state, op, written_at_ms)
         SELECT owner_user_id, context_id, partition_id, namespace, key,
                version, NULL, value, state, 'set', 0
         FROM kv_local;",
    )
    .map_err(sql)?;
    tx.pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(sql)?;
    tx.commit().map_err(sql)
}

/// Append the next version for the tuple, point the overlay at it and
/// enqueue a pending sweep intent under the tuple's current generation.
fn append_head(
    tx: &Transaction<'_>,
    identity: &Identity<'_>,
    namespace: &str,
    key: &str,
    value: &[u8],
    state: &str,
    op: &str,
) -> Result<u64, KvError> {
    let tuple = params![
        identity.owner_user_id,
        identity.context_id,
        identity.partition_id,
        namespace,
        key
    ];
    let parent: Option<i64> = tx
        .query_row(
            "SELECT MAX(version) FROM kv_versions_local
             WHERE owner_user_id=?1 AND context_id=?2 AND partition_id=?3 AND namespace=?4 AND key=?5",
            tuple,
            |row| row.get(0),
        )
        .map_err(sql)?;
    let version = parent.map_or(1, |parent| parent + 1);
    let generation: i64 = tx
        .query_row(
            "SELECT generation FROM kv_fence
             WHERE owner_user_id=?1 AND context_id=?2 AND partition_id=?3 AND namespace=?4 AND key=?5",
            tuple,
            |row| row.get(0),
        )
        .optional()
        .map_err(sql)?
        .unwrap_or(0);
    let written_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        });
    let ids = (
        identity.owner_user_id,
        identity.context_id,
        identity.partition_id,
    );
    tx.execute(
        "INSERT INTO kv_versions_local(
             owner_user_id, context_id, partition_id, namespace, key,
             version, parent_version, value, state, op, written_at_ms)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            ids.0,
            ids.1,
            ids.2,
            namespace,
            key,
            version,
            parent,
            value,
            state,
            op,
            written_at_ms
        ],
    )
    .map_err(sql)?;
    tx.execute(
        "INSERT INTO kv_local(owner_user_id,context_id,partition_id,namespace,key,value,version,state)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
         ON CONFLICT(owner_user_id,context_id,partition_id,namespace,key)
         DO UPDATE SET value=excluded.value, version=excluded.version, state=excluded.state",
        params![ids.0, ids.1, ids.2, namespace, key, value, version, state],
    )
    .map_err(sql)?;
    tx.execute(
        "INSERT INTO kv_outbox(owner_user_id,context_id,partition_id,namespace,key,version,op,generation,status)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'pending')",
        params![ids.0, ids.1, ids.2, namespace, key, version, op, generation],
    )
    .map_err(sql)?;
    Ok(unsigned(version))
}

/// Versions and generations are stored as SQLite integers and are never
/// negative; these keep the public API in `u64` without a lossy cast.
fn unsigned(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn signed(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn validate_namespace(namespace: &str) -> Result<(), KvError> {
    let mut bytes = namespace.bytes();
    let valid = namespace.len() <= 63
        && bytes
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    if valid {
        Ok(())
    } else {
        Err(KvError::MalformedNamespace)
    }
}

fn validate_key(key: &str) -> Result<(), KvError> {
    // The design's "Key syntax and limits" contract permits printable ASCII
    // (0x20-0x7E, space included), not just graphic characters.
    if key.is_empty()
        || key.len() > 512
        || key.starts_with('/')
        || key.ends_with('/')
        || !key.is_ascii()
        || key.split('/').any(|segment| {
            segment.is_empty() || !segment.bytes().all(|b| b.is_ascii_graphic() || b == b' ')
        })
    {
        return Err(KvError::MalformedKey);
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn reads_are_scoped_by_the_complete_identity_tuple() {
        let store = LocalStore::in_memory().expect("store opens");
        let first = Identity::new("owner-a", "context-a", "partition-a");
        let second = Identity::new("owner-b", "context-a", "partition-a");
        store
            .set(&first, "notes", "key", b"first")
            .expect("set first");
        store
            .set(&second, "notes", "key", b"second")
            .expect("set second");

        assert_eq!(
            store.get(&first, "notes", "key").expect("get first"),
            Some(b"first".to_vec())
        );
        assert_eq!(
            store.get(&second, "notes", "key").expect("get second"),
            Some(b"second".to_vec())
        );
        assert_eq!(
            store
                .get(
                    &Identity::new("owner-a", "context-b", "partition-a"),
                    "notes",
                    "key"
                )
                .expect("cross-context read"),
            None,
        );
        assert_eq!(
            store
                .get(
                    &Identity::new("owner-a", "context-a", "partition-b"),
                    "notes",
                    "key"
                )
                .expect("cross-partition read"),
            None,
        );
    }

    #[test]
    fn malformed_keys_are_typed_refusals() {
        assert!(matches!(validate_key("a//b"), Err(KvError::MalformedKey)));
        assert!(matches!(
            validate_key("/leading"),
            Err(KvError::MalformedKey)
        ));
        assert!(matches!(
            validate_key("trailing/"),
            Err(KvError::MalformedKey)
        ));
        assert!(validate_key("repo/status").is_ok());
    }

    #[test]
    fn keys_with_spaces_round_trip() {
        let store = LocalStore::in_memory().expect("store opens");
        let identity = Identity::new("owner", "context", "partition");
        store
            .set(&identity, "notes", "notes/meeting agenda", b"space key")
            .expect("set with space");
        assert_eq!(
            store
                .get(&identity, "notes", "notes/meeting agenda")
                .expect("get with space"),
            Some(b"space key".to_vec())
        );
    }

    #[test]
    fn key_length_boundaries_match_the_oracle() {
        assert!(matches!(validate_key(""), Err(KvError::MalformedKey)));
        assert!(validate_key(&"k".repeat(512)).is_ok());
        assert!(matches!(
            validate_key(&"k".repeat(513)),
            Err(KvError::MalformedKey)
        ));
    }

    #[test]
    fn namespace_grammar_boundaries() {
        assert!(validate_namespace("a").is_ok());
        assert!(validate_namespace("n-0Z").is_ok());
        assert!(validate_namespace(&format!("a{}", "b".repeat(62))).is_ok());
        assert!(matches!(
            validate_namespace(&format!("a{}", "b".repeat(63))),
            Err(KvError::MalformedNamespace)
        ));
        assert!(matches!(
            validate_namespace("0abc"),
            Err(KvError::MalformedNamespace)
        ));
        assert!(matches!(
            validate_namespace("has space"),
            Err(KvError::MalformedNamespace)
        ));
        assert!(matches!(
            validate_namespace("under_score"),
            Err(KvError::MalformedNamespace)
        ));
        assert!(matches!(
            validate_namespace(""),
            Err(KvError::MalformedNamespace)
        ));
    }

    /// Independent key-grammar oracle transcribed from the fleet-kv design's
    /// "Key syntax and limits" contract, not from `validate_key`.
    fn oracle_key_accepts(key: &str) -> bool {
        let bytes = key.as_bytes();
        if bytes.is_empty() || bytes.len() > 512 {
            return false;
        }
        if !bytes.iter().all(|b| (0x20..=0x7E).contains(b)) {
            return false;
        }
        if key.starts_with('/') || key.ends_with('/') {
            return false;
        }
        key.split('/').all(|segment| !segment.is_empty())
    }

    /// Independent namespace-grammar oracle transcribed from the `NamespaceId`
    /// rule (letter first, then letters/digits/hyphens, at most 63 bytes).
    fn oracle_namespace_accepts(namespace: &str) -> bool {
        let bytes = namespace.as_bytes();
        if bytes.is_empty() || bytes.len() > 63 {
            return false;
        }
        let mut cursor = bytes.iter();
        if !cursor
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic())
        {
            return false;
        }
        cursor.all(|b| b.is_ascii_alphanumeric() || *b == b'-')
    }

    fn key_char() -> impl Strategy<Value = char> {
        prop_oneof![
            10 => (0x20u8..0x7Fu8).prop_map(|b| b as char),
            2 => Just('/'),
            2 => (0x00u8..0x20u8).prop_map(|b| b as char),
            1 => Just('\u{7f}'),
            1 => prop_oneof![Just('é'), Just('中'), Just('💾')],
        ]
    }

    fn generated_key() -> impl Strategy<Value = String> {
        prop_oneof![
            3 => proptest::collection::vec(key_char(), 0..600)
                .prop_map(|chars| chars.into_iter().collect()),
            1 => proptest::collection::vec(key_char(), 505..520)
                .prop_map(|chars| chars.into_iter().collect()),
        ]
    }

    fn namespace_char() -> impl Strategy<Value = char> {
        prop_oneof![
            8 => prop_oneof![
                (0u8..26u8).prop_map(|b| (b'a' + b) as char),
                (0u8..26u8).prop_map(|b| (b'A' + b) as char),
                (0u8..10u8).prop_map(|b| (b'0' + b) as char),
                Just('-'),
            ],
            1 => Just(' '),
            1 => prop_oneof![Just('_'), Just('.'), Just('/'), Just('é'), Just('\u{7f}')],
        ]
    }

    fn generated_namespace() -> impl Strategy<Value = String> {
        prop_oneof![
            3 => proptest::collection::vec(namespace_char(), 0..70)
                .prop_map(|chars| chars.into_iter().collect()),
            1 => proptest::collection::vec(namespace_char(), 60..68)
                .prop_map(|chars| chars.into_iter().collect()),
        ]
    }

    #[test]
    fn file_backed_store_reopens_with_the_same_identity_scoped_value() {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("fleet-kv-{}-{suffix}.db", std::process::id()));
        let identity = Identity::new("owner", "context", "session");
        {
            let store = LocalStore::open(&path).expect("file store opens");
            store
                .set(&identity, "notes", "restore/check", b"durable")
                .expect("write");
        }
        {
            let store = LocalStore::open(&path).expect("file store reopens");
            assert_eq!(
                store
                    .get(&identity, "notes", "restore/check")
                    .expect("read after reopen"),
                Some(b"durable".to_vec())
            );
        }
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    proptest! {
        #[test]
        fn generated_identity_coordinates_never_cross_read(
            owner_a in "[a-z]{1,8}", owner_b in "[a-z]{1,8}",
            context_a in "[a-z]{1,8}", context_b in "[a-z]{1,8}",
            partition_a in "[a-z]{1,8}", partition_b in "[a-z]{1,8}",
        ) {
            let store = LocalStore::in_memory().expect("store opens");
            let owner = if owner_a == owner_b { "owner-other" } else { owner_b.as_str() };
            let context = if context_a == context_b { "context-other" } else { context_b.as_str() };
            let partition = if partition_a == partition_b { "partition-other" } else { partition_b.as_str() };
            let original = Identity::new(&owner_a, &context_a, &partition_a);
            let foreign_owner = Identity::new(owner, &context_a, &partition_a);
            let foreign_context = Identity::new(&owner_a, context, &partition_a);
            let foreign_partition = Identity::new(&owner_a, &context_a, partition);

            store.set(&original, "notes", "same/key", b"owned").expect("set");
            prop_assert_eq!(store.get(&original, "notes", "same/key").expect("own read"), Some(b"owned".to_vec()));
            prop_assert_eq!(store.get(&foreign_owner, "notes", "same/key").expect("foreign owner read"), None);
            prop_assert_eq!(store.get(&foreign_context, "notes", "same/key").expect("foreign context read"), None);
            prop_assert_eq!(store.get(&foreign_partition, "notes", "same/key").expect("foreign partition read"), None);
        }

        #[test]
        fn generated_keys_agree_with_the_grammar_oracle(key in generated_key()) {
            prop_assert_eq!(
                validate_key(&key).is_ok(),
                oracle_key_accepts(&key),
                "key {:?}",
                key
            );
        }

        #[test]
        fn generated_namespaces_agree_with_the_grammar_oracle(namespace in generated_namespace()) {
            prop_assert_eq!(
                validate_namespace(&namespace).is_ok(),
                oracle_namespace_accepts(&namespace),
                "namespace {:?}",
                namespace
            );
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod semantics_tests {
    use super::*;

    fn me() -> Identity<'static> {
        Identity::new("owner", "context", "partition")
    }

    #[test]
    fn versions_increase_and_tombstones_mask_reads() {
        let store = LocalStore::in_memory().expect("store");
        assert_eq!(store.set(&me(), "notes", "k", b"one").expect("set"), 1);
        assert_eq!(store.set(&me(), "notes", "k", b"two").expect("set"), 2);
        assert_eq!(store.delete(&me(), "notes", "k").expect("delete"), 3);
        assert_eq!(store.get(&me(), "notes", "k").expect("get"), None);
        assert_eq!(store.list(&me(), "notes", false).expect("list"), vec![]);
        assert_eq!(
            store.list(&me(), "notes", true).expect("list"),
            vec![ListEntry {
                key: "k".into(),
                version: 3,
                origin: Origin::Masked
            }]
        );
        assert_eq!(store.set(&me(), "notes", "k", b"back").expect("set"), 4);
        assert_eq!(
            store.get(&me(), "notes", "k").expect("get"),
            Some(b"back".to_vec())
        );
    }

    #[test]
    fn list_is_scoped_to_identity_and_namespace() {
        let store = LocalStore::in_memory().expect("store");
        store.set(&me(), "notes", "b", b"1").expect("set");
        store.set(&me(), "notes", "a", b"1").expect("set");
        store.set(&me(), "other", "c", b"1").expect("set");
        store
            .set(
                &Identity::new("someone", "context", "partition"),
                "notes",
                "z",
                b"1",
            )
            .expect("set");
        let keys: Vec<String> = store
            .list(&me(), "notes", true)
            .expect("list")
            .into_iter()
            .map(|entry| entry.key)
            .collect();
        assert_eq!(keys, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn clear_override_drops_overlay_fences_and_cancels_unsent_intents() {
        let store = LocalStore::in_memory().expect("store");
        store.set(&me(), "notes", "k", b"one").expect("set");
        store.delete(&me(), "notes", "k").expect("delete");
        assert!(store.clear_override(&me(), "notes", "k").expect("clear"));
        assert_eq!(store.get(&me(), "notes", "k").expect("get"), None);
        assert_eq!(store.list(&me(), "notes", true).expect("list"), vec![]);

        let intents = store.outbox(&me(), "notes", "k").expect("outbox");
        assert_eq!(intents.len(), 2);
        assert!(intents.iter().all(|intent| intent.status == "cancelled"));
        assert!(intents.iter().all(|intent| intent.generation == 0));

        // A write after the clear is a new intent under the advanced fence,
        // and history continues rather than restarting.
        assert_eq!(store.set(&me(), "notes", "k", b"after").expect("set"), 3);
        let intents = store.outbox(&me(), "notes", "k").expect("outbox");
        let last = intents.last().expect("new intent");
        assert_eq!(
            (last.status.as_str(), last.generation, last.version),
            ("pending", 1, 3)
        );
        assert!(!store.clear_override(&me(), "other", "k").expect("clear"));
    }

    #[test]
    fn revert_appends_a_copy_and_refuses_unknown_or_tombstone_targets() {
        let store = LocalStore::in_memory().expect("store");
        store.set(&me(), "notes", "k", b"one").expect("set");
        store.set(&me(), "notes", "k", b"two").expect("set");
        store.delete(&me(), "notes", "k").expect("delete");
        assert_eq!(store.revert(&me(), "notes", "k", 1).expect("revert"), 4);
        assert_eq!(
            store.get(&me(), "notes", "k").expect("get"),
            Some(b"one".to_vec())
        );
        assert!(matches!(
            store.revert(&me(), "notes", "k", 3),
            Err(KvError::RevertTargetIsTombstone)
        ));
        assert!(matches!(
            store.revert(&me(), "notes", "k", 99),
            Err(KvError::UnknownVersion)
        ));
        assert!(matches!(
            store.revert(
                &Identity::new("someone", "context", "partition"),
                "notes",
                "k",
                1
            ),
            Err(KvError::UnknownVersion)
        ));
        let ops: Vec<String> = store
            .outbox(&me(), "notes", "k")
            .expect("outbox")
            .into_iter()
            .map(|intent| intent.op)
            .collect();
        assert_eq!(ops, ["set", "set", "tombstone", "revert"]);
    }

    #[test]
    fn version_one_databases_migrate_in_place() {
        let connection = Connection::open_in_memory().expect("open");
        connection
            .execute_batch(
                "CREATE TABLE kv_local (
                     owner_user_id TEXT NOT NULL, context_id TEXT NOT NULL,
                     partition_id TEXT NOT NULL, namespace TEXT NOT NULL,
                     key TEXT NOT NULL, value BLOB NOT NULL,
                     PRIMARY KEY(owner_user_id, context_id, partition_id, namespace, key)
                 ) WITHOUT ROWID;
                 INSERT INTO kv_local VALUES ('owner','context','partition','notes','k',x'6f6c64');",
            )
            .expect("v1 schema");
        let store = LocalStore::from_connection(connection).expect("migrate");
        assert_eq!(
            store.get(&me(), "notes", "k").expect("get"),
            Some(b"old".to_vec())
        );
        assert_eq!(store.set(&me(), "notes", "k", b"new").expect("set"), 2);
        assert_eq!(store.revert(&me(), "notes", "k", 1).expect("revert"), 3);
        assert_eq!(
            store.get(&me(), "notes", "k").expect("get"),
            Some(b"old".to_vec())
        );
    }

    #[test]
    fn a_held_write_lock_makes_clear_override_busy_without_changes() {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("fleet-kv-busy-{}-{suffix}.db", std::process::id()));
        let store = LocalStore::open(&path).expect("open");
        store.set(&me(), "notes", "k", b"v").expect("set");
        store
            .connection
            .busy_timeout(std::time::Duration::from_millis(50))
            .expect("short timeout");
        let holder = Connection::open(&path).expect("second handle");
        holder.execute_batch("BEGIN IMMEDIATE;").expect("hold lock");
        assert!(matches!(
            store.clear_override(&me(), "notes", "k"),
            Err(KvError::ClearBusyTryAgain)
        ));
        holder.execute_batch("ROLLBACK;").expect("release");
        assert_eq!(
            store.get(&me(), "notes", "k").expect("get"),
            Some(b"v".to_vec())
        );
        assert!(
            store
                .outbox(&me(), "notes", "k")
                .expect("outbox")
                .iter()
                .all(|intent| intent.status == "pending")
        );
        drop(store);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }
}
