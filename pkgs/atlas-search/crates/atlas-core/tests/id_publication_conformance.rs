//! Bounded audit of the real unsigned version and lineage projections (#11186).
//! Oracle: fixture-derived row facts, never another version projector invocation.
//! This suite checks pinned attestation replay (§6.1), complete publication-unit
//! readiness (§6.4), correction clears (§6.2), and retirement tombstones (§6.3).
//! Atlas-core exposes unsigned descriptors; event signing/binding remains the
//! responsibility of the later publication adapter.

use atlas_core::attestation::{
    Batch, BatchOutcome, BatchPredicate, BatchStatement, Builder, BuilderVersion, Classification,
    DigestSet, MaterialEntry, SubjectAnnotations, SubjectEntry,
};
use atlas_core::classify::sha256_hex;
use atlas_core::id_projection::{
    PROJECTION_VERSION as ID_PROJECTION_VERSION, ProjectionRunPins, project_run_with_pins,
    replay_run_from_pins,
};
use atlas_core::shards::Tier;
use atlas_core::version_projection::{
    AtlasCorrection, AtlasFileNoteBinding, AtlasRevisionEdge, AtlasTombstone, AtlasVersionKey,
    AtlasVersionRow, PROJECTION_VERSION, ProjectedCorrection, ProjectedDocumentRef,
    ProjectedLineageClaim, ProjectedLineageRef, ProjectedLineageValue, ProjectedNote,
    ProjectedNoteEventRef, ProjectedSelectedVersion, ProjectedSubject, ProjectedTag,
    ProjectedTagClear, ProjectedTombstone, ProjectedTombstoneClaim, ProjectedVersion,
    project_corrections, project_lineage_claims, project_selected_versions, project_tombstone,
};
use proptest::prelude::*;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/id_projection_rows.json"
));

struct Fixture {
    values: Vec<Value>,
    rows: Vec<AtlasVersionRow>,
    heads: Vec<AtlasVersionKey>,
    bindings: Vec<AtlasFileNoteBinding>,
    edges: Vec<AtlasRevisionEdge>,
    corrections: Vec<AtlasCorrection>,
    tombstones: Vec<AtlasTombstone>,
}

fn fixture() -> Fixture {
    let values: Vec<Value> = serde_json::from_str(FIXTURE).unwrap();
    let rows = values
        .iter()
        .cloned()
        .map(serde_json::from_value)
        .collect::<Result<_, _>>()
        .unwrap();
    let heads = values
        .iter()
        .filter(|row| row["head_transition"] == true)
        .map(|row| serde_json::from_value(row.clone()).unwrap())
        .collect();
    let bindings = values
        .iter()
        .filter_map(|row| row.get("file_note_binding"))
        .map(|binding| serde_json::from_value(binding.clone()).unwrap())
        .collect();
    let edges = values
        .iter()
        .filter_map(|row| row.get("revision_edges").and_then(Value::as_array))
        .flatten()
        .cloned()
        .map(serde_json::from_value)
        .collect::<Result<_, _>>()
        .unwrap();
    let corrections = values
        .iter()
        .filter_map(|row| row.get("corrections").and_then(Value::as_array))
        .flatten()
        .cloned()
        .map(serde_json::from_value)
        .collect::<Result<_, _>>()
        .unwrap();
    let tombstones = values
        .iter()
        .filter_map(|row| row.get("tombstone"))
        .cloned()
        .map(serde_json::from_value)
        .collect::<Result<_, _>>()
        .unwrap();
    Fixture {
        values,
        rows,
        heads,
        bindings,
        edges,
        corrections,
        tombstones,
    }
}

#[derive(Debug)]
struct AuditReport {
    failures: Vec<String>,
    checked: Vec<&'static str>,
}

impl AuditReport {
    fn has(&self, property: &str) -> bool {
        self.failures.iter().any(|failure| failure == property)
    }

    fn summary(&self) -> String {
        let verdicts = self
            .checked
            .iter()
            .map(|property| {
                let status = if self.has(property) { "FAIL" } else { "PASS" };
                format!("{property}={status}")
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("{verdicts}")
    }
}

fn expected_hash(row: &AtlasVersionRow) -> String {
    blake3::hash(row.content.as_bytes()).to_hex().to_string()
}

fn expected_tags(row: &AtlasVersionRow) -> Vec<ProjectedTag> {
    let version = serde_json::json!({
        "file_id": row.file_id, "version_seq": row.version_seq,
        "path": row.path, "source_locator": row.source_locator,
        "ingested_at": row.ingested_at, "batch_id": row.batch_id,
        "projection_version": PROJECTION_VERSION,
    });
    let generated = serde_json::json!({
        "batch_id": row.batch_id, "file_id": row.file_id,
        "version_seq": row.version_seq, "projection_version": PROJECTION_VERSION,
    });
    vec![
        ProjectedTag {
            kind: "tag.set".into(),
            subject: atlas_core::version_projection::ProjectedSubject::DocumentPut,
            name: "atlas.version".into(),
            value: serde_json::to_string(&version).unwrap(),
        },
        ProjectedTag {
            kind: "tag.set".into(),
            subject: atlas_core::version_projection::ProjectedSubject::DocumentPut,
            name: "atlas.wasGeneratedBy".into(),
            value: serde_json::to_string(&generated).unwrap(),
        },
    ]
}

fn expected_document(row: &AtlasVersionRow) -> ProjectedVersion {
    let content = row.content.as_bytes().to_vec();
    ProjectedVersion {
        kind: "document.put".into(),
        content_hash: expected_hash(row),
        media_type: row.media_type.clone(),
        size: content.len() as u64,
        content,
        projection_version: PROJECTION_VERSION.into(),
        claims: expected_tags(row),
    }
}

fn expected_lineage(f: &Fixture) -> Vec<ProjectedLineageClaim> {
    let mut claims = Vec::new();
    let mut bindings = f.bindings.iter().collect::<Vec<_>>();
    bindings.sort_by(|a, b| a.file_id.cmp(&b.file_id));
    for binding in bindings {
        claims.push(ProjectedLineageClaim {
            subject: ProjectedLineageRef::NoteCreate(binding.note_create.clone()),
            name: "atlas.fileId".into(),
            value: ProjectedLineageValue::File {
                file_id: binding.file_id.clone(),
                source_anchor: binding.source_anchor.clone(),
            },
        });
    }
    let mut keys = f
        .rows
        .iter()
        .map(|r| AtlasVersionKey {
            file_id: r.file_id.clone(),
            version_seq: r.version_seq,
        })
        .collect::<Vec<_>>();
    keys.sort();
    for key in keys {
        let binding = f
            .bindings
            .iter()
            .find(|b| b.file_id == key.file_id)
            .unwrap();
        claims.push(ProjectedLineageClaim {
            subject: ProjectedLineageRef::DocumentPut(key),
            name: "atlas.specializationOf".into(),
            value: ProjectedLineageValue::Specialization {
                note: ProjectedLineageRef::NoteCreate(binding.note_create.clone()),
            },
        });
    }
    let mut edges = f.edges.iter().collect::<Vec<_>>();
    edges.sort_by(|a, b| {
        (&a.newer, &a.prior, &a.edge_kind).cmp(&(&b.newer, &b.prior, &b.edge_kind))
    });
    for edge in edges {
        claims.push(ProjectedLineageClaim {
            subject: ProjectedLineageRef::DocumentPut(edge.newer.clone()),
            name: "atlas.wasRevisionOf".into(),
            value: ProjectedLineageValue::Revision {
                document: ProjectedLineageRef::DocumentPut(edge.prior.clone()),
                edge_kind: edge.edge_kind.clone(),
                confidence: edge.confidence,
            },
        });
    }
    claims
}

fn expected_notes(f: &Fixture) -> BTreeMap<AtlasVersionKey, ProjectedNote> {
    let mut heads = f.heads.clone();
    heads.sort();
    let rows = f
        .rows
        .iter()
        .map(|r| {
            (
                AtlasVersionKey {
                    file_id: r.file_id.clone(),
                    version_seq: r.version_seq,
                },
                r,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut first = BTreeMap::<String, ProjectedNoteEventRef>::new();
    let mut previous = BTreeMap::<String, ProjectedNoteEventRef>::new();
    let mut notes = BTreeMap::new();
    for key in heads {
        let row = rows.get(&key).unwrap();
        let reference = ProjectedNoteEventRef {
            file_id: key.file_id.clone(),
            version_seq: key.version_seq,
        };
        let content = atlas_core::version_projection::ProjectedDocumentRef {
            content: expected_hash(row),
            media_type: row.media_type.clone(),
            size: row.content.len() as u64,
        };
        let note = if let Some(create) = first.get(&key.file_id) {
            let parent = previous.get(&key.file_id).unwrap().clone();
            previous.insert(key.file_id.clone(), reference.clone());
            ProjectedNote::Revise {
                note: create.clone(),
                content,
                parents: vec![parent],
            }
        } else {
            first.insert(key.file_id.clone(), reference.clone());
            previous.insert(key.file_id.clone(), reference);
            ProjectedNote::Create { content }
        };
        notes.insert(key, note);
    }
    notes
}

fn expected_corrections(f: &Fixture) -> Vec<ProjectedCorrection> {
    f.corrections
        .iter()
        .map(|correction| {
            let edge = &correction.replacement;
            let mut clear_ids = correction.superseded_claim_ids.clone();
            clear_ids.sort();
            ProjectedCorrection {
                replacement: ProjectedLineageClaim {
                    subject: ProjectedLineageRef::DocumentPut(edge.newer.clone()),
                    name: "atlas.wasRevisionOf".into(),
                    value: ProjectedLineageValue::Revision {
                        document: ProjectedLineageRef::DocumentPut(edge.prior.clone()),
                        edge_kind: edge.edge_kind.clone(),
                        confidence: edge.confidence,
                    },
                },
                clear: ProjectedTagClear {
                    kind: "tag.clear".into(),
                    claims: clear_ids,
                },
            }
        })
        .collect()
}

fn expected_tombstone(tombstone: &AtlasTombstone) -> ProjectedTombstone {
    let marker_content = serde_json::to_vec(&serde_json::json!({
        "file_id": tombstone.file_id,
        "version_seq": tombstone.prior_head.version_seq,
    }))
    .unwrap();
    let marker_hash = blake3::hash(&marker_content).to_hex().to_string();
    let marker = ProjectedVersion {
        kind: "document.put".into(),
        content_hash: marker_hash.clone(),
        media_type: "text/atlas-tombstone".into(),
        size: marker_content.len() as u64,
        content: marker_content,
        projection_version: PROJECTION_VERSION.into(),
        claims: vec![],
    };
    let tombstone_claim = serde_json::to_string(&serde_json::json!({
        "file_id": tombstone.file_id,
        "version_seq": tombstone.prior_head.version_seq,
        "trigger": tombstone.trigger,
        "rule": tombstone.rule,
        "run_id": tombstone.run_id,
        "projection_version": PROJECTION_VERSION,
    }))
    .unwrap();
    ProjectedTombstone {
        note: ProjectedNote::Revise {
            note: ProjectedNoteEventRef {
                file_id: tombstone.note_create.file_id.clone(),
                version_seq: tombstone.note_create.version_seq,
            },
            content: ProjectedDocumentRef {
                content: marker_hash,
                media_type: "text/atlas-tombstone".into(),
                size: marker.size,
            },
            parents: vec![ProjectedNoteEventRef {
                file_id: tombstone.prior_head.file_id.clone(),
                version_seq: tombstone.prior_head.version_seq,
            }],
        },
        marker,
        claim: ProjectedTombstoneClaim {
            kind: "tag.set".into(),
            subject: ProjectedLineageRef::DocumentPut(tombstone.prior_head.clone()),
            name: "atlas.tombstone".into(),
            value: tombstone_claim,
        },
    }
}

fn canonical<T: serde::Serialize>(items: &[T]) -> Vec<String> {
    let mut values = items
        .iter()
        .map(|item| serde_json::to_string(item).unwrap())
        .collect::<Vec<_>>();
    values.sort();
    values
}

struct ActualProjection {
    selected: Vec<ProjectedSelectedVersion>,
    claims: Vec<ProjectedLineageClaim>,
    corrections: Vec<ProjectedCorrection>,
    tombstones: Vec<ProjectedTombstone>,
}

fn project_fixture(f: &Fixture) -> ActualProjection {
    ActualProjection {
        selected: project_selected_versions(&f.rows, &f.heads).unwrap(),
        claims: project_lineage_claims(&f.rows, &f.bindings, &f.edges).unwrap(),
        corrections: project_corrections(&f.rows, &f.corrections).unwrap(),
        tombstones: f
            .tombstones
            .iter()
            .map(project_tombstone)
            .collect::<Result<_, _>>()
            .unwrap(),
    }
}

fn fixture_snapshot_marker(f: &Fixture) -> BTreeMap<String, u64> {
    let mut marker = BTreeMap::new();
    for row in &f.rows {
        marker
            .entry(row.file_id.clone())
            .and_modify(|max_seq: &mut u64| *max_seq = (*max_seq).max(row.version_seq))
            .or_insert(row.version_seq);
    }
    marker
}

fn fixture_batch_statements(f: &Fixture) -> Vec<BatchStatement> {
    let mut batches = BTreeMap::<String, Vec<&AtlasVersionRow>>::new();
    for row in &f.rows {
        batches.entry(row.batch_id.clone()).or_default().push(row);
    }

    let mut prior_attestation = None;
    let mut statements = Vec::with_capacity(batches.len());
    for (batch_id, rows) in batches {
        let subject = rows
            .iter()
            .map(|row| SubjectEntry {
                uri: format!("atlas://{}/{}", row.file_id, row.version_seq),
                digest: DigestSet::new(&sha256_hex(row.content.as_bytes())).unwrap(),
                annotations: SubjectAnnotations {
                    spans: vec![format!("version_seq={}", row.version_seq)],
                },
            })
            .collect();
        let materials = rows
            .iter()
            .map(|row| MaterialEntry {
                uri: row.source_locator.clone(),
                digest: DigestSet::new(&sha256_hex(row.content.as_bytes())).unwrap(),
            })
            .collect();
        let statement = BatchStatement::new(
            subject,
            materials,
            BatchPredicate {
                builder: Builder {
                    id: "atlas-fixture-ingester".into(),
                    version: BuilderVersion {
                        ingester_version: "fixture-v1".into(),
                        code_rev: "0123456789abcdef0123456789abcdef01234567".into(),
                    },
                },
                batch: Batch {
                    batch_id: batch_id.clone(),
                    staging_unit_id: format!("staging-{batch_id}"),
                    gate_run_id: format!("gate-{batch_id}"),
                    started_at: "2026-11-24T00:00:00Z".into(),
                    finished_at: "2026-11-24T00:01:00Z".into(),
                    outcome: BatchOutcome::Published,
                    dedup_dropped: Vec::new(),
                },
                classification: Classification {
                    tier: Tier::Public,
                    scanner_name: "atlas-fixture-scanner".into(),
                    scanner_version: "fixture-v1".into(),
                    scan_verdict: "pass".into(),
                    scan_evidence: sha256_hex(batch_id.as_bytes()),
                },
                prior_attestation: prior_attestation.clone(),
            },
        );
        prior_attestation = Some(statement.envelope_digest().unwrap());
        statements.push(statement);
    }
    statements
}

/// Build fixture-derived batch claims under explicit pins, then replay the
/// persisted log and require byte-identical claims. The corrupted-input
/// negative control must fail closed rather than accept a prefix.
fn replay_fixture_batches(f: &Fixture) -> Result<BTreeSet<String>, String> {
    let statements = fixture_batch_statements(f);
    let pins = ProjectionRunPins {
        atlas_snapshot_marker: fixture_snapshot_marker(f),
        batch_chain_head: None,
        publisher_key_id: "fixture-publisher-key".into(),
        projection_version: ID_PROJECTION_VERSION.into(),
    };
    let log = project_run_with_pins("id-publication-fixture-run", &statements, pins.clone())
        .map_err(|error| error.to_string())?;
    if log.pins != pins {
        return Err("run log did not preserve the fixture input pins".into());
    }
    let expected_batches = f
        .rows
        .iter()
        .map(|row| row.batch_id.clone())
        .collect::<BTreeSet<_>>();
    let logged_batches = log
        .claims
        .iter()
        .map(|claim| claim.batch_id.clone())
        .collect::<BTreeSet<_>>();
    if logged_batches != expected_batches {
        return Err("run log batch set differs from Atlas fixture rows".into());
    }
    let replay = replay_run_from_pins(&log, &statements).map_err(|error| error.to_string())?;
    if replay.claims_replayed != log.claims.len() || replay.batch_chain_head != log.batch_chain_head
    {
        return Err("pinned replay did not reproduce the complete logged run".into());
    }
    let mut changed_inputs = statements.clone();
    changed_inputs[0]
        .predicate
        .batch
        .batch_id
        .push_str("-tampered");
    if replay_run_from_pins(&log, &changed_inputs).is_ok() {
        return Err("pinned replay accepted changed Atlas batch input".into());
    }
    Ok(logged_batches)
}

fn audit(
    f: &Fixture,
    selected: &[ProjectedSelectedVersion],
    claims: &[ProjectedLineageClaim],
    corrections: &[ProjectedCorrection],
    tombstones: &[ProjectedTombstone],
) -> AuditReport {
    let mut failures = BTreeSet::new();
    let checked: Vec<&'static str> = vec![
        "version.key-set",
        "version.content",
        "version.hash",
        "version.metadata",
        "version.claims",
        "lineage.explicit-facts",
        "note.selection-and-chain",
        "correction.exact-clear",
        "retirement.remove-tombstone",
        "replay.pinned-inputs",
        "publication.complete-unit",
    ];
    let mut expected = BTreeMap::new();
    for row in &f.rows {
        expected.insert(
            AtlasVersionKey {
                file_id: row.file_id.clone(),
                version_seq: row.version_seq,
            },
            row,
        );
    }
    let actual_keys = selected.iter().map(|v| v.key.clone()).collect::<Vec<_>>();
    let unique = actual_keys.iter().cloned().collect::<BTreeSet<_>>();
    if actual_keys.len() != expected.len()
        || unique.len() != actual_keys.len()
        || unique != expected.keys().cloned().collect()
    {
        failures.insert("version.key-set".to_owned());
        failures.insert("publication.complete-unit".to_owned());
    }
    let replayed_batches = match replay_fixture_batches(f) {
        Ok(batches) => batches,
        Err(_) => {
            failures.insert("replay.pinned-inputs".to_owned());
            BTreeSet::new()
        }
    };
    let expected_note_map = expected_notes(f);
    for item in selected {
        let Some(row) = expected.get(&item.key) else {
            failures.insert("version.key-set".into());
            continue;
        };
        let document = expected_document(row);
        if item.document.content != document.content {
            failures.insert("version.content".into());
        }
        if item.document.content_hash != document.content_hash {
            failures.insert("version.hash".into());
        }
        if item.document.kind != document.kind
            || item.document.media_type != document.media_type
            || item.document.size != document.size
            || item.document.projection_version != document.projection_version
        {
            failures.insert("version.metadata".into());
        }
        if item.document.claims != document.claims {
            failures.insert("version.claims".into());
        }
        if item.note != expected_note_map.get(&item.key).cloned() {
            failures.insert("note.selection-and-chain".into());
        }

        let document_subject = ProjectedLineageRef::DocumentPut(item.key.clone());
        let expected_note_ref = f
            .bindings
            .iter()
            .find(|binding| binding.file_id == item.key.file_id)
            .map(|binding| ProjectedLineageRef::NoteCreate(binding.note_create.clone()));
        let generated_by_matches = item.document.claims.iter().any(|tag| {
            if tag.kind != "tag.set"
                || tag.subject != ProjectedSubject::DocumentPut
                || tag.name != "atlas.wasGeneratedBy"
            {
                return false;
            }
            let Ok(value) = serde_json::from_str::<Value>(&tag.value) else {
                return false;
            };
            value["batch_id"].as_str() == Some(row.batch_id.as_str())
                && value["file_id"].as_str() == Some(row.file_id.as_str())
                && value["version_seq"].as_u64() == Some(row.version_seq)
                && value["projection_version"].as_str() == Some(PROJECTION_VERSION)
                && replayed_batches.contains(&row.batch_id)
        });
        let specialization_matches = claims.iter().any(|claim| {
            claim.subject == document_subject
                && claim.name == "atlas.specializationOf"
                && matches!(
                    &claim.value,
                    ProjectedLineageValue::Specialization { note }
                        if Some(note) == expected_note_ref.as_ref()
                )
        });
        let lineage_values_are_signer_ready = claims
            .iter()
            .filter(|claim| claim.subject == document_subject)
            .all(|claim| claim.value_json(|_| Some("0".repeat(64))).is_ok());
        let expected_note_matches = item.note == expected_note_map.get(&item.key).cloned();
        if item.document.kind != "document.put"
            || !generated_by_matches
            || !specialization_matches
            || !lineage_values_are_signer_ready
            || !expected_note_matches
        {
            failures.insert("publication.complete-unit".into());
        }
    }
    if expected_note_map
        .keys()
        .any(|key| !actual_keys.contains(key))
    {
        failures.insert("note.selection-and-chain".into());
    }
    let actual_notes = selected
        .iter()
        .filter(|item| item.note.is_some())
        .map(|item| item.key.clone())
        .collect::<BTreeSet<_>>();
    if actual_notes != expected_note_map.keys().cloned().collect() {
        failures.insert("note.selection-and-chain".into());
    }
    let expected_claims = expected_lineage(f);
    if claims.len() != expected_claims.len() || canonical(claims) != canonical(&expected_claims) {
        failures.insert("lineage.explicit-facts".into());
    }
    let expected_correction_facts = expected_corrections(f);
    if corrections.len() != expected_correction_facts.len()
        || canonical(corrections) != canonical(&expected_correction_facts)
    {
        failures.insert("correction.exact-clear".into());
    }
    let expected_tombstones = f
        .tombstones
        .iter()
        .map(expected_tombstone)
        .collect::<Vec<_>>();
    if tombstones.len() != expected_tombstones.len()
        || canonical(tombstones) != canonical(&expected_tombstones)
    {
        failures.insert("retirement.remove-tombstone".into());
    }
    AuditReport {
        failures: failures.into_iter().collect::<Vec<String>>(),
        checked,
    }
}

fn audit_actual(f: &Fixture, actual: &ActualProjection) -> AuditReport {
    audit(
        f,
        &actual.selected,
        &actual.claims,
        &actual.corrections,
        &actual.tombstones,
    )
}

#[test]
fn audit_rejects_corrupted_real_version_content() {
    let f = fixture();
    let mut actual = project_fixture(&f);
    actual.selected[0].document.content[0] ^= 1;
    let report = audit_actual(&f, &actual);
    assert!(
        report.has("version.content"),
        "oracle: recomputed fixture bytes; {}",
        report.summary()
    );
}

#[test]
fn real_projection_matches_fixture_oracle_and_permutations() {
    let f = fixture();
    let actual = project_fixture(&f);
    let report = audit_actual(&f, &actual);
    println!("{}", report.summary());
    assert!(
        report.failures.is_empty(),
        "fixture oracle mismatch: {}",
        report.summary()
    );
    assert!(report.summary().contains("replay.pinned-inputs=PASS"));
    assert!(report.summary().contains("publication.complete-unit=PASS"));

    let mut reversed = Fixture {
        values: f.values.clone(),
        rows: f.rows.clone(),
        heads: f.heads.clone(),
        bindings: f.bindings.clone(),
        edges: f.edges.clone(),
        corrections: f.corrections.clone(),
        tombstones: f.tombstones.clone(),
    };
    reversed.rows.reverse();
    reversed.heads.reverse();
    reversed.bindings.reverse();
    reversed.edges.reverse();
    reversed.corrections.reverse();
    reversed.tombstones.reverse();
    let permuted = project_fixture(&reversed);
    assert_eq!(
        permuted.selected, actual.selected,
        "oracle: real projection is input-order invariant"
    );
    assert_eq!(
        permuted.claims, actual.claims,
        "oracle: real lineage projection is input-order invariant"
    );
    assert_eq!(
        permuted.corrections, actual.corrections,
        "oracle: correction facts and exact clear IDs are input-order invariant"
    );
    assert_eq!(
        permuted.tombstones, actual.tombstones,
        "oracle: explicit tombstone keys are input-order invariant"
    );
    let permuted_report = audit_actual(&reversed, &permuted);
    assert!(
        permuted_report.failures.is_empty(),
        "permutation oracle: fixture-derived facts and key ordering: {}",
        permuted_report.summary()
    );
}

proptest! {
    #[test]
    fn fixture_oracle_survives_random_permutations_and_detects_content_tampering(
        ranks in prop::array::uniform4(prop::collection::vec(any::<u64>(), 7)),
        output_index in 0usize..7,
    ) {
        // Oracle: fixture-derived bytes, explicit lineage facts and head flags.
        // Random ranks permute each input independently; index breaks rank ties.
        fn permute<T>(items: &mut Vec<T>, ranks: &[u64]) {
            let mut indexed = items.drain(..).enumerate().collect::<Vec<_>>();
            indexed.sort_by_key(|(index, _)| (ranks[*index], *index));
            *items = indexed.into_iter().map(|(_, item)| item).collect();
        }
        let mut f = fixture();
        permute(&mut f.rows, &ranks[0]);
        permute(&mut f.heads, &ranks[1]);
        permute(&mut f.bindings, &ranks[2]);
        permute(&mut f.edges, &ranks[3]);
        let mut actual = project_fixture(&f);
        let report = audit_actual(&f, &actual);
        prop_assert!(report.failures.is_empty(), "fixture oracle: {}", report.summary());

        // Negative-control oracle: XOR changes a real nonempty fixture document byte.
        actual.selected[output_index].document.content[0] ^= 1;
        let report = audit_actual(&f, &actual);
        prop_assert!(report.has("version.content"), "byte equality oracle: {}", report.summary());
    }
}

#[test]
fn audit_reports_named_failures_for_missing_and_duplicate_versions() {
    let f = fixture();
    let actual = project_fixture(&f);
    let mut missing = actual.selected.clone();
    missing.pop();
    assert!(
        audit(
            &f,
            &missing,
            &actual.claims,
            &actual.corrections,
            &actual.tombstones
        )
        .has("version.key-set"),
        "oracle: exact fixture key set"
    );
    let mut duplicate = actual.selected.clone();
    duplicate.push(actual.selected[0].clone());
    assert!(
        audit(
            &f,
            &duplicate,
            &actual.claims,
            &actual.corrections,
            &actual.tombstones
        )
        .has("version.key-set"),
        "oracle: unique fixture key set"
    );
}

#[test]
fn audit_reports_hash_claim_lineage_and_note_tampering() {
    let f = fixture();
    let actual = project_fixture(&f);
    let mut hash = actual.selected.clone();
    hash[0].document.content_hash.push('0');
    assert!(
        audit(
            &f,
            &hash,
            &actual.claims,
            &actual.corrections,
            &actual.tombstones
        )
        .has("version.hash"),
        "oracle: BLAKE3 over fixture bytes"
    );
    let mut claim_field = actual.selected.clone();
    claim_field[0].document.claims[0].name = "tampered".into();
    assert!(
        audit(
            &f,
            &claim_field,
            &actual.claims,
            &actual.corrections,
            &actual.tombstones
        )
        .has("version.claims"),
        "oracle: fixture-derived version claim fields"
    );
    let mut no_lineage = actual.claims.clone();
    no_lineage.pop();
    assert!(
        audit(
            &f,
            &actual.selected,
            &no_lineage,
            &actual.corrections,
            &actual.tombstones
        )
        .has("lineage.explicit-facts"),
        "oracle: exact explicit binding/specialization/revision facts"
    );
    let mut duplicate_lineage = actual.claims.clone();
    duplicate_lineage.push(actual.claims[0].clone());
    assert!(
        audit(
            &f,
            &actual.selected,
            &duplicate_lineage,
            &actual.corrections,
            &actual.tombstones
        )
        .has("lineage.explicit-facts"),
        "oracle: exact lineage multiplicity"
    );
    let mut note = actual.selected.clone();
    note[0].note = Some(ProjectedNote::Create {
        content: atlas_core::version_projection::ProjectedDocumentRef {
            content: "altered".into(),
            media_type: "text/plain".into(),
            size: 7,
        },
    });
    assert!(
        audit(
            &f,
            &note,
            &actual.claims,
            &actual.corrections,
            &actual.tombstones
        )
        .has("note.selection-and-chain"),
        "oracle: head flags and selected-version content/ref chain"
    );
}

#[test]
fn audit_reports_pinned_replay_and_complete_unit_properties() {
    // Oracle: the fixture audit advertises both remaining design properties
    // independently, rather than silently treating its already-implemented
    // correction/retirement checks as the whole conformance suite.
    let f = fixture();
    let actual = project_fixture(&f);
    let report = audit_actual(&f, &actual);
    let summary = report.summary();
    assert!(
        summary.contains("replay.pinned-inputs=PASS"),
        "the report includes pinned replay status: {summary}"
    );
    assert!(
        summary.contains("publication.complete-unit=PASS"),
        "the report includes complete-unit status: {summary}"
    );
}

#[test]
fn audit_reports_incomplete_publication_units() {
    // Oracle: removing the fixture-derived batch link from one version makes
    // that version's document/relation group incomplete even if other rows pass.
    let f = fixture();
    let mut actual = project_fixture(&f);
    actual.selected[0]
        .document
        .claims
        .retain(|claim| claim.name != "atlas.wasGeneratedBy");
    let report = audit_actual(&f, &actual);
    assert!(
        report.has("publication.complete-unit"),
        "missing unit component is reported by name: {}",
        report.summary()
    );
}

#[test]
fn audit_reports_exact_correction_clears_and_retirement_marker() {
    let f = fixture();
    let mut actual = project_fixture(&f);
    assert!(audit_actual(&f, &actual).failures.is_empty());

    // Oracle: correction IDs come from the explicit superseded_claim_ids in
    // the Atlas fixture, not from claim-name/value matching.
    actual.corrections[0].clear.claims.pop();
    assert!(audit_actual(&f, &actual).has("correction.exact-clear"));

    // Oracle: the retirement marker bytes are derived from the explicit
    // prior-head key, and its digest must match those bytes.
    actual = project_fixture(&f);
    actual.tombstones[0].marker.content[0] ^= 1;
    assert!(audit_actual(&f, &actual).has("retirement.remove-tombstone"));
}
