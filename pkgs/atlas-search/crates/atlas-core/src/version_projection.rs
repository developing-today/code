//! Deterministic Atlas immutable-version projection primitives (#11182).
//!
//! Projects immutable rows to `document.put` and version/batch claims, with a
//! separate complete-head-history replay that adds `note.create`/`note.revise`
//! descriptors. References remain symbolic until signing. Runtime publication,
//! non-head/backfill selection, lineage, tombstones, and correction clears
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn fixture_rows() -> Vec<AtlasVersionRow> {
        serde_json::from_str(include_str!("../../../fixtures/id_projection_rows.json")).unwrap()
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
