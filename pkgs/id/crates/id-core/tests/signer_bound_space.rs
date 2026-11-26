//! Signer-bound space addresses and local petnames (slice
//! A-GRAPH-9131-01, issue #10224; design part A §10, statements S5.54,
//! S5.55, S5.62).
//!
//! Oracle: **reference**. This file carries its own canonical address
//! decoder (written from the ID-shape and term wire forms, not from the
//! code under test) and checks signatures with `ed25519-dalek` directly.
//! The module must agree with it: the exact canonical binding verifies,
//! a wrong signer, slot position or profile version rejects, a new key
//! cannot take over an existing address, a redirect to a new address
//! needs the old key's signature, and petnames change neither an address
//! nor any verification. The existing petname store is used as is, as a
//! regression check of its locality contract.
//!
//! This suite includes the graph modules and their dependencies by path.

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

#[path = "../src/graph/term.rs"]
#[allow(dead_code)]
mod term;

#[path = "../src/graph/space_binding.rs"]
mod space_binding;

use canonical_cbor::{Value, encode_canonical};
use ed25519_dalek::{Signature, SigningKey, VerifyingKey};
use hash_profile::ProfileDispatch;
use id_core::petname::Petnames;
use id_shape::{ArrayId, ContentId, HashProfile, Id};
use proptest::prelude::*;
use signed_envelope::{Body, Envelope, PayloadId, SUITE_ED25519, signature_preimage};
use space_binding::{
    SPACE_BINDING_DOMAIN, SPACE_BINDING_PROFILE, SPACE_REDIRECT_DOMAIN, SpaceAddress, SpaceError,
    sign_binding, sign_redirect, verify_binding, verify_redirect,
};
use term::Term;

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn address(seed: u8) -> SpaceAddress {
    SpaceAddress::for_key(key(seed).verifying_key().to_bytes()).unwrap()
}

fn dispatch() -> ProfileDispatch {
    ProfileDispatch::default()
}

/// Signs any payload under any domain and context, so tests can build
/// records the module would never make.
fn record(domain: &str, context: &str, payload: Vec<u8>, signer: &SigningKey) -> Envelope {
    let body = Body {
        domain: domain.to_owned(),
        context: context.to_owned(),
        subject: None,
        graph: None,
        predecessor: None,
        signer_id: "tester".to_owned(),
        key: signer.verifying_key().to_bytes(),
        suite: SUITE_ED25519.to_owned(),
        payload_id: PayloadId::of(HashProfile::RawBlake3_256, &payload).unwrap(),
        payload_length: payload.len() as u64,
    };
    Envelope::sign(body, payload, signer).unwrap()
}

// ---------------------------------------------------------------------
// Independent reference: address encoder/decoder and signature check.

const TERM_PREFIX: [u8; 6] = [0x83, 0x00, 0x18, 0xED, 0x58, 0x20];

/// `[0x02 array, 0x01 version][2 slots LE] 0x00 null 0x01 [38 LE][term]`.
fn ref_address_bytes(public: &[u8; 32]) -> Vec<u8> {
    let mut out = vec![0x02, 0x01, 2, 0, 0, 0, 0x00, 0x01, 38, 0, 0, 0];
    out.extend_from_slice(&TERM_PREFIX);
    out.extend_from_slice(public);
    out
}

/// The key in a `[null, typed key]` slot list, or `None`.
fn ref_key_from_slots(slots: &[Option<Vec<u8>>]) -> Option<[u8; 32]> {
    let [None, Some(slot)] = slots else {
        return None;
    };
    if slot.len() != 38 || slot[..6] != TERM_PREFIX {
        return None;
    }
    let key: [u8; 32] = slot[6..].try_into().ok()?;
    VerifyingKey::from_bytes(&key).ok()?;
    Some(key)
}

fn ref_key_from_bytes(bytes: &[u8]) -> Option<[u8; 32]> {
    if bytes.len() != 50 || bytes[..12] != [0x02, 0x01, 2, 0, 0, 0, 0x00, 0x01, 38, 0, 0, 0] {
        return None;
    }
    ref_key_from_slots(&[None, Some(bytes[12..].to_vec())])
}

/// Independent check of a binding record: right domain and profile, a
/// payload that decodes to an address, a signature by that address's key.
fn ref_binding_ok(envelope: &Envelope) -> bool {
    let body = &envelope.body;
    let Some(bound) = ref_key_from_bytes(&envelope.payload) else {
        return false;
    };
    let Ok(verifier) = VerifyingKey::from_bytes(&body.key) else {
        return false;
    };
    body.domain == SPACE_BINDING_DOMAIN
        && body.context == SPACE_BINDING_PROFILE
        && body.key == bound
        && body.payload_id == PayloadId::of(HashProfile::RawBlake3_256, &envelope.payload).unwrap()
        && verifier
            .verify_strict(
                &signature_preimage(body).unwrap(),
                &Signature::from_bytes(&envelope.signature),
            )
            .is_ok()
}

// ---------------------------------------------------------------------
// Fixtures.

#[test]
fn address_is_the_exact_canonical_null_key_array() {
    let public = key(7).verifying_key().to_bytes();
    let space = address(7);
    assert_eq!(space.to_bytes(), ref_address_bytes(&public));
    assert_eq!(space.key(), &public);
    assert_eq!(space.array_id().slots()[0], None);
    assert_eq!(SpaceAddress::from_bytes(&space.to_bytes()).unwrap(), space);
    // The text form follows the ID-shape array grammar.
    let text = Id::Array(space.array_id().clone()).to_text();
    assert!(text.starts_with("array/~/b."), "{text}");
}

#[test]
fn a_binding_verifies_offline_against_the_reference() {
    let envelope = sign_binding(&key(7), "alice").unwrap();
    assert!(ref_binding_ok(&envelope));
    assert_eq!(verify_binding(&envelope, &dispatch()).unwrap(), address(7));
    // Verification needs only the record's bytes.
    let bytes = envelope.canonical_bytes().unwrap();
    let decoded = Envelope::decode(&bytes).unwrap();
    assert_eq!(verify_binding(&decoded, &dispatch()).unwrap(), address(7));
}

#[test]
fn wrong_signer_rejects_and_cannot_take_over_an_address() {
    // Key 8 signs a binding for key 7's coordinates.
    let forged = record(
        SPACE_BINDING_DOMAIN,
        SPACE_BINDING_PROFILE,
        address(7).to_bytes(),
        &key(8),
    );
    assert!(!ref_binding_ok(&forged));
    assert_eq!(
        verify_binding(&forged, &dispatch()),
        Err(SpaceError::WrongSigner)
    );
    // Key 8's own binding names a different address; 7's is unchanged.
    let own = verify_binding(&sign_binding(&key(8), "bob").unwrap(), &dispatch()).unwrap();
    assert_ne!(own, address(7));
    assert_eq!(address(7).to_bytes(), ref_address_bytes(address(7).key()));
}

#[test]
fn wrong_position_key_type_and_key_bytes_reject() {
    let public = key(7).verifying_key().to_bytes();
    let typed = Term::typed(237, public.to_vec()).to_bytes().unwrap();
    let as_ids = |slots: Vec<Option<Vec<u8>>>| Id::Array(ArrayId::new(slots).unwrap()).to_bytes();
    let class = |bytes: Vec<u8>| SpaceAddress::from_bytes(&bytes).unwrap_err();

    assert_eq!(
        class(as_ids(vec![Some(typed.clone()), None])),
        SpaceError::WrongPosition
    );
    assert_eq!(
        class(as_ids(vec![None, Some(typed.clone()), None])),
        SpaceError::WrongPosition
    );
    assert_eq!(
        class(as_ids(vec![Some(vec![]), Some(typed.clone())])),
        SpaceError::WrongPosition
    );
    assert_eq!(class(as_ids(vec![None, None])), SpaceError::WrongPosition);
    assert_eq!(
        class(as_ids(vec![
            None,
            Some(Term::typed(238, public.to_vec()).to_bytes().unwrap())
        ])),
        SpaceError::UnsupportedKeyType(238)
    );
    assert_eq!(
        class(as_ids(vec![
            None,
            Some(Term::typed(237, public[..31].to_vec()).to_bytes().unwrap())
        ])),
        SpaceError::InvalidKey
    );
    assert_eq!(
        class(as_ids(vec![None, Some(public.to_vec())])).class(),
        "malformed"
    );
    let content = Id::Content(ContentId::new(HashProfile::RawBlake3_256, public)).to_bytes();
    assert_eq!(class(content).class(), "malformed");
    let mut trailing = address(7).to_bytes();
    trailing.push(0);
    assert_eq!(class(trailing).class(), "malformed");

    // The same rejections hold inside a signed binding.
    let wrong_slot = record(
        SPACE_BINDING_DOMAIN,
        SPACE_BINDING_PROFILE,
        as_ids(vec![Some(typed), None]),
        &key(7),
    );
    assert!(!ref_binding_ok(&wrong_slot));
    assert_eq!(
        verify_binding(&wrong_slot, &dispatch()),
        Err(SpaceError::WrongPosition)
    );
}

#[test]
fn thirty_two_bytes_that_are_not_a_curve_point_reject() {
    let off_curve = (0..=255_u8)
        .map(|n| {
            let mut bytes = [0_u8; 32];
            bytes[0] = n;
            bytes[31] = 0x7f;
            bytes
        })
        .find(|bytes| VerifyingKey::from_bytes(bytes).is_err())
        .expect("some y has no curve point");
    assert_eq!(
        SpaceAddress::for_key(off_curve),
        Err(SpaceError::InvalidKey)
    );
    let slots = vec![
        None,
        Some(Term::typed(237, off_curve.to_vec()).to_bytes().unwrap()),
    ];
    assert_eq!(ref_key_from_slots(&slots), None);
    assert_eq!(
        SpaceAddress::from_array_id(&ArrayId::new(slots).unwrap()),
        Err(SpaceError::InvalidKey)
    );
}

#[test]
fn wrong_profile_version_domain_or_signature_rejects() {
    let payload = address(7).to_bytes();
    let v2 = record(
        SPACE_BINDING_DOMAIN,
        "space-binding/v2",
        payload.clone(),
        &key(7),
    );
    assert!(!ref_binding_ok(&v2));
    assert_eq!(
        verify_binding(&v2, &dispatch()).unwrap_err().class(),
        "unsupported"
    );

    let other = record("id/graph-record", SPACE_BINDING_PROFILE, payload, &key(7));
    assert_eq!(
        verify_binding(&other, &dispatch()).unwrap_err().class(),
        "wrong_domain"
    );

    let mut tampered = sign_binding(&key(7), "alice").unwrap();
    tampered.signature[0] ^= 1;
    assert!(!ref_binding_ok(&tampered));
    assert_eq!(
        verify_binding(&tampered, &dispatch()).unwrap_err().class(),
        "bad_signature"
    );

    // A redirect is not a binding, and the reverse.
    let redirect = sign_redirect(&key(7), &address(8), "alice").unwrap();
    assert_eq!(
        verify_binding(&redirect, &dispatch()).unwrap_err().class(),
        "wrong_domain"
    );
    let binding = sign_binding(&key(7), "alice").unwrap();
    assert_eq!(
        verify_redirect(&binding, &dispatch()).unwrap_err().class(),
        "wrong_domain"
    );
}

#[test]
fn a_redirect_to_a_new_address_needs_the_old_key() {
    let redirect = verify_redirect(
        &sign_redirect(&key(7), &address(8), "alice").unwrap(),
        &dispatch(),
    )
    .unwrap();
    assert_eq!(redirect.from, address(7));
    assert_eq!(redirect.to, address(8));
    // The old address is still exactly the old key's.
    assert_eq!(
        address(7).to_bytes(),
        ref_address_bytes(key(7).verifying_key().as_bytes())
    );

    let payload = |version: u64, from: &SpaceAddress, to: &SpaceAddress| {
        encode_canonical(&Value::Array(vec![
            Value::UInt(version),
            Value::Bytes(from.to_bytes()),
            Value::Bytes(to.to_bytes()),
        ]))
        .unwrap()
    };
    // Only the new key signed it: no old-key proof.
    let by_new = record(
        SPACE_REDIRECT_DOMAIN,
        SPACE_BINDING_PROFILE,
        payload(1, &address(7), &address(8)),
        &key(8),
    );
    assert_eq!(
        verify_redirect(&by_new, &dispatch()),
        Err(SpaceError::WrongSigner)
    );
    // A third key cannot redirect someone else's space.
    let by_third = record(
        SPACE_REDIRECT_DOMAIN,
        SPACE_BINDING_PROFILE,
        payload(1, &address(7), &address(9)),
        &key(9),
    );
    assert_eq!(
        verify_redirect(&by_third, &dispatch()),
        Err(SpaceError::WrongSigner)
    );
    // Self redirects and other payload versions reject.
    assert_eq!(
        sign_redirect(&key(7), &address(7), "alice"),
        Err(SpaceError::SelfRedirect)
    );
    let to_self = record(
        SPACE_REDIRECT_DOMAIN,
        SPACE_BINDING_PROFILE,
        payload(1, &address(7), &address(7)),
        &key(7),
    );
    assert_eq!(
        verify_redirect(&to_self, &dispatch()),
        Err(SpaceError::SelfRedirect)
    );
    let v2 = record(
        SPACE_REDIRECT_DOMAIN,
        SPACE_BINDING_PROFILE,
        payload(2, &address(7), &address(8)),
        &key(7),
    );
    assert_eq!(
        verify_redirect(&v2, &dispatch()).unwrap_err().class(),
        "unsupported"
    );
    let short = record(
        SPACE_REDIRECT_DOMAIN,
        SPACE_BINDING_PROFILE,
        encode_canonical(&Value::Array(vec![Value::UInt(1)])).unwrap(),
        &key(7),
    );
    assert_eq!(
        verify_redirect(&short, &dispatch()).unwrap_err().class(),
        "malformed"
    );
}

#[test]
fn petnames_stay_local_and_change_no_address_or_verification() {
    let binding = sign_binding(&key(7), "alice").unwrap();
    let account = Id::Array(address(7).array_id().clone()).to_text();
    let before = (address(7).to_bytes(), verify_binding(&binding, &dispatch()));

    let mut names = Petnames::new();
    names.set("owner-a", &account, "Work space", []).unwrap();
    names.set("owner-b", &account, "Alice", []).unwrap();
    assert_eq!(names.get("owner-a", &account), Some("Work space"));
    assert_eq!(names.get("owner-b", &account), Some("Alice"));
    // Not global: another owner sees nothing.
    assert_eq!(names.get("owner-c", &account), None);
    assert_eq!(
        (address(7).to_bytes(), verify_binding(&binding, &dispatch())),
        before
    );

    names
        .rename("owner-a", &account, "Old work space", [])
        .unwrap();
    names.remove("owner-b", &account).unwrap();
    assert_eq!(
        (address(7).to_bytes(), verify_binding(&binding, &dispatch())),
        before
    );
}

// ---------------------------------------------------------------------
// Properties against the reference.

fn slot() -> impl Strategy<Value = Option<Vec<u8>>> {
    prop_oneof![
        Just(None),
        prop::collection::vec(any::<u8>(), 0..40).prop_map(Some),
        any::<[u8; 32]>().prop_map(|k| Some(Term::typed(237, k.to_vec()).to_bytes().unwrap())),
        any::<u8>().prop_map(|s| Some(
            Term::typed(237, key(s).verifying_key().to_bytes().to_vec())
                .to_bytes()
                .unwrap()
        )),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn decoding_agrees_with_the_reference(slots in prop::collection::vec(slot(), 0..4)) {
        let array = ArrayId::new(slots.clone()).unwrap();
        let got = SpaceAddress::from_array_id(&array).ok().map(|a| *a.key());
        prop_assert_eq!(got, ref_key_from_slots(&slots));
        let bytes = Id::Array(array).to_bytes();
        let got = SpaceAddress::from_bytes(&bytes).ok().map(|a| *a.key());
        prop_assert_eq!(got, ref_key_from_bytes(&bytes));
    }

    #[test]
    fn bindings_round_trip_and_cross_signing_never_verifies(a in any::<u8>(), b in any::<u8>()) {
        let own = sign_binding(&key(a), "a").unwrap();
        prop_assert!(ref_binding_ok(&own));
        prop_assert_eq!(verify_binding(&own, &dispatch()).unwrap(), address(a));
        let claimed = record(SPACE_BINDING_DOMAIN, SPACE_BINDING_PROFILE, address(a).to_bytes(), &key(b));
        let same = key(a).verifying_key() == key(b).verifying_key();
        prop_assert_eq!(ref_binding_ok(&claimed), same);
        prop_assert_eq!(verify_binding(&claimed, &dispatch()).is_ok(), same);
    }
}
