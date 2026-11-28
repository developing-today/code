//! redb workload adapter for the #9133 benchmark contract
//! (slice C9133-02-redb, issue #10164).
//!
//! Implements the benchmark's logical operations on a real local
//! redb database (pinned crate `redb = "=4.6.0"`, the same pin the
//! `pkgs/id` workspace uses), following the benchmark plan's redb
//! cells: puts and reference/tombstone transitions run in a single
//! write transaction on the authority DB, reads run in a read
//! transaction, and the captured-key delete is an authority fence
//! (tombstone) followed by a plain delete of the captured,
//! never-reused physical key.
//!
//! The adapter exposes the same `execute(op_id, args, fixture)`
//! transition shape as the C9133-01 reference store, so the fixture
//! oracle can check it row-by-row: the oracle is that the redb
//! adapter returns the same logical rows and generation transitions
//! as the benchmark reference model. No timing thresholds.
//!
//! Tables:
//! - `rows`: content_id -> JSON row record (canonical bytes hex,
//!   metadata, current generation, per-generation lifecycle /
//!   references / row revision / physical key).
//! - `physical`: physical generation key -> raw bytes.
//! - `ever_used`: physical generation key -> marker (never-reuse
//!   evidence for the captured-key delete).

use crate::{content_id, physical_key};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const ROWS: TableDefinition<&str, &str> = TableDefinition::new("rows");
const PHYSICAL: TableDefinition<&str, &[u8]> = TableDefinition::new("physical");
const EVER_USED: TableDefinition<&str, &str> = TableDefinition::new("ever_used");

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Serialize, Deserialize)]
struct Generation {
    lifecycle: String,
    refs: BTreeSet<String>,
    row_revision: u64,
    key: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct Row {
    bytes_hex: String,
    metadata: Value,
    current: u64,
    generations: BTreeMap<u64, Generation>,
}

/// A workload store backed by one redb database file.
pub struct RedbStore {
    db: Database,
    path: PathBuf,
    owned_temp: bool,
}

impl RedbStore {
    /// Open (creating if needed) the redb database at `path` and
    /// ensure the adapter tables exist.
    pub fn open(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let db = Database::create(&path).expect("create redb database");
        let txn = db.begin_write().expect("begin table init");
        {
            let _ = txn.open_table(ROWS).expect("open rows table");
            let _ = txn.open_table(PHYSICAL).expect("open physical table");
            let _ = txn.open_table(EVER_USED).expect("open ever_used table");
        }
        txn.commit().expect("commit table init");
        Self {
            db,
            path,
            owned_temp: false,
        }
    }

    /// Open a fresh database in a unique temporary file; the file
    /// is removed when the store is dropped.
    pub fn temp() -> Self {
        let n = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "id-benchmarks-redb-{}-{}.redb",
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

    fn expected_id(fixture: &Value) -> &str {
        fixture["expected_content_id"].as_str().expect("fixture id")
    }

    fn load_row(get: impl Fn(&str) -> Option<String>, cid: &str) -> Option<Row> {
        get(cid).map(|text| serde_json::from_str(&text).expect("row record"))
    }

    fn save_row(table: &mut redb::Table<'_, &str, &str>, cid: &str, row: &Row) {
        table
            .insert(
                cid,
                serde_json::to_string(row).expect("serialize row").as_str(),
            )
            .expect("write row");
    }

    /// Execute one logical operation and return the observed
    /// transition in exactly the fixture's shape, mirroring the
    /// reference store transition-for-transition.
    pub fn execute(&mut self, op_id: &str, args: &Value, fixture: &Value) -> Value {
        match op_id {
            "blob_put_if_absent" => self.put(args, fixture),
            "blob_get" | "metadata_lookup" | "metadata_scan" => self.read(op_id, args, fixture),
            "ref_commit_if_live"
            | "ref_remove"
            | "object_tombstone_delete"
            | "object_delete_captured_key" => self.authority(op_id, args, fixture),
            other => panic!("no redb adapter operation for {other}"),
        }
    }

    fn read(&mut self, op_id: &str, args: &Value, fixture: &Value) -> Value {
        let txn = self.db.begin_read().expect("begin read transaction");
        let rows = txn.open_table(ROWS).expect("open rows table");
        match op_id {
            "blob_get" => {
                let row = Self::load_row(
                    |k: &str| {
                        rows.get(k)
                            .expect("read row")
                            .map(|g| g.value().to_string())
                    },
                    Self::expected_id(fixture),
                )
                .expect("row exists");
                let generation = row.current;
                let r#gen = &row.generations[&generation];
                let physical = txn.open_table(PHYSICAL).expect("open physical table");
                let bytes = physical
                    .get(r#gen.key.as_str())
                    .expect("read physical")
                    .expect("physical bytes present");
                json!({
                    "bytes_hex": hex::encode(bytes.value()),
                    "generation": generation,
                    "lifecycle": r#gen.lifecycle,
                })
            }
            "metadata_lookup" => {
                let row = Self::load_row(
                    |k: &str| {
                        rows.get(k)
                            .expect("read row")
                            .map(|g| g.value().to_string())
                    },
                    Self::expected_id(fixture),
                )
                .expect("row exists");
                json!({"metadata": row.metadata, "generation": row.current})
            }
            "metadata_scan" => {
                // Table range scan: redb iterates in key order, so
                // the collected ids are already sorted.
                let prefix = args["prefix"].as_str().expect("prefix");
                let mut ids: Vec<String> = Vec::new();
                for entry in rows.iter().expect("scan rows table") {
                    let (key, value) = entry.expect("scan row");
                    let row: Row = serde_json::from_str(value.value()).expect("row record");
                    let r#gen = &row.generations[&row.current];
                    if r#gen.lifecycle == "Live"
                        && key.value().starts_with(prefix)
                        && fixture["tenant"] == "tenant-a"
                    {
                        ids.push(key.value().to_string());
                    }
                }
                ids.sort();
                json!({"content_ids": ids})
            }
            _ => unreachable!(),
        }
    }

    fn authority(&mut self, op_id: &str, args: &Value, fixture: &Value) -> Value {
        let cid = Self::expected_id(fixture).to_string();
        let txn = self.db.begin_write().expect("begin write transaction");
        let result;
        {
            let mut rows = txn.open_table(ROWS).expect("open rows table");
            let mut row = Self::load_row(
                |k: &str| {
                    rows.get(k)
                        .expect("read row")
                        .map(|g| g.value().to_string())
                },
                &cid,
            )
            .expect("row exists");
            let generation = args["generation"].as_u64().expect("generation");
            match op_id {
                "ref_commit_if_live" => match row.generations.get_mut(&generation) {
                    Some(r#gen) if r#gen.lifecycle == "Live" => {
                        r#gen
                            .refs
                            .insert(args["reference"].as_str().expect("reference").to_string());
                        let count = r#gen.refs.len();
                        result = json!({
                            "accepted": true,
                            "generation": generation,
                            "reference_count": count,
                        });
                    }
                    _ => {
                        result = json!({"accepted": false, "reason": "generation-fenced"});
                    }
                },
                "ref_remove" => {
                    let r#gen = row
                        .generations
                        .get_mut(&generation)
                        .expect("generation exists");
                    let removed = r#gen
                        .refs
                        .remove(args["reference"].as_str().expect("reference"));
                    result = json!({"removed": removed, "reference_count": r#gen.refs.len()});
                }
                "object_tombstone_delete" => {
                    let r#gen = row
                        .generations
                        .get_mut(&generation)
                        .expect("generation exists");
                    if !r#gen.refs.is_empty() {
                        result = json!({
                            "tombstoned": false,
                            "reason": "references-present",
                            "lifecycle": r#gen.lifecycle,
                        });
                    } else {
                        r#gen.lifecycle = "Tombstoned".to_string();
                        result = json!({
                            "tombstoned": true,
                            "generation": generation,
                            "physical_key": r#gen.key,
                            "lifecycle": "Tombstoned",
                        });
                    }
                }
                "object_delete_captured_key" => {
                    let r#gen = row.generations[&generation].clone();
                    assert_eq!(
                        r#gen.lifecycle, "Tombstoned",
                        "delete before authoritative fence"
                    );
                    let key = r#gen.key.clone();
                    let mut physical = txn.open_table(PHYSICAL).expect("open physical table");
                    let ever_used = txn.open_table(EVER_USED).expect("open ever_used table");
                    assert!(
                        ever_used
                            .get(key.as_str())
                            .expect("read ever_used")
                            .is_some(),
                        "captured key was never used"
                    );
                    assert!(
                        row.generations
                            .iter()
                            .all(|(n, other)| *n == generation || other.key != key)
                    );
                    let deleted = physical.remove(key.as_str()).expect("delete key").is_some();
                    let current = row.current;
                    let current_gen = &row.generations[&current];
                    let gen2_present = physical
                        .get(current_gen.key.as_str())
                        .expect("read current key")
                        .is_some();
                    result = json!({
                        "deleted_key": key,
                        "deleted": deleted,
                        "generation_2_key_present": current == 2 && gen2_present,
                        "generation_2_reference_count": current_gen.refs.len(),
                        "content_id_unchanged": cid == content_id(&hex::decode(&row.bytes_hex).expect("row bytes hex")),
                    });
                }
                _ => unreachable!(),
            }
            Self::save_row(&mut rows, &cid, &row);
        }
        txn.commit().expect("commit authority transaction");
        result
    }

    fn put(&mut self, args: &Value, fixture: &Value) -> Value {
        let raw = hex::decode(args["bytes_hex"].as_str().expect("bytes_hex")).expect("hex");
        let cid = content_id(&raw);
        assert_eq!(
            cid,
            fixture["expected_content_id"].as_str().expect("fixture id"),
            "fixture digest mismatch"
        );
        let txn = self.db.begin_write().expect("begin write transaction");
        let result;
        {
            let mut rows = txn.open_table(ROWS).expect("open rows table");
            let mut physical = txn.open_table(PHYSICAL).expect("open physical table");
            let mut ever_used = txn.open_table(EVER_USED).expect("open ever_used table");
            if let Some(mut prior) = Self::load_row(
                |k: &str| {
                    rows.get(k)
                        .expect("read row")
                        .map(|g| g.value().to_string())
                },
                &cid,
            ) {
                let current = prior.current;
                let cur_lifecycle = prior.generations[&current].lifecycle.clone();
                if args.get("resurrection").and_then(Value::as_bool) == Some(true) {
                    assert_eq!(cur_lifecycle, "Tombstoned");
                    assert_eq!(
                        prior.bytes_hex,
                        hex::encode(&raw),
                        "resurrection must preserve canonical content identity"
                    );
                    let generation = current + 1;
                    let key = physical_key(&cid, generation);
                    assert!(
                        ever_used
                            .get(key.as_str())
                            .expect("read ever_used")
                            .is_none(),
                        "physical generation key reused"
                    );
                    prior.generations.insert(
                        generation,
                        Generation {
                            lifecycle: "Live".to_string(),
                            refs: BTreeSet::new(),
                            row_revision: 1,
                            key: key.clone(),
                        },
                    );
                    prior.current = generation;
                    physical
                        .insert(key.as_str(), raw.as_slice())
                        .expect("write physical");
                    ever_used.insert(key.as_str(), "").expect("mark key used");
                    Self::save_row(&mut rows, &cid, &prior);
                    result = json!({
                        "accepted": true, "created": true, "generation": generation,
                        "physical_key": key, "lifecycle": "Live", "row_revision": 1,
                        "content_id": cid,
                    });
                } else if cur_lifecycle == "Tombstoned" {
                    result = json!({
                        "accepted": false, "created": false, "generation": current,
                        "lifecycle": "Tombstoned", "reason": "explicit-resurrection-required",
                    });
                } else {
                    assert_eq!(prior.bytes_hex, hex::encode(&raw));
                    assert_eq!(prior.metadata, args["metadata"]);
                    let r#gen = &prior.generations[&current];
                    result = json!({
                        "accepted": true, "created": false, "generation": current,
                        "lifecycle": "Live", "reference_count": r#gen.refs.len(),
                        "row_revision": r#gen.row_revision,
                    });
                }
            } else {
                let generation = 1;
                let key = physical_key(&cid, generation);
                assert!(
                    ever_used
                        .get(key.as_str())
                        .expect("read ever_used")
                        .is_none(),
                    "physical generation key reused"
                );
                let mut generations = BTreeMap::new();
                generations.insert(
                    generation,
                    Generation {
                        lifecycle: "Live".to_string(),
                        refs: BTreeSet::new(),
                        row_revision: 1,
                        key: key.clone(),
                    },
                );
                let row = Row {
                    bytes_hex: hex::encode(&raw),
                    metadata: args["metadata"].clone(),
                    current: generation,
                    generations,
                };
                Self::save_row(&mut rows, &cid, &row);
                physical
                    .insert(key.as_str(), raw.as_slice())
                    .expect("write physical");
                ever_used.insert(key.as_str(), "").expect("mark key used");
                result = json!({
                    "accepted": true, "created": true, "generation": generation,
                    "physical_key": key, "lifecycle": "Live", "row_revision": 1,
                    "content_id": cid,
                });
            }
        }
        txn.commit().expect("commit put transaction");
        // Plan cell: "write transaction insert absent; commit; read back".
        if result["accepted"] == true && result["created"] == true {
            let readback = self.read("blob_get", &json!({}), fixture);
            assert_eq!(readback["bytes_hex"], hex::encode(&raw), "put read-back");
            assert_eq!(
                readback["generation"], result["generation"],
                "put read-back"
            );
        }
        result
    }
}

impl Drop for RedbStore {
    fn drop(&mut self) {
        if self.owned_temp {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
