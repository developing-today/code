//! Oracle tests for the staging aggregator's embed-at-promotion
//! step (#5669 aggregator slice, #1518): a staged code/config unit
//! is embedded into a new core scan shard, the shard is discoverable
//! and searchable by the production fan-out, and the unit ends
//! promoted (merged) through the M6 gate.

use std::path::{Path, PathBuf};

use atlas_core::aggregate::{embed_unit_into_shard, tier_sensitivity_name};
use atlas_core::embed::DocEmbedder;
use atlas_core::error::Result;
use atlas_core::filters::Filters;
use atlas_core::shards::scan::{FanoutQuery, fanout_scan};
use atlas_core::shards::{Tier, TierCeiling, open_from_spec};
use atlas_core::stage::{ArtifactEntry, Registry, UnitState, stage_artifact_in_db};

const NOW: &str = "2026-11-16T00:00:00Z";

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-aggregate-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

/// A document embedder keyed on content: DNS text lands on the
/// first axis, everything else on the second — enough structure for
/// the fan-out oracle to rank by.
struct KeywordEmbedder;

impl DocEmbedder for KeywordEmbedder {
    fn embed_doc(&self, text: &str) -> Result<Vec<f32>> {
        if text.contains("dns") || text.contains("DNS") {
            Ok(vec![1.0, 0.0, 0.0])
        } else {
            Ok(vec![0.0, 1.0, 0.0])
        }
    }
}

fn stage_unit(dir: &Path) -> (Registry, PathBuf) {
    let registry_path = dir.join("registry.db");
    let registry = Registry::open(&registry_path).unwrap();
    let db = dir.join("unit.db");
    registry
        .provision(&db, "task-17164", "repo-worker-code", Tier::Internal, NOW)
        .unwrap();
    let dns = ArtifactEntry {
        kind: "config".into(),
        repo: "developing-today/code".into(),
        path: "infra/dns/dns_config.yaml".into(),
        commit_sha: "d85bd4ff1f3b73700f881aa99357adeae997e1bd".into(),
        language: "yaml".into(),
        sensitivity: Tier::Internal,
        text: "dns:\n  zones:\n    - name: example.internal\n".into(),
    };
    stage_artifact_in_db(&db, "task-17164", &dns, NOW).unwrap();
    let prose = ArtifactEntry {
        kind: "prose".into(),
        repo: "developing-today/code".into(),
        path: "doc/notes.md".into(),
        commit_sha: "d85bd4ff1f3b73700f881aa99357adeae997e1bd".into(),
        language: "md".into(),
        sensitivity: Tier::Internal,
        text: "release checklist and meeting notes".into(),
    };
    stage_artifact_in_db(&db, "task-17164", &prose, NOW).unwrap();
    (registry, db)
}

#[test]
fn promotion_embeds_the_unit_into_a_searchable_shard_and_merges_it() {
    let dir = temp_dir("promote");
    let (registry, _db) = stage_unit(&dir);
    let shard_dir = dir.join("shards");

    // Before the aggregator runs, the registry gate refuses: the
    // unit DB records no embedding model (unknown provenance).
    assert!(registry.promote("task-17164", NOW).is_err());
    assert_eq!(
        registry.lookup("task-17164").unwrap().unwrap().state,
        UnitState::Open
    );

    let report = embed_unit_into_shard(
        &registry,
        "task-17164",
        &shard_dir,
        "codeconfig",
        &KeywordEmbedder,
        NOW,
    )
    .unwrap();
    assert_eq!(report.items, 2);
    assert_eq!(report.chunks, 2);
    assert_eq!(report.dims, 3);
    assert_eq!(report.shard_name, "atlas-core-internal-codeconfig-00");
    assert!(report.shard_path.exists());
    assert_eq!(
        registry.lookup("task-17164").unwrap().unwrap().state,
        UnitState::Merged
    );

    // The shard is discoverable by the production path and the
    // fan-out ranks the DNS config chunk first for a DNS-shaped
    // query vector.
    let set = open_from_spec(
        &format!("dir:{}", shard_dir.display()),
        TierCeiling(Tier::Internal),
        atlas_core::DEFAULT_VEC_TABLE,
    )
    .unwrap();
    assert_eq!(set.shards().len(), 1);
    assert_eq!(set.shards()[0].chunks, 2);
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
    assert_eq!(hits[0].chunk_id, 1, "the DNS config chunk ranks first");
    assert!(hits[0].dist < hits[1].dist);
}

#[test]
fn promotion_refuses_an_empty_unit_and_a_second_run() {
    let dir = temp_dir("refusals");
    let registry = Registry::open(&dir.join("registry.db")).unwrap();
    let db = dir.join("empty.db");
    registry
        .provision(&db, "task-empty", "repo-worker-code", Tier::Internal, NOW)
        .unwrap();
    let shard_dir = dir.join("shards");
    let err = embed_unit_into_shard(
        &registry,
        "task-empty",
        &shard_dir,
        "codeconfig",
        &KeywordEmbedder,
        NOW,
    )
    .unwrap_err();
    assert!(err.to_string().contains("no staged chunks"), "{err}");
    assert_eq!(
        registry.lookup("task-empty").unwrap().unwrap().state,
        UnitState::Open
    );

    // A second promotion of a merged unit refuses.
    let dir2 = temp_dir("second-run");
    let (registry2, _db2) = stage_unit(&dir2);
    let shard_dir2 = dir2.join("shards");
    embed_unit_into_shard(
        &registry2,
        "task-17164",
        &shard_dir2,
        "codeconfig",
        &KeywordEmbedder,
        NOW,
    )
    .unwrap();
    let err = embed_unit_into_shard(
        &registry2,
        "task-17164",
        &shard_dir2,
        "codeconfig",
        &KeywordEmbedder,
        NOW,
    )
    .unwrap_err();
    assert!(err.to_string().contains("already merged"), "{err}");
}

#[test]
fn tier_sensitivity_names_match_the_scan_ladder() {
    assert_eq!(tier_sensitivity_name(Tier::Public), "shareable");
    assert_eq!(tier_sensitivity_name(Tier::Internal), "personal");
    assert_eq!(tier_sensitivity_name(Tier::Secret), "secret-bearing");
}
