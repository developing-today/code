//! Journal checkpoint reference and handoff snapshot — JRN-03
//! (developing-today/code#9465; design parent #9208, program #9196).
//!
//! This module owns the checkpoint schema/projection named by
//! `components/9208-typed-dependencies-notes-journal-field-authority.md`
//! §3.4: a durable, append-only snapshot reference a later session
//! resumes from. A checkpoint is not a transcript and not a
//! completion claim. It carries the objective, the steps done and
//! still outstanding, the important paths, the branch and commit it
//! was taken at, the test state at that moment, the hard blocker
//! edges, the per-source activity cursor refs and hydration ref it
//! was hydrated from, and one suggested next action.
//!
//! Boundaries kept, per the design:
//!
//! * **Private by construction.** Checkpoints live in the Fleet
//!   journal; their visibility is `private`. A payload presenting
//!   any other visibility is rejected at validation, never silently
//!   projected (§3.1: private notes, findings and checkpoints never
//!   auto-mirror).
//! * **Freshness is explicit.** A checkpoint names the per-work
//!   journal sequence it was taken at. Projecting it against a
//!   newer stream sequence — or after a superseding checkpoint
//!   exists — yields [`CheckpointFreshness::Stale`]; a stale
//!   checkpoint never projects as current.
//! * **Handoff never implies completion.** [`HandoffRef`] transfers
//!   ownership and records the lease release/takeover result; it
//!   has no completion field and its projection leaves the
//!   workflow state untouched.
//! * **Foreign surfaces stay opaque.** Cursor refs (#9207),
//!   hydration generations (#9207) and blocker edge targets are
//!   carried as typed references, not re-implemented here; the
//!   lifecycle command that writes checkpoints remains #9206.
//!
//! The module is a pure model: no I/O, no storage, no queue. A
//! rejection is a typed [`CheckpointError`] that mutates nothing.

use serde::{Deserialize, Serialize};

use super::contracts::{is_canonical_decimal, ContractError, WorkRef};
use super::lifecycle::WorkflowState;

/// Wire schema identifier of the journal checkpoint reference.
pub const CHECKPOINT_SCHEMA_VERSION: &str = "fleet.journal-checkpoint/v1";
/// Wire schema identifier of the handoff snapshot reference.
pub const HANDOFF_SCHEMA_VERSION: &str = "fleet.journal-handoff/v1";

/// A typed checkpoint-contract failure. Every variant is
/// non-mutating: a value of this type is the whole outcome of a
/// rejected parse or validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointError {
    /// The payload does not match the v1 schema shapes.
    InvalidSchema {
        /// What was wrong, in one line.
        detail: String,
    },
    /// A schema major version this contract does not support.
    UnsupportedVersion {
        /// The version string that was presented.
        schema_version: String,
    },
    /// A checkpoint presented with a visibility other than
    /// `private`; checkpoints never project publicly.
    NonPrivateVisibility {
        /// The visibility spelling that was presented.
        visibility: String,
    },
    /// An important-path entry that names a secret-bearing
    /// location; checkpoints store no secrets and do not point at
    /// credential material.
    SecretPath {
        /// The offending path, verbatim.
        path: String,
    },
    /// The checkpoint carries no commit identity (empty branch or
    /// head SHA); a snapshot that cannot name its commit cannot be
    /// resumed or audited.
    MissingCommitIdentity,
    /// One test name appears in two outcome buckets (for example
    /// both pending and passed); the state is contradictory and a
    /// pending test must never be shown passed.
    ConflictingTestState {
        /// The test named by two buckets.
        name: String,
    },
}

impl CheckpointError {
    /// Stable machine-readable code for this failure.
    pub fn code(&self) -> &'static str {
        match self {
            CheckpointError::InvalidSchema { .. } => "invalid_schema",
            CheckpointError::UnsupportedVersion { .. } => "unsupported_version",
            CheckpointError::NonPrivateVisibility { .. } => "non_private_visibility",
            CheckpointError::SecretPath { .. } => "secret_path",
            CheckpointError::MissingCommitIdentity => "missing_commit_identity",
            CheckpointError::ConflictingTestState { .. } => "conflicting_test_state",
        }
    }
}

impl From<ContractError> for CheckpointError {
    fn from(error: ContractError) -> Self {
        match error {
            ContractError::InvalidSchema { detail } => {
                CheckpointError::InvalidSchema { detail }
            }
            ContractError::UnsupportedVersion { schema_version } => {
                CheckpointError::UnsupportedVersion { schema_version }
            }
            other => CheckpointError::InvalidSchema {
                detail: format!("work identity rejected: {}", other.code()),
            },
        }
    }
}

fn invalid(detail: impl Into<String>) -> CheckpointError {
    CheckpointError::InvalidSchema {
        detail: detail.into(),
    }
}

/// Journal visibility vocabulary (design §3.2). Checkpoints accept
/// only [`JournalVisibility::Private`]; the wider vocabulary exists
/// so a foreign visibility is a typed rejection, not a parse guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalVisibility {
    /// Fleet-private; authorized readers only.
    Private,
    /// Internal to the fleet deployment.
    Internal,
    /// An explicitly authorized public projection.
    PublicProjection,
}

impl JournalVisibility {
    /// The wire spelling of this visibility.
    pub fn as_str(self) -> &'static str {
        match self {
            JournalVisibility::Private => "private",
            JournalVisibility::Internal => "internal",
            JournalVisibility::PublicProjection => "public_projection",
        }
    }
}

/// The commit a checkpoint was taken at: branch plus head SHA.
/// Both halves are required — a checkpoint with no commit identity
/// cannot be resumed against the code it describes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitRef {
    /// Branch the work was on when the checkpoint was taken.
    pub branch: String,
    /// Head commit SHA (40-hex SHA-1 or 64-hex SHA-256).
    pub head_sha: String,
}

impl CommitRef {
    /// Validate the commit identity.
    pub fn validate(&self) -> Result<(), CheckpointError> {
        if self.branch.is_empty() || self.head_sha.is_empty() {
            return Err(CheckpointError::MissingCommitIdentity);
        }
        let sha_ok = (self.head_sha.len() == 40 || self.head_sha.len() == 64)
            && self
                .head_sha
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
        if !sha_ok {
            return Err(invalid(format!(
                "head_sha is not a lowercase hex commit SHA: {:?}",
                self.head_sha
            )));
        }
        Ok(())
    }
}

/// One per-source activity cursor reference (#9207 owns the cursor
/// format; the checkpoint carries the reference opaquely so a
/// resume re-reads exactly the streams the checkpoint saw).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CursorRef {
    /// Activity source the cursor belongs to (for example
    /// `github-comments`); unique within one checkpoint.
    pub source: String,
    /// The cursor value as issued by the source's owner.
    pub cursor: String,
}

/// A reference to the hydration generation a checkpoint was
/// hydrated from (#9207 owns the manifest; this is the ref only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HydrationRef {
    /// Hydration generation, a canonical decimal string.
    pub generation: String,
    /// Digest of the hydration manifest, when recorded.
    #[serde(default)]
    pub manifest_digest: Option<String>,
}

/// One hard blocker edge carried by a checkpoint: the edge's own
/// identity plus the target it blocks on, kept as an opaque
/// reference so dependency semantics stay with their owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockerEdgeRef {
    /// The dependency edge's identifier.
    pub edge_id: String,
    /// What the edge blocks on (for example `fleet:761`).
    pub target: String,
    /// Whether the edge is hard (work cannot proceed past it).
    pub hard: bool,
}

/// The outcome of one named test at checkpoint time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestOutcome {
    /// The test ran and passed.
    Passed,
    /// The test has not run yet, or has not finished.
    Pending,
    /// The test ran and failed.
    Failed,
}

/// Test state at checkpoint time: which tests ran, and how each
/// ended. The three buckets are disjoint by validation — a test
/// that is still pending is never also recorded as passed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestState {
    /// Reference to the test run these results came from.
    #[serde(default)]
    pub run_ref: Option<String>,
    /// Names of tests that ran and passed.
    #[serde(default)]
    pub passed: Vec<String>,
    /// Names of tests not yet run or not yet finished.
    #[serde(default)]
    pub pending: Vec<String>,
    /// Names of tests that ran and failed.
    #[serde(default)]
    pub failed: Vec<String>,
}

impl TestState {
    /// Validate bucket disjointness.
    pub fn validate(&self) -> Result<(), CheckpointError> {
        for name in &self.passed {
            if self.pending.contains(name) || self.failed.contains(name) {
                return Err(CheckpointError::ConflictingTestState { name: name.clone() });
            }
        }
        for name in &self.pending {
            if self.failed.contains(name) {
                return Err(CheckpointError::ConflictingTestState { name: name.clone() });
            }
        }
        Ok(())
    }

    /// The recorded outcome of one test, if it is in any bucket.
    pub fn outcome_of(&self, name: &str) -> Option<TestOutcome> {
        if self.passed.iter().any(|n| n == name) {
            Some(TestOutcome::Passed)
        } else if self.pending.iter().any(|n| n == name) {
            Some(TestOutcome::Pending)
        } else if self.failed.iter().any(|n| n == name) {
            Some(TestOutcome::Failed)
        } else {
            None
        }
    }

    /// How many tests are recorded passed. Pending tests are never
    /// counted here.
    pub fn passed_count(&self) -> usize {
        self.passed.len()
    }
}

/// Screen one important-path entry: checkpoints name code and
/// notes, never credential material. The screen is conservative —
/// it rejects known secret-bearing locations and file shapes, in
/// any path segment.
fn screen_path(path: &str) -> Result<(), CheckpointError> {
    let lowered = path.to_ascii_lowercase();
    let secret_segment = |segment: &str| {
        matches!(segment, ".ssh" | ".aws" | ".gnupg" | ".bitwarden")
            || segment.starts_with(".env")
            || segment.contains("secret")
            || segment.contains("credential")
            || segment.starts_with("id_rsa")
            || segment.starts_with("id_ed25519")
            || segment.ends_with(".pem")
            || segment.ends_with(".key")
            || segment.ends_with(".p12")
    };
    if lowered.split(['/', '\\']).any(secret_segment) {
        return Err(CheckpointError::SecretPath {
            path: path.to_string(),
        });
    }
    Ok(())
}

/// A journal checkpoint reference (design §3.4): the durable
/// snapshot a later session resumes from. Append-only — editing a
/// checkpoint produces a new checkpoint whose `supersedes` names
/// this one; the original stays addressable and unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointRef {
    /// `fleet.journal-checkpoint/v1`.
    pub schema_version: String,
    /// This checkpoint's own identifier.
    pub checkpoint_id: String,
    /// The work item the checkpoint belongs to.
    pub work: WorkRef,
    /// Per-work journal stream sequence this checkpoint was taken
    /// at, a canonical decimal string (design §3.2/§3.3.1).
    pub sequence: String,
    /// What the work is trying to achieve.
    pub objective: String,
    /// Steps completed when the checkpoint was taken.
    #[serde(default)]
    pub completed_steps: Vec<String>,
    /// Work still outstanding at checkpoint time.
    #[serde(default)]
    pub outstanding_work: Vec<String>,
    /// Paths a resuming session should look at first. Screened:
    /// secret-bearing paths are rejected.
    #[serde(default)]
    pub important_paths: Vec<String>,
    /// Branch and head commit the checkpoint was taken at.
    pub commit: CommitRef,
    /// Test state at checkpoint time.
    pub tests: TestState,
    /// Hard blocker edges in force at checkpoint time.
    #[serde(default)]
    pub blockers: Vec<BlockerEdgeRef>,
    /// Per-source activity cursor refs at checkpoint time.
    #[serde(default)]
    pub cursors: Vec<CursorRef>,
    /// Hydration generation ref, when the checkpoint was hydrated.
    #[serde(default)]
    pub hydration: Option<HydrationRef>,
    /// The one suggested next action for the resuming session.
    pub next_action: String,
    /// Session that produced the checkpoint, when known.
    #[serde(default)]
    pub session_ref: Option<String>,
    /// Worker/lane that owned the work at checkpoint time.
    #[serde(default)]
    pub worker_ref: Option<String>,
    /// Authority timestamp of the checkpoint's creation.
    pub created_at: String,
    /// The checkpoint this one supersedes, when it is a correction
    /// or refresh of an earlier snapshot.
    #[serde(default)]
    pub supersedes: Option<String>,
    /// Always `private` for a checkpoint; validated, never assumed.
    pub visibility: JournalVisibility,
}

impl CheckpointRef {
    /// Validate the checkpoint against the JRN-03 contract.
    pub fn validate(&self) -> Result<(), CheckpointError> {
        if self.schema_version != CHECKPOINT_SCHEMA_VERSION {
            if self.schema_version.starts_with("fleet.journal-checkpoint/") {
                return Err(CheckpointError::UnsupportedVersion {
                    schema_version: self.schema_version.clone(),
                });
            }
            return Err(invalid(format!(
                "schema_version is not {CHECKPOINT_SCHEMA_VERSION}: {:?}",
                self.schema_version
            )));
        }
        if self.checkpoint_id.is_empty() {
            return Err(invalid("checkpoint_id must not be empty"));
        }
        self.work.validate()?;
        if !is_canonical_decimal(&self.sequence) {
            return Err(invalid(format!(
                "sequence is not a canonical decimal string: {:?}",
                self.sequence
            )));
        }
        if self.objective.is_empty() {
            return Err(invalid("objective must not be empty"));
        }
        if self.next_action.is_empty() {
            return Err(invalid(
                "next_action must not be empty: a checkpoint exists to be resumed",
            ));
        }
        if self.created_at.is_empty() {
            return Err(invalid("created_at must not be empty"));
        }
        for path in &self.important_paths {
            screen_path(path)?;
        }
        self.commit.validate()?;
        self.tests.validate()?;
        let mut sources: Vec<&str> = Vec::with_capacity(self.cursors.len());
        for cursor in &self.cursors {
            if cursor.source.is_empty() || cursor.cursor.is_empty() {
                return Err(invalid("cursor refs need a source and a cursor value"));
            }
            if sources.contains(&cursor.source.as_str()) {
                return Err(invalid(format!(
                    "duplicate cursor source in one checkpoint: {:?}",
                    cursor.source
                )));
            }
            sources.push(&cursor.source);
        }
        if let Some(hydration) = &self.hydration {
            if !is_canonical_decimal(&hydration.generation) {
                return Err(invalid(format!(
                    "hydration generation is not a canonical decimal string: {:?}",
                    hydration.generation
                )));
            }
        }
        if self.supersedes.as_deref() == Some(self.checkpoint_id.as_str()) {
            return Err(invalid("a checkpoint cannot supersede itself"));
        }
        if self.visibility != JournalVisibility::Private {
            return Err(CheckpointError::NonPrivateVisibility {
                visibility: self.visibility.as_str().to_string(),
            });
        }
        Ok(())
    }

    /// Serialize to the wire JSON shape.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("checkpoint serialization cannot fail")
    }

    /// Parse and validate a wire payload. Unknown major schema
    /// versions and non-private visibility are typed rejections,
    /// never best-effort projections.
    pub fn from_json(text: &str) -> Result<Self, CheckpointError> {
        let checkpoint: CheckpointRef = serde_json::from_str(text).map_err(|error| {
            CheckpointError::InvalidSchema {
                detail: format!("checkpoint JSON does not parse: {error}"),
            }
        })?;
        checkpoint.validate()?;
        Ok(checkpoint)
    }

    /// Assess this checkpoint's freshness against the work
    /// stream's current sequence and any superseding checkpoint.
    /// The checkpoint is current only when nothing has superseded
    /// it and the stream has not moved past it; every other case
    /// is explicitly stale, with the numbers carried on the value.
    pub fn assess_freshness(
        &self,
        current_sequence: &str,
        superseded_by: Option<&str>,
    ) -> CheckpointFreshness {
        if superseded_by.is_none() && self.sequence == current_sequence {
            CheckpointFreshness::Current
        } else {
            CheckpointFreshness::Stale {
                checkpoint_sequence: self.sequence.clone(),
                current_sequence: current_sequence.to_string(),
                superseded_by: superseded_by.map(str::to_string),
            }
        }
    }

    /// Build the resumption projection a new session starts from.
    /// Freshness travels on the projection: a stale checkpoint
    /// projects as stale, so no consumer can mistake it for the
    /// current state of the work.
    pub fn project_resume(
        &self,
        current_sequence: &str,
        superseded_by: Option<&str>,
    ) -> ResumeProjection {
        ResumeProjection {
            checkpoint_id: self.checkpoint_id.clone(),
            work: self.work.clone(),
            objective: self.objective.clone(),
            outstanding_work: self.outstanding_work.clone(),
            important_paths: self.important_paths.clone(),
            commit: self.commit.clone(),
            cursors: self.cursors.clone(),
            next_action: self.next_action.clone(),
            freshness: self.assess_freshness(current_sequence, superseded_by),
        }
    }
}

/// Freshness of a checkpoint relative to the live work stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointFreshness {
    /// The checkpoint is the stream's latest word on the work.
    Current,
    /// The stream moved on, or a newer checkpoint superseded this
    /// one; the values say by how much and by whom.
    Stale {
        /// Sequence the checkpoint was taken at.
        checkpoint_sequence: String,
        /// Sequence the stream is at now.
        current_sequence: String,
        /// The superseding checkpoint, when one exists.
        superseded_by: Option<String>,
    },
}

/// The resumption shape projected from a checkpoint (design
/// §3.4's "resumption shape/freshness" doc surface): what a new
/// session needs to continue, with freshness stated, never implied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeProjection {
    /// Checkpoint the projection was built from.
    pub checkpoint_id: String,
    /// Work item being resumed.
    pub work: WorkRef,
    /// Objective carried from the checkpoint.
    pub objective: String,
    /// Work still outstanding at checkpoint time.
    pub outstanding_work: Vec<String>,
    /// Paths to look at first.
    pub important_paths: Vec<String>,
    /// Commit to resume against.
    pub commit: CommitRef,
    /// Cursor refs to re-read activity from.
    pub cursors: Vec<CursorRef>,
    /// Suggested next action.
    pub next_action: String,
    /// Whether the checkpoint is still the stream's latest word.
    pub freshness: CheckpointFreshness,
}

/// The lease side of a handoff: what happened to the old owner's
/// lease when ownership moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaseTransfer {
    /// The old owner released the lease.
    Released,
    /// The new owner's takeover was confirmed.
    TakeoverConfirmed,
    /// Transfer initiated; neither release nor takeover confirmed.
    Pending,
}

/// A handoff snapshot reference: ownership of a work item moved
/// from one owner to another, anchored to the checkpoint the new
/// owner resumes from. A handoff records the transfer only — it
/// deliberately has no completion field, and projecting it over a
/// workflow state returns that state unchanged: being handed off
/// is not being done.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffRef {
    /// `fleet.journal-handoff/v1`.
    pub schema_version: String,
    /// This handoff's own identifier.
    pub handoff_id: String,
    /// The work item whose ownership moved.
    pub work: WorkRef,
    /// The checkpoint the new owner resumes from.
    pub checkpoint_id: String,
    /// Previous owner, when known.
    #[serde(default)]
    pub from_owner: Option<String>,
    /// New owner.
    pub to_owner: String,
    /// Lease release/takeover result.
    pub lease: LeaseTransfer,
    /// Authority timestamp of the handoff.
    pub created_at: String,
}

impl HandoffRef {
    /// Validate the handoff reference.
    pub fn validate(&self) -> Result<(), CheckpointError> {
        if self.schema_version != HANDOFF_SCHEMA_VERSION {
            if self.schema_version.starts_with("fleet.journal-handoff/") {
                return Err(CheckpointError::UnsupportedVersion {
                    schema_version: self.schema_version.clone(),
                });
            }
            return Err(invalid(format!(
                "schema_version is not {HANDOFF_SCHEMA_VERSION}: {:?}",
                self.schema_version
            )));
        }
        if self.handoff_id.is_empty() || self.checkpoint_id.is_empty() {
            return Err(invalid("handoff_id and checkpoint_id must not be empty"));
        }
        self.work.validate()?;
        if self.to_owner.is_empty() {
            return Err(invalid("to_owner must not be empty"));
        }
        if self.created_at.is_empty() {
            return Err(invalid("created_at must not be empty"));
        }
        Ok(())
    }

    /// Serialize to the wire JSON shape.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("handoff serialization cannot fail")
    }

    /// Parse and validate a wire payload.
    pub fn from_json(text: &str) -> Result<Self, CheckpointError> {
        let handoff: HandoffRef = serde_json::from_str(text).map_err(|error| {
            CheckpointError::InvalidSchema {
                detail: format!("handoff JSON does not parse: {error}"),
            }
        })?;
        handoff.validate()?;
        Ok(handoff)
    }

    /// Project this handoff over a workflow state. Ownership moved;
    /// the workflow dimension did not. In particular an active item
    /// stays active — a handoff never marks work completed.
    pub fn projected_workflow_state(&self, current: WorkflowState) -> WorkflowState {
        current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_checkpoint() -> CheckpointRef {
        CheckpointRef {
            schema_version: CHECKPOINT_SCHEMA_VERSION.to_string(),
            checkpoint_id: "ckpt-842-18".to_string(),
            work: WorkRef::new("local", "842").unwrap(),
            sequence: "18".to_string(),
            objective: "Land the checkpoint schema slice".to_string(),
            completed_steps: vec!["design read".to_string(), "module drafted".to_string()],
            outstanding_work: vec!["wire the lifecycle command".to_string()],
            important_paths: vec![
                "pkgs/fleet/crates/fleet-queue/src/work/journal.rs".to_string(),
                "doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/components/9208-typed-dependencies-notes-journal-field-authority.md".to_string(),
            ],
            commit: CommitRef {
                branch: "felix/9465-next-slice".to_string(),
                head_sha: "ac59375f7e111111111111111111111111111111".to_string(),
            },
            tests: TestState {
                run_ref: Some("run-4538".to_string()),
                passed: vec![
                    "pmo_journal_checkpoint_round_trip_preserves_cursor_and_commit_refs"
                        .to_string(),
                ],
                pending: vec!["pmo_journal_checkpoint_stale_checkpoint_never_projects_current".to_string()],
                failed: vec![],
            },
            blockers: vec![BlockerEdgeRef {
                edge_id: "edge-842-761".to_string(),
                target: "fleet:761".to_string(),
                hard: true,
            }],
            cursors: vec![
                CursorRef {
                    source: "github-comments".to_string(),
                    cursor: "comment-cursor-1042".to_string(),
                },
                CursorRef {
                    source: "github-reviews".to_string(),
                    cursor: "review-cursor-77".to_string(),
                },
                CursorRef {
                    source: "queue-events".to_string(),
                    cursor: "queue-cursor-9001".to_string(),
                },
            ],
            hydration: Some(HydrationRef {
                generation: "3".to_string(),
                manifest_digest: Some("sha256:manifest-3".to_string()),
            }),
            next_action: "Run the exact-filter done command remotely".to_string(),
            session_ref: Some("session-opaque-1".to_string()),
            worker_ref: Some("felix".to_string()),
            created_at: "2026-11-24T12:31:46Z".to_string(),
            supersedes: None,
            visibility: JournalVisibility::Private,
        }
    }

    /// JRN-03 oracle: a checkpoint reference round-trips through
    /// its wire shape with cursor refs and the commit ref preserved
    /// exactly, and its visibility is asserted private — on the
    /// wire, in the parsed value, and against a tampered payload
    /// that tries to present the checkpoint publicly.
    #[test]
    fn pmo_journal_checkpoint_round_trip_preserves_cursor_and_commit_refs() {
        let checkpoint = fixture_checkpoint();
        checkpoint.validate().unwrap();

        let json = checkpoint.to_json();
        // Private-visibility assertion, on the wire.
        assert!(
            json.contains("\"visibility\":\"private\""),
            "checkpoint must serialize as private: {json}"
        );

        let parsed = CheckpointRef::from_json(&json).unwrap();
        assert_eq!(parsed, checkpoint);
        assert_eq!(parsed.to_json(), json, "round trip must be byte-exact");

        // Cursor refs survive exactly: same sources, same values,
        // same order — a resume re-reads from precisely these.
        assert_eq!(parsed.cursors, checkpoint.cursors);
        assert_eq!(parsed.cursors.len(), 3);
        assert_eq!(parsed.cursors[0].source, "github-comments");
        assert_eq!(parsed.cursors[0].cursor, "comment-cursor-1042");
        assert_eq!(parsed.cursors[2].source, "queue-events");
        assert_eq!(parsed.cursors[2].cursor, "queue-cursor-9001");

        // The commit ref survives exactly: branch and head SHA.
        assert_eq!(parsed.commit, checkpoint.commit);
        assert_eq!(parsed.commit.branch, "felix/9465-next-slice");
        assert_eq!(
            parsed.commit.head_sha,
            "ac59375f7e111111111111111111111111111111"
        );

        // A payload presenting the checkpoint as a public
        // projection is a typed rejection, not a projection.
        let tampered = json.replace(
            "\"visibility\":\"private\"",
            "\"visibility\":\"public_projection\"",
        );
        let error = CheckpointRef::from_json(&tampered).unwrap_err();
        assert_eq!(error.code(), "non_private_visibility");
    }

    /// Negative: a checkpoint the stream has moved past — or one a
    /// newer checkpoint superseded — is stale, with the evidence on
    /// the value; it never projects as current.
    #[test]
    fn pmo_journal_checkpoint_stale_checkpoint_never_projects_current() {
        let checkpoint = fixture_checkpoint();

        assert_eq!(
            checkpoint.assess_freshness("18", None),
            CheckpointFreshness::Current
        );
        assert_eq!(
            checkpoint.assess_freshness("19", None),
            CheckpointFreshness::Stale {
                checkpoint_sequence: "18".to_string(),
                current_sequence: "19".to_string(),
                superseded_by: None,
            }
        );
        // Superseded at the same sequence is still stale.
        let superseded = checkpoint.assess_freshness("18", Some("ckpt-842-19"));
        assert!(matches!(
            superseded,
            CheckpointFreshness::Stale {
                superseded_by: Some(ref by),
                ..
            } if by == "ckpt-842-19"
        ));
        // The resume projection carries the staleness with it.
        let projection = checkpoint.project_resume("19", None);
        assert!(matches!(
            projection.freshness,
            CheckpointFreshness::Stale { .. }
        ));
        assert_eq!(projection.next_action, checkpoint.next_action);
    }

    /// Negative: a checkpoint may not point a resuming session at
    /// credential material.
    #[test]
    fn pmo_journal_checkpoint_secret_path_is_rejected() {
        for secret in [
            "~/.ssh/id_ed25519",
            "home/user/.aws/credentials",
            "config/.env.production",
            "certs/server.pem",
            "notes/credential-store.txt",
        ] {
            let mut checkpoint = fixture_checkpoint();
            checkpoint.important_paths = vec![secret.to_string()];
            let error = checkpoint.validate().unwrap_err();
            assert_eq!(error.code(), "secret_path", "path: {secret}");
        }
        // Ordinary code and doc paths pass the screen.
        fixture_checkpoint().validate().unwrap();
    }

    /// Negative: a snapshot that cannot name its commit cannot be
    /// validated — empty identity is the typed missing-identity
    /// failure, a malformed SHA is a schema failure.
    #[test]
    fn pmo_journal_checkpoint_missing_commit_identity_is_rejected() {
        let mut no_sha = fixture_checkpoint();
        no_sha.commit.head_sha = String::new();
        assert_eq!(
            no_sha.validate().unwrap_err().code(),
            "missing_commit_identity"
        );

        let mut no_branch = fixture_checkpoint();
        no_branch.commit.branch = String::new();
        assert_eq!(
            no_branch.validate().unwrap_err().code(),
            "missing_commit_identity"
        );

        let mut bad_sha = fixture_checkpoint();
        bad_sha.commit.head_sha = "not-a-sha".to_string();
        assert_eq!(bad_sha.validate().unwrap_err().code(), "invalid_schema");
    }

    /// Negative: a test recorded pending is never shown passed —
    /// naming it in both buckets is a contradiction the contract
    /// rejects, and the passed count never includes pending tests.
    #[test]
    fn pmo_journal_checkpoint_pending_test_is_never_shown_passed() {
        let checkpoint = fixture_checkpoint();
        let pending_name =
            "pmo_journal_checkpoint_stale_checkpoint_never_projects_current";
        assert_eq!(
            checkpoint.tests.outcome_of(pending_name),
            Some(TestOutcome::Pending)
        );
        assert_eq!(checkpoint.tests.passed_count(), 1);

        let mut conflicting = fixture_checkpoint();
        conflicting.tests.passed.push(pending_name.to_string());
        let error = conflicting.validate().unwrap_err();
        assert_eq!(error.code(), "conflicting_test_state");
    }

    /// The handoff half of JRN-03: a handoff round-trips, carries
    /// no completion claim on the wire, and projecting it over any
    /// workflow state returns that state unchanged — handed-off
    /// work is not done work.
    #[test]
    fn pmo_journal_checkpoint_handoff_never_implies_completion() {
        let handoff = HandoffRef {
            schema_version: HANDOFF_SCHEMA_VERSION.to_string(),
            handoff_id: "handoff-842-1".to_string(),
            work: WorkRef::new("local", "842").unwrap(),
            checkpoint_id: "ckpt-842-18".to_string(),
            from_owner: Some("lane-a".to_string()),
            to_owner: "felix".to_string(),
            lease: LeaseTransfer::Released,
            created_at: "2026-11-24T12:31:46Z".to_string(),
        };
        handoff.validate().unwrap();
        let json = handoff.to_json();
        assert!(!json.contains("completed"), "handoff wire: {json}");
        assert_eq!(HandoffRef::from_json(&json).unwrap(), handoff);

        for state in WorkflowState::ALL {
            assert_eq!(handoff.projected_workflow_state(state), state);
        }
        assert_eq!(
            handoff.projected_workflow_state(WorkflowState::Active),
            WorkflowState::Active
        );
    }

    /// Migration: an earlier v1 payload written before the
    /// hydration/supersedes/run refs existed stays readable —
    /// absent optional refs parse as absent, nothing is invented.
    #[test]
    fn pmo_journal_checkpoint_earlier_payload_without_optional_refs_stays_readable() {
        let mut value: serde_json::Value =
            serde_json::from_str(&fixture_checkpoint().to_json()).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("hydration");
        object.remove("supersedes");
        object.remove("session_ref");
        object.remove("worker_ref");
        object
            .get_mut("tests")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("run_ref");

        let parsed = CheckpointRef::from_json(&value.to_string()).unwrap();
        assert_eq!(parsed.hydration, None);
        assert_eq!(parsed.supersedes, None);
        assert_eq!(parsed.session_ref, None);
        assert_eq!(parsed.tests.run_ref, None);
        assert_eq!(parsed.cursors.len(), 3);
    }
}
