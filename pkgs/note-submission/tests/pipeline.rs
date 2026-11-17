use note_submission::{
    AtlasTier, CorpusControl, DestinationWriter, HumanApproval, MemoryStore, NoteClass,
    NoteOutStatus, PlacementDecision, PlacementDeclaration, PlacementHome, Provenance,
    ProvenanceSpan, SensitivityCeiling, SensitivityTier, SubmissionState, SubmissionStore,
    UploadAttemptStatus, content_sha256, promote, stage, submit, submit_with_declaration,
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

#[test]
fn declared_finding_defaults_to_zettel_tier_1_and_staging_shows_it() {
    // Oracle (#8204, design §2 + §7 item 1): a submission declaring
    // the durable-finding class with no intended tier gets the
    // decision table's defaults — home zettel, Atlas Tier 1 — and
    // the staged record a promoter reviews carries that effective
    // declaration next to the routing decision.
    let mut store = MemoryStore::default();
    let content = "verified finding".to_owned();
    submit_with_declaration(
        &mut store,
        "sub-decl-1",
        content.clone(),
        provenance(&content),
        SensitivityCeiling::Internal,
        PlacementDeclaration {
            note_class: NoteClass::DurableFinding,
            atlas_tier: None,
        },
        "2026-11-16T23:01:00Z",
    )
    .unwrap();
    let effective = store
        .submission("sub-decl-1")
        .unwrap()
        .effective_declaration()
        .expect("declared submission has an effective declaration");
    assert_eq!(effective.note_class, NoteClass::DurableFinding);
    assert_eq!(effective.home, PlacementHome::Zettel);
    assert_eq!(effective.atlas_tier, AtlasTier::Tier1);
    assert!(effective.tier_was_defaulted);

    stage(
        &mut store,
        "sub-decl-1",
        placement(),
        "2026-11-16T23:02:00Z",
    )
    .unwrap();
    assert_eq!(
        store.staged("sub-decl-1").unwrap().declaration,
        Some(effective)
    );
}

#[test]
fn declared_tier_overrides_table_default_and_undeclared_keeps_prior_behaviour() {
    // Oracle (#8204): an intended tier replaces the class default in
    // the effective declaration; a submission with no declaration
    // has none at submission or staging — exactly the prior
    // behaviour — and nothing about the gate changes.
    let mut store = MemoryStore::default();
    let content = "design record".to_owned();
    submit_with_declaration(
        &mut store,
        "sub-decl-2",
        content.clone(),
        provenance(&content),
        SensitivityCeiling::Internal,
        PlacementDeclaration {
            note_class: NoteClass::DesignRecord,
            atlas_tier: Some(AtlasTier::Tier1),
        },
        "t0",
    )
    .unwrap();
    let effective = store
        .submission("sub-decl-2")
        .unwrap()
        .effective_declaration()
        .unwrap();
    assert_eq!(effective.home, PlacementHome::RepoDoc);
    assert_eq!(effective.atlas_tier, AtlasTier::Tier1);
    assert!(!effective.tier_was_defaulted);

    submit(
        &mut store,
        "sub-decl-3",
        content.clone(),
        provenance(&content),
        SensitivityCeiling::Internal,
        "t0",
    )
    .unwrap();
    let undeclared = store.submission("sub-decl-3").unwrap();
    assert_eq!(undeclared.declared_placement, None);
    assert_eq!(undeclared.effective_declaration(), None);
    stage(&mut store, "sub-decl-3", placement(), "t1").unwrap();
    assert_eq!(store.staged("sub-decl-3").unwrap().declaration, None);
}

#[test]
fn decision_table_defaults_cover_every_class() {
    // Oracle: the design §2 decision table, one row per class —
    // home and Atlas tier exactly as the table states them.
    for (class, home, tier) in [
        (
            NoteClass::EphemeralWorkingNote,
            PlacementHome::WorkspaceScratch,
            AtlasTier::NotIngested,
        ),
        (
            NoteClass::SessionDailyLog,
            PlacementHome::MemoryDailyLog,
            AtlasTier::Tier3,
        ),
        (
            NoteClass::DurableFinding,
            PlacementHome::Zettel,
            AtlasTier::Tier1,
        ),
        (
            NoteClass::ResearchSynthesis,
            PlacementHome::Zettel,
            AtlasTier::Tier1,
        ),
        (
            NoteClass::DesignRecord,
            PlacementHome::RepoDoc,
            AtlasTier::Tier2,
        ),
        (
            NoteClass::Runbook,
            PlacementHome::OwningRepo,
            AtlasTier::Tier2,
        ),
        (
            NoteClass::UserFacingDocument,
            PlacementHome::RequestingSurface,
            AtlasTier::NotIngested,
        ),
        (
            NoteClass::MemoryTopic,
            PlacementHome::AgentMemory,
            AtlasTier::Tier3,
        ),
        (
            NoteClass::MachineStateSnapshot,
            PlacementHome::OwningSystemStore,
            AtlasTier::NotIngested,
        ),
        (
            NoteClass::IdNativeNote,
            PlacementHome::IdGraph,
            AtlasTier::Tier0,
        ),
    ] {
        assert_eq!(class.default_home(), home, "{class:?} home");
        assert_eq!(class.default_tier(), tier, "{class:?} tier");
    }
}
