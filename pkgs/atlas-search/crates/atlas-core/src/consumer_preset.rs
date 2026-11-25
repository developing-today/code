//! The attested-only consumer view preset over an Atlas publication
//! (#13766; publication design §4 and §9.4).
//!
//! A consumer that counts only attested batches selects, from one pinned
//! publication, the versions whose batch has a published
//! `attestation.claim`. The predicate is the one recorded on #13766: a
//! published version is attested iff the publication contains a `tag.set`
//! claim named `atlas.wasGeneratedBy` whose subject is that version's
//! document and whose value names an `attestation.claim` present in the
//! same publication. A rejected batch writes no corpus rows, so no version
//! can name a rejected batch's attestation and the preset makes no outcome
//! check.
//!
//! Until events are signed, both sides of the hop are symbolic: the
//! version's `atlas.wasGeneratedBy` value names its batch by `batch_id`
//! ([`project_version`](crate::version_projection::project_version)), and
//! each [`ProjectedClaim`] carries the `batch_id` of the statement whose
//! canonical bytes are its body ([`project_batch`](crate::id_projection::project_batch)).
//! The preset joins on that `batch_id`. It adds no view machinery and does
//! not filter the publication itself: the projector publishes everything
//! (§4), and this is a consumer-side selection over what was published.
//!
//! Every input is checked before it is trusted, and any inconsistency
//! refuses the whole selection rather than returning a partial one:
//!
//! - an attestation claim must have the `attestation.claim` kind, a body
//!   that decodes to a canonical statement, a subject that recomputes as
//!   the body's sha256, and a `batch_id` equal to the statement's;
//! - two different attestations may not claim the same batch;
//! - a version carries at most one `atlas.wasGeneratedBy` claim, and its
//!   value must name that version's own `file_id` and `version_seq`.
//!
//! The heads-only and full-history presets (#11185) map onto the landed
//! writer-set `include_history` axis and are not defined here.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::attestation::BatchStatement;
use crate::classify::sha256_hex;
use crate::error::{Error, Result};
use crate::id_projection::{CLAIM_KIND, ProjectedClaim};
use crate::version_projection::{ProjectedSelectedVersion, ProjectedSubject};

/// The claim name linking a version document to its ingest batch (§3).
pub const WAS_GENERATED_BY: &str = "atlas.wasGeneratedBy";

/// The `tag.set` kind every Atlas relation claim carries (§3).
const TAG_SET_KIND: &str = "tag.set";

/// The fields of an `atlas.wasGeneratedBy` value the preset reads.
#[derive(Deserialize)]
struct GeneratedBy {
    batch_id: String,
    file_id: String,
    version_seq: u64,
}

fn refuse(message: impl std::fmt::Display) -> Error {
    Error::fatal(format!("consumer_preset: {message}"))
}

/// Index the publication's attestation claims by the batch they attest,
/// after checking that each claim recomputes from its own body.
fn attested_batches(attestations: &[ProjectedClaim]) -> Result<BTreeMap<&str, &str>> {
    let mut batches: BTreeMap<&str, &str> = BTreeMap::new();
    for claim in attestations {
        if claim.kind != CLAIM_KIND {
            return Err(refuse(format!(
                "attestation for batch {:?} has kind {:?}, not {CLAIM_KIND:?}",
                claim.batch_id, claim.kind
            )));
        }
        let statement = BatchStatement::from_canonical_bytes(&claim.body)?;
        if claim.subject != sha256_hex(&claim.body) {
            return Err(refuse(format!(
                "attestation for batch {:?} has subject {:?}, which is not the sha256 of its body",
                claim.batch_id, claim.subject
            )));
        }
        if statement.predicate.batch.batch_id != claim.batch_id {
            return Err(refuse(format!(
                "attestation claims batch {:?} but its statement is for batch {:?}",
                claim.batch_id, statement.predicate.batch.batch_id
            )));
        }
        match batches.insert(claim.batch_id.as_str(), claim.subject.as_str()) {
            Some(previous) if previous != claim.subject => {
                return Err(refuse(format!(
                    "batch {:?} has two different attestations, {previous:?} and {:?}",
                    claim.batch_id, claim.subject
                )));
            }
            _ => {}
        }
    }
    Ok(batches)
}

/// The batch a version's `atlas.wasGeneratedBy` claim names, or `None`
/// when the version carries no such claim.
fn generating_batch(version: &ProjectedSelectedVersion) -> Result<Option<String>> {
    let mut found: Option<String> = None;
    for claim in &version.document.claims {
        if claim.name != WAS_GENERATED_BY {
            continue;
        }
        if claim.kind != TAG_SET_KIND || claim.subject != ProjectedSubject::DocumentPut {
            return Err(refuse(format!(
                "version {:?} has an {WAS_GENERATED_BY} claim that is not a tag.set on its document",
                version.key
            )));
        }
        let value: GeneratedBy = serde_json::from_str(&claim.value).map_err(|error| {
            refuse(format!(
                "version {:?} has an unreadable {WAS_GENERATED_BY} value: {error}",
                version.key
            ))
        })?;
        if value.file_id != version.key.file_id || value.version_seq != version.key.version_seq {
            return Err(refuse(format!(
                "version {:?} has an {WAS_GENERATED_BY} claim for {}@{}",
                version.key, value.file_id, value.version_seq
            )));
        }
        if found.replace(value.batch_id).is_some() {
            return Err(refuse(format!(
                "version {:?} has more than one {WAS_GENERATED_BY} claim",
                version.key
            )));
        }
    }
    Ok(found)
}

/// The attested-only preset: the versions of `versions` whose
/// `atlas.wasGeneratedBy` batch has an attestation in `attestations`,
/// in input order.
///
/// # Errors
///
/// [`Error::Fatal`] when an attestation claim does not recompute from its
/// body, two attestations claim one batch, or a version's
/// `atlas.wasGeneratedBy` claim is malformed, duplicated, or names
/// another version. Nothing is selected when any input is refused.
pub fn attested_only<'a>(
    versions: &'a [ProjectedSelectedVersion],
    attestations: &[ProjectedClaim],
) -> Result<Vec<&'a ProjectedSelectedVersion>> {
    let batches = attested_batches(attestations)?;
    let mut selected = Vec::new();
    for version in versions {
        if let Some(batch_id) = generating_batch(version)? {
            if batches.contains_key(batch_id.as_str()) {
                selected.push(version);
            }
        }
    }
    Ok(selected)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::collections::BTreeSet;

    use proptest::prelude::*;

    use super::*;
    use crate::attestation::{
        Batch, BatchOutcome, BatchPredicate, Builder, BuilderVersion, Classification, DigestSet,
        MaterialEntry, SubjectAnnotations, SubjectEntry,
    };
    use crate::id_projection::project_batch;
    use crate::shards::Tier;
    use crate::version_projection::{AtlasVersionKey, AtlasVersionRow, project_selected_versions};

    const FIXTURE: &str = include_str!("../../../fixtures/id_projection_rows.json");

    fn fixture_rows() -> Vec<AtlasVersionRow> {
        serde_json::from_str(FIXTURE).unwrap()
    }

    fn fixture_head_keys() -> Vec<AtlasVersionKey> {
        #[derive(Deserialize)]
        struct Selection {
            file_id: String,
            version_seq: u64,
            head_transition: bool,
        }
        let rows: Vec<Selection> = serde_json::from_str(FIXTURE).unwrap();
        rows.into_iter()
            .filter(|row| row.head_transition)
            .map(|row| AtlasVersionKey {
                file_id: row.file_id,
                version_seq: row.version_seq,
            })
            .collect()
    }

    fn publication() -> Vec<ProjectedSelectedVersion> {
        project_selected_versions(&fixture_rows(), &fixture_head_keys()).unwrap()
    }

    fn batch_ids() -> Vec<String> {
        fixture_rows()
            .into_iter()
            .map(|row| row.batch_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    fn digest(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    fn statement(batch_id: &str) -> BatchStatement {
        BatchStatement::new(
            vec![SubjectEntry {
                uri: format!("shard-0:{batch_id}"),
                digest: DigestSet { sha256: digest(1) },
                annotations: SubjectAnnotations {
                    spans: vec!["L1-L9".to_owned()],
                },
            }],
            vec![MaterialEntry {
                uri: format!("repo@{}:docs", digest(2)),
                digest: DigestSet { sha256: digest(3) },
            }],
            BatchPredicate {
                builder: Builder {
                    id: "atlas-ingest".to_owned(),
                    version: BuilderVersion {
                        ingester_version: "0.1.0".to_owned(),
                        code_rev: "0123456789abcdef0123456789abcdef01234567".to_owned(),
                    },
                },
                batch: Batch {
                    batch_id: batch_id.to_owned(),
                    staging_unit_id: "unit-1".to_owned(),
                    gate_run_id: "gate-1".to_owned(),
                    started_at: "2026-11-23T00:00:00Z".to_owned(),
                    finished_at: "2026-11-23T00:01:00Z".to_owned(),
                    outcome: BatchOutcome::Published,
                    dedup_dropped: Vec::new(),
                },
                classification: Classification {
                    tier: Tier::Public,
                    scanner_name: "atlas-classify".to_owned(),
                    scanner_version: "0.1.0".to_owned(),
                    scan_verdict: "pass".to_owned(),
                    scan_evidence: digest(4),
                },
                prior_attestation: None,
            },
        )
    }

    fn attestation(batch_id: &str) -> ProjectedClaim {
        project_batch(&statement(batch_id)).unwrap()
    }

    /// Oracle: recompute the selection from the Atlas rows alone — the
    /// keys of rows whose batch is in `attested`, in Atlas key order.
    fn oracle(attested: &BTreeSet<String>) -> Vec<AtlasVersionKey> {
        let mut keys: Vec<_> = fixture_rows()
            .into_iter()
            .filter(|row| attested.contains(&row.batch_id))
            .map(|row| AtlasVersionKey {
                file_id: row.file_id,
                version_seq: row.version_seq,
            })
            .collect();
        keys.sort();
        keys
    }

    fn keys(selected: &[&ProjectedSelectedVersion]) -> Vec<AtlasVersionKey> {
        selected.iter().map(|version| version.key.clone()).collect()
    }

    #[test]
    fn fixture_selects_exactly_the_versions_of_attested_batches() {
        let versions = publication();
        let attested: BTreeSet<String> = ["batch-alpha-1", "batch-beta-1", "batch-only-backfill"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let claims: Vec<_> = attested.iter().map(|id| attestation(id)).collect();
        let selected = attested_only(&versions, &claims).unwrap();
        assert!(!selected.is_empty());
        assert!(selected.len() < versions.len());
        assert_eq!(keys(&selected), oracle(&attested));
    }

    #[test]
    fn no_attestations_selects_nothing_and_all_selects_everything() {
        let versions = publication();
        assert!(attested_only(&versions, &[]).unwrap().is_empty());
        let claims: Vec<_> = batch_ids().iter().map(|id| attestation(id)).collect();
        assert_eq!(
            attested_only(&versions, &claims).unwrap().len(),
            versions.len()
        );
    }

    #[test]
    fn heads_and_backfills_are_selected_alike() {
        let versions = publication();
        let claims: Vec<_> = batch_ids().iter().map(|id| attestation(id)).collect();
        let selected = attested_only(&versions, &claims).unwrap();
        assert!(selected.iter().any(|version| version.note.is_some()));
        assert!(selected.iter().any(|version| version.note.is_none()));
    }

    #[test]
    fn an_attestation_for_an_unpublished_batch_selects_nothing() {
        let versions = publication();
        let claims = vec![attestation("batch-not-in-corpus")];
        assert!(attested_only(&versions, &claims).unwrap().is_empty());
    }

    #[test]
    fn a_version_without_a_generated_by_claim_is_not_attested() {
        let mut versions = publication();
        let target = versions[0].key.clone();
        versions[0]
            .document
            .claims
            .retain(|claim| claim.name != WAS_GENERATED_BY);
        let claims: Vec<_> = batch_ids().iter().map(|id| attestation(id)).collect();
        let selected = attested_only(&versions, &claims).unwrap();
        assert_eq!(selected.len(), versions.len() - 1);
        assert!(selected.iter().all(|version| version.key != target));
    }

    #[test]
    fn the_same_attestation_twice_is_accepted() {
        let versions = publication();
        let claims = vec![attestation("batch-alpha-1"), attestation("batch-alpha-1")];
        let attested: BTreeSet<String> = ["batch-alpha-1".to_owned()].into();
        assert_eq!(
            keys(&attested_only(&versions, &claims).unwrap()),
            oracle(&attested)
        );
    }

    #[test]
    fn two_different_attestations_for_one_batch_are_refused() {
        let versions = publication();
        let mut other = statement("batch-alpha-1");
        other.predicate.batch.gate_run_id = "gate-2".to_owned();
        let claims = vec![attestation("batch-alpha-1"), project_batch(&other).unwrap()];
        let error = attested_only(&versions, &claims).unwrap_err();
        assert!(error.to_string().contains("two different attestations"));
    }

    #[test]
    fn an_attestation_whose_subject_does_not_recompute_is_refused() {
        let versions = publication();
        let mut claim = attestation("batch-alpha-1");
        claim.subject = digest(9);
        let error = attested_only(&versions, &[claim]).unwrap_err();
        assert!(error.to_string().contains("not the sha256 of its body"));
    }

    #[test]
    fn an_attestation_whose_batch_id_disagrees_with_its_body_is_refused() {
        let versions = publication();
        let mut claim = attestation("batch-alpha-1");
        claim.batch_id = "batch-beta-1".to_owned();
        let error = attested_only(&versions, &[claim]).unwrap_err();
        assert!(error.to_string().contains("its statement is for batch"));
    }

    #[test]
    fn an_attestation_with_a_tampered_body_is_refused() {
        let versions = publication();
        let mut claim = attestation("batch-alpha-1");
        claim.body.push(b' ');
        assert!(attested_only(&versions, &[claim]).is_err());
    }

    #[test]
    fn an_attestation_of_another_kind_is_refused() {
        let versions = publication();
        let mut claim = attestation("batch-alpha-1");
        claim.kind = "tag.set".to_owned();
        assert!(attested_only(&versions, &[claim]).is_err());
    }

    #[test]
    fn a_generated_by_claim_for_another_version_is_refused() {
        let mut versions = publication();
        let other = versions[1].key.clone();
        for claim in &mut versions[0].document.claims {
            if claim.name == WAS_GENERATED_BY {
                claim.value = serde_json::json!({
                    "batch_id": "batch-alpha-1",
                    "file_id": other.file_id,
                    "version_seq": other.version_seq,
                })
                .to_string();
            }
        }
        let error = attested_only(&versions, &[attestation("batch-alpha-1")]).unwrap_err();
        assert!(error.to_string().contains("claim for"));
    }

    #[test]
    fn two_generated_by_claims_on_one_version_are_refused() {
        let mut versions = publication();
        let duplicate = versions[0]
            .document
            .claims
            .iter()
            .find(|claim| claim.name == WAS_GENERATED_BY)
            .unwrap()
            .clone();
        versions[0].document.claims.push(duplicate);
        let error = attested_only(&versions, &[]).unwrap_err();
        assert!(error.to_string().contains("more than one"));
    }

    #[test]
    fn an_unreadable_generated_by_value_is_refused() {
        let mut versions = publication();
        for claim in &mut versions[0].document.claims {
            if claim.name == WAS_GENERATED_BY {
                claim.value = "not json".to_owned();
            }
        }
        let error = attested_only(&versions, &[]).unwrap_err();
        assert!(error.to_string().contains("unreadable"));
    }

    proptest! {
        #[test]
        fn any_attested_subset_matches_the_row_oracle(mask in prop::collection::vec(any::<bool>(), 7)) {
            let versions = publication();
            let attested: BTreeSet<String> = batch_ids()
                .into_iter()
                .zip(mask)
                .filter_map(|(id, keep)| keep.then_some(id))
                .collect();
            let claims: Vec<_> = attested.iter().map(|id| attestation(id)).collect();
            let selected = attested_only(&versions, &claims).unwrap();
            prop_assert_eq!(keys(&selected), oracle(&attested));
        }
    }
}
