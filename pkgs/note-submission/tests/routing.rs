//! #7717 classifier tests: the #6022 decision table as routing behaviour.
//!
//! Oracles are the two merged placement records: the 2026-11-10 record's
//! §2 table (home + Atlas tier per class) and §5 ID-native conditions,
//! and the 2026-11-08 record's §2 sensitivity rules (physical tiers
//! public/internal/secret, internal by default, applied only after the
//! source/caller ceiling; local-only never leaves the machine).

use note_submission::{
    AtlasTier, Classification, CorpusControl, DestinationWriter, HumanApproval,
    ID_GRAPH_DESTINATION, IdNativeEligibility, MemoryStore, NoteClass, NoteOutStatus,
    PlacementDecision, PlacementDeclaration, PlacementHome, Provenance, ProvenanceSpan,
    RoutingContext, SensitivityCeiling, SensitivityTier, Submission, SubmissionState,
    SubmissionStore, UploadAttemptStatus, classify, content_sha256, promote, stage_classified,
    submit_with_declaration,
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
        captured_at: "2026-11-23T17:30:00Z".to_owned(),
        author: "agent-ob4".to_owned(),
        owner: "team-notes".to_owned(),
        spans: vec![ProvenanceSpan {
            source: "doc/notes/example.md".to_owned(),
            byte_range: Some((0, content.len() as u64)),
            content_sha256: content_sha256(content),
        }],
    }
}

fn declared_submission(id: &str, class: NoteClass, ceiling: SensitivityCeiling) -> Submission {
    let content = format!("body of {id}");
    Submission {
        id: id.to_owned(),
        content_sha256: content_sha256(&content),
        content,
        provenance: provenance("span"),
        sensitivity_ceiling: ceiling,
        declared_placement: Some(PlacementDeclaration {
            note_class: class,
            atlas_tier: None,
        }),
        placement: None,
        state: SubmissionState::Submitted,
        submitted_at: "t0".to_owned(),
        updated_at: "t0".to_owned(),
    }
}

fn full_context() -> RoutingContext {
    RoutingContext {
        zettel_corpus: Some("developing-today/zettel".to_owned()),
        owning_repo: Some("developing-today/code".to_owned()),
        requested_tier: None,
        id_native: Some(IdNativeEligibility {
            cited_across_systems: true,
            stable_identity: true,
            writer_set: Some("idset.writer:notes-publishers".to_owned()),
            object_id: Some("id:note:programme-record-1".to_owned()),
        }),
        canonical_identity: None,
    }
}

fn routed(classification: Classification) -> PlacementDecision {
    match classification {
        Classification::Routed { decision, .. } => decision,
        other => panic!("expected Routed, got {other:?}"),
    }
}

fn stays_staged(classification: Classification) -> String {
    match classification {
        Classification::StaysStaged { reason, .. } => reason,
        other => panic!("expected StaysStaged, got {other:?}"),
    }
}

#[test]
fn decision_table_routes_every_class() {
    // Oracle: one row per §2 class — selected destination (or the
    // non-published home), Atlas treatment, and sensitivity resolved
    // under an internal ceiling with no explicit tier request.
    let context = full_context();

    // Classes this pipeline publishes, with their destinations.
    for (class, destination, tier) in [
        (
            NoteClass::DurableFinding,
            "developing-today/zettel",
            AtlasTier::Tier1,
        ),
        (
            NoteClass::ResearchSynthesis,
            "developing-today/zettel",
            AtlasTier::Tier1,
        ),
        (
            NoteClass::DesignRecord,
            "developing-today/code",
            AtlasTier::Tier2,
        ),
        (
            NoteClass::Runbook,
            "developing-today/code",
            AtlasTier::Tier2,
        ),
        (
            NoteClass::IdNativeNote,
            ID_GRAPH_DESTINATION,
            AtlasTier::Tier0,
        ),
    ] {
        let submission = declared_submission("sub-t", class, SensitivityCeiling::Internal);
        let Classification::Routed {
            declaration,
            decision,
        } = classify(&submission, &context)
        else {
            panic!("{class:?} must route with a full context");
        };
        assert_eq!(decision.destination, destination, "{class:?} destination");
        assert_eq!(
            decision.sensitivity,
            SensitivityTier::Internal,
            "{class:?} defaults to the internal physical tier"
        );
        assert_eq!(
            decision.source_revision, submission.content_sha256,
            "{class:?} pins the source revision/hash"
        );
        assert_eq!(declaration.home, class.default_home(), "{class:?} home");
        assert_eq!(declaration.atlas_tier, tier, "{class:?} Atlas treatment");
    }

    // Classes whose canonical home this pipeline does not publish.
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
    ] {
        let submission = declared_submission("sub-t", class, SensitivityCeiling::Internal);
        let classification = classify(&submission, &context);
        let Classification::NotPublished { declaration, .. } = &classification else {
            panic!("{class:?} must not be published, got {classification:?}");
        };
        assert_eq!(declaration.home, home, "{class:?} home");
        assert_eq!(declaration.atlas_tier, tier, "{class:?} Atlas treatment");
        assert!(classification.decision().is_none());
    }
}

#[test]
fn missing_routing_facts_fail_closed() {
    // Oracle: a publishable class without its registry/repo fact stays
    // staged with a reason; an undeclared submission is uncertain.
    let submission = declared_submission(
        "sub-nocorpus",
        NoteClass::DurableFinding,
        SensitivityCeiling::Internal,
    );
    let reason = stays_staged(classify(&submission, &RoutingContext::default()));
    assert!(reason.contains("registry-selected corpus"), "{reason}");

    let submission = declared_submission(
        "sub-norepo",
        NoteClass::DesignRecord,
        SensitivityCeiling::Internal,
    );
    let reason = stays_staged(classify(&submission, &RoutingContext::default()));
    assert!(reason.contains("owning repo"), "{reason}");

    let mut undeclared = declared_submission(
        "sub-undeclared",
        NoteClass::DurableFinding,
        SensitivityCeiling::Internal,
    );
    undeclared.declared_placement = None;
    let classification = classify(&undeclared, &full_context());
    assert!(matches!(
        classification,
        Classification::StaysStaged {
            declaration: None,
            ..
        }
    ));
}

#[test]
fn id_native_eligibility_is_all_three_conditions_plus_identity() {
    // Oracle (§5): each missing condition independently keeps the note
    // staged; memory-shaped and secret-shaped material never qualifies.
    let submission = declared_submission(
        "sub-id",
        NoteClass::IdNativeNote,
        SensitivityCeiling::Internal,
    );

    let mut context = full_context();
    context.id_native = None;
    assert!(stays_staged(classify(&submission, &context)).contains("eligibility evidence"));

    let mut context = full_context();
    context.id_native.as_mut().unwrap().cited_across_systems = false;
    assert!(stays_staged(classify(&submission, &context)).contains("condition (a)"));

    let mut context = full_context();
    context.id_native.as_mut().unwrap().stable_identity = false;
    assert!(stays_staged(classify(&submission, &context)).contains("condition (b)"));

    let mut context = full_context();
    context.id_native.as_mut().unwrap().writer_set = None;
    assert!(stays_staged(classify(&submission, &context)).contains("writer-set"));

    let mut context = full_context();
    context.id_native.as_mut().unwrap().object_id = Some("  ".to_owned());
    assert!(stays_staged(classify(&submission, &context)).contains("graph object id"));

    // With all evidence present the graph object id is the canonical
    // identity — projections never become a second canonical owner.
    let decision = routed(classify(&submission, &full_context()));
    assert_eq!(decision.canonical_identity, "id:note:programme-record-1");
    assert!(decision.rationale.contains("idset.writer:notes-publishers"));

    // Secret-bearing material is banned from every note home (§5),
    // even with full eligibility evidence and an explicit secret tier.
    let secret = declared_submission(
        "sub-id-secret",
        NoteClass::IdNativeNote,
        SensitivityCeiling::SecretBearing,
    );
    let mut context = full_context();
    context.requested_tier = Some(SensitivityTier::Secret);
    assert!(stays_staged(classify(&secret, &context)).contains("never ID-native"));
}

#[test]
fn physical_tier_is_resolved_only_after_the_ceiling() {
    // Oracle (2026-11-08 record §2): default internal; public/secret
    // are explicit; ceilings cap or compel the tier; personal and
    // local-only material is never routed.
    let finding = |ceiling| declared_submission("sub-tier", NoteClass::DurableFinding, ceiling);
    let mut context = full_context();

    // Shareable ceiling: explicit public is permitted.
    context.requested_tier = Some(SensitivityTier::Public);
    let decision = routed(classify(&finding(SensitivityCeiling::Shareable), &context));
    assert_eq!(decision.sensitivity, SensitivityTier::Public);

    // Internal ceiling: explicit public is refused, fail-closed.
    let reason = stays_staged(classify(&finding(SensitivityCeiling::Internal), &context));
    assert!(reason.contains("internal ceiling"), "{reason}");

    // Secret-bearing: the secret tier must be explicit.
    context.requested_tier = None;
    let reason = stays_staged(classify(
        &finding(SensitivityCeiling::SecretBearing),
        &context,
    ));
    assert!(reason.contains("explicit secret"), "{reason}");
    context.requested_tier = Some(SensitivityTier::Secret);
    let decision = routed(classify(
        &finding(SensitivityCeiling::SecretBearing),
        &context,
    ));
    assert_eq!(decision.sensitivity, SensitivityTier::Secret);

    // Personal / local-only ceilings: no route at any requested tier.
    for ceiling in [SensitivityCeiling::Personal, SensitivityCeiling::LocalOnly] {
        let classification = classify(&finding(ceiling), &context);
        assert!(
            matches!(classification, Classification::StaysStaged { .. }),
            "{ceiling:?} must never route"
        );
    }
}

#[test]
fn stage_classified_stages_only_routed_verdicts() {
    // Oracle: a routed verdict enters staging with the classifier's
    // exact decision; a fail-closed verdict leaves the submission in
    // the submission DB, unstaged and unpublished.
    let mut store = MemoryStore::default();
    let content = "verified finding".to_owned();
    submit_with_declaration(
        &mut store,
        "sub-sc-1",
        content.clone(),
        provenance(&content),
        SensitivityCeiling::Internal,
        PlacementDeclaration {
            note_class: NoteClass::DurableFinding,
            atlas_tier: None,
        },
        "t0",
    )
    .unwrap();
    let classification = stage_classified(&mut store, "sub-sc-1", &full_context(), "t1").unwrap();
    let decision = routed(classification);
    assert_eq!(
        store.submission("sub-sc-1").unwrap().state,
        SubmissionState::Staged
    );
    assert_eq!(store.staged("sub-sc-1").unwrap().placement, decision);
    assert!(store.notes_out.is_empty(), "staging must not publish");

    submit_with_declaration(
        &mut store,
        "sub-sc-2",
        content.clone(),
        provenance(&content),
        SensitivityCeiling::Internal,
        PlacementDeclaration {
            note_class: NoteClass::ResearchSynthesis,
            atlas_tier: None,
        },
        "t0",
    )
    .unwrap();
    let classification =
        stage_classified(&mut store, "sub-sc-2", &RoutingContext::default(), "t1").unwrap();
    assert!(matches!(classification, Classification::StaysStaged { .. }));
    assert_eq!(
        store.submission("sub-sc-2").unwrap().state,
        SubmissionState::Submitted
    );
    assert!(store.staged("sub-sc-2").is_none());
    assert!(store.notes_out.is_empty());
}

#[test]
fn end_to_end_stage_approve_publish_verifies_hash_and_provenance() {
    // Oracle (#7717 acceptance): submit with a declared class →
    // classifier stages the decision → human approval is rechecked
    // against the exact classified destination/tier → publication
    // lands at the selected destination with the content hash and
    // provenance verified against the submission record.
    let mut store = MemoryStore::default();
    let content = "cross-project finding worth keeping".to_owned();
    let note_provenance = provenance(&content);
    submit_with_declaration(
        &mut store,
        "sub-e2e",
        content.clone(),
        note_provenance.clone(),
        SensitivityCeiling::Internal,
        PlacementDeclaration {
            note_class: NoteClass::DurableFinding,
            atlas_tier: None,
        },
        "2026-11-23T17:31:00Z",
    )
    .unwrap();

    let classification = stage_classified(
        &mut store,
        "sub-e2e",
        &full_context(),
        "2026-11-23T17:32:00Z",
    )
    .unwrap();
    let decision = routed(classification);
    assert_eq!(decision.destination, "developing-today/zettel");

    store.corpus_controls.insert(
        "developing-today/zettel".to_owned(),
        CorpusControl {
            destination: "developing-today/zettel".to_owned(),
            enabled: true,
            manual_hold: false,
            updated_at: "2026-11-23T17:32:30Z".to_owned(),
        },
    );

    // Approval recheck: an approval naming a different tier than the
    // classified decision is rejected and audited; nothing is written.
    let wrong = HumanApproval {
        human_id: "reviewer@example.test".to_owned(),
        approved_destination: decision.destination.clone(),
        approved_sensitivity: SensitivityTier::Public,
        rationale: "wrong tier".to_owned(),
        approved_at: "2026-11-23T17:33:00Z".to_owned(),
    };
    let mut destination = TestDestination::default();
    assert!(promote(&mut store, &mut destination, "sub-e2e", &wrong).is_err());
    assert!(destination.writes.is_empty());
    assert_eq!(
        store.upload_attempts.last().unwrap().status,
        UploadAttemptStatus::Rejected
    );
    assert_eq!(
        store.submission("sub-e2e").unwrap().state,
        SubmissionState::Staged
    );

    // The matching approval publishes exactly the submitted bytes.
    let approval = HumanApproval {
        human_id: "reviewer@example.test".to_owned(),
        approved_destination: decision.destination.clone(),
        approved_sensitivity: decision.sensitivity,
        rationale: "classification, destination, and sensitivity reviewed".to_owned(),
        approved_at: "2026-11-23T17:34:00Z".to_owned(),
    };
    promote(&mut store, &mut destination, "sub-e2e", &approval).unwrap();

    assert_eq!(destination.writes.len(), 1);
    assert_eq!(destination.writes[0].0, "developing-today/zettel");
    assert_eq!(destination.writes[0].1, decision.canonical_identity);
    assert_eq!(destination.writes[0].2, content);

    let submission = store.submission("sub-e2e").unwrap();
    assert_eq!(submission.state, SubmissionState::Promoted);
    assert_eq!(submission.provenance, note_provenance);
    assert_eq!(submission.content_sha256, content_sha256(&content));

    assert_eq!(store.notes_out.len(), 1);
    let note_out = &store.notes_out[0];
    assert_eq!(note_out.status, NoteOutStatus::Published);
    assert_eq!(note_out.destination, decision.destination);
    assert_eq!(note_out.canonical_identity, decision.canonical_identity);
    assert_eq!(note_out.source_revision, decision.source_revision);
    assert_eq!(note_out.source_sha256, submission.content_sha256);
    assert_eq!(note_out.promoted_by, "reviewer@example.test");
    assert_eq!(
        store.upload_attempts.last().unwrap().status,
        UploadAttemptStatus::Published
    );
}
