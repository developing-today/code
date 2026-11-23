//! Identity-scoped local storage primitives for the `fleet kv` domain.

use rusqlite::{Connection, OptionalExtension, params};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KvError {
    MalformedKey,
    MalformedNamespace,
    Sql(rusqlite::Error),
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

pub struct LocalStore {
    connection: Connection,
}

impl LocalStore {
    pub fn in_memory() -> Result<Self, KvError> {
        let connection = Connection::open_in_memory().map_err(KvError::Sql)?;
        Self::from_connection(connection)
    }

    /// Open (or create) the durable host-local SQLite database.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, KvError> {
        let connection = Connection::open(path).map_err(KvError::Sql)?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(KvError::Sql)?;
        Self::from_connection(connection)
    }

    fn from_connection(connection: Connection) -> Result<Self, KvError> {
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
            .map_err(KvError::Sql)?;
        Ok(Self { connection })
    }

    /// Store arbitrary bytes under the caller's complete identity tuple.
    pub fn set(
        &self,
        identity: &Identity<'_>,
        namespace: &str,
        key: &str,
        value: &[u8],
    ) -> Result<(), KvError> {
        validate_namespace(namespace)?;
        validate_key(key)?;
        self.connection
            .execute(
                "INSERT INTO kv_local(owner_user_id,context_id,partition_id,namespace,key,value)
             VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(owner_user_id,context_id,partition_id,namespace,key)
             DO UPDATE SET value=excluded.value",
                params![
                    identity.owner_user_id,
                    identity.context_id,
                    identity.partition_id,
                    namespace,
                    key,
                    value
                ],
            )
            .map_err(KvError::Sql)?;
        Ok(())
    }

    /// Read only a row owned by the caller's complete identity tuple.
    pub fn get(
        &self,
        identity: &Identity<'_>,
        namespace: &str,
        key: &str,
    ) -> Result<Option<Vec<u8>>, KvError> {
        validate_namespace(namespace)?;
        validate_key(key)?;
        self.connection.query_row(
            "SELECT value FROM kv_local
             WHERE owner_user_id=?1 AND context_id=?2 AND partition_id=?3 AND namespace=?4 AND key=?5",
            params![identity.owner_user_id, identity.context_id, identity.partition_id, namespace, key],
            |row| row.get(0),
        ).optional().map_err(KvError::Sql)
    }
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
    if key.is_empty()
        || key.len() > 512
        || key.starts_with('/')
        || key.ends_with('/')
        || !key.is_ascii()
        || key
            .split('/')
            .any(|segment| segment.is_empty() || !segment.bytes().all(|b| b.is_ascii_graphic()))
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
    }
}
