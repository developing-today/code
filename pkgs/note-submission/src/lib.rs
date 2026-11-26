//! Typed envelope and fail-closed staging/promotion gate for agent notes.
//!
//! This crate defines the pipeline contract and its table-shaped records. A
//! persistent adapter (the Turso submission DB is the system of record) can
//! implement [`SubmissionStore`]; this crate deliberately does not create a
//! competing local database. Placement classification is the #7717
//! classifier ([`classify`]): it applies the #6022 decision table to the
//! submission's declared class, the caller-supplied routing context, and
//! the sensitivity ceiling, and fails closed whenever classification or
//! sensitivity is uncertain. A caller may still supply a
//! [`PlacementDecision`] directly; either way the exact decision must be
//! confirmed by a named human before [`promote`] can publish anything.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A content-addressed source span used to preserve the submission's lineage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceSpan {
    /// Source location, such as a repository path or workspace URI.
    pub source: String,
    /// Half-open byte range in the source; absent for sources without byte offsets.
    pub byte_range: Option<(u64, u64)>,
    /// Hash of the bytes represented by this span.
    pub content_sha256: String,
}

/// Provenance required by the placement design for every submission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub source_system: String,
    pub machine: String,
    /// RFC 3339 timestamp supplied by the caller.
    pub captured_at: String,
    pub author: String,
    pub owner: String,
    pub spans: Vec<ProvenanceSpan>,
}

/// Source/caller sensitivity ceiling. These are not physical Atlas tiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensitivityCeiling {
    Shareable,
    Internal,
    SecretBearing,
    Personal,
    LocalOnly,
}

/// Physical destination sensitivity selected during review.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensitivityTier {
    Public,
    Internal,
    Secret,
}

/// A note's placement class, from the §2 placement decision table in
/// `doc/2026-11-10T20-00-00Z_design_agent_note_placement.md` (the agent-note
/// placement design, #6022). A submission declares its class so the
/// promoter — and later the #7717 classifier — can see what the
/// submitter judged the note to be. Declaring a class does not route
/// anything by itself: the routing destination still arrives as a
/// [`PlacementDecision`] and still needs human approval (#8204).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteClass {
    /// Scratch / intermediate output; task lifetime only.
    EphemeralWorkingNote,
    /// What a shift did; memory daily log plus sweep/backlog records.
    SessionDailyLog,
    /// Verified fact that survives its repo.
    DurableFinding,
    /// Multi-source, cross-repo synthesis.
    ResearchSynthesis,
    /// Design / ADR / architecture record for a specific repo.
    DesignRecord,
    /// Operations procedure for a specific system.
    Runbook,
    /// A deliverable the user opens.
    UserFacingDocument,
    /// Standing fact about the user, their people, or their decisions.
    MemoryTopic,
    /// Machine-generated state snapshot owned by a system's store.
    MachineStateSnapshot,
    /// Published note destined for the ID graph (§5 of the design).
    IdNativeNote,
}

/// The home the §2 decision table assigns to a class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacementHome {
    WorkspaceScratch,
    MemoryDailyLog,
    Zettel,
    RepoDoc,
    OwningRepo,
    RequestingSurface,
    AgentMemory,
    OwningSystemStore,
    IdGraph,
}

/// Atlas ingestion tier (design §2): 0 = ID-native (the graph object
/// is the Atlas record), 1 = first-class corpus, 2 = repo corpus,
/// 3 = provenance only. `NotIngested` is the table's "not ingested"
/// outcome, not a tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AtlasTier {
    #[serde(rename = "tier_0")]
    Tier0,
    #[serde(rename = "tier_1")]
    Tier1,
    #[serde(rename = "tier_2")]
    Tier2,
    #[serde(rename = "tier_3")]
    Tier3,
    #[serde(rename = "not_ingested")]
    NotIngested,
}

impl NoteClass {
    /// The §2 decision table's home for this class.
    #[must_use]
    pub fn default_home(self) -> PlacementHome {
        match self {
            Self::EphemeralWorkingNote => PlacementHome::WorkspaceScratch,
            Self::SessionDailyLog => PlacementHome::MemoryDailyLog,
            Self::DurableFinding | Self::ResearchSynthesis => PlacementHome::Zettel,
            Self::DesignRecord => PlacementHome::RepoDoc,
            Self::Runbook => PlacementHome::OwningRepo,
            Self::UserFacingDocument => PlacementHome::RequestingSurface,
            Self::MemoryTopic => PlacementHome::AgentMemory,
            Self::MachineStateSnapshot => PlacementHome::OwningSystemStore,
            Self::IdNativeNote => PlacementHome::IdGraph,
        }
    }

    /// The §2 decision table's Atlas tier for this class. A
    /// user-facing document is Tier 2 only once canonicalized; the
    /// table's default for the class itself is not ingested.
    #[must_use]
    pub fn default_tier(self) -> AtlasTier {
        match self {
            Self::DurableFinding | Self::ResearchSynthesis => AtlasTier::Tier1,
            Self::DesignRecord | Self::Runbook => AtlasTier::Tier2,
            Self::SessionDailyLog | Self::MemoryTopic => AtlasTier::Tier3,
            Self::IdNativeNote => AtlasTier::Tier0,
            Self::EphemeralWorkingNote | Self::UserFacingDocument | Self::MachineStateSnapshot => {
                AtlasTier::NotIngested
            }
        }
    }
}

/// What a submitter declares about a note's placement (#8204, design
/// §7 item 1): its §2 class, plus the intended Atlas tier when the
/// submitter means to override the class default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacementDeclaration {
    pub note_class: NoteClass,
    pub atlas_tier: Option<AtlasTier>,
}

/// A declaration with the §2 decision table applied: the class's
/// home, and the intended tier or the class's default tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveDeclaration {
    pub note_class: NoteClass,
    pub home: PlacementHome,
    pub atlas_tier: AtlasTier,
    /// True when `atlas_tier` came from the decision table rather
    /// than from the submitter's declaration.
    pub tier_was_defaulted: bool,
}

/// Classification and canonical destination selected by routing.
///
/// This is deliberately an opaque routing result: #7717 implements the
/// policy classifier. The gate only checks that a complete decision is
/// present and that a human approves the exact result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacementDecision {
    pub destination: String,
    pub canonical_identity: String,
    pub source_revision: String,
    pub sensitivity: SensitivityTier,
    pub rationale: String,
}

/// Lifecycle of a submission row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionState {
    Submitted,
    Staged,
    Promoted,
}

/// `submissions` table row / transport envelope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Submission {
    pub id: String,
    pub content: String,
    pub content_sha256: String,
    pub provenance: Provenance,
    pub sensitivity_ceiling: SensitivityCeiling,
    /// The submitter's declared §2 placement (#8204), when declared.
    /// Absent on envelopes written before the declaration existed.
    #[serde(default)]
    pub declared_placement: Option<PlacementDeclaration>,
    pub placement: Option<PlacementDecision>,
    pub state: SubmissionState,
    /// RFC 3339 timestamp.
    pub submitted_at: String,
    /// RFC 3339 timestamp of the most recent state transition.
    pub updated_at: String,
}

impl Submission {
    /// The declared placement with the §2 decision table's defaults
    /// applied, or `None` when the submission declared nothing (the
    /// pre-#8204 behaviour: routing is entirely the caller's
    /// [`PlacementDecision`], confirmed by a human at promotion).
    #[must_use]
    pub fn effective_declaration(&self) -> Option<EffectiveDeclaration> {
        let declared = self.declared_placement?;
        Some(EffectiveDeclaration {
            note_class: declared.note_class,
            home: declared.note_class.default_home(),
            atlas_tier: declared
                .atlas_tier
                .unwrap_or_else(|| declared.note_class.default_tier()),
            tier_was_defaulted: declared.atlas_tier.is_none(),
        })
    }
}

/// `notes_out` table row, including canonical identity and source pin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteOut {
    pub id: String,
    pub submission_id: String,
    pub destination: String,
    pub canonical_identity: String,
    pub source_revision: String,
    pub source_sha256: String,
    pub status: NoteOutStatus,
    pub promoted_by: String,
    pub promoted_at: String,
}

/// Publication state recorded in `notes_out`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteOutStatus {
    Published,
}

/// `upload_attempts` table row. Failed and held attempts are retained.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadAttempt {
    pub id: String,
    pub submission_id: String,
    pub destination: String,
    pub actor: String,
    pub attempted_at: String,
    pub status: UploadAttemptStatus,
    pub detail: String,
}

/// Result recorded for one destination attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadAttemptStatus {
    Published,
    HeldByControl,
    Rejected,
    DestinationFailed,
}

/// `corpus_controls` table row, consulted immediately before each write.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusControl {
    pub destination: String,
    pub enabled: bool,
    pub manual_hold: bool,
    pub updated_at: String,
}

/// Human decision bound to the exact staged destination and sensitivity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanApproval {
    pub human_id: String,
    pub approved_destination: String,
    pub approved_sensitivity: SensitivityTier,
    pub rationale: String,
    /// RFC 3339 timestamp.
    pub approved_at: String,
}

/// A destination writer. Implementations must publish the exact bytes passed
/// to this method and return an error if publication did not complete.
pub trait DestinationWriter {
    fn publish(
        &mut self,
        destination: &str,
        canonical_identity: &str,
        content: &str,
    ) -> Result<(), String>;
}

/// Persistence boundary for the #6267 submission tables.
///
/// The order here is intentionally visible: implementations must re-read the
/// destination control immediately before calling `publish`, then persist the
/// attempt and resulting rows. A Turso adapter can implement this contract
/// without changing the envelope or gate.
pub trait SubmissionStore {
    fn submission(&self, id: &str) -> Option<&Submission>;
    fn submission_mut(&mut self, id: &str) -> Option<&mut Submission>;
    fn insert_submission(&mut self, submission: Submission) -> Result<(), PipelineError>;
    fn insert_attempt(&mut self, attempt: UploadAttempt);
    /// Allocate an attempt ID unique within this submission ledger.
    fn allocate_attempt_id(&self, submission_id: &str) -> String;
    fn insert_note_out(&mut self, note_out: NoteOut);
    fn control(&self, destination: &str) -> Option<&CorpusControl>;
    fn stage(&mut self, staged: StagedNote) -> Result<(), PipelineError>;
    fn staged(&self, submission_id: &str) -> Option<&StagedNote>;
    fn remove_staged(&mut self, submission_id: &str);
}

/// Staging record for a submission awaiting human promotion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagedNote {
    pub submission_id: String,
    pub placement: PlacementDecision,
    /// The submission's effective §2 declaration (#8204), snapshotted
    /// at stage time so the promoter reviews the declared class and
    /// tier alongside the routing decision. `None` for undeclared
    /// submissions.
    pub declaration: Option<EffectiveDeclaration>,
    /// RFC 3339 timestamp.
    pub staged_at: String,
}

/// Simple in-memory `SubmissionStore`, useful for tests and process-local
/// adapters. Production callers should supply a durable implementation.
#[derive(Clone, Debug, Default)]
pub struct MemoryStore {
    pub submissions: BTreeMap<String, Submission>,
    pub notes_out: Vec<NoteOut>,
    pub upload_attempts: Vec<UploadAttempt>,
    pub corpus_controls: BTreeMap<String, CorpusControl>,
    pub staging: BTreeMap<String, StagedNote>,
}

impl SubmissionStore for MemoryStore {
    fn submission(&self, id: &str) -> Option<&Submission> {
        self.submissions.get(id)
    }

    fn submission_mut(&mut self, id: &str) -> Option<&mut Submission> {
        self.submissions.get_mut(id)
    }

    fn insert_submission(&mut self, submission: Submission) -> Result<(), PipelineError> {
        if self.submissions.contains_key(&submission.id) {
            return Err(PipelineError::DuplicateSubmission(submission.id));
        }
        self.submissions.insert(submission.id.clone(), submission);
        Ok(())
    }

    fn insert_attempt(&mut self, attempt: UploadAttempt) {
        self.upload_attempts.push(attempt);
    }

    fn allocate_attempt_id(&self, submission_id: &str) -> String {
        let next = self
            .upload_attempts
            .iter()
            .filter(|attempt| attempt.submission_id == submission_id)
            .count();
        format!("{submission_id}:attempt:{next}")
    }

    fn insert_note_out(&mut self, note_out: NoteOut) {
        self.notes_out.push(note_out);
    }

    fn control(&self, destination: &str) -> Option<&CorpusControl> {
        self.corpus_controls.get(destination)
    }

    fn stage(&mut self, staged: StagedNote) -> Result<(), PipelineError> {
        if self.staging.contains_key(&staged.submission_id) {
            return Err(PipelineError::AlreadyStaged(staged.submission_id));
        }
        self.staging.insert(staged.submission_id.clone(), staged);
        Ok(())
    }

    fn staged(&self, submission_id: &str) -> Option<&StagedNote> {
        self.staging.get(submission_id)
    }

    fn remove_staged(&mut self, submission_id: &str) {
        self.staging.remove(submission_id);
    }
}

/// Gate error; rejected/held/destination failures still leave an attempt row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PipelineError {
    InvalidEnvelope(String),
    DuplicateSubmission(String),
    UnknownSubmission(String),
    InvalidState(String),
    AlreadyStaged(String),
    MissingPlacement(String),
    PolicyViolation(String),
    InvalidApproval(String),
    ControlUnavailable(String),
    ControlHeld(String),
    DestinationFailed(String),
}

impl fmt::Display for PipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEnvelope(detail) => write!(f, "invalid submission envelope: {detail}"),
            Self::DuplicateSubmission(id) => write!(f, "submission already exists: {id}"),
            Self::UnknownSubmission(id) => write!(f, "unknown submission: {id}"),
            Self::InvalidState(id) => write!(f, "submission is not in the required state: {id}"),
            Self::AlreadyStaged(id) => write!(f, "submission is already staged: {id}"),
            Self::MissingPlacement(id) => write!(f, "submission has no placement decision: {id}"),
            Self::PolicyViolation(detail) => {
                write!(f, "placement exceeds source ceiling: {detail}")
            }
            Self::InvalidApproval(detail) => write!(f, "human approval rejected: {detail}"),
            Self::ControlUnavailable(destination) => {
                write!(f, "destination control is missing: {destination}")
            }
            Self::ControlHeld(destination) => {
                write!(f, "destination is disabled or held: {destination}")
            }
            Self::DestinationFailed(detail) => {
                write!(f, "destination publication failed: {detail}")
            }
        }
    }
}

impl std::error::Error for PipelineError {}

/// Compute lowercase SHA-256 hex for content and provenance checks.
#[must_use]
pub fn content_sha256(content: &str) -> String {
    let digest = Sha256::digest(content.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Validate and store a new `submissions` row.
pub fn submit<S: SubmissionStore>(
    store: &mut S,
    id: impl Into<String>,
    content: String,
    provenance: Provenance,
    sensitivity_ceiling: SensitivityCeiling,
    submitted_at: impl Into<String>,
) -> Result<(), PipelineError> {
    submit_inner(
        store,
        id,
        content,
        provenance,
        sensitivity_ceiling,
        None,
        submitted_at,
    )
}

/// Validate and store a new `submissions` row carrying the
/// submitter's §2 placement declaration (#8204). The declaration is
/// recorded on the envelope and surfaced to the promoter at staging;
/// it changes nothing about routing or the human gate.
pub fn submit_with_declaration<S: SubmissionStore>(
    store: &mut S,
    id: impl Into<String>,
    content: String,
    provenance: Provenance,
    sensitivity_ceiling: SensitivityCeiling,
    declaration: PlacementDeclaration,
    submitted_at: impl Into<String>,
) -> Result<(), PipelineError> {
    submit_inner(
        store,
        id,
        content,
        provenance,
        sensitivity_ceiling,
        Some(declaration),
        submitted_at,
    )
}

fn submit_inner<S: SubmissionStore>(
    store: &mut S,
    id: impl Into<String>,
    content: String,
    provenance: Provenance,
    sensitivity_ceiling: SensitivityCeiling,
    declared_placement: Option<PlacementDeclaration>,
    submitted_at: impl Into<String>,
) -> Result<(), PipelineError> {
    let id = id.into();
    let submitted_at = submitted_at.into();
    validate_provenance(&provenance)?;
    if id.trim().is_empty() || submitted_at.trim().is_empty() {
        return Err(PipelineError::InvalidEnvelope(
            "submission id and timestamp are required".to_owned(),
        ));
    }
    let hash = content_sha256(&content);
    for span in &provenance.spans {
        if span.source.trim().is_empty() || span.content_sha256.trim().is_empty() {
            return Err(PipelineError::InvalidEnvelope(
                "each provenance span needs a source and content hash".to_owned(),
            ));
        }
        if let Some((start, end)) = span.byte_range {
            if start > end {
                return Err(PipelineError::InvalidEnvelope(
                    "provenance byte ranges must be ordered".to_owned(),
                ));
            }
        }
    }
    store.insert_submission(Submission {
        id,
        content,
        content_sha256: hash,
        provenance,
        sensitivity_ceiling,
        declared_placement,
        placement: None,
        state: SubmissionState::Submitted,
        submitted_at: submitted_at.clone(),
        updated_at: submitted_at,
    })
}

/// Move a submitted note into staging with a proposed route. No publication
/// happens here, and no destination is inferred by this function.
pub fn stage<S: SubmissionStore>(
    store: &mut S,
    submission_id: &str,
    placement: PlacementDecision,
    staged_at: impl Into<String>,
) -> Result<(), PipelineError> {
    let staged_at = staged_at.into();
    let submission = store
        .submission(submission_id)
        .ok_or_else(|| PipelineError::UnknownSubmission(submission_id.to_owned()))?;
    if submission.state != SubmissionState::Submitted {
        return Err(PipelineError::InvalidState(submission_id.to_owned()));
    }
    let declaration = submission.effective_declaration();
    validate_placement(&placement)?;
    validate_sensitivity_ceiling(submission.sensitivity_ceiling, &placement)?;
    store.stage(StagedNote {
        submission_id: submission_id.to_owned(),
        placement: placement.clone(),
        declaration,
        staged_at: staged_at.clone(),
    })?;
    if let Some(submission) = store.submission_mut(submission_id) {
        submission.placement = Some(placement);
        submission.state = SubmissionState::Staged;
        submission.updated_at = staged_at;
    }
    Ok(())
}

/// Execute the promotion gate: validate the exact human decision, re-read the
/// destination kill switch, publish, and record the output and attempt.
pub fn promote<S: SubmissionStore, W: DestinationWriter>(
    store: &mut S,
    writer: &mut W,
    submission_id: &str,
    approval: &HumanApproval,
) -> Result<(), PipelineError> {
    let attempt_id = store.allocate_attempt_id(submission_id);
    let staged = store
        .staged(submission_id)
        .cloned()
        .ok_or_else(|| PipelineError::InvalidState(submission_id.to_owned()))?;
    let submission = store
        .submission(submission_id)
        .cloned()
        .ok_or_else(|| PipelineError::UnknownSubmission(submission_id.to_owned()))?;
    if submission.state != SubmissionState::Staged {
        return Err(PipelineError::InvalidState(submission_id.to_owned()));
    }
    let decision = submission
        .placement
        .as_ref()
        .ok_or_else(|| PipelineError::MissingPlacement(submission_id.to_owned()))?;
    if decision != &staged.placement {
        return Err(PipelineError::InvalidState(submission_id.to_owned()));
    }
    if submission.content_sha256 != content_sha256(&submission.content) {
        record_attempt(
            store,
            &attempt_id,
            &submission,
            &staged,
            approval,
            UploadAttemptStatus::Rejected,
            "content hash changed after submission",
        );
        return Err(PipelineError::InvalidApproval(
            "content hash changed after submission".to_owned(),
        ));
    }
    if let Err(error) = validate_approval(approval, decision) {
        record_attempt(
            store,
            &attempt_id,
            &submission,
            &staged,
            approval,
            UploadAttemptStatus::Rejected,
            &error.to_string(),
        );
        return Err(error);
    }

    // Re-read the control immediately before publication. Missing controls
    // fail closed; disabled or held destinations produce an audit row.
    let control = store
        .control(&decision.destination)
        .cloned()
        .ok_or_else(|| PipelineError::ControlUnavailable(decision.destination.clone()));
    let control = match control {
        Ok(control) => control,
        Err(error) => {
            record_attempt(
                store,
                &attempt_id,
                &submission,
                &staged,
                approval,
                UploadAttemptStatus::HeldByControl,
                &error.to_string(),
            );
            return Err(error);
        }
    };
    if !control.enabled || control.manual_hold {
        record_attempt(
            store,
            &attempt_id,
            &submission,
            &staged,
            approval,
            UploadAttemptStatus::HeldByControl,
            "destination is disabled or manual hold is active",
        );
        return Err(PipelineError::ControlHeld(decision.destination.clone()));
    }
    if control.destination != decision.destination {
        let detail = "control row destination does not match its lookup key";
        record_attempt(
            store,
            &attempt_id,
            &submission,
            &staged,
            approval,
            UploadAttemptStatus::HeldByControl,
            detail,
        );
        return Err(PipelineError::ControlUnavailable(
            decision.destination.clone(),
        ));
    }

    if let Err(detail) = writer.publish(
        &decision.destination,
        &decision.canonical_identity,
        &submission.content,
    ) {
        record_attempt(
            store,
            &attempt_id,
            &submission,
            &staged,
            approval,
            UploadAttemptStatus::DestinationFailed,
            &detail,
        );
        return Err(PipelineError::DestinationFailed(detail));
    }

    store.insert_note_out(NoteOut {
        id: format!("out:{submission_id}"),
        submission_id: submission_id.to_owned(),
        destination: decision.destination.clone(),
        canonical_identity: decision.canonical_identity.clone(),
        source_revision: decision.source_revision.clone(),
        source_sha256: submission.content_sha256.clone(),
        status: NoteOutStatus::Published,
        promoted_by: approval.human_id.clone(),
        promoted_at: approval.approved_at.clone(),
    });
    record_attempt(
        store,
        &attempt_id,
        &submission,
        &staged,
        approval,
        UploadAttemptStatus::Published,
        "published after human approval and control recheck",
    );
    if let Some(record) = store.submission_mut(submission_id) {
        record.state = SubmissionState::Promoted;
        record.updated_at = approval.approved_at.clone();
    }
    store.remove_staged(submission_id);
    Ok(())
}

fn validate_provenance(provenance: &Provenance) -> Result<(), PipelineError> {
    if provenance.source_system.trim().is_empty()
        || provenance.machine.trim().is_empty()
        || provenance.captured_at.trim().is_empty()
        || provenance.author.trim().is_empty()
        || provenance.owner.trim().is_empty()
        || provenance.spans.is_empty()
    {
        return Err(PipelineError::InvalidEnvelope(
            "source system, machine, capture time, author, owner, and provenance spans are required"
                .to_owned(),
        ));
    }
    Ok(())
}

fn validate_placement(placement: &PlacementDecision) -> Result<(), PipelineError> {
    if placement.destination.trim().is_empty()
        || placement.canonical_identity.trim().is_empty()
        || placement.source_revision.trim().is_empty()
        || placement.rationale.trim().is_empty()
    {
        return Err(PipelineError::InvalidEnvelope(
            "placement needs destination, canonical identity, source revision, and rationale"
                .to_owned(),
        ));
    }
    Ok(())
}

fn validate_sensitivity_ceiling(
    ceiling: SensitivityCeiling,
    placement: &PlacementDecision,
) -> Result<(), PipelineError> {
    let permitted = match ceiling {
        // Source-classification ceilings are not Atlas tiers. The #7717
        // classifier resolves physical tiers under exactly these rules;
        // this check remains the staging backstop for caller-supplied
        // decisions.
        SensitivityCeiling::Shareable => true,
        SensitivityCeiling::Internal => placement.sensitivity != SensitivityTier::Public,
        SensitivityCeiling::SecretBearing => placement.sensitivity == SensitivityTier::Secret,
        SensitivityCeiling::Personal | SensitivityCeiling::LocalOnly => false,
    };
    if permitted {
        Ok(())
    } else {
        Err(PipelineError::PolicyViolation(format!(
            "{ceiling:?} cannot be promoted to {:?}",
            placement.sensitivity
        )))
    }
}

fn validate_approval(
    approval: &HumanApproval,
    placement: &PlacementDecision,
) -> Result<(), PipelineError> {
    if approval.human_id.trim().is_empty()
        || approval.rationale.trim().is_empty()
        || approval.approved_at.trim().is_empty()
    {
        return Err(PipelineError::InvalidApproval(
            "human identity, timestamp, and rationale are required".to_owned(),
        ));
    }
    if approval.approved_destination != placement.destination {
        return Err(PipelineError::InvalidApproval(
            "approved destination differs from staged destination".to_owned(),
        ));
    }
    if approval.approved_sensitivity != placement.sensitivity {
        return Err(PipelineError::InvalidApproval(
            "approved sensitivity differs from staged sensitivity".to_owned(),
        ));
    }
    Ok(())
}

fn record_attempt<S: SubmissionStore>(
    store: &mut S,
    attempt_id: &str,
    submission: &Submission,
    staged: &StagedNote,
    approval: &HumanApproval,
    status: UploadAttemptStatus,
    detail: &str,
) {
    store.insert_attempt(UploadAttempt {
        id: attempt_id.to_owned(),
        submission_id: submission.id.clone(),
        destination: staged.placement.destination.clone(),
        actor: approval.human_id.clone(),
        attempted_at: approval.approved_at.clone(),
        status,
        detail: detail.to_owned(),
    });
}

// ---------------------------------------------------------------------------
// #7717: policy-aware placement classification and routing.
//
// The classifier consumes the submission's declared §2 class (the #8204
// declaration), the registry/routing context a caller supplies, and the
// submission's sensitivity ceiling, and produces exactly one outcome: a
// complete [`PlacementDecision`] ready for the human gate, a certain
// classification into a home this pipeline does not publish, or a
// fail-closed refusal that leaves the submission unpublished in the
// submission DB. It invents no destination: Zettel routing needs the
// registry-selected corpus, repo routing needs the owning repo, and
// ID-native routing needs the §5 eligibility evidence — all supplied by
// the caller. Atlas physical tiers are resolved only after the
// source/caller sensitivity ceiling is applied; the default physical
// tier is internal, and public/secret are explicit requests. Nothing
// here publishes: [`promote`]'s human gate still decides publication,
// and its approval/control rechecks run on classifier decisions exactly
// as on caller-supplied ones.
// ---------------------------------------------------------------------------

/// Destination name for ID-native publication. The graph object is
/// canonical (§5 of the placement design); repo/Zettel copies of an
/// ID-native note are projections that link back to the object id.
pub const ID_GRAPH_DESTINATION: &str = "id-graph";

/// §5 ID-native eligibility evidence, supplied by the caller. All three
/// conditions must hold. The writer-set is named under the existing
/// #5545 authority — this crate introduces no parallel permission
/// schema — and the object encoding stays #6062's format.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdNativeEligibility {
    /// (a) The note is meant to be cited and related across systems,
    /// not just read.
    pub cited_across_systems: bool,
    /// (b) The note has a stable identity worth addressing.
    pub stable_identity: bool,
    /// (c) The #5545 writer-set that can be named for the object.
    pub writer_set: Option<String>,
    /// The graph object id; it is the note's canonical identity.
    pub object_id: Option<String>,
}

/// Registry and routing facts the classifier cannot infer: the
/// registry-selected Zettelkasten corpus, the owning repo for
/// repo-coupled classes, an explicit physical-tier request, the §5
/// eligibility evidence, and an optional canonical-identity override.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingContext {
    pub zettel_corpus: Option<String>,
    pub owning_repo: Option<String>,
    pub requested_tier: Option<SensitivityTier>,
    pub id_native: Option<IdNativeEligibility>,
    pub canonical_identity: Option<String>,
}

/// The classifier's verdict for one submission (#7717).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Classification {
    /// A complete routing decision, ready to stage for human approval.
    Routed {
        declaration: EffectiveDeclaration,
        decision: PlacementDecision,
    },
    /// Classified with certainty into a home this pipeline does not
    /// publish (workspace scratch, memory, the requesting surface, or
    /// the owning system's store). The submission stays in the
    /// submission DB as the staging record; nothing is promoted.
    NotPublished {
        declaration: EffectiveDeclaration,
        reason: String,
    },
    /// Fail-closed: classification or sensitivity is uncertain, a
    /// required routing fact is missing, or the ceiling forbids the
    /// route. The submission stays unpublished until a human resolves
    /// the reason; nothing auto-commits or auto-promotes.
    StaysStaged {
        declaration: Option<EffectiveDeclaration>,
        reason: String,
    },
}

impl Classification {
    /// The routing decision, when the verdict is `Routed`.
    #[must_use]
    pub fn decision(&self) -> Option<&PlacementDecision> {
        match self {
            Self::Routed { decision, .. } => Some(decision),
            Self::NotPublished { .. } | Self::StaysStaged { .. } => None,
        }
    }
}

/// Classify one submission against the #6022 decision table (#7717).
///
/// Pure and deterministic: the same submission and context always
/// produce the same verdict, and nothing is written anywhere. Apply a
/// `Routed` verdict with [`stage_classified`] (or stage the decision
/// through [`stage`] directly); the human gate in [`promote`] still
/// decides publication.
#[must_use]
pub fn classify(submission: &Submission, context: &RoutingContext) -> Classification {
    let Some(declaration) = submission.effective_declaration() else {
        return Classification::StaysStaged {
            declaration: None,
            reason: "submission declares no §2 placement class; classification is uncertain"
                .to_owned(),
        };
    };
    let staged = |reason: String| Classification::StaysStaged {
        declaration: Some(declaration),
        reason,
    };
    match declaration.home {
        PlacementHome::WorkspaceScratch => Classification::NotPublished {
            declaration,
            reason: "ephemeral working notes live in workspace scratch for the task lifetime \
                     and are not published"
                .to_owned(),
        },
        PlacementHome::MemoryDailyLog | PlacementHome::AgentMemory => {
            Classification::NotPublished {
                declaration,
                reason: "memory homes are written through the memory system, never published \
                         by this pipeline; Atlas sees provenance only"
                    .to_owned(),
            }
        }
        PlacementHome::OwningSystemStore => Classification::NotPublished {
            declaration,
            reason: "machine state snapshots live in the owning system's store; Atlas reads \
                     that system of record"
                .to_owned(),
        },
        PlacementHome::RequestingSurface => Classification::NotPublished {
            declaration,
            reason: "user-facing documents are delivered on the requesting surface; a \
                     canonicalized copy is submitted as its own note"
                .to_owned(),
        },
        PlacementHome::Zettel => {
            let Some(corpus) = non_empty(context.zettel_corpus.as_deref()) else {
                return staged("Zettel routing needs the registry-selected corpus".to_owned());
            };
            route(
                submission,
                &declaration,
                context,
                corpus.to_owned(),
                "registry-selected Zettelkasten corpus",
                context.canonical_identity.clone(),
                None,
            )
        }
        PlacementHome::RepoDoc => {
            let Some(repo) = non_empty(context.owning_repo.as_deref()) else {
                return staged("repo-document routing needs the owning repo".to_owned());
            };
            route(
                submission,
                &declaration,
                context,
                repo.to_owned(),
                "owning repo doc/",
                context.canonical_identity.clone(),
                None,
            )
        }
        PlacementHome::OwningRepo => {
            let Some(repo) = non_empty(context.owning_repo.as_deref()) else {
                return staged("runbook routing needs the owning repo".to_owned());
            };
            route(
                submission,
                &declaration,
                context,
                repo.to_owned(),
                "owning repo",
                context.canonical_identity.clone(),
                None,
            )
        }
        PlacementHome::IdGraph => classify_id_native(submission, &declaration, context, staged),
    }
}

fn classify_id_native(
    submission: &Submission,
    declaration: &EffectiveDeclaration,
    context: &RoutingContext,
    staged: impl Fn(String) -> Classification,
) -> Classification {
    if submission.sensitivity_ceiling == SensitivityCeiling::SecretBearing {
        return staged(
            "secret-bearing material is banned from every note home and is never ID-native (§5)"
                .to_owned(),
        );
    }
    let Some(eligibility) = &context.id_native else {
        return staged("ID-native routing needs the §5 eligibility evidence".to_owned());
    };
    if !eligibility.cited_across_systems {
        return staged(
            "§5 condition (a) unmet: the note is not meant to be cited and related across systems"
                .to_owned(),
        );
    }
    if !eligibility.stable_identity {
        return staged("§5 condition (b) unmet: no stable identity worth addressing".to_owned());
    }
    let Some(writer_set) = non_empty(eligibility.writer_set.as_deref()) else {
        return staged("§5 condition (c) unmet: no #5545 writer-set can be named".to_owned());
    };
    let Some(object_id) = non_empty(eligibility.object_id.as_deref()) else {
        return staged(
            "ID-native routing needs the graph object id as canonical identity".to_owned(),
        );
    };
    route(
        submission,
        declaration,
        context,
        ID_GRAPH_DESTINATION.to_owned(),
        "ID graph",
        Some(object_id.to_owned()),
        Some(format!(
            "graph object {object_id} is canonical and copies are projections; writer-set \
             {writer_set} under #5545 authority; publication format per #6062"
        )),
    )
}

/// Build the `Routed` verdict for a publishable home: resolve the
/// physical tier under the ceiling, then bind the destination, the
/// canonical identity (supplied for ID-native notes and by context
/// override, else `note:<submission id>`), and the submission's
/// content hash as the source revision/hash pin.
fn route(
    submission: &Submission,
    declaration: &EffectiveDeclaration,
    context: &RoutingContext,
    destination: String,
    destination_kind: &str,
    canonical_identity: Option<String>,
    rationale_note: Option<String>,
) -> Classification {
    let sensitivity =
        match resolve_physical_tier(submission.sensitivity_ceiling, context.requested_tier) {
            Ok(tier) => tier,
            Err(reason) => {
                return Classification::StaysStaged {
                    declaration: Some(*declaration),
                    reason,
                };
            }
        };
    let canonical_identity =
        canonical_identity.unwrap_or_else(|| format!("note:{}", submission.id));
    let mut rationale = format!(
        "§2 decision table: {:?} -> {:?} via {destination_kind}; Atlas {:?}; physical tier \
         {sensitivity:?} under {:?} ceiling",
        declaration.note_class,
        declaration.home,
        declaration.atlas_tier,
        submission.sensitivity_ceiling,
    );
    if let Some(note) = rationale_note {
        rationale.push_str("; ");
        rationale.push_str(&note);
    }
    Classification::Routed {
        declaration: *declaration,
        decision: PlacementDecision {
            destination,
            canonical_identity,
            source_revision: submission.content_sha256.clone(),
            sensitivity,
            rationale,
        },
    }
}

/// Resolve the Atlas physical tier for a routed note, applying the
/// source/caller sensitivity ceiling first (#7717; design §2 of the
/// 2026-11-08 placement record). The default is `internal`; `public`
/// and `secret` are explicit requests, never defaults. Ceilings are
/// not tier aliases: an internal ceiling forbids a public tier,
/// secret-bearing material requires an explicit secret tier, and
/// personal/local-only material is never routed by this pipeline.
fn resolve_physical_tier(
    ceiling: SensitivityCeiling,
    requested: Option<SensitivityTier>,
) -> Result<SensitivityTier, String> {
    match ceiling {
        SensitivityCeiling::Personal => Err(
            "personal-ceiling material stays in agent memory; a shareable extract needs its \
             own reviewed submission"
                .to_owned(),
        ),
        SensitivityCeiling::LocalOnly => {
            Err("local-only material never leaves its machine".to_owned())
        }
        SensitivityCeiling::SecretBearing => match requested {
            Some(SensitivityTier::Secret) => Ok(SensitivityTier::Secret),
            _ => Err("secret-bearing ceiling requires an explicit secret tier request".to_owned()),
        },
        SensitivityCeiling::Internal => match requested {
            Some(SensitivityTier::Public) => {
                Err("internal ceiling does not permit a public physical tier".to_owned())
            }
            Some(tier) => Ok(tier),
            None => Ok(SensitivityTier::Internal),
        },
        SensitivityCeiling::Shareable => Ok(requested.unwrap_or(SensitivityTier::Internal)),
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

/// Classify a stored submission and, when the verdict is `Routed`,
/// stage the classifier's decision for human approval. `NotPublished`
/// and `StaysStaged` verdicts change nothing: the submission remains
/// in the submission DB, unpublished. The returned verdict is the
/// audit record of why.
pub fn stage_classified<S: SubmissionStore>(
    store: &mut S,
    submission_id: &str,
    context: &RoutingContext,
    staged_at: impl Into<String>,
) -> Result<Classification, PipelineError> {
    let submission = store
        .submission(submission_id)
        .cloned()
        .ok_or_else(|| PipelineError::UnknownSubmission(submission_id.to_owned()))?;
    let classification = classify(&submission, context);
    if let Classification::Routed { decision, .. } = &classification {
        stage(store, submission_id, decision.clone(), staged_at)?;
    }
    Ok(classification)
}
