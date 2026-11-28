//! Atlas/ID compatibility boundary for slice A9113-atlas-profile-contract
//! (issue #10179; design A-9113.2, S3.4–S3.10).
//!
//! Oracle: **golden** — the pinned 9110-R2 research disposition. R2 records
//! the production producer as unavailable, so this fixture has no chunk
//! algorithm, input bytes, profile, or known-answer vectors. Atlas chunk
//! identity remains opaque; the explicit Atlas profile is not interchangeable
//! with raw Iroh BLAKE3 and cannot be admitted as one by policy.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/id_shape.rs"]
#[allow(dead_code)]
mod id_shape;

#[path = "../src/content_id.rs"]
mod content_id;

use content_id::{ContentIdError, ContentTextForm, parse_content_text, to_iroh_hash};
use id_shape::{HashProfile, IdShapeError, ParsePolicy};
use serde_json::Value;

const ATLAS: HashProfile = HashProfile::AtlasChunkSha256Utf8;
const RAW: HashProfile = HashProfile::RawBlake3_256;
const VECTORS: &str = include_str!("../../../test-vectors/atlas-content-hash-v1.json");

#[test]
fn unavailable_producer_keeps_atlas_chunk_identity_opaque() {
    let vectors: Value = serde_json::from_str(VECTORS).unwrap();
    let chunk = &vectors["atlas_chunk_content_hash"];

    assert_eq!(vectors["schema"], "atlas-content-hash-compatibility-v1");
    assert_eq!(chunk["status"], "producer_unavailable");
    assert!(chunk["algorithm"].is_null());
    assert!(chunk["input_bytes"].is_null());
    assert!(chunk["profile"].is_null());
    assert_eq!(chunk["vectors"].as_array().unwrap().len(), 0);
    assert!(
        chunk["interop_claim"]
            .as_str()
            .unwrap()
            .contains("forbidden")
    );

    // A typed Atlas identifier is never an Iroh identifier. This checks the
    // seam only; no Atlas chunk hash is recomputed or inferred from the absent
    // producer vector.
    let text = format!("content/{}.{}", ATLAS.token(), "00".repeat(32));
    let atlas_id = parse_content_text(&text, ContentTextForm::Profiled, ParsePolicy::ALL).unwrap();
    assert_eq!(atlas_id.profile(), ATLAS);
    assert_eq!(
        to_iroh_hash(&atlas_id).unwrap_err(),
        ContentIdError::ProfileMismatch {
            expected: RAW,
            found: ATLAS,
        }
    );

    // A caller configured for raw Iroh IDs cannot reinterpret the foreign
    // Atlas profile, even when handed a syntactically valid profiled string.
    assert_eq!(
        parse_content_text(&text, ContentTextForm::Profiled, ParsePolicy::only(RAW)).unwrap_err(),
        ContentIdError::Shape(IdShapeError::UnsupportedProfile {
            profile: ATLAS.token().to_owned(),
        })
    );
}
