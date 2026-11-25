//! The survival archive (#13316 acceptance 3, design §1 and §7).
//!
//! A checkpoint uploads one full-identity partition's dirty set (every key
//! with a pending sweep intent, at its newest pending version) to the
//! central archive tables, replacing that partition's earlier checkpoint in
//! one transaction. Only inline payloads up to the effective threshold are
//! archived. Bytes over it, and every working-file value, are never
//! uploaded: they are recorded as `unprotected` and reported as
//! `protection_unavailable_needs_phase3`, because Phase 2 has no blob
//! upload.
//!
//! The 15-minute interval is an online successful-checkpoint target, not a
//! recovery point: with no network, or with the disk destroyed before the
//! next checkpoint, unarchived writes are lost without bound, and
//! [`Checkpoint::protection_gap`] says so plainly.
//!
//! Restore adopts the archived rows into a store as new local writes, so
//! they sweep through the ordinary CAS path. A key the store already has
//! history for keeps its local rows (the returning-host case): its own
//! intents merge through CAS, and the archived row is reported as kept.

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, params};

use crate::central::{self, CentralError, DEFAULT_BLOB_THRESHOLD, Pipeline, Statement, Value};
use crate::sweep::host_incarnation;
use crate::sync::{Lookup, lookup};
use crate::{Identity, KvError, LocalStore, signed, sql, unsigned};

/// The online successful-checkpoint target (design §1): 15 minutes.
pub const CHECKPOINT_TARGET_MS: i64 = 15 * 60 * 1000;

/// What a key over the archive threshold is reported as.
pub const PROTECTION_UNAVAILABLE: &str = "protection_unavailable_needs_phase3";

/// Version 8 records each partition's last successful checkpoint.
pub(crate) fn migrate_to_v8(tx: &rusqlite::Transaction<'_>) -> Result<(), KvError> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS kv_archive_local (
             owner_user_id TEXT NOT NULL,
             context_id TEXT NOT NULL,
             partition_id TEXT NOT NULL,
             checkpoint_at_ms INTEGER NOT NULL,
             archived INTEGER NOT NULL,
             unprotected INTEGER NOT NULL,
             PRIMARY KEY (owner_user_id, context_id, partition_id)
         ) WITHOUT ROWID;",
    )
    .map_err(sql)
}

#[derive(Debug)]
pub enum ArchiveError {
    Local(KvError),
    Central(CentralError),
    /// The store already has rows for every archived key: nothing adopted.
    Malformed(&'static str),
}

impl ArchiveError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Local(error) => error.code(),
            Self::Central(error) => error.code(),
            Self::Malformed(code) => *code,
        }
    }

    pub fn is_unavailable(&self) -> bool {
        matches!(self, Self::Central(CentralError::Unavailable(_)))
    }
}

impl std::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local(error) => write!(f, "{error}"),
            Self::Central(error) => write!(f, "{error}"),
            Self::Malformed(code) => write!(f, "{code}"),
        }
    }
}

impl std::error::Error for ArchiveError {}

impl From<KvError> for ArchiveError {
    fn from(error: KvError) -> Self {
        Self::Local(error)
    }
}

impl From<CentralError> for ArchiveError {
    fn from(error: CentralError) -> Self {
        Self::Central(error)
    }
}

/// One key of a dirty set, as the archive holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchivedKey {
    pub namespace: String,
    pub key: String,
    pub version: u64,
    pub state: ArchivedState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArchivedState {
    Live(Vec<u8>),
    Tombstone,
    /// Over the effective threshold: no bytes were uploaded.
    Unprotected,
}

impl ArchivedState {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Live(_) => "live",
            Self::Tombstone => "tombstone",
            Self::Unprotected => "unprotected",
        }
    }
}

/// What one checkpoint uploaded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Checkpoint {
    /// Every key of the dirty set, archived or recorded as unprotected.
    pub keys: Vec<ArchivedKey>,
    pub at_ms: i64,
}

impl Checkpoint {
    pub fn archived(&self) -> usize {
        self.keys
            .iter()
            .filter(|key| key.state != ArchivedState::Unprotected)
            .count()
    }

    /// Keys whose bytes the archive does not hold.
    pub fn unprotected(&self) -> impl Iterator<Item = &ArchivedKey> {
        self.keys
            .iter()
            .filter(|key| key.state == ArchivedState::Unprotected)
    }

    /// The plain statement of what this checkpoint does not protect.
    pub fn protection_gap(&self) -> String {
        format!(
            "archive is a {} min online target, not a recovery point: writes made after this \
             checkpoint, or while the primary is unreachable, are lost if this disk is",
            CHECKPOINT_TARGET_MS / 60_000
        )
    }
}

pub(crate) const ARCHIVE_CLEAR: &str = "DELETE FROM kv_archive
              WHERE owner_user_id = ?1 AND context_id = ?2 AND partition_id = ?3";

pub(crate) const ARCHIVE_INSERT: &str = "INSERT INTO kv_archive (
                      owner_user_id, context_id, partition_id, namespace, key, version,
                      state, value, host_incarnation, outbox_id, archived_at_ms)
                  VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)";

pub(crate) const CHECKPOINT_UPSERT: &str = "INSERT INTO kv_archive_checkpoints (
                  owner_user_id, context_id, partition_id, host_incarnation,
                  checkpoint_at_ms, archived, unprotected)
              VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
              ON CONFLICT (owner_user_id, context_id, partition_id) DO UPDATE SET
                  host_incarnation = excluded.host_incarnation,
                  checkpoint_at_ms = excluded.checkpoint_at_ms,
                  archived = excluded.archived,
                  unprotected = excluded.unprotected";

pub(crate) const CHECKPOINT_READ: &str = "SELECT checkpoint_at_ms FROM kv_archive_checkpoints
                  WHERE owner_user_id = ?1 AND context_id = ?2 AND partition_id = ?3";

pub(crate) const ARCHIVE_READ: &str = "SELECT namespace, key, version, state, value FROM kv_archive
                  WHERE owner_user_id = ?1 AND context_id = ?2 AND partition_id = ?3
                  ORDER BY namespace, key";

struct Dirty {
    namespace: String,
    key: String,
    version: u64,
    outbox_id: i64,
    state: String,
    value: Vec<u8>,
    working: bool,
}

/// The newest pending intent of each key of `identity`'s partition.
fn dirty_set(store: &LocalStore, identity: &Identity<'_>) -> Result<Vec<Dirty>, KvError> {
    let mut statement = store
        .connection
        .prepare(
            "SELECT outbox.namespace, outbox.key, outbox.version, outbox.id,
                    version.state, version.value, version.working_id IS NOT NULL
             FROM kv_outbox AS outbox
             JOIN kv_versions_local AS version
               ON version.owner_user_id = outbox.owner_user_id
              AND version.context_id = outbox.context_id
              AND version.partition_id = outbox.partition_id
              AND version.namespace = outbox.namespace
              AND version.key = outbox.key
              AND version.version = outbox.version
             WHERE outbox.owner_user_id = ?1 AND outbox.context_id = ?2
               AND outbox.partition_id = ?3 AND outbox.status = 'pending'
             ORDER BY outbox.id",
        )
        .map_err(sql)?;
    let rows = statement
        .query_map(
            params![
                identity.owner_user_id,
                identity.context_id,
                identity.partition_id
            ],
            |row| {
                Ok(Dirty {
                    namespace: row.get(0)?,
                    key: row.get(1)?,
                    version: unsigned(row.get(2)?),
                    outbox_id: row.get(3)?,
                    state: row.get(4)?,
                    value: row.get(5)?,
                    working: row.get(6)?,
                })
            },
        )
        .map_err(sql)?;
    // Later intents replace earlier ones: the archive holds each key once,
    // at its newest pending version.
    let mut newest: BTreeMap<(String, String), Dirty> = BTreeMap::new();
    for row in rows {
        let row = row.map_err(sql)?;
        newest.insert((row.namespace.clone(), row.key.clone()), row);
    }
    Ok(newest.into_values().collect())
}

/// The effective inline threshold for `namespace`: its pulled definition's,
/// or the default when this host has none.
fn threshold(store: &LocalStore, namespace: &str) -> Result<u64, KvError> {
    Ok(match lookup(store, namespace)? {
        Lookup::Known(definition) => definition.blob_threshold,
        Lookup::NeverPulled | Lookup::Unknown => DEFAULT_BLOB_THRESHOLD,
    })
}

/// Uploads `identity`'s dirty set as its partition's new checkpoint.
/// An empty dirty set still checkpoints: it clears the partition's archive,
/// because everything in it has swept.
pub fn checkpoint(
    store: &LocalStore,
    identity: &Identity<'_>,
    primary: &dyn Pipeline,
    now_ms: i64,
) -> Result<Checkpoint, ArchiveError> {
    let incarnation = host_incarnation(store)?;
    let mut keys = Vec::new();
    let partition = [
        Value::Text(identity.owner_user_id.to_string()),
        Value::Text(identity.context_id.to_string()),
        Value::Text(identity.partition_id.to_string()),
    ];
    let mut statements = vec![Statement {
        sql: ARCHIVE_CLEAR.into(),
        args: partition.to_vec(),
    }];
    for dirty in dirty_set(store, identity)? {
        let state = if dirty.state == "tombstone" {
            ArchivedState::Tombstone
        } else if dirty.working || dirty.value.len() as u64 > threshold(store, &dirty.namespace)? {
            ArchivedState::Unprotected
        } else {
            ArchivedState::Live(dirty.value)
        };
        let bytes = match &state {
            ArchivedState::Live(value) => value.clone(),
            _ => Vec::new(),
        };
        let mut args = partition.to_vec();
        args.extend([
            Value::Text(dirty.namespace.clone()),
            Value::Text(dirty.key.clone()),
            Value::Integer(signed(dirty.version)),
            Value::Text(state.as_str().into()),
            Value::Blob(bytes),
            Value::Text(incarnation.clone()),
            Value::Integer(dirty.outbox_id),
            Value::Integer(now_ms),
        ]);
        statements.push(Statement {
            sql: ARCHIVE_INSERT.into(),
            args,
        });
        keys.push(ArchivedKey {
            namespace: dirty.namespace,
            key: dirty.key,
            version: dirty.version,
            state,
        });
    }
    let checkpoint = Checkpoint {
        keys,
        at_ms: now_ms,
    };
    let archived = checkpoint.archived() as i64;
    let unprotected = checkpoint.unprotected().count() as i64;
    let mut args = partition.to_vec();
    args.extend([
        Value::Text(incarnation),
        Value::Integer(now_ms),
        Value::Integer(archived),
        Value::Integer(unprotected),
    ]);
    statements.push(Statement {
        sql: CHECKPOINT_UPSERT.into(),
        args,
    });
    // Either the whole checkpoint lands or none of it does; the local
    // record moves only after the primary acknowledged it.
    primary.transaction(&statements)?;
    store
        .connection
        .execute(
            "INSERT INTO kv_archive_local (
                 owner_user_id, context_id, partition_id, checkpoint_at_ms, archived, unprotected)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (owner_user_id, context_id, partition_id) DO UPDATE SET
                 checkpoint_at_ms = excluded.checkpoint_at_ms,
                 archived = excluded.archived,
                 unprotected = excluded.unprotected",
            params![
                identity.owner_user_id,
                identity.context_id,
                identity.partition_id,
                now_ms,
                archived,
                unprotected
            ],
        )
        .map_err(sql)?;
    Ok(checkpoint)
}

/// When this host last checkpointed `identity`'s partition.
pub fn last_checkpoint_ms(
    store: &LocalStore,
    identity: &Identity<'_>,
) -> Result<Option<i64>, KvError> {
    store
        .connection
        .query_row(
            "SELECT checkpoint_at_ms FROM kv_archive_local
             WHERE owner_user_id = ?1 AND context_id = ?2 AND partition_id = ?3",
            params![
                identity.owner_user_id,
                identity.context_id,
                identity.partition_id
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql)
}

/// Whether a checkpoint is due: the partition has unswept writes and its
/// last successful checkpoint is older than the target (or never was).
pub fn due(store: &LocalStore, identity: &Identity<'_>, now_ms: i64) -> Result<bool, KvError> {
    let pending: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM kv_outbox
             WHERE owner_user_id = ?1 AND context_id = ?2 AND partition_id = ?3
               AND status = 'pending'",
            params![
                identity.owner_user_id,
                identity.context_id,
                identity.partition_id
            ],
            |row| row.get(0),
        )
        .map_err(sql)?;
    if pending == 0 {
        return Ok(false);
    }
    Ok(match last_checkpoint_ms(store, identity)? {
        None => true,
        Some(at) => now_ms.saturating_sub(at) >= CHECKPOINT_TARGET_MS,
    })
}

/// The archived checkpoint of `identity`'s partition, if any.
pub fn fetch(
    primary: &dyn Pipeline,
    identity: &Identity<'_>,
) -> Result<Option<Checkpoint>, CentralError> {
    let partition = vec![
        Value::Text(identity.owner_user_id.to_string()),
        Value::Text(identity.context_id.to_string()),
        Value::Text(identity.partition_id.to_string()),
    ];
    let results = primary.transaction(&[
        Statement {
            sql: CHECKPOINT_READ.into(),
            args: partition.clone(),
        },
        Statement {
            sql: ARCHIVE_READ.into(),
            args: partition,
        },
    ])?;
    let mut results = results.into_iter();
    let Some(at_ms) = results
        .next()
        .and_then(|rows| rows.rows.into_iter().next())
        .and_then(|row| row.first().and_then(Value::as_i64))
    else {
        return Ok(None);
    };
    let mut keys = Vec::new();
    for mut row in results.next().map(|rows| rows.rows).unwrap_or_default() {
        if row.len() < 5 {
            return Err(CentralError::Store(
                "archive row has too few columns".into(),
            ));
        }
        let value = std::mem::replace(&mut row[4], Value::Null);
        let state = match row[3].as_str() {
            Some("live") => ArchivedState::Live(value.into_bytes().unwrap_or_default()),
            Some("tombstone") => ArchivedState::Tombstone,
            _ => ArchivedState::Unprotected,
        };
        keys.push(ArchivedKey {
            namespace: central::string(&row[0]),
            key: central::string(&row[1]),
            version: central::unsigned(&row[2]),
            state,
        });
    }
    Ok(Some(Checkpoint { keys, at_ms }))
}

/// What a restore did with each archived key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Restored {
    /// Adopted as new local writes, queued to sweep.
    pub adopted: Vec<(String, String)>,
    /// The store already has history for these keys; its rows were kept.
    pub kept_local: Vec<(String, String)>,
    /// The archive holds no bytes for these (`protection_unavailable_needs_phase3`).
    pub unprotected: Vec<(String, String)>,
    /// When the restored checkpoint was taken.
    pub checkpoint_at_ms: i64,
}

/// Restores `identity`'s archived checkpoint into `store`. Refuses with
/// `archive_not_found` when the partition has none.
pub fn restore(
    store: &LocalStore,
    identity: &Identity<'_>,
    primary: &dyn Pipeline,
) -> Result<Restored, ArchiveError> {
    let Some(checkpoint) = fetch(primary, identity)? else {
        return Err(ArchiveError::Malformed("archive_not_found"));
    };
    let mut restored = Restored {
        checkpoint_at_ms: checkpoint.at_ms,
        ..Restored::default()
    };
    for archived in checkpoint.keys {
        let pair = (archived.namespace.clone(), archived.key.clone());
        if archived.state == ArchivedState::Unprotected {
            restored.unprotected.push(pair);
            continue;
        }
        if has_history(store, identity, &archived.namespace, &archived.key)? {
            restored.kept_local.push(pair);
            continue;
        }
        match archived.state {
            ArchivedState::Live(value) => {
                store.set(identity, &archived.namespace, &archived.key, &value)?;
            }
            ArchivedState::Tombstone => {
                store.delete(identity, &archived.namespace, &archived.key)?;
            }
            ArchivedState::Unprotected => unreachable!("handled above"),
        }
        restored.adopted.push(pair);
    }
    Ok(restored)
}

fn has_history(
    store: &LocalStore,
    identity: &Identity<'_>,
    namespace: &str,
    key: &str,
) -> Result<bool, KvError> {
    let found: Option<i64> = store
        .connection
        .query_row(
            "SELECT 1 FROM kv_versions_local
             WHERE owner_user_id = ?1 AND context_id = ?2 AND partition_id = ?3
               AND namespace = ?4 AND key = ?5
             LIMIT 1",
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
        .map_err(sql)?;
    Ok(found.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::central::{Address, Namespace, SqlitePrimary, define_namespace};
    use crate::sweep::Sweeper;
    use crate::sync::{pull_namespaces, sync};
    use std::time::Duration;

    fn primary() -> SqlitePrimary {
        let primary = SqlitePrimary::in_memory().unwrap();
        define_namespace(&primary, &Namespace::shared("status", "fleet-status")).unwrap();
        let mut small = Namespace::shared("small", "fleet-small");
        small.blob_threshold = 4;
        define_namespace(&primary, &small).unwrap();
        primary
    }

    fn me() -> Identity<'static> {
        Identity::new("user-1", "ctx-1", "session-1")
    }

    #[test]
    fn central_archive_checkpoints_the_dirty_set_inline_only() {
        let primary = primary();
        let store = LocalStore::in_memory().unwrap();
        pull_namespaces(&store, &primary).unwrap();
        store.set(&me(), "status", "a", b"one").unwrap();
        store.set(&me(), "status", "a", b"two").unwrap();
        store.set(&me(), "small", "big", b"too large").unwrap();
        store.set(&me(), "small", "tiny", b"ok").unwrap();
        store.delete(&me(), "status", "gone").unwrap();
        let checkpoint = checkpoint(&store, &me(), &primary, 1_000).unwrap();
        assert_eq!(checkpoint.archived(), 3);
        let unprotected: Vec<_> = checkpoint
            .unprotected()
            .map(|key| key.key.as_str())
            .collect();
        assert_eq!(unprotected, ["big"]);
        let fetched = fetch(&primary, &me()).unwrap().expect("archived");
        assert_eq!(fetched.at_ms, 1_000);
        let a = fetched.keys.iter().find(|key| key.key == "a").unwrap();
        assert_eq!(a.state, ArchivedState::Live(b"two".to_vec()));
        assert_eq!(a.version, 2);
        let big = fetched.keys.iter().find(|key| key.key == "big").unwrap();
        // No bytes over the threshold ever reach the archive.
        assert_eq!(big.state, ArchivedState::Unprotected);
        assert!(
            fetched
                .keys
                .iter()
                .any(|key| key.key == "gone" && key.state == ArchivedState::Tombstone)
        );
        assert!(checkpoint.protection_gap().contains("not a recovery point"));
    }

    #[test]
    fn central_archive_checkpoint_shrinks_after_sweep_and_is_due_on_target() {
        let primary = primary();
        let store = LocalStore::in_memory().unwrap();
        store.set(&me(), "status", "a", b"one").unwrap();
        assert!(due(&store, &me(), 0).unwrap());
        checkpoint(&store, &me(), &primary, 0).unwrap();
        assert!(!due(&store, &me(), CHECKPOINT_TARGET_MS - 1).unwrap());
        assert!(due(&store, &me(), CHECKPOINT_TARGET_MS).unwrap());
        sync(&store, &me(), &primary, "user-1", &mut |_: Duration| {}).unwrap();
        // Everything swept: nothing is due, and the next checkpoint clears
        // the partition's archive.
        assert!(!due(&store, &me(), 10 * CHECKPOINT_TARGET_MS).unwrap());
        let empty = checkpoint(&store, &me(), &primary, 5).unwrap();
        assert!(empty.keys.is_empty());
        assert!(fetch(&primary, &me()).unwrap().unwrap().keys.is_empty());
    }

    #[test]
    fn central_archive_restore_adopts_on_a_fresh_host_and_sweeps() {
        let primary = primary();
        let lost = LocalStore::in_memory().unwrap();
        pull_namespaces(&lost, &primary).unwrap();
        lost.set(&me(), "status", "a", b"kept").unwrap();
        lost.set(&me(), "small", "big", b"too large").unwrap();
        lost.delete(&me(), "status", "gone").unwrap();
        checkpoint(&lost, &me(), &primary, 7).unwrap();
        drop(lost);

        let fresh = LocalStore::in_memory().unwrap();
        let restored = restore(&fresh, &me(), &primary).unwrap();
        assert_eq!(restored.checkpoint_at_ms, 7);
        assert_eq!(restored.adopted.len(), 2);
        assert_eq!(
            restored.unprotected,
            [("small".to_string(), "big".to_string())]
        );
        let mut out = Vec::new();
        fresh.get_to(&me(), "status", "a", &mut out).unwrap();
        assert_eq!(out, b"kept");
        assert!(fresh.is_masked(&me(), "status", "gone").unwrap());
        // The adopted rows sweep through the ordinary CAS path.
        let mut sleep = |_: Duration| {};
        let resolve = |namespace: &str| central::resolve(&primary, central::REGISTRY_ID, namespace);
        let report = Sweeper {
            primary: &primary,
            writer: "user-1",
            resolve: &resolve,
            sleep: &mut sleep,
            blobs: None,
        }
        .sweep(&fresh, &me())
        .unwrap();
        assert_eq!(report.committed(), 2);
        let head = central::head(&primary, &Address::new("fleet-status", "status", "a"))
            .unwrap()
            .expect("swept");
        assert_eq!(head.value, b"kept");
    }

    #[test]
    fn central_archive_restore_keeps_local_rows_and_refuses_without_archive() {
        let primary = primary();
        let fresh = LocalStore::in_memory().unwrap();
        assert_eq!(
            restore(&fresh, &me(), &primary).unwrap_err().code(),
            "archive_not_found"
        );
        let lost = LocalStore::in_memory().unwrap();
        lost.set(&me(), "status", "a", b"archived").unwrap();
        checkpoint(&lost, &me(), &primary, 1).unwrap();
        let returning = LocalStore::in_memory().unwrap();
        returning.set(&me(), "status", "a", b"local").unwrap();
        let restored = restore(&returning, &me(), &primary).unwrap();
        assert_eq!(
            restored.kept_local,
            [("status".to_string(), "a".to_string())]
        );
        let mut out = Vec::new();
        returning.get_to(&me(), "status", "a", &mut out).unwrap();
        assert_eq!(out, b"local");
    }

    #[test]
    fn central_archive_failed_checkpoint_changes_nothing_locally() {
        struct Down;
        impl Pipeline for Down {
            fn transaction(&self, _: &[Statement]) -> central::Result<Vec<central::Rows>> {
                Err(CentralError::Unavailable("offline".into()))
            }
        }
        let store = LocalStore::in_memory().unwrap();
        store.set(&me(), "status", "a", b"one").unwrap();
        let error = checkpoint(&store, &me(), &Down, 0).unwrap_err();
        assert!(error.is_unavailable());
        assert_eq!(last_checkpoint_ms(&store, &me()).unwrap(), None);
        assert!(due(&store, &me(), 0).unwrap());
    }

    #[test]
    fn central_archive_partitions_are_isolated() {
        let primary = primary();
        let store = LocalStore::in_memory().unwrap();
        let other = Identity::new("user-1", "ctx-1", "session-2");
        store.set(&me(), "status", "mine", b"1").unwrap();
        store.set(&other, "status", "theirs", b"2").unwrap();
        checkpoint(&store, &me(), &primary, 1).unwrap();
        checkpoint(&store, &other, &primary, 2).unwrap();
        let mine = fetch(&primary, &me()).unwrap().unwrap();
        assert_eq!(mine.keys.len(), 1);
        assert_eq!(mine.keys[0].key, "mine");
        let theirs = fetch(&primary, &other).unwrap().unwrap();
        assert_eq!(theirs.keys[0].key, "theirs");
    }
}
