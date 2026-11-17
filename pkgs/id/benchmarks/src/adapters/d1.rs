//! D1 workload adapter + native counters for the #9133 benchmark
//! contract (slice C9133-02-D1, issue #10162).
//!
//! Maps each supported logical operation of the synthetic workload
//! onto D1 (SQLite) statements over four tables — `objects`,
//! `generations`, `generation_refs`, `physical_objects` — and captures
//! D1's native result counters (`rows_read`, `rows_written`,
//! `duration`, `size_after`, `changes`, `last_row_id`) for every
//! operation, the evidence a D1 parity run reports.
//!
//! Finite, timer-free, provider-free, exactly like the reference
//! store in the crate root: [`FakeD1`] stands in for the D1 SQL API
//! and applies each prepared statement's semantics to in-memory
//! tables. There is no network call anywhere in this module, and the
//! fake reports a synthetic `duration` of 0.0 — durations are never
//! measured here and no test may assert on latency.
//!
//! Counter model (the fake's documented counting rules, mirroring
//! how SQLite/D1 account row visits):
//! - a primary-key probe reads 1 row, hit or miss;
//! - a reference count reads one index entry per matching reference;
//! - a scan reads every row it examines;
//! - an INSERT/UPDATE/DELETE writes one row per row changed;
//! - `size_after` is the fake database's total stored bytes after
//!   the operation (ids, metadata JSON, keys, blob bytes).
//!
//! D1 support levels come from `support_level` in the crate root:
//! `object_tombstone_delete` is `authority-fence-only` for D1 (the
//! tombstone IS the fence, applied in the authority table itself) and
//! `object_delete_captured_key` is `delete-captured-key-after-fence`
//! (a plain delete of the captured generation row, only after the
//! tombstone fence). This adapter executes both under those terms.
//!
//! Rollback: remove this module and its `pub mod` wiring in the
//! crate root; the benchmark harness and other adapters remain
//! usable.

use crate::{content_id, physical_key};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// D1 result `meta` counters for one executed operation.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct D1Counters {
    pub rows_read: u64,
    pub rows_written: u64,
    /// Synthetic only: the fake never measures time, so this is
    /// always 0.0. Real D1 reports wall-clock milliseconds here.
    pub duration_ms: f64,
    /// Total stored bytes in the fake database after the operation.
    pub size_after: u64,
    pub changes: u64,
    pub last_row_id: u64,
}

/// One executed operation: the reference-shaped transition plus the
/// native counters and the SQL statements that produced it.
#[derive(Clone, Debug)]
pub struct D1Outcome {
    pub transition: Value,
    pub counters: D1Counters,
    pub statements: Vec<&'static str>,
}

/// Schema for the D1 mapping, in creation order.
pub const SCHEMA: [&str; 4] = [
    "CREATE TABLE objects (content_id TEXT PRIMARY KEY, metadata TEXT NOT NULL, current_generation INTEGER NOT NULL)",
    "CREATE TABLE generations (content_id TEXT NOT NULL, generation INTEGER NOT NULL, physical_key TEXT NOT NULL UNIQUE, lifecycle TEXT NOT NULL, row_revision INTEGER NOT NULL, PRIMARY KEY (content_id, generation))",
    "CREATE TABLE generation_refs (content_id TEXT NOT NULL, generation INTEGER NOT NULL, reference TEXT NOT NULL, PRIMARY KEY (content_id, generation, reference))",
    "CREATE TABLE physical_objects (physical_key TEXT PRIMARY KEY, bytes BLOB NOT NULL)",
];

// The D1 mapping: one SQL template per statement role.
const SELECT_OBJECT: &str =
    "SELECT content_id, metadata, current_generation FROM objects WHERE content_id = ?1";
const INSERT_OBJECT: &str = "INSERT INTO objects (content_id, metadata, current_generation) VALUES (?1, ?2, ?3) ON CONFLICT(content_id) DO NOTHING";
const UPDATE_OBJECT_GENERATION: &str =
    "UPDATE objects SET current_generation = ?2 WHERE content_id = ?1";
const SELECT_GENERATION: &str = "SELECT physical_key, lifecycle, row_revision FROM generations WHERE content_id = ?1 AND generation = ?2";
const INSERT_GENERATION: &str = "INSERT INTO generations (content_id, generation, physical_key, lifecycle, row_revision) VALUES (?1, ?2, ?3, 'Live', 1)";
const UPDATE_GENERATION_TOMBSTONE: &str = "UPDATE generations SET lifecycle = 'Tombstoned' WHERE content_id = ?1 AND generation = ?2 AND lifecycle = 'Live'";
const SCAN_OBJECTS: &str =
    "SELECT content_id FROM objects WHERE content_id LIKE ?1 ORDER BY content_id";
const COUNT_REFS: &str =
    "SELECT COUNT(*) FROM generation_refs WHERE content_id = ?1 AND generation = ?2";
const INSERT_REF_IF_LIVE: &str = "INSERT INTO generation_refs (content_id, generation, reference) SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM generations WHERE content_id = ?1 AND generation = ?2 AND lifecycle = 'Live') ON CONFLICT DO NOTHING";
const DELETE_REF: &str =
    "DELETE FROM generation_refs WHERE content_id = ?1 AND generation = ?2 AND reference = ?3";
const SELECT_PHYSICAL: &str = "SELECT bytes FROM physical_objects WHERE physical_key = ?1";
const INSERT_PHYSICAL: &str = "INSERT INTO physical_objects (physical_key, bytes) VALUES (?1, ?2)";
const DELETE_PHYSICAL: &str = "DELETE FROM physical_objects WHERE physical_key = ?1";

#[derive(Clone)]
struct ObjectRow {
    metadata: Value,
    current_generation: u64,
}

#[derive(Clone)]
struct GenerationRow {
    physical_key: String,
    lifecycle: String,
    row_revision: u64,
}

/// In-memory fake of the D1 SQL API surface the adapter uses. It
/// applies statement semantics to its tables and accounts counters
/// under the rules in the module docs. No network, no timers.
#[derive(Default)]
pub struct FakeD1 {
    objects: BTreeMap<String, ObjectRow>,
    generations: BTreeMap<(String, u64), GenerationRow>,
    refs: BTreeSet<(String, u64, String)>,
    physical: BTreeMap<String, Vec<u8>>,
    ever_used_keys: HashSet<String>,
    last_row_id: u64,
}

impl FakeD1 {
    pub fn new() -> Self {
        Self::default()
    }

    fn size_after(&self) -> u64 {
        let mut total: u64 = 0;
        for (cid, row) in &self.objects {
            total += cid.len() as u64 + row.metadata.to_string().len() as u64 + 8;
        }
        for ((cid, _), gen) in &self.generations {
            total +=
                cid.len() as u64 + gen.physical_key.len() as u64 + gen.lifecycle.len() as u64 + 16;
        }
        for (cid, _, reference) in &self.refs {
            total += cid.len() as u64 + reference.len() as u64 + 8;
        }
        for (key, bytes) in &self.physical {
            total += key.len() as u64 + bytes.len() as u64;
        }
        total
    }

    fn finish(&self, mut counters: D1Counters) -> D1Counters {
        counters.size_after = self.size_after();
        counters.last_row_id = self.last_row_id;
        counters
    }

    fn ref_count(&self, cid: &str, generation: u64) -> u64 {
        self.refs
            .iter()
            .filter(|(c, g, _)| c == cid && *g == generation)
            .count() as u64
    }
}

/// The D1 adapter: executes logical operations against [`FakeD1`]
/// through the SQL mapping above and returns reference-shaped
/// transitions (identical shapes to `ReferenceStore::execute`) with
/// the native counters attached.
#[derive(Default)]
pub struct D1Adapter {
    db: FakeD1,
    /// Object bytes by content id, kept adapter-side because the
    /// reference transition for a delete must prove the surviving
    /// row's content identity is unchanged — in D1 terms this is
    /// the `objects` row's immutable payload digest, re-derived
    /// from the stored bytes.
    payloads: HashMap<String, Vec<u8>>,
}

impl D1Adapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn execute(&mut self, op_id: &str, args: &Value, fixture: &Value) -> D1Outcome {
        match op_id {
            "blob_put_if_absent" => self.put(args, fixture),
            "blob_get" => {
                let cid = Self::expected_id(fixture);
                let mut c = D1Counters::default();
                let row = self.db.objects.get(cid).expect("row exists");
                c.rows_read += 1; // SELECT_OBJECT probe
                let generation = row.current_generation;
                let gen = &self.db.generations[&(cid.to_string(), generation)];
                c.rows_read += 1; // SELECT_GENERATION probe
                let bytes = &self.db.physical[&gen.physical_key];
                c.rows_read += 1; // SELECT_PHYSICAL probe
                D1Outcome {
                    transition: json!({
                        "bytes_hex": hex::encode(bytes),
                        "generation": generation,
                        "lifecycle": gen.lifecycle,
                    }),
                    counters: self.db.finish(c),
                    statements: vec![SELECT_OBJECT, SELECT_GENERATION, SELECT_PHYSICAL],
                }
            }
            "metadata_lookup" => {
                let cid = Self::expected_id(fixture);
                let mut c = D1Counters::default();
                let row = self.db.objects.get(cid).expect("row exists");
                c.rows_read += 1; // SELECT_OBJECT probe
                D1Outcome {
                    transition: json!({"metadata": row.metadata, "generation": row.current_generation}),
                    counters: self.db.finish(c),
                    statements: vec![SELECT_OBJECT],
                }
            }
            "metadata_scan" => {
                let prefix = args["prefix"].as_str().expect("prefix");
                let mut c = D1Counters::default();
                // SCAN_OBJECTS examines every objects row; the live
                // check probes each candidate's current generation.
                let mut ids: Vec<String> = Vec::new();
                for (cid, row) in &self.db.objects {
                    c.rows_read += 1;
                    let gen = &self.db.generations[&(cid.clone(), row.current_generation)];
                    c.rows_read += 1;
                    if gen.lifecycle == "Live"
                        && cid.starts_with(prefix)
                        && fixture["tenant"] == "tenant-a"
                    {
                        ids.push(cid.clone());
                    }
                }
                ids.sort();
                D1Outcome {
                    transition: json!({"content_ids": ids}),
                    counters: self.db.finish(c),
                    statements: vec![SCAN_OBJECTS, SELECT_GENERATION],
                }
            }
            "ref_commit_if_live" => {
                let cid = Self::expected_id(fixture).to_string();
                let generation = args["generation"].as_u64().expect("generation");
                let reference = args["reference"].as_str().expect("reference").to_string();
                let mut c = D1Counters::default();
                let gen = self
                    .db
                    .generations
                    .get(&(cid.clone(), generation))
                    .expect("generation exists");
                c.rows_read += 1; // EXISTS probe inside INSERT_REF_IF_LIVE
                if gen.lifecycle == "Live" {
                    let inserted = self.db.refs.insert((cid.clone(), generation, reference));
                    if inserted {
                        c.rows_written += 1;
                        c.changes = 1;
                        self.db.last_row_id += 1;
                    }
                    D1Outcome {
                        transition: json!({
                            "accepted": true,
                            "generation": generation,
                            "reference_count": self.db.ref_count(&cid, generation),
                        }),
                        counters: self.db.finish(c),
                        statements: vec![INSERT_REF_IF_LIVE],
                    }
                } else {
                    D1Outcome {
                        transition: json!({"accepted": false, "reason": "generation-fenced"}),
                        counters: self.db.finish(c),
                        statements: vec![INSERT_REF_IF_LIVE],
                    }
                }
            }
            "ref_remove" => {
                let cid = Self::expected_id(fixture).to_string();
                let generation = args["generation"].as_u64().expect("generation");
                let reference = args["reference"].as_str().expect("reference");
                let mut c = D1Counters::default();
                let removed =
                    self.db
                        .refs
                        .remove(&(cid.clone(), generation, reference.to_string()));
                if removed {
                    c.rows_written += 1;
                    c.changes = 1;
                }
                let remaining = self.db.ref_count(&cid, generation);
                c.rows_read += remaining; // COUNT_REFS index entries examined
                D1Outcome {
                    transition: json!({"removed": removed, "reference_count": remaining}),
                    counters: self.db.finish(c),
                    statements: vec![DELETE_REF, COUNT_REFS],
                }
            }
            "object_tombstone_delete" => {
                let cid = Self::expected_id(fixture).to_string();
                let generation = args["generation"].as_u64().expect("generation");
                let mut c = D1Counters::default();
                let refs = self.db.ref_count(&cid, generation);
                c.rows_read += 1; // SELECT_GENERATION probe
                c.rows_read += refs; // COUNT_REFS index entries examined
                let lifecycle = self.db.generations[&(cid.clone(), generation)]
                    .lifecycle
                    .clone();
                if refs > 0 {
                    D1Outcome {
                        transition: json!({
                            "tombstoned": false,
                            "reason": "references-present",
                            "lifecycle": lifecycle,
                        }),
                        counters: self.db.finish(c),
                        statements: vec![SELECT_GENERATION, COUNT_REFS],
                    }
                } else {
                    let key = {
                        let gen = self
                            .db
                            .generations
                            .get_mut(&(cid.clone(), generation))
                            .expect("generation exists");
                        gen.lifecycle = "Tombstoned".to_string();
                        gen.physical_key.clone()
                    };
                    c.rows_written += 1;
                    c.changes = 1;
                    D1Outcome {
                        transition: json!({
                            "tombstoned": true,
                            "generation": generation,
                            "physical_key": key,
                            "lifecycle": "Tombstoned",
                        }),
                        counters: self.db.finish(c),
                        statements: vec![
                            SELECT_GENERATION,
                            COUNT_REFS,
                            UPDATE_GENERATION_TOMBSTONE,
                        ],
                    }
                }
            }
            "object_delete_captured_key" => {
                let cid = Self::expected_id(fixture).to_string();
                let generation = args["generation"].as_u64().expect("generation");
                let mut c = D1Counters::default();
                let gen = &self.db.generations[&(cid.clone(), generation)];
                c.rows_read += 1; // SELECT_GENERATION probe
                assert_eq!(
                    gen.lifecycle, "Tombstoned",
                    "delete before authoritative fence"
                );
                let key = gen.physical_key.clone();
                assert!(self.db.ever_used_keys.contains(&key));
                assert!(self
                    .db
                    .generations
                    .iter()
                    .all(|((_, n), other)| *n == generation || other.physical_key != key));
                let deleted = self.db.physical.remove(&key).is_some();
                if deleted {
                    c.rows_written += 1;
                    c.changes = 1;
                }
                let current = self.db.objects[&cid].current_generation;
                let current_gen = &self.db.generations[&(cid.clone(), current)];
                let gen2_present =
                    current == 2 && self.db.physical.contains_key(&current_gen.physical_key);
                c.rows_read += 1; // SELECT_PHYSICAL probe for the surviving generation
                let gen2_refs = self.db.ref_count(&cid, current);
                c.rows_read += gen2_refs; // COUNT_REFS index entries examined
                D1Outcome {
                    transition: json!({
                        "deleted_key": key,
                        "deleted": deleted,
                        "generation_2_key_present": gen2_present,
                        "generation_2_reference_count": gen2_refs,
                        "content_id_unchanged": cid == content_id(&self.payloads[&cid]),
                    }),
                    counters: self.db.finish(c),
                    statements: vec![
                        SELECT_GENERATION,
                        DELETE_PHYSICAL,
                        SELECT_PHYSICAL,
                        COUNT_REFS,
                    ],
                }
            }
            other => panic!("no D1 mapping for {other}"),
        }
    }

    fn expected_id(fixture: &Value) -> &str {
        fixture["expected_content_id"].as_str().expect("fixture id")
    }

    fn put(&mut self, args: &Value, fixture: &Value) -> D1Outcome {
        let raw = hex::decode(args["bytes_hex"].as_str().expect("bytes_hex")).expect("hex");
        let cid = content_id(&raw);
        assert_eq!(cid, Self::expected_id(fixture), "fixture digest mismatch");
        let mut c = D1Counters::default();
        c.rows_read += 1; // SELECT_OBJECT probe (the ON CONFLICT check)

        if let Some(existing) = self.db.objects.get(&cid).cloned() {
            let current = existing.current_generation;
            let gen = self.db.generations[&(cid.clone(), current)].clone();
            c.rows_read += 1; // SELECT_GENERATION probe
            if gen.lifecycle == "Tombstoned"
                && args.get("resurrection").and_then(Value::as_bool) != Some(true)
            {
                return D1Outcome {
                    transition: json!({
                        "accepted": false, "created": false, "generation": current,
                        "lifecycle": "Tombstoned", "reason": "explicit-resurrection-required",
                    }),
                    counters: self.db.finish(c),
                    statements: vec![SELECT_OBJECT, SELECT_GENERATION],
                };
            }
            if gen.lifecycle == "Tombstoned" {
                // Explicit resurrection: a new generation row under a
                // never-reused physical key; content identity is
                // preserved by construction (asserted like the
                // reference store).
                assert_eq!(
                    self.payloads[&cid], raw,
                    "resurrection must preserve canonical content identity"
                );
                let generation = current + 1;
                let key = physical_key(&cid, generation);
                assert!(
                    !self.db.ever_used_keys.contains(&key),
                    "physical generation key reused"
                );
                self.db.generations.insert(
                    (cid.clone(), generation),
                    GenerationRow {
                        physical_key: key.clone(),
                        lifecycle: "Live".to_string(),
                        row_revision: 1,
                    },
                );
                c.rows_written += 1; // INSERT_GENERATION
                self.db.physical.insert(key.clone(), raw);
                c.rows_written += 1; // INSERT_PHYSICAL
                self.db.ever_used_keys.insert(key.clone());
                self.db
                    .objects
                    .get_mut(&cid)
                    .expect("row exists")
                    .current_generation = generation;
                c.rows_written += 1; // UPDATE_OBJECT_GENERATION
                c.changes = 1;
                self.db.last_row_id += 1;
                return D1Outcome {
                    transition: json!({
                        "accepted": true, "created": true, "generation": generation,
                        "physical_key": key, "lifecycle": "Live", "row_revision": 1,
                        "content_id": cid,
                    }),
                    counters: self.db.finish(c),
                    statements: vec![
                        SELECT_OBJECT,
                        SELECT_GENERATION,
                        INSERT_GENERATION,
                        INSERT_PHYSICAL,
                        UPDATE_OBJECT_GENERATION,
                    ],
                };
            }
            // Live row already present: idempotent re-put.
            let refs = self.db.ref_count(&cid, current);
            c.rows_read += refs; // COUNT_REFS index entries examined
            return D1Outcome {
                transition: json!({
                    "accepted": true, "created": false, "generation": current,
                    "lifecycle": "Live", "reference_count": refs,
                    "row_revision": gen.row_revision,
                }),
                counters: self.db.finish(c),
                statements: vec![SELECT_OBJECT, SELECT_GENERATION, COUNT_REFS],
            };
        }

        // First put: object + generation + physical rows.
        let key = physical_key(&cid, 1);
        assert!(
            !self.db.ever_used_keys.contains(&key),
            "physical generation key reused"
        );
        self.db.objects.insert(
            cid.clone(),
            ObjectRow {
                metadata: args["metadata"].clone(),
                current_generation: 1,
            },
        );
        c.rows_written += 1; // INSERT_OBJECT
        self.db.generations.insert(
            (cid.clone(), 1),
            GenerationRow {
                physical_key: key.clone(),
                lifecycle: "Live".to_string(),
                row_revision: 1,
            },
        );
        c.rows_written += 1; // INSERT_GENERATION
        self.db.physical.insert(key.clone(), raw.clone());
        c.rows_written += 1; // INSERT_PHYSICAL
        self.db.ever_used_keys.insert(key.clone());
        self.payloads.insert(cid.clone(), raw);
        c.changes = 1;
        self.db.last_row_id += 1;
        D1Outcome {
            transition: json!({
                "accepted": true, "created": true, "generation": 1,
                "physical_key": key, "lifecycle": "Live", "row_revision": 1,
                "content_id": cid,
            }),
            counters: self.db.finish(c),
            statements: vec![
                SELECT_OBJECT,
                INSERT_OBJECT,
                INSERT_GENERATION,
                INSERT_PHYSICAL,
            ],
        }
    }
}
