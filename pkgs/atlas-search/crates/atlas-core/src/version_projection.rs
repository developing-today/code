//! Deterministic Atlas immutable-version projection primitives (#11182).
//!
//! This first slice projects each Atlas version row to a content-addressed
//! `document.put` descriptor and the version/batch claims that let a verifier
//! recompute the publication from the row. It does not publish ID events or
//! create file notes; head tracking, note revisions, lineage relations, and
//! correction clears remain later slices of the #6062 follow-on.

use std::collections::BTreeSet;

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

    proptest! {
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
