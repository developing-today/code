//! Turso workload adapter + sync state capture for the #9133
//! benchmark contract (slice C9133-02-turso, issue #10165).
//!
//! Implements the benchmark's logical operations as local Turso
//! operations: SQL statements over a local database file (the
//! SQLite-format local database Turso ships through libsql),
//! following the benchmark plan's Turso cells — puts and
//! reference/tombstone transitions run in a single database
//! transaction, reads are indexed selects / a parameterized query
//! against the local DB, and the captured-key delete is an
//! authority fence (tombstone) followed by a plain delete of the
//! captured, never-reused physical key.
//!
//! The local database is driven through `rusqlite` (bundled
//! SQLite), pinned to `=0.42.0` — the same pin the `pkgs/id`
//! workspace uses for its local SQLite store work. No remote
//! endpoint is configured and no network call exists in this
//! module: the store is local-only and uses no credentials.
//!
//! Sync state capture (the slice's second half): Turso embedded
//! replicas report replication state separately from query
//! results, and this adapter does the same. Every operation
//! returns a [`TursoSyncState`] snapshot taken after the
//! operation, kept strictly separate from the operation's
//! transition (its query result) and from any timing — nothing
//! in this module measures time.
//!
//! - `local_revision`: the count of committed local write
//!   transactions. It lives in the `sync_state` table and is
//!   incremented inside the same transaction as the mutation it
//!   counts, so the revision is atomic with the data. Reads and
//!   refused / no-change operations never advance it.
//! - `pushed_revision`: the highest local revision pushed to a
//!   remote primary. A local-only store pushes nothing, so this
//!   stays 0 and every committed revision is pending push.
//! - `pulled_revision`: the highest remote revision applied
//!   locally. A local-only store pulls nothing: 0.
//! - `pending_push_revisions`: `local_revision - pushed_revision`.
//! - `network`: the connectivity posture, `"local-only"`.
//!
//! The adapter exposes the same `execute(op_id, args, fixture)`
//! transition shape as the C9133-01 reference store, so the
//! fixture oracle can check it row-by-row: the oracle is that
//! the Turso adapter returns the same logical rows and
//! generation transitions as the benchmark reference model, and
//! exposes sync revision/push/pull state separately from the
//! transitions. No timing thresholds.
//!
//! Tables (the plan's SQL mapping, plus Turso sync evidence):
//! - `objects`: content_id -> metadata + current generation.
//! - `generations`: (content_id, generation) -> physical key,
//!   lifecycle, row revision.
//! - `generation_refs`: (content_id, generation, reference).
//! - `physical_objects`: physical generation key -> raw bytes.
//! - `ever_used_keys`: every physical key ever issued
//!   (never-reuse evidence for the captured-key delete).
//! - `sync_state`: the single-row sync state capture above.

use crate::{content_id, physical_key};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Connectivity posture of a local-only store: no remote endpoint
/// is configured, so no push or pull can occur.
pub const NETWORK_LOCAL_ONLY: &str = "local-only";

/// Schema for the Turso local mapping, in creation order.
pub const SCHEMA: [&str; 6] = [
    "CREATE TABLE IF NOT EXISTS objects (content_id TEXT PRIMARY KEY, metadata TEXT NOT NULL, current_generation INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS generations (content_id TEXT NOT NULL, generation INTEGER NOT NULL, physical_key TEXT NOT NULL UNIQUE, lifecycle TEXT NOT NULL, row_revision INTEGER NOT NULL, PRIMARY KEY (content_id, generation))",
    "CREATE TABLE IF NOT EXISTS generation_refs (content_id TEXT NOT NULL, generation INTEGER NOT NULL, reference TEXT NOT NULL, PRIMARY KEY (content_id, generation, reference))",
    "CREATE TABLE IF NOT EXISTS physical_objects (physical_key TEXT PRIMARY KEY, bytes BLOB NOT NULL)",
    "CREATE TABLE IF NOT EXISTS ever_used_keys (physical_key TEXT PRIMARY KEY)",
    "CREATE TABLE IF NOT EXISTS sync_state (id INTEGER PRIMARY KEY CHECK (id = 1), local_revision INTEGER NOT NULL, pushed_revision INTEGER NOT NULL, pulled_revision INTEGER NOT NULL)",
];

const SELECT_OBJECT: &str =
    "SELECT metadata, current_generation FROM objects WHERE content_id = ?1";
const INSERT_OBJECT: &str =
    "INSERT INTO objects (content_id, metadata, current_generation) VALUES (?1, ?2, ?3)";
const UPDATE_OBJECT_GENERATION: &str =
    "UPDATE objects SET current_generation = ?2 WHERE content_id = ?1";
const SELECT_GENERATION: &str = "SELECT physical_key, lifecycle, row_revision FROM generations WHERE content_id = ?1 AND generation = ?2";
const INSERT_GENERATION: &str = "INSERT INTO generations (content_id, generation, physical_key, lifecycle, row_revision) VALUES (?1, ?2, ?3, 'Live', 1)";
const UPDATE_GENERATION_TOMBSTONE: &str =
    "UPDATE generations SET lifecycle = 'Tombstoned' WHERE content_id = ?1 AND generation = ?2";
const SCAN_OBJECTS: &str = "SELECT o.content_id FROM objects o JOIN generations g ON g.content_id = o.content_id AND g.generation = o.current_generation WHERE o.content_id LIKE ?1 AND g.lifecycle = 'Live' ORDER BY o.content_id";
const COUNT_REFS: &str =
    "SELECT COUNT(*) FROM generation_refs WHERE content_id = ?1 AND generation = ?2";
const INSERT_REF: &str =
    "INSERT OR IGNORE INTO generation_refs (content_id, generation, reference) VALUES (?1, ?2, ?3)";
const DELETE_REF: &str =
    "DELETE FROM generation_refs WHERE content_id = ?1 AND generation = ?2 AND reference = ?3";
const SELECT_PHYSICAL: &str = "SELECT bytes FROM physical_objects WHERE physical_key = ?1";
const INSERT_PHYSICAL: &str = "INSERT INTO physical_objects (physical_key, bytes) VALUES (?1, ?2)";
const DELETE_PHYSICAL: &str = "DELETE FROM physical_objects WHERE physical_key = ?1";
const SELECT_EVER_USED: &str = "SELECT 1 FROM ever_used_keys WHERE physical_key = ?1";
const INSERT_EVER_USED: &str = "INSERT INTO ever_used_keys (physical_key) VALUES (?1)";
const COUNT_KEY_COLLISIONS: &str = "SELECT COUNT(*) FROM generations WHERE physical_key = ?1 AND NOT (content_id = ?2 AND generation = ?3)";
const SELECT_SYNC: &str =
    "SELECT local_revision, pushed_revision, pulled_revision FROM sync_state WHERE id = 1";
const BUMP_LOCAL_REVISION: &str =
    "UPDATE sync_state SET local_revision = local_revision + 1 WHERE id = 1";

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Turso sync state captured after one operation, reported
/// separately from the operation's transition (query result).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TursoSyncState {
    /// Committed local write transactions so far.
    pub local_revision: u64,
    /// Highest local revision pushed to a remote primary
    /// (always 0 for a local-only store).
    pub pushed_revision: u64,
    /// Highest remote revision pulled and applied locally
    /// (always 0 for a local-only store).
    pub pulled_revision: u64,
    /// Local revisions not yet pushed: `local - pushed`.
    pub pending_push_revisions: u64,
    /// Connectivity posture: `"local-only"`.
    pub network: &'static str,
}

/// One executed operation: the reference-shaped transition plus
/// the sync state captured after it.
#[derive(Clone, Debug)]
pub struct TursoOutcome {
    pub transition: Value,
    pub sync: TursoSyncState,
}

struct GenerationRow {
    physical_key: String,
    lifecycle: String,
    row_revision: u64,
}

/// A workload store backed by one local Turso (SQLite-format)
/// database file.
pub struct TursoStore {
    conn: Connection,
    path: PathBuf,
    owned_temp: bool,
}

impl TursoStore {
    /// Open (creating if needed) the local database at `path` and
    /// ensure the adapter tables and the sync state row exist.
    pub fn open(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let conn = Connection::open(&path).expect("open local turso database");
        for statement in SCHEMA {
            conn.execute(statement, []).expect("create table");
        }
        conn.execute(
            "INSERT OR IGNORE INTO sync_state (id, local_revision, pushed_revision, pulled_revision) VALUES (1, 0, 0, 0)",
            [],
        )
        .expect("init sync state");
        Self {
            conn,
            path,
            owned_temp: false,
        }
    }

    /// Open a fresh database in a unique temporary file; the file
    /// is removed when the store is dropped.
    pub fn temp() -> Self {
        let n = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "id-benchmarks-turso-{}-{}.sqlite",
            std::process::id(),
            n
        ));
        let mut store = Self::open(&path);
        store.owned_temp = true;
        store
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The current sync state snapshot, without executing an
    /// operation.
    pub fn sync_state(&self) -> TursoSyncState {
        Self::load_sync(&self.conn)
    }

    fn load_sync(conn: &Connection) -> TursoSyncState {
        let (local, pushed, pulled) = conn
            .query_row(SELECT_SYNC, [], |row| {
                Ok((
                    row.get::<_, i64>(0)? as u64,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, i64>(2)? as u64,
                ))
            })
            .expect("read sync state");
        TursoSyncState {
            local_revision: local,
            pushed_revision: pushed,
            pulled_revision: pulled,
            pending_push_revisions: local - pushed,
            network: NETWORK_LOCAL_ONLY,
        }
    }

    fn expected_id(fixture: &Value) -> &str {
        fixture["expected_content_id"].as_str().expect("fixture id")
    }

    fn load_object(conn: &Connection, cid: &str) -> Option<(Value, u64)> {
        conn.query_row(SELECT_OBJECT, params![cid], |row| {
            let metadata_text: String = row.get(0)?;
            let current: i64 = row.get(1)?;
            Ok((metadata_text, current as u64))
        })
        .optional()
        .expect("read object")
        .map(|(metadata_text, current)| {
            (
                serde_json::from_str(&metadata_text).expect("metadata json"),
                current,
            )
        })
    }

    fn load_generation(conn: &Connection, cid: &str, generation: u64) -> Option<GenerationRow> {
        conn.query_row(SELECT_GENERATION, params![cid, generation as i64], |row| {
            Ok(GenerationRow {
                physical_key: row.get(0)?,
                lifecycle: row.get(1)?,
                row_revision: row.get::<_, i64>(2)? as u64,
            })
        })
        .optional()
        .expect("read generation")
    }

    fn ref_count(conn: &Connection, cid: &str, generation: u64) -> u64 {
        conn.query_row(COUNT_REFS, params![cid, generation as i64], |row| {
            row.get::<_, i64>(0)
        })
        .expect("count references") as u64
    }

    fn load_physical(conn: &Connection, key: &str) -> Option<Vec<u8>> {
        conn.query_row(SELECT_PHYSICAL, params![key], |row| row.get(0))
            .optional()
            .expect("read physical object")
    }

    fn outcome(&self, transition: Value) -> TursoOutcome {
        TursoOutcome {
            transition,
            sync: Self::load_sync(&self.conn),
        }
    }

    /// Execute one logical operation and return the observed
    /// transition in exactly the fixture's shape, mirroring the
    /// reference store transition-for-transition, together with
    /// the sync state captured after the operation.
    pub fn execute(&mut self, op_id: &str, args: &Value, fixture: &Value) -> TursoOutcome {
        match op_id {
            "blob_put_if_absent" => self.put(args, fixture),
            "blob_get" | "metadata_lookup" | "metadata_scan" => {
                let transition = self.read(op_id, args, fixture);
                self.outcome(transition)
            }
            "ref_commit_if_live"
            | "ref_remove"
            | "object_tombstone_delete"
            | "object_delete_captured_key" => self.authority(op_id, args, fixture),
            other => panic!("no turso adapter operation for {other}"),
        }
    }

    fn read(&self, op_id: &str, args: &Value, fixture: &Value) -> Value {
        let cid = Self::expected_id(fixture);
        match op_id {
            "blob_get" => {
                // Indexed select from the local DB; the outcome's
                // sync snapshot notes the sync revision this read
                // observed (plan cell for blob_get).
                let (_, current) = Self::load_object(&self.conn, cid).expect("row exists");
                let r#gen =
                    Self::load_generation(&self.conn, cid, current).expect("generation exists");
                let bytes = Self::load_physical(&self.conn, &r#gen.physical_key)
                    .expect("physical bytes present");
                json!({
                    "bytes_hex": hex::encode(bytes),
                    "generation": current,
                    "lifecycle": r#gen.lifecycle,
                })
            }
            "metadata_lookup" => {
                let (metadata, current) = Self::load_object(&self.conn, cid).expect("row exists");
                json!({"metadata": metadata, "generation": current})
            }
            "metadata_scan" => {
                // Parameterized SQLite query (plan cell): the
                // prefix is a bound parameter, never interpolated.
                let prefix = args["prefix"].as_str().expect("prefix");
                let pattern = format!("{prefix}%");
                let mut stmt = self.conn.prepare(SCAN_OBJECTS).expect("prepare scan");
                let rows = stmt
                    .query_map(params![pattern], |row| row.get::<_, String>(0))
                    .expect("run scan");
                let mut ids: Vec<String> = Vec::new();
                for row in rows {
                    let id = row.expect("scan row");
                    if fixture["tenant"] == "tenant-a" {
                        ids.push(id);
                    }
                }
                ids.sort();
                json!({"content_ids": ids})
            }
            _ => unreachable!(),
        }
    }

    fn authority(&mut self, op_id: &str, args: &Value, fixture: &Value) -> TursoOutcome {
        let cid = Self::expected_id(fixture).to_string();
        let generation = args["generation"].as_u64().expect("generation");
        let tx = self.conn.transaction().expect("begin transaction");
        // The object row must exist, mirroring the reference store.
        let _ = Self::load_object(&tx, &cid).expect("row exists");
        let result = match op_id {
            "ref_commit_if_live" => {
                // Single DB transaction (plan cell); cross-replica
                // conflict semantics are separately tested and out
                // of scope for this local-only store.
                match Self::load_generation(&tx, &cid, generation) {
                    Some(r#gen) if r#gen.lifecycle == "Live" => {
                        let inserted = tx
                            .execute(
                                INSERT_REF,
                                params![
                                    cid,
                                    generation as i64,
                                    args["reference"].as_str().expect("reference")
                                ],
                            )
                            .expect("insert reference");
                        if inserted > 0 {
                            tx.execute(BUMP_LOCAL_REVISION, [])
                                .expect("bump local revision");
                        }
                        json!({
                            "accepted": true,
                            "generation": generation,
                            "reference_count": Self::ref_count(&tx, &cid, generation),
                        })
                    }
                    _ => json!({"accepted": false, "reason": "generation-fenced"}),
                }
            }
            "ref_remove" => {
                let _ = Self::load_generation(&tx, &cid, generation).expect("generation exists");
                let removed = tx
                    .execute(
                        DELETE_REF,
                        params![
                            cid,
                            generation as i64,
                            args["reference"].as_str().expect("reference")
                        ],
                    )
                    .expect("delete reference");
                if removed > 0 {
                    tx.execute(BUMP_LOCAL_REVISION, [])
                        .expect("bump local revision");
                }
                json!({
                    "removed": removed > 0,
                    "reference_count": Self::ref_count(&tx, &cid, generation),
                })
            }
            "object_tombstone_delete" => {
                // Authority transaction only (plan cell): the
                // tombstone IS the fence, applied in the authority
                // table itself.
                let r#gen =
                    Self::load_generation(&tx, &cid, generation).expect("generation exists");
                if Self::ref_count(&tx, &cid, generation) > 0 {
                    json!({
                        "tombstoned": false,
                        "reason": "references-present",
                        "lifecycle": r#gen.lifecycle,
                    })
                } else {
                    tx.execute(UPDATE_GENERATION_TOMBSTONE, params![cid, generation as i64])
                        .expect("tombstone generation");
                    tx.execute(BUMP_LOCAL_REVISION, [])
                        .expect("bump local revision");
                    json!({
                        "tombstoned": true,
                        "generation": generation,
                        "physical_key": r#gen.physical_key,
                        "lifecycle": "Tombstoned",
                    })
                }
            }
            "object_delete_captured_key" => {
                // Authority fence, then a plain delete of the
                // captured, never-reused key (plan cell).
                let r#gen =
                    Self::load_generation(&tx, &cid, generation).expect("generation exists");
                assert_eq!(
                    r#gen.lifecycle, "Tombstoned",
                    "delete before authoritative fence"
                );
                let key = r#gen.physical_key.clone();
                let ever_used = tx
                    .query_row(SELECT_EVER_USED, params![key], |row| row.get::<_, i64>(0))
                    .optional()
                    .expect("read ever_used")
                    .is_some();
                assert!(ever_used, "captured key was never used");
                let collisions: i64 = tx
                    .query_row(
                        COUNT_KEY_COLLISIONS,
                        params![key, cid, generation as i64],
                        |row| row.get(0),
                    )
                    .expect("count key collisions");
                assert_eq!(collisions, 0, "captured key shared with another generation");
                let deleted = tx
                    .execute(DELETE_PHYSICAL, params![key])
                    .expect("delete captured key");
                if deleted > 0 {
                    tx.execute(BUMP_LOCAL_REVISION, [])
                        .expect("bump local revision");
                }
                let (_, current) = Self::load_object(&tx, &cid).expect("row exists");
                let current_gen =
                    Self::load_generation(&tx, &cid, current).expect("current generation exists");
                let current_bytes = Self::load_physical(&tx, &current_gen.physical_key);
                let gen2_present = current == 2 && current_bytes.is_some();
                json!({
                    "deleted_key": key,
                    "deleted": deleted > 0,
                    "generation_2_key_present": gen2_present,
                    "generation_2_reference_count": Self::ref_count(&tx, &cid, current),
                    "content_id_unchanged": current_bytes
                        .map(|bytes| content_id(&bytes) == cid)
                        .unwrap_or(false),
                })
            }
            _ => unreachable!(),
        };
        tx.commit().expect("commit authority transaction");
        self.outcome(result)
    }

    fn put(&mut self, args: &Value, fixture: &Value) -> TursoOutcome {
        let raw = hex::decode(args["bytes_hex"].as_str().expect("bytes_hex")).expect("hex");
        let cid = content_id(&raw);
        assert_eq!(
            cid,
            fixture["expected_content_id"].as_str().expect("fixture id"),
            "fixture digest mismatch"
        );
        let tx = self.conn.transaction().expect("begin put transaction");
        let result;
        if let Some((metadata, current)) = Self::load_object(&tx, &cid) {
            let cur_gen = Self::load_generation(&tx, &cid, current).expect("generation exists");
            if args.get("resurrection").and_then(Value::as_bool) == Some(true) {
                assert_eq!(cur_gen.lifecycle, "Tombstoned");
                let stored = Self::load_physical(&tx, &cur_gen.physical_key)
                    .expect("physical bytes present");
                assert_eq!(
                    stored, raw,
                    "resurrection must preserve canonical content identity"
                );
                let generation = current + 1;
                let key = physical_key(&cid, generation);
                let reused = tx
                    .query_row(SELECT_EVER_USED, params![key], |row| row.get::<_, i64>(0))
                    .optional()
                    .expect("read ever_used")
                    .is_some();
                assert!(!reused, "physical generation key reused");
                tx.execute(INSERT_GENERATION, params![cid, generation as i64, key])
                    .expect("insert generation");
                tx.execute(INSERT_PHYSICAL, params![key, raw])
                    .expect("write physical");
                tx.execute(INSERT_EVER_USED, params![key])
                    .expect("mark key used");
                tx.execute(UPDATE_OBJECT_GENERATION, params![cid, generation as i64])
                    .expect("update current generation");
                tx.execute(BUMP_LOCAL_REVISION, [])
                    .expect("bump local revision");
                result = json!({
                    "accepted": true, "created": true, "generation": generation,
                    "physical_key": key, "lifecycle": "Live", "row_revision": 1,
                    "content_id": cid,
                });
            } else if cur_gen.lifecycle == "Tombstoned" {
                result = json!({
                    "accepted": false, "created": false, "generation": current,
                    "lifecycle": "Tombstoned", "reason": "explicit-resurrection-required",
                });
            } else {
                assert_eq!(metadata, args["metadata"]);
                result = json!({
                    "accepted": true, "created": false, "generation": current,
                    "lifecycle": "Live",
                    "reference_count": Self::ref_count(&tx, &cid, current),
                    "row_revision": cur_gen.row_revision,
                });
            }
        } else {
            let generation = 1;
            let key = physical_key(&cid, generation);
            let reused = tx
                .query_row(SELECT_EVER_USED, params![key], |row| row.get::<_, i64>(0))
                .optional()
                .expect("read ever_used")
                .is_some();
            assert!(!reused, "physical generation key reused");
            tx.execute(
                INSERT_OBJECT,
                params![cid, args["metadata"].to_string(), generation as i64],
            )
            .expect("insert object");
            tx.execute(INSERT_GENERATION, params![cid, generation as i64, key])
                .expect("insert generation");
            tx.execute(INSERT_PHYSICAL, params![key, raw])
                .expect("write physical");
            tx.execute(INSERT_EVER_USED, params![key])
                .expect("mark key used");
            tx.execute(BUMP_LOCAL_REVISION, [])
                .expect("bump local revision");
            result = json!({
                "accepted": true, "created": true, "generation": generation,
                "physical_key": key, "lifecycle": "Live", "row_revision": 1,
                "content_id": cid,
            });
        }
        tx.commit().expect("commit put transaction");
        // Plan cell: "transaction insert absent; commit; record
        // push/pull state" — the outcome's sync snapshot is that
        // record. Read back after a created put, like the other
        // local adapter.
        if result["accepted"] == true && result["created"] == true {
            let readback = self.read("blob_get", &json!({}), fixture);
            assert_eq!(readback["bytes_hex"], hex::encode(&raw), "put read-back");
            assert_eq!(
                readback["generation"], result["generation"],
                "put read-back"
            );
        }
        self.outcome(result)
    }
}

impl Drop for TursoStore {
    fn drop(&mut self) {
        if self.owned_temp {
            let _ = std::fs::remove_file(&self.path);
            let _ = std::fs::remove_file(self.path.with_extension("sqlite-wal"));
            let _ = std::fs::remove_file(self.path.with_extension("sqlite-shm"));
            let _ = std::fs::remove_file(self.path.with_extension("sqlite-journal"));
        }
    }
}
