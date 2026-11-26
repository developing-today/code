//! Local-first merged reads (code#13316, design §3, D004, D008).
//!
//! A plain `get` reads the caller's overlay first. A live overlay row wins;
//! a masking tombstone reads absent and is never filled in from central. On
//! a miss, and only when the host has the namespace's definition cached and
//! a central store is configured, the read falls through to the central head
//! of the scope that definition selects for the caller.
//!
//! Central heads read this way are cached in the local store for at most
//! [`CACHE_MAX_AGE_MS`] (1 s). On a miss or an expired entry the reader
//! refreshes from central; if that fails it refuses with `unavailable` (no
//! entry) or `stale` (an expired entry), never returning an old value
//! silently. `--allow-stale` is the opt-in to read the expired entry, and the
//! read then reports its age. `--global` bypasses this cache entirely.
//!
//! A merged `list` shows local and central keys with origin marks (`local`,
//! `central`, `both`, `masked`) and, in verbose output, the central row a
//! local row shadows, so no row is ever hidden silently.

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, params};

use crate::blob::{BlobError, RAW_ENVELOPE};
use crate::blob_read::{self, BlobReader, ValueError};
use crate::central::{self, Address, CentralError, Head, Namespace, Pipeline, REGISTRY_ID};
use crate::sync::{Lookup, lookup};
use crate::{Identity, KvError, LocalStore, signed, sql, unsigned};

/// How old an ordinary cached central read may be (design D008).
pub const CACHE_MAX_AGE_MS: i64 = 1_000;

/// Local schema version 7: central heads read through a local miss.
pub(crate) fn migrate_to_v7(tx: &rusqlite::Transaction<'_>) -> Result<(), KvError> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS kv_central_cache (
             scope_id TEXT NOT NULL,
             namespace TEXT NOT NULL,
             key TEXT NOT NULL,
             present INTEGER NOT NULL CHECK (present IN (0, 1)),
             version INTEGER NOT NULL,
             live INTEGER NOT NULL CHECK (live IN (0, 1)),
             value BLOB NOT NULL,
             fetched_at_ms INTEGER NOT NULL,
             PRIMARY KEY (scope_id, namespace, key)
         ) WITHOUT ROWID;",
    )
    .map_err(sql)
}

/// Local schema version 10: a cached central head keeps its row envelope
/// (design §11), so a cached blob reference is never served as bytes.
pub(crate) fn migrate_to_v10(tx: &rusqlite::Transaction<'_>) -> Result<(), KvError> {
    let present: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('kv_central_cache') WHERE name='envelope'",
            [],
            |row| row.get(0),
        )
        .map_err(sql)?;
    if present == 0 {
        tx.execute_batch(
            "ALTER TABLE kv_central_cache ADD COLUMN envelope TEXT NOT NULL DEFAULT 'kv:raw-v1';",
        )
        .map_err(sql)?;
    }
    Ok(())
}

/// Where a read's bytes came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Local,
    /// A central head no more than 1 s old.
    Central {
        version: u64,
    },
    /// An expired central head, read only under `--allow-stale`.
    Snapshot {
        version: u64,
        age_ms: i64,
    },
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local => f.write_str("local"),
            Self::Central { version } => write!(f, "central@{version}"),
            Self::Snapshot { version, age_ms } => {
                write!(f, "snapshot@{version} age={age_ms}ms (stale)")
            }
        }
    }
}

#[derive(Debug)]
pub enum ReadError {
    Local(KvError),
    /// Central refused for a reason that is not about reaching it.
    Central(CentralError),
    /// A blob-backed value could not be read and verified.
    Blob(BlobError),
    /// Central could not be reached and nothing is cached for this key.
    Unavailable(String),
    /// Central could not be reached and the cached head is older than 1 s.
    Stale {
        age_ms: i64,
        reason: String,
    },
}

impl ReadError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Local(error) => error.code(),
            Self::Central(error) => error.code(),
            Self::Blob(error) => error.code(),
            Self::Unavailable(_) => "unavailable",
            Self::Stale { .. } => "stale",
        }
    }

    /// Exit 69 rather than 1.
    pub fn is_unavailable(&self) -> bool {
        matches!(
            self,
            Self::Unavailable(_)
                | Self::Stale { .. }
                | Self::Central(CentralError::Unavailable(_) | CentralError::NotConfigured(_))
        )
    }
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local(error) => write!(f, "{error}"),
            Self::Central(error) => write!(f, "{error}"),
            Self::Blob(error) => write!(f, "{error}"),
            Self::Unavailable(reason) => write!(
                f,
                "the central store could not be reached and nothing is cached: {reason}"
            ),
            Self::Stale { age_ms, reason } => write!(
                f,
                "the central store could not be reached ({reason}); the cached value is {age_ms} ms old, past the 1 s limit; --allow-stale reads it anyway"
            ),
        }
    }
}

impl std::error::Error for ReadError {}

impl From<KvError> for ReadError {
    fn from(error: KvError) -> Self {
        Self::Local(error)
    }
}

impl From<ValueError> for ReadError {
    fn from(error: ValueError) -> Self {
        match error {
            ValueError::Central(error) => Self::Central(error),
            ValueError::Blob(error) => Self::Blob(error),
        }
    }
}

/// Turns a central failure into a read refusal: transient failures are
/// `unavailable`, anything else (a refused credential, a malformed answer)
/// is reported as itself.
fn unreachable_or(error: CentralError) -> Result<String, ReadError> {
    if error.is_retryable() {
        Ok(error.to_string())
    } else {
        Err(ReadError::Central(error))
    }
}

/// The central side of a merged read: the store, the caller and, when one
/// is configured, the central plane.
pub struct Reader<'a> {
    pub store: &'a LocalStore,
    pub identity: &'a Identity<'a>,
    pub central: Option<&'a dyn Pipeline>,
    /// The blob door's read side, for heads that hold a blob reference.
    pub blobs: Option<&'a dyn BlobReader>,
    /// Read an expired cached head when central cannot be reached.
    pub allow_stale: bool,
    /// The current time, in Unix milliseconds.
    pub now_ms: i64,
}

/// What a merged `get` found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// The value was written to `out`.
    Value(Source),
    /// Missing everywhere the caller may read.
    Absent,
    /// The overlay masks the key.
    Masked,
}

struct Cached {
    present: bool,
    version: u64,
    live: bool,
    value: Vec<u8>,
    envelope: String,
    fetched_at_ms: i64,
}

impl Reader<'_> {
    /// The namespace definition the host has cached, if central reads are
    /// possible at all for it.
    fn definition(&self, namespace: &str) -> Result<Option<Namespace>, ReadError> {
        if self.central.is_none() {
            return Ok(None);
        }
        match lookup(self.store, namespace)? {
            Lookup::Known(definition) => Ok(Some(definition)),
            Lookup::NeverPulled => Ok(None),
            Lookup::Unknown => Err(KvError::UnknownNamespace(namespace.to_string()).into()),
        }
    }

    /// Plain `get`: the overlay, then the caller's central head.
    pub fn get(
        &self,
        namespace: &str,
        key: &str,
        out: &mut dyn std::io::Write,
    ) -> Result<Found, ReadError> {
        if self.store.get_to(self.identity, namespace, key, out)? {
            return Ok(Found::Value(Source::Local));
        }
        if self.store.is_masked(self.identity, namespace, key)? {
            return Ok(Found::Masked);
        }
        let Some(definition) = self.definition(namespace)? else {
            return Ok(Found::Absent);
        };
        let scope_id = definition.scope_for(self.identity.owner_user_id, self.identity.context_id);
        let (cached, source) = self.central_head(&scope_id, namespace, key)?;
        if !(cached.present && cached.live) {
            return Ok(Found::Absent);
        }
        let address = Address::new(&scope_id, namespace, key);
        blob_read::write_value(&cached.envelope, &cached.value, &address, self.blobs, out)?;
        Ok(Found::Value(source))
    }

    /// The caller's central head, from a cache entry at most 1 s old or a
    /// refresh; under `--allow-stale` an expired entry when refresh fails.
    fn central_head(
        &self,
        scope_id: &str,
        namespace: &str,
        key: &str,
    ) -> Result<(Cached, Source), ReadError> {
        let cached = self.cached(scope_id, namespace, key)?;
        if let Some(entry) = cached.as_ref()
            && self.now_ms - entry.fetched_at_ms <= CACHE_MAX_AGE_MS
        {
            let source = Source::Central {
                version: entry.version,
            };
            return Ok((self.take(cached), source));
        }
        let primary = self
            .central
            .ok_or_else(|| ReadError::Unavailable("no central store is configured".to_string()))?;
        let address = Address::new(scope_id, namespace, key);
        match central::head(primary, &address) {
            Ok(head) => {
                let fresh = self.remember(scope_id, namespace, key, head.as_ref())?;
                let source = Source::Central {
                    version: fresh.version,
                };
                Ok((fresh, source))
            }
            Err(error) => {
                let reason = unreachable_or(error)?;
                match cached {
                    None => Err(ReadError::Unavailable(reason)),
                    Some(entry) => {
                        let age_ms = self.now_ms - entry.fetched_at_ms;
                        if self.allow_stale {
                            let source = Source::Snapshot {
                                version: entry.version,
                                age_ms,
                            };
                            Ok((entry, source))
                        } else {
                            Err(ReadError::Stale { age_ms, reason })
                        }
                    }
                }
            }
        }
    }

    fn take(&self, cached: Option<Cached>) -> Cached {
        cached.unwrap_or(Cached {
            present: false,
            version: 0,
            live: false,
            value: Vec::new(),
            envelope: RAW_ENVELOPE.to_string(),
            fetched_at_ms: self.now_ms,
        })
    }

    fn cached(
        &self,
        scope_id: &str,
        namespace: &str,
        key: &str,
    ) -> Result<Option<Cached>, ReadError> {
        Ok(self
            .store
            .connection
            .query_row(
                "SELECT present, version, live, value, fetched_at_ms, envelope FROM kv_central_cache
                 WHERE scope_id = ?1 AND namespace = ?2 AND key = ?3",
                params![scope_id, namespace, key],
                |row| {
                    Ok(Cached {
                        present: row.get::<_, i64>(0)? == 1,
                        version: unsigned(row.get(1)?),
                        live: row.get::<_, i64>(2)? == 1,
                        value: row.get(3)?,
                        fetched_at_ms: row.get(4)?,
                        envelope: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(sql)?)
    }

    /// Records a freshly read head (or its absence) in the cache.
    fn remember(
        &self,
        scope_id: &str,
        namespace: &str,
        key: &str,
        head: Option<&Head>,
    ) -> Result<Cached, ReadError> {
        let entry = Cached {
            present: head.is_some(),
            version: head.map_or(0, |head| head.version),
            live: head.is_some_and(|head| head.live),
            value: head.map(|head| head.value.clone()).unwrap_or_default(),
            envelope: head.map_or_else(|| RAW_ENVELOPE.to_string(), |head| head.envelope.clone()),
            fetched_at_ms: self.now_ms,
        };
        self.store
            .connection
            .execute(
                "INSERT INTO kv_central_cache
                     (scope_id, namespace, key, present, version, live, value, fetched_at_ms,
                      envelope)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT (scope_id, namespace, key) DO UPDATE SET
                     present = excluded.present,
                     version = excluded.version,
                     live = excluded.live,
                     value = excluded.value,
                     fetched_at_ms = excluded.fetched_at_ms,
                     envelope = excluded.envelope",
                params![
                    scope_id,
                    namespace,
                    key,
                    i64::from(entry.present),
                    signed(entry.version),
                    i64::from(entry.live),
                    entry.value,
                    entry.fetched_at_ms,
                    entry.envelope
                ],
            )
            .map_err(sql)?;
        Ok(entry)
    }

    /// Merged `list`: local rows and the caller's central heads, by key.
    /// The central listing is read fresh on every call. When central cannot
    /// be reached the list refuses with `unavailable`, or under
    /// `--allow-stale` shows local rows only and says so.
    pub fn list(
        &self,
        namespace: &str,
        include_tombstones: bool,
        verbose: bool,
    ) -> Result<MergedList, ReadError> {
        let local = self.store.list(self.identity, namespace, true)?;
        let mut merged: BTreeMap<String, Vec<MergedEntry>> = BTreeMap::new();
        let mut central_skipped = None;
        let central_rows = match self.definition(namespace)? {
            None => Vec::new(),
            Some(definition) => {
                let scope_id =
                    definition.scope_for(self.identity.owner_user_id, self.identity.context_id);
                let primary = self.central.ok_or_else(|| {
                    ReadError::Unavailable("no central store is configured".to_string())
                })?;
                match central::listing(primary, &scope_id, REGISTRY_ID, namespace) {
                    Ok(rows) => rows,
                    Err(error) => {
                        let reason = unreachable_or(error)?;
                        if !self.allow_stale {
                            return Err(ReadError::Unavailable(reason));
                        }
                        central_skipped = Some(reason);
                        Vec::new()
                    }
                }
            }
        };
        let central_by_key: BTreeMap<String, central::Listed> = central_rows
            .into_iter()
            .map(|row| (row.key.clone(), row))
            .collect();
        for entry in &local {
            let masked = entry.origin == crate::Origin::Masked;
            let shadowed = central_by_key.get(&entry.key).filter(|row| row.live);
            let origin = match (masked, shadowed.is_some()) {
                (true, _) => Mark::Masked,
                (false, true) => Mark::Both,
                (false, false) => Mark::Local,
            };
            if masked && !include_tombstones {
                // The key reads absent; verbose output still names what the
                // tombstone hides.
                if verbose && let Some(row) = shadowed {
                    merged
                        .entry(entry.key.clone())
                        .or_default()
                        .push(MergedEntry {
                            key: entry.key.clone(),
                            version: row.version,
                            origin: Mark::ShadowedByLocal,
                        });
                }
                continue;
            }
            let rows = merged.entry(entry.key.clone()).or_default();
            rows.push(MergedEntry {
                key: entry.key.clone(),
                version: entry.version,
                origin,
            });
            if verbose && let Some(row) = shadowed {
                rows.push(MergedEntry {
                    key: entry.key.clone(),
                    version: row.version,
                    origin: Mark::ShadowedByLocal,
                });
            }
        }
        let local_keys: std::collections::HashSet<&str> =
            local.iter().map(|entry| entry.key.as_str()).collect();
        for (key, row) in central_by_key {
            if local_keys.contains(key.as_str()) {
                continue;
            }
            if !row.live && !include_tombstones {
                continue;
            }
            merged.entry(key.clone()).or_default().push(MergedEntry {
                key,
                version: row.version,
                origin: if row.live {
                    Mark::Central
                } else {
                    Mark::CentralMasked
                },
            });
        }
        Ok(MergedList {
            entries: merged.into_values().flatten().collect(),
            central_skipped,
        })
    }
}

/// A merged list row's origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// Only the overlay has the key.
    Local,
    /// Only central has the key.
    Central,
    /// Both have it; the overlay's version is shown and wins reads.
    Both,
    /// The overlay masks the key.
    Masked,
    /// Central's head is a tombstone (shown with `--include-tombstones`).
    CentralMasked,
    /// Verbose only: the central row a local row or tombstone hides.
    ShadowedByLocal,
}

impl Mark {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Central => "central",
            Self::Both => "both",
            Self::Masked => "masked",
            Self::CentralMasked => "central-masked",
            Self::ShadowedByLocal => "shadowed-by-local",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergedEntry {
    pub key: String,
    pub version: u64,
    pub origin: Mark,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergedList {
    pub entries: Vec<MergedEntry>,
    /// Set when `--allow-stale` let the list go out without central rows.
    pub central_skipped: Option<String>,
}

/// The current time in Unix milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::central::{
        IdempotencyKey, Intent, Op, Rows, SqlitePrimary, Statement, cas, define_namespace,
    };
    use crate::sync::pull_namespaces;
    use std::cell::Cell;

    /// A primary that can be switched off.
    struct Switch {
        primary: SqlitePrimary,
        up: Cell<bool>,
    }

    impl Pipeline for Switch {
        fn transaction(&self, statements: &[Statement]) -> central::Result<Vec<Rows>> {
            if self.up.get() {
                self.primary.transaction(statements)
            } else {
                Err(CentralError::Unavailable("switched off".into()))
            }
        }
    }

    fn central() -> Switch {
        let primary = SqlitePrimary::in_memory().unwrap();
        define_namespace(&primary, &Namespace::shared("status", "fleet-status")).unwrap();
        Switch {
            primary,
            up: Cell::new(true),
        }
    }

    fn write(primary: &dyn Pipeline, key: &str, expected: u64, op: Op) {
        let intent = Intent {
            address: Address::new("fleet-status", "status", key),
            expected_version: expected,
            op,
            idempotency: IdempotencyKey {
                writer: "other".into(),
                host_incarnation: "elsewhere".into(),
                outbox_id: format!("{key}-{expected}"),
            },
        };
        cas(primary, &intent).unwrap();
    }

    fn me() -> Identity<'static> {
        Identity::new("user-1", "ctx-1", "session-1")
    }

    fn reader<'a>(
        store: &'a LocalStore,
        identity: &'a Identity<'a>,
        central: &'a dyn Pipeline,
        now_ms: i64,
        allow_stale: bool,
    ) -> Reader<'a> {
        Reader {
            store,
            identity,
            central: Some(central),
            blobs: None,
            allow_stale,
            now_ms,
        }
    }

    fn get(reader: &Reader<'_>, key: &str) -> Result<(Found, Vec<u8>), ReadError> {
        let mut out = Vec::new();
        let found = reader.get("status", key, &mut out)?;
        Ok((found, out))
    }

    #[test]
    fn central_read_local_overrides_and_miss_falls_through() {
        let central = central();
        let store = LocalStore::in_memory().unwrap();
        let identity = me();
        pull_namespaces(&store, &central).unwrap();
        write(&central, "agent", 0, Op::Set(b"from central".to_vec()));
        let read = reader(&store, &identity, &central, 10_000, false);
        assert_eq!(
            get(&read, "agent").unwrap(),
            (
                Found::Value(Source::Central { version: 1 }),
                b"from central".to_vec()
            )
        );
        store.set(&identity, "status", "agent", b"mine").unwrap();
        assert_eq!(
            get(&read, "agent").unwrap(),
            (Found::Value(Source::Local), b"mine".to_vec())
        );
        // A masking tombstone hides central; it is not filled in.
        store.delete(&identity, "status", "agent").unwrap();
        assert_eq!(get(&read, "agent").unwrap().0, Found::Masked);
        // clear-override reveals central again.
        store.clear_override(&identity, "status", "agent").unwrap();
        assert_eq!(get(&read, "agent").unwrap().1, b"from central".to_vec());
        assert_eq!(get(&read, "missing").unwrap().0, Found::Absent);
    }

    /// A blob door that serves `bytes` and counts its reads.
    struct Door {
        bytes: Vec<u8>,
        opened: Cell<u32>,
    }

    impl BlobReader for Door {
        fn open(
            &self,
            address: &Address,
            _reference: &crate::blob::KvBlobRefV1,
        ) -> central::Result<Box<dyn std::io::Read>> {
            assert_eq!(address, &Address::new("fleet-status", "status", "big"));
            self.opened.set(self.opened.get() + 1);
            Ok(Box::new(std::io::Cursor::new(self.bytes.clone())))
        }
    }

    #[test]
    fn central_read_blob_heads_stream_through_the_door_verified() {
        use crate::blob::{KvBlobRefV1, Partition};
        use crate::blob_central::{mark_published, publish_cas};
        let central = central();
        let store = LocalStore::in_memory().unwrap();
        let identity = me();
        pull_namespaces(&store, &central).unwrap();
        let bytes = b"a value that lives in a blob".to_vec();
        let reference = KvBlobRefV1 {
            digest: blake3::hash(&bytes).to_hex().to_string(),
            bytes_len: bytes.len() as u64,
            partition: Partition::new("eu", "standard"),
            locator: "r2:kv/blobs/eu/standard/x".to_string(),
        };
        mark_published(&central, &reference).unwrap();
        let intent = Intent {
            address: Address::new("fleet-status", "status", "big"),
            expected_version: 0,
            op: Op::Set(reference.encode()),
            idempotency: IdempotencyKey {
                writer: "other".into(),
                host_incarnation: "elsewhere".into(),
                outbox_id: "big".into(),
            },
        };
        publish_cas(&central, &intent, &reference).unwrap();
        // No blob door: refused, and the reference is never served as bytes.
        let refused = get(&reader(&store, &identity, &central, 10_000, false), "big").unwrap_err();
        assert_eq!(refused.code(), "central_not_configured");
        assert!(refused.is_unavailable());
        // With the door, the bytes; a cached head keeps its envelope, so the
        // second read inside the second goes through the door too.
        let door = Door {
            bytes: bytes.clone(),
            opened: Cell::new(0),
        };
        let mut read = reader(&store, &identity, &central, 10_200, false);
        read.blobs = Some(&door);
        assert_eq!(
            get(&read, "big").unwrap(),
            (Found::Value(Source::Central { version: 1 }), bytes.clone())
        );
        central.up.set(false);
        assert_eq!(get(&read, "big").unwrap().1, bytes);
        assert_eq!(door.opened.get(), 2);
        // Bytes that differ from the reference are refused.
        let liar = Door {
            bytes: b"something else entirely, same length!".to_vec(),
            opened: Cell::new(0),
        };
        read.blobs = Some(&liar);
        assert_eq!(
            get(&read, "big").unwrap_err().code(),
            "blob_length_mismatch"
        );
    }

    #[test]
    fn central_read_inline_bytes_that_spell_a_reference_stay_bytes() {
        use crate::blob::{KvBlobRefV1, Partition};
        let central = central();
        let store = LocalStore::in_memory().unwrap();
        let identity = me();
        pull_namespaces(&store, &central).unwrap();
        let spelled = KvBlobRefV1 {
            digest: "ab".repeat(32),
            bytes_len: 3,
            partition: Partition::new("eu", "standard"),
            locator: "r2:x".to_string(),
        }
        .encode();
        write(&central, "big", 0, Op::Set(spelled.clone()));
        assert_eq!(
            get(&reader(&store, &identity, &central, 10_000, false), "big")
                .unwrap()
                .1,
            spelled
        );
    }

    #[test]
    fn central_read_cache_is_at_most_one_second_old() {
        let central = central();
        let store = LocalStore::in_memory().unwrap();
        let identity = me();
        pull_namespaces(&store, &central).unwrap();
        write(&central, "agent", 0, Op::Set(b"v1".to_vec()));
        assert_eq!(
            get(&reader(&store, &identity, &central, 10_000, false), "agent")
                .unwrap()
                .1,
            b"v1"
        );
        write(&central, "agent", 1, Op::Set(b"v2".to_vec()));
        // Within 1 s the cached head may be served.
        assert_eq!(
            get(&reader(&store, &identity, &central, 11_000, false), "agent")
                .unwrap()
                .1,
            b"v1"
        );
        // Past 1 s the reader refreshes.
        assert_eq!(
            get(&reader(&store, &identity, &central, 11_001, false), "agent").unwrap(),
            (Found::Value(Source::Central { version: 2 }), b"v2".to_vec())
        );
    }

    #[test]
    fn central_read_expired_cache_refuses_unless_allow_stale() {
        let central = central();
        let store = LocalStore::in_memory().unwrap();
        let identity = me();
        pull_namespaces(&store, &central).unwrap();
        write(&central, "agent", 0, Op::Set(b"v1".to_vec()));
        get(&reader(&store, &identity, &central, 10_000, false), "agent").unwrap();
        central.up.set(false);
        // Still fresh: served from the cache with central down.
        assert_eq!(
            get(&reader(&store, &identity, &central, 10_500, false), "agent")
                .unwrap()
                .1,
            b"v1"
        );
        // Expired: a typed refusal, never the old value silently.
        let refused =
            get(&reader(&store, &identity, &central, 12_500, false), "agent").unwrap_err();
        assert_eq!(refused.code(), "stale");
        assert!(refused.is_unavailable());
        // --allow-stale reads it, labelled with its age.
        assert_eq!(
            get(&reader(&store, &identity, &central, 12_500, true), "agent").unwrap(),
            (
                Found::Value(Source::Snapshot {
                    version: 1,
                    age_ms: 2_500
                }),
                b"v1".to_vec()
            )
        );
        // Nothing cached at all: unavailable.
        let missing = get(&reader(&store, &identity, &central, 12_500, true), "other").unwrap_err();
        assert_eq!(missing.code(), "unavailable");
        // The overlay still answers offline.
        store.set(&identity, "status", "agent", b"local").unwrap();
        assert_eq!(
            get(&reader(&store, &identity, &central, 99_000, false), "agent")
                .unwrap()
                .1,
            b"local"
        );
    }

    #[test]
    fn central_read_without_central_or_definitions_is_local_only() {
        let store = LocalStore::in_memory().unwrap();
        let identity = me();
        let local_only = Reader {
            store: &store,
            identity: &identity,
            central: None,
            blobs: None,
            allow_stale: false,
            now_ms: 0,
        };
        assert_eq!(get(&local_only, "agent").unwrap().0, Found::Absent);
        // Configured central, but this host never pulled definitions.
        let central = central();
        write(&central, "agent", 0, Op::Set(b"v1".to_vec()));
        assert_eq!(
            get(&reader(&store, &identity, &central, 0, false), "agent")
                .unwrap()
                .0,
            Found::Absent
        );
    }

    #[test]
    fn central_read_merged_list_marks_origins_and_never_hides_shadowed_rows() {
        let central = central();
        let store = LocalStore::in_memory().unwrap();
        let identity = me();
        pull_namespaces(&store, &central).unwrap();
        write(&central, "both", 0, Op::Set(b"c".to_vec()));
        write(&central, "only-central", 0, Op::Set(b"c".to_vec()));
        write(&central, "masked-here", 0, Op::Set(b"c".to_vec()));
        write(&central, "gone", 0, Op::Set(b"c".to_vec()));
        write(&central, "gone", 1, Op::Tombstone);
        store.set(&identity, "status", "both", b"l").unwrap();
        store.set(&identity, "status", "only-local", b"l").unwrap();
        store.delete(&identity, "status", "masked-here").unwrap();
        let read = reader(&store, &identity, &central, 0, false);
        let marks = |list: MergedList| -> Vec<(String, &'static str)> {
            list.entries
                .into_iter()
                .map(|entry| (entry.key, entry.origin.as_str()))
                .collect()
        };
        assert_eq!(
            marks(read.list("status", false, false).unwrap()),
            vec![
                ("both".to_string(), "both"),
                ("only-central".to_string(), "central"),
                ("only-local".to_string(), "local"),
            ]
        );
        assert_eq!(
            marks(read.list("status", true, true).unwrap()),
            vec![
                ("both".to_string(), "both"),
                ("both".to_string(), "shadowed-by-local"),
                ("gone".to_string(), "central-masked"),
                ("masked-here".to_string(), "masked"),
                ("masked-here".to_string(), "shadowed-by-local"),
                ("only-central".to_string(), "central"),
                ("only-local".to_string(), "local"),
            ]
        );
        // Verbose without tombstones still names what a mask hides.
        assert!(
            marks(read.list("status", false, true).unwrap())
                .contains(&("masked-here".to_string(), "shadowed-by-local"))
        );
        central.up.set(false);
        assert_eq!(
            read.list("status", false, false).unwrap_err().code(),
            "unavailable"
        );
        let stale = reader(&store, &identity, &central, 0, true)
            .list("status", false, false)
            .unwrap();
        assert!(stale.central_skipped.is_some());
        assert_eq!(
            marks(stale),
            vec![
                ("both".to_string(), "local"),
                ("only-local".to_string(), "local"),
            ]
        );
    }
}
