//! Identity-separation tests for signed envelopes (slice
//! A9114-envelope-identities, issue #10226; design A-9114.1–A-9114.3,
//! S3.11–S3.20).
//!
//! Oracle: **invariant** — reference functions over canonical bytes. A
//! payload ID is the profile hash of the exact raw payload; the
//! signature preimage is the fixed prefix, the framed domain and the
//! canonical body, and contains neither the signature nor the ID of its
//! own enclosing envelope; an envelope ID is the profile hash of the
//! complete canonical envelope and changes whenever the signature does;
//! a logical graph ID stays put across assertions. Every rejection is
//! checked for its exact failure class.
//!
//! The crate-root export of `signed_envelope` belongs to slice 9110-02,
//! so until that lands this test includes it, and the modules it builds
//! on, by path.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/canonical_cbor.rs"]
#[allow(dead_code)]
mod canonical_cbor;

#[path = "../src/id_shape.rs"]
#[allow(dead_code)]
mod id_shape;

#[path = "../src/hash_profile.rs"]
#[allow(dead_code)]
mod hash_profile;

#[path = "../src/signed_envelope.rs"]
mod signed_envelope;

use canonical_cbor::{CborError, Value, decode_canonical, encode_canonical};
use ed25519_dalek::SigningKey;
use hash_profile::ProfileDispatch;
use id_shape::{ContentId, HashProfile, ParsePolicy};
use proptest::prelude::*;
use signed_envelope::{
    Body, Envelope, EnvelopeError, EnvelopeId, LogicalGraphId, PayloadId, SIGNATURE_DOMAIN_PREFIX,
    SUITE_ED25519, signature_preimage,
};

const RAW: HashProfile = HashProfile::RawBlake3_256;

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn body_for(signer: &SigningKey, payload: &[u8], profile: HashProfile) -> Body {
    Body {
        domain: "id/sidecar".to_owned(),
        context: "namespace/main".to_owned(),
        subject: Some(ContentId::of_bytes(RAW, b"subject file").unwrap()),
        graph: None,
        predecessor: None,
        signer_id: "node-a".to_owned(),
        key: signer.verifying_key().to_bytes(),
        suite: SUITE_ED25519.to_owned(),
        payload_id: PayloadId::of(profile, payload).unwrap(),
        payload_length: payload.len() as u64,
    }
}

fn envelope(seed: u8, payload: &[u8]) -> Envelope {
    let signer = key(seed);
    Envelope::sign(body_for(&signer, payload, RAW), payload.to_vec(), &signer).unwrap()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

#[test]
fn payload_id_is_the_raw_exact_payload_hash() {
    // BLAKE3("abc"), with no prefix or framing added.
    let id = PayloadId::of(RAW, b"abc").unwrap();
    assert_eq!(
        hex(id.content_id().digest()),
        "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
    );
    assert_eq!(id.content_id().profile(), RAW);
    // SHA-256("abc") under the Atlas text profile.
    let atlas = PayloadId::of(HashProfile::AtlasChunkSha256Utf8, b"abc").unwrap();
    assert_eq!(
        hex(atlas.content_id().digest()),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn preimage_is_prefix_framed_domain_and_canonical_body() {
    let env = envelope(1, b"metadata v1");
    let body_bytes = env.body.canonical_bytes().unwrap();
    let mut expected = SIGNATURE_DOMAIN_PREFIX.to_vec();
    expected.extend_from_slice(&10u32.to_be_bytes());
    expected.extend_from_slice(b"id/sidecar");
    expected.extend_from_slice(&body_bytes);
    assert_eq!(signature_preimage(&env.body).unwrap(), expected);
    // The body commits references and lengths, never the payload or the
    // signature, and has no field that could hold either.
    let Value::Map(fields) = decode_canonical(&body_bytes).unwrap() else {
        panic!("body is not a map");
    };
    let names: Vec<String> = fields
        .iter()
        .map(|(k, _)| match k {
            Value::Text(name) => name.clone(),
            other => panic!("non-text key {other:?}"),
        })
        .collect();
    assert_eq!(
        names,
        [
            "v",
            "key",
            "suite",
            "domain",
            "context",
            "subject",
            "signer_id",
            "payload_id",
            "payload_length"
        ]
    );
}

#[test]
fn envelope_id_hashes_the_complete_canonical_envelope() {
    let env = envelope(2, b"payload bytes");
    let bytes = env.canonical_bytes().unwrap();
    // The envelope is {body, payload, signature} and nothing else.
    let Value::Map(fields) = decode_canonical(&bytes).unwrap() else {
        panic!("envelope is not a map");
    };
    assert_eq!(
        fields.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>(),
        [
            Value::Text("body".to_owned()),
            Value::Text("payload".to_owned()),
            Value::Text("signature".to_owned()),
        ]
    );
    let id = env.id(RAW).unwrap();
    assert_eq!(*id.content_id(), ContentId::of_bytes(RAW, &bytes).unwrap());
    // Not embedded anywhere in the bytes it hashes.
    assert!(!contains(&bytes, id.content_id().digest()));
    // The envelope ID, payload ID and subject are three distinct IDs.
    assert_ne!(id.content_id(), env.body.payload_id.content_id());
    assert_ne!(Some(*id.content_id()), env.body.subject);
}

#[test]
fn round_trip_and_verify() {
    let env = envelope(3, b"round trip");
    let bytes = env.canonical_bytes().unwrap();
    let decoded = Envelope::decode(&bytes).unwrap();
    assert_eq!(decoded, env);
    assert_eq!(decoded.canonical_bytes().unwrap(), bytes);
    decoded.verify(&ProfileDispatch::default()).unwrap();
}

#[test]
fn signed_predecessor_reference_is_not_a_self_reference() {
    let first = envelope(4, b"generation 1");
    let first_id = first.id(RAW).unwrap();
    let signer = key(4);
    let mut body = body_for(&signer, b"generation 2", RAW);
    body.predecessor = Some(first_id);
    let second = Envelope::sign(body, b"generation 2".to_vec(), &signer).unwrap();
    second.verify(&ProfileDispatch::default()).unwrap();
    let preimage = signature_preimage(&second.body).unwrap();
    // The predecessor's ID is signed; the envelope's own ID is not.
    assert!(contains(&preimage, first_id.content_id().digest()));
    let second_id = second.id(RAW).unwrap();
    assert!(!contains(&preimage, second_id.content_id().digest()));
    assert_eq!(
        Envelope::decode(&second.canonical_bytes().unwrap())
            .unwrap()
            .body
            .predecessor,
        Some(first_id)
    );
}

#[test]
fn logical_graph_id_is_stable_across_assertions() {
    let graph = LogicalGraphId::new([0x47; 32]);
    let signer = key(5);
    let mut ids = Vec::new();
    for payload in [&b"assertion one"[..], b"assertion two", b"assertion three"] {
        let mut body = body_for(&signer, payload, RAW);
        body.graph = Some(graph);
        let env = Envelope::sign(body, payload.to_vec(), &signer).unwrap();
        let decoded = Envelope::decode(&env.canonical_bytes().unwrap()).unwrap();
        assert_eq!(decoded.body.graph, Some(graph));
        ids.push(env.id(RAW).unwrap());
    }
    // Same graph, three distinct assertion identities, none equal to the
    // graph address.
    assert_ne!(ids[0], ids[1]);
    assert_ne!(ids[1], ids[2]);
    assert_ne!(ids[0], ids[2]);
    for id in ids {
        assert_ne!(id.content_id().digest(), graph.as_bytes());
    }
}

#[test]
fn rejections_carry_their_exact_class() {
    let dispatch = ProfileDispatch::default();

    // Same-length payload swap: the payload ID no longer matches.
    let mut env = envelope(6, b"original");
    env.payload = b"replaced".to_vec();
    assert_eq!(
        env.verify(&dispatch).unwrap_err().class(),
        "digest_mismatch"
    );

    // Length differs from the committed length.
    let mut env = envelope(6, b"original");
    env.payload.push(b'!');
    assert_eq!(
        env.verify(&dispatch).unwrap_err(),
        EnvelopeError::PayloadLength {
            declared: 8,
            actual: 9
        }
    );

    // Any body change breaks the signature.
    let mut env = envelope(6, b"original");
    env.body.context = "namespace/other".to_owned();
    assert_eq!(
        env.verify(&dispatch).unwrap_err(),
        EnvelopeError::BadSignature
    );

    // A payload profile this node does not admit is unsupported, and is
    // not judged a mismatch.
    let signer = key(7);
    let text = b"utf-8 text";
    let atlas = Envelope::sign(
        body_for(&signer, text, HashProfile::AtlasChunkSha256Utf8),
        text.to_vec(),
        &signer,
    )
    .unwrap();
    atlas.verify(&dispatch).unwrap();
    let only_raw = ProfileDispatch::new(ParsePolicy::only(RAW), 1 << 20);
    assert_eq!(
        atlas.verify(&only_raw).unwrap_err().class(),
        "unsupported_profile"
    );

    // Non-UTF-8 payload under the Atlas text profile.
    let mut bad = atlas;
    bad.payload = b"\xff\xfef-8 text".to_vec();
    assert_eq!(
        bad.verify(&dispatch).unwrap_err().class(),
        "invalid_profile_parameters"
    );

    // Signing with a key other than the body's.
    let body = body_for(&key(8), b"x", RAW);
    assert_eq!(
        Envelope::sign(body, b"x".to_vec(), &key(9)).unwrap_err(),
        EnvelopeError::KeyMismatch
    );

    // Unsupported suite.
    let mut body = body_for(&key(8), b"x", RAW);
    body.suite = "rsa".to_owned();
    assert_eq!(
        Envelope::sign(body, b"x".to_vec(), &key(8))
            .unwrap_err()
            .class(),
        "unsupported"
    );
}

/// Re-encodes an envelope after editing its body map.
fn with_body_edit(env: &Envelope, edit: impl FnOnce(&mut Vec<(Value, Value)>)) -> Vec<u8> {
    let Value::Map(mut fields) = env.to_value().unwrap() else {
        panic!("envelope is not a map");
    };
    let Value::Map(body) = &mut fields[0].1 else {
        panic!("body is not a map");
    };
    edit(body);
    encode_canonical(&Value::Map(fields)).unwrap()
}

#[test]
fn decode_rejects_unknown_missing_and_alternative_encodings() {
    let env = envelope(10, b"strict");

    // Unknown field: every V1 field is critical.
    let unknown = with_body_edit(&env, |body| {
        body.push((Value::Text("extra".to_owned()), Value::UInt(1)));
    });
    assert_eq!(
        Envelope::decode(&unknown).unwrap_err(),
        EnvelopeError::Cbor(CborError::UnknownCriticalField {
            field: "extra".to_owned()
        })
    );

    // An absent optional is omitted; null is not a second encoding of it.
    let null_graph = with_body_edit(&env, |body| {
        body.push((Value::Text("graph".to_owned()), Value::Null));
    });
    assert_eq!(
        Envelope::decode(&null_graph).unwrap_err().class(),
        "malformed"
    );

    // A missing required field.
    let missing = with_body_edit(&env, |body| {
        body.retain(|(k, _)| *k != Value::Text("payload_length".to_owned()));
    });
    assert_eq!(
        Envelope::decode(&missing).unwrap_err(),
        EnvelopeError::Field {
            field: "payload_length",
            problem: "is missing"
        }
    );

    // A future body version.
    let future = with_body_edit(&env, |body| {
        for (k, v) in body.iter_mut() {
            if *k == Value::Text("v".to_owned()) {
                *v = Value::UInt(2);
            }
        }
    });
    assert_eq!(
        Envelope::decode(&future).unwrap_err(),
        EnvelopeError::UnsupportedVersion(2)
    );

    // An unknown profile token inside a signed reference.
    let unknown_profile = with_body_edit(&env, |body| {
        for (k, v) in body.iter_mut() {
            if *k == Value::Text("subject".to_owned()) {
                let Value::Map(id) = v else { panic!() };
                id[1].1 = Value::Text("sha3-256".to_owned());
            }
        }
    });
    assert_eq!(
        Envelope::decode(&unknown_profile).unwrap_err().class(),
        "unsupported_profile"
    );

    // Non-canonical bytes: trailing data after the envelope.
    let mut trailing = env.canonical_bytes().unwrap();
    trailing.push(0);
    assert_eq!(
        Envelope::decode(&trailing).unwrap_err(),
        EnvelopeError::Cbor(CborError::TrailingBytes)
    );
}

prop_compose! {
    fn arb_envelope()(
        seed in 1u8..=255,
        payload in proptest::collection::vec(any::<u8>(), 0..256),
        domain in "[a-z/]{1,32}",
        context in "\\PC{1,40}",
        graph in proptest::option::of(any::<[u8; 32]>()),
        predecessor in proptest::option::of(any::<[u8; 32]>()),
        subject in proptest::option::of(any::<[u8; 32]>()),
    ) -> Envelope {
        let signer = key(seed);
        let mut body = body_for(&signer, &payload, RAW);
        body.domain = domain;
        body.context = context;
        body.graph = graph.map(LogicalGraphId::new);
        body.predecessor =
            predecessor.map(|d| EnvelopeId::from_content_id(ContentId::new(RAW, d)));
        body.subject = subject.map(|d| ContentId::new(RAW, d));
        Envelope::sign(body, payload, &signer).unwrap()
    }
}

proptest! {
    #[test]
    fn preimage_never_holds_its_own_envelope_id_or_signature(env in arb_envelope()) {
        let preimage = signature_preimage(&env.body).unwrap();
        let id = env.id(RAW).unwrap();
        prop_assert!(!contains(&preimage, id.content_id().digest()));
        prop_assert!(!contains(&preimage, &env.signature));
        prop_assert!(preimage.starts_with(SIGNATURE_DOMAIN_PREFIX));
        prop_assert!(preimage.ends_with(&env.body.canonical_bytes().unwrap()));
    }

    #[test]
    fn envelope_id_changes_when_the_signature_changes(
        env in arb_envelope(),
        index in 0usize..64,
        flip in 1u8..=255,
    ) {
        let original_id = env.id(RAW).unwrap();
        let body_bytes = env.body.canonical_bytes().unwrap();
        let mut tampered = env;
        tampered.signature[index] ^= flip;
        // The body and its preimage are unchanged; the envelope ID is not.
        prop_assert_eq!(tampered.body.canonical_bytes().unwrap(), body_bytes);
        prop_assert_ne!(tampered.id(RAW).unwrap(), original_id);
        prop_assert_eq!(
            tampered.verify(&ProfileDispatch::default()).unwrap_err().class(),
            "bad_signature"
        );
    }

    #[test]
    fn decode_encode_round_trips_and_verifies(env in arb_envelope()) {
        let bytes = env.canonical_bytes().unwrap();
        let decoded = Envelope::decode(&bytes).unwrap();
        prop_assert_eq!(&decoded, &env);
        prop_assert_eq!(decoded.canonical_bytes().unwrap(), bytes);
        prop_assert!(decoded.verify(&ProfileDispatch::default()).is_ok());
    }

    #[test]
    fn payload_id_is_exactly_the_profile_hash(payload in proptest::collection::vec(any::<u8>(), 0..512)) {
        prop_assert_eq!(
            *PayloadId::of(RAW, &payload).unwrap().content_id(),
            ContentId::new(RAW, *blake3::hash(&payload).as_bytes())
        );
    }

    #[test]
    fn any_single_byte_change_to_the_envelope_never_verifies_as_the_same_id(
        env in arb_envelope(),
        index in any::<usize>(),
        flip in 1u8..=255,
    ) {
        let bytes = env.canonical_bytes().unwrap();
        let id = env.id(RAW).unwrap();
        let mut corrupted = bytes;
        let i = index % corrupted.len();
        corrupted[i] ^= flip;
        // Either the bytes no longer decode, or they decode to an
        // envelope that fails verification or has a different ID.
        if let Ok(other) = Envelope::decode(&corrupted) {
            let still_valid = other.verify(&ProfileDispatch::default()).is_ok();
            prop_assert!(!still_valid || other.id(RAW).unwrap() != id);
        }
    }
}
