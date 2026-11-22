//! Classification operating-procedure tests (#12405).
//!
//! Oracles: the classification design
//! (`doc/2026-11-21T21-32-00Z_design_atlas_classification_model_v1.md`)
//! §5.1 (remainder sweep), §5.2 (schema-change review), and §8
//! guarantees 2, 4, 6, 7, and 10 — plus #12405's acceptance:
//! dry-run and committed runs enumerate only unclassified/
//! default-state embeds; previously classified embeds and
//! vectors are preserved; candidate context is assembled and
//! recorded; missing/contradictory context stays secret-bearing
//! with an explicit unresolved reason; the relatedness window is
//! declared and recorded; schema review examines changed
//! sections only and never lowers without a reviewed
//! disposition; runs are deterministic, resumable/idempotent,
//! and report manifest/result counts.

use std::collections::BTreeMap;
use std::path::PathBuf;

use atlas_core::classification::ClassificationRecord;
use atlas_core::classify::{
    CURRENT_SCHEMA_JSON, CandidateContext, DecisionOutcome, RECORDS_TABLE, RunConfig, SchemaDoc,
    apply_schema_review, decide, parse_dispositions, review_schema_change, run_classification,
};
use atlas_core::filters::Sensitivity;
use rusqlite::{Connection, params};
use serde_json::{Value as Json, json};

const NOW: &str = "2026-11-22T12:00:00Z";
const FAKE_TOKEN: &str = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

fn config() -> RunConfig {
    RunConfig {
        actor: "atlas-classify/remainder-sweep".to_string(),
        now: NOW.to_string(),
        window_days: 90,
        schema_version: 1,
        max_decisions: None,
    }
}

fn decided_record(embed_id: &str, sensitivity: &str) -> ClassificationRecord {
    ClassificationRecord::parse(json!({
        "schema_version": 1,
        "embed_id": embed_id,
        "sensitivity": sensitivity,
        "provenance": {
            "actor": "reviewer/1",
            "decided_at": "2026-11-20T00:00:00Z",
            "method": "manual",
            "method_version": "1",
            "config_version": "1",
            "evidence_refs": ["issue:1"],
            "previous_decision_id": null
        }
    }))
    .expect("fixture record parses")
}

fn insert_record(con: &Connection, record: &ClassificationRecord) {
    con.execute(
        &format!(
            "INSERT INTO {RECORDS_TABLE} (embed_id, record_json, sensitivity, schema_version, decided_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)"
        ),
        params![
            record.embed_id,
            record.to_json().to_string(),
            record.sensitivity.name(),
            record.schema_version as i64,
            record.provenance.decided_at,
        ],
    )
    .expect("insert fixture record");
}

/// The fixture corpus: twelve chunks across nine sessions,
/// covering every sweep branch. Sessions sA/sB/sF/sG/sH/sI share
/// project `atlas`; sD is 92 days before sA (outside the 90-day
/// window) and 19 days before sI (inside it).
fn fixture_db(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "atlas-classify-{}-{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let path = dir.join("corpus.db");
    let con = Connection::open(&path).expect("open fixture db");
    con.execute_batch(
        "CREATE TABLE sessions (id TEXT PRIMARY KEY, title TEXT, client TEXT, created_at TEXT);
         CREATE TABLE messages (id TEXT PRIMARY KEY, session_id TEXT, role TEXT, text TEXT);
         CREATE TABLE chunks (id INTEGER PRIMARY KEY, message_id TEXT NOT NULL, chunk_idx INTEGER, \
             text TEXT NOT NULL, content_hash TEXT UNIQUE NOT NULL, sensitivity TEXT);
         CREATE TABLE session_labels (session_id TEXT PRIMARY KEY, client TEXT, kind TEXT, \
             topics TEXT, projects TEXT, sensitivity TEXT, confidence TEXT, scheme TEXT);
         CREATE TABLE generic_labels (session_id TEXT NOT NULL, dimension TEXT NOT NULL, \
             values_json TEXT NOT NULL, PRIMARY KEY (session_id, dimension));
         CREATE TABLE vec_snowflake_m_v2_768_f32 (chunk_id INTEGER PRIMARY KEY, vec BLOB NOT NULL);",
    )
    .expect("fixture schema");
    let sessions = [
        ("sA", "2026-11-01T00:00:00Z"),
        ("sB", "2026-11-03T00:00:00Z"),
        ("sC", "2026-11-05T00:00:00Z"),
        ("sD", "2026-08-01T00:00:00Z"),
        ("sE", "2026-11-04T00:00:00Z"),
        ("sF", "2026-11-06T00:00:00Z"),
        ("sG", "2026-11-07T00:00:00Z"),
        ("sH", "2026-11-08T00:00:00Z"),
        ("sI", "2026-08-20T00:00:00Z"),
    ];
    for (id, created) in sessions {
        con.execute(
            "INSERT INTO sessions VALUES (?1, ?2, 'claude-code', ?3)",
            params![id, format!("session {id}"), created],
        )
        .expect("session");
        con.execute(
            "INSERT INTO messages VALUES (?1, ?2, 'user', 'msg')",
            params![format!("m{}1", id.trim_start_matches('s')), id],
        )
        .expect("message");
    }
    // (session, kind, topics, projects, sensitivity, confidence)
    let labels = [
        ("sA", "chat", "[\"nixos\"]", "[\"atlas\"]", "personal", "high"),
        ("sB", "code", "[\"rust\"]", "[\"atlas\"]", "shareable", "high"),
        ("sE", "chat", "[\"personal-life\"]", "[]", "personal", "high"),
        ("sF", "chat", "[]", "[\"atlas\"]", "bogus-value", "high"),
        ("sG", "chat", "[]", "[\"atlas\"]", "personal", "low"),
        ("sH", "chat", "[]", "[\"atlas\"]", "local-only", "high"),
        ("sI", "chat", "[]", "[\"atlas\"]", "personal", "high"),
    ];
    for (sid, kind, topics, projects, sens, conf) in labels {
        con.execute(
            "INSERT INTO session_labels VALUES (?1, 'claude-code', ?2, ?3, ?4, ?5, ?6, '1.0')",
            params![sid, kind, topics, projects, sens, conf],
        )
        .expect("label");
    }
    con.execute(
        "INSERT INTO generic_labels VALUES ('sH', 'issues', '[\"issue:4242\"]')",
        [],
    )
    .expect("assoc issues");
    con.execute(
        "INSERT INTO generic_labels VALUES ('sH', 'links', '[\"pr:77\"]')",
        [],
    )
    .expect("assoc links");
    // (id, message, text, content_hash, legacy sensitivity)
    let token_text = format!("the token is {FAKE_TOKEN} keep it safe");
    let chunks: [(i64, &str, &str, &str, Option<&str>); 12] = [
        (1, "mA1", "restic backup runbook steps", "h1", None),
        (2, "mA1", &token_text, "h2", None),
        (3, "mB1", "refactoring the rust parser", "h3", None),
        (4, "mB1", "shared snippet text", "h4", None),
        (5, "mC1", "random notes with no context", "h5", None),
        (7, "mD1", "shared snippet text", "h4", None),
        (8, "mD1", "already decided content", "h8", None),
        (9, "mE1", "weekend trip planning with family", "h9", None),
        (10, "mF1", "some project text", "h10", None),
        (11, "mG1", "more project text", "h11", None),
        (12, "mH1", "ops notes", "h12", Some("secret-bearing")),
        (13, "mI1", "older project notes", "h13", None),
    ];
    for (id, message, text, hash, sens) in chunks {
        con.execute(
            "INSERT INTO chunks VALUES (?1, ?2, 0, ?3, ?4, ?5)",
            params![id, message, text, hash, sens],
        )
        .expect("chunk");
        let vec: Vec<u8> = (0..16).map(|i| (id as u8) ^ i).collect();
        con.execute(
            "INSERT INTO vec_snowflake_m_v2_768_f32 VALUES (?1, ?2)",
            params![id, vec],
        )
        .expect("vector");
    }
    con.execute_batch(atlas_core::classify::RECORDS_DDL).expect("records ddl");
    insert_record(&con, &decided_record("chunk:7", "local-only"));
    insert_record(&con, &decided_record("chunk:8", "personal"));
    path
}

fn read_record(path: &PathBuf, embed_id: &str) -> Option<(String, ClassificationRecord)> {
    let con = Connection::open(path).expect("open corpus");
    let json_text: Option<String> = con
        .query_row(
            &format!("SELECT record_json FROM {RECORDS_TABLE} WHERE embed_id=?1"),
            params![embed_id],
            |r| r.get(0),
        )
        .ok();
    json_text.map(|text| {
        let record = ClassificationRecord::parse(
            serde_json::from_str(&text).expect("stored record is JSON"),
        )
        .expect("stored record parses");
        (text, record)
    })
}

fn all_records(path: &PathBuf) -> BTreeMap<String, String> {
    let con = Connection::open(path).expect("open corpus");
    let mut stmt = con
        .prepare(&format!(
            "SELECT embed_id, record_json FROM {RECORDS_TABLE} ORDER BY embed_id"
        ))
        .expect("prepare");
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .expect("query");
    rows.map(|r| {
        let (a, b) = r.expect("row");
        (a, b)
    })
    .collect()
}

fn entry<'a>(
    manifest: &'a atlas_core::classify::RunManifest,
    embed_id: &str,
) -> &'a atlas_core::classify::DecisionEntry {
    manifest
        .decisions
        .iter()
        .find(|d| d.embed_id == embed_id)
        .unwrap_or_else(|| panic!("no decision entry for {embed_id}"))
}

#[test]
fn dry_run_enumerates_only_unclassified_and_writes_nothing() {
    let path = fixture_db("dry");
    let before = all_records(&path);
    let manifest = run_classification(&path, false, &config()).expect("dry run");
    assert_eq!(manifest.mode, "dry-run");
    assert_eq!(manifest.counts.total_embeds, 12);
    assert_eq!(manifest.counts.already_classified, 2);
    assert_eq!(manifest.counts.candidates, 10);
    assert_eq!(manifest.counts.written, 0);
    assert_eq!(
        manifest.counts.by_prior_state.get("default_state"),
        Some(&9)
    );
    assert_eq!(manifest.counts.by_prior_state.get("absent"), Some(&1));
    assert_eq!(
        manifest.counts.by_outcome.get("classified"),
        Some(&4),
        "{:?}",
        manifest.counts.by_outcome
    );
    assert_eq!(
        manifest.counts.by_outcome.get("confirmed_secret"),
        Some(&1)
    );
    assert_eq!(
        manifest.counts.by_outcome.get("held_by_reuse"),
        Some(&1)
    );
    assert_eq!(
        manifest.counts.by_outcome.get("excluded_personal_life"),
        Some(&1)
    );
    assert_eq!(manifest.counts.by_outcome.get("unresolved"), Some(&3));
    // The declared window is recorded.
    assert_eq!(manifest.window.days, 90);
    assert_eq!(manifest.window.scope, "same-project");
    // Nothing was written: the records table is byte-identical.
    assert_eq!(all_records(&path), before);
}

#[test]
fn commit_classifies_the_remainder_and_preserves_classified_embeds_and_vectors() {
    let path = fixture_db("commit");
    let (json7_before, _) = read_record(&path, "chunk:7").expect("chunk 7 json");
    let (json8_before, _) = read_record(&path, "chunk:8").expect("chunk 8 json");

    let manifest = run_classification(&path, true, &config()).expect("commit run");
    assert_eq!(manifest.mode, "commit");
    assert_eq!(manifest.counts.written, 10);
    assert!(manifest.vectors_preserved);
    assert_eq!(manifest.chunks_hash_before, manifest.chunks_hash_after);
    assert_eq!(manifest.vectors_hash_before, manifest.vectors_hash_after);

    // Previously classified embeds are byte-identical afterwards.
    let (json7_after, _) = read_record(&path, "chunk:7").expect("chunk 7 after");
    let (json8_after, _) = read_record(&path, "chunk:8").expect("chunk 8 after");
    assert_eq!(json7_after, json7_before);
    assert_eq!(json8_after, json8_before);

    // Label evidence classifies; shareable is capped at personal.
    let (_, r1) = read_record(&path, "chunk:1").expect("chunk 1 record");
    assert_eq!(r1.sensitivity, Sensitivity::Personal);
    assert_eq!(r1.provenance.method, "remainder-sweep");
    assert_eq!(r1.provenance.actor, "atlas-classify/remainder-sweep");
    let (_, r3) = read_record(&path, "chunk:3").expect("chunk 3 record");
    assert_eq!(r3.sensitivity, Sensitivity::Personal);
    assert!(
        r3.provenance
            .evidence_refs
            .iter()
            .any(|e| e == "cap:shareable_requires_per_embed_review"),
        "{:?}",
        r3.provenance.evidence_refs
    );

    // The detector confirms the token-bearing embed at
    // secret-bearing despite the personal session label, and no
    // secret text reaches the manifest.
    let (_, r2) = read_record(&path, "chunk:2").expect("chunk 2 record");
    assert_eq!(r2.sensitivity, Sensitivity::SecretBearing);
    let manifest_json = serde_json::to_string(&manifest).expect("manifest json");
    assert!(
        !manifest_json.contains(FAKE_TOKEN),
        "manifest must never carry secret text"
    );

    // Reuse floor: chunk 4 shares content with decided chunk 7.
    let (_, r4) = read_record(&path, "chunk:4").expect("chunk 4 record");
    assert_eq!(r4.sensitivity, Sensitivity::LocalOnly);
    assert!(
        r4.provenance
            .evidence_refs
            .iter()
            .any(|e| e == "reuse_floor:chunk:7")
    );

    // Personal-life evidence excludes chunk 9.
    let (_, r9) = read_record(&path, "chunk:9").expect("chunk 9 record");
    assert_eq!(
        r9.disposition,
        Some(atlas_core::classification::ClassificationDisposition::ExcludedPersonalLife)
    );

    // Associations land in the evidence of chunk 12, whose own
    // legacy value floors the label: secret-bearing, decided.
    let (_, r12) = read_record(&path, "chunk:12").expect("chunk 12 record");
    assert_eq!(r12.sensitivity, Sensitivity::SecretBearing);
    for want in ["session:sH", "issue:4242", "pr:77"] {
        assert!(
            r12.provenance.evidence_refs.iter().any(|e| e == want),
            "missing evidence {want}: {:?}",
            r12.provenance.evidence_refs
        );
    }

    // Window: sD is 92 days before sA (outside), 19 days before
    // sI (inside). Chunk 1 sees 5 related sessions, not 6.
    assert_eq!(entry(&manifest, "chunk:1").related_sessions, 5);
    let e13 = entry(&manifest, "chunk:13");
    assert_eq!(e13.related_sessions, 6);
    assert_eq!(e13.related_classified_max.as_deref(), Some("local-only"));
    assert_eq!(entry(&manifest, "chunk:5").related_sessions, 0);
}

#[test]
fn missing_and_contradictory_context_stay_secret_bearing_with_reasons() {
    let path = fixture_db("unresolved");
    let manifest = run_classification(&path, true, &config()).expect("commit run");
    let cases = [
        ("chunk:5", "no_classification_context"),
        ("chunk:10", "contradictory_context"),
        ("chunk:11", "weak_evidence"),
    ];
    for (embed_id, reason) in cases {
        let e = entry(&manifest, embed_id);
        assert_eq!(e.outcome, DecisionOutcome::Unresolved, "{embed_id}");
        assert_eq!(e.unresolved_reason.as_deref(), Some(reason), "{embed_id}");
        let (_, record) = read_record(&path, embed_id).expect("unresolved record written");
        assert_eq!(record.sensitivity, Sensitivity::SecretBearing);
        assert_eq!(
            record.extras.get("unresolved_reason"),
            Some(&Json::from(reason)),
            "{embed_id}"
        );
    }
}

#[test]
fn rerun_is_idempotent_and_resume_completes_an_interrupted_run() {
    // Idempotency: a second full run writes nothing new.
    let path = fixture_db("idem");
    let first = run_classification(&path, true, &config()).expect("first run");
    assert_eq!(first.counts.written, 10);
    let after_first = all_records(&path);
    let second = run_classification(&path, true, &config()).expect("second run");
    assert_eq!(second.counts.written, 0);
    assert_eq!(second.counts.already_classified, 9);
    assert_eq!(second.counts.candidates, 3);
    assert_eq!(
        second.counts.by_prior_state.get("unresolved"),
        Some(&3)
    );
    assert_eq!(all_records(&path), after_first);

    // Resume: cap the first run at 4 decisions; the follow-up run
    // completes exactly the remainder, and the final records
    // match a single uninterrupted run (modulo the run id in
    // provenance extras).
    let interrupted = fixture_db("resume-a");
    let mut capped = config();
    capped.max_decisions = Some(4);
    let part = run_classification(&interrupted, true, &capped).expect("capped run");
    assert_eq!(part.counts.written, 4);
    let rest = run_classification(&interrupted, true, &config()).expect("resume run");
    assert_eq!(rest.counts.written, 6);

    let single = fixture_db("resume-b");
    run_classification(&single, true, &config()).expect("single run");
    let strip = |map: BTreeMap<String, String>| -> BTreeMap<String, Json> {
        map.into_iter()
            .map(|(id, text)| {
                let mut v: Json = serde_json::from_str(&text).expect("record json");
                if let Some(p) = v.get_mut("provenance") {
                    p.as_object_mut().expect("provenance").remove("run_id");
                }
                (id, v)
            })
            .collect()
    };
    assert_eq!(strip(all_records(&interrupted)), strip(all_records(&single)));
}

fn reviewer() -> atlas_core::classify::ReviewerIdentity {
    atlas_core::classify::ReviewerIdentity {
        model: "atlas-classify".to_string(),
        model_version: "0.1.0".to_string(),
        config_version: "sha256:test".to_string(),
    }
}

fn schema_v2_with_rename() -> SchemaDoc {
    SchemaDoc::parse(&json!({
        "version": 2,
        "sections": {
            "sensitivity_values": {
                "values": ["shareable", "personal", "local-only", "secret-bearing"],
                "order": "shareable < personal < local-only < secret-bearing",
                "missing_or_invalid": "secret-bearing",
                "renames": {"local-only": "personal"}
            },
            "dispositions": {"values": ["included", "excluded_personal_life"]},
            "provenance_fields": {
                "required": ["actor", "decided_at", "method", "method_version", "config_version", "evidence_refs", "run_id"],
                "optional": ["previous_decision_id"]
            },
            "record_fields": {
                "required": ["schema_version", "embed_id", "sensitivity", "provenance"],
                "optional": ["disposition"],
                "unknown_fields": "preserved in extras"
            }
        }
    }))
    .expect("v2 schema parses")
}

#[test]
fn schema_review_examines_changed_sections_only_and_records_provenance() {
    let path = fixture_db("review");
    run_classification(&path, true, &config()).expect("seed records");
    let old = SchemaDoc::parse(&serde_json::from_str(CURRENT_SCHEMA_JSON).expect("v1 json"))
        .expect("v1 schema");
    let review = review_schema_change(&path, &old, &schema_v2_with_rename(), reviewer())
        .expect("review");
    assert_eq!(review.old_version, 1);
    assert_eq!(review.new_version, 2);
    assert!(review.old_schema_hash.starts_with("sha256:"));
    assert_eq!(
        review.sections_examined,
        vec!["provenance_fields".to_string(), "sensitivity_values".to_string()]
    );
    assert_eq!(
        review.sections_skipped,
        vec!["dispositions".to_string(), "record_fields".to_string()]
    );
    assert_eq!(review.reviewer.model, "atlas-classify");
    // The local-only records (chunk 4 held by reuse, chunk 7
    // decided) are the lowering proposals.
    assert_eq!(review.lowering_proposals, 2);
    let lowering: Vec<&str> = review
        .sections
        .iter()
        .flat_map(|s| s.proposals.iter())
        .filter(|p| p.lowering)
        .map(|p| p.embed_id.as_str())
        .collect();
    assert_eq!(lowering, vec!["chunk:4", "chunk:7"]);
}

#[test]
fn schema_review_never_lowers_without_a_reviewed_disposition() {
    let path = fixture_db("apply");
    run_classification(&path, true, &config()).expect("seed records");
    let old = SchemaDoc::parse(&serde_json::from_str(CURRENT_SCHEMA_JSON).expect("v1 json"))
        .expect("v1 schema");
    let review = review_schema_change(&path, &old, &schema_v2_with_rename(), reviewer())
        .expect("review");

    // No dispositions: nothing applies, least of all a lowering.
    let none = parse_dispositions(&json!({"dispositions": {}})).expect("empty dispositions");
    let report = apply_schema_review(&path, &review, &none, NOW).expect("apply none");
    assert_eq!(report.applied, 0);
    assert!(report.entries.iter().all(|e| e.result != "applied"));
    let (_, r7) = read_record(&path, "chunk:7").expect("chunk 7");
    assert_eq!(r7.sensitivity, Sensitivity::LocalOnly);
    assert_eq!(r7.schema_version, 1);

    // A disposition for chunk:7 only: its revalidation and its
    // lowering both apply, chained; chunk:4's lowering stays
    // refused.
    let one = parse_dispositions(&json!({
        "dispositions": {
            "chunk:7": {"reviewer": "reviewer/drew", "rationale": "local-only retired in v2"}
        }
    }))
    .expect("dispositions");
    let report = apply_schema_review(&path, &review, &one, NOW).expect("apply one");
    assert_eq!(report.applied, 2, "{:?}", report.entries);
    let (_, r7) = read_record(&path, "chunk:7").expect("chunk 7 after");
    assert_eq!(r7.sensitivity, Sensitivity::Personal);
    assert_eq!(r7.schema_version, 2);
    assert_eq!(r7.provenance.method, "schema-review");
    assert_eq!(r7.provenance.actor, "reviewer/drew");
    assert!(r7.provenance.previous_decision_id.is_some());
    let (_, r4) = read_record(&path, "chunk:4").expect("chunk 4 after");
    assert_eq!(r4.sensitivity, Sensitivity::LocalOnly);
    assert_eq!(r4.schema_version, 1);
}

#[test]
fn schema_review_breaking_event_examines_all_sections() {
    let path = fixture_db("breaking");
    run_classification(&path, true, &config()).expect("seed records");
    let old = SchemaDoc::parse(&serde_json::from_str(CURRENT_SCHEMA_JSON).expect("v1 json"))
        .expect("v1 schema");
    let mut new_doc: Json = serde_json::from_str(CURRENT_SCHEMA_JSON).expect("v1 json");
    new_doc["version"] = Json::from(2);
    new_doc["breaking_change_event"] = Json::from(true);
    let new = SchemaDoc::parse(&new_doc).expect("v2 schema");
    let review = review_schema_change(&path, &old, &new, reviewer()).expect("review");
    assert!(review.breaking_change_event);
    assert_eq!(review.sections_examined.len(), 4);
    assert!(review.sections_skipped.is_empty());
    // Every section revalidates every v1 record (12 records).
    assert_eq!(review.proposals, 48);
    assert_eq!(review.lowering_proposals, 0);
}

#[test]
fn removed_sensitivity_value_fails_closed_to_the_locked_default() {
    let path = fixture_db("removed");
    run_classification(&path, true, &config()).expect("seed records");
    let old = SchemaDoc::parse(&serde_json::from_str(CURRENT_SCHEMA_JSON).expect("v1 json"))
        .expect("v1 schema");
    let mut v2: Json = serde_json::from_str(CURRENT_SCHEMA_JSON).expect("v1 json");
    v2["version"] = Json::from(2);
    v2["sections"]["sensitivity_values"]["values"] =
        json!(["shareable", "personal", "secret-bearing"]);
    let new = SchemaDoc::parse(&v2).expect("v2 schema");
    let review = review_schema_change(&path, &old, &new, reviewer()).expect("review");
    let proposals: Vec<_> = review
        .sections
        .iter()
        .flat_map(|s| s.proposals.iter())
        .filter(|p| p.section == "sensitivity_values" && p.action == "change")
        .collect();
    assert_eq!(proposals.len(), 2);
    assert!(proposals
        .iter()
        .all(|p| p.proposed_sensitivity == "secret-bearing" && !p.lowering));
}

mod decide_properties {
    use super::*;
    use proptest::prelude::*;

    fn context_strategy() -> impl Strategy<Value = CandidateContext> {
        let sens = prop_oneof![
            Just(Option::<String>::None),
            Just(Some("shareable".to_string())),
            Just(Some("personal".to_string())),
            Just(Some("local-only".to_string())),
            Just(Some("secret-bearing".to_string())),
            Just(Some("bogus".to_string())),
        ];
        let floor = prop_oneof![
            Just(None),
            Just(Some(Sensitivity::Shareable)),
            Just(Some(Sensitivity::Personal)),
            Just(Some(Sensitivity::LocalOnly)),
            Just(Some(Sensitivity::SecretBearing)),
        ];
        (sens.clone(), sens, floor, any::<bool>(), any::<bool>())
            .prop_map(|(label, chunk, floor, detector, personal_life)| {
                CandidateContext {
                    embed_id: "chunk:1".to_string(),
                    chunk_legacy_sensitivity: chunk,
                    label_sensitivity: label,
                    label_confidence: Some("high".to_string()),
                    kind: personal_life.then(|| "personal-life".to_string()),
                    detector_hits: if detector {
                        vec!["github-token"]
                    } else {
                        vec![]
                    },
                    reuse_floor: floor,
                    reuse_floor_ref: floor.map(|_| "chunk:9".to_string()),
                    ..CandidateContext::default()
                }
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// Oracles: design §8 guarantees 2, 4, 6 — a detector hit
        /// is decisive; the result never drops below the reuse
        /// floor; `shareable` is never produced; evidence-free
        /// candidates stay unresolved at the locked default.
        #[test]
        fn decide_never_lowers_below_floor_and_never_produces_shareable(
            ctx in context_strategy(),
        ) {
            let decision = decide(&ctx);
            prop_assert_ne!(decision.sensitivity, Sensitivity::Shareable);
            if let Some(floor) = ctx.reuse_floor {
                prop_assert!(decision.sensitivity >= floor);
            }
            if !ctx.detector_hits.is_empty() {
                prop_assert_eq!(decision.sensitivity, Sensitivity::SecretBearing);
                prop_assert_eq!(decision.outcome, DecisionOutcome::ConfirmedSecret);
            }
            if ctx.detector_hits.is_empty()
                && ctx.kind.is_none()
                && ctx.label_sensitivity.is_none()
                && ctx.chunk_legacy_sensitivity.is_none()
            {
                prop_assert_eq!(decision.outcome, DecisionOutcome::Unresolved);
                prop_assert_eq!(
                    decision.unresolved_reason,
                    Some(atlas_core::classify::REASON_NO_CONTEXT),
                );
            }
        }
    }
}
