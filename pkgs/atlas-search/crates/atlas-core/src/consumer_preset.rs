//! The consumer view presets over an Atlas publication (#6062 design §4
//! and §9.4; issues #11185 and #13766).
//!
//! The projector publishes everything (§4). A consumer then selects, from
//! one pinned publication, the slice its audience counts. Three presets
//! exist, one per audience the design names:
//!
//! - [`heads_only`]: each file's current head, the tip of its note chain.
//!   Superseded heads and non-head backfills are left out, and a file that
//!   has only backfills has no head.
//! - [`attested_only`]: the versions whose batch has a published
//!   `attestation.claim`.
//! - [`full_history`]: every published version, heads, superseded heads
//!   and backfills alike.
//!
//! None of them adds view machinery or filters the publication itself.
//! Whether the pipeline key's earlier publications count for a reader at
//! all is that reader's writer-set choice (`Entry::Key { include_history }`
//! in `id-events`' `idset`, #5545), which is consumed unchanged and not
//! redefined here: these presets choose within what counts.
//!
//! Every input is checked before it is trusted, and any inconsistency
//! refuses the whole selection rather than returning a partial one.
//!
//! ## Heads and history
//!
//! A version is a head transition when the publication gives it a note
//! operation. A file's first transition is a `note.create`; each later one
//! is a `note.revise` naming that create and, as its one parent, the
//! transition it replaces. The current head is the transition no other
//! transition replaces. [`heads_only`] and [`full_history`] both check the
//! chains first:
//!
//! - no two versions share a key;
//! - a note's content names its own version's document (hash, media type
//!   and size);
//! - each file with transitions has exactly one `note.create`, and every
//!   `note.revise` names that create and one parent in the same file;
//! - no transition is replaced twice, and walking back from the one tip
//!   reaches every transition of the file.
//!
//! ## Attested
//!
//! The attested predicate is the one recorded on #13766: a published
//! version is attested iff the publication contains a `tag.set` claim
//! named `atlas.wasGeneratedBy` whose subject is that version's document
//! and whose value names an `attestation.claim` present in the same
//! publication. A rejected batch writes no corpus rows, so no version can
//! name a rejected batch's attestation and the preset makes no outcome
//! check.
//!
//! Until events are signed, both sides of the hop are symbolic: the
//! version's `atlas.wasGeneratedBy` value names its batch by `batch_id`
//! ([`project_version`](crate::version_projection::project_version)), and
//! each [`ProjectedClaim`] carries the `batch_id` of the statement whose
//! canonical bytes are its body ([`project_batch`](crate::id_projection::project_batch)).
//! The preset joins on that `batch_id`. Its checks:
//!
//! - an attestation claim must have the `attestation.claim` kind, a body
//!   that decodes to a canonical statement, a subject that recomputes as
//!   the body's sha256, and a `batch_id` equal to the statement's;
//! - two different attestations may not claim the same batch;
//! - a version carries at most one `atlas.wasGeneratedBy` claim, and its
//!   value must name that version's own `file_id` and `version_seq`.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::attestation::BatchStatement;
use crate::classify::sha256_hex;
use crate::error::{Error, Result};
use crate::id_projection::{CLAIM_KIND, ProjectedClaim};
use crate::version_projection::{
    AtlasVersionKey, ProjectedNote, ProjectedNoteEventRef, ProjectedSelectedVersion,
    ProjectedSubject,
};

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

fn note_key(reference: &ProjectedNoteEventRef) -> AtlasVersionKey {
    AtlasVersionKey {
        file_id: reference.file_id.clone(),
        version_seq: reference.version_seq,
    }
}

/// Check the publication's keys and note chains, and return the index of
/// each file's current head, keyed by `file_id`. A file with no head
/// transitions has no entry.
fn current_heads(versions: &[ProjectedSelectedVersion]) -> Result<BTreeMap<&str, usize>> {
    let mut seen: BTreeSet<&AtlasVersionKey> = BTreeSet::new();
    for version in versions {
        if !seen.insert(&version.key) {
            return Err(refuse(format!(
                "version {:?} appears more than once",
                version.key
            )));
        }
    }

    // Per file: its create, and each transition's replaced parent.
    let mut creates: BTreeMap<&str, usize> = BTreeMap::new();
    let mut transitions: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    let mut parent_of: BTreeMap<usize, AtlasVersionKey> = BTreeMap::new();
    for (index, version) in versions.iter().enumerate() {
        let Some(note) = &version.note else {
            continue;
        };
        let content = match note {
            ProjectedNote::Create { content } | ProjectedNote::Revise { content, .. } => content,
        };
        let document = &version.document;
        if content.content != document.content_hash
            || content.media_type != document.media_type
            || content.size != document.size
        {
            return Err(refuse(format!(
                "version {:?} has a note whose content is not its own document",
                version.key
            )));
        }
        let file = version.key.file_id.as_str();
        transitions.entry(file).or_default().push(index);
        match note {
            ProjectedNote::Create { .. } => {
                if creates.insert(file, index).is_some() {
                    return Err(refuse(format!(
                        "file {file:?} has more than one note.create"
                    )));
                }
            }
            ProjectedNote::Revise {
                note: create,
                parents,
                ..
            } => {
                if create.file_id != file {
                    return Err(refuse(format!(
                        "version {:?} revises the note of file {:?}",
                        version.key, create.file_id
                    )));
                }
                let [parent] = parents.as_slice() else {
                    return Err(refuse(format!(
                        "version {:?} has {} parents, not one",
                        version.key,
                        parents.len()
                    )));
                };
                if parent.file_id != file {
                    return Err(refuse(format!(
                        "version {:?} replaces a head of file {:?}",
                        version.key, parent.file_id
                    )));
                }
                parent_of.insert(index, note_key(parent));
            }
        }
    }

    let index_of: BTreeMap<&AtlasVersionKey, usize> = versions
        .iter()
        .enumerate()
        .map(|(index, version)| (&version.key, index))
        .collect();
    let mut heads = BTreeMap::new();
    for (file, members) in &transitions {
        let Some(&create) = creates.get(file) else {
            return Err(refuse(format!(
                "file {file:?} has note revisions but no note.create"
            )));
        };
        let mut replaced: BTreeSet<usize> = BTreeSet::new();
        for member in members {
            let Some(parent) = parent_of.get(member) else {
                continue;
            };
            let named = match versions[*member].note.as_ref() {
                Some(ProjectedNote::Revise { note, .. }) => note_key(note),
                _ => continue,
            };
            if named != versions[create].key {
                return Err(refuse(format!(
                    "version {:?} revises note {named:?}, not the file's create {:?}",
                    versions[*member].key, versions[create].key
                )));
            }
            let parent_index = match index_of.get(parent) {
                Some(&parent_index) if versions[parent_index].note.is_some() => parent_index,
                _ => {
                    return Err(refuse(format!(
                        "version {:?} replaces {parent:?}, which is not a published head transition",
                        versions[*member].key
                    )));
                }
            };
            if !replaced.insert(parent_index) {
                return Err(refuse(format!(
                    "head {parent:?} of file {file:?} is replaced more than once"
                )));
            }
        }
        let tips: Vec<usize> = members
            .iter()
            .copied()
            .filter(|member| !replaced.contains(member))
            .collect();
        let [tip] = tips.as_slice() else {
            return Err(refuse(format!(
                "file {file:?} has {} current heads, not one",
                tips.len()
            )));
        };
        // Walk back from the tip: a linear chain reaches every transition
        // of the file and ends at its create.
        let mut walked = 1;
        let mut at = *tip;
        while let Some(parent) = parent_of.get(&at) {
            at = index_of[parent];
            walked += 1;
            if walked > members.len() {
                break;
            }
        }
        if at != create || walked != members.len() {
            return Err(refuse(format!(
                "file {file:?} has head transitions that its note chain does not reach"
            )));
        }
        heads.insert(*file, *tip);
    }
    Ok(heads)
}

/// The heads-only preset: each file's current head, the tip of its note
/// chain, in input order.
///
/// # Errors
///
/// [`Error::Fatal`] when two versions share a key or a note chain is
/// malformed: a note whose content is not its own document, a file with no
/// or two `note.create`s, a revision naming another file or another
/// create, a parent that is not a published head transition, a head
/// replaced twice, or transitions the chain does not reach. Nothing is
/// selected when any input is refused.
pub fn heads_only(versions: &[ProjectedSelectedVersion]) -> Result<Vec<&ProjectedSelectedVersion>> {
    let tips: BTreeSet<usize> = current_heads(versions)?.into_values().collect();
    Ok(versions
        .iter()
        .enumerate()
        .filter(|(index, _)| tips.contains(index))
        .map(|(_, version)| version)
        .collect())
}

/// The full-history preset: every published version, in input order,
/// once the publication's keys and note chains check out.
///
/// # Errors
///
/// The same refusals as [`heads_only`]: a full-history reader is handed a
/// publication whose history is coherent, or nothing.
pub fn full_history(
    versions: &[ProjectedSelectedVersion],
) -> Result<Vec<&ProjectedSelectedVersion>> {
    current_heads(versions)?;
    Ok(versions.iter().collect())
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

    /// Oracle: each file's current head from the Atlas rows alone — the
    /// largest `version_seq` among the file's selected head transitions,
    /// in Atlas key order.
    fn heads_oracle(head_keys: &[AtlasVersionKey]) -> Vec<AtlasVersionKey> {
        let mut tips: BTreeMap<&str, u64> = BTreeMap::new();
        for key in head_keys {
            let tip = tips.entry(key.file_id.as_str()).or_default();
            *tip = (*tip).max(key.version_seq);
        }
        tips.into_iter()
            .map(|(file_id, version_seq)| AtlasVersionKey {
                file_id: file_id.to_owned(),
                version_seq,
            })
            .collect()
    }

    fn all_keys() -> Vec<AtlasVersionKey> {
        let mut keys: Vec<_> = fixture_rows()
            .into_iter()
            .map(|row| AtlasVersionKey {
                file_id: row.file_id,
                version_seq: row.version_seq,
            })
            .collect();
        keys.sort();
        keys
    }

    fn key(file_id: &str, version_seq: u64) -> AtlasVersionKey {
        AtlasVersionKey {
            file_id: file_id.to_owned(),
            version_seq,
        }
    }

    fn index(versions: &[ProjectedSelectedVersion], wanted: &AtlasVersionKey) -> usize {
        versions
            .iter()
            .position(|version| &version.key == wanted)
            .unwrap()
    }

    #[test]
    fn fixture_heads_only_selects_each_files_current_head() {
        let versions = publication();
        let selected = heads_only(&versions).unwrap();
        assert_eq!(
            keys(&selected),
            vec![key("file-alpha", 4), key("file-beta", 1)]
        );
        assert_eq!(keys(&selected), heads_oracle(&fixture_head_keys()));
    }

    #[test]
    fn heads_only_leaves_out_superseded_heads_and_backfills() {
        let versions = publication();
        let selected = heads_only(&versions).unwrap();
        // file-alpha@1 and @2 are superseded heads; @3 is a backfill with a
        // larger sequence than @2 that is still not a head.
        for dropped in [
            key("file-alpha", 1),
            key("file-alpha", 2),
            key("file-alpha", 3),
            key("file-beta", 2),
            key("file-backfill-only", 1),
        ] {
            assert!(selected.iter().all(|version| version.key != dropped));
        }
        assert!(selected.iter().all(|version| version.note.is_some()));
    }

    #[test]
    fn fixture_full_history_selects_every_published_version() {
        let versions = publication();
        let selected = full_history(&versions).unwrap();
        assert_eq!(keys(&selected), all_keys());
        assert!(selected.iter().any(|version| version.note.is_none()));
    }

    #[test]
    fn the_three_presets_nest_inside_full_history() {
        let versions = publication();
        let full: BTreeSet<_> = keys(&full_history(&versions).unwrap())
            .into_iter()
            .collect();
        let heads = keys(&heads_only(&versions).unwrap());
        let claims: Vec<_> = ["batch-alpha-4", "batch-beta-backfill"]
            .into_iter()
            .map(attestation)
            .collect();
        let attested = keys(&attested_only(&versions, &claims).unwrap());
        assert!(heads.iter().all(|key| full.contains(key)));
        assert!(attested.iter().all(|key| full.contains(key)));
        assert!(heads.len() < full.len());
        assert_ne!(heads, attested);
    }

    #[test]
    fn a_backfill_only_publication_has_no_heads_but_full_history() {
        let rows = fixture_rows();
        let versions = project_selected_versions(&rows, &[]).unwrap();
        assert!(heads_only(&versions).unwrap().is_empty());
        assert_eq!(full_history(&versions).unwrap().len(), versions.len());
    }

    #[test]
    fn an_empty_publication_selects_nothing() {
        assert!(heads_only(&[]).unwrap().is_empty());
        assert!(full_history(&[]).unwrap().is_empty());
    }

    #[test]
    fn a_repeated_version_is_refused() {
        let mut versions = publication();
        versions.push(versions[0].clone());
        let error = heads_only(&versions).unwrap_err();
        assert!(error.to_string().contains("more than once"));
        assert!(full_history(&versions).is_err());
    }

    #[test]
    fn a_note_for_another_document_is_refused() {
        let mut versions = publication();
        let at = index(&versions, &key("file-alpha", 4));
        versions[at].document.content_hash = digest(7);
        let error = heads_only(&versions).unwrap_err();
        assert!(error.to_string().contains("not its own document"));
        assert!(full_history(&versions).is_err());
    }

    #[test]
    fn a_revision_without_its_create_is_refused() {
        let mut versions = publication();
        let at = index(&versions, &key("file-alpha", 1));
        versions[at].note = None;
        let error = heads_only(&versions).unwrap_err();
        assert!(error.to_string().contains("no note.create"));
    }

    #[test]
    fn a_second_create_is_refused() {
        let mut versions = publication();
        let first = index(&versions, &key("file-alpha", 1));
        let later = index(&versions, &key("file-alpha", 4));
        let document = versions[later].document.clone();
        versions[later].note = versions[first].note.clone();
        if let Some(ProjectedNote::Create { content }) = &mut versions[later].note {
            content.content = document.content_hash;
            content.media_type = document.media_type;
            content.size = document.size;
        }
        let error = heads_only(&versions).unwrap_err();
        assert!(error.to_string().contains("more than one note.create"));
    }

    #[test]
    fn a_fork_with_two_current_heads_is_refused() {
        let mut versions = publication();
        let at = index(&versions, &key("file-alpha", 4));
        if let Some(ProjectedNote::Revise { parents, .. }) = &mut versions[at].note {
            parents[0].version_seq = 1;
        }
        let error = heads_only(&versions).unwrap_err();
        assert!(error.to_string().contains("replaced more than once"));
        assert!(full_history(&versions).is_err());
    }

    #[test]
    fn a_parent_that_is_a_backfill_is_refused() {
        let mut versions = publication();
        let at = index(&versions, &key("file-alpha", 4));
        if let Some(ProjectedNote::Revise { parents, .. }) = &mut versions[at].note {
            parents[0].version_seq = 3;
        }
        let error = heads_only(&versions).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("not a published head transition")
        );
    }

    #[test]
    fn a_revision_of_another_files_note_is_refused() {
        let mut versions = publication();
        let at = index(&versions, &key("file-alpha", 4));
        if let Some(ProjectedNote::Revise { note, .. }) = &mut versions[at].note {
            note.file_id = "file-beta".to_owned();
        }
        let error = heads_only(&versions).unwrap_err();
        assert!(error.to_string().contains("revises the note of file"));
    }

    #[test]
    fn a_revision_naming_a_non_create_is_refused() {
        let mut versions = publication();
        let at = index(&versions, &key("file-alpha", 4));
        if let Some(ProjectedNote::Revise { note, .. }) = &mut versions[at].note {
            note.version_seq = 2;
        }
        let error = heads_only(&versions).unwrap_err();
        assert!(error.to_string().contains("not the file's create"));
    }

    #[test]
    fn a_revision_with_two_parents_is_refused() {
        let mut versions = publication();
        let at = index(&versions, &key("file-alpha", 4));
        if let Some(ProjectedNote::Revise { parents, .. }) = &mut versions[at].note {
            let extra = parents[0].clone();
            parents.push(extra);
        }
        let error = heads_only(&versions).unwrap_err();
        assert!(error.to_string().contains("parents, not one"));
    }

    #[test]
    fn a_cycle_the_chain_does_not_reach_is_refused() {
        // alpha@2 replaces @4 and @4 replaces @2: @1 is the only tip, and
        // walking back from it never reaches the other two.
        let mut versions = publication();
        let second = index(&versions, &key("file-alpha", 2));
        if let Some(ProjectedNote::Revise { parents, .. }) = &mut versions[second].note {
            parents[0].version_seq = 4;
        }
        let error = heads_only(&versions).unwrap_err();
        assert!(error.to_string().contains("does not reach"));
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

        #[test]
        fn any_head_selection_matches_the_row_oracle(mask in prop::collection::vec(any::<bool>(), 7)) {
            let rows = fixture_rows();
            let head_keys: Vec<AtlasVersionKey> = rows
                .iter()
                .zip(mask)
                .filter_map(|(row, keep)| keep.then(|| AtlasVersionKey {
                    file_id: row.file_id.clone(),
                    version_seq: row.version_seq,
                }))
                .collect();
            let versions = project_selected_versions(&rows, &head_keys).unwrap();
            prop_assert_eq!(keys(&heads_only(&versions).unwrap()), heads_oracle(&head_keys));
            prop_assert_eq!(keys(&full_history(&versions).unwrap()), all_keys());
        }
    }
}
