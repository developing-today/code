//! Typed triple terms and canonical signed graph records (slice
//! A-GRAPH-9125-01, issue #10196; design part A §3.1, §3.2 and §4.2,
//! statements S5.1–S5.7, S5.11).
//!
//! Oracle: **reference**. The known-answer constants below were computed
//! outside this codebase, by a hand-written Python canonical-CBOR encoder
//! with the `blake3` package and `cryptography`'s Ed25519, from the
//! design's wire forms. The code under test must reproduce them byte for
//! byte: exact canonical body, raw payload ID, profile-byte content IDs,
//! the complete signed-envelope ID and `source_record` provenance. A
//! second canonical-CBOR encoder written in this file checks arbitrary
//! typed-byte terms and triples, and every ambiguous or self-referential
//! encoding must reject with its exact failure class. No vector hashes
//! an ID into its own preimage.
//!
//! `graph/mod.rs` and the crate root do not export these modules yet
//! (slice 9110-09 wires them), so this test includes them, and the
//! modules they build on, by path.

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
#[allow(dead_code)]
mod signed_envelope;

// `record.rs` reaches `term.rs` as `super::term`, which resolves the same
// way here (both at this crate's root) as under `graph/` in id-core.
#[path = "../src/graph/term.rs"]
mod term;

#[path = "../src/graph/record.rs"]
mod record;

use canonical_cbor::{CborError, Value, encode_canonical};
use ed25519_dalek::{Signature, SigningKey, VerifyingKey};
use hash_profile::ProfileDispatch;
use id_shape::{ContentId, HashProfile};
use proptest::prelude::*;
use record::{GRAPH_RECORD_DOMAIN, GraphRecord, MAX_TRIPLES, RECORD_ID_PROFILE, RecordError, open};
use signed_envelope::{
    Body, Envelope, EnvelopeId, LogicalGraphId, PayloadId, SIGNATURE_DOMAIN_PREFIX, SUITE_ED25519,
    signature_preimage,
};
use term::{MAX_TERM_BYTES, ObjectRef, Term, TermError, Triple};

const RAW: HashProfile = HashProfile::RawBlake3_256;

// Known answers from the independent Python encoder.
const TERM_TYPED: &str = "830002446e616d65";
const TERM_CONTENT: &str = concat!(
    "83016e7261772d626c616b65332d3235365820",
    "16482bb7fb9567571e6d3f2a8a962bfe9225ece0d57901772bff058d6bb6d66f",
);
const PAYLOAD: &str = concat!(
    "a4617601656772617068582011111111111111111111111111111111111111111111111111111111111111",
    "1167747269706c657383838300014a75726e3a757365723a31830002446e616d6583000346557365722031",
    "838300014a75726e3a757365723a318300024661766174617283016e7261772d626c616b65332d32353658",
    "2016482bb7fb9567571e6d3f2a8a962bfe9225ece0d57901772bff058d6bb6d66f838300014a75726e3a75",
    "7365723a3183000245636974657383026e7261772d626c616b65332d323536582022222222222222222222",
    "222222222222222222222222222222222222222222226d736f757263655f7265636f726483026e7261772d",
    "626c616b65332d32353658203333333333333333333333333333333333333333333333333333333333333333",
);
const PAYLOAD_ID: &str = "ccb72aa99f3793ae283772b0424bee488b0d38bee228da5c20550c81cc70281d";
const BODY: &str = concat!(
    "a9617601636b65795820ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c65",
    "677261706858201111111111111111111111111111111111111111111111111111111111111111657375",
    "697465676564323535313966646f6d61696e6f69642f67726170682d7265636f726467636f6e74657874",
    "6e6e616d6573706163652f6d61696e697369676e65725f6964666e6f64652d616a7061796c6f61645f69",
    "64a2666469676573745820ccb72aa99f3793ae283772b0424bee488b0d38bee228da5c20550c81cc7028",
    "1d6770726f66696c656e7261772d626c616b65332d3235366e7061796c6f61645f6c656e67746819012e",
);
const SIGNATURE: &str = concat!(
    "b1e966571cce9c509de68b7d634460876b11205bb811598b7409ef5d861b6cfe",
    "8691908085a03dec1c829e6a12354b6c658cfd1cd7ff339b538a5131b37aad0c",
);
const ENVELOPE_ID: &str = "9d65e8755bc058c6dc205f87adad03ba028f86571116bd462cece09737d568d6";

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn key() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}

const fn graph_id() -> LogicalGraphId {
    LogicalGraphId::new([0x11; 32])
}

fn typed(tag: u64, bytes: &[u8]) -> Term {
    Term::typed(tag, bytes.to_vec())
}

fn envelope_ref(byte: u8) -> EnvelopeId {
    EnvelopeId::from_content_id(ContentId::new(RAW, [byte; 32]))
}

fn known_record() -> GraphRecord {
    let user = || typed(1, b"urn:user:1");
    GraphRecord {
        graph: graph_id(),
        triples: vec![
            Triple::new(user(), typed(2, b"name"), typed(3, b"User 1")),
            Triple::new(
                user(),
                typed(2, b"avatar"),
                Term::Object(ObjectRef::Content(
                    ContentId::of_bytes(RAW, b"avatar").unwrap(),
                )),
            ),
            Triple::new(
                user(),
                typed(2, b"cites"),
                Term::Object(ObjectRef::Envelope(envelope_ref(0x22))),
            ),
        ],
        source_record: Some(envelope_ref(0x33)),
    }
}

#[test]
fn terms_match_the_known_answers() {
    assert_eq!(hex(&typed(2, b"name").to_bytes().unwrap()), TERM_TYPED);
    let avatar = Term::Object(ObjectRef::Content(
        ContentId::of_bytes(RAW, b"avatar").unwrap(),
    ));
    assert_eq!(hex(&avatar.to_bytes().unwrap()), TERM_CONTENT);
    // The content ID is the profile hash of the exact bytes.
    assert_eq!(
        hex(ContentId::of_bytes(RAW, b"avatar").unwrap().digest()),
        hex(blake3::hash(b"avatar").as_bytes())
    );
    assert_eq!(Term::from_bytes(&unhex(TERM_CONTENT)).unwrap(), avatar);
}

#[test]
fn a_signed_record_matches_the_known_answers() {
    let signed = known_record()
        .sign("namespace/main", "node-a", &key())
        .unwrap();
    let env = &signed.envelope;
    assert_eq!(hex(&env.payload), PAYLOAD);
    // PayloadId is the raw BLAKE3 of the payload bytes, with no framing.
    assert_eq!(hex(env.body.payload_id.content_id().digest()), PAYLOAD_ID);
    assert_eq!(
        hex(blake3::hash(&unhex(PAYLOAD)).as_bytes()),
        PAYLOAD_ID,
        "the vector itself is off"
    );
    assert_eq!(env.body.payload_length, unhex(PAYLOAD).len() as u64);
    assert_eq!(hex(&env.body.canonical_bytes().unwrap()), BODY);
    assert_eq!(hex(&env.signature), SIGNATURE);
    assert_eq!(hex(signed.id.content_id().digest()), ENVELOPE_ID);
    assert_eq!(signed.id.content_id().profile(), RECORD_ID_PROFILE);
    // EnvelopeId hashes the complete canonical envelope, signature and all.
    assert_eq!(
        hex(blake3::hash(&env.canonical_bytes().unwrap()).as_bytes()),
        ENVELOPE_ID
    );
}

#[test]
fn the_signature_verifies_independently_over_the_framed_body() {
    let mut preimage = SIGNATURE_DOMAIN_PREFIX.to_vec();
    preimage.extend(
        u32::try_from(GRAPH_RECORD_DOMAIN.len())
            .unwrap()
            .to_be_bytes(),
    );
    preimage.extend(GRAPH_RECORD_DOMAIN.as_bytes());
    preimage.extend(unhex(BODY));
    let signature = <[u8; 64]>::try_from(unhex(SIGNATURE).as_slice()).unwrap();
    key()
        .verifying_key()
        .verify_strict(&preimage, &Signature::from_bytes(&signature))
        .unwrap();
    let signed = known_record()
        .sign("namespace/main", "node-a", &key())
        .unwrap();
    assert_eq!(signature_preimage(&signed.envelope.body).unwrap(), preimage);
}

#[test]
fn no_identity_is_in_its_own_preimage() {
    let signed = known_record()
        .sign("namespace/main", "node-a", &key())
        .unwrap();
    let body = unhex(BODY);
    let payload = unhex(PAYLOAD);
    let contains =
        |haystack: &[u8], needle: &[u8]| haystack.windows(needle.len()).any(|w| w == needle);
    let own = signed.id.content_id().digest();
    assert!(!contains(&body, own), "envelope ID in its body");
    assert!(!contains(&payload, own), "envelope ID in its payload");
    assert!(
        !contains(&body, &signed.envelope.signature),
        "signature in body"
    );
    assert!(
        !contains(&payload, &unhex(PAYLOAD_ID)),
        "payload ID in payload"
    );
    // The body commits the payload by reference, never the bytes.
    assert!(contains(&body, &unhex(PAYLOAD_ID)));
    assert!(!contains(&body, &payload));
}

#[test]
fn opening_round_trips_and_keeps_provenance() {
    let signed = known_record()
        .sign("namespace/main", "node-a", &key())
        .unwrap();
    let bytes = signed.envelope.canonical_bytes().unwrap();
    let opened = open(
        Envelope::decode(&bytes).unwrap(),
        &ProfileDispatch::default(),
    )
    .unwrap();
    assert_eq!(opened, signed);
    let assertions: Vec<_> = opened.assertions().collect();
    assert_eq!(assertions.len(), 3);
    for (index, assertion) in assertions.iter().enumerate() {
        assert_eq!(assertion.record, signed.id);
        assert_eq!(assertion.index as usize, index);
        assert_eq!(assertion.graph, graph_id());
        assert_eq!(assertion.triple, &known_record().triples[index]);
        assert_eq!(assertion.source_record, Some(envelope_ref(0x33)));
    }
    // A second record citing the first keeps the first's ID as evidence,
    // and the logical graph stays put.
    let follow_up = GraphRecord {
        source_record: Some(signed.id),
        ..GraphRecord::new(graph_id(), vec![])
    };
    let second = follow_up.sign("namespace/main", "node-a", &key()).unwrap();
    assert_ne!(second.id, signed.id);
    assert_eq!(second.envelope.body.graph, signed.envelope.body.graph);
    assert_eq!(
        second.assertions().count(),
        0,
        "an empty record is still a record"
    );
    assert_eq!(
        open(second.envelope, &ProfileDispatch::default())
            .unwrap()
            .record
            .source_record,
        Some(signed.id)
    );
}

#[test]
fn self_references_reject() {
    let record = known_record();
    let payload = record.payload_bytes().unwrap();
    // Naming the enclosing envelope as its own source.
    let error = record
        .check_not_self_referential(&envelope_ref(0x33), &payload)
        .unwrap_err();
    assert_eq!(error, RecordError::SelfReference);
    assert_eq!(error.class(), "self_reference");
    // Naming the enclosing envelope from a reference term.
    assert_eq!(
        record.check_not_self_referential(&envelope_ref(0x22), &payload),
        Err(RecordError::SelfReference)
    );
    // Naming the payload it is carried in.
    assert_eq!(
        record.check_not_self_referential(&envelope_ref(0x44), b"avatar"),
        Err(RecordError::SelfReference)
    );
    record
        .check_not_self_referential(&envelope_ref(0x44), &payload)
        .unwrap();
}

// ---------------------------------------------------------------------
// A second canonical-CBOR encoder for terms and triples, written from
// RFC 8949 §4.2.1 without reference to `canonical_cbor`.
// ---------------------------------------------------------------------

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

fn reference_term(term: &Term, out: &mut Vec<u8>) {
    head(4, 3, out);
    let (kind, id) = match term {
        Term::Typed { type_tag, bytes } => {
            head(0, 0, out);
            head(0, *type_tag, out);
            head(2, bytes.len() as u64, out);
            out.extend(bytes);
            return;
        }
        Term::Object(ObjectRef::Content(id)) => (1, *id),
        Term::Object(ObjectRef::Envelope(id)) => (2, *id.content_id()),
    };
    head(0, kind, out);
    let token = id.profile().token();
    head(3, token.len() as u64, out);
    out.extend(token.as_bytes());
    head(2, 32, out);
    out.extend(id.digest());
}

fn reference_triple(triple: &Triple) -> Vec<u8> {
    let mut out = Vec::new();
    head(4, 3, &mut out);
    for term in triple.terms() {
        reference_term(term, &mut out);
    }
    out
}

fn any_term() -> impl Strategy<Value = Term> {
    prop_oneof![
        (any::<u64>(), proptest::collection::vec(any::<u8>(), 0..300))
            .prop_map(|(tag, bytes)| Term::typed(tag, bytes)),
        (any::<[u8; 32]>(), any::<bool>()).prop_map(|(digest, raw)| {
            let profile = if raw {
                RAW
            } else {
                HashProfile::AtlasChunkSha256Utf8
            };
            Term::Object(ObjectRef::Content(ContentId::new(profile, digest)))
        }),
        any::<[u8; 32]>().prop_map(|digest| Term::Object(ObjectRef::Envelope(
            EnvelopeId::from_content_id(ContentId::new(RAW, digest))
        ))),
    ]
}

proptest! {
    #[test]
    fn arbitrary_triples_match_the_reference_and_round_trip(
        subject in any_term(),
        predicate in any_term(),
        object in any_term(),
    ) {
        let triple = Triple::new(subject, predicate, object);
        let bytes = triple.to_bytes().unwrap();
        prop_assert_eq!(hex(&bytes), hex(&reference_triple(&triple)));
        prop_assert_eq!(Triple::from_bytes(&bytes).unwrap(), triple);
    }

    #[test]
    fn arbitrary_records_round_trip_through_a_signature(
        terms in proptest::collection::vec(any_term(), 0..12),
        graph in any::<[u8; 32]>(),
    ) {
        let triples: Vec<Triple> = terms
            .chunks_exact(3)
            .map(|t| Triple::new(t[0].clone(), t[1].clone(), t[2].clone()))
            .collect();
        let record = GraphRecord::new(LogicalGraphId::new(graph), triples);
        let signed = record.sign("ctx", "node-a", &key()).unwrap();
        let bytes = signed.envelope.canonical_bytes().unwrap();
        let opened = open(Envelope::decode(&bytes).unwrap(), &ProfileDispatch::default()).unwrap();
        prop_assert_eq!(opened.record, record);
        prop_assert_eq!(
            hex(opened.id.content_id().digest()),
            hex(blake3::hash(&bytes).as_bytes())
        );
    }
}

#[test]
fn bytes_are_opaque_and_kinds_stay_distinct() {
    // Unknown tags are preserved; nothing is normalized.
    for term in [
        typed(u64::MAX, b"\0\xff"),
        typed(0, b""),
        typed(7, "Cafe\u{301}".as_bytes()),
    ] {
        assert_eq!(Term::from_bytes(&term.to_bytes().unwrap()).unwrap(), term);
    }
    assert_ne!(
        typed(7, "Caf\u{e9}".as_bytes()),
        typed(7, "Cafe\u{301}".as_bytes())
    );
    // An IRI-looking string is bytes, not a reference, and a typed term
    // holding a digest is not the reference to that digest.
    let digest = *ContentId::of_bytes(RAW, b"avatar").unwrap().digest();
    let as_bytes = typed(1, &digest);
    let as_ref = Term::Object(ObjectRef::Content(ContentId::new(RAW, digest)));
    assert_ne!(as_bytes, as_ref);
    assert_ne!(as_bytes.to_bytes().unwrap(), as_ref.to_bytes().unwrap());
    // The same digest as content and as envelope are different terms.
    let as_envelope = Term::Object(ObjectRef::Envelope(EnvelopeId::from_content_id(
        ContentId::new(RAW, digest),
    )));
    assert_ne!(as_ref.to_bytes().unwrap(), as_envelope.to_bytes().unwrap());
}

fn encode(value: &Value) -> Vec<u8> {
    encode_canonical(value).unwrap()
}

fn term_error(value: &Value) -> TermError {
    Term::from_bytes(&encode(value)).unwrap_err()
}

#[test]
fn ambiguous_terms_reject() {
    let uint = Value::UInt;
    let bytes = |b: &[u8]| Value::Bytes(b.to_vec());
    let raw = || Value::Text(RAW.token().to_owned());
    let cases = [
        (Value::Bytes(vec![1]), "malformed"),
        (Value::Array(vec![uint(0), uint(1)]), "malformed"),
        (
            Value::Array(vec![uint(0), uint(1), bytes(b"x"), bytes(b"y")]),
            "malformed",
        ),
        (
            Value::Array(vec![Value::Int(-1), uint(1), bytes(b"x")]),
            "malformed",
        ),
        (
            Value::Array(vec![uint(0), Value::Int(-1), bytes(b"x")]),
            "malformed",
        ),
        (
            Value::Array(vec![uint(0), uint(1), Value::Text("x".to_owned())]),
            "malformed",
        ),
        (
            Value::Array(vec![uint(1), raw(), bytes(&[0; 31])]),
            "malformed",
        ),
        (
            Value::Array(vec![uint(2), bytes(b"raw"), bytes(&[0; 32])]),
            "malformed",
        ),
        (
            Value::Array(vec![
                uint(1),
                Value::Text("md5".to_owned()),
                bytes(&[0; 32]),
            ]),
            "unsupported",
        ),
        (
            Value::Array(vec![uint(3), uint(1), bytes(b"x")]),
            "unsupported",
        ),
    ];
    for (value, class) in cases {
        assert_eq!(term_error(&value).class(), class, "{value:?}");
    }
    assert_eq!(
        term_error(&Value::Array(vec![uint(9), uint(1), bytes(b"x")])),
        TermError::UnknownKind(9)
    );
    // A non-shortest integer is a second encoding of the same term.
    let mut long_tag = unhex(TERM_TYPED);
    long_tag.splice(2..3, [0x18, 0x02]);
    assert_eq!(
        Term::from_bytes(&long_tag).unwrap_err(),
        TermError::Cbor(CborError::NonShortestForm)
    );
    let mut trailing = unhex(TERM_TYPED);
    trailing.push(0);
    assert_eq!(
        Term::from_bytes(&trailing).unwrap_err(),
        TermError::Cbor(CborError::TrailingBytes)
    );
    // Size limits hold on both sides.
    let huge = vec![0; MAX_TERM_BYTES + 1];
    assert!(matches!(
        Term::typed(1, huge.clone()).to_bytes(),
        Err(TermError::TooLarge { .. })
    ));
    assert_eq!(
        term_error(&Value::Array(vec![uint(0), uint(1), Value::Bytes(huge)])).class(),
        "resource_limit"
    );
    assert!(Term::typed(1, vec![0; MAX_TERM_BYTES]).to_bytes().is_ok());
}

fn payload_map(entries: Vec<(&str, Value)>) -> Vec<u8> {
    let mut entries: Vec<(&str, Value)> = entries;
    entries.sort_by(|(a, _), (b, _)| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    encode(&Value::Map(
        entries
            .into_iter()
            .map(|(k, v)| (Value::Text(k.to_owned()), v))
            .collect(),
    ))
}

fn base_fields() -> Vec<(&'static str, Value)> {
    vec![
        ("v", Value::UInt(1)),
        ("graph", Value::Bytes(vec![0x11; 32])),
        ("triples", Value::Array(vec![])),
    ]
}

#[test]
fn ambiguous_payloads_reject() {
    // The minimal payload decodes.
    let minimal = payload_map(base_fields());
    assert_eq!(
        GraphRecord::from_payload(&minimal).unwrap(),
        GraphRecord::new(graph_id(), vec![])
    );
    let with = |name: &'static str, value: Value| {
        let mut fields = base_fields();
        fields.retain(|(key, _)| *key != name);
        fields.push((name, value));
        GraphRecord::from_payload(&payload_map(fields)).unwrap_err()
    };
    let without = |name: &'static str| {
        let mut fields = base_fields();
        fields.retain(|(key, _)| *key != name);
        GraphRecord::from_payload(&payload_map(fields)).unwrap_err()
    };
    assert_eq!(with("v", Value::UInt(2)), RecordError::UnsupportedSchema(2));
    assert_eq!(with("v", Value::UInt(2)).class(), "unsupported");
    assert_eq!(with("v", Value::Text("1".to_owned())).class(), "malformed");
    assert_eq!(
        with("graph", Value::Bytes(vec![0; 31])).class(),
        "malformed"
    );
    assert_eq!(with("triples", Value::Bytes(vec![])).class(), "malformed");
    assert_eq!(
        with("count", Value::UInt(0)),
        RecordError::Cbor(CborError::UnknownCriticalField {
            field: "count".to_owned()
        })
    );
    for name in ["v", "graph", "triples"] {
        assert_eq!(without(name).class(), "malformed", "missing {name}");
    }
    // A source must be an envelope reference, not content or bytes.
    let content = Term::Object(ObjectRef::Content(ContentId::new(RAW, [0x33; 32])));
    assert_eq!(
        with("source_record", content.to_value().unwrap()),
        RecordError::Field {
            field: "source_record",
            problem: "must be an envelope reference"
        }
    );
    assert_eq!(
        with("source_record", typed(1, &[0x33; 32]).to_value().unwrap()).class(),
        "malformed"
    );
    // A triple of two terms is not a triple.
    let pair = Value::Array(vec![
        typed(1, b"s").to_value().unwrap(),
        typed(2, b"p").to_value().unwrap(),
    ]);
    assert_eq!(
        with("triples", Value::Array(vec![pair])).class(),
        "malformed"
    );
    // Keys out of canonical order are a second encoding.
    let mut swapped = Vec::new();
    head(5, 3, &mut swapped);
    for (k, v) in [
        ("graph", Value::Bytes(vec![0x11; 32])),
        ("v", Value::UInt(1)),
        ("triples", Value::Array(vec![])),
    ] {
        swapped.extend(encode(&Value::Text(k.to_owned())));
        swapped.extend(encode(&v));
    }
    assert_eq!(
        GraphRecord::from_payload(&swapped).unwrap_err(),
        RecordError::Cbor(CborError::KeysOutOfOrder)
    );
    // Too many triples, both writing and reading.
    let triple = Triple::new(typed(1, b"s"), typed(2, b"p"), typed(3, b"o"));
    let crowded = GraphRecord::new(graph_id(), vec![triple.clone(); MAX_TRIPLES + 1]);
    assert_eq!(
        crowded.payload_bytes().unwrap_err(),
        RecordError::TooManyTriples(MAX_TRIPLES + 1)
    );
    let triple_value = triple.to_value().unwrap();
    assert_eq!(
        with("triples", Value::Array(vec![triple_value; MAX_TRIPLES + 1])).class(),
        "resource_limit"
    );
}

fn signed_body(payload: &[u8], domain: &str, graph: Option<LogicalGraphId>) -> Body {
    Body {
        domain: domain.to_owned(),
        context: "namespace/main".to_owned(),
        subject: None,
        graph,
        predecessor: None,
        signer_id: "node-a".to_owned(),
        key: key().verifying_key().to_bytes(),
        suite: SUITE_ED25519.to_owned(),
        payload_id: PayloadId::of(RAW, payload).unwrap(),
        payload_length: payload.len() as u64,
    }
}

#[test]
fn envelopes_that_disagree_with_their_record_reject() {
    let payload = known_record().payload_bytes().unwrap();
    let sign = |body| Envelope::sign(body, payload.clone(), &key()).unwrap();
    let dispatch = ProfileDispatch::default();
    let other_graph = sign(signed_body(
        &payload,
        GRAPH_RECORD_DOMAIN,
        Some(LogicalGraphId::new([0x12; 32])),
    ));
    assert_eq!(
        open(other_graph, &dispatch).unwrap_err(),
        RecordError::GraphMismatch
    );
    let no_graph = sign(signed_body(&payload, GRAPH_RECORD_DOMAIN, None));
    assert_eq!(
        open(no_graph, &dispatch).unwrap_err(),
        RecordError::GraphMismatch
    );
    let elsewhere = sign(signed_body(&payload, "id/sidecar", Some(graph_id())));
    assert_eq!(
        open(elsewhere, &dispatch).unwrap_err(),
        RecordError::WrongDomain("id/sidecar".to_owned())
    );
    let mut forged = known_record()
        .sign("namespace/main", "node-a", &key())
        .unwrap()
        .envelope;
    forged.signature[0] ^= 1;
    assert_eq!(
        open(forged, &dispatch).unwrap_err().class(),
        "bad_signature"
    );
    let mut swapped = known_record()
        .sign("namespace/main", "node-a", &key())
        .unwrap()
        .envelope;
    swapped.payload = GraphRecord::new(graph_id(), vec![])
        .payload_bytes()
        .unwrap();
    assert!(open(swapped, &dispatch).is_err(), "payload swap accepted");
    // The key that signs must be the key the body commits.
    let stranger = SigningKey::from_bytes(&[8; 32]);
    let mut body = signed_body(&payload, GRAPH_RECORD_DOMAIN, Some(graph_id()));
    body.key = VerifyingKey::from(&stranger).to_bytes();
    assert!(Envelope::sign(body, payload.clone(), &key()).is_err());
}
