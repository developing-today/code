//! Reference store + generation-key model for the #9133 benchmark
//! contract (slice C9133-01, issue #10153).
//!
//! This is a direct Rust port of the design's Python reference oracle
//! (`doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/benchmark-workload-oracle.py`):
//! a finite, timer-free, provider-free model whose transitions the
//! synthetic workload fixture pins row-by-row. It is a specification
//! model for benchmark adapters, not a production store.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};

pub mod adapters {
    //! Engine workload adapters for the #9133 benchmark contract
    //! (one module per engine slice in C9133-02).
    pub mod d1;
}

pub const ENGINES: [&str; 4] = ["d1", "r2", "redb", "turso"];

/// Canonical fixture content identity: `sha256:` + hex of the exact bytes.
pub fn content_id(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

/// Test-only key profile; the property tested is uniqueness/non-reuse,
/// not a production object-key encoding recommendation.
pub fn physical_key(content_id: &str, generation: u64) -> String {
    let short = &content_id.split(':').nth(1).expect("content id shape")[..8];
    format!("fixture/sha256-{short}/gen-{generation}")
}

/// Engine capability mapping for one plan cell, mirroring the Python
/// oracle's `support_level` exactly (same text probes, same order).
pub fn support_level(op_id: &str, engine: &str, cell: &str) -> String {
    let text = cell.to_lowercase().replace('-', " ");
    if op_id == "metadata_scan" && engine == "r2" {
        return if text.contains("otherwise unsupported") {
            "unsupported"
        } else {
            "supported"
        }
        .to_string();
    }
    if (op_id == "ref_commit_if_live" || op_id == "ref_remove") && engine == "r2" {
        return if text.contains("external transactional authority") {
            "external-authority-required"
        } else {
            "supported"
        }
        .to_string();
    }
    if op_id == "object_tombstone_delete" {
        if engine == "r2" {
            return if text.contains("external authoritative fence") {
                "external-authority-fence-required"
            } else {
                "unsupported"
            }
            .to_string();
        }
        return if text.contains("authority transaction only") {
            "authority-fence-only"
        } else {
            "unsupported"
        }
        .to_string();
    }
    if op_id == "object_delete_captured_key" {
        if engine == "r2" {
            return if text.contains("plain deleteobject on captured never reused key") {
                "plain-delete-captured-never-reused-key-after-fence"
            } else {
                "unsupported"
            }
            .to_string();
        }
        if engine == "d1" {
            return if text.contains("delete captured generation row") {
                "delete-captured-key-after-fence"
            } else {
                "unsupported"
            }
            .to_string();
        }
        return if text.contains("authority fence then external plain delete")
            && text.contains("never reused")
        {
            "authority-then-plain-delete-captured-key"
        } else {
            "unsupported"
        }
        .to_string();
    }
    if text.contains("unsupported") {
        "unsupported".to_string()
    } else {
        "supported".to_string()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lifecycle {
    Live,
    Tombstoned,
}

impl Lifecycle {
    fn as_str(&self) -> &'static str {
        match self {
            Lifecycle::Live => "Live",
            Lifecycle::Tombstoned => "Tombstoned",
        }
    }
}

#[derive(Clone)]
struct Generation {
    lifecycle: Lifecycle,
    refs: HashSet<String>,
    row_revision: u64,
    key: String,
}

struct Row {
    bytes: Vec<u8>,
    metadata: Value,
    current: u64,
    generations: BTreeMap<u64, Generation>,
}

/// The finite reference store. All operations return the observed
/// transition as a JSON object in exactly the fixture's shape.
#[derive(Default)]
pub struct ReferenceStore {
    rows: HashMap<String, Row>,
    physical: HashMap<String, Vec<u8>>,
    ever_used_keys: HashSet<String>,
}

impl ReferenceStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn expected_id(fixture: &Value) -> &str {
        fixture["expected_content_id"].as_str().expect("fixture id")
    }

    pub fn execute(&mut self, op_id: &str, args: &Value, fixture: &Value) -> Value {
        match op_id {
            "blob_put_if_absent" => self.put(args, fixture),
            "blob_get" => {
                let row = &self.rows[Self::expected_id(fixture)];
                let generation = row.current;
                let gen = &row.generations[&generation];
                json!({
                    "bytes_hex": hex::encode(&self.physical[&gen.key]),
                    "generation": generation,
                    "lifecycle": gen.lifecycle.as_str(),
                })
            }
            "metadata_lookup" => {
                let row = &self.rows[Self::expected_id(fixture)];
                json!({"metadata": row.metadata, "generation": row.current})
            }
            "metadata_scan" => {
                let prefix = args["prefix"].as_str().expect("prefix");
                let mut ids: Vec<&String> = self
                    .rows
                    .iter()
                    .filter(|(cid, row)| {
                        row.generations[&row.current].lifecycle == Lifecycle::Live
                            && cid.starts_with(prefix)
                            && fixture["tenant"] == "tenant-a"
                    })
                    .map(|(cid, _)| cid)
                    .collect();
                ids.sort();
                json!({"content_ids": ids})
            }
            "ref_commit_if_live" => {
                let row = self
                    .rows
                    .get_mut(Self::expected_id(fixture))
                    .expect("row exists");
                let generation = args["generation"].as_u64().expect("generation");
                match row.generations.get_mut(&generation) {
                    Some(gen) if gen.lifecycle == Lifecycle::Live => {
                        gen.refs
                            .insert(args["reference"].as_str().expect("reference").to_string());
                        json!({
                            "accepted": true,
                            "generation": generation,
                            "reference_count": gen.refs.len(),
                        })
                    }
                    _ => json!({"accepted": false, "reason": "generation-fenced"}),
                }
            }
            "ref_remove" => {
                let row = self
                    .rows
                    .get_mut(Self::expected_id(fixture))
                    .expect("row exists");
                let generation = args["generation"].as_u64().expect("generation");
                let gen = row
                    .generations
                    .get_mut(&generation)
                    .expect("generation exists");
                let reference = args["reference"].as_str().expect("reference");
                let removed = gen.refs.remove(reference);
                json!({"removed": removed, "reference_count": gen.refs.len()})
            }
            "object_tombstone_delete" => {
                let row = self
                    .rows
                    .get_mut(Self::expected_id(fixture))
                    .expect("row exists");
                let generation = args["generation"].as_u64().expect("generation");
                let gen = row
                    .generations
                    .get_mut(&generation)
                    .expect("generation exists");
                if !gen.refs.is_empty() {
                    json!({
                        "tombstoned": false,
                        "reason": "references-present",
                        "lifecycle": gen.lifecycle.as_str(),
                    })
                } else {
                    gen.lifecycle = Lifecycle::Tombstoned;
                    json!({
                        "tombstoned": true,
                        "generation": generation,
                        "physical_key": gen.key,
                        "lifecycle": "Tombstoned",
                    })
                }
            }
            "object_delete_captured_key" => {
                let id = Self::expected_id(fixture).to_string();
                let row = self.rows.get_mut(&id).expect("row exists");
                let generation = args["generation"].as_u64().expect("generation");
                let gen = &row.generations[&generation];
                assert_eq!(
                    gen.lifecycle,
                    Lifecycle::Tombstoned,
                    "delete before authoritative fence"
                );
                let key = gen.key.clone();
                assert!(self.ever_used_keys.contains(&key));
                assert!(row
                    .generations
                    .iter()
                    .all(|(n, other)| *n == generation || other.key != key));
                let deleted = self.physical.remove(&key).is_some();
                let current = row.current;
                let current_gen = &row.generations[&current];
                json!({
                    "deleted_key": key,
                    "deleted": deleted,
                    "generation_2_key_present": current == 2 && self.physical.contains_key(&current_gen.key),
                    "generation_2_reference_count": current_gen.refs.len(),
                    "content_id_unchanged": id == content_id(&row.bytes),
                })
            }
            other => panic!("no reference model for {other}"),
        }
    }

    fn put(&mut self, args: &Value, fixture: &Value) -> Value {
        let raw = hex::decode(args["bytes_hex"].as_str().expect("bytes_hex")).expect("hex");
        let cid = content_id(&raw);
        assert_eq!(
            cid,
            fixture["expected_content_id"].as_str().expect("fixture id"),
            "fixture digest mismatch"
        );
        if let Some(prior) = self.rows.get_mut(&cid) {
            let current = prior.current;
            let cur_lifecycle = prior.generations[&current].lifecycle;
            if args.get("resurrection").and_then(Value::as_bool) == Some(true) {
                assert_eq!(cur_lifecycle, Lifecycle::Tombstoned);
                assert_eq!(
                    prior.bytes, raw,
                    "resurrection must preserve canonical content identity"
                );
                let generation = current + 1;
                let key = physical_key(&cid, generation);
                assert!(
                    !self.ever_used_keys.contains(&key),
                    "physical generation key reused"
                );
                prior.generations.insert(
                    generation,
                    Generation {
                        lifecycle: Lifecycle::Live,
                        refs: HashSet::new(),
                        row_revision: 1,
                        key: key.clone(),
                    },
                );
                prior.current = generation;
                self.physical.insert(key.clone(), raw);
                self.ever_used_keys.insert(key.clone());
                return json!({
                    "accepted": true, "created": true, "generation": generation,
                    "physical_key": key, "lifecycle": "Live", "row_revision": 1,
                    "content_id": cid,
                });
            }
            let gen = &prior.generations[&current];
            if gen.lifecycle == Lifecycle::Tombstoned {
                return json!({
                    "accepted": false, "created": false, "generation": current,
                    "lifecycle": "Tombstoned", "reason": "explicit-resurrection-required",
                });
            }
            assert_eq!(prior.bytes, raw);
            assert_eq!(prior.metadata, args["metadata"]);
            return json!({
                "accepted": true, "created": false, "generation": current,
                "lifecycle": "Live", "reference_count": gen.refs.len(),
                "row_revision": gen.row_revision,
            });
        }
        let generation = 1;
        let key = physical_key(&cid, generation);
        assert!(!self.ever_used_keys.contains(&key));
        let mut generations = BTreeMap::new();
        generations.insert(
            generation,
            Generation {
                lifecycle: Lifecycle::Live,
                refs: HashSet::new(),
                row_revision: 1,
                key: key.clone(),
            },
        );
        self.rows.insert(
            cid.clone(),
            Row {
                bytes: raw.clone(),
                metadata: args["metadata"].clone(),
                current: generation,
                generations,
            },
        );
        self.physical.insert(key.clone(), raw);
        self.ever_used_keys.insert(key.clone());
        json!({
            "accepted": true, "created": true, "generation": generation,
            "physical_key": key, "lifecycle": "Live", "row_revision": 1,
            "content_id": cid,
        })
    }
}
