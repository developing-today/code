//! Deterministic planning and execution core for the classification-only
//! Atlas sweep (#12405). Storage adapters expose classifications and
//! evidence references only; this module has no embedding/vector API.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::classification::{
    ClassificationDisposition, ClassificationProvenance, ClassificationRecord,
};
use crate::error::{Error, Result};
use crate::filters::Sensitivity;

pub const DEFAULT_WINDOW_SECONDS: i64 = 90 * 24 * 60 * 60;
pub const QUARTER_WINDOW_SECONDS: i64 = 92 * 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelatednessWindow {
    pub start_at: i64,
    pub end_at: i64,
    /// Required when the requested interval exceeds the default window.
    pub rationale: Option<String>,
}

impl RelatednessWindow {
    pub fn default_at(end_at: i64) -> Self {
        Self {
            start_at: end_at.saturating_sub(DEFAULT_WINDOW_SECONDS),
            end_at,
            rationale: None,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.start_at > self.end_at {
            return Err(Error::fatal(
                "classification sweep: window start is after end",
            ));
        }
        let width = self.end_at.saturating_sub(self.start_at);
        if width > DEFAULT_WINDOW_SECONDS && self.rationale.as_deref().is_none_or(str::is_empty) {
            return Err(Error::fatal(
                "classification sweep: widening the 90-day window requires a rationale",
            ));
        }
        if width > QUARTER_WINDOW_SECONDS {
            return Err(Error::fatal(
                "classification sweep: relatedness window may not exceed one quarter",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub embed_id: String,
    pub project: String,
    pub section: Option<String>,
    pub captured_at: i64,
    pub kind: String,
    pub source_path: Option<String>,
    pub current_sensitivity: Option<Sensitivity>,
    pub has_classification_provenance: bool,
}

impl Candidate {
    /// Existing explicit decisions are immutable to the common sweep.
    /// A locked default without provenance is still eligible for review.
    pub fn needs_classification(&self) -> bool {
        self.current_sensitivity.is_none()
            || (self.current_sensitivity == Some(Sensitivity::SecretBearing)
                && !self.has_classification_provenance)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceReference {
    pub id: String,
    pub project: String,
    pub section: Option<String>,
    pub occurred_at: i64,
}

/// Context is represented only by references; linked content is never
/// copied into classifier input by this runner.
pub fn related_context<'a>(
    candidate: &Candidate,
    references: &'a [EvidenceReference],
    window: &RelatednessWindow,
) -> Vec<&'a EvidenceReference> {
    let mut selected: Vec<_> = references
        .iter()
        .filter(|reference| {
            reference.project == candidate.project
                && candidate
                    .section
                    .as_ref()
                    .is_none_or(|section| reference.section.as_ref() == Some(section))
                && (window.start_at..=window.end_at).contains(&reference.occurred_at)
        })
        .collect();
    selected.sort_by(|a, b| (a.occurred_at, &a.id).cmp(&(b.occurred_at, &b.id)));
    selected
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assessment {
    Supported {
        sensitivity: Sensitivity,
        disposition: Option<ClassificationDisposition>,
        evidence_refs: Vec<String>,
        rationale: String,
    },
    Unresolved {
        reason: String,
    },
    Contradictory {
        reason: String,
    },
}

pub trait Classifier {
    fn assess(&self, candidate: &Candidate, context: &[&EvidenceReference]) -> Assessment;
}

/// The storage seam intentionally permits classification writes only.
/// Implementations must upsert by embed id and must not rewrite content,
/// chunks, embeddings, or vectors.
pub trait ClassificationStore {
    fn candidates(&self) -> Result<Vec<Candidate>>;
    fn evidence_references(&self, window: &RelatednessWindow) -> Result<Vec<EvidenceReference>>;
    fn write_classification(&mut self, record: &ClassificationRecord) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepRequest {
    pub run_id: String,
    pub decided_at: String,
    pub window: RelatednessWindow,
    pub current_schema_version: u64,
    pub actor: String,
    pub method: String,
    pub method_version: String,
    pub model_id: String,
    pub model_version: String,
    pub prompt_config_hash: String,
    pub config_version: String,
    pub dry_run: bool,
    /// IDs already fully handled by a prior invocation of this run.
    pub completed_embed_ids: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateOutcome {
    pub embed_id: String,
    pub outcome: String,
    pub proposed_sensitivity: String,
    pub rationale: Option<String>,
    pub reason: Option<String>,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SweepManifest {
    pub run_id: String,
    pub dry_run: bool,
    pub window_start: i64,
    pub window_end: i64,
    pub schema_version: u64,
    pub actor: String,
    pub method: String,
    pub method_version: String,
    pub model_id: String,
    pub model_version: String,
    pub prompt_config_hash: String,
    pub config_version: String,
    pub candidates_seen: usize,
    pub already_classified: usize,
    pub resumed: usize,
    pub resumed_embed_ids: Vec<String>,
    pub outcomes: Vec<CandidateOutcome>,
}

impl SweepManifest {
    /// Committed outcomes are completed checkpoints, including unresolved
    /// items retained at the locked default. Dry-run proposals are not
    /// checkpoints because no classification write occurred.
    pub fn completed_embed_ids(&self) -> BTreeSet<String> {
        let mut completed: BTreeSet<_> = self.resumed_embed_ids.iter().cloned().collect();
        if !self.dry_run {
            completed.extend(self.outcomes.iter().map(|outcome| outcome.embed_id.clone()));
        }
        completed
    }
}

pub fn run_sweep(
    store: &mut impl ClassificationStore,
    classifier: &impl Classifier,
    request: &SweepRequest,
) -> Result<SweepManifest> {
    request.window.validate()?;
    if request.decided_at.trim().is_empty() {
        return Err(Error::fatal(
            "classification sweep: decision timestamp must not be empty",
        ));
    }
    if [
        &request.run_id,
        &request.actor,
        &request.method,
        &request.method_version,
        &request.model_id,
        &request.model_version,
        &request.prompt_config_hash,
        &request.config_version,
    ]
    .iter()
    .any(|value| value.trim().is_empty())
    {
        return Err(Error::fatal(
            "classification sweep: run and classifier provenance fields must not be empty",
        ));
    }
    let mut candidates = store.candidates()?;
    candidates.sort_by(|a, b| a.embed_id.cmp(&b.embed_id));
    if candidates
        .windows(2)
        .any(|pair| pair[0].embed_id == pair[1].embed_id)
    {
        return Err(Error::fatal(
            "classification sweep: candidate source returned duplicate embed ids",
        ));
    }
    let candidates_seen = candidates.len();
    let resumed = candidates
        .iter()
        .filter(|candidate| request.completed_embed_ids.contains(&candidate.embed_id))
        .count();
    let already_classified = candidates
        .iter()
        .filter(|candidate| {
            !request.completed_embed_ids.contains(&candidate.embed_id)
                && !candidate.needs_classification()
        })
        .count();
    let references = store.evidence_references(&request.window)?;
    let mut outcomes = Vec::new();

    for candidate in candidates {
        if request.completed_embed_ids.contains(&candidate.embed_id)
            || !candidate.needs_classification()
        {
            continue;
        }
        let context = related_context(&candidate, &references, &request.window);
        let assessment = if context.is_empty() {
            Assessment::Unresolved {
                reason: "no_classification_context".to_string(),
            }
        } else {
            classifier.assess(&candidate, &context)
        };
        let (sensitivity, disposition, evidence_refs, outcome, rationale, reason) = match assessment
        {
            Assessment::Supported {
                sensitivity,
                disposition,
                evidence_refs,
                rationale,
            } if !evidence_refs.is_empty() => (
                sensitivity,
                disposition,
                evidence_refs,
                "proposed",
                Some(rationale),
                None,
            ),
            Assessment::Supported { .. } => (
                Sensitivity::SecretBearing,
                None,
                Vec::new(),
                "unresolved",
                None,
                Some("supported assessment contained no evidence references".to_string()),
            ),
            Assessment::Unresolved { reason } => (
                Sensitivity::SecretBearing,
                None,
                Vec::new(),
                "unresolved",
                None,
                Some(reason),
            ),
            Assessment::Contradictory { reason } => (
                Sensitivity::SecretBearing,
                None,
                Vec::new(),
                "unresolved",
                None,
                Some(format!("contradictory_evidence: {reason}")),
            ),
        };
        let cited = evidence_refs.iter().collect::<BTreeSet<_>>();
        let valid_context_ids = context
            .iter()
            .map(|reference| &reference.id)
            .collect::<BTreeSet<_>>();
        let invalid_citation = cited.iter().any(|id| !valid_context_ids.contains(*id));
        let (sensitivity, outcome, rationale, reason, evidence_refs) =
            if invalid_citation && outcome == "proposed" {
                (
                    Sensitivity::SecretBearing,
                    "unresolved",
                    None,
                    Some("assessment cited evidence outside the bounded context".to_string()),
                    Vec::new(),
                )
            } else {
                (
                    sensitivity,
                    outcome,
                    rationale,
                    reason,
                    cited.into_iter().cloned().collect::<Vec<_>>(),
                )
            };
        let record = ClassificationRecord {
            schema_version: request.current_schema_version,
            embed_id: candidate.embed_id.clone(),
            sensitivity,
            disposition,
            provenance: ClassificationProvenance {
                actor: request.actor.clone(),
                decided_at: request.decided_at.clone(),
                method: request.method.clone(),
                method_version: request.method_version.clone(),
                config_version: request.config_version.clone(),
                evidence_refs: evidence_refs.clone(),
                previous_decision_id: None,
                extras: serde_json::Map::from_iter([
                    ("model_id".into(), request.model_id.clone().into()),
                    ("model_version".into(), request.model_version.clone().into()),
                    (
                        "prompt_config_hash".into(),
                        request.prompt_config_hash.clone().into(),
                    ),
                ]),
            },
            extras: Default::default(),
        };
        if !request.dry_run {
            store.write_classification(&record)?;
        }
        outcomes.push(CandidateOutcome {
            embed_id: candidate.embed_id,
            outcome: outcome.to_string(),
            proposed_sensitivity: sensitivity.name().to_string(),
            rationale,
            reason,
            evidence_refs,
        });
    }

    Ok(SweepManifest {
        run_id: request.run_id.clone(),
        dry_run: request.dry_run,
        window_start: request.window.start_at,
        window_end: request.window.end_at,
        schema_version: request.current_schema_version,
        actor: request.actor.clone(),
        method: request.method.clone(),
        method_version: request.method_version.clone(),
        model_id: request.model_id.clone(),
        model_version: request.model_version.clone(),
        prompt_config_hash: request.prompt_config_hash.clone(),
        config_version: request.config_version.clone(),
        candidates_seen,
        already_classified,
        resumed,
        resumed_embed_ids: request.completed_embed_ids.iter().cloned().collect(),
        outcomes,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaSectionChange {
    pub section: String,
    pub changed: bool,
    pub newly_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaReviewPlan {
    pub from_schema_version: u64,
    pub to_schema_version: u64,
    pub model_id: String,
    pub model_version: String,
    pub prompt_config_hash: String,
    pub breaking_change_batch: bool,
    pub sections_examined: Vec<String>,
    pub sections_skipped: Vec<String>,
}

/// Default schema review is section-scoped. The explicitly authorized
/// breaking-change batch is the sole full-reclassification exception.
pub fn sections_for_review(
    changes: &[SchemaSectionChange],
    breaking_change_batch: bool,
) -> (Vec<String>, Vec<String>) {
    let mut examined = BTreeSet::new();
    let mut skipped = BTreeSet::new();
    for change in changes {
        if breaking_change_batch || change.changed || change.newly_required {
            examined.insert(change.section.clone());
            skipped.remove(&change.section);
        } else if !examined.contains(&change.section) {
            skipped.insert(change.section.clone());
        }
    }
    (
        examined.into_iter().collect(),
        skipped.into_iter().collect(),
    )
}

pub fn plan_schema_review(
    from_schema_version: u64,
    to_schema_version: u64,
    model_id: &str,
    model_version: &str,
    prompt_config_hash: &str,
    changes: &[SchemaSectionChange],
    breaking_change_batch: bool,
) -> Result<SchemaReviewPlan> {
    if from_schema_version >= to_schema_version {
        return Err(Error::fatal(
            "classification schema review: target version must be newer",
        ));
    }
    if model_id.is_empty() || model_version.is_empty() || prompt_config_hash.is_empty() {
        return Err(Error::fatal(
            "classification schema review: model and prompt provenance are required",
        ));
    }
    let (sections_examined, sections_skipped) = sections_for_review(changes, breaking_change_batch);
    Ok(SchemaReviewPlan {
        from_schema_version,
        to_schema_version,
        model_id: model_id.to_string(),
        model_version: model_version.to_string(),
        prompt_config_hash: prompt_config_hash.to_string(),
        breaking_change_batch,
        sections_examined,
        sections_skipped,
    })
}

/// Counts are emitted in stable order so the same inputs produce the
/// same manifest summary regardless of storage iteration order.
pub fn outcome_counts(outcomes: &[CandidateOutcome]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for outcome in outcomes {
        *counts.entry(outcome.outcome.clone()).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[derive(Default)]
    struct MemoryStore {
        candidates: Vec<Candidate>,
        references: Vec<EvidenceReference>,
        records: BTreeMap<String, ClassificationRecord>,
        vectors: BTreeMap<String, Vec<u8>>,
    }

    impl ClassificationStore for MemoryStore {
        fn candidates(&self) -> Result<Vec<Candidate>> {
            Ok(self.candidates.clone())
        }

        fn evidence_references(&self, _: &RelatednessWindow) -> Result<Vec<EvidenceReference>> {
            Ok(self.references.clone())
        }

        fn write_classification(&mut self, record: &ClassificationRecord) -> Result<()> {
            self.records.insert(record.embed_id.clone(), record.clone());
            Ok(())
        }
    }

    struct EvidenceClassifier;

    impl Classifier for EvidenceClassifier {
        fn assess(&self, _: &Candidate, context: &[&EvidenceReference]) -> Assessment {
            Assessment::Supported {
                sensitivity: Sensitivity::Shareable,
                disposition: None,
                evidence_refs: context
                    .iter()
                    .map(|reference| reference.id.clone())
                    .collect(),
                rationale: "supported by linked project evidence".to_string(),
            }
        }
    }

    fn request(dry_run: bool) -> SweepRequest {
        SweepRequest {
            run_id: "run-1".to_string(),
            decided_at: "2026-11-22T15:00:00Z".to_string(),
            window: RelatednessWindow::default_at(1_000_000),
            current_schema_version: 2,
            actor: "classifier/test".to_string(),
            method: "local-reviewer".to_string(),
            method_version: "1".to_string(),
            model_id: "local/model".to_string(),
            model_version: "3".to_string(),
            prompt_config_hash: "sha256:prompt".to_string(),
            config_version: "sha256:test".to_string(),
            dry_run,
            completed_embed_ids: BTreeSet::new(),
        }
    }

    fn candidate(embed_id: &str, sensitivity: Option<Sensitivity>, provenance: bool) -> Candidate {
        Candidate {
            embed_id: embed_id.to_string(),
            project: "code".to_string(),
            section: Some("atlas".to_string()),
            captured_at: 1_000_000,
            kind: "design".to_string(),
            source_path: None,
            current_sensitivity: sensitivity,
            has_classification_provenance: provenance,
        }
    }

    fn reference(id: &str, project: &str, section: &str, occurred_at: i64) -> EvidenceReference {
        EvidenceReference {
            id: id.to_string(),
            project: project.to_string(),
            section: Some(section.to_string()),
            occurred_at,
        }
    }

    #[test]
    fn dry_run_selects_only_unclassified_and_records_bounded_evidence_without_writes() {
        let mut store = MemoryStore {
            candidates: vec![
                candidate("a", None, false),
                candidate("b", Some(Sensitivity::Personal), true),
                candidate("c", Some(Sensitivity::SecretBearing), false),
            ],
            references: vec![reference("issue:1", "code", "atlas", 999_999)],
            vectors: BTreeMap::from([
                ("a".to_string(), vec![0, 1, 255]),
                ("b".to_string(), vec![8, 9]),
                ("c".to_string(), vec![42]),
            ]),
            ..MemoryStore::default()
        };
        let vectors_before = store.vectors.clone();
        let manifest = run_sweep(&mut store, &EvidenceClassifier, &request(true)).unwrap();

        assert_eq!(manifest.candidates_seen, 3);
        assert_eq!(manifest.already_classified, 1);
        assert_eq!(manifest.outcomes.len(), 2);
        assert_eq!(manifest.outcomes[0].embed_id, "a");
        assert_eq!(manifest.outcomes[1].embed_id, "c");
        assert!(
            manifest
                .outcomes
                .iter()
                .all(|row| row.proposed_sensitivity == "shareable")
        );
        assert!(
            manifest
                .outcomes
                .iter()
                .all(|row| row.evidence_refs == ["issue:1"])
        );
        assert!(store.records.is_empty(), "dry-run must not persist records");
        assert_eq!(
            store.vectors, vectors_before,
            "classification path cannot mutate vectors"
        );
        assert_eq!(
            manifest.window_end - manifest.window_start,
            DEFAULT_WINDOW_SECONDS
        );
        let encoded = serde_json::to_vec(&manifest).unwrap();
        let decoded: SweepManifest = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, manifest, "manifest is a durable resume checkpoint");
        assert_eq!(
            decoded.completed_embed_ids(),
            BTreeSet::new(),
            "dry-run proposals must not suppress a later commit"
        );
    }

    #[test]
    fn absent_or_contradictory_evidence_stays_locked_and_is_resumable() {
        struct NoEvidence;
        impl Classifier for NoEvidence {
            fn assess(&self, _: &Candidate, _: &[&EvidenceReference]) -> Assessment {
                Assessment::Contradictory {
                    reason: "conflicting links".to_string(),
                }
            }
        }

        let mut store = MemoryStore {
            candidates: vec![candidate("a", None, false), candidate("b", None, false), {
                let mut candidate = candidate("c", None, false);
                candidate.project = "another-project".to_string();
                candidate
            }],
            references: vec![reference("issue:1", "code", "atlas", 999_999)],
            ..MemoryStore::default()
        };
        let mut req = request(false);
        req.completed_embed_ids.insert("a".to_string());
        let manifest = run_sweep(&mut store, &NoEvidence, &req).unwrap();

        assert_eq!(manifest.resumed, 1);
        assert_eq!(manifest.outcomes.len(), 2);
        let record = store.records.get("b").unwrap();
        assert_eq!(record.sensitivity, Sensitivity::SecretBearing);
        assert_eq!(manifest.outcomes[0].outcome, "unresolved");
        assert_eq!(manifest.outcomes[0].proposed_sensitivity, "secret-bearing");
        assert!(
            manifest.outcomes[0]
                .reason
                .as_deref()
                .unwrap()
                .contains("contradictory_evidence")
        );
        assert_eq!(manifest.outcomes[1].embed_id, "c");
        assert_eq!(
            manifest.outcomes[1].reason.as_deref(),
            Some("no_classification_context")
        );
        assert_eq!(
            manifest.completed_embed_ids(),
            BTreeSet::from(["a".into(), "b".into(), "c".into()])
        );
    }

    #[test]
    fn committed_classification_only_update_preserves_vectors_and_counts_results() {
        let mut store = MemoryStore {
            candidates: vec![
                candidate("new", None, false),
                candidate("existing", Some(Sensitivity::Personal), true),
            ],
            references: vec![reference("issue:1", "code", "atlas", 999_999)],
            vectors: BTreeMap::from([
                ("new".to_string(), vec![1, 2, 3]),
                ("existing".to_string(), vec![4, 5, 6]),
            ]),
            ..MemoryStore::default()
        };
        let vectors_before = store.vectors.clone();
        let manifest = run_sweep(&mut store, &EvidenceClassifier, &request(false)).unwrap();

        assert_eq!(store.records.len(), 1);
        assert!(store.records.contains_key("new"));
        assert_eq!(
            store.records["new"].provenance.extras["model_id"],
            "local/model"
        );
        assert_eq!(
            store.records["new"].provenance.extras["prompt_config_hash"],
            "sha256:prompt"
        );
        assert_eq!(manifest.already_classified, 1);
        assert_eq!(outcome_counts(&manifest.outcomes).get("proposed"), Some(&1));
        assert_eq!(
            store.vectors, vectors_before,
            "classification-only writes must not re-embed"
        );
    }

    #[test]
    fn context_is_bounded_to_project_section_and_window() {
        let candidate = candidate("a", None, false);
        let window = RelatednessWindow::default_at(1_000_000);
        let refs = vec![
            reference("right", "code", "atlas", 999_999),
            reference("other-project", "elsewhere", "atlas", 999_999),
            reference("other-section", "code", "runtime", 999_999),
            reference("too-old", "code", "atlas", window.start_at - 1),
        ];
        let selected = related_context(&candidate, &refs, &window);
        assert_eq!(
            selected.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["right"]
        );
    }

    #[test]
    fn schema_review_skips_unchanged_sections_except_explicit_batch() {
        let changes = vec![
            SchemaSectionChange {
                section: "A".to_string(),
                changed: true,
                newly_required: false,
            },
            SchemaSectionChange {
                section: "B".to_string(),
                changed: false,
                newly_required: false,
            },
        ];
        assert_eq!(
            sections_for_review(&changes, false),
            (vec!["A".into()], vec!["B".into()])
        );
        assert_eq!(
            sections_for_review(&changes, true),
            (vec!["A".into(), "B".into()], vec![])
        );
        let duplicate_section = vec![
            SchemaSectionChange {
                section: "A".to_string(),
                changed: true,
                newly_required: false,
            },
            SchemaSectionChange {
                section: "A".to_string(),
                changed: false,
                newly_required: false,
            },
        ];
        assert_eq!(
            sections_for_review(&duplicate_section, false),
            (vec!["A".into()], vec![]),
            "any changed record must take precedence for a duplicated section"
        );
    }

    #[test]
    fn wider_window_needs_rationale_and_cannot_exceed_quarter() {
        let mut window = RelatednessWindow {
            start_at: 0,
            end_at: DEFAULT_WINDOW_SECONDS + 1,
            rationale: None,
        };
        assert!(window.validate().is_err());
        window.rationale = Some("sparse section evidence".to_string());
        assert!(window.validate().is_ok());
        window.start_at = window.end_at - QUARTER_WINDOW_SECONDS - 1;
        assert!(window.validate().is_err());
    }

    #[test]
    fn assessment_cannot_cite_evidence_outside_the_bounded_context() {
        struct OutOfScopeClassifier;
        impl Classifier for OutOfScopeClassifier {
            fn assess(&self, _: &Candidate, _: &[&EvidenceReference]) -> Assessment {
                Assessment::Supported {
                    sensitivity: Sensitivity::Shareable,
                    disposition: None,
                    evidence_refs: vec!["issue:outside-window".to_string()],
                    rationale: "unsupported citation".to_string(),
                }
            }
        }

        let mut store = MemoryStore {
            candidates: vec![candidate("a", None, false)],
            references: vec![reference("issue:in-window", "code", "atlas", 999_999)],
            ..MemoryStore::default()
        };
        let manifest = run_sweep(&mut store, &OutOfScopeClassifier, &request(false)).unwrap();

        assert_eq!(store.records["a"].sensitivity, Sensitivity::SecretBearing);
        assert_eq!(manifest.outcomes[0].outcome, "unresolved");
        assert_eq!(
            manifest.outcomes[0].reason.as_deref(),
            Some("assessment cited evidence outside the bounded context")
        );
        assert!(manifest.outcomes[0].evidence_refs.is_empty());
    }
}
