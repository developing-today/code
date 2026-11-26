use atlas_core::filters::Sensitivity;
use atlas_core::remediation::{
    build_report, DestinationChunk, MappingStatus, SourceChunk,
};

#[test]
fn maps_by_content_hash_and_classifies_incomplete_secret_destinations() {
    // Oracle: only equal content hashes with a destination vector are verified;
    // missing hashes, id-only matches, and absent vectors stay unresolved.
    let source = [
        SourceChunk { id: 1, content_hash: "hash-a".into() },
        SourceChunk { id: 2, content_hash: "hash-b".into() },
        SourceChunk { id: 3, content_hash: "hash-c".into() },
        SourceChunk { id: 4, content_hash: "hash-d".into() },
    ];
    let destination = [
        DestinationChunk { id: 11, content_hash: "hash-a".into(), has_vector: true },
        DestinationChunk { id: 2, content_hash: "different-hash".into(), has_vector: true },
        DestinationChunk { id: 12, content_hash: "hash-b".into(), has_vector: false },
    ];

    let report = build_report(
        "monolith",
        "atlas-core-secret-main-01",
        Sensitivity::SecretBearing,
        &source,
        &destination,
    );

    assert_eq!(report.counts.source, 4);
    assert_eq!(report.counts.destination, 3);
    assert_eq!(report.counts.verified, 1);
    assert_eq!(report.counts.hash_mismatch, 1);
    assert_eq!(report.counts.missing_vector, 1);
    assert_eq!(report.counts.missing_destination, 1);
    assert_eq!(report.mappings.iter().map(|mapping| mapping.status).collect::<Vec<_>>(), [
        MappingStatus::Verified,
        MappingStatus::MissingVector,
        MappingStatus::HashMismatch,
        MappingStatus::MissingDestination,
    ]);
}
