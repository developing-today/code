use note_submission::{
    CorpusControl, DestinationWriter, HumanApproval, MemoryStore, NoteOutStatus, PlacementDecision,
    Provenance, ProvenanceSpan, SensitivityCeiling, SensitivityTier, SubmissionState,
    SubmissionStore, UploadAttemptStatus, content_sha256, promote, stage, submit,
};

#[derive(Default)]
struct TestDestination {
    writes: Vec<(String, String, String)>,
}

impl DestinationWriter for TestDestination {
    fn publish(
        &mut self,
        destination: &str,
        canonical_identity: &str,
        content: &str,
    ) -> Result<(), String> {
        self.writes.push((
            destination.to_owned(),
            canonical_identity.to_owned(),
            content.to_owned(),
        ));
        Ok(())
    }
}

fn provenance(content: &str) -> Provenance {
    Provenance {
        source_system: "developing-today/code".to_owned(),
        machine: "laptop-1".to_owned(),
        captured_at: "2026-11-16T23:00:00Z".to_owned(),
        author: "agent-7".to_owned(),
        owner: "team-notes".to_owned(),
        spans: vec![ProvenanceSpan {
            source: "doc/notes/example.md".to_owned(),
            byte_range: Some((0, content.len() as u64)),
            content_sha256: content_sha256(content),
        }],
    }
}

fn placement() -> PlacementDecision {
    PlacementDecision {
        destination: "developing-today/zettel".to_owned(),
        canonical_identity: "note:2026-11-16-example".to_owned(),
        source_revision: "abc1234".to_owned(),
        sensitivity: SensitivityTier::Internal,
        rationale: "durable cross-project finding".to_owned(),
    }
}

#[test]
fn submit_stage_human_approve_and_promote_preserves_envelope_provenance() {
    // Oracle: the destination writer receives exactly the submitted content;
    // the published `notes_out` row pins the original SHA-256 and provenance;
    // the submission and attempt ledgers reach their corresponding terminal states.
    let mut store = MemoryStore::default();
    let content = "reviewed note body".to_owned();
    submit(
        &mut store,
        "sub-1",
        content.clone(),
        provenance(&content),
        SensitivityCeiling::Internal,
        "2026-11-16T23:01:00Z",
    )
    .unwrap();
    assert_eq!(
        store.submission("sub-1").unwrap().state,
        SubmissionState::Submitted
    );

    stage(&mut store, "sub-1", placement(), "2026-11-16T23:02:00Z").unwrap();
    assert_eq!(
        store.submission("sub-1").unwrap().state,
        SubmissionState::Staged
    );
    assert!(store.notes_out.is_empty(), "staging must not publish");

    store.corpus_controls.insert(
        "developing-today/zettel".to_owned(),
        CorpusControl {
            destination: "developing-today/zettel".to_owned(),
            enabled: true,
            manual_hold: false,
            updated_at: "2026-11-16T23:02:30Z".to_owned(),
        },
    );
    let approval = HumanApproval {
        human_id: "reviewer@example.test".to_owned(),
        approved_destination: "developing-today/zettel".to_owned(),
        approved_sensitivity: SensitivityTier::Internal,
        rationale: "placement and sensitivity reviewed".to_owned(),
        approved_at: "2026-11-16T23:03:00Z".to_owned(),
    };
    let mut destination = TestDestination::default();
    promote(&mut store, &mut destination, "sub-1", &approval).unwrap();

    assert_eq!(destination.writes.len(), 1);
    assert_eq!(destination.writes[0].0, "developing-today/zettel");
    assert_eq!(destination.writes[0].1, "note:2026-11-16-example");
    assert_eq!(destination.writes[0].2, content);
    let submission = store.submission("sub-1").unwrap();
    assert_eq!(submission.state, SubmissionState::Promoted);
    assert_eq!(
        submission.content_sha256,
        content_sha256(&submission.content)
    );
    assert_eq!(submission.provenance, provenance(&submission.content));
    assert!(store.staged("sub-1").is_none());
    assert_eq!(store.notes_out.len(), 1);
    assert_eq!(store.notes_out[0].source_sha256, submission.content_sha256);
    assert_eq!(store.notes_out[0].promoted_by, "reviewer@example.test");
    assert_eq!(store.notes_out[0].status, NoteOutStatus::Published);
    assert_eq!(store.upload_attempts.len(), 1);
    assert_eq!(
        store.upload_attempts[0].status,
        UploadAttemptStatus::Published
    );
}

#[test]
fn missing_or_held_control_fails_closed_and_records_attempt() {
    // Oracle: a missing control prevents any destination write and leaves the
    // candidate staged; once configured, manual_hold has the same behavior.
    let mut store = MemoryStore::default();
    let content = "candidate".to_owned();
    submit(
        &mut store,
        "sub-2",
        content.clone(),
        provenance(&content),
        SensitivityCeiling::Internal,
        "t0",
    )
    .unwrap();
    stage(&mut store, "sub-2", placement(), "t1").unwrap();
    let approval = HumanApproval {
        human_id: "reviewer".to_owned(),
        approved_destination: "developing-today/zettel".to_owned(),
        approved_sensitivity: SensitivityTier::Internal,
        rationale: "approved".to_owned(),
        approved_at: "t2".to_owned(),
    };
    let mut destination = TestDestination::default();
    assert!(promote(&mut store, &mut destination, "sub-2", &approval).is_err());
    assert!(destination.writes.is_empty());
    assert_eq!(
        store.upload_attempts[0].status,
        UploadAttemptStatus::HeldByControl
    );

    store.corpus_controls.insert(
        "developing-today/zettel".to_owned(),
        CorpusControl {
            destination: "developing-today/zettel".to_owned(),
            enabled: true,
            manual_hold: true,
            updated_at: "t3".to_owned(),
        },
    );
    assert!(promote(&mut store, &mut destination, "sub-2", &approval).is_err());
    assert!(destination.writes.is_empty());
    assert_eq!(
        store.upload_attempts[1].status,
        UploadAttemptStatus::HeldByControl
    );
    assert_eq!(
        store.submission("sub-2").unwrap().state,
        SubmissionState::Staged
    );
}

#[test]
fn approval_must_match_the_staged_route_and_tier() {
    // Oracle: a mismatched human decision is rejected, audited, and cannot call
    // the destination writer.
    let mut store = MemoryStore::default();
    let content = "candidate".to_owned();
    submit(
        &mut store,
        "sub-3",
        content.clone(),
        provenance(&content),
        SensitivityCeiling::Internal,
        "t0",
    )
    .unwrap();
    stage(&mut store, "sub-3", placement(), "t1").unwrap();
    store.corpus_controls.insert(
        "developing-today/zettel".to_owned(),
        CorpusControl {
            destination: "developing-today/zettel".to_owned(),
            enabled: true,
            manual_hold: false,
            updated_at: "t1".to_owned(),
        },
    );
    let approval = HumanApproval {
        human_id: "reviewer".to_owned(),
        approved_destination: "other-corpus".to_owned(),
        approved_sensitivity: SensitivityTier::Internal,
        rationale: "wrong route".to_owned(),
        approved_at: "t2".to_owned(),
    };
    let mut destination = TestDestination::default();
    assert!(promote(&mut store, &mut destination, "sub-3", &approval).is_err());
    assert!(destination.writes.is_empty());
    assert_eq!(
        store.upload_attempts[0].status,
        UploadAttemptStatus::Rejected
    );
    assert_eq!(
        store.submission("sub-3").unwrap().state,
        SubmissionState::Staged
    );
}

#[test]
fn source_sensitivity_ceiling_is_not_a_physical_tier_alias() {
    // Oracle: the policy guard does not let an internal source be staged as
    // public, and personal/local-only source classes cannot be promoted.
    let mut store = MemoryStore::default();
    let content = "internal candidate".to_owned();
    submit(
        &mut store,
        "sub-4",
        content.clone(),
        provenance(&content),
        SensitivityCeiling::Internal,
        "t0",
    )
    .unwrap();
    let public = PlacementDecision {
        sensitivity: SensitivityTier::Public,
        ..placement()
    };
    assert!(stage(&mut store, "sub-4", public, "t1").is_err());
    assert_eq!(
        store.submission("sub-4").unwrap().state,
        SubmissionState::Submitted
    );
    assert!(store.staged("sub-4").is_none());
}
