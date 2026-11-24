//! Oracle tests for the #12628 wiring of boundary-safe chunks into
//! staging (`stage_classified_artifact`) and promotion
//! (`embed_unit_into_shard`):
//!
//! * staging composes a producer's declared classification segments
//!   through `split_by_boundaries` — chunks never cross a declared
//!   boundary, and gaps / overlaps / uncertain declarations fail
//!   closed to `secret-bearing`;
//! * one #12408 classification record per chunk is persisted with
//!   the chunk's exact source offsets and effective sensitivity;
//! * promotion writes each chunk's effective sensitivity into the
//!   shard (not the item's blanket tier) and embeds each distinct
//!   text once, so unchanged chunks keep their one vector.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use atlas_core::aggregate::embed_unit_into_shard;
use atlas_core::boundary::ClassificationSegment;
use atlas_core::embed::DocEmbedder;
use atlas_core::error::Result;
use atlas_core::filters::{Filters, Sensitivity};
use atlas_core::shards::scan::{FanoutQuery, fanout_scan};
use atlas_core::shards::{Tier, TierCeiling, open_from_spec};
use atlas_core::stage::{
    ArtifactEntry, ChunkClassification, Registry, UnitState, chunk_classifications,
    stage_classified_artifact_in_db,
};

const NOW: &str = "2026-11-22T00:00:00Z";

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-wiring-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

fn entry(path: &str, text: &str) -> ArtifactEntry {
    ArtifactEntry {
        kind: "prose".into(),
        repo: "developing-today/code".into(),
        path: path.into(),
        commit_sha: "abc123".into(),
        language: "md".into(),
        sensitivity: Tier::Internal,
        text: text.into(),
    }
}

fn provision(dir: &Path, unit: &str) -> (Registry, PathBuf) {
    let registry = Registry::open(&dir.join("registry.db")).unwrap();
    let db = dir.join("unit.db");
    registry
        .provision(&db, unit, "repo-worker", Tier::Internal, NOW)
        .unwrap();
    (registry, db)
}

/// The staged chunks of one item, in chunk order.
fn staged_chunks(db: &Path, item_id: i64) -> Vec<String> {
    let conn = rusqlite::Connection::open(db).unwrap();
    let mut stmt = conn
        .prepare("SELECT text FROM chunks_raw WHERE item_id = ?1 ORDER BY idx")
        .unwrap();
    stmt.query_map([item_id], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap()
}

fn classifications(db: &Path, item_id: i64) -> Vec<ChunkClassification> {
    let conn = rusqlite::Connection::open(db).unwrap();
    chunk_classifications(&conn, item_id)
        .unwrap()
        .expect("classified item carries records")
}

#[test]
fn staging_transcript_turns_become_boundary_chunks_with_records() {
    let dir = temp_dir("turns");
    let (_registry, db) = provision(&dir, "unit-turns");
    let trip = "Trip: book flights for the offsite next week.\n";
    let deploy = "Deploy: roll out the new queue worker tonight.\n";
    let text = format!("{trip}{deploy}");
    let segments = [
        ClassificationSegment::certain(0, trip.len(), Sensitivity::Personal),
        ClassificationSegment::certain(trip.len(), text.len(), Sensitivity::Shareable),
    ];
    let item_id = stage_classified_artifact_in_db(
        &db,
        "unit-turns",
        &entry("doc/chat.md", &text),
        &segments,
        NOW,
    )
    .unwrap();

    // Oracle: the chunks are exactly the two turns — no chunk
    // carries text from the other turn.
    assert_eq!(staged_chunks(&db, item_id), vec![trip, deploy]);

    // Oracle: one persisted record per chunk with exact offsets and
    // effective sensitivity.
    let records = classifications(&db, item_id);
    assert_eq!(records.len(), 2);
    assert_eq!(
        (records[0].start, records[0].end, records[0].sensitivity),
        (0, trip.len(), Sensitivity::Personal)
    );
    assert_eq!(
        (records[1].start, records[1].end, records[1].sensitivity),
        (trip.len(), text.len(), Sensitivity::Shareable)
    );
    assert!(!records[0].uncertain && !records[1].uncertain);
    assert!(
        records[0].embed_id.ends_with("doc/chat.md@abc123#chunk-0"),
        "embed id names the source and chunk: {}",
        records[0].embed_id
    );
    assert!(records[1].embed_id.ends_with("#chunk-1"));
}

#[test]
fn staging_embedded_secret_is_isolated_in_its_own_chunk() {
    let dir = temp_dir("secret");
    let (_registry, db) = provision(&dir, "unit-secret");
    let before = "fn deploy() {\n    let region = \"eu-west-1\";\n";
    let secret = "    let token = \"AKIAIOSFODNN7EXAMPLE\";\n";
    let after = "    start_server(region);\n}\n";
    let text = format!("{before}{secret}{after}");
    let secret_start = before.len();
    let secret_end = secret_start + secret.len();
    let segments = [
        ClassificationSegment::certain(0, secret_start, Sensitivity::Shareable),
        ClassificationSegment::certain(secret_start, secret_end, Sensitivity::SecretBearing),
        ClassificationSegment::certain(secret_end, text.len(), Sensitivity::Shareable),
    ];
    let mut e = entry("src/deploy.rs", &text);
    e.kind = "code".into();
    e.language = "rust".into();
    let item_id = stage_classified_artifact_in_db(&db, "unit-secret", &e, &segments, NOW).unwrap();

    let chunks = staged_chunks(&db, item_id);
    assert_eq!(chunks, vec![before, secret, after]);
    // Oracle: the secret text appears in exactly one chunk — no
    // overlap copies it into a neighbour.
    let carriers = chunks
        .iter()
        .filter(|c| c.contains("AKIAIOSFODNN7EXAMPLE"))
        .count();
    assert_eq!(carriers, 1);
    assert_eq!(chunks.concat(), text, "chunks partition the source");

    let records = classifications(&db, item_id);
    let sensitivities: Vec<Sensitivity> = records.iter().map(|r| r.sensitivity).collect();
    assert_eq!(
        sensitivities,
        vec![
            Sensitivity::Shareable,
            Sensitivity::SecretBearing,
            Sensitivity::Shareable
        ]
    );
    assert_eq!(
        (records[1].start, records[1].end),
        (secret_start, secret_end)
    );
}

#[test]
fn staging_code_and_log_units_align_to_declared_units() {
    let dir = temp_dir("units");
    let (_registry, db) = provision(&dir, "unit-units");

    // Code: one segment per declaration.
    let code = "fn alpha() {}\nfn beta() {}\n";
    let segments = [
        ClassificationSegment::certain(0, 14, Sensitivity::Shareable),
        ClassificationSegment::certain(14, code.len(), Sensitivity::Personal),
    ];
    let mut e = entry("src/lib.rs", code);
    e.kind = "code".into();
    let item_id = stage_classified_artifact_in_db(&db, "unit-units", &e, &segments, NOW).unwrap();
    assert_eq!(
        staged_chunks(&db, item_id),
        vec!["fn alpha() {}\n", "fn beta() {}\n"]
    );

    // Logs: one segment per record.
    let logs = "2026-11-22T09:00:00Z INFO started\n2026-11-22T09:01:00Z ERROR boom\n";
    let first_len = "2026-11-22T09:00:00Z INFO started\n".len();
    let segments = [
        ClassificationSegment::certain(0, first_len, Sensitivity::Shareable),
        ClassificationSegment::certain(first_len, logs.len(), Sensitivity::LocalOnly),
    ];
    let item_id = stage_classified_artifact_in_db(
        &db,
        "unit-units",
        &entry("logs/run.log", logs),
        &segments,
        NOW,
    )
    .unwrap();
    let records = classifications(&db, item_id);
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].sensitivity, Sensitivity::LocalOnly);
    assert_eq!((records[1].start, records[1].end), (first_len, logs.len()));
}

#[test]
fn staging_uncertain_segment_fails_closed() {
    let dir = temp_dir("uncertain");
    let (_registry, db) = provision(&dir, "unit-uncertain");
    let text = "aaaaBBBBcccc";
    let segments = [
        ClassificationSegment::certain(0, 4, Sensitivity::Shareable),
        ClassificationSegment::uncertain(4, 8, Sensitivity::Personal),
        ClassificationSegment::certain(8, 12, Sensitivity::Shareable),
    ];
    let item_id = stage_classified_artifact_in_db(
        &db,
        "unit-uncertain",
        &entry("doc/x.md", text),
        &segments,
        NOW,
    )
    .unwrap();
    let records = classifications(&db, item_id);
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].sensitivity, Sensitivity::Shareable);
    // Oracle: uncertain material is secret-bearing and marked
    // uncertain — never joined to the less restricted neighbours.
    assert_eq!(records[1].sensitivity, Sensitivity::SecretBearing);
    assert!(records[1].uncertain);
    assert_eq!(records[2].sensitivity, Sensitivity::Shareable);
    assert_eq!(staged_chunks(&db, item_id).concat(), text);
}

#[test]
fn staging_undeclared_gap_and_missing_segments_fail_closed() {
    let dir = temp_dir("gap");
    let (_registry, db) = provision(&dir, "unit-gap");

    // A gap between declared segments.
    let text = "aaaa----bbbb";
    let segments = [
        ClassificationSegment::certain(0, 4, Sensitivity::Shareable),
        ClassificationSegment::certain(8, 12, Sensitivity::Shareable),
    ];
    let item_id = stage_classified_artifact_in_db(
        &db,
        "unit-gap",
        &entry("doc/gap.md", text),
        &segments,
        NOW,
    )
    .unwrap();
    let records = classifications(&db, item_id);
    assert_eq!(records.len(), 3);
    assert_eq!(records[1].sensitivity, Sensitivity::SecretBearing);
    assert!(records[1].uncertain);
    assert_eq!(&text[records[1].start..records[1].end], "----");

    // No declarations at all: the whole artifact is one
    // secret-bearing, uncertain chunk.
    let item_id = stage_classified_artifact_in_db(
        &db,
        "unit-gap",
        &entry("doc/none.md", "unclassified material"),
        &[],
        NOW,
    )
    .unwrap();
    let records = classifications(&db, item_id);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].sensitivity, Sensitivity::SecretBearing);
    assert!(records[0].uncertain);
}

#[test]
fn staging_overlapping_declarations_fail_closed() {
    let dir = temp_dir("overlap");
    let (_registry, db) = provision(&dir, "unit-overlap");
    let text = "abcdefgh";
    let segments = [
        ClassificationSegment::certain(0, 6, Sensitivity::Shareable),
        ClassificationSegment::certain(4, 8, Sensitivity::Personal),
    ];
    let item_id = stage_classified_artifact_in_db(
        &db,
        "unit-overlap",
        &entry("doc/o.md", text),
        &segments,
        NOW,
    )
    .unwrap();
    let records = classifications(&db, item_id);
    // Oracle: only the contradicted overlap fails closed; the
    // uncontested ranges keep their declared sensitivity.
    let got: Vec<(usize, usize, Sensitivity, bool)> = records
        .iter()
        .map(|r| (r.start, r.end, r.sensitivity, r.uncertain))
        .collect();
    assert_eq!(
        got,
        vec![
            (0, 4, Sensitivity::Shareable, false),
            (4, 6, Sensitivity::SecretBearing, true),
            (6, 8, Sensitivity::Personal, false),
        ]
    );
}

#[test]
fn staging_boundary_revision_changes_only_affected_records() {
    let dir = temp_dir("revision");
    let (_registry, db) = provision(&dir, "unit-revision");
    let text = "aaaabbbbcccc";
    let v1 = [
        ClassificationSegment::certain(0, 4, Sensitivity::Shareable),
        ClassificationSegment::certain(4, 8, Sensitivity::Personal),
        ClassificationSegment::certain(8, 12, Sensitivity::Shareable),
    ];
    let first =
        stage_classified_artifact_in_db(&db, "unit-revision", &entry("doc/r.md", text), &v1, NOW)
            .unwrap();
    // Revise only the middle segment's decision.
    let v2 = [
        ClassificationSegment::certain(0, 4, Sensitivity::Shareable),
        ClassificationSegment::certain(4, 8, Sensitivity::LocalOnly),
        ClassificationSegment::certain(8, 12, Sensitivity::Shareable),
    ];
    let second =
        stage_classified_artifact_in_db(&db, "unit-revision", &entry("doc/r.md", text), &v2, NOW)
            .unwrap();

    let before = classifications(&db, first);
    let after = classifications(&db, second);
    assert_eq!(before.len(), after.len());
    // Oracle: unaffected chunks' records are identical — same embed
    // id, offsets and sensitivity, so their classifications (and
    // vectors) stay valid; only the revised chunk's record changes,
    // and only in its sensitivity.
    assert_eq!(before[0], after[0]);
    assert_eq!(before[2], after[2]);
    assert_eq!(before[1].start, after[1].start);
    assert_eq!(before[1].end, after[1].end);
    assert_eq!(before[1].embed_id, after[1].embed_id);
    assert_eq!(before[1].sensitivity, Sensitivity::Personal);
    assert_eq!(after[1].sensitivity, Sensitivity::LocalOnly);
}

/// A document embedder keyed on content: DNS text lands on the
/// first axis, everything else on the second (mirrors the
/// aggregator oracle tests).
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

#[test]
fn promotion_applies_per_chunk_effective_sensitivity() {
    let dir = temp_dir("promote-sens");
    let (registry, db) = provision(&dir, "unit-promote-sens");
    let public = "dns zone config for example.internal\n";
    let secret = "dns token hunter2 do not share\n";
    let text = format!("{public}{secret}");
    let segments = [
        ClassificationSegment::certain(0, public.len(), Sensitivity::Shareable),
        ClassificationSegment::certain(public.len(), text.len(), Sensitivity::SecretBearing),
    ];
    stage_classified_artifact_in_db(
        &db,
        "unit-promote-sens",
        &entry("infra/dns.yaml", &text),
        &segments,
        NOW,
    )
    .unwrap();

    let shard_dir = dir.join("shards");
    let report = embed_unit_into_shard(
        &registry,
        "unit-promote-sens",
        &shard_dir,
        "codeconfig",
        &KeywordEmbedder,
        NOW,
    )
    .unwrap();
    assert_eq!(report.chunks, 2);
    assert_eq!(
        registry.lookup("unit-promote-sens").unwrap().unwrap().state,
        UnitState::Merged
    );

    // Oracle: the shard carries each chunk's own effective
    // sensitivity — not the item's blanket internal tier
    // ("personal"), which the legacy path would have written for
    // both chunks.
    let conn = rusqlite::Connection::open(&report.shard_path).unwrap();
    let mut stmt = conn
        .prepare("SELECT text, sensitivity FROM chunks ORDER BY id")
        .unwrap();
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            (public.to_string(), "shareable".to_string()),
            (secret.to_string(), "secret-bearing".to_string()),
        ]
    );

    // Oracle: at a personal ceiling the fan-out serves the shareable
    // chunk and withholds the secret-bearing one, even though both
    // live in the same internal-tier shard.
    let set = open_from_spec(
        &format!("dir:{}", shard_dir.display()),
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
    assert_eq!(hits.len(), 1, "only the shareable chunk is served");
    assert_eq!(hits[0].chunk_id, 1);
}

/// A document embedder that counts its calls: the oracle for
/// "unchanged chunks are not re-embedded". Its vector is a pure
/// function of the text, like the production model's.
struct CountingEmbedder {
    calls: RefCell<usize>,
}

impl DocEmbedder for CountingEmbedder {
    fn embed_doc(&self, text: &str) -> Result<Vec<f32>> {
        *self.calls.borrow_mut() += 1;
        Ok(vec![
            text.len() as f32,
            text.bytes().filter(|b| *b == b'a').count() as f32,
            1.0,
        ])
    }
}

#[test]
fn promotion_preserves_unchanged_vectors_embeds_each_text_once() {
    let dir = temp_dir("promote-dedup");
    let (registry, db) = provision(&dir, "unit-dedup");
    // Two items with identical text and identical declared segments
    // (the same file staged twice), plus one distinct chunk: three
    // distinct chunk texts across four chunks... two per item, the
    // pair repeated.
    let head = "alpha zone header\n";
    let tail = "omega zone footer\n";
    let text = format!("{head}{tail}");
    let segments = [
        ClassificationSegment::certain(0, head.len(), Sensitivity::Shareable),
        ClassificationSegment::certain(head.len(), text.len(), Sensitivity::Personal),
    ];
    for path in ["infra/a.yaml", "infra/b.yaml"] {
        stage_classified_artifact_in_db(&db, "unit-dedup", &entry(path, &text), &segments, NOW)
            .unwrap();
    }

    let embedder = CountingEmbedder {
        calls: RefCell::new(0),
    };
    let shard_dir = dir.join("shards");
    let report = embed_unit_into_shard(
        &registry,
        "unit-dedup",
        &shard_dir,
        "codeconfig",
        &embedder,
        NOW,
    )
    .unwrap();
    assert_eq!(report.chunks, 4, "every chunk is kept");
    // Oracle: only the two distinct texts were embedded — the
    // repeated item's chunks kept the first computation's vectors.
    assert_eq!(*embedder.calls.borrow(), 2);

    // Oracle: identical texts carry bit-identical vectors in the
    // shard.
    let conn = rusqlite::Connection::open(&report.shard_path).unwrap();
    let mut stmt = conn
        .prepare(&format!(
            "SELECT c.text, v.vec FROM chunks c JOIN {} v ON v.chunk_id = c.id \
             ORDER BY c.id",
            atlas_core::DEFAULT_VEC_TABLE
        ))
        .unwrap();
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
        })
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].0, rows[2].0);
    assert_eq!(rows[0].1, rows[2].1, "same text, same vector bytes");
    assert_eq!(rows[1].1, rows[3].1);
    assert_ne!(rows[0].1, rows[1].1);
}

/// A document embedder whose token is a word, with a tiny context:
/// forces the aggregator's embed-time split at a readable scale
/// (mirrors the aggregator oracle tests).
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
fn promotion_embed_split_pieces_inherit_effective_sensitivity() {
    let dir = temp_dir("promote-split");
    let (registry, db) = provision(&dir, "unit-promote-split");
    // One secret-bearing chunk of 8 word-tokens; the embedder's
    // context is 3, so promotion must split it — the pieces stay
    // within the chunk, so none may drop the classification.
    let text = "alpha beta gamma delta\nepsilon zeta eta theta\n";
    let segments = [ClassificationSegment::certain(
        0,
        text.len(),
        Sensitivity::SecretBearing,
    )];
    stage_classified_artifact_in_db(
        &db,
        "unit-promote-split",
        &entry("doc/secret.md", text),
        &segments,
        NOW,
    )
    .unwrap();

    let shard_dir = dir.join("shards");
    let report = embed_unit_into_shard(
        &registry,
        "unit-promote-split",
        &shard_dir,
        "codeconfig",
        &WordEmbedder,
        NOW,
    )
    .unwrap();
    assert!(report.chunks >= 3, "split into pieces: {}", report.chunks);

    let conn = rusqlite::Connection::open(&report.shard_path).unwrap();
    let mut stmt = conn
        .prepare("SELECT text, sensitivity FROM chunks ORDER BY id")
        .unwrap();
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    // Oracle: pieces partition the chunk exactly and every piece
    // keeps the chunk's secret-bearing classification.
    assert_eq!(rows.iter().map(|r| r.0.as_str()).collect::<String>(), text);
    for (_, sensitivity) in &rows {
        assert_eq!(sensitivity, "secret-bearing");
    }
}
