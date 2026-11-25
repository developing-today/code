//! The host's namespace cache, `fleet kv sync`, and the `--global` verbs
//! (code#13316, design §2-§3).
//!
//! A host resolves namespaces only from its own cache of the central
//! `kv_namespaces` table (registry `fleet-kv-central`). `fleet kv sync` pulls
//! that table, replacing the cache and recording when, then sweeps the
//! session's pending intents through central CAS using the freshly pulled
//! definitions. Before the first pull the host has no definitions at all and
//! local verbs work as in Phase 1; after it, a namespace missing from the
//! cache is a typed `unknown_namespace` refusal. Resolution never falls back
//! to another registry.
//!
//! The `--global` verbs bypass the local overlay and every cache: they read
//! the namespace definition and the head straight from central, and write by
//! CAS against the head they read (or the version the caller expects).

use std::io::{Read, Write};
use std::time::Duration;

use rusqlite::{OptionalExtension, params};

use crate::central::{
    self, Address, CentralError, Head, IdempotencyKey, Intent, Namespace, Op, Outcome, Pipeline,
    REGISTRY_ID, Scope,
};
use crate::sweep::{SweepError, SweepReport, Sweeper, host_incarnation};
use crate::{Identity, KvError, LocalStore, sql, unsigned};

/// Local schema version 6: the cached namespace definitions and the record
/// of the last pull.
pub(crate) fn migrate_to_v6(tx: &rusqlite::Transaction<'_>) -> Result<(), KvError> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS kv_namespace_cache (
             namespace TEXT PRIMARY KEY,
             registry_id TEXT NOT NULL,
             scope_kind TEXT NOT NULL CHECK (scope_kind IN ('shared', 'private')),
             shared_scope_id TEXT,
             core_binding TEXT,
             global_flag INTEGER NOT NULL CHECK (global_flag IN (0, 1)),
             blob_threshold INTEGER NOT NULL,
             policy_epoch INTEGER NOT NULL,
             CHECK ((scope_kind = 'shared') = (shared_scope_id IS NOT NULL))
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS kv_namespace_pull (
             singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
             registry_id TEXT NOT NULL,
             pulled_at_ms INTEGER NOT NULL,
             definitions INTEGER NOT NULL
         );",
    )
    .map_err(sql)
}

/// What the host's cache says about one namespace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    /// This host has never pulled definitions: Phase 1 local behaviour.
    NeverPulled,
    Known(Namespace),
    /// Pulled, and the namespace is not defined.
    Unknown,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Replaces the cache with central's current definitions, in one local
/// transaction. Returns how many definitions the host now holds.
pub fn pull_namespaces(store: &LocalStore, primary: &dyn Pipeline) -> Result<usize, SweepError> {
    let definitions = central::namespaces(primary, REGISTRY_ID)?;
    let tx = store.connection.unchecked_transaction().map_err(sql)?;
    tx.execute("DELETE FROM kv_namespace_cache", [])
        .map_err(sql)?;
    for definition in &definitions {
        let (kind, shared) = match &definition.scope {
            Scope::Shared(scope_id) => ("shared", Some(scope_id.as_str())),
            Scope::Private => ("private", None),
        };
        tx.execute(
            "INSERT INTO kv_namespace_cache (
                 namespace, registry_id, scope_kind, shared_scope_id, core_binding,
                 global_flag, blob_threshold, policy_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                definition.namespace,
                definition.registry_id,
                kind,
                shared,
                definition.core_binding,
                i64::from(definition.global_flag),
                i64::try_from(definition.blob_threshold).unwrap_or(i64::MAX),
                i64::try_from(definition.policy_epoch).unwrap_or(i64::MAX),
            ],
        )
        .map_err(sql)?;
    }
    tx.execute(
        "INSERT INTO kv_namespace_pull (singleton, registry_id, pulled_at_ms, definitions)
         VALUES (1, ?1, ?2, ?3)
         ON CONFLICT (singleton) DO UPDATE SET
             registry_id = excluded.registry_id,
             pulled_at_ms = excluded.pulled_at_ms,
             definitions = excluded.definitions",
        params![REGISTRY_ID, now_ms(), definitions.len() as i64],
    )
    .map_err(sql)?;
    tx.commit().map_err(sql)?;
    Ok(definitions.len())
}

/// When this host last pulled definitions, in Unix milliseconds.
pub fn last_pull_ms(store: &LocalStore) -> Result<Option<i64>, KvError> {
    store
        .connection
        .query_row(
            "SELECT pulled_at_ms FROM kv_namespace_pull WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql)
}

/// What the cache says about `namespace`.
pub fn lookup(store: &LocalStore, namespace: &str) -> Result<Lookup, KvError> {
    crate::validate_namespace(namespace)?;
    if last_pull_ms(store)?.is_none() {
        return Ok(Lookup::NeverPulled);
    }
    let found = store
        .connection
        .query_row(
            "SELECT registry_id, scope_kind, shared_scope_id, core_binding, global_flag,
                    blob_threshold, policy_epoch
             FROM kv_namespace_cache WHERE namespace = ?1",
            params![namespace],
            |row| {
                let kind: String = row.get(1)?;
                let shared: Option<String> = row.get(2)?;
                Ok(Namespace {
                    registry_id: row.get(0)?,
                    namespace: namespace.to_string(),
                    scope: match (kind.as_str(), shared) {
                        ("shared", Some(scope_id)) => Scope::Shared(scope_id),
                        _ => Scope::Private,
                    },
                    core_binding: row.get(3)?,
                    global_flag: row.get::<_, i64>(4)? == 1,
                    blob_threshold: unsigned(row.get(5)?),
                    policy_epoch: unsigned(row.get(6)?),
                })
            },
        )
        .optional()
        .map_err(sql)?;
    Ok(found.map_or(Lookup::Unknown, Lookup::Known))
}

/// Refuses `unknown_namespace` once the host has pulled definitions and
/// `namespace` is not among them. Before the first pull every valid name
/// passes, so a host that never syncs keeps Phase 1 behaviour.
pub fn require_known(store: &LocalStore, namespace: &str) -> Result<(), KvError> {
    match lookup(store, namespace)? {
        Lookup::Unknown => Err(KvError::UnknownNamespace(namespace.to_string())),
        Lookup::NeverPulled | Lookup::Known(_) => Ok(()),
    }
}

/// The sweep's resolver: the cached definition, or `unknown_namespace`.
fn cached_definition(store: &LocalStore, namespace: &str) -> central::Result<Namespace> {
    match lookup(store, namespace) {
        Ok(Lookup::Known(definition)) => Ok(definition),
        Ok(Lookup::NeverPulled | Lookup::Unknown) => Err(CentralError::UnknownNamespace {
            registry_id: REGISTRY_ID.to_string(),
            namespace: namespace.to_string(),
        }),
        Err(KvError::MalformedNamespace) => Err(CentralError::Malformed("malformed_namespace")),
        Err(error) => Err(CentralError::Store(error.to_string())),
    }
}

/// What one `fleet kv sync` did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Definitions the host holds after the pull.
    pub namespaces: usize,
    pub sweep: SweepReport,
}

/// Pulls namespace definitions, then sweeps `identity`'s pending intents
/// into central as `writer`. A pull that fails leaves the old cache and
/// sweeps nothing.
pub fn sync(
    store: &LocalStore,
    identity: &Identity<'_>,
    primary: &dyn Pipeline,
    writer: &str,
    sleep: &mut dyn FnMut(Duration),
) -> Result<SyncReport, SweepError> {
    let namespaces = pull_namespaces(store, primary)?;
    let resolve = |namespace: &str| cached_definition(store, namespace);
    let sweep = Sweeper {
        primary,
        writer,
        resolve: &resolve,
        sleep,
    }
    .sweep(store, identity)?;
    Ok(SyncReport { namespaces, sweep })
}

/// A refusal from a `--global` verb.
#[derive(Debug)]
pub enum GlobalError {
    Local(KvError),
    Central(CentralError),
    NotFound,
    /// The value is over the namespace's inline threshold; central has no
    /// blob plane before Phase 3.
    NeedsPhase3 {
        threshold: u64,
    },
    Conflict {
        expected: u64,
        actual: u64,
    },
}

impl GlobalError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Local(error) => error.code(),
            Self::Central(error) => error.code(),
            Self::NotFound => "not_found",
            Self::NeedsPhase3 { .. } => "pending_needs_phase3",
            Self::Conflict { .. } => "conflict",
        }
    }

    /// Central could not be reached or is not configured: exit 69, not 1.
    pub fn is_unavailable(&self) -> bool {
        matches!(
            self,
            Self::Central(CentralError::Unavailable(_) | CentralError::NotConfigured(_))
        )
    }
}

impl std::fmt::Display for GlobalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local(error) => write!(f, "{error}"),
            Self::Central(error) => write!(f, "{error}"),
            Self::NotFound => f.write_str("no live central value for this key"),
            Self::NeedsPhase3 { threshold } => write!(
                f,
                "the value is over this namespace's {threshold}-byte inline threshold, and central blob placement arrives in Phase 3"
            ),
            Self::Conflict { expected, actual } => write!(
                f,
                "expected central version {expected}, but central is at {actual}; the proposed value is kept as a conflict"
            ),
        }
    }
}

impl std::error::Error for GlobalError {}

impl From<KvError> for GlobalError {
    fn from(error: KvError) -> Self {
        Self::Local(error)
    }
}

impl From<CentralError> for GlobalError {
    fn from(error: CentralError) -> Self {
        Self::Central(error)
    }
}

/// The central address `identity` reaches for `namespace`/`key`, from the
/// authoritative definition.
fn address(
    primary: &dyn Pipeline,
    identity: &Identity<'_>,
    namespace: &str,
    key: &str,
) -> Result<(Namespace, Address), GlobalError> {
    crate::validate_namespace(namespace)?;
    crate::validate_key(key)?;
    let definition = central::resolve(primary, REGISTRY_ID, namespace)?;
    let scope_id = definition.scope_for(identity.owner_user_id, identity.context_id);
    let address = Address::new(&scope_id, namespace, key);
    Ok((definition, address))
}

/// `--global get`: the authoritative central head, bypassing the overlay and
/// any cache. Writes the value's bytes to `out`.
pub fn global_get(
    primary: &dyn Pipeline,
    identity: &Identity<'_>,
    namespace: &str,
    key: &str,
    out: &mut dyn Write,
) -> Result<u64, GlobalError> {
    let (_, address) = address(primary, identity, namespace, key)?;
    match central::head(primary, &address)? {
        Some(Head {
            live: true,
            value,
            version,
            ..
        }) => {
            out.write_all(&value)
                .map_err(|error| GlobalError::Local(KvError::Io(error)))?;
            Ok(version)
        }
        _ => Err(GlobalError::NotFound),
    }
}

/// One row of `--global list`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobalEntry {
    pub key: String,
    pub version: u64,
    pub live: bool,
}

/// `--global list`: the heads in the caller's scope of `namespace`.
pub fn global_list(
    primary: &dyn Pipeline,
    identity: &Identity<'_>,
    namespace: &str,
    include_tombstones: bool,
) -> Result<Vec<GlobalEntry>, GlobalError> {
    crate::validate_namespace(namespace)?;
    let definition = central::resolve(primary, REGISTRY_ID, namespace)?;
    let scope_id = definition.scope_for(identity.owner_user_id, identity.context_id);
    Ok(central::heads(primary, &scope_id, REGISTRY_ID, namespace)?
        .into_iter()
        .filter(|(_, head)| include_tombstones || head.live)
        .map(|(key, head)| GlobalEntry {
            key,
            version: head.version,
            live: head.live,
        })
        .collect())
}

/// `--global set` and `--global delete`: one CAS against the head the call
/// reads, or against `expect` when the caller names the version it saw.
/// Returns the committed version; a moved head is a typed conflict whose
/// body central keeps. Transient failures retry up to 5 times.
#[allow(clippy::too_many_arguments)]
pub fn global_write(
    store: &LocalStore,
    primary: &dyn Pipeline,
    identity: &Identity<'_>,
    namespace: &str,
    key: &str,
    op: Op,
    expect: Option<u64>,
    sleep: &mut dyn FnMut(Duration),
) -> Result<u64, GlobalError> {
    let (definition, address) = address(primary, identity, namespace, key)?;
    if let Op::Set(value) = &op
        && value.len() as u64 > definition.blob_threshold
    {
        return Err(GlobalError::NeedsPhase3 {
            threshold: definition.blob_threshold,
        });
    }
    let expected_version = match expect {
        Some(version) => version,
        None => central::head(primary, &address)?.map_or(0, |head| head.version),
    };
    let intent = Intent {
        address,
        expected_version,
        op,
        idempotency: IdempotencyKey {
            writer: identity.owner_user_id.to_string(),
            host_incarnation: host_incarnation(store)?,
            outbox_id: fresh_request_id(),
        },
    };
    let receipt = central::cas_with_retry(primary, &intent, sleep)?;
    match receipt.outcome {
        Outcome::Committed { version } => Ok(version),
        Outcome::Conflict {
            expected, actual, ..
        } => Err(GlobalError::Conflict { expected, actual }),
    }
}

/// Reads a `--global set --stdin` value, refusing past `limit` bytes
/// without buffering the rest.
pub fn read_bounded(reader: &mut dyn Read, limit: u64) -> Result<Vec<u8>, GlobalError> {
    let mut value = Vec::new();
    reader
        .take(limit.saturating_add(1))
        .read_to_end(&mut value)
        .map_err(|error| GlobalError::Local(KvError::Io(error)))?;
    if value.len() as u64 > limit {
        return Err(GlobalError::NeedsPhase3 { threshold: limit });
    }
    Ok(value)
}

/// A request id for a direct `--global` write: unique per call, so a retry
/// inside one call replays and a second call is a new request.
fn fresh_request_id() -> String {
    let mut hasher = blake3::Hasher::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    hasher.update(&now.as_nanos().to_le_bytes());
    hasher.update(&std::process::id().to_le_bytes());
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    hasher.update(
        &COUNTER
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .to_le_bytes(),
    );
    format!("global-{}", &hasher.finalize().to_hex()[..32])
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::central::{SqlitePrimary, define_namespace, history};

    fn primary() -> SqlitePrimary {
        let primary = SqlitePrimary::in_memory().unwrap();
        define_namespace(&primary, &Namespace::shared("status", "fleet-status")).unwrap();
        define_namespace(&primary, &Namespace::private("notes")).unwrap();
        primary
    }

    fn me() -> Identity<'static> {
        Identity::new("user-1", "ctx-1", "session-1")
    }

    fn no_sleep() -> impl FnMut(Duration) {
        |_| {}
    }

    #[test]
    fn central_sync_pull_caches_definitions_and_unknown_namespace_follows() {
        let primary = primary();
        let store = LocalStore::in_memory().unwrap();
        // Before any pull, every valid namespace works locally (Phase 1).
        assert_eq!(lookup(&store, "anything").unwrap(), Lookup::NeverPulled);
        require_known(&store, "anything").unwrap();
        assert_eq!(pull_namespaces(&store, &primary).unwrap(), 2);
        assert!(last_pull_ms(&store).unwrap().is_some());
        match lookup(&store, "status").unwrap() {
            Lookup::Known(definition) => {
                assert_eq!(definition.scope, Scope::Shared("fleet-status".into()));
                assert_eq!(definition.registry_id, REGISTRY_ID);
            }
            other => panic!("expected a definition, got {other:?}"),
        }
        assert_eq!(lookup(&store, "anything").unwrap(), Lookup::Unknown);
        let refused = require_known(&store, "anything").unwrap_err();
        assert_eq!(refused.code(), "unknown_namespace");
        assert!(matches!(
            require_known(&store, "Not_valid").unwrap_err(),
            KvError::MalformedNamespace
        ));
    }

    #[test]
    fn central_sync_pull_replaces_the_cache() {
        let primary = primary();
        let store = LocalStore::in_memory().unwrap();
        pull_namespaces(&store, &primary).unwrap();
        // A redefinition (new epoch) replaces the cached row.
        define_namespace(&primary, &Namespace::shared("status", "status-v2")).unwrap();
        pull_namespaces(&store, &primary).unwrap();
        let Lookup::Known(definition) = lookup(&store, "status").unwrap() else {
            panic!("status should be cached");
        };
        assert_eq!(definition.scope, Scope::Shared("status-v2".into()));
        assert_eq!(definition.policy_epoch, 2);
    }

    #[test]
    fn central_sync_sweeps_with_pulled_definitions() {
        let primary = primary();
        let store = LocalStore::in_memory().unwrap();
        store.set(&me(), "status", "agent", b"working").unwrap();
        store.set(&me(), "nowhere", "agent", b"lost").unwrap();
        let report = sync(&store, &me(), &primary, "user-1", &mut no_sleep()).unwrap();
        assert_eq!(report.namespaces, 2);
        assert_eq!(report.sweep.committed(), 1);
        assert_eq!(report.sweep.waiting.len(), 1);
        assert_eq!(report.sweep.waiting[0].reason, "unknown_namespace");
        let head = central::head(&primary, &Address::new("fleet-status", "status", "agent"))
            .unwrap()
            .unwrap();
        assert_eq!(head.value, b"working");
        // Syncing again pulls again and sends nothing new.
        let again = sync(&store, &me(), &primary, "user-1", &mut no_sleep()).unwrap();
        assert_eq!(again.sweep.committed(), 0);
    }

    #[test]
    fn central_sync_failed_pull_keeps_the_old_cache_and_sweeps_nothing() {
        struct Down;
        impl Pipeline for Down {
            fn transaction(&self, _: &[central::Statement]) -> central::Result<Vec<central::Rows>> {
                Err(CentralError::Unavailable("down".into()))
            }
        }
        let primary = primary();
        let store = LocalStore::in_memory().unwrap();
        pull_namespaces(&store, &primary).unwrap();
        store.set(&me(), "status", "agent", b"working").unwrap();
        let error = sync(&store, &me(), &Down, "user-1", &mut no_sleep()).unwrap_err();
        assert_eq!(error.code(), "unavailable");
        assert!(matches!(
            lookup(&store, "status").unwrap(),
            Lookup::Known(_)
        ));
    }

    #[test]
    fn central_global_set_get_list_delete_round_trip() {
        let primary = primary();
        let store = LocalStore::in_memory().unwrap();
        let version = global_write(
            &store,
            &primary,
            &me(),
            "status",
            "agent",
            Op::Set(b"one".to_vec()),
            None,
            &mut no_sleep(),
        )
        .unwrap();
        assert_eq!(version, 1);
        let mut out = Vec::new();
        assert_eq!(
            global_get(&primary, &me(), "status", "agent", &mut out).unwrap(),
            1
        );
        assert_eq!(out, b"one");
        // A global write never touches the local overlay.
        assert!(store.get(&me(), "status", "agent").unwrap().is_none());
        let listed = global_list(&primary, &me(), "status", false).unwrap();
        assert_eq!(
            listed,
            vec![GlobalEntry {
                key: "agent".into(),
                version: 1,
                live: true
            }]
        );
        let deleted = global_write(
            &store,
            &primary,
            &me(),
            "status",
            "agent",
            Op::Tombstone,
            None,
            &mut no_sleep(),
        )
        .unwrap();
        assert_eq!(deleted, 2);
        let mut out = Vec::new();
        assert!(matches!(
            global_get(&primary, &me(), "status", "agent", &mut out),
            Err(GlobalError::NotFound)
        ));
        assert!(
            global_list(&primary, &me(), "status", false)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            global_list(&primary, &me(), "status", true).unwrap().len(),
            1
        );
    }

    #[test]
    fn central_global_set_with_a_stale_expectation_is_a_kept_conflict() {
        let primary = primary();
        let store = LocalStore::in_memory().unwrap();
        let write = |value: &[u8], expect| {
            global_write(
                &store,
                &primary,
                &me(),
                "status",
                "claim",
                Op::Set(value.to_vec()),
                expect,
                &mut no_sleep(),
            )
        };
        assert_eq!(write(b"mine", Some(0)).unwrap(), 1);
        match write(b"also mine", Some(0)) {
            Err(GlobalError::Conflict { expected, actual }) => {
                assert_eq!((expected, actual), (0, 1));
            }
            other => panic!("expected a conflict, got {other:?}"),
        }
        let address = Address::new("fleet-status", "status", "claim");
        assert_eq!(history(&primary, &address).unwrap().len(), 1);
        let kept = central::conflicts(&primary, &address).unwrap();
        assert_eq!(kept[0].proposed, Op::Set(b"also mine".to_vec()));
    }

    #[test]
    fn central_global_private_scope_and_unknown_namespace() {
        let primary = primary();
        let store = LocalStore::in_memory().unwrap();
        let other = Identity::new("user-2", "ctx-1", "session-9");
        global_write(
            &store,
            &primary,
            &me(),
            "notes",
            "todo",
            Op::Set(b"mine".to_vec()),
            None,
            &mut no_sleep(),
        )
        .unwrap();
        let mut out = Vec::new();
        assert!(matches!(
            global_get(&primary, &other, "notes", "todo", &mut out),
            Err(GlobalError::NotFound)
        ));
        let unknown = global_get(&primary, &me(), "nowhere", "todo", &mut out).unwrap_err();
        assert_eq!(unknown.code(), "unknown_namespace");
        assert!(!unknown.is_unavailable());
    }

    #[test]
    fn central_global_over_threshold_is_pending_needs_phase3() {
        let primary = SqlitePrimary::in_memory().unwrap();
        let mut small = Namespace::shared("status", "fleet-status");
        small.blob_threshold = 4;
        define_namespace(&primary, &small).unwrap();
        let store = LocalStore::in_memory().unwrap();
        let refused = global_write(
            &store,
            &primary,
            &me(),
            "status",
            "big",
            Op::Set(b"12345".to_vec()),
            None,
            &mut no_sleep(),
        )
        .unwrap_err();
        assert_eq!(refused.code(), "pending_needs_phase3");
        assert!(
            central::head(&primary, &Address::new("fleet-status", "status", "big"))
                .unwrap()
                .is_none()
        );
        let mut reader: &[u8] = b"12345";
        assert_eq!(
            read_bounded(&mut reader, 4).unwrap_err().code(),
            "pending_needs_phase3"
        );
        let mut reader: &[u8] = b"1234";
        assert_eq!(read_bounded(&mut reader, 4).unwrap(), b"1234");
    }
}
