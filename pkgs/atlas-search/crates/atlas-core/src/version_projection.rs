//! Deterministic Atlas immutable-version projection primitives (#11182).
//!
//! Projects immutable rows to `document.put` and version/batch claims, with a
//! separate complete-head-history replay that adds `note.create`/`note.revise`
//! descriptors and explicit selection that leaves non-head backfills document-only.
//! Explicit row-key lineage claims remain symbolic until signing.
//! Runtime publication, tombstones, and correction clears
//! remain later slices of the #6062 follow-on.

use std::collections::{btree_map::Entry, BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use unicode_normalization::is_nfc;

use crate::error::{Error, Result};

/// The ID event kind for an immutable content document.
pub const DOCUMENT_PUT_KIND: &str = "document.put";
/// The fixed projection version for this mapping.
pub const PROJECTION_VERSION: &str = "v1";

/// The Atlas fields needed to deterministically publish one immutable version.
///
/// `content` is the exact byte sequence Atlas stored (represented as UTF-8 in
/// this fixture-facing slice); `content_hash` is deliberately derived by the
/// projector rather than trusted from a caller.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AtlasVersionRow {
    pub file_id: String,
    pub version_seq: u64,
    pub media_type: String,
    pub content: String,
    pub path: String,
    pub source_locator: String,
    pub ingested_at: String,
    pub batch_id: String,
}

/// The subject binding for an unsigned tag in a publication unit.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum ProjectedSubject {
    /// Bind to the `document.put` event ID after that event is signed.
    DocumentPut,
}

/// An unsigned `tag.set` payload. The actual event ID is assigned when signed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProjectedTag {
    pub kind: String,
    /// The subject is symbolic until the corresponding `document.put` is signed.
    pub subject: ProjectedSubject,
    pub name: String,
    /// JSON text, as required for Atlas claims by the publication design.
    pub value: String,
}

/// One deterministic, unsigned publication unit for an Atlas version.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProjectedVersion {
    pub kind: String,
    pub content_hash: String,
    pub media_type: String,
    pub size: u64,
    pub content: Vec<u8>,
    pub projection_version: String,
    pub claims: Vec<ProjectedTag>,
}

/// Symbolic identity of a note event, resolved to its signed ID by a publisher.
/// The create event is both the stable note identity and its first revision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProjectedNoteEventRef {
    pub file_id: String,
    pub version_seq: u64,
}

/// ID's blob reference shape; this is not a `document.put` event reference.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProjectedDocumentRef {
    /// BLAKE3 blob hash (ID's `DocumentRef.content`, not an event ID).
    pub content: String,
    pub media_type: String,
    pub size: u64,
}

/// Unsigned note operation matching ID's create/revise content and parent model.
/// Symbolic references must be bound to signed note event IDs, not blob hashes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind")]
pub enum ProjectedNote {
    #[serde(rename = "note.create")]
    Create { content: ProjectedDocumentRef },
    #[serde(rename = "note.revise")]
    Revise {
        note: ProjectedNoteEventRef,
        content: ProjectedDocumentRef,
        parents: Vec<ProjectedNoteEventRef>,
    },
}

/// One head transition: immutable document/claims plus a logical-file revision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProjectedHeadVersion {
    pub event_ref: ProjectedNoteEventRef,
    pub document: ProjectedVersion,
    pub note: ProjectedNote,
}

/// Immutable Atlas version identity, independent of content and path.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct AtlasVersionKey {
    pub file_id: String,
    pub version_seq: u64,
}

/// One immutable version, with a note operation only if explicitly selected
/// as a historical head transition.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProjectedSelectedVersion {
    pub key: AtlasVersionKey,
    pub document: ProjectedVersion,
    pub note: Option<ProjectedNote>,
}

/// Replay immutable rows with an explicit complete set of historical head
/// transitions, not merely the current heads. Sequence is ingestion order:
/// neither a larger sequence nor later ingestion makes a backfill a head.
///
/// Every row produces document/claims; only selected keys produce note events.
/// Empty selection is document-only. Selected keys must exist and be unique.
/// As with `project_head_versions`, selected rows must include each file's first
/// published head and every subsequent transition; completeness is the caller's
/// obligation. This does not accept incremental suffixes or resolve lineage.
pub fn project_selected_versions(
    rows: &[AtlasVersionRow],
    head_transitions: &[AtlasVersionKey],
) -> Result<Vec<ProjectedSelectedVersion>> {
    // Validate all immutable rows, including backfills, before selecting heads.
    let documents = project_versions(rows)?;
    let key = |row: &AtlasVersionRow| AtlasVersionKey {
        file_id: row.file_id.clone(),
        version_seq: row.version_seq,
    };
    let ordered: BTreeMap<_, _> = rows.iter().map(|row| (key(row), row)).collect();
    let mut selected = BTreeSet::new();
    for transition in head_transitions {
        if !ordered.contains_key(transition) {
            return Err(refuse(format!(
                "unknown head-transition key {transition:?}"
            )));
        }
        if !selected.insert(transition.clone()) {
            return Err(refuse(format!(
                "duplicate head-transition key {transition:?}"
            )));
        }
    }
    let heads: Vec<_> = selected.iter().map(|key| ordered[key].clone()).collect();
    let mut notes: BTreeMap<_, _> = project_head_versions(&heads)?
        .into_iter()
        .map(|head| {
            (
                AtlasVersionKey {
                    file_id: head.event_ref.file_id,
                    version_seq: head.event_ref.version_seq,
                },
                head.note,
            )
        })
        .collect();
    Ok(ordered
        .into_keys()
        .zip(documents)
        .map(|(key, document)| {
            let note = notes.remove(&key);
            ProjectedSelectedVersion {
                key,
                document,
                note,
            }
        })
        .collect())
}

fn refuse(message: impl std::fmt::Display) -> Error {
    Error::fatal(format!("version_projection: {message}"))
}

/// Project one Atlas version row to a document and deterministic Atlas claims.
///
/// The document identity follows ID's BLAKE3 document hash. `atlas.version`
/// carries the keying §2.1 puts in the publication event or its claims
/// (`file_id`, `version_seq`, path, locator, ingestion time); §7 permits
/// appending a name without redefining a v1 name. `atlas.wasGeneratedBy` is
/// the §3 relation to the ingest batch. Both claim values are serialized from
/// the input row, so a verifier can recompute them without projector state.
pub fn project_version(row: &AtlasVersionRow) -> Result<ProjectedVersion> {
    if row.file_id.trim().is_empty() || row.version_seq == 0 || row.batch_id.trim().is_empty() {
        return Err(refuse(
            "file_id, positive version_seq, and batch_id are required",
        ));
    }
    if row.media_type.is_empty() || row.media_type.len() > 128 || !is_nfc(&row.media_type) {
        return Err(refuse("media_type must be 1 to 128 bytes of NFC text"));
    }
    let content = row.content.as_bytes().to_vec();
    let content_hash = blake3::hash(&content).to_hex().to_string();
    let version_value = serde_json::to_string(&serde_json::json!({
        "file_id": row.file_id,
        "version_seq": row.version_seq,
        "path": row.path,
        "source_locator": row.source_locator,
        "ingested_at": row.ingested_at,
        "batch_id": row.batch_id,
        "projection_version": PROJECTION_VERSION,
    }))
    .map_err(|error| refuse(error))?;
    let batch_value = serde_json::to_string(&serde_json::json!({
        "batch_id": row.batch_id,
        "file_id": row.file_id,
        "version_seq": row.version_seq,
        "projection_version": PROJECTION_VERSION,
    }))
    .map_err(|error| refuse(error))?;

    Ok(ProjectedVersion {
        kind: DOCUMENT_PUT_KIND.to_owned(),
        content_hash: content_hash.clone(),
        media_type: row.media_type.clone(),
        size: content.len() as u64,
        content,
        projection_version: PROJECTION_VERSION.to_owned(),
        claims: vec![
            ProjectedTag {
                kind: "tag.set".to_owned(),
                subject: ProjectedSubject::DocumentPut,
                name: "atlas.version".to_owned(),
                value: version_value,
            },
            ProjectedTag {
                kind: "tag.set".to_owned(),
                subject: ProjectedSubject::DocumentPut,
                name: "atlas.wasGeneratedBy".to_owned(),
                value: batch_value,
            },
        ],
    })
}

/// Project a fixture/read batch in stable Atlas key order, rejecting duplicate
/// `(file_id, version_seq)` rows rather than making output depend on row order.
pub fn project_versions(rows: &[AtlasVersionRow]) -> Result<Vec<ProjectedVersion>> {
    let mut ordered = rows.iter().collect::<Vec<_>>();
    ordered.sort_by(|a, b| (&a.file_id, a.version_seq).cmp(&(&b.file_id, b.version_seq)));
    let mut seen = BTreeSet::new();
    let mut projected = Vec::with_capacity(ordered.len());
    for row in ordered {
        if !seen.insert((row.file_id.as_str(), row.version_seq)) {
            return Err(refuse(format!(
                "duplicate Atlas version key ({:?}, {})",
                row.file_id, row.version_seq
            )));
        }
        projected.push(project_version(row)?);
    }
    Ok(projected)
}

/// Recompute a complete head history from successive Atlas head-version rows.
///
/// Every supplied row must represent a head transition. This is a stateless
/// replay from the first published head, not an incremental publication API:
/// do not pass arbitrary non-head backfills or only a suffix of the history.
/// Rows are ordered by `(file_id, version_seq)`; gaps and renames are allowed.
/// Ordinary immutable-version/backfill projection remains `project_versions`.
pub fn project_head_versions(rows: &[AtlasVersionRow]) -> Result<Vec<ProjectedHeadVersion>> {
    let documents = project_versions(rows)?;
    let mut ordered = rows.iter().collect::<Vec<_>>();
    ordered.sort_by(|a, b| (&a.file_id, a.version_seq).cmp(&(&b.file_id, b.version_seq)));
    // The stable create reference and changing prior-head reference are distinct.
    // Neither the path nor content hash determines logical-file identity.
    let mut heads: BTreeMap<String, (ProjectedNoteEventRef, ProjectedNoteEventRef)> =
        BTreeMap::new();
    Ok(ordered
        .into_iter()
        .zip(documents)
        .map(|(row, document)| {
            let event_ref = ProjectedNoteEventRef {
                file_id: row.file_id.clone(),
                version_seq: row.version_seq,
            };
            let content = ProjectedDocumentRef {
                content: document.content_hash.clone(),
                media_type: document.media_type.clone(),
                size: document.size,
            };
            let note = match heads.entry(row.file_id.clone()) {
                Entry::Vacant(entry) => {
                    entry.insert((event_ref.clone(), event_ref.clone()));
                    ProjectedNote::Create { content }
                }
                Entry::Occupied(mut entry) => {
                    let (create, prior_head) = entry.get_mut();
                    let note = ProjectedNote::Revise {
                        note: create.clone(),
                        content,
                        parents: vec![prior_head.clone()],
                    };
                    *prior_head = event_ref.clone();
                    note
                }
            };
            ProjectedHeadVersion {
                event_ref,
                document,
                note,
            }
        })
        .collect())
}

/// Explicit file identity binding. `note_create` is caller-attested: it may name
/// an already-published create outside this batch, including backfill-only files.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AtlasFileNoteBinding {
    pub file_id: String,
    pub note_create: AtlasVersionKey,
    pub source_anchor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AtlasRevisionKind {
    Edit,
    Rename,
    Split,
    Merge,
}

/// Explicit Atlas lineage, unrelated to the selected note-head predecessor.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AtlasRevisionEdge {
    pub newer: AtlasVersionKey,
    pub prior: AtlasVersionKey,
    pub edge_kind: AtlasRevisionKind,
    pub confidence: f64,
}

/// Event namespaces stay distinct even when they share an Atlas row key.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum ProjectedLineageRef {
    DocumentPut(AtlasVersionKey),
    NoteCreate(AtlasVersionKey),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum ProjectedLineageValue {
    File {
        file_id: String,
        source_anchor: Option<String>,
    },
    Specialization {
        note: ProjectedLineageRef,
    },
    Revision {
        document: ProjectedLineageRef,
        edge_kind: AtlasRevisionKind,
        confidence: f64,
    },
}

/// Unsigned descriptor, not a tag payload containing fabricated event hashes.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProjectedLineageClaim {
    pub subject: ProjectedLineageRef,
    pub name: String,
    pub value: ProjectedLineageValue,
}

impl ProjectedLineageClaim {
    /// Materialize JSON text only after typed event references can be resolved.
    /// The subject must separately be resolved in the same typed namespace.
    pub fn value_json(
        &self,
        resolve: impl Fn(&ProjectedLineageRef) -> Option<String>,
    ) -> Result<String> {
        let event_id = |reference: &ProjectedLineageRef| -> Result<String> {
            let id = resolve(reference).ok_or_else(|| refuse("unresolved lineage event"))?;
            if id.len() != 64
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(refuse("lineage event ID must be 64 lowercase hex bytes"));
            }
            Ok(id)
        };
        let mut value = match &self.value {
            ProjectedLineageValue::File {
                file_id,
                source_anchor,
            } => {
                let mut value = serde_json::json!({"file_id": file_id});
                if let Some(anchor) = source_anchor {
                    value["source_anchor"] = anchor.clone().into();
                }
                value
            }
            ProjectedLineageValue::Specialization { note } => {
                serde_json::json!({"note": event_id(note)?})
            }
            ProjectedLineageValue::Revision {
                document,
                edge_kind,
                confidence,
            } => {
                if !confidence.is_finite() || !(0.0..=1.0).contains(confidence) {
                    return Err(refuse("confidence must be finite and in [0,1]"));
                }
                serde_json::json!({"document": event_id(document)?, "edge_kind": edge_kind, "confidence": confidence})
            }
        };
        value["projection_version"] = PROJECTION_VERSION.into();
        let text = serde_json::to_string(&value).map_err(refuse)?;
        if text.len() > 512 || !is_nfc(&text) {
            return Err(refuse("lineage JSON must be at most 512 bytes of NFC text"));
        }
        Ok(text)
    }
}

/// Project only explicit relation facts; emits no note operations. All local
/// versions require a binding, and both revision endpoints must be local rows.
/// External note-create identities are trusted inputs, never inferred from rows.
pub fn project_lineage_claims(
    rows: &[AtlasVersionRow],
    bindings: &[AtlasFileNoteBinding],
    edges: &[AtlasRevisionEdge],
) -> Result<Vec<ProjectedLineageClaim>> {
    // Reuse immutable-row validation without changing its output or contracts.
    project_versions(rows)?;
    let keys: BTreeSet<_> = rows
        .iter()
        .map(|row| AtlasVersionKey {
            file_id: row.file_id.clone(),
            version_seq: row.version_seq,
        })
        .collect();
    let files: BTreeSet<_> = rows.iter().map(|row| row.file_id.as_str()).collect();
    let mut by_file = BTreeMap::new();
    for binding in bindings {
        if !files.contains(binding.file_id.as_str())
            || binding.note_create.file_id != binding.file_id
            || binding.note_create.version_seq == 0
            || by_file.insert(binding.file_id.as_str(), binding).is_some()
        {
            return Err(refuse("unknown, duplicate or invalid file-note binding"));
        }
    }
    if by_file.len() != files.len() {
        return Err(refuse(
            "every file requires an explicit note-create binding",
        ));
    }
    let mut claims = Vec::new();
    for binding in by_file.values() {
        claims.push(ProjectedLineageClaim {
            subject: ProjectedLineageRef::NoteCreate(binding.note_create.clone()),
            name: "atlas.fileId".into(),
            value: ProjectedLineageValue::File {
                file_id: binding.file_id.clone(),
                source_anchor: binding.source_anchor.clone(),
            },
        });
    }
    for key in &keys {
        let binding = by_file
            .get(key.file_id.as_str())
            .ok_or_else(|| refuse("missing binding"))?;
        claims.push(ProjectedLineageClaim {
            subject: ProjectedLineageRef::DocumentPut(key.clone()),
            name: "atlas.specializationOf".into(),
            value: ProjectedLineageValue::Specialization {
                note: ProjectedLineageRef::NoteCreate(binding.note_create.clone()),
            },
        });
    }
    // Confidence is data, never an ordering key. Duplicate facts are refused
    // even if they disagree on confidence instead of silently overwriting.
    let mut ordered_edges = BTreeMap::new();
    for edge in edges {
        if !keys.contains(&edge.newer)
            || !keys.contains(&edge.prior)
            || edge.newer == edge.prior
            || !edge.confidence.is_finite()
            || !(0.0..=1.0).contains(&edge.confidence)
            || ordered_edges
                .insert((&edge.newer, &edge.prior, &edge.edge_kind), edge)
                .is_some()
        {
            return Err(refuse("unknown, self, duplicate or invalid revision edge"));
        }
    }
    for edge in ordered_edges.values() {
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
    // A real hash has exactly this width and ASCII encoding; do not let short
    // symbolic row references disguise an oversized eventual ID claim value.
    for claim in &claims {
        claim.value_json(|_| Some("0".repeat(64)))?;
    }
    Ok(claims)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn fixture_rows() -> Vec<AtlasVersionRow> {
        serde_json::from_str(include_str!("../../../fixtures/id_projection_rows.json")).unwrap()
    }

    fn lineage_fixture() -> (Vec<AtlasFileNoteBinding>, Vec<AtlasRevisionEdge>) {
        // Oracle input: explicit Atlas row-key facts carried by the fixture,
        // not inferred from sequence, matching content, paths or selected heads.
        let values: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../fixtures/id_projection_rows.json"))
                .unwrap();
        let bindings = values
            .iter()
            .filter_map(|v| v.get("file_note_binding"))
            .map(|v| serde_json::from_value(v.clone()).unwrap())
            .collect();
        let edges = values
            .iter()
            .filter_map(|v| v.get("revision_edges"))
            .flat_map(|v| v.as_array().unwrap())
            .map(|v| serde_json::from_value(v.clone()).unwrap())
            .collect();
        (bindings, edges)
    }

    #[test]
    fn fixture_lineage_recomputes_from_explicit_atlas_keys() {
        let rows = fixture_rows();
        let (bindings, edges) = lineage_fixture();
        let claims = project_lineage_claims(&rows, &bindings, &edges).unwrap();
        assert_eq!(claims.len(), bindings.len() + rows.len() + edges.len());
        // Independent set oracle: exactly the supplied binding and edge facts,
        // plus one specialization for every immutable row, backfills included.
        for binding in &bindings {
            assert!(claims.contains(&ProjectedLineageClaim {
                subject: ProjectedLineageRef::NoteCreate(binding.note_create.clone()),
                name: "atlas.fileId".into(),
                value: ProjectedLineageValue::File {
                    file_id: binding.file_id.clone(),
                    source_anchor: binding.source_anchor.clone()
                },
            }));
        }
        for row in &rows {
            let binding = bindings.iter().find(|b| b.file_id == row.file_id).unwrap();
            assert!(claims.contains(&ProjectedLineageClaim {
                subject: ProjectedLineageRef::DocumentPut(AtlasVersionKey {
                    file_id: row.file_id.clone(),
                    version_seq: row.version_seq
                }),
                name: "atlas.specializationOf".into(),
                value: ProjectedLineageValue::Specialization {
                    note: ProjectedLineageRef::NoteCreate(binding.note_create.clone())
                },
            }));
        }
        for edge in &edges {
            assert!(claims.contains(&ProjectedLineageClaim {
                subject: ProjectedLineageRef::DocumentPut(edge.newer.clone()),
                name: "atlas.wasRevisionOf".into(),
                value: ProjectedLineageValue::Revision {
                    document: ProjectedLineageRef::DocumentPut(edge.prior.clone()),
                    edge_kind: edge.edge_kind.clone(),
                    confidence: edge.confidence
                },
            }));
        }
        for claim in &claims {
            let text = claim.value_json(|_| Some("a".repeat(64))).unwrap();
            assert!(text.len() <= 512 && is_nfc(&text));
        }
    }

    #[test]
    fn lineage_rejects_invalid_facts_and_final_id_values() {
        let rows = fixture_rows();
        let (bindings, edges) = lineage_fixture();
        assert!(project_lineage_claims(&rows, &[], &edges).is_err());
        let mut duplicate = bindings.clone();
        duplicate.push(bindings[0].clone());
        assert!(project_lineage_claims(&rows, &duplicate, &edges).is_err());
        let mut invalid = edges.clone();
        invalid[0].prior.version_seq = u64::MAX;
        assert!(project_lineage_claims(&rows, &bindings, &invalid).is_err());
        for confidence in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
            let mut invalid = edges.clone();
            invalid[0].confidence = confidence;
            assert!(project_lineage_claims(&rows, &bindings, &invalid).is_err());
        }
        let mut invalid = edges.clone();
        invalid[0].prior = invalid[0].newer.clone();
        assert!(project_lineage_claims(&rows, &bindings, &invalid).is_err());
        let mut duplicate = edges.clone();
        duplicate.push(edges[0].clone());
        assert!(project_lineage_claims(&rows, &bindings, &duplicate).is_err());
        for anchor in ["x".repeat(512), "\"".repeat(250), "e\u{301}".into()] {
            let mut invalid = bindings.clone();
            invalid[0].source_anchor = Some(anchor);
            assert!(project_lineage_claims(&rows, &invalid, &edges).is_err());
        }
        let claims = project_lineage_claims(&rows, &bindings, &edges).unwrap();
        let specialization = claims
            .iter()
            .find(|c| c.name == "atlas.specializationOf")
            .unwrap();
        assert!(specialization.value_json(|_| None).is_err());
        assert!(specialization.value_json(|_| Some("A".repeat(64))).is_err());
    }

    #[test]
    fn lineage_wire_values_match_typed_id_and_byte_limit_oracles() {
        // Oracle: ID tag JSON, with distinct note/document IDs to detect
        // namespace substitution, and exact serialized UTF-8 byte boundaries.
        let (bindings, edges) = lineage_fixture();
        let claims = project_lineage_claims(&fixture_rows(), &bindings, &edges).unwrap();
        for claim in claims {
            let value: serde_json::Value = serde_json::from_str(
                &claim
                    .value_json(|reference| {
                        Some(match reference {
                            ProjectedLineageRef::NoteCreate(_) => "a".repeat(64),
                            ProjectedLineageRef::DocumentPut(_) => "b".repeat(64),
                        })
                    })
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(value["projection_version"], "v1");
            match claim.value {
                ProjectedLineageValue::File {
                    file_id,
                    source_anchor,
                } => {
                    assert_eq!(value["file_id"], file_id);
                    assert_eq!(
                        value.get("source_anchor").and_then(|v| v.as_str()),
                        source_anchor.as_deref()
                    );
                }
                ProjectedLineageValue::Specialization { .. } => {
                    assert_eq!(value["note"], "a".repeat(64))
                }
                ProjectedLineageValue::Revision {
                    edge_kind,
                    confidence,
                    ..
                } => {
                    assert_eq!(value["document"], "b".repeat(64));
                    assert_eq!(value["edge_kind"], serde_json::to_value(edge_kind).unwrap());
                    assert_eq!(value["confidence"], confidence);
                }
            }
        }
        let mut claim = ProjectedLineageClaim {
            subject: ProjectedLineageRef::NoteCreate(bindings[0].note_create.clone()),
            name: "atlas.fileId".into(),
            value: ProjectedLineageValue::File {
                file_id: "f".into(),
                source_anchor: Some(String::new()),
            },
        };
        let overhead = claim.value_json(|_| None).unwrap().len();
        if let ProjectedLineageValue::File { source_anchor, .. } = &mut claim.value {
            *source_anchor = Some("x".repeat(512 - overhead));
        }
        assert_eq!(claim.value_json(|_| None).unwrap().len(), 512);
        if let ProjectedLineageValue::File { source_anchor, .. } = &mut claim.value {
            source_anchor.as_mut().unwrap().push('x');
        }
        assert!(claim.value_json(|_| None).is_err());
    }

    proptest! {
        #[test]
        fn lineage_replay_is_independent_of_content_paths_and_input_order(seed in any::<u64>()) {
            // Oracle: explicit row-key relation set is invariant under row/edge/
            // binding permutations, renames and blob deduplication.
            let mut rows = fixture_rows();
            let (mut bindings, mut edges) = lineage_fixture();
            let expected = project_lineage_claims(&rows, &bindings, &edges).unwrap();
            let amount = seed as usize % rows.len(); rows.rotate_left(amount);
            rows.reverse(); bindings.reverse(); edges.reverse();
            for row in &mut rows { row.content = "shared blob".into(); row.path = format!("renamed-{seed}"); }
            let actual = project_lineage_claims(&rows, &bindings, &edges).unwrap();
            prop_assert_eq!(serde_json::to_vec(&actual).unwrap(), serde_json::to_vec(&expected).unwrap());
        }
    }

    fn fixture_head_keys() -> Vec<AtlasVersionKey> {
        // Oracle input: explicit historical transition facts, not max(sequence).
        #[derive(Deserialize)]
        struct Selection {
            file_id: String,
            version_seq: u64,
            head_transition: bool,
        }
        let rows: Vec<Selection> =
            serde_json::from_str(include_str!("../../../fixtures/id_projection_rows.json"))
                .unwrap();
        rows.into_iter()
            .filter(|row| row.head_transition)
            .map(|row| AtlasVersionKey {
                file_id: row.file_id,
                version_seq: row.version_seq,
            })
            .collect()
    }

    #[test]
    fn fixture_explicit_selection_recomputes_heads_without_backfill_revisions() {
        let rows = fixture_rows();
        let keys = fixture_head_keys();
        let heads: Vec<_> = rows
            .iter()
            .filter(|row| {
                keys.contains(&AtlasVersionKey {
                    file_id: row.file_id.clone(),
                    version_seq: row.version_seq,
                })
            })
            .cloned()
            .collect();
        let output = project_selected_versions(&rows, &keys).unwrap();
        assert_eq!(output.len(), rows.len());
        assert!(
            heads.len() < rows.len(),
            "fixture must include non-head backfills"
        );
        for projected in &output {
            let row = rows
                .iter()
                .find(|row| {
                    row.file_id == projected.key.file_id
                        && row.version_seq == projected.key.version_seq
                })
                .unwrap();
            assert_recomputes(row, &projected.document);
            if keys.contains(&projected.key) {
                assert_head_recomputes(
                    &heads,
                    &ProjectedHeadVersion {
                        event_ref: ProjectedNoteEventRef {
                            file_id: row.file_id.clone(),
                            version_seq: row.version_seq,
                        },
                        document: projected.document.clone(),
                        note: projected.note.clone().unwrap(),
                    },
                );
            } else {
                assert_eq!(
                    projected.note, None,
                    "non-head backfills never revise a note"
                );
            }
        }
        let mut reversed_rows = rows.clone();
        reversed_rows.reverse();
        let mut reversed_keys = keys.clone();
        reversed_keys.reverse();
        assert_eq!(
            serde_json::to_vec(&output).unwrap(),
            serde_json::to_vec(&project_selected_versions(&reversed_rows, &reversed_keys).unwrap())
                .unwrap()
        );
    }

    #[test]
    fn explicit_selection_refuses_unknown_duplicate_and_invalid_rows() {
        // Oracle: a selection is a set of valid immutable keys, with no dangling
        // references or order-dependent overwrites, even for document-only rows.
        let rows = fixture_rows();
        let mut keys = fixture_head_keys();
        keys.push(keys[0].clone());
        assert!(project_selected_versions(&rows, &keys).is_err());
        let absent = AtlasVersionKey {
            file_id: "missing".into(),
            version_seq: 1,
        };
        assert!(project_selected_versions(&rows, &[absent]).is_err());
        let wrong_seq = AtlasVersionKey {
            file_id: rows[0].file_id.clone(),
            version_seq: u64::MAX,
        };
        assert!(project_selected_versions(&rows, &[wrong_seq]).is_err());
        let mut duplicates = rows.clone();
        duplicates.push(rows[0].clone());
        assert!(project_selected_versions(&duplicates, &[]).is_err());
        let mut invalid = rows.clone();
        invalid.last_mut().unwrap().version_seq = 0;
        assert!(project_selected_versions(&invalid, &[]).is_err());
        let documents_only = project_selected_versions(&rows, &[]).unwrap();
        assert!(documents_only.iter().all(|version| version.note.is_none()));
        assert_eq!(
            documents_only
                .into_iter()
                .map(|version| version.document)
                .collect::<Vec<_>>(),
            project_versions(&rows).unwrap()
        );
        assert!(project_selected_versions(&[], &[]).unwrap().is_empty());
    }

    /// Oracle: every publication field is recomputed from the fixture Atlas
    /// row, not from projector state or a precomputed expected event.
    fn assert_recomputes(row: &AtlasVersionRow, projected: &ProjectedVersion) {
        let bytes = row.content.as_bytes();
        let hash = blake3::hash(bytes).to_hex().to_string();
        assert_eq!(projected.kind, DOCUMENT_PUT_KIND);
        assert_eq!(projected.content_hash, hash);
        assert_eq!(projected.content, bytes);
        assert_eq!(projected.media_type, row.media_type);
        assert_eq!(projected.size, bytes.len() as u64);
        assert_eq!(projected.projection_version, PROJECTION_VERSION);
        assert_eq!(projected.claims.len(), 2);
        for claim in &projected.claims {
            assert_eq!(claim.kind, "tag.set");
            assert_eq!(claim.subject, ProjectedSubject::DocumentPut);
        }
        let version: serde_json::Value = serde_json::from_str(&projected.claims[0].value).unwrap();
        assert_eq!(projected.claims[0].name, "atlas.version");
        assert_eq!(version["file_id"], row.file_id);
        assert_eq!(version["version_seq"], row.version_seq);
        assert_eq!(version["path"], row.path);
        assert_eq!(version["source_locator"], row.source_locator);
        assert_eq!(version["ingested_at"], row.ingested_at);
        assert_eq!(version["batch_id"], row.batch_id);
        assert_eq!(version["projection_version"], PROJECTION_VERSION);
        let batch: serde_json::Value = serde_json::from_str(&projected.claims[1].value).unwrap();
        assert_eq!(projected.claims[1].name, "atlas.wasGeneratedBy");
        assert_eq!(batch["batch_id"], row.batch_id);
        assert_eq!(batch["file_id"], row.file_id);
        assert_eq!(batch["version_seq"], row.version_seq);
        assert_eq!(batch["projection_version"], PROJECTION_VERSION);
    }

    #[test]
    fn fixture_versions_project_to_recomputable_documents_and_atlas_claims() {
        let rows = fixture_rows();
        let projected = project_versions(&rows).unwrap();
        assert_eq!(projected.len(), rows.len());
        for output in &projected {
            let row = rows
                .iter()
                .find(|row| {
                    let content_hash = blake3::hash(row.content.as_bytes()).to_hex().to_string();
                    content_hash == output.content_hash
                })
                .unwrap();
            assert_recomputes(row, output);
        }
    }

    #[test]
    fn fixture_projection_is_stable_when_atlas_rows_arrive_out_of_order() {
        let mut rows = fixture_rows();
        let expected = project_versions(&rows).unwrap();
        rows.reverse();
        assert_eq!(project_versions(&rows).unwrap(), expected);
    }

    #[test]
    fn duplicate_atlas_version_key_is_refused() {
        let mut rows = fixture_rows();
        rows.push(rows[0].clone());
        assert!(project_versions(&rows).is_err());
    }

    /// Oracle: recomputation from Atlas fixture rows, independently selecting
    /// the minimum and predecessor keys within each file (no projector state).
    fn assert_head_recomputes(rows: &[AtlasVersionRow], output: &ProjectedHeadVersion) {
        let row = rows
            .iter()
            .find(|row| {
                row.file_id == output.event_ref.file_id
                    && row.version_seq == output.event_ref.version_seq
            })
            .unwrap();
        assert_recomputes(row, &output.document);
        let reference = |row: &AtlasVersionRow| ProjectedNoteEventRef {
            file_id: row.file_id.clone(),
            version_seq: row.version_seq,
        };
        let content = ProjectedDocumentRef {
            content: blake3::hash(row.content.as_bytes()).to_hex().to_string(),
            media_type: row.media_type.clone(),
            size: row.content.len() as u64,
        };
        let prior = rows
            .iter()
            .filter(|candidate| {
                candidate.file_id == row.file_id && candidate.version_seq < row.version_seq
            })
            .max_by_key(|candidate| candidate.version_seq);
        let expected = match prior {
            None => ProjectedNote::Create { content },
            Some(prior) => {
                let first = rows
                    .iter()
                    .filter(|candidate| candidate.file_id == row.file_id)
                    .min_by_key(|candidate| candidate.version_seq)
                    .unwrap();
                ProjectedNote::Revise {
                    note: reference(first),
                    content,
                    parents: vec![reference(prior)],
                }
            }
        };
        assert_eq!(output.note, expected);
        let wire = serde_json::to_value(&output.note).unwrap();
        // Contract oracle: ID DocumentRef has exactly these fields (and denies
        // unknown fields); it names the blob as `content`, not `content_hash`.
        assert_eq!(
            wire["content"],
            serde_json::json!({
                "content": blake3::hash(row.content.as_bytes()).to_hex().to_string(),
                "media_type": row.media_type,
                "size": row.content.len() as u64,
            })
        );
        match expected {
            ProjectedNote::Create { .. } => {
                assert_eq!(wire["kind"], "note.create");
                assert!(wire.get("parents").is_none());
                assert!(wire.get("note").is_none());
            }
            ProjectedNote::Revise { .. } => assert_eq!(wire["kind"], "note.revise"),
        }
    }

    #[test]
    fn fixture_head_chains_recompute_with_independent_file_heads() {
        let mut rows = fixture_rows();
        // A third alpha head distinguishes stable note identity from prior head.
        let mut third = rows[1].clone();
        third.version_seq = 7;
        rows.push(third);
        let projected = project_head_versions(&rows).unwrap();
        assert_eq!(projected.len(), rows.len());
        for output in &projected {
            assert_head_recomputes(&rows, output);
        }
    }

    #[test]
    fn fixture_head_replay_is_deterministic_and_rejects_duplicate_keys() {
        // Oracle: full row recomputation is invariant under input permutation.
        let mut rows = fixture_rows();
        let first = project_head_versions(&rows).unwrap();
        rows.reverse();
        let replay = project_head_versions(&rows).unwrap();
        assert_eq!(first, replay);
        assert_eq!(
            serde_json::to_vec(&first).unwrap(),
            serde_json::to_vec(&replay).unwrap()
        );
        rows.push(rows[0].clone());
        assert!(project_head_versions(&rows).is_err());
        assert!(project_head_versions(&[]).unwrap().is_empty());
    }

    proptest! {
        #[test]
        fn selected_histories_match_fixture_row_model_and_replay(
            inputs in prop::collection::btree_map((0u8..4, 1u64..100), any::<bool>(), 0..40),
        ) {
            // Oracle: recompute every document from its row and note parents
            // from only the explicitly selected row subset, separately per file.
            let rows: Vec<_> = inputs.keys().map(|(file, version_seq)| {
                let mut row = fixture_rows()[0].clone();
                row.file_id = format!("file-{file}");
                row.version_seq = *version_seq;
                row.path = format!("renamed-{file}-{version_seq}.md");
                row
            }).collect();
            let heads: Vec<_> = rows.iter().filter(|row| {
                let file = row.file_id.trim_start_matches("file-").parse::<u8>().unwrap();
                inputs[&(file, row.version_seq)]
            }).cloned().collect();
            let keys: Vec<_> = heads.iter().map(|row| AtlasVersionKey {
                file_id: row.file_id.clone(), version_seq: row.version_seq,
            }).collect();
            let output = project_selected_versions(&rows, &keys).unwrap();
            prop_assert_eq!(output.len(), rows.len());
            prop_assert_eq!(output.iter().filter(|version| version.note.is_some()).count(), keys.len());
            for (row, version) in rows.iter().zip(&output) {
                assert_eq!(version.key.file_id, row.file_id);
                assert_eq!(version.key.version_seq, row.version_seq);
                assert_recomputes(row, &version.document);
                if keys.contains(&version.key) {
                    assert_head_recomputes(&heads, &ProjectedHeadVersion {
                        event_ref: ProjectedNoteEventRef { file_id: row.file_id.clone(), version_seq: row.version_seq },
                        document: version.document.clone(), note: version.note.clone().unwrap(),
                    });
                } else {
                    prop_assert_eq!(&version.note, &None);
                }
            }
            let mut reversed_rows = rows;
            reversed_rows.reverse();
            let mut reversed_keys = keys;
            reversed_keys.reverse();
            prop_assert_eq!(project_selected_versions(&reversed_rows, &reversed_keys).unwrap(), output);
        }

        #[test]
        fn head_histories_match_row_model_despite_gaps_and_shared_content(
            keys in prop::collection::vec((0u8..4, 1u64..100), 0..32),
        ) {
            // Oracle: independent row-key predecessor/minimum recomputation.
            // All files share bytes deliberately: blob identity is not note identity.
            let keys: BTreeSet<_> = keys.into_iter().collect();
            let rows: Vec<_> = keys.into_iter().map(|(file, version_seq)| {
                let mut row = fixture_rows()[0].clone();
                row.file_id = format!("file-{file}");
                row.version_seq = version_seq;
                row.path = format!("renamed-{file}-{version_seq}.md");
                row
            }).collect();
            let projected = project_head_versions(&rows).unwrap();
            prop_assert_eq!(projected.len(), rows.len());
            for output in &projected {
                assert_head_recomputes(&rows, output);
            }
            let mut permuted = rows.clone();
            permuted.reverse();
            prop_assert_eq!(project_head_versions(&permuted).unwrap(), projected);
        }

        #[test]
        fn arbitrary_utf8_content_recomputes_to_its_document_hash(
            content in "[ -~]{0,128}",
            version_seq in 1u64..10_000,
        ) {
            let row = AtlasVersionRow {
                file_id: "property-file".to_owned(),
                version_seq,
                media_type: "text/plain".to_owned(),
                content: content.clone(),
                path: "fixture.txt".to_owned(),
                source_locator: "system@property@now".to_owned(),
                ingested_at: "2026-11-24T00:00:00Z".to_owned(),
                batch_id: "property-batch".to_owned(),
            };
            let projected = project_version(&row).unwrap();
            prop_assert_eq!(projected.content, content.as_bytes());
            prop_assert_eq!(projected.content_hash, blake3::hash(content.as_bytes()).to_hex().to_string());
            prop_assert_eq!(projected.size, content.len() as u64);
            prop_assert!(projected.claims.iter().all(|claim| claim.subject == ProjectedSubject::DocumentPut));
        }
    }
}
