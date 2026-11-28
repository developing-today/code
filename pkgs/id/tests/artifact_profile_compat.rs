//! Artifact signed-bytes profile compatibility (slice 9110-11, issue
//! #10176; design S0.7, S0.13, S0.15).
//!
//! Oracle: **golden**. Legacy signature bytes verify unchanged, while
//! the new signed-envelope profile keeps its signature and IDs out of
//! the canonical body preimage. The verifiers are independent of the
//! code under test: Ed25519 `verify_strict` straight from
//! `ed25519-dalek` against the checked-in vectors in
//! `test-vectors/legacy-artifact-signatures.json`, and a second
//! deterministic-CBOR encoder written out below.
//!
//! The two profiles are separate formats. Legacy artifacts are compact
//! serde JSON and are never re-signed or re-serialized; the new profile
//! is the deterministic-CBOR signed envelope from id-core. A reader
//! tells them apart by their first byte
//! (`id_sync::artifact::record_profile`) and neither decoder accepts
//! the other's bytes. If a legacy assertion here ever fails, the fix is
//! to restore the old bytes, never to re-baseline a vector.
//!
//! The crate-root export of `signed_envelope` belongs to slice 9110-02,
//! so this test includes it, and the modules it builds on, by path, as
//! `crates/id-core/tests/signed_envelope_identity.rs` does.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../crates/id-core/src/canonical_cbor.rs"]
#[allow(dead_code)]
mod canonical_cbor;

#[path = "../crates/id-core/src/id_shape.rs"]
#[allow(dead_code)]
mod id_shape;

#[path = "../crates/id-core/src/hash_profile.rs"]
#[allow(dead_code)]
mod hash_profile;

#[path = "../crates/id-core/src/signed_envelope.rs"]
#[allow(dead_code)]
mod signed_envelope;

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use hash_profile::ProfileDispatch;
use id_shape::{ContentId, HashProfile};
use id_sync::artifact::{
    LEGACY_PROFILE, ProfileError, RecordProfile, SignedArtifact, record_profile, signed_bytes,
};
use serde_json::Value as Json;
use sha2::{Digest as _, Sha256};
use signed_envelope::{
    Body, Envelope, LogicalGraphId, PayloadId, SIGNATURE_DOMAIN_PREFIX, SUITE_ED25519,
    signature_preimage,
};

const VECTORS: &str = include_str!("../test-vectors/legacy-artifact-signatures.json");

fn unhex(text: &str) -> Vec<u8> {
    assert_eq!(text.len() % 2, 0, "odd hex length");
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn vectors() -> Json {
    serde_json::from_str(VECTORS).unwrap()
}

fn test_key(doc: &Json) -> SigningKey {
    let seed = unhex(doc["test_key"]["seed_hex"].as_str().unwrap());
    SigningKey::from_bytes(&<[u8; 32]>::try_from(seed.as_slice()).unwrap())
}

fn legacy_vectors(doc: &Json) -> &Vec<Json> {
    doc["legacy_profile"]["vectors"].as_array().unwrap()
}

/// The fields a legacy vector pins, as `SignedArtifact` holds them.
fn artifact_of(vector: &Json) -> SignedArtifact {
    SignedArtifact {
        author: vector["author"].as_str().unwrap().to_owned(),
        seq: vector["seq"].as_u64().unwrap(),
        prev: vector["prev"].as_str().unwrap().to_owned(),
        kind: vector["kind"].as_str().unwrap().to_owned(),
        body: vector["body"].clone(),
        after: vector["after"]
            .as_array()
            .unwrap()
            .iter()
            .map(|hash| hash.as_str().unwrap().to_owned())
            .collect(),
        hash: vector["hash"].as_str().unwrap().to_owned(),
        signature: vector["signature"].as_str().unwrap().to_owned(),
    }
}

fn legacy_bytes(vector: &Json) -> Vec<u8> {
    let artifact = artifact_of(vector);
    signed_bytes(
        &artifact.author,
        artifact.seq,
        &artifact.prev,
        &artifact.kind,
        &artifact.body,
        &artifact.after,
    )
    .unwrap()
}

#[test]
fn the_vector_file_is_the_one_this_slice_reads() {
    let doc = vectors();
    assert!(
        legacy_vectors(&doc).len() >= 4,
        "the vector set lost entries"
    );
    let author = test_key(&doc).verifying_key().to_bytes();
    for vector in legacy_vectors(&doc) {
        assert_eq!(vector["author"].as_str().unwrap(), hex(&author));
    }
    assert_eq!(LEGACY_PROFILE, "id-artifact-serde-json/v1");
}

#[test]
fn legacy_preimages_are_the_checked_in_bytes() {
    let doc = vectors();
    for vector in legacy_vectors(&doc) {
        let name = vector["name"].as_str().unwrap();
        let bytes = legacy_bytes(vector);
        assert_eq!(
            hex(&bytes),
            vector["signed_bytes_hex"].as_str().unwrap(),
            "{name}: legacy preimage changed"
        );
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            vector["signed_bytes_utf8"].as_str().unwrap(),
            "{name}: legacy preimage text changed"
        );
        // Plain JSON: no length prefix and no domain bytes before it.
        assert_eq!(bytes.first(), Some(&b'{'), "{name}: framed preimage");
        assert_eq!(
            hex(&Sha256::digest(&bytes)),
            vector["hash"].as_str().unwrap(),
            "{name}: legacy hash changed"
        );
    }
}

#[test]
fn legacy_signatures_verify_independently_and_unchanged() {
    let doc = vectors();
    let signer = test_key(&doc);
    for vector in legacy_vectors(&doc) {
        let name = vector["name"].as_str().unwrap();
        let preimage = unhex(vector["signed_bytes_hex"].as_str().unwrap());
        let author = unhex(vector["author"].as_str().unwrap());
        let key =
            VerifyingKey::from_bytes(&<[u8; 32]>::try_from(author.as_slice()).unwrap()).unwrap();
        let signature_bytes = unhex(vector["signature"].as_str().unwrap());
        let signature =
            Signature::from_bytes(&<[u8; 64]>::try_from(signature_bytes.as_slice()).unwrap());
        // Independent of the artifact module: the raw checked-in bytes.
        key.verify_strict(&preimage, &signature)
            .unwrap_or_else(|error| panic!("{name}: independent verify failed: {error}"));
        // Ed25519 is deterministic, so the test key re-signs the same
        // preimage to the very same signature.
        assert_eq!(
            hex(&signer.sign(&preimage).to_bytes()),
            vector["signature"].as_str().unwrap(),
            "{name}: re-signing drifted"
        );
        // And the production verifier accepts the stored artifact as is.
        artifact_of(vector)
            .verify()
            .unwrap_or_else(|error| panic!("{name}: artifact verify failed: {error}"));
    }
}

#[test]
fn a_changed_legacy_byte_no_longer_verifies() {
    let doc = vectors();
    let vector = &legacy_vectors(&doc)[0];
    let mut tampered = artifact_of(vector);
    tampered.seq += 1;
    assert!(tampered.verify().is_err(), "seq is not signed");
    let mut tampered = artifact_of(vector);
    tampered.kind.push('x');
    assert!(tampered.verify().is_err(), "kind is not signed");
}

// ---------------------------------------------------------------------
// A second deterministic-CBOR encoder (RFC 8949 section 4.2.1: shortest
// heads, definite lengths, map keys ordered by their encoded bytes),
// written without reference to `canonical_cbor`.
// ---------------------------------------------------------------------

enum Cbor {
    Uint(u64),
    Bytes(Vec<u8>),
    Text(String),
    Map(Vec<(String, Self)>),
}

fn head(major: u8, argument: u64, out: &mut Vec<u8>) {
    let major = major << 5;
    if argument < 24 {
        out.push(major | u8::try_from(argument).unwrap());
    } else if let Ok(small) = u8::try_from(argument) {
        out.extend([major | 0x18, small]);
    } else if let Ok(short) = u16::try_from(argument) {
        out.push(major | 0x19);
        out.extend(short.to_be_bytes());
    } else if let Ok(word) = u32::try_from(argument) {
        out.push(major | 0x1a);
        out.extend(word.to_be_bytes());
    } else {
        out.push(major | 0x1b);
        out.extend(argument.to_be_bytes());
    }
}

fn encode(value: &Cbor, out: &mut Vec<u8>) {
    match value {
        Cbor::Uint(n) => head(0, *n, out),
        Cbor::Bytes(bytes) => {
            head(2, bytes.len() as u64, out);
            out.extend(bytes);
        }
        Cbor::Text(text) => {
            head(3, text.len() as u64, out);
            out.extend(text.as_bytes());
        }
        Cbor::Map(entries) => {
            let mut encoded: Vec<(Vec<u8>, Vec<u8>)> = entries
                .iter()
                .map(|(key, value)| {
                    let mut k = Vec::new();
                    encode(&Cbor::Text(key.clone()), &mut k);
                    let mut v = Vec::new();
                    encode(value, &mut v);
                    (k, v)
                })
                .collect();
            encoded.sort_by(|(a, _), (b, _)| a.cmp(b));
            head(5, encoded.len() as u64, out);
            for (k, v) in encoded {
                out.extend(k);
                out.extend(v);
            }
        }
    }
}

fn content_id(id: &ContentId) -> Cbor {
    Cbor::Map(vec![
        (
            "profile".to_owned(),
            Cbor::Text(id.profile().token().to_owned()),
        ),
        ("digest".to_owned(), Cbor::Bytes(id.digest().to_vec())),
    ])
}

/// The body as the design lists it, independently encoded.
fn reference_body(body: &Body) -> Vec<u8> {
    let mut fields = vec![
        ("v".to_owned(), Cbor::Uint(1)),
        ("domain".to_owned(), Cbor::Text(body.domain.clone())),
        ("context".to_owned(), Cbor::Text(body.context.clone())),
        ("signer_id".to_owned(), Cbor::Text(body.signer_id.clone())),
        ("key".to_owned(), Cbor::Bytes(body.key.to_vec())),
        ("suite".to_owned(), Cbor::Text(body.suite.clone())),
        (
            "payload_id".to_owned(),
            content_id(body.payload_id.content_id()),
        ),
        ("payload_length".to_owned(), Cbor::Uint(body.payload_length)),
    ];
    if let Some(subject) = &body.subject {
        fields.push(("subject".to_owned(), content_id(subject)));
    }
    if let Some(graph) = &body.graph {
        fields.push(("graph".to_owned(), Cbor::Bytes(graph.as_bytes().to_vec())));
    }
    if let Some(predecessor) = &body.predecessor {
        fields.push((
            "predecessor".to_owned(),
            content_id(predecessor.content_id()),
        ));
    }
    let mut out = Vec::new();
    encode(&Cbor::Map(fields), &mut out);
    out
}

fn new_body(signer: &SigningKey, payload: &[u8], predecessor: Option<&Envelope>) -> Body {
    Body {
        domain: "id/artifact".to_owned(),
        context: "namespace/main".to_owned(),
        subject: Some(ContentId::of_bytes(HashProfile::RawBlake3_256, b"subject").unwrap()),
        graph: Some(LogicalGraphId::new([0x42; 32])),
        predecessor: predecessor.map(|env| env.id(HashProfile::RawBlake3_256).unwrap()),
        signer_id: "node-a".to_owned(),
        key: signer.verifying_key().to_bytes(),
        suite: SUITE_ED25519.to_owned(),
        payload_id: PayloadId::of(HashProfile::RawBlake3_256, payload).unwrap(),
        payload_length: payload.len() as u64,
    }
}

/// The new-profile envelope for the first legacy vector's content.
fn new_envelopes() -> (Envelope, Envelope) {
    let doc = vectors();
    let signer = test_key(&doc);
    let payload = legacy_bytes(&legacy_vectors(&doc)[0]);
    let first =
        Envelope::sign(new_body(&signer, &payload, None), payload.clone(), &signer).unwrap();
    let second = Envelope::sign(
        new_body(&signer, b"second", Some(&first)),
        b"second".to_vec(),
        &signer,
    )
    .unwrap();
    (first, second)
}

#[test]
fn the_new_body_matches_a_second_canonical_encoder() {
    let (first, second) = new_envelopes();
    for env in [&first, &second] {
        assert_eq!(
            hex(&env.body.canonical_bytes().unwrap()),
            hex(&reference_body(&env.body)),
        );
    }
    let minimal = Body {
        subject: None,
        graph: None,
        predecessor: None,
        ..first.body
    };
    assert_eq!(
        hex(&minimal.canonical_bytes().unwrap()),
        hex(&reference_body(&minimal))
    );
}

#[test]
fn the_new_preimage_is_prefix_framed_domain_and_body() {
    let (first, _) = new_envelopes();
    let mut expected = SIGNATURE_DOMAIN_PREFIX.to_vec();
    assert_eq!(expected, b"id/signed-body/v1\0");
    let domain = first.body.domain.as_bytes();
    expected.extend(u32::try_from(domain.len()).unwrap().to_be_bytes());
    expected.extend(domain);
    expected.extend(reference_body(&first.body));
    assert_eq!(signature_preimage(&first.body).unwrap(), expected);
    // Independent Ed25519 over the independently built preimage.
    VerifyingKey::from_bytes(&first.body.key)
        .unwrap()
        .verify_strict(&expected, &Signature::from_bytes(&first.signature))
        .unwrap();
    first.verify(&ProfileDispatch::default()).unwrap();
}

#[test]
fn the_new_preimage_excludes_its_signature_and_ids() {
    let (first, second) = new_envelopes();
    for env in [&first, &second] {
        let preimage = signature_preimage(&env.body).unwrap();
        assert!(
            !contains(&preimage, &env.signature),
            "signature in preimage"
        );
        assert!(!contains(&preimage, &env.payload), "payload in preimage");
        // The raw-byte profile is the one that admits CBOR envelopes;
        // the Atlas text profile only hashes UTF-8 chunk text.
        let own_id = env.id(HashProfile::RawBlake3_256).unwrap();
        assert!(
            !contains(&preimage, own_id.content_id().digest()),
            "own envelope ID in preimage"
        );
    }
    // A predecessor's ID is a reference the body commits on purpose.
    let preimage = signature_preimage(&second.body).unwrap();
    let predecessor = first.id(HashProfile::RawBlake3_256).unwrap();
    assert!(contains(&preimage, predecessor.content_id().digest()));
}

#[test]
fn the_envelope_id_follows_the_signature_and_the_body_does_not() {
    let (first, _) = new_envelopes();
    let mut resigned = first.clone();
    resigned.signature[0] ^= 1;
    assert_eq!(
        resigned.body.canonical_bytes().unwrap(),
        first.body.canonical_bytes().unwrap()
    );
    assert_eq!(resigned.body.payload_id, first.body.payload_id);
    assert_ne!(
        resigned.id(HashProfile::RawBlake3_256).unwrap(),
        first.id(HashProfile::RawBlake3_256).unwrap()
    );
    assert!(resigned.verify(&ProfileDispatch::default()).is_err());
}

#[test]
fn the_new_profile_round_trips_and_leaves_legacy_bytes_alone() {
    let doc = vectors();
    let (first, _) = new_envelopes();
    let bytes = first.canonical_bytes().unwrap();
    assert_eq!(Envelope::decode(&bytes).unwrap(), first);
    // Wrapping a legacy artifact as a payload copies its bytes exactly.
    assert_eq!(
        hex(&first.payload),
        legacy_vectors(&doc)[0]["signed_bytes_hex"]
            .as_str()
            .unwrap()
    );
}

#[test]
fn readers_tell_the_two_profiles_apart() {
    let doc = vectors();
    let (first, second) = new_envelopes();
    for vector in legacy_vectors(&doc) {
        let stored = serde_json::to_vec(&artifact_of(vector)).unwrap();
        assert_eq!(
            record_profile(&stored),
            Ok(RecordProfile::LegacyArtifactJson)
        );
        assert!(Envelope::decode(&stored).is_err(), "envelope read JSON");
    }
    for env in [&first, &second] {
        let stored = env.canonical_bytes().unwrap();
        assert_eq!(
            record_profile(&stored),
            Ok(RecordProfile::SignedEnvelopeCbor)
        );
        assert!(
            serde_json::from_slice::<SignedArtifact>(&stored).is_err(),
            "artifact read CBOR"
        );
    }
    assert_eq!(record_profile(b""), Err(ProfileError::Empty));
    assert_eq!(
        record_profile(b" {}"),
        Err(ProfileError::Unrecognized(b' '))
    );
    assert_eq!(
        record_profile(&[0x80]),
        Err(ProfileError::Unrecognized(0x80))
    );
}
