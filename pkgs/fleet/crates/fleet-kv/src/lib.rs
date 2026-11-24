//! Identity-scoped local storage primitives for the `fleet kv` domain.

use rusqlite::{Connection, OptionalExtension, params};

#[derive(Debug)]
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
