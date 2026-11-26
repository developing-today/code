//! The durable session sweep (design §4): merge a session's unsent local
//! intents into the central plane by per-key CAS.
//!
//! For each pending outbox intent, oldest first, the sweeper takes the local
//! write lock (the same lock `clear-override` takes, so a clear either
//! cancels the intent before it is sent or waits for the submission), checks
//! that the intent is still pending, submits one CAS keyed by `(writer, host
//! incarnation, outbox id)`, and only then records the receipt, advances the
//! key's central base and marks the intent decided. A crash anywhere before
//! that local commit leaves the intent pending; the next sweep retransmits
//! the same request and the central receipt replays the recorded result, so
//! a sweep never applies twice and never turns into a spurious conflict.
//!
//! A conflict is recorded centrally with both bodies retrievable; the key's
//! later intents wait (`blocked_by_conflict`) until a rebase, which is a new
//! intent.
//!
//! A value over the namespace's inline threshold, or one held in a working
//! file, goes through the blob plane (code#13318, design §9): the sweeper
//! streams its bytes to the trusted upload door, which verifies and
//! publishes them, then writes the reference by publish-CAS, so the head is
//! never a reference to bytes central has not published. The upload runs
//! before the local write lock is taken, so a long upload never blocks a
//! local write; an upload whose intent is then cleared leaves a published,
//! unreferenced blob that GC retires after its grace. With no blob plane
//! configured those values stay local (`pending_needs_phase3`).

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::time::Duration;

use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use crate::blob::{self, KvBlobRefV1, Partition, Placement};
use crate::blob_central::{self, PublishCas};
use crate::central::{
    self, Address, CentralError, IdempotencyKey, Intent, Namespace, Op, Outcome, Pipeline,
};
use crate::{Identity, KvError, LocalStore, busy_or_sql, sql, unsigned};

/// One blob the sweeper asks the upload door to verify and publish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlobUpload<'a> {
    pub registry_id: &'a str,
    pub namespace: &'a str,
    pub scope_id: &'a str,
    pub partition: &'a Partition,
    /// Lower-case hex BLAKE3-256 of the bytes: an assertion the door
    /// verifies, never authority.
    pub digest: &'a str,
    pub bytes_len: u64,
}

/// The trusted upload door (design §9): streams the bytes, which the door
/// verifies against the digest and length and publishes in the partition,
/// and answers the published reference.
pub trait BlobUploader {
    fn upload(&self, upload: &BlobUpload<'_>, bytes: &mut dyn Read)
    -> central::Result<KvBlobRefV1>;
}

/// Where the sweeper places blobs: the upload door. The partition is never
/// the host's choice: it comes from the namespace's placement policy on the
/// central definition (design §10), and a namespace with none waits.
pub struct BlobPlane<'a> {
    pub uploader: &'a dyn BlobUploader,
}

/// Local schema version 5: the host incarnation, each key's central base and
/// local copies of sweep receipts.
pub(crate) fn migrate_to_v5(tx: &Transaction<'_>) -> Result<(), KvError> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS kv_host (
             singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
             incarnation TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS kv_central_base (
             owner_user_id TEXT NOT NULL,
             context_id TEXT NOT NULL,
             partition_id TEXT NOT NULL,
             namespace TEXT NOT NULL,
             key TEXT NOT NULL,
             scope_id TEXT NOT NULL,
             version INTEGER NOT NULL,
             conflicted INTEGER NOT NULL DEFAULT 0 CHECK (conflicted IN (0, 1)),
             PRIMARY KEY (owner_user_id, context_id, partition_id, namespace, key)
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS kv_receipts_local (
             outbox_id INTEGER PRIMARY KEY,
             scope_id TEXT NOT NULL,
             namespace TEXT NOT NULL,
             key TEXT NOT NULL,
             outcome TEXT NOT NULL CHECK (outcome IN ('committed', 'conflict')),
             expected_version INTEGER NOT NULL,
             version INTEGER NOT NULL,
             conflict_id INTEGER,
             recorded_at_ms INTEGER NOT NULL
         );",
    )
    .map_err(sql)
}

/// This host's incarnation id, made once and kept in the store, so a reborn
/// sweeper reuses its own idempotency keys and never another host's.
pub fn host_incarnation(store: &LocalStore) -> Result<String, KvError> {
    let existing: Option<String> = store
        .connection
        .query_row(
            "SELECT incarnation FROM kv_host WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql)?;
    if let Some(incarnation) = existing {
        return Ok(incarnation);
    }
    store
        .connection
        .execute(
            "INSERT OR IGNORE INTO kv_host (singleton, incarnation) VALUES (1, ?1)",
            params![fresh_incarnation()],
        )
        .map_err(sql)?;
    store
        .connection
        .query_row(
            "SELECT incarnation FROM kv_host WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(sql)
}

fn fresh_incarnation() -> String {
    let mut hasher = blake3::Hasher::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    hasher.update(&now.as_nanos().to_le_bytes());
    hasher.update(&std::process::id().to_le_bytes());
    let stack_marker = 0u8;
    hasher.update(&(std::ptr::addr_of!(stack_marker) as usize).to_le_bytes());
    if let Ok(name) = std::env::var("HOSTNAME") {
        hasher.update(name.as_bytes());
    }
    let hex = hasher.finalize().to_hex();
    // Shaped like a UUID for readability; the bits are a hash.
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// One decided intent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Swept {
    pub outbox_id: i64,
    pub namespace: String,
    pub key: String,
    pub local_version: u64,
    pub outcome: Outcome,
    /// The central plane had already decided this request.
    pub replayed: bool,
}

/// A key the sweep left pending, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waiting {
    pub namespace: String,
    pub key: String,
    /// `pending_needs_phase3`, `blob_not_published`, `unknown_namespace` or
    /// `blocked_by_conflict`.
    pub reason: &'static str,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SweepReport {
    pub swept: Vec<Swept>,
    pub waiting: Vec<Waiting>,
}

impl SweepReport {
    pub fn committed(&self) -> usize {
        self.swept
            .iter()
            .filter(|swept| matches!(swept.outcome, Outcome::Committed { .. }))
            .count()
    }

    pub fn conflicts(&self) -> usize {
        self.swept.len() - self.committed()
    }
}

#[derive(Debug)]
pub enum SweepError {
    Local(KvError),
    Central(CentralError),
}

impl SweepError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Local(error) => error.code(),
            Self::Central(error) => error.code(),
        }
    }
}

impl std::fmt::Display for SweepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local(error) => write!(f, "{error}"),
            Self::Central(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for SweepError {}

impl From<KvError> for SweepError {
    fn from(error: KvError) -> Self {
        Self::Local(error)
    }
}

impl From<CentralError> for SweepError {
    fn from(error: CentralError) -> Self {
        Self::Central(error)
    }
}

struct Pending {
    id: i64,
    namespace: String,
    key: String,
    version: u64,
    /// Set with `--force-blob`: a blob whatever its size.
    force_blob: bool,
}

/// Who the sweep writes as, and how it finds namespace definitions.
pub struct Sweeper<'a> {
    pub primary: &'a dyn Pipeline,
    /// The authenticated writer recorded on every central row.
    pub writer: &'a str,
    pub resolve: &'a dyn Fn(&str) -> central::Result<Namespace>,
    pub sleep: &'a mut dyn FnMut(Duration),
    /// The blob plane; `None` keeps over-threshold values local.
    pub blobs: Option<&'a BlobPlane<'a>>,
}

/// What the sweep does with one intent's bytes before it takes the lock.
enum Staged {
    /// A tombstone, or bytes that fit an inline row.
    Inline,
    /// Bytes published through the upload door.
    Blob(KvBlobRefV1),
    Wait(&'static str),
}

impl Sweeper<'_> {
    /// Sweeps every pending intent of `identity`, oldest first. Stops at the
    /// first error that is not about one key (an unreachable primary, a local
    /// failure); every intent decided before it stays decided.
    pub fn sweep(
        &mut self,
        store: &LocalStore,
        identity: &Identity<'_>,
    ) -> Result<SweepReport, SweepError> {
        let incarnation = host_incarnation(store)?;
        let pending = pending_intents(store, identity)?;
        let mut definitions: HashMap<String, Option<Namespace>> = HashMap::new();
        let mut held: HashSet<(String, String)> = HashSet::new();
        // Keys whose earlier intent just conflicted: their next intent is
        // reported once as waiting for a rebase rather than silently skipped.
        let mut just_conflicted: HashSet<(String, String)> = HashSet::new();
        let mut report = SweepReport::default();
        for intent in pending {
            let pair = (intent.namespace.clone(), intent.key.clone());
            if held.contains(&pair) {
                if just_conflicted.remove(&pair) {
                    wait(&mut report, &intent, "blocked_by_conflict");
                }
                continue;
            }
            let definition = match definitions.get(&intent.namespace) {
                Some(found) => found.clone(),
                None => {
                    let found = match (self.resolve)(&intent.namespace) {
                        Ok(definition) => Some(definition),
                        Err(CentralError::UnknownNamespace { .. }) => None,
                        Err(error) => return Err(error.into()),
                    };
                    definitions.insert(intent.namespace.clone(), found.clone());
                    found
                }
            };
            let Some(definition) = definition else {
                wait(&mut report, &intent, "unknown_namespace");
                held.insert(pair);
                continue;
            };
            match self.sweep_one(store, identity, &incarnation, &intent, &definition)? {
                Step::Decided(swept) => {
                    if matches!(swept.outcome, Outcome::Conflict { .. }) {
                        just_conflicted.insert(pair.clone());
                        held.insert(pair);
                    }
                    report.swept.push(swept);
                }
                Step::Wait(reason) => {
                    wait(&mut report, &intent, reason);
                    held.insert(pair);
                }
                Step::Gone => {}
            }
        }
        Ok(report)
    }

    fn sweep_one(
        &mut self,
        store: &LocalStore,
        identity: &Identity<'_>,
        incarnation: &str,
        intent: &Pending,
        definition: &Namespace,
    ) -> Result<Step, SweepError> {
        let reference = match self.stage_blob(store, identity, intent, definition)? {
            Staged::Inline => None,
            Staged::Blob(reference) => Some(reference),
            Staged::Wait(reason) => return Ok(Step::Wait(reason)),
        };
        // The lock clear-override takes: held from the pending check until
        // the decision is recorded, so a clear cancels this intent before it
        // is sent or waits (bounded) for the submission to finish.
        let tx = Transaction::new_unchecked(&store.connection, TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        let status: Option<String> = tx
            .query_row(
                "SELECT status FROM kv_outbox WHERE id = ?1",
                params![intent.id],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql)?;
        if status.as_deref() != Some("pending") {
            return Ok(Step::Gone);
        }
        let tuple = params![
            identity.owner_user_id,
            identity.context_id,
            identity.partition_id,
            intent.namespace,
            intent.key
        ];
        let scope_id = definition.scope_for(identity.owner_user_id, identity.context_id);
        let base: Option<(String, i64, i64)> = tx
            .query_row(
                "SELECT scope_id, version, conflicted FROM kv_central_base
                 WHERE owner_user_id=?1 AND context_id=?2 AND partition_id=?3 AND namespace=?4 AND key=?5",
                tuple,
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(sql)?;
        let expected = match &base {
            Some((_, _, 1)) => return Ok(Step::Wait("blocked_by_conflict")),
            // A base in another scope (the namespace was redefined) says
            // nothing about this scope's head.
            Some((base_scope, version, _)) if *base_scope == scope_id => unsigned(*version),
            _ => 0,
        };
        let (value, state, working_id): (Vec<u8>, String, Option<String>) = tx
            .query_row(
                "SELECT value, state, working_id FROM kv_versions_local
                 WHERE owner_user_id=?1 AND context_id=?2 AND partition_id=?3 AND namespace=?4 AND key=?5
                   AND version=?6",
                params![
                    identity.owner_user_id,
                    identity.context_id,
                    identity.partition_id,
                    intent.namespace,
                    intent.key,
                    crate::signed(intent.version)
                ],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(sql)?;
        let op = if state == "tombstone" {
            Op::Tombstone
        } else if let Some(reference) = &reference {
            Op::Set(reference.encode())
        } else {
            if working_id.is_some() || value.len() as u64 > definition.blob_threshold {
                return Ok(Step::Wait("pending_needs_phase3"));
            }
            Op::Set(value)
        };
        let request = Intent {
            address: Address {
                scope_id: scope_id.clone(),
                registry_id: definition.registry_id.clone(),
                namespace: intent.namespace.clone(),
                key: intent.key.clone(),
            },
            expected_version: expected,
            op,
            idempotency: IdempotencyKey {
                writer: self.writer.to_string(),
                host_incarnation: incarnation.to_string(),
                outbox_id: intent.id.to_string(),
            },
        };
        // An error here drops the transaction: nothing local changes, the
        // intent stays pending, and the next sweep retransmits it.
        let receipt = match &reference {
            None => match central::cas_with_retry(self.primary, &request, &mut *self.sleep) {
                Ok(receipt) => receipt,
                // Central's threshold is lower than the definition this
                // sweep resolved: the next sweep resolves again and places
                // the value as a blob.
                Err(CentralError::Refused {
                    code: central::INLINE_ABOVE_CEILING,
                    ..
                }) => return Ok(Step::Wait(central::INLINE_ABOVE_CEILING)),
                Err(error) => return Err(error.into()),
            },
            Some(reference) => match blob_central::publish_cas_with_retry(
                self.primary,
                &request,
                reference,
                &mut *self.sleep,
            )? {
                PublishCas::Decided(receipt) => receipt,
                // Deleted under a fence between upload and CAS: the next
                // sweep uploads the bytes again.
                PublishCas::NotPublished => return Ok(Step::Wait("blob_not_published")),
            },
        };
        let (outcome, version, conflict_id, conflicted, base_version) = match receipt.outcome {
            Outcome::Committed { version } => ("committed", version, None, 0, version),
            Outcome::Conflict {
                actual,
                conflict_id,
                ..
            } => ("conflict", actual, Some(conflict_id), 1, expected),
        };
        tx.execute(
            "INSERT OR REPLACE INTO kv_receipts_local (
                 outbox_id, scope_id, namespace, key, outcome, expected_version, version,
                 conflict_id, recorded_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                intent.id,
                scope_id,
                intent.namespace,
                intent.key,
                outcome,
                crate::signed(expected),
                crate::signed(version),
                conflict_id,
                now_ms()
            ],
        )
        .map_err(sql)?;
        tx.execute(
            "INSERT INTO kv_central_base (
                 owner_user_id, context_id, partition_id, namespace, key, scope_id, version, conflicted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (owner_user_id, context_id, partition_id, namespace, key) DO UPDATE SET
                 scope_id = excluded.scope_id,
                 version = excluded.version,
                 conflicted = excluded.conflicted",
            params![
                identity.owner_user_id,
                identity.context_id,
                identity.partition_id,
                intent.namespace,
                intent.key,
                scope_id,
                crate::signed(base_version),
                conflicted
            ],
        )
        .map_err(sql)?;
        tx.execute(
            "UPDATE kv_outbox SET status = 'acked' WHERE id = ?1",
            params![intent.id],
        )
        .map_err(sql)?;
        tx.commit().map_err(sql)?;
        Ok(Step::Decided(Swept {
            outbox_id: intent.id,
            namespace: intent.namespace.clone(),
            key: intent.key.clone(),
            local_version: intent.version,
            outcome: receipt.outcome,
            replayed: receipt.replayed,
        }))
    }
}

impl Sweeper<'_> {
    /// Upload `intent`'s bytes when they belong in a blob. Reads the
    /// immutable version row without the write lock.
    fn stage_blob(
        &self,
        store: &LocalStore,
        identity: &Identity<'_>,
        intent: &Pending,
        definition: &Namespace,
    ) -> Result<Staged, SweepError> {
        let (value, state, working_id, working_len): (
            Vec<u8>,
            String,
            Option<String>,
            Option<i64>,
        ) = store
            .connection
            .query_row(
                "SELECT value, state, working_id, working_len FROM kv_versions_local
                     WHERE owner_user_id=?1 AND context_id=?2 AND partition_id=?3 AND namespace=?4
                       AND key=?5 AND version=?6",
                params![
                    identity.owner_user_id,
                    identity.context_id,
                    identity.partition_id,
                    intent.namespace,
                    intent.key,
                    crate::signed(intent.version)
                ],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(sql)?;
        if state == "tombstone" {
            return Ok(Staged::Inline);
        }
        let working = crate::working_ref(working_id, working_len);
        let len = working
            .as_ref()
            .map_or(value.len() as u64, |reference| reference.len);
        // A working file is never read whole into a row, whatever the knob.
        if working.is_none()
            && blob::place(len, definition.blob_threshold, intent.force_blob) == Placement::Inline
        {
            return Ok(Staged::Inline);
        }
        let Some(plane) = self.blobs else {
            return Ok(Staged::Wait("pending_needs_phase3"));
        };
        let Some(partition) = definition.placement.as_ref() else {
            return Ok(Staged::Wait("no_blob_placement"));
        };
        let (digest, mut reader): (String, Box<dyn Read>) = match &working {
            Some(reference) => {
                let dir = store
                    .working_dir
                    .as_deref()
                    .ok_or_else(|| KvError::WorkingFileMissing(reference.id.clone()))?;
                let file = crate::working::open(dir, reference).map_err(KvError::from)?;
                (reference.id.clone(), Box::new(file))
            }
            None => (
                blake3::hash(&value).to_hex().to_string(),
                Box::new(std::io::Cursor::new(value)),
            ),
        };
        let scope_id = definition.scope_for(identity.owner_user_id, identity.context_id);
        let upload = BlobUpload {
            registry_id: &definition.registry_id,
            namespace: &intent.namespace,
            scope_id: &scope_id,
            partition,
            digest: &digest,
            bytes_len: len,
        };
        let reference = plane.uploader.upload(&upload, &mut reader)?;
        if reference.digest != digest
            || reference.bytes_len != len
            || &reference.partition != partition
        {
            return Err(CentralError::Store(format!(
                "the upload door answered a reference to other bytes: {} {} in {}",
                reference.digest, reference.bytes_len, reference.partition
            ))
            .into());
        }
        Ok(Staged::Blob(reference))
    }
}

/// What [`rebase`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rebased {
    /// The central head version the key now builds on (0: absent).
    pub onto: u64,
    /// The local version re-queued against that base, if the overlay has
    /// one. `None` means the key now follows central.
    pub requeued: Option<u64>,
}

/// Resolves a conflicted key by a new intent (design D007): reads the
/// central head, makes its version the key's base and clears the conflict
/// mark, cancels the key's pending intents, and queues one new intent for
/// the overlay's current version against that base. With no overlay row
/// (after `clear-override`) nothing is queued and the key follows central.
/// Never overwrites silently: the re-queued intent is an ordinary CAS, so a
/// head that moves again is another recorded conflict.
pub fn rebase(
    store: &LocalStore,
    identity: &Identity<'_>,
    primary: &dyn Pipeline,
    definition: &Namespace,
    namespace: &str,
    key: &str,
) -> Result<Rebased, SweepError> {
    let scope_id = definition.scope_for(identity.owner_user_id, identity.context_id);
    let address = Address {
        scope_id: scope_id.clone(),
        registry_id: definition.registry_id.clone(),
        namespace: namespace.to_string(),
        key: key.to_string(),
    };
    let onto = central::head(primary, &address)?.map_or(0, |head| head.version);
    let tx = Transaction::new_unchecked(&store.connection, TransactionBehavior::Immediate)
        .map_err(busy_or_sql)?;
    let tuple = params![
        identity.owner_user_id,
        identity.context_id,
        identity.partition_id,
        namespace,
        key
    ];
    tx.execute(
        "INSERT INTO kv_central_base (
             owner_user_id, context_id, partition_id, namespace, key, scope_id, version, conflicted)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)
         ON CONFLICT (owner_user_id, context_id, partition_id, namespace, key) DO UPDATE SET
             scope_id = excluded.scope_id,
             version = excluded.version,
             conflicted = 0",
        params![
            identity.owner_user_id,
            identity.context_id,
            identity.partition_id,
            namespace,
            key,
            scope_id,
            crate::signed(onto)
        ],
    )
    .map_err(sql)?;
    tx.execute(
        "UPDATE kv_outbox SET status = 'cancelled'
         WHERE owner_user_id=?1 AND context_id=?2 AND partition_id=?3 AND namespace=?4 AND key=?5
           AND status = 'pending'",
        tuple,
    )
    .map_err(sql)?;
    let local: Option<i64> = tx
        .query_row(
            "SELECT version FROM kv_local
             WHERE owner_user_id=?1 AND context_id=?2 AND partition_id=?3 AND namespace=?4 AND key=?5",
            tuple,
            |row| row.get(0),
        )
        .optional()
        .map_err(sql)?;
    if let Some(version) = local {
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
        tx.execute(
            "INSERT INTO kv_outbox (
                 owner_user_id, context_id, partition_id, namespace, key, version, op, generation, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'rebase', ?7, 'pending')",
            params![
                identity.owner_user_id,
                identity.context_id,
                identity.partition_id,
                namespace,
                key,
                version,
                generation
            ],
        )
        .map_err(sql)?;
    }
    tx.commit().map_err(sql)?;
    Ok(Rebased {
        onto,
        requeued: local.map(unsigned),
    })
}

enum Step {
    Decided(Swept),
    Wait(&'static str),
    /// Cancelled or decided since the pending list was read.
    Gone,
}

fn wait(report: &mut SweepReport, intent: &Pending, reason: &'static str) {
    report.waiting.push(Waiting {
        namespace: intent.namespace.clone(),
        key: intent.key.clone(),
        reason,
    });
}

fn pending_intents(store: &LocalStore, identity: &Identity<'_>) -> Result<Vec<Pending>, KvError> {
    let mut statement = store
        .connection
        .prepare(
            "SELECT id, namespace, key, version, op FROM kv_outbox
             WHERE owner_user_id=?1 AND context_id=?2 AND partition_id=?3 AND status='pending'
             ORDER BY id",
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
                Ok(Pending {
                    id: row.get(0)?,
                    namespace: row.get(1)?,
                    key: row.get(2)?,
                    version: unsigned(row.get(3)?),
                    force_blob: row.get::<_, String>(4)? == crate::OP_SET_BLOB,
                })
            },
        )
        .map_err(sql)?;
    rows.collect::<Result<_, _>>().map_err(sql)
}

fn now_ms() -> i64 {
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
        REGISTRY_ID, Rows, SqlitePrimary, Statement, conflicts, define_namespace, head, history,
        private_scope_id,
    };
    use std::sync::Mutex;

    fn session(partition: &str) -> Identity<'_> {
        Identity::new("drew", "laptop", partition)
    }

    fn primary_with_namespaces() -> SqlitePrimary {
        let primary = SqlitePrimary::in_memory().unwrap();
        define_namespace(&primary, &Namespace::shared("status", "shared:team")).unwrap();
        define_namespace(&primary, &Namespace::private("notes")).unwrap();
        primary
    }

    fn sweep(
        primary: &dyn Pipeline,
        resolver: &dyn Pipeline,
        store: &LocalStore,
        identity: &Identity<'_>,
    ) -> Result<SweepReport, SweepError> {
        let resolve = |namespace: &str| central::resolve(resolver, REGISTRY_ID, namespace);
        let mut sleep = |_: Duration| {};
        Sweeper {
            primary,
            writer: "drew",
            resolve: &resolve,
            sleep: &mut sleep,
            blobs: None,
        }
        .sweep(store, identity)
    }

    fn shared(key: &str) -> Address {
        Address::new("shared:team", "status", key)
    }

    fn pending(store: &LocalStore, identity: &Identity<'_>, namespace: &str, key: &str) -> usize {
        store
            .outbox(identity, namespace, key)
            .unwrap()
            .iter()
            .filter(|intent| intent.status == "pending")
            .count()
    }

    #[test]
    fn central_sweep_commits_each_intent_once() {
        let primary = primary_with_namespaces();
        let store = LocalStore::in_memory().unwrap();
        let me = session("s1");
        store.set(&me, "status", "agent", b"starting").unwrap();
        store.set(&me, "status", "agent", b"working").unwrap();
        store.delete(&me, "status", "agent").unwrap();
        let report = sweep(&primary, &primary, &store, &me).unwrap();
        assert_eq!((report.committed(), report.conflicts()), (3, 0));
        let versions = history(&primary, &shared("agent")).unwrap();
        assert_eq!(versions.len(), 3);
        assert_eq!(versions[1].value, b"working");
        assert!(!head(&primary, &shared("agent")).unwrap().unwrap().live);
        // Re-running is safe: nothing is pending, central is unchanged.
        let again = sweep(&primary, &primary, &store, &me).unwrap();
        assert_eq!(again, SweepReport::default());
        assert_eq!(history(&primary, &shared("agent")).unwrap().len(), 3);
        assert_eq!(pending(&store, &me, "status", "agent"), 0);
    }

    #[test]
    fn central_sweep_two_sessions_racing_one_key() {
        let primary = primary_with_namespaces();
        let store = LocalStore::in_memory().unwrap();
        let (one, two) = (session("s1"), session("s2"));
        store.set(&one, "status", "claim", b"one").unwrap();
        store.set(&two, "status", "claim", b"two").unwrap();
        store.set(&two, "status", "claim", b"two again").unwrap();
        assert_eq!(
            sweep(&primary, &primary, &store, &one).unwrap().committed(),
            1
        );
        let report = sweep(&primary, &primary, &store, &two).unwrap();
        assert_eq!(report.conflicts(), 1);
        assert!(matches!(
            report.swept[0].outcome,
            Outcome::Conflict {
                expected: 0,
                actual: 1,
                ..
            }
        ));
        // The later intent waits for a rebase instead of conflicting again.
        assert_eq!(
            report.waiting,
            vec![Waiting {
                namespace: "status".into(),
                key: "claim".into(),
                reason: "blocked_by_conflict",
            }]
        );
        assert_eq!(pending(&store, &two, "status", "claim"), 1);
        // One winner, and the loser's body is retrievable.
        assert_eq!(
            head(&primary, &shared("claim")).unwrap().unwrap().value,
            b"one"
        );
        let conflicts = conflicts(&primary, &shared("claim")).unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].proposed, Op::Set(b"two".to_vec()));
        // Sweeping again neither re-sends nor adds a conflict.
        let again = sweep(&primary, &primary, &store, &two).unwrap();
        assert!(again.swept.is_empty());
        assert_eq!(
            central::conflicts(&primary, &shared("claim"))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn central_sweep_rebase_requeues_the_local_head_against_the_winner() {
        let primary = primary_with_namespaces();
        let store = LocalStore::in_memory().unwrap();
        let (one, two) = (session("s1"), session("s2"));
        store.set(&one, "status", "claim", b"one").unwrap();
        store.set(&two, "status", "claim", b"two").unwrap();
        store.set(&two, "status", "claim", b"two again").unwrap();
        sweep(&primary, &primary, &store, &one).unwrap();
        assert_eq!(
            sweep(&primary, &primary, &store, &two).unwrap().conflicts(),
            1
        );
        let definition = central::resolve(&primary, central::REGISTRY_ID, "status").unwrap();
        let rebased = rebase(&store, &two, &primary, &definition, "status", "claim").unwrap();
        assert_eq!(
            rebased,
            Rebased {
                onto: 1,
                requeued: Some(2)
            }
        );
        // Exactly one intent is pending: the re-proposed local head.
        assert_eq!(pending(&store, &two, "status", "claim"), 1);
        let report = sweep(&primary, &primary, &store, &two).unwrap();
        assert_eq!(report.committed(), 1, "{report:?}");
        let winner = head(&primary, &shared("claim")).unwrap().unwrap();
        assert_eq!(
            (winner.version, winner.value.as_slice()),
            (2, &b"two again"[..])
        );
        // The first body stays a retrievable conflict; nothing was lost.
        assert_eq!(conflicts(&primary, &shared("claim")).unwrap().len(), 1);
    }

    #[test]
    fn central_sweep_rebase_after_clear_follows_central() {
        let primary = primary_with_namespaces();
        let store = LocalStore::in_memory().unwrap();
        let (one, two) = (session("s1"), session("s2"));
        store.set(&one, "status", "claim", b"one").unwrap();
        store.set(&two, "status", "claim", b"two").unwrap();
        sweep(&primary, &primary, &store, &one).unwrap();
        sweep(&primary, &primary, &store, &two).unwrap();
        store.clear_override(&two, "status", "claim").unwrap();
        let definition = central::resolve(&primary, central::REGISTRY_ID, "status").unwrap();
        let rebased = rebase(&store, &two, &primary, &definition, "status", "claim").unwrap();
        assert_eq!(rebased.requeued, None);
        assert_eq!(pending(&store, &two, "status", "claim"), 0);
        // A later write sweeps against the adopted base instead of waiting.
        store.set(&two, "status", "claim", b"later").unwrap();
        assert_eq!(
            sweep(&primary, &primary, &store, &two).unwrap().committed(),
            1
        );
        assert_eq!(
            head(&primary, &shared("claim")).unwrap().unwrap().version,
            2
        );
    }

    /// Commits, then loses the first `failures` answers.
    struct LosesAnswers<'a> {
        inner: &'a SqlitePrimary,
        failures: Mutex<u32>,
    }

    impl Pipeline for LosesAnswers<'_> {
        fn transaction(&self, statements: &[Statement]) -> central::Result<Vec<Rows>> {
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
    fn central_sweep_crash_between_cas_and_ack_replays() {
        let primary = primary_with_namespaces();
        let store = LocalStore::in_memory().unwrap();
        let me = session("s1");
        store.set(&me, "status", "agent", b"done").unwrap();
        let lossy = LosesAnswers {
            inner: &primary,
            failures: Mutex::new(central::MAX_ATTEMPTS),
        };
        let error = sweep(&lossy, &primary, &store, &me).unwrap_err();
        assert_eq!(error.code(), "unavailable");
        // Central committed, but nothing was acked locally.
        assert_eq!(
            head(&primary, &shared("agent")).unwrap().unwrap().version,
            1
        );
        assert_eq!(pending(&store, &me, "status", "agent"), 1);
        let report = sweep(&primary, &primary, &store, &me).unwrap();
        assert_eq!(report.committed(), 1);
        assert!(report.swept[0].replayed);
        assert!(conflicts(&primary, &shared("agent")).unwrap().is_empty());
        assert_eq!(history(&primary, &shared("agent")).unwrap().len(), 1);
        assert_eq!(pending(&store, &me, "status", "agent"), 0);
    }

    #[test]
    fn central_sweep_clear_versus_sweep() {
        let primary = primary_with_namespaces();
        let store = LocalStore::in_memory().unwrap();
        let me = session("s1");
        store.set(&me, "status", "draft", b"oops").unwrap();
        assert!(store.clear_override(&me, "status", "draft").unwrap());
        let report = sweep(&primary, &primary, &store, &me).unwrap();
        assert_eq!(report, SweepReport::default());
        assert_eq!(head(&primary, &shared("draft")).unwrap(), None);
    }

    #[test]
    fn central_sweep_keeps_over_threshold_values_local() {
        let primary = primary_with_namespaces();
        let mut tight = Namespace::shared("status", "shared:team");
        tight.blob_threshold = 4;
        define_namespace(&primary, &tight).unwrap();
        let store = LocalStore::in_memory().unwrap();
        let me = session("s1");
        store.set(&me, "status", "big", b"more than four").unwrap();
        store.set(&me, "status", "small", b"ok").unwrap();
        let report = sweep(&primary, &primary, &store, &me).unwrap();
        assert_eq!(report.committed(), 1);
        assert_eq!(report.waiting[0].reason, "pending_needs_phase3");
        assert_eq!(head(&primary, &shared("big")).unwrap(), None);
        assert_eq!(pending(&store, &me, "status", "big"), 1);
    }

    #[test]
    fn central_sweep_waits_when_central_holds_a_lower_threshold() {
        // The host resolved the default threshold; central has narrowed it.
        let resolver = primary_with_namespaces();
        let primary = SqlitePrimary::in_memory().unwrap();
        let mut tight = Namespace::shared("status", "shared:team");
        tight.blob_threshold = 4;
        define_namespace(&primary, &tight).unwrap();
        let store = LocalStore::in_memory().unwrap();
        let me = session("s1");
        store.set(&me, "status", "big", b"more than four").unwrap();
        let report = sweep(&primary, &resolver, &store, &me).unwrap();
        assert_eq!(report.committed(), 0);
        assert_eq!(report.waiting[0].reason, "inline_above_ceiling");
        assert_eq!(head(&primary, &shared("big")).unwrap(), None);
        assert_eq!(pending(&store, &me, "status", "big"), 1);
    }

    /// The upload door, as the hosted worker runs it: verify the bytes
    /// against the assertion, then publish them in the partition.
    struct Door<'a> {
        primary: &'a SqlitePrimary,
        /// Publish nothing: the reference names bytes central never saw.
        skip_publish: bool,
        uploads: Mutex<Vec<(String, u64, String)>>,
    }

    impl<'a> Door<'a> {
        fn new(primary: &'a SqlitePrimary) -> Self {
            Self {
                primary,
                skip_publish: false,
                uploads: Mutex::new(Vec::new()),
            }
        }
    }

    impl BlobUploader for Door<'_> {
        fn upload(
            &self,
            upload: &BlobUpload<'_>,
            bytes: &mut dyn Read,
        ) -> central::Result<KvBlobRefV1> {
            let mut body = Vec::new();
            bytes
                .read_to_end(&mut body)
                .map_err(|error| CentralError::Unavailable(error.to_string()))?;
            if body.len() as u64 != upload.bytes_len
                || blake3::hash(&body).to_hex().as_str() != upload.digest
            {
                return Err(CentralError::Refused {
                    code: "central_refused",
                    message: "blob_digest_mismatch".into(),
                });
            }
            let reference = KvBlobRefV1 {
                digest: upload.digest.to_string(),
                bytes_len: upload.bytes_len,
                partition: upload.partition.clone(),
                locator: format!(
                    "r2:kv/blobs/{}/{}/{}",
                    upload.partition.jurisdiction, upload.partition.tier, upload.digest
                ),
            };
            if !self.skip_publish {
                blob_central::mark_published(self.primary, &reference)?;
            }
            self.uploads.lock().unwrap().push((
                upload.scope_id.to_string(),
                upload.bytes_len,
                upload.digest.to_string(),
            ));
            Ok(reference)
        }
    }

    fn sweep_with(
        primary: &SqlitePrimary,
        store: &LocalStore,
        identity: &Identity<'_>,
        plane: &BlobPlane<'_>,
    ) -> Result<SweepReport, SweepError> {
        let resolve = |namespace: &str| central::resolve(primary, REGISTRY_ID, namespace);
        let mut sleep = |_: Duration| {};
        Sweeper {
            primary,
            writer: "drew",
            resolve: &resolve,
            sleep: &mut sleep,
            blobs: Some(plane),
        }
        .sweep(store, identity)
    }

    /// The "status" namespace placed in eu/standard, with this threshold.
    fn placed_primary(threshold: Option<u64>) -> SqlitePrimary {
        let primary = primary_with_namespaces();
        let mut placed = Namespace::shared("status", "shared:team");
        if let Some(threshold) = threshold {
            placed.blob_threshold = threshold;
        }
        placed.placement = Some(Partition::new("eu", "standard"));
        define_namespace(&primary, &placed).unwrap();
        primary
    }

    fn tight_primary() -> SqlitePrimary {
        placed_primary(Some(4))
    }

    #[test]
    fn central_sweep_waits_when_the_namespace_has_no_blob_placement() {
        let primary = primary_with_namespaces();
        let mut tight = Namespace::shared("status", "shared:team");
        tight.blob_threshold = 4;
        define_namespace(&primary, &tight).unwrap();
        let door = Door::new(&primary);
        let plane = BlobPlane { uploader: &door };
        let store = LocalStore::in_memory().unwrap();
        let me = session("s1");
        store.set(&me, "status", "big", b"more than four").unwrap();
        store.set(&me, "status", "small", b"ok").unwrap();
        let report = sweep_with(&primary, &store, &me, &plane).unwrap();
        // The small value lands; the big one never picks a partition itself.
        assert_eq!(report.committed(), 1, "{report:?}");
        assert_eq!(report.waiting[0].reason, "no_blob_placement");
        assert!(door.uploads.lock().unwrap().is_empty());
        assert_eq!(head(&primary, &shared("big")).unwrap(), None);
        assert_eq!(pending(&store, &me, "status", "big"), 1);
    }

    #[test]
    fn central_sweep_uploads_into_the_placed_partition() {
        let primary = primary_with_namespaces();
        let mut placed = Namespace::shared("status", "shared:team");
        placed.blob_threshold = 4;
        placed.placement = Some(Partition::new("us", "cold"));
        define_namespace(&primary, &placed).unwrap();
        let door = Door::new(&primary);
        let plane = BlobPlane { uploader: &door };
        let store = LocalStore::in_memory().unwrap();
        let me = session("s1");
        store.set(&me, "status", "big", b"more than four").unwrap();
        assert_eq!(
            sweep_with(&primary, &store, &me, &plane)
                .unwrap()
                .committed(),
            1
        );
        let reference =
            KvBlobRefV1::decode(&head(&primary, &shared("big")).unwrap().unwrap().value).unwrap();
        assert_eq!(reference.partition, Partition::new("us", "cold"));
    }

    #[test]
    fn central_sweep_places_over_threshold_values_as_published_blobs() {
        let primary = tight_primary();
        let door = Door::new(&primary);
        let partition = Partition::new("eu", "standard");
        let plane = BlobPlane { uploader: &door };
        let store = LocalStore::in_memory().unwrap();
        let me = session("s1");
        store.set(&me, "status", "big", b"more than four").unwrap();
        store.set(&me, "status", "small", b"ok").unwrap();
        let report = sweep_with(&primary, &store, &me, &plane).unwrap();
        assert_eq!(report.committed(), 2, "{report:?}");
        assert!(report.waiting.is_empty());
        // Only the big value went through the door, in the namespace's scope.
        let digest = blake3::hash(b"more than four").to_hex().to_string();
        assert_eq!(
            *door.uploads.lock().unwrap(),
            vec![("shared:team".to_string(), 14, digest.clone())]
        );
        // Its head is the published reference; the small one stays inline.
        let big = head(&primary, &shared("big")).unwrap().unwrap();
        let reference = KvBlobRefV1::decode(&big.value).unwrap();
        assert_eq!(reference.digest, digest);
        assert_eq!(reference.partition, partition);
        assert_eq!(
            head(&primary, &shared("small")).unwrap().unwrap().value,
            b"ok"
        );
        // The reference is a GC root of its partition, and of no other.
        assert!(
            blob_central::roots(&primary, &partition)
                .unwrap()
                .content_ids
                .contains(&digest)
        );
        assert!(
            blob_central::roots(&primary, &Partition::new("us", "standard"))
                .unwrap()
                .content_ids
                .is_empty()
        );
        // A second sweep has nothing left to send.
        assert_eq!(pending(&store, &me, "status", "big"), 0);
        let again = sweep_with(&primary, &store, &me, &plane).unwrap();
        assert!(again.swept.is_empty() && again.waiting.is_empty());
        assert_eq!(door.uploads.lock().unwrap().len(), 1);
    }

    #[test]
    fn central_sweep_streams_working_files_through_the_door() {
        let dir = std::env::temp_dir().join(format!(
            "fleet-kv-sweep-blob-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let primary = placed_primary(None);
        let door = Door::new(&primary);
        let plane = BlobPlane { uploader: &door };
        let store = LocalStore::open(dir.join("kv.db")).unwrap();
        let me = session("s1");
        let big: Vec<u8> = (0..(crate::working::INLINE_LIMIT + 4096))
            .map(|i| (i % 251) as u8)
            .collect();
        store.set(&me, "status", "large", &big).unwrap();
        let report = sweep_with(&primary, &store, &me, &plane).unwrap();
        assert_eq!(report.committed(), 1, "{report:?}");
        let reference =
            KvBlobRefV1::decode(&head(&primary, &shared("large")).unwrap().unwrap().value).unwrap();
        assert_eq!(reference.bytes_len, big.len() as u64);
        assert_eq!(reference.digest, blake3::hash(&big).to_hex().as_str());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn central_sweep_waits_when_the_blob_is_not_published() {
        let primary = tight_primary();
        let mut door = Door::new(&primary);
        door.skip_publish = true;
        let plane = BlobPlane { uploader: &door };
        let store = LocalStore::in_memory().unwrap();
        let me = session("s1");
        store.set(&me, "status", "big", b"more than four").unwrap();
        let report = sweep_with(&primary, &store, &me, &plane).unwrap();
        assert_eq!(report.committed(), 0);
        assert_eq!(report.waiting[0].reason, "blob_not_published");
        // Nothing was written, and the intent is still pending.
        assert_eq!(head(&primary, &shared("big")).unwrap(), None);
        assert_eq!(pending(&store, &me, "status", "big"), 1);
        // Once the door publishes, the next sweep lands it.
        door.skip_publish = false;
        let plane = BlobPlane { uploader: &door };
        assert_eq!(
            sweep_with(&primary, &store, &me, &plane)
                .unwrap()
                .committed(),
            1
        );
    }

    #[test]
    fn central_sweep_force_blob_places_a_small_value_in_a_blob() {
        let primary = placed_primary(None);
        let store = LocalStore::in_memory().unwrap();
        let me = session("s1");
        store.set_forcing_blob(&me, "status", "tiny", b"x").unwrap();
        store.set(&me, "status", "plain", b"x").unwrap();
        // With no blob plane the forced value waits; the plain one lands.
        let report = sweep(&primary, &primary, &store, &me).unwrap();
        assert_eq!(report.committed(), 1);
        assert_eq!(report.waiting[0].reason, "pending_needs_phase3");
        assert_eq!(head(&primary, &shared("tiny")).unwrap(), None);
        let door = Door::new(&primary);
        let plane = BlobPlane { uploader: &door };
        assert_eq!(
            sweep_with(&primary, &store, &me, &plane)
                .unwrap()
                .committed(),
            1
        );
        let reference =
            KvBlobRefV1::decode(&head(&primary, &shared("tiny")).unwrap().unwrap().value).unwrap();
        assert_eq!(reference.bytes_len, 1);
        assert_eq!(
            head(&primary, &shared("plain")).unwrap().unwrap().value,
            b"x"
        );
        // The local row reads back as the bytes either way.
        assert_eq!(
            store.get(&me, "status", "tiny").unwrap(),
            Some(b"x".to_vec())
        );
        // A later plain set of the same key is inline again.
        store.set(&me, "status", "tiny", b"y").unwrap();
        sweep_with(&primary, &store, &me, &plane).unwrap();
        assert_eq!(
            head(&primary, &shared("tiny")).unwrap().unwrap().value,
            b"y"
        );
        assert_eq!(door.uploads.lock().unwrap().len(), 1);
    }

    #[test]
    fn central_sweep_refuses_a_reference_to_other_bytes() {
        struct Liar<'a>(Door<'a>);
        impl BlobUploader for Liar<'_> {
            fn upload(
                &self,
                upload: &BlobUpload<'_>,
                bytes: &mut dyn Read,
            ) -> central::Result<KvBlobRefV1> {
                let mut reference = self.0.upload(upload, bytes)?;
                reference.partition = Partition::new("us", "standard");
                Ok(reference)
            }
        }
        let primary = tight_primary();
        let liar = Liar(Door::new(&primary));
        let plane = BlobPlane { uploader: &liar };
        let store = LocalStore::in_memory().unwrap();
        let me = session("s1");
        store.set(&me, "status", "big", b"more than four").unwrap();
        let error = sweep_with(&primary, &store, &me, &plane).unwrap_err();
        assert!(
            matches!(error, SweepError::Central(CentralError::Store(_))),
            "{error:?}"
        );
        assert_eq!(head(&primary, &shared("big")).unwrap(), None);
        assert_eq!(pending(&store, &me, "status", "big"), 1);
    }

    #[test]
    fn central_sweep_blob_tombstones_stay_inline() {
        let primary = tight_primary();
        let door = Door::new(&primary);
        let plane = BlobPlane { uploader: &door };
        let store = LocalStore::in_memory().unwrap();
        let me = session("s1");
        store.set(&me, "status", "big", b"more than four").unwrap();
        store.delete(&me, "status", "big").unwrap();
        let report = sweep_with(&primary, &store, &me, &plane).unwrap();
        assert_eq!(report.committed(), 2);
        assert!(!head(&primary, &shared("big")).unwrap().unwrap().live);
        assert_eq!(door.uploads.lock().unwrap().len(), 1);
    }

    #[test]
    fn central_sweep_waits_on_unknown_namespaces() {
        let primary = primary_with_namespaces();
        let store = LocalStore::in_memory().unwrap();
        let me = session("s1");
        store.set(&me, "undefined", "k", b"v").unwrap();
        let report = sweep(&primary, &primary, &store, &me).unwrap();
        assert_eq!(report.waiting[0].reason, "unknown_namespace");
        assert_eq!(pending(&store, &me, "undefined", "k"), 1);
    }

    #[test]
    fn central_sweep_private_namespaces_stay_per_identity() {
        let primary = primary_with_namespaces();
        let store = LocalStore::in_memory().unwrap();
        let drew = Identity::new("drew", "laptop", "s1");
        let felix = Identity::new("felix", "laptop", "s1");
        store.set(&drew, "notes", "todo", b"drew's").unwrap();
        store.set(&felix, "notes", "todo", b"felix's").unwrap();
        assert_eq!(
            sweep(&primary, &primary, &store, &drew)
                .unwrap()
                .committed(),
            1
        );
        assert_eq!(
            sweep(&primary, &primary, &store, &felix)
                .unwrap()
                .committed(),
            1
        );
        let drews = Address::new(&private_scope_id("drew", "laptop"), "notes", "todo");
        let felixs = Address::new(&private_scope_id("felix", "laptop"), "notes", "todo");
        assert_eq!(head(&primary, &drews).unwrap().unwrap().value, b"drew's");
        assert_eq!(head(&primary, &felixs).unwrap().unwrap().value, b"felix's");
    }

    #[test]
    fn central_sweep_host_incarnation_survives_reopen() {
        let path = std::env::temp_dir().join(format!(
            "fleet-kv-incarnation-{}-{}.db",
            std::process::id(),
            fresh_incarnation()
        ));
        let first = host_incarnation(&LocalStore::open(&path).unwrap()).unwrap();
        let again = host_incarnation(&LocalStore::open(&path).unwrap()).unwrap();
        assert_eq!(first, again);
        assert_eq!(first.len(), 36);
        let other = host_incarnation(&LocalStore::in_memory().unwrap()).unwrap();
        assert_ne!(first, other);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(crate::working_dir_for(&path));
    }
}
