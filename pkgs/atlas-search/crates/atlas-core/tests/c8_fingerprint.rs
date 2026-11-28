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
//! The final test is this slice's share of the design's
//! investigation model test (verification contract): a fixture
//! with first vs earliest-retained-observation distinction
//! inputs, exact-vs-inferred edge labels, and missing-link gaps
//! preserved as gaps. The fixture's unauthorized-target and
//! unpushed-final-commit elements belong to the later live-
//! interface slices and are out of scope here.

use atlas_core::c8_fingerprint::{
    EdgeLabel, ErrorFingerprint, ErrorOccurrence, FirstObservationKind, InvestigationEdge,
    build_bundle, first_observation, group_occurrences, normalize_error_string,
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
