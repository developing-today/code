//! Slice-2 oracles (#8623): per-corpus embedding-model identity,
//! mismatch refusal, migration, and the re-embed job end-to-end on
//! the agent-memory corpus, plus a cross-corpus comparable-score spot.

use std::path::PathBuf;

use atlas_core::aggregate::{embed_unit_into_shard, reembed_unit_into_shard};
use atlas_core::embed::DocEmbedder;
use atlas_core::error::Result;
use atlas_core::filters::Filters;
use atlas_core::memory::{
    AGENT_MEMORY_UNIT, parse_memory_export, register_memory_corpora, stage_memory_export_in_db,
};
use atlas_core::shards::EMBEDDING_MODEL_ID;
use atlas_core::shards::scan::{FanoutQuery, fanout_scan};
use atlas_core::shards::{Tier, TierCeiling, open_from_spec};
use atlas_core::stage::{Registry, UnitState};

const NOW: &str = "2026-11-24T00:00:00Z";

const EXPORT: &str = r#"[
  {
    "id": "mem-transcript",
    "text": "Continue the current task from the existing transcript, preserving completed work.",
    "vector": [0.25, 0.5, 0.75],
    "importance": 0.9,
    "category": "fact",
    "createdAt": 1763000000000.0,
    "agentId": "main"
  },
  {
    "id": "mem-deploy",
    "text": "The deploy window opens on Sunday evening after the freeze lifts.",
    "vector": [0.1, 0.2, 0.3],
    "importance": 0.4,
    "category": "fact",
    "createdAt": 1763100000000.0,
    "agentId": "main"
  }
]"#;

struct KeywordEmbedder;

impl DocEmbedder for KeywordEmbedder {
    fn embed_doc(&self, text: &str) -> Result<Vec<f32>> {
        if text.contains("transcript") {
            Ok(vec![1.0, 0.0, 0.0])
        } else {
            Ok(vec![0.0, 1.0, 0.0])
        }
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let d =
        std::env::temp_dir().join(format!("atlas-embed-id-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

/// Provision records the mandated per-corpus model identity, and the
/// ingestion-time gate passes for it.
#[test]
fn provision_records_the_mandated_model_identity() {
    let dir = temp_dir("provision");
    let registry = Registry::open(&dir.join("registry.db")).unwrap();
    let db = dir.join("unit.db");
    let unit = registry
        .provision(&db, "unit-a", "tester", Tier::Internal, NOW)
        .unwrap();
    assert_eq!(unit.embedding_model, EMBEDDING_MODEL_ID);
    registry.assert_corpus_model("unit-a").unwrap();
}

/// A stale recorded model refuses loudly at every ingestion gate:
/// the registry check, the memory connector, and promotion.
#[test]
fn stale_recorded_model_refuses_at_every_ingestion_gate() {
    let dir = temp_dir("stale");
    let registry = Registry::open(&dir.join("registry.db")).unwrap();
    let db = dir.join("unit.db");
    registry
        .provision(&db, "unit-stale", "tester", Tier::Internal, NOW)
        .unwrap();
    registry
        .record_embedding_model("unit-stale", "retired-model-v0", NOW)
        .unwrap();

    let err = registry.assert_corpus_model("unit-stale").unwrap_err().to_string();
    assert!(err.contains("retired-model-v0"), "{err}");
    assert!(err.contains("re-embed"), "{err}");

    // The memory connector refuses a stale corpus instead of mixing vectors.
    let mem_dir = temp_dir("stale-mem");
    let mem_registry = Registry::open(&mem_dir.join("registry.db")).unwrap();
    let units = register_memory_corpora(&mem_registry, &mem_dir, NOW).unwrap();
    assert_eq!(units.len(), 5);
    mem_registry
        .record_embedding_model(AGENT_MEMORY_UNIT, "retired-model-v0", NOW)
        .unwrap();
    let err = register_memory_corpora(&mem_registry, &mem_dir, NOW)
        .unwrap_err()
        .to_string();
    assert!(err.contains(AGENT_MEMORY_UNIT), "{err}");
    assert!(err.contains("retired-model-v0"), "{err}");

    // Promotion refuses the stale unit even when its staging DB was
    // freshly stamped with the mandated model.
    let err = mem_registry
        .promote(AGENT_MEMORY_UNIT, NOW)
        .unwrap_err()
        .to_string();
    assert!(err.contains("retired-model-v0"), "{err}");
}

/// Pre-slice-2 registry databases (no `embedding_model` column) migrate
/// in place, defaulting every corpus to the mandated model.
#[test]
fn pre_slice2_registry_migrates_to_the_mandated_model() {
    let dir = temp_dir("migrate");
    let path = dir.join("registry.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE staging_units (unit_id TEXT PRIMARY KEY, db_uri TEXT NOT NULL, owner TEXT NOT NULL, sensitivity TEXT NOT NULL, state TEXT NOT NULL, schema_version INTEGER NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL)",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO staging_units VALUES ('legacy', '/tmp/x.db', 'o', 'internal', 'open', 1, 't', 't')",
            [],
        )
        .unwrap();
    }
    let registry = Registry::open(&path).unwrap();
    let unit = registry.lookup("legacy").unwrap().unwrap();
    assert_eq!(unit.embedding_model, EMBEDDING_MODEL_ID);
    registry.assert_corpus_model("legacy").unwrap();
}

/// The re-embed job rebuilds the named agent-memory corpus: the before
/// count from the promotion archive equals the after count in the new
/// shard part, the registry record closes out to the mandated model,
/// and the rebuilt shard stays searchable.
#[test]
fn reembed_rebuilds_agent_memory_with_matching_counts() {
    let dir = temp_dir("reembed");
    let registry = Registry::open(&dir.join("registry.db")).unwrap();
    let units = register_memory_corpora(&registry, &dir, NOW).unwrap();
    let agent_db = units
        .iter()
        .find(|u| u.unit_id == AGENT_MEMORY_UNIT)
        .map(|u| PathBuf::from(&u.db_uri))
        .unwrap();
    let entries = parse_memory_export(EXPORT).unwrap();
    stage_memory_export_in_db(&agent_db, AGENT_MEMORY_UNIT, &entries, NOW).unwrap();

    let shard_dir = dir.join("shards");
    let promoted = embed_unit_into_shard(
        &registry,
        AGENT_MEMORY_UNIT,
        &shard_dir,
        "agent-memory",
        &KeywordEmbedder,
        NOW,
    )
    .unwrap();
    assert_eq!(promoted.items, 2);
    assert_eq!(promoted.chunks, 2);

    let rebuilt = reembed_unit_into_shard(
        &registry,
        AGENT_MEMORY_UNIT,
        &shard_dir,
        "agent-memory",
        &KeywordEmbedder,
        NOW,
    )
    .unwrap();
    assert_eq!(rebuilt.unit_id, AGENT_MEMORY_UNIT);
    assert_eq!(rebuilt.items, promoted.items);
    assert_eq!(rebuilt.prior_chunks, promoted.chunks);
    assert_eq!(rebuilt.chunks, promoted.chunks);
    assert_eq!(rebuilt.embedding_model, EMBEDDING_MODEL_ID);
    assert_ne!(rebuilt.shard_path, promoted.shard_path);
    assert!(
        rebuilt.shard_name.ends_with("-01"),
        "re-embed writes the next part, keeping the promoted shard: {}",
        rebuilt.shard_name
    );
    assert_eq!(
        registry
            .lookup(AGENT_MEMORY_UNIT)
            .unwrap()
            .unwrap()
            .embedding_model,
        EMBEDDING_MODEL_ID
    );
    assert_eq!(
        registry
            .lookup(AGENT_MEMORY_UNIT)
            .unwrap()
            .unwrap()
            .state,
        UnitState::Merged
    );

    // The rebuilt shard carries the mandated model in its manifest.
    let conn = rusqlite::Connection::open(&rebuilt.shard_path).unwrap();
    let model: String = conn
        .query_row(
            "SELECT value FROM atlas_manifest WHERE \"key\" = 'embedding_model'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(model, EMBEDDING_MODEL_ID);

    // ... and the rebuilt shard is searchable by the fan-out
    // (copied alone into a dir so the promoted original does not
    // dedupe its chunk ids away in the merge).
    let only_dir = dir.join("rebuilt-only");
    std::fs::create_dir_all(&only_dir).unwrap();
    std::fs::copy(
        &rebuilt.shard_path,
        only_dir.join(rebuilt.shard_name.clone() + ".db"),
    )
    .unwrap();
    let set = open_from_spec(
        &format!("dir:{}", only_dir.display()),
        TierCeiling(Tier::Internal),
        atlas_core::DEFAULT_VEC_TABLE,
    )
    .unwrap();
    let filters = Filters::new(vec![], vec![], vec![], vec![], "personal", None, None).unwrap();
    let hits = fanout_scan(
        &set,
        &FanoutQuery {
            qvec: &[1.0, 0.0, 0.0],
            shard_top_k: 10,
            global_top_k: 10,
            filters: &filters,
            topics: None,
            ceiling: TierCeiling(Tier::Internal),
        },
    )
    .unwrap()
    .hits;
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].chunk_id, 1);
}

/// Cross-corpus spot: two corpora promoted under the one mandated model
/// record the same identity and return comparable fan-out scores.
#[test]
fn two_corpora_under_one_model_compare_scores() {
    let dir = temp_dir("xcorpus");
    let registry = Registry::open(&dir.join("registry.db")).unwrap();
    let units = register_memory_corpora(&registry, &dir, NOW).unwrap();
    let db_for = |id: &str| {
        units
            .iter()
            .find(|u| u.unit_id == id)
            .map(|u| PathBuf::from(&u.db_uri))
            .unwrap()
    };
    let agent_db = db_for(AGENT_MEMORY_UNIT);
    let fleet_db = db_for("fleet-memory");
    let entries = parse_memory_export(EXPORT).unwrap();
    stage_memory_export_in_db(&agent_db, AGENT_MEMORY_UNIT, &entries, NOW).unwrap();
    stage_memory_export_in_db(&fleet_db, "fleet-memory", &entries, NOW).unwrap();

    let shard_dir = dir.join("shards");
    let a = embed_unit_into_shard(
        &registry,
        AGENT_MEMORY_UNIT,
        &shard_dir,
        "agent-memory",
        &KeywordEmbedder,
        NOW,
    )
    .unwrap();
    let b = embed_unit_into_shard(
        &registry,
        "fleet-memory",
        &shard_dir,
        "agent-memory",
        &KeywordEmbedder,
        NOW,
    )
    .unwrap();
    assert_eq!(a.dims, b.dims);
    for shard in [&a.shard_path, &b.shard_path] {
        let conn = rusqlite::Connection::open(shard).unwrap();
        let model: String = conn
            .query_row(
                "SELECT value FROM atlas_manifest WHERE \"key\" = 'embedding_model'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(model, EMBEDDING_MODEL_ID);
    }

    let set = open_from_spec(
        &format!("dir:{}", shard_dir.display()),
        TierCeiling(Tier::Internal),
        atlas_core::DEFAULT_VEC_TABLE,
    )
    .unwrap();
    let filters = Filters::new(vec![], vec![], vec![], vec![], "personal", None, None).unwrap();
    let result = fanout_scan(
        &set,
        &FanoutQuery {
            qvec: &[1.0, 0.0, 0.0],
            shard_top_k: 10,
            global_top_k: 10,
            filters: &filters,
            topics: None,
            ceiling: TierCeiling(Tier::Internal),
        },
    )
    .unwrap();
    // Both shards scan (the M1 unified-model gate passes), and the
    // merge dedupes the shared chunk ids to one hit per id — the
    // scores that survive are finite, i.e. comparable, across corpora
    // under the one model.
    assert_eq!(result.scanned.len(), 2, "both corpora scan");
    assert_eq!(result.hits.len(), 2, "shared chunk ids merge to one hit each");
    assert!(
        result.hits.iter().all(|h| h.dist.is_finite()),
        "scores are comparable (finite) across corpora under one model"
    );
}
