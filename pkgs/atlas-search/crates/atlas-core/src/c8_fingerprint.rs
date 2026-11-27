//! C8 error-driven investigation — fingerprint core (design slice
//! C8-E, developing-today/code#9576, #12962).
//!
//! This module is the seam-independent core of C8-E: the pure types
//! and derivations the merged C8 design specifies in §4
//! (error-investigation mode) and §5 steps 1–2 and 6 (the
//! investigation recipe and its evidence bundle). It deliberately
//! contains no telemetry search and performs no live
//! commit/PR/issue joins or unpushed-commit lookups: gathering
//! consumes live interfaces owned by later C8-E slices. What lives
//! here is everything that can be computed from error strings,
//! occurrence records, and already-gathered investigation findings
//! alone:
//!
//! - [`normalize_error_string`]: the conservative normalization of
//!   design §4 — Unicode normalization (NFC) and whitespace
//!   normalization only. Case, punctuation, digits, and every other
//!   character survive untouched, so merely similar errors are
//!   never merged.
//! - [`ErrorFingerprint`]: a stable fingerprint (SHA-256 over the
//!   normalized string) recorded alongside the original string; it
//!   groups exact normalized occurrences only.
//! - [`ErrorOccurrence`] / [`group_occurrences`]: occurrence
//!   records that preserve the original string verbatim, grouped
//!   deterministically by fingerprint.
//! - [`first_observation`]: the earliest exact occurrence in a
//!   retained set, labelled per design §5 step 2 — "first observed"
//!   only when the retention floor predates it, otherwise
//!   "earliest retained observation", never "first ever".
//! - [`InvestigationEdge`] / [`EvidenceBundle`]: the step-6 record
//!   fields this core can fill — query/fingerprint, time range,
//!   evidence IDs, exact-vs-inferred edge labels, and missing-data
//!   gaps. A missing link is an edge with no target, preserved as
//!   a gap; it is never dropped and never fabricated. The bundle
//!   carries no causation claim: an investigation is evidence,
//!   not a verdict (design §5 preamble).
//! - [`CandidateChange`] / [`RejectedCandidate`] /
//!   [`UnpushedCommitCheck`] / [`InvestigationFindings`]:
//!   assembly of already-gathered findings into the full step-6
//!   record (design §5 steps 4–6) — candidate changes with
//!   exact-vs-inferred association labels, rejected candidates
//!   with their reasons, the step-5 unpushed-final-commit check
//!   exactly as an authoritative source reported it, and the
//!   next action. Gathering the findings is the live-interface
//!   slices' work; assembling them is deterministic and lives
//!   here. Absent findings stay absent: a check that was not
//!   performed is recorded as not performed, never as a negative
//!   result (design §6).
//!
//! Timestamps are RFC 3339 strings, compared lexicographically —
//! the convention of the C8 profile in [`crate::c8_profile`].

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

/// Version of the C8-E fingerprint-core record shapes
/// (additive-only changes, as in the design §2 envelope rule).
pub const C8_FINGERPRINT_SCHEMA_VERSION: u32 = 1;

/// Hash algorithm [`ErrorFingerprint`] digests are computed with.
/// Recorded on every fingerprint so a digest is self-describing
/// and a future algorithm change is an additive migration.
pub const FINGERPRINT_ALGORITHM: &str = "sha256";

/// Conservative error-string normalization (design §4): Unicode
/// normalization (NFC) and whitespace normalization only.
///
/// Whitespace normalization collapses every run of Unicode
/// whitespace to a single ASCII space and trims the ends. Nothing
/// else is rewritten: no case folding, no punctuation or digit
/// masking, no compatibility decomposition — so two errors that
/// differ in any non-whitespace character keep different
/// normalized forms and are never grouped together.
pub fn normalize_error_string(raw: &str) -> String {
    let nfc: String = raw.nfc().collect();
    let mut out = String::with_capacity(nfc.len());
    let mut pending_space = false;
    for c in nfc.chars() {
        if c.is_whitespace() {
            pending_space = true;
        } else {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.push(c);
        }
    }
    out
}

/// A stable fingerprint of an error string (design §5 step 1):
/// the SHA-256 digest of the conservatively normalized string,
/// recorded alongside that normalized string. Two occurrences
/// share a fingerprint if and only if their normalized strings
/// are identical — a fingerprint groups exact normalized
/// occurrences only.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ErrorFingerprint {
    /// Hash algorithm of `digest` ([`FINGERPRINT_ALGORITHM`]).
    pub algorithm: String,
    /// Lowercase hex digest of the normalized string's UTF-8 bytes.
    pub digest: String,
    /// The normalized string the digest covers.
    pub normalized: String,
}

impl ErrorFingerprint {
    /// Fingerprint an error string exactly as observed: the string
    /// is normalized conservatively and the digest taken over the
    /// normalized form.
    pub fn of(raw: &str) -> ErrorFingerprint {
        let normalized = normalize_error_string(raw);
        let digest: String = Sha256::digest(normalized.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        ErrorFingerprint {
            algorithm: FINGERPRINT_ALGORITHM.to_string(),
            digest,
            normalized,
        }
    }

    /// Canonical identifier of this fingerprint,
    /// `"<algorithm>:<digest>"` — the form an evidence bundle or
    /// log line cites.
    pub fn id(&self) -> String {
        format!("{}:{}", self.algorithm, self.digest)
    }
}

/// One observed occurrence of an error string in a source record
/// (design §5 step 1). The original string is preserved verbatim
/// alongside its fingerprint: normalization decides grouping,
/// never what the record says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorOccurrence {
    /// The error string exactly as observed; never rewritten.
    pub original: String,
    /// Fingerprint of `original`, derived at construction.
    pub fingerprint: ErrorFingerprint,
    /// Canonical ID of the source record (trace, log, or report)
    /// in its owning system.
    pub source_id: String,
    /// When the source observed the error (RFC 3339).
    pub observed_at: String,
}

impl ErrorOccurrence {
    pub fn new(
        original: impl Into<String>,
        source_id: impl Into<String>,
        observed_at: impl Into<String>,
    ) -> ErrorOccurrence {
        let original = original.into();
        let fingerprint = ErrorFingerprint::of(&original);
        ErrorOccurrence {
            original,
            fingerprint,
            source_id: source_id.into(),
            observed_at: observed_at.into(),
        }
    }
}

/// Occurrences sharing one fingerprint, in observation order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FingerprintGroup {
    pub fingerprint: ErrorFingerprint,
    /// The group's occurrences, sorted by `observed_at` then
    /// `source_id`; originals preserved verbatim.
    pub occurrences: Vec<ErrorOccurrence>,
}

/// Group occurrences by fingerprint (design §4/§5 step 1).
///
/// Deterministic: groups are ordered by fingerprint ID and each
/// group's occurrences by (`observed_at`, `source_id`). Grouping
/// is by the recorded fingerprint, so occurrences whose originals
/// differ only in Unicode composition or whitespace land in one
/// group, and merely similar errors never do.
pub fn group_occurrences(occurrences: &[ErrorOccurrence]) -> Vec<FingerprintGroup> {
    let mut groups: Vec<FingerprintGroup> = Vec::new();
    for occurrence in occurrences {
        match groups
            .iter_mut()
            .find(|g| g.fingerprint.id() == occurrence.fingerprint.id())
        {
            Some(group) => group.occurrences.push(occurrence.clone()),
            None => groups.push(FingerprintGroup {
                fingerprint: occurrence.fingerprint.clone(),
                occurrences: vec![occurrence.clone()],
            }),
        }
    }
    for group in &mut groups {
        group
            .occurrences
            .sort_by(|a, b| (&a.observed_at, &a.source_id).cmp(&(&b.observed_at, &b.source_id)));
    }
    groups.sort_by_key(|a| a.fingerprint.id());
    groups
}

/// Whether an earliest exact occurrence may be called the first
/// observed occurrence, or only the earliest retained one
/// (design §5 step 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FirstObservationKind {
    /// The retention floor predates the occurrence, so within the
    /// retained record it is the first observed occurrence.
    FirstObserved,
    /// The occurrence sits at or before the retention floor (or
    /// the floor is unknown): the actual first occurrence may
    /// predate retention, so this is only the earliest retained
    /// observation — never "first ever".
    EarliestRetainedObservation,
}

impl FirstObservationKind {
    /// The label an investigation record states (design §5 step 2).
    pub fn label(&self) -> &'static str {
        match self {
            FirstObservationKind::FirstObserved => "first observed",
            FirstObservationKind::EarliestRetainedObservation => "earliest retained observation",
        }
    }
}

/// The first-observation result of design §5 step 2 over a
/// retained occurrence set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirstObservation {
    /// The earliest exact occurrence, by (`observed_at`,
    /// `source_id`).
    pub occurrence: ErrorOccurrence,
    /// First-observed vs earliest-retained, per the retention
    /// floor.
    pub kind: FirstObservationKind,
    /// The retention floor the search stated (RFC 3339): the
    /// start of the retained window. Absent when the floor is
    /// unknown — which always yields
    /// [`FirstObservationKind::EarliestRetainedObservation`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_floor: Option<String>,
    /// How many exact occurrences the searched set contained.
    pub exact_occurrences: usize,
}

/// Find the first observed occurrence of `fingerprint` among
/// `occurrences` (design §5 step 2), over exact fingerprint
/// matches only.
///
/// `retention_floor` is the start of the retained window the
/// caller searched (RFC 3339), when known. The earliest match is
/// labelled [`FirstObservationKind::FirstObserved`] only when a
/// known floor strictly predates it; at or before the floor — or
/// with no floor stated — it is
/// [`FirstObservationKind::EarliestRetainedObservation`].
/// Returns `None` when the set holds no exact occurrence.
pub fn first_observation(
    occurrences: &[ErrorOccurrence],
    fingerprint: &ErrorFingerprint,
    retention_floor: Option<&str>,
) -> Option<FirstObservation> {
    let mut matches: Vec<&ErrorOccurrence> = occurrences
        .iter()
        .filter(|o| o.fingerprint.id() == fingerprint.id())
        .collect();
    if matches.is_empty() {
        return None;
    }
    matches.sort_by(|a, b| (&a.observed_at, &a.source_id).cmp(&(&b.observed_at, &b.source_id)));
    let earliest = matches[0].clone();
    let kind = match retention_floor {
        Some(floor) if floor < earliest.observed_at.as_str() => FirstObservationKind::FirstObserved,
        _ => FirstObservationKind::EarliestRetainedObservation,
    };
    Some(FirstObservation {
        occurrence: earliest,
        kind,
        retention_floor: retention_floor.map(str::to_string),
        exact_occurrences: matches.len(),
    })
}

/// Exact-vs-inferred label of an investigation edge (design §5
/// step 6). Exact edges restate an explicit source link (design
/// §3, `evidence: explicit`); inferred edges are associations the
/// investigation derived, and always carry a confidence and a
/// provenance source (§3: inferred links always carry confidence
/// and provenance).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeLabel {
    Exact,
    Inferred,
}

impl EdgeLabel {
    /// The label an investigation record states.
    pub fn as_str(&self) -> &'static str {
        match self {
            EdgeLabel::Exact => "exact",
            EdgeLabel::Inferred => "inferred",
        }
    }
}

/// One evidenced relation in an investigation (design §5 steps 4
/// and 6), e.g. commit → pull request or pull request → issue.
///
/// A missing link is represented with `to_id: None` and preserved
/// as a gap: the edge stays in the record and surfaces in the
/// bundle's gap list. A gap edge's label describes the gap report
/// itself — no target was found — never a relation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InvestigationEdge {
    /// Relation kind, e.g. `"commit_to_pull_request"`.
    pub kind: String,
    /// Canonical ID of the relation's source record.
    pub from_id: String,
    /// Canonical ID of the target record; absent when the link
    /// is missing — a preserved gap, not a dropped edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_id: Option<String>,
    pub label: EdgeLabel,
    /// Confidence of an inferred edge; always present for
    /// inferred edges, never for exact ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// Provenance of the edge: where the relation was read from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl InvestigationEdge {
    /// An exact edge: the relation is stated by an explicit
    /// source link.
    pub fn exact(
        kind: impl Into<String>,
        from_id: impl Into<String>,
        to_id: impl Into<String>,
    ) -> InvestigationEdge {
        InvestigationEdge {
            kind: kind.into(),
            from_id: from_id.into(),
            to_id: Some(to_id.into()),
            label: EdgeLabel::Exact,
            confidence: None,
            source: None,
        }
    }

    /// An inferred edge: an association the investigation
    /// derived. Always carries its confidence and provenance.
    pub fn inferred(
        kind: impl Into<String>,
        from_id: impl Into<String>,
        to_id: impl Into<String>,
        confidence: f32,
        source: impl Into<String>,
    ) -> InvestigationEdge {
        InvestigationEdge {
            kind: kind.into(),
            from_id: from_id.into(),
            to_id: Some(to_id.into()),
            label: EdgeLabel::Inferred,
            confidence: Some(confidence),
            source: Some(source.into()),
        }
    }

    /// A missing link: the relation was looked for and no
    /// evidenced target exists. Preserved as a gap in the record.
    pub fn missing(kind: impl Into<String>, from_id: impl Into<String>) -> InvestigationEdge {
        InvestigationEdge {
            kind: kind.into(),
            from_id: from_id.into(),
            to_id: None,
            label: EdgeLabel::Exact,
            confidence: None,
            source: None,
        }
    }

    /// Whether this edge is a preserved gap (no target).
    pub fn is_gap(&self) -> bool {
        self.to_id.is_none()
    }

    /// The gap-list line this edge contributes, when it is a gap.
    pub fn gap_description(&self) -> Option<String> {
        self.is_gap()
            .then(|| format!("missing link: {} from {}", self.kind, self.from_id))
    }
}

/// A candidate change an investigation associated with the error
/// (design §5 step 6): a commit reached through the evidenced
/// commit → PR → issue joins of step 4. A candidate is an
/// association, never a causation claim (design §5 preamble).
///
/// The association carries the same exact-vs-inferred discipline
/// as [`InvestigationEdge`]: an inferred candidate always carries
/// its confidence and provenance source; an exact one never does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateChange {
    /// Canonical ID of the commit in its owning system.
    pub commit_id: String,
    /// Short summary of the change, as recorded by its source.
    pub summary: String,
    /// How the investigation associated this change with the
    /// error: an explicit source link, or a derived association.
    pub label: EdgeLabel,
    /// Confidence of an inferred association; always present for
    /// inferred candidates, never for exact ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// Provenance of an inferred association: where it was
    /// derived from. Always present for inferred candidates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl CandidateChange {
    /// An exact candidate: an explicit source link associates
    /// the change with the error.
    pub fn exact(commit_id: impl Into<String>, summary: impl Into<String>) -> CandidateChange {
        CandidateChange {
            commit_id: commit_id.into(),
            summary: summary.into(),
            label: EdgeLabel::Exact,
            confidence: None,
            source: None,
        }
    }

    /// An inferred candidate: the investigation derived the
    /// association. Always carries its confidence and provenance.
    pub fn inferred(
        commit_id: impl Into<String>,
        summary: impl Into<String>,
        confidence: f32,
        source: impl Into<String>,
    ) -> CandidateChange {
        CandidateChange {
            commit_id: commit_id.into(),
            summary: summary.into(),
            label: EdgeLabel::Inferred,
            confidence: Some(confidence),
            source: Some(source.into()),
        }
    }
}

/// A candidate the investigation considered and rejected, with
/// the reason (design §5 step 6). Rejected candidates stay in
/// the record so a later reader can see what was ruled out and
/// why; they are not evidence the bundle rests on, so their IDs
/// never appear in the bundle's evidence IDs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedCandidate {
    /// Canonical ID of the rejected candidate record.
    pub candidate_id: String,
    /// Why the investigation rejected it.
    pub reason: String,
}

/// Outcome of the unpushed-final-commit check (design §5 step 5),
/// exactly as reported — or not reported — by an authoritative
/// state source.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnpushedCommitStatus {
    /// The authoritative source reports an unpushed final commit.
    UnpushedFound,
    /// The authoritative source reports no unpushed final commit.
    NoneFound,
    /// No authoritative source reported: the check was not
    /// performed. This is a missing-data gap, never a negative
    /// result — an unpushed commit is never inferred from an
    /// absent PR (design §5 step 5, §6).
    #[default]
    NotChecked,
}

/// The unpushed-final-commit check of design §5 step 5, as one
/// recorded input: the status an authoritative state source
/// reported, when it reported it, and the check time. The core
/// never performs the check itself; it records what it is given
/// and names the gap when nothing was reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnpushedCommitCheck {
    pub status: UnpushedCommitStatus,
    /// When the authoritative source was checked (RFC 3339);
    /// present whenever a check was performed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<String>,
    /// The authoritative state source that reported the status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The unpushed commit the source reported; present only
    /// when the status is
    /// [`UnpushedCommitStatus::UnpushedFound`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_id: Option<String>,
}

impl UnpushedCommitCheck {
    /// The check was not performed: no authoritative state
    /// source reported.
    pub fn not_checked() -> UnpushedCommitCheck {
        UnpushedCommitCheck {
            status: UnpushedCommitStatus::NotChecked,
            checked_at: None,
            source: None,
            commit_id: None,
        }
    }

    /// A check an authoritative source reported, with its check
    /// time and source; `commit_id` only for
    /// [`UnpushedCommitStatus::UnpushedFound`].
    pub fn reported(
        status: UnpushedCommitStatus,
        checked_at: impl Into<String>,
        source: impl Into<String>,
        commit_id: Option<String>,
    ) -> UnpushedCommitCheck {
        UnpushedCommitCheck {
            status,
            checked_at: Some(checked_at.into()),
            source: Some(source.into()),
            commit_id,
        }
    }

    /// The gap-list line this check contributes, when the check
    /// was not performed.
    pub fn gap_description(&self) -> Option<String> {
        (self.status == UnpushedCommitStatus::NotChecked).then(|| {
            "unpushed-final-commit check not performed: no authoritative state source reported"
                .to_string()
        })
    }
}

impl Default for UnpushedCommitCheck {
    fn default() -> UnpushedCommitCheck {
        UnpushedCommitCheck::not_checked()
    }
}

/// The gathered findings an investigation assembles into the
/// full step-6 record (design §5 steps 4–6): the output of the
/// live-interface gathering slices, supplied as data. Assembly
/// is deterministic; nothing here is fetched, searched, or
/// inferred by the core.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InvestigationFindings {
    /// Candidate changes from the commit → PR → issue joins,
    /// in the order the investigation recorded them.
    pub candidate_changes: Vec<CandidateChange>,
    /// Candidates considered and rejected, with reasons.
    pub rejected_candidates: Vec<RejectedCandidate>,
    /// The step-5 unpushed-final-commit check, as reported.
    pub unpushed_commit_check: UnpushedCommitCheck,
    /// The investigation's stated next action. When absent, the
    /// assembly derives one deterministically from the bundle's
    /// own state (see [`build_bundle_with_findings`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
}

/// The compact investigation record of design §5 step 6:
/// query/fingerprint, time range, first observation, evidence
/// IDs, exact-vs-inferred edges, and missing-data gaps — plus,
/// when gathered findings are assembled in via
/// [`build_bundle_with_findings`], candidate changes, rejected
/// candidates/reasons, the unpushed-final-commit check, and the
/// next action.
///
/// Without findings those fields are absent rather than guessed.
/// The record never claims causation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceBundle {
    pub schema_version: u32,
    /// The error string exactly as supplied to the investigation.
    pub query: String,
    /// Fingerprint of `query`.
    pub fingerprint: ErrorFingerprint,
    /// (start, end) RFC 3339 bounds of the occurrence set actually
    /// searched; both empty when the set was empty.
    pub time_range: (String, String),
    /// First-observation result over the searched set, when an
    /// exact occurrence exists there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_observation: Option<FirstObservation>,
    /// IDs of the records the bundle's evidence rests on: exact
    /// occurrences' source records first (in observation order),
    /// then edge endpoints, then candidate changes and a reported
    /// unpushed commit, deduplicated in first-seen order.
    /// Rejected candidates are not evidence and never appear.
    pub evidence_ids: Vec<String>,
    /// Every edge supplied, gaps included, in supplied order.
    pub edges: Vec<InvestigationEdge>,
    /// Candidate changes assembled from gathered findings, in
    /// supplied order; empty when no findings were assembled.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_changes: Vec<CandidateChange>,
    /// Rejected candidates with their reasons, in supplied
    /// order; empty when no findings were assembled.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejected_candidates: Vec<RejectedCandidate>,
    /// The unpushed-final-commit check, when findings were
    /// assembled. Absent in a core-only bundle: the check is a
    /// gathered finding, not a core derivation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unpushed_commit_check: Option<UnpushedCommitCheck>,
    /// The next action the record states: the investigation's
    /// own when supplied, otherwise derived deterministically
    /// from the bundle's gaps and candidates. Absent in a
    /// core-only bundle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
    /// Missing-data warnings: every gap edge, plus the
    /// no-exact-occurrence and predates-retention warnings when
    /// they apply, plus — with findings assembled — the
    /// not-performed unpushed check and the no-candidate-changes
    /// warnings when they apply. Partial investigation is a
    /// valid result if it names its gaps (design §6).
    pub gaps: Vec<String>,
}

/// Build the core evidence bundle for an investigation of
/// `query` over a retained occurrence set and a set of evidenced
/// edges (design §5 steps 1–2 and 6).
///
/// `occurrences` is the retained, authorized set the caller
/// searched; `retention_floor` is that set's stated floor, when
/// known. Only exact fingerprint matches of `query` count as
/// occurrences of the investigated error.
pub fn build_bundle(
    query: &str,
    occurrences: &[ErrorOccurrence],
    edges: &[InvestigationEdge],
    retention_floor: Option<&str>,
) -> EvidenceBundle {
    assemble_bundle(query, occurrences, edges, retention_floor, None)
}

/// Build the full evidence bundle: the core bundle of
/// [`build_bundle`] plus the gathered findings of design §5
/// steps 4–6, assembled deterministically.
///
/// The findings' candidate changes join the evidence IDs (their
/// rejected candidates do not); a not-performed
/// unpushed-final-commit check and an empty candidate list are
/// named as missing-data gaps, never converted into negative
/// facts (design §6). The record's next action is the findings'
/// own when stated; otherwise it is derived from the assembled
/// bundle: with gaps named, resolve them and re-run; with a
/// clean record, a human reviews the candidate changes.
pub fn build_bundle_with_findings(
    query: &str,
    occurrences: &[ErrorOccurrence],
    edges: &[InvestigationEdge],
    retention_floor: Option<&str>,
    findings: &InvestigationFindings,
) -> EvidenceBundle {
    assemble_bundle(query, occurrences, edges, retention_floor, Some(findings))
}

fn assemble_bundle(
    query: &str,
    occurrences: &[ErrorOccurrence],
    edges: &[InvestigationEdge],
    retention_floor: Option<&str>,
    findings: Option<&InvestigationFindings>,
) -> EvidenceBundle {
    let fingerprint = ErrorFingerprint::of(query);
    let time_range = match (
        occurrences.iter().map(|o| o.observed_at.as_str()).min(),
        occurrences.iter().map(|o| o.observed_at.as_str()).max(),
    ) {
        (Some(start), Some(end)) => (start.to_string(), end.to_string()),
        _ => (String::new(), String::new()),
    };
    let first = first_observation(occurrences, &fingerprint, retention_floor);

    let mut evidence_ids: Vec<String> = Vec::new();
    let mut push_id = |id: &str| {
        if !evidence_ids.iter().any(|seen| seen == id) {
            evidence_ids.push(id.to_string());
        }
    };
    let mut exact: Vec<&ErrorOccurrence> = occurrences
        .iter()
        .filter(|o| o.fingerprint.id() == fingerprint.id())
        .collect();
    exact.sort_by(|a, b| (&a.observed_at, &a.source_id).cmp(&(&b.observed_at, &b.source_id)));
    for occurrence in &exact {
        push_id(&occurrence.source_id);
    }
    for edge in edges {
        push_id(&edge.from_id);
        if let Some(to_id) = &edge.to_id {
            push_id(to_id);
        }
    }
    if let Some(findings) = findings {
        for candidate in &findings.candidate_changes {
            push_id(&candidate.commit_id);
        }
        if let Some(commit_id) = &findings.unpushed_commit_check.commit_id {
            push_id(commit_id);
        }
    }

    let mut gaps: Vec<String> = edges.iter().filter_map(|e| e.gap_description()).collect();
    match &first {
        None => gaps.push(
            "no exact occurrences of the fingerprint in the retained occurrence set".to_string(),
        ),
        Some(first) if first.kind == FirstObservationKind::EarliestRetainedObservation => {
            gaps.push(format!(
                "earliest occurrence is the {}: the actual first occurrence may predate the retention floor",
                first.kind.label()
            ));
        }
        _ => {}
    }

    let (candidate_changes, rejected_candidates, unpushed_commit_check, next_action) =
        match findings {
            None => (Vec::new(), Vec::new(), None, None),
            Some(findings) => {
                if let Some(gap) = findings.unpushed_commit_check.gap_description() {
                    gaps.push(gap);
                }
                if findings.candidate_changes.is_empty() {
                    gaps.push(
                        "no candidate changes recorded: the commit/PR/issue joins produced no evidenced candidates"
                            .to_string(),
                    );
                }
                let next_action = findings.next_action.clone().or_else(|| {
                    if gaps.is_empty() {
                        Some(
                            "human review of the candidate changes; the bundle records evidence only"
                                .to_string(),
                        )
                    } else {
                        Some(
                            "resolve the named missing-data gaps, then re-run the investigation"
                                .to_string(),
                        )
                    }
                });
                (
                    findings.candidate_changes.clone(),
                    findings.rejected_candidates.clone(),
                    Some(findings.unpushed_commit_check.clone()),
                    next_action,
                )
            }
        };

    EvidenceBundle {
        schema_version: C8_FINGERPRINT_SCHEMA_VERSION,
        query: query.to_string(),
        fingerprint,
        time_range,
        first_observation: first,
        evidence_ids,
        edges: edges.to_vec(),
        candidate_changes,
        rejected_candidates,
        unpushed_commit_check,
        next_action,
        gaps,
    }
}
