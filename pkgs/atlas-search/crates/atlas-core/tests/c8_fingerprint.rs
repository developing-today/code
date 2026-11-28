//! C8-E fingerprint core tests (developing-today/code#9576,
//! #12962, first slice).
//!
//! Oracle: the merged C8 design,
//! `doc/2026-11-22T01-48-21Z_design_c8_atlas_ingestion_investigation`,
//! §4 (error-investigation mode: normalization is conservative —
//! Unicode normalization, whitespace normalization only; preserve
//! the original string; do not merge merely similar errors) and
//! §5 steps 1–2 (record a stable fingerprint alongside the
//! original; a fingerprint groups exact normalized occurrences
//! only; find the first observed occurrence within retained
//! telemetry, labelled "earliest retained observation" when the
//! actual first occurrence may predate retention) and step 6
//! (the bundle: query/fingerprint, time range, evidence IDs,
//! exact-vs-inferred edge labels, missing-data warnings — never
//! an automated causation claim).
//!
//! The model test is this slice's share of the design's
//! investigation model test (verification contract): a fixture
//! with first vs earliest-retained-observation distinction
//! inputs, exact-vs-inferred edge labels, and missing-link gaps
//! preserved as gaps. The fixture's unauthorized-target element
//! belongs to the later live-interface slices.
//!
//! The findings-assembly tests are the second slice's share
//! (design §5 steps 4–6): candidate changes with exact-vs-
//! inferred association labels, rejected candidates with
//! reasons, the step-5 unpushed-final-commit check recorded
//! exactly as an authoritative source reported it (a check not
//! performed is a gap, never a negative result), and the next
//! action — the investigation's own, or one derived
//! deterministically from the bundle's gaps and candidates.

use atlas_core::c8_fingerprint::{
    CandidateChange, EdgeLabel, ErrorFingerprint, ErrorOccurrence, FirstObservationKind,
    InvestigationEdge, InvestigationFindings, RejectedCandidate, UnpushedCommitCheck,
    UnpushedCommitStatus, build_bundle, build_bundle_with_findings, first_observation,
    group_occurrences, normalize_error_string,
};
use proptest::prelude::*;

#[test]
fn c8e_normalization_is_nfc_and_whitespace_only() {
    // NFC: decomposed and composed forms of the same text agree.
    let decomposed = "cafe\u{301} failure"; // "café" with combining acute
    let composed = "caf\u{e9} failure";
    assert_eq!(
        normalize_error_string(decomposed),
        normalize_error_string(composed)
    );
    // Whitespace: runs of spaces, tabs, and newlines collapse to a
    // single space; the ends are trimmed.
    assert_eq!(
        normalize_error_string("  connection \t refused\n\nby  peer "),
        "connection refused by peer"
    );
    // Nothing else is rewritten: case, punctuation, and digits
    // survive untouched.
    assert_eq!(
        normalize_error_string("Error E-1042: Timeout (30s)"),
        "Error E-1042: Timeout (30s)"
    );
}

#[test]
fn c8e_normalization_never_merges_merely_similar_errors() {
    // Differing digits, differing case, and a compatibility
    // ligature (NFC is not NFKC) all stay distinct.
    assert_ne!(
        ErrorFingerprint::of("error 42").id(),
        ErrorFingerprint::of("error 43").id()
    );
    assert_ne!(
        ErrorFingerprint::of("Error refused").id(),
        ErrorFingerprint::of("error refused").id()
    );
    assert_ne!(
        ErrorFingerprint::of("\u{fb01}le missing").id(), // "ﬁle" with ligature
        ErrorFingerprint::of("file missing").id()
    );
}

#[test]
fn c8e_fingerprint_groups_exact_normalized_occurrences_only() {
    let occurrences = vec![
        ErrorOccurrence::new("connection  refused", "trace:b", "2026-11-20T10:00:00Z"),
        ErrorOccurrence::new("connection refused", "trace:a", "2026-11-20T09:00:00Z"),
        ErrorOccurrence::new("connection refused!", "trace:c", "2026-11-20T08:00:00Z"),
        ErrorOccurrence::new("cafe\u{301} failure", "trace:d", "2026-11-20T11:00:00Z"),
        ErrorOccurrence::new("caf\u{e9} failure", "trace:e", "2026-11-20T12:00:00Z"),
    ];
    let groups = group_occurrences(&occurrences);
    assert_eq!(
        groups.len(),
        3,
        "whitespace/NFC variants group; the punctuated string does not"
    );

    let refused = groups
        .iter()
        .find(|g| g.fingerprint.normalized == "connection refused")
        .expect("the refused group exists");
    // Observation order inside the group, originals verbatim.
    let originals: Vec<&str> = refused
        .occurrences
        .iter()
        .map(|o| o.original.as_str())
        .collect();
    assert_eq!(originals, ["connection refused", "connection  refused"]);

    let cafe = groups
        .iter()
        .find(|g| g.fingerprint.normalized == "caf\u{e9} failure")
        .expect("the café group exists");
    assert_eq!(cafe.occurrences.len(), 2);
}

#[test]
fn c8e_first_observation_labels_first_vs_earliest_retained() {
    let fingerprint = ErrorFingerprint::of("disk full");
    let occurrences = vec![
        ErrorOccurrence::new("disk full", "log:2", "2026-11-10T00:00:00Z"),
        ErrorOccurrence::new("disk full", "log:1", "2026-11-05T00:00:00Z"),
        ErrorOccurrence::new("disk fuller", "log:0", "2026-11-01T00:00:00Z"),
    ];

    // Retention floor strictly before the earliest exact
    // occurrence: it is the first observed occurrence. The merely
    // similar earlier string is not an occurrence of this error.
    let first = first_observation(&occurrences, &fingerprint, Some("2026-11-01T00:00:00Z"))
        .expect("an exact occurrence exists");
    assert_eq!(first.kind, FirstObservationKind::FirstObserved);
    assert_eq!(first.kind.label(), "first observed");
    assert_eq!(first.occurrence.source_id, "log:1");
    assert_eq!(first.exact_occurrences, 2);

    // Floor at the earliest occurrence: the actual first
    // occurrence may predate retention.
    let at_floor = first_observation(&occurrences, &fingerprint, Some("2026-11-05T00:00:00Z"))
        .expect("an exact occurrence exists");
    assert_eq!(
        at_floor.kind,
        FirstObservationKind::EarliestRetainedObservation
    );
    assert_eq!(at_floor.kind.label(), "earliest retained observation");

    // No floor stated: never claim "first observed".
    let unknown_floor =
        first_observation(&occurrences, &fingerprint, None).expect("an exact occurrence exists");
    assert_eq!(
        unknown_floor.kind,
        FirstObservationKind::EarliestRetainedObservation
    );

    // No exact occurrence in the set: no result.
    let other = ErrorFingerprint::of("never seen");
    assert!(first_observation(&occurrences, &other, Some("2026-11-01T00:00:00Z")).is_none());
}

#[test]
fn c8e_investigation_model_test_fixture_graph() {
    // Fixture (this slice's share of the design's investigation
    // model test): the same error observed in three retained
    // sources, the earliest sitting exactly at the retention
    // floor; an exact commit → PR edge, an inferred PR → issue
    // edge, and two missing links (issue → commit, session →
    // queue attempt) that must survive as gaps.
    let query = "  quota   read failed: pool walled ";
    let occurrences = vec![
        ErrorOccurrence::new(
            "quota read failed: pool walled",
            "trace:run-3",
            "2026-11-22T03:00:00Z",
        ),
        ErrorOccurrence::new(
            "quota\tread failed: pool walled",
            "trace:run-1",
            "2026-11-22T01:00:00Z",
        ),
        ErrorOccurrence::new(
            "quota read failed: pool walled",
            "report:cycle-464",
            "2026-11-22T02:00:00Z",
        ),
        ErrorOccurrence::new(
            "quota read failed: pool walled!",
            "trace:other",
            "2026-11-22T00:30:00Z",
        ),
    ];
    let edges = vec![
        InvestigationEdge::exact("commit_to_pull_request", "commit:abc123", "pr:12959"),
        InvestigationEdge::inferred(
            "pull_request_to_issue",
            "pr:12959",
            "issue:9576",
            0.8,
            "pr-body-reference",
        ),
        InvestigationEdge::missing("issue_to_commit", "issue:12962"),
        InvestigationEdge::missing("session_to_queue_attempt", "session:s-1"),
    ];

    let bundle = build_bundle(query, &occurrences, &edges, Some("2026-11-22T01:00:00Z"));

    // Query preserved as supplied; fingerprint over the
    // normalized form.
    assert_eq!(bundle.query, query);
    assert_eq!(
        bundle.fingerprint.normalized,
        "quota read failed: pool walled"
    );
    assert_eq!(bundle.time_range.0, "2026-11-22T00:30:00Z");
    assert_eq!(bundle.time_range.1, "2026-11-22T03:00:00Z");

    // First vs earliest-retained: the earliest exact occurrence
    // sits at the retention floor, so it is the earliest retained
    // observation — and the bundle warns about exactly that. The
    // punctuated near-miss at 00:30 is not an occurrence.
    let first = bundle
        .first_observation
        .as_ref()
        .expect("exact occurrences exist");
    assert_eq!(
        first.kind,
        FirstObservationKind::EarliestRetainedObservation
    );
    assert_eq!(first.occurrence.source_id.as_str(), "trace:run-1");
    assert_eq!(first.exact_occurrences, 3);
    assert!(
        bundle
            .gaps
            .iter()
            .any(|g| g.contains("earliest retained observation")),
        "gaps name the retention caveat: {:?}",
        bundle.gaps
    );

    // Evidence IDs: exact occurrences in observation order, then
    // edge endpoints; the near-miss source is not evidence.
    assert_eq!(
        bundle.evidence_ids,
        [
            "trace:run-1",
            "report:cycle-464",
            "trace:run-3",
            "commit:abc123",
            "pr:12959",
            "issue:9576",
            "issue:12962",
            "session:s-1",
        ]
    );

    // Edge labels preserved exactly as evidenced: exact carries
    // no confidence, inferred always carries confidence and
    // provenance.
    assert_eq!(bundle.edges.len(), 4);
    assert_eq!(bundle.edges[0].label, EdgeLabel::Exact);
    assert_eq!(bundle.edges[0].confidence, None);
    assert_eq!(bundle.edges[1].label, EdgeLabel::Inferred);
    assert_eq!(bundle.edges[1].label.as_str(), "inferred");
    assert_eq!(bundle.edges[1].confidence, Some(0.8));
    assert_eq!(bundle.edges[1].source.as_deref(), Some("pr-body-reference"));

    // Missing links are preserved as gaps, in edge order, and
    // the gap edges themselves stay in the record.
    assert!(bundle.edges[2].is_gap());
    assert!(bundle.edges[3].is_gap());
    assert_eq!(
        bundle.gaps[0],
        "missing link: issue_to_commit from issue:12962"
    );
    assert_eq!(
        bundle.gaps[1],
        "missing link: session_to_queue_attempt from session:s-1"
    );

    // The bundle is evidence, not a verdict: its serialized form
    // contains no causation claim.
    let serialized = serde_json::to_string(&bundle).expect("bundle serializes");
    assert!(!serialized.contains("caused"));
    assert!(!serialized.contains("causation"));
}

#[test]
fn c8e_full_bundle_assembles_findings() {
    // Fixture: one exact occurrence after the retention floor,
    // an exact commit → PR edge and an inferred PR → issue edge,
    // and gathered findings — one exact and one inferred
    // candidate change, one rejected candidate, an
    // unpushed-final-commit check an authoritative source
    // reported as none-found, and the investigation's own next
    // action.
    let occurrences = vec![ErrorOccurrence::new(
        "disk full",
        "log:1",
        "2026-11-05T00:00:00Z",
    )];
    let edges = vec![
        InvestigationEdge::exact("commit_to_pull_request", "commit:abc123", "pr:12959"),
        InvestigationEdge::inferred(
            "pull_request_to_issue",
            "pr:12959",
            "issue:9576",
            0.8,
            "pr-body-reference",
        ),
    ];
    let findings = InvestigationFindings {
        candidate_changes: vec![
            CandidateChange::exact("commit:abc123", "raise the pool ceiling"),
            CandidateChange::inferred(
                "commit:def456",
                "touch the same config property",
                0.6,
                "same-file search",
            ),
        ],
        rejected_candidates: vec![RejectedCandidate {
            candidate_id: "commit:999999".to_string(),
            reason: "merged after the first observed occurrence".to_string(),
        }],
        unpushed_commit_check: UnpushedCommitCheck::reported(
            UnpushedCommitStatus::NoneFound,
            "2026-11-23T00:00:00Z",
            "session-state registry",
            None,
        ),
        next_action: Some("review commit:abc123 first".to_string()),
    };

    let bundle = build_bundle_with_findings(
        "disk full",
        &occurrences,
        &edges,
        Some("2026-11-01T00:00:00Z"),
        &findings,
    );

    // Candidates keep their association labels: exact carries no
    // confidence, inferred always carries confidence and
    // provenance — the edge discipline of design §3.
    assert_eq!(bundle.candidate_changes.len(), 2);
    assert_eq!(bundle.candidate_changes[0].label, EdgeLabel::Exact);
    assert_eq!(bundle.candidate_changes[0].confidence, None);
    assert_eq!(bundle.candidate_changes[1].label, EdgeLabel::Inferred);
    assert_eq!(bundle.candidate_changes[1].confidence, Some(0.6));
    assert_eq!(
        bundle.candidate_changes[1].source.as_deref(),
        Some("same-file search")
    );

    // The rejected candidate stays in the record with its
    // reason, but it is not evidence: its ID never joins the
    // evidence IDs, while both candidates' commits do.
    assert_eq!(bundle.rejected_candidates.len(), 1);
    assert_eq!(
        bundle.rejected_candidates[0].reason,
        "merged after the first observed occurrence"
    );
    assert!(bundle.evidence_ids.iter().any(|id| id == "commit:abc123"));
    assert!(bundle.evidence_ids.iter().any(|id| id == "commit:def456"));
    assert!(
        !bundle.evidence_ids.iter().any(|id| id == "commit:999999"),
        "rejected candidates are not evidence: {:?}",
        bundle.evidence_ids
    );

    // The check is recorded exactly as reported, and the
    // investigation's own next action wins over any derivation.
    let check = bundle
        .unpushed_commit_check
        .as_ref()
        .expect("findings were assembled");
    assert_eq!(check.status, UnpushedCommitStatus::NoneFound);
    assert_eq!(check.checked_at.as_deref(), Some("2026-11-23T00:00:00Z"));
    assert_eq!(check.source.as_deref(), Some("session-state registry"));
    assert_eq!(
        bundle.next_action.as_deref(),
        Some("review commit:abc123 first")
    );

    // Nothing is missing in this record: no unpushed-check gap,
    // no no-candidates gap.
    assert!(
        !bundle.gaps.iter().any(|g| g.contains("unpushed")),
        "gaps: {:?}",
        bundle.gaps
    );
    assert!(
        !bundle.gaps.iter().any(|g| g.contains("candidate")),
        "gaps: {:?}",
        bundle.gaps
    );
}

#[test]
fn c8e_unpushed_check_states_are_recorded_not_inferred() {
    let occurrences = vec![ErrorOccurrence::new(
        "disk full",
        "log:1",
        "2026-11-05T00:00:00Z",
    )];
    let edges: Vec<InvestigationEdge> = Vec::new();
    let floor = Some("2026-11-01T00:00:00Z");

    // Not performed: the status is recorded as not-checked and
    // named as a gap — never converted into "no unpushed
    // commit" (design §5 step 5, §6).
    let findings = InvestigationFindings {
        candidate_changes: vec![CandidateChange::exact("commit:abc123", "a change")],
        ..Default::default()
    };
    let bundle = build_bundle_with_findings("disk full", &occurrences, &edges, floor, &findings);
    let check = bundle
        .unpushed_commit_check
        .as_ref()
        .expect("findings were assembled");
    assert_eq!(check.status, UnpushedCommitStatus::NotChecked);
    assert_eq!(check.checked_at, None);
    assert!(
        bundle
            .gaps
            .iter()
            .any(|g| g.contains("unpushed-final-commit check not performed")),
        "gaps: {:?}",
        bundle.gaps
    );

    // Reported as found: the unpushed commit joins the evidence
    // IDs and no gap is named for the check.
    let findings = InvestigationFindings {
        candidate_changes: vec![CandidateChange::exact("commit:abc123", "a change")],
        unpushed_commit_check: UnpushedCommitCheck::reported(
            UnpushedCommitStatus::UnpushedFound,
            "2026-11-23T00:00:00Z",
            "worktree state source",
            Some("commit:unpushed1".to_string()),
        ),
        ..Default::default()
    };
    let bundle = build_bundle_with_findings("disk full", &occurrences, &edges, floor, &findings);
    let check = bundle
        .unpushed_commit_check
        .as_ref()
        .expect("findings were assembled");
    assert_eq!(check.status, UnpushedCommitStatus::UnpushedFound);
    assert_eq!(check.commit_id.as_deref(), Some("commit:unpushed1"));
    assert!(
        bundle
            .evidence_ids
            .iter()
            .any(|id| id == "commit:unpushed1")
    );
    assert!(!bundle.gaps.iter().any(|g| g.contains("unpushed")));

    // Core-only bundle: the check field is absent entirely —
    // absence, not a fabricated status.
    let core = build_bundle("disk full", &occurrences, &edges, floor);
    assert_eq!(core.unpushed_commit_check, None);
    assert_eq!(core.next_action, None);
    let serialized = serde_json::to_string(&core).expect("core bundle serializes");
    assert!(!serialized.contains("unpushed_commit_check"));
    assert!(!serialized.contains("candidate_changes"));
}

#[test]
fn c8e_full_bundle_derives_next_action_and_names_missing_candidates() {
    let occurrences = vec![ErrorOccurrence::new(
        "disk full",
        "log:1",
        "2026-11-05T00:00:00Z",
    )];
    let edges: Vec<InvestigationEdge> = Vec::new();
    let floor = Some("2026-11-01T00:00:00Z");

    // Empty findings: no candidates and no check are named as
    // gaps, and the derived next action is to resolve them.
    let bundle = build_bundle_with_findings(
        "disk full",
        &occurrences,
        &edges,
        floor,
        &InvestigationFindings::default(),
    );
    assert!(
        bundle
            .gaps
            .iter()
            .any(|g| g.contains("no candidate changes recorded")),
        "gaps: {:?}",
        bundle.gaps
    );
    assert_eq!(
        bundle.next_action.as_deref(),
        Some("resolve the named missing-data gaps, then re-run the investigation")
    );

    // Clean findings: candidates present, check reported, no
    // gap edges, first observation is first-observed — the
    // derived next action is human review, and gaps are empty.
    let findings = InvestigationFindings {
        candidate_changes: vec![CandidateChange::exact("commit:abc123", "a change")],
        unpushed_commit_check: UnpushedCommitCheck::reported(
            UnpushedCommitStatus::NoneFound,
            "2026-11-23T00:00:00Z",
            "session-state registry",
            None,
        ),
        ..Default::default()
    };
    let bundle = build_bundle_with_findings("disk full", &occurrences, &edges, floor, &findings);
    assert!(bundle.gaps.is_empty(), "gaps: {:?}", bundle.gaps);
    assert_eq!(
        bundle.next_action.as_deref(),
        Some("human review of the candidate changes; the bundle records evidence only")
    );
}

#[test]
fn c8e_bundle_shape_is_additive_and_round_trips() {
    // A full bundle survives a serde round trip unchanged.
    let occurrences = vec![ErrorOccurrence::new(
        "disk full",
        "log:1",
        "2026-11-05T00:00:00Z",
    )];
    let findings = InvestigationFindings {
        candidate_changes: vec![CandidateChange::inferred(
            "commit:def456",
            "touch the same config property",
            0.6,
            "same-file search",
        )],
        rejected_candidates: vec![RejectedCandidate {
            candidate_id: "commit:999999".to_string(),
            reason: "outside the time range".to_string(),
        }],
        ..Default::default()
    };
    let full = build_bundle_with_findings(
        "disk full",
        &occurrences,
        &[],
        Some("2026-11-01T00:00:00Z"),
        &findings,
    );
    let serialized = serde_json::to_string(&full).expect("full bundle serializes");
    let back: atlas_core::c8_fingerprint::EvidenceBundle =
        serde_json::from_str(&serialized).expect("full bundle deserializes");
    assert_eq!(back, full);

    // A core bundle serialized before findings existed — the
    // same string the core builder produces today, with the
    // findings fields absent — still deserializes, with the
    // findings fields at their empty defaults (design §2:
    // additive-only changes, old versions remain readable).
    let core = build_bundle("disk full", &occurrences, &[], Some("2026-11-01T00:00:00Z"));
    let core_serialized = serde_json::to_string(&core).expect("core bundle serializes");
    let core_back: atlas_core::c8_fingerprint::EvidenceBundle =
        serde_json::from_str(&core_serialized).expect("core bundle deserializes");
    assert_eq!(core_back, core);
    assert!(core_back.candidate_changes.is_empty());
    assert!(core_back.rejected_candidates.is_empty());
}

proptest! {
    #[test]
    fn c8e_normalization_is_idempotent(raw in any::<String>()) {
        let once = normalize_error_string(&raw);
        prop_assert_eq!(normalize_error_string(&once), once);
    }

    #[test]
    fn c8e_fingerprint_of_raw_equals_fingerprint_of_normalized(raw in any::<String>()) {
        prop_assert_eq!(
            ErrorFingerprint::of(&raw).id(),
            ErrorFingerprint::of(&normalize_error_string(&raw)).id()
        );
    }

    #[test]
    fn c8e_grouping_never_merges_distinct_normalized_strings(
        raws in prop::collection::vec(any::<String>(), 0..16)
    ) {
        let occurrences: Vec<ErrorOccurrence> = raws
            .iter()
            .enumerate()
            .map(|(i, raw)| {
                ErrorOccurrence::new(raw.clone(), format!("src:{i}"), "2026-11-22T00:00:00Z")
            })
            .collect();
        let groups = group_occurrences(&occurrences);
        // One group per distinct normalized string, and every
        // occurrence is grouped exactly once, original intact.
        let mut distinct: Vec<String> = raws.iter().map(|r| normalize_error_string(r)).collect();
        distinct.sort();
        distinct.dedup();
        prop_assert_eq!(groups.len(), distinct.len());
        prop_assert_eq!(
            groups.iter().map(|g| g.occurrences.len()).sum::<usize>(),
            occurrences.len()
        );
        for group in &groups {
            for occurrence in &group.occurrences {
                prop_assert_eq!(
                    normalize_error_string(&occurrence.original),
                    group.fingerprint.normalized.clone()
                );
            }
        }
    }
}
