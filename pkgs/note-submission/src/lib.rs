//! Typed envelope and fail-closed staging/promotion gate for agent notes.
//!
//! This crate defines the pipeline contract and its table-shaped records. A
//! persistent adapter (the Turso submission DB is the system of record) can
//! implement [`SubmissionStore`]; this crate deliberately does not create a
//! competing local database or infer placement policy. Classification and the
//! destination are supplied by the caller, then must be confirmed by a named
//! human before [`promote`] can publish anything.

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
        // Source-classification ceilings are not Atlas tiers. These checks
        // are deliberately conservative until #7717 supplies the classifier.
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
