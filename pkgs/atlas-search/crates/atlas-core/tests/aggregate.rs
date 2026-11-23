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
use proptest::prelude::*;

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

/// A document embedder whose token is a word, with a tiny context:
/// forces the aggregator's embed-time split at a readable scale.
struct WordEmbedder;

impl DocEmbedder for WordEmbedder {
    fn embed_doc(&self, text: &str) -> Result<Vec<f32>> {
        if text.contains("dns") {
            Ok(vec![1.0, 0.0])
        } else {
            Ok(vec![0.0, 1.0])
        }
    }

    fn token_count(&self, text: &str) -> Option<usize> {
        Some(text.split_whitespace().count())
    }

    fn max_input_tokens(&self) -> usize {
        3
    }
}

#[test]
fn promotion_splits_a_chunk_that_exceeds_the_model_context() {
    let dir = temp_dir("split");
    let registry = Registry::open(&dir.join("registry.db")).unwrap();
    let db = dir.join("unit.db");
    registry
        .provision(&db, "task-split", "repo-worker-code", Tier::Internal, NOW)
        .unwrap();
    // One staged whole-file chunk of 8 word-tokens; the embedder's
    // context is 3, so promotion must split it into pieces of at
    // most 3 words that partition the text exactly.
    let text = "dns alpha beta\ndns gamma delta\ndns epsilon zeta\n";
    let entry = ArtifactEntry {
        kind: "config".into(),
        repo: "developing-today/code".into(),
        path: "infra/dns/dns_config.yaml".into(),
        commit_sha: "abc".into(),
        language: "yaml".into(),
        sensitivity: Tier::Internal,
        text: text.into(),
    };
    stage_artifact_in_db(&db, "task-split", &entry, NOW).unwrap();

    let shard_dir = dir.join("shards");
    let report = embed_unit_into_shard(
        &registry,
        "task-split",
        &shard_dir,
        "codeconfig",
        &WordEmbedder,
        NOW,
    )
    .unwrap();
    assert!(
        report.chunks >= 3,
        "split into pieces, got {}",
        report.chunks
    );

    // The pieces, read back from the shard in chunk order,
    // partition the staged text exactly.
    let conn = rusqlite::Connection::open(&report.shard_path).unwrap();
    let mut stmt = conn.prepare("SELECT text FROM chunks ORDER BY id").unwrap();
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(rows.concat(), text);
    for piece in &rows {
        assert!(
            piece.split_whitespace().count() <= 3,
            "piece fits: {piece:?}"
        );
    }
    let spans = conn
        .prepare("SELECT span_start, span_end FROM chunk_occurrences ORDER BY shard_chunk_id")
        .unwrap()
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        spans.len(),
        rows.len(),
        "each split piece has an occurrence"
    );
    let mut offset = 0;
    for ((start, end), piece) in spans.iter().zip(&rows) {
        assert_eq!(*start, offset, "source spans are contiguous");
        assert_eq!(*end - *start, piece.len() as i64, "span covers exact bytes");
        offset = *end;
    }
    assert_eq!(offset, text.len() as i64, "spans cover the original item");
}

#[test]
fn promotion_keeps_pieces_and_items_with_identical_text() {
    // #10836: the shard's content_hash is UNIQUE, so a chunk that
    // splits into identical pieces, or two items staging the same
    // file, must still promote with every piece kept.
    let dir = temp_dir("dup-pieces");
    let registry = Registry::open(&dir.join("registry.db")).unwrap();
    let db = dir.join("unit.db");
    registry
        .provision(&db, "task-dup", "repo-worker-code", Tier::Internal, NOW)
        .unwrap();
    let text = "dns alpha beta\ndns alpha beta\ndns alpha beta\n";
    for path in ["infra/dns/a.yaml", "infra/dns/b.yaml"] {
        let entry = ArtifactEntry {
            kind: "config".into(),
            repo: "developing-today/code".into(),
            path: path.into(),
            commit_sha: "abc".into(),
            language: "yaml".into(),
            sensitivity: Tier::Internal,
            text: text.into(),
        };
        stage_artifact_in_db(&db, "task-dup", &entry, NOW).unwrap();
    }

    let shard_dir = dir.join("shards");
    let report = embed_unit_into_shard(
        &registry,
        "task-dup",
        &shard_dir,
        "codeconfig",
        &WordEmbedder,
        NOW,
    )
    .unwrap();
    assert_eq!(report.chunks, 6, "three pieces per item, none dropped");

    let conn = rusqlite::Connection::open(&report.shard_path).unwrap();
    let mut stmt = conn
        .prepare("SELECT message_id, text, content_hash FROM chunks ORDER BY id")
        .unwrap();
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    let hashes: std::collections::HashSet<&str> = rows.iter().map(|r| r.2.as_str()).collect();
    assert_eq!(hashes.len(), rows.len(), "every content_hash is unique");
    let mut by_source: std::collections::BTreeMap<&str, String> = std::collections::BTreeMap::new();
    for (source, piece, _) in &rows {
        by_source
            .entry(source.as_str())
            .or_default()
            .push_str(piece);
    }
    assert_eq!(by_source.len(), 2, "both items keep their pieces");
    for joined in by_source.values() {
        assert_eq!(joined, text);
    }
}

#[test]
fn promotion_records_chunk_occurrences_for_identical_text() {
    // #3125 rule 3: every chunk records every place it was found.
    // Two items staging byte-identical text must promote to two
    // shard chunks with distinct shard-unique content_hashes AND
    // one occurrence row each sharing the canonical text hash, so
    // the duplication link is recorded instead of silently lost.
    // Oracle: SHA-256 of the staged text (content identity); row
    // count and shared canonical hash are the acceptance signal.
    let dir = temp_dir("occurrences");
    let registry = Registry::open(&dir.join("registry.db")).unwrap();
    let db = dir.join("unit.db");
    registry
        .provision(&db, "task-occ", "repo-worker-code", Tier::Internal, NOW)
        .unwrap();
    let text = "dns shared block\n";
    for path in ["infra/dns/a.yaml", "infra/dns/b.yaml"] {
        let entry = ArtifactEntry {
            kind: "config".into(),
            repo: "developing-today/code".into(),
            path: path.into(),
            commit_sha: "abc".into(),
            language: "yaml".into(),
            sensitivity: Tier::Internal,
            text: text.into(),
        };
        stage_artifact_in_db(&db, "task-occ", &entry, NOW).unwrap();
    }

    let shard_dir = dir.join("shards");
    let report = embed_unit_into_shard(
        &registry,
        "task-occ",
        &shard_dir,
        "codeconfig",
        &KeywordEmbedder,
        NOW,
    )
    .unwrap();
    assert_eq!(report.chunks, 2, "both occurrences kept as chunks");

    let conn = rusqlite::Connection::open(&report.shard_path).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT shard_chunk_id, canonical_hash, source, span_start, span_end, observed_at \
             FROM chunk_occurrences ORDER BY shard_chunk_id",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
            ))
        })
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(rows.len(), 2, "one occurrence row per chunk");
    assert_eq!(rows[0].0, 1);
    assert_eq!(rows[1].0, 2);
    assert_eq!(
        rows[0].1, rows[1].1,
        "identical texts share the canonical hash"
    );
    assert_ne!(rows[0].2, rows[1].2, "occurrences record distinct sources");
    for row in &rows {
        assert_eq!((row.3, row.4), (0, text.len() as i64));
        assert_eq!(row.5, NOW, "observed_at is the staged observation time");
    }

    // The shard-unique content_hashes differ (UNIQUE constraint),
    // but the canonical hashes agree with SHA-256 of the text.
    use sha2::{Digest, Sha256};
    let want: String = Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(rows[0].1, want, "canonical hash is SHA-256 of the text");
}

// Oracle: every stored occurrence span selects exactly the persisted
// chunk bytes from its source item's original UTF-8 text, and the
// ordered spans form a gap-free, overlap-free partition of each item.
// Generated documents contain multibyte characters, line boundaries,
// multiple source items, and enough tokens to exercise embed-time splits.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn promotion_occurrence_spans_partition_generated_utf8_sources(
        documents in prop::collection::vec(
            prop::collection::vec(
                prop::collection::vec(
                    prop_oneof![Just('a'), Just('é'), Just('🦀'), Just('β'), Just(' '), Just('\t')],
                    1..16,
                ),
                6..14,
            ),
            2..5,
        )
    ) {
        struct FourTokenEmbedder;
        impl DocEmbedder for FourTokenEmbedder {
            fn embed_doc(&self, _text: &str) -> Result<Vec<f32>> {
                Ok(vec![1.0, 0.0])
            }

            fn token_count(&self, text: &str) -> Option<usize> {
                Some(text.split_whitespace().count())
            }

            fn max_input_tokens(&self) -> usize {
                4
            }
        }

        let originals: Vec<String> = documents
            .iter()
            .map(|lines| {
                lines
                    .iter()
                    .map(|line| line.iter().collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n"
            })
            .collect();
        let dir = temp_dir("property-spans");
        let registry = Registry::open(&dir.join("registry.db")).unwrap();
        let db = dir.join("unit.db");
        registry.provision(&db, "task-property", "repo-worker-code", Tier::Internal, NOW).unwrap();

        let mut expected = std::collections::BTreeMap::<String, Vec<u8>>::new();
        for (index, text) in originals.iter().enumerate() {
            let path = format!("generated/source-{index}.txt");
            let source = format!("developing-today/code:{path}@abc");
            expected.insert(source, text.as_bytes().to_vec());
            let entry = ArtifactEntry {
                kind: "prose".into(),
                repo: "developing-today/code".into(),
                path,
                commit_sha: "abc".into(),
                language: "txt".into(),
                sensitivity: Tier::Internal,
                text: text.clone(),
            };
            stage_artifact_in_db(&db, "task-property", &entry, NOW).unwrap();
        }

        let report = embed_unit_into_shard(
            &registry,
            "task-property",
            &dir.join("shards"),
            "property",
            &FourTokenEmbedder,
            NOW,
        ).unwrap();
        let conn = rusqlite::Connection::open(&report.shard_path).unwrap();
        let mut stmt = conn.prepare(
            "SELECT o.source, o.span_start, o.span_end, c.text \
             FROM chunk_occurrences o JOIN chunks c ON c.id = o.shard_chunk_id \
             ORDER BY o.source, o.span_start, o.shard_chunk_id",
        ).unwrap();
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        }).unwrap().collect::<std::result::Result<Vec<_>, _>>().unwrap();

        let mut grouped = std::collections::BTreeMap::<String, Vec<(i64, i64, String)>>::new();
        for (source, start, end, chunk) in rows {
            grouped.entry(source).or_default().push((start, end, chunk));
        }
        prop_assert_eq!(grouped.len(), expected.len(), "each source has occurrence rows");
        for (source, original) in &expected {
            let spans = grouped.get(source).expect("every staged source is registered");
            let mut cursor = 0usize;
            for (start, end, chunk) in spans {
                prop_assert_eq!(*start as usize, cursor, "source spans have no gaps or overlaps");
                let end = *end as usize;
                prop_assert!(end <= original.len(), "span ends within source bytes");
                prop_assert_eq!(
                    &original[cursor..end],
                    chunk.as_bytes(),
                    "occurrence selects the exact persisted UTF-8 chunk bytes",
                );
                cursor = end;
            }
            prop_assert_eq!(cursor, original.len(), "spans cover all source bytes");
        }
    }
}
