//! Mutual cross-signed key links (slice A-GRAPH-9132-02, issue #10240;
//! design part A §11, statements S5.58, S5.59, S5.60, S5.63).
//!
//! Oracle: **invariant**. This file carries its own signature check
//! (`ed25519-dalek` directly over the signed-body preimage) and its own
//! exact field-equality checker for challenge bytes. The module must
//! agree with them: only both valid signatures over identical challenge
//! bytes become mutual; one-sided, unrelated, mismatched or revoked
//! claims stay pending or end; and a mutual link changes no
//! authorization state.
//!
//! This suite includes the graph modules and their dependencies by path.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::type_complexity
)]

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
#[allow(dead_code)]
mod space_binding;

#[path = "../src/graph/delegation.rs"]
#[allow(dead_code)]
mod delegation;

#[path = "../src/graph/cross_signing.rs"]
mod cross_signing;

use std::collections::{HashMap, HashSet};

use canonical_cbor::{Value, decode_canonical, encode_canonical};
use cross_signing::{
    ChallengeId, KeyLinks, LINK_DOMAIN, LINK_REVOCATION_DOMAIN, LinkChallenge, LinkError,
    LinkStatus, sign_link_revocation,
};
use delegation::{DelegationError, DelegationSet, Permission, Profile, Request, Scope};
use ed25519_dalek::{Signature, SigningKey, VerifyingKey};
use hash_profile::ProfileDispatch;
use id_shape::HashProfile;
use proptest::prelude::*;
use signed_envelope::{Body, Envelope, PayloadId, SUITE_ED25519, signature_preimage};
use space_binding::SpaceAddress;
use term::Term;

const SIGNER: &str = "tester";
const PROFILE: Profile = Profile::INITIAL;

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn public(seed: u8) -> [u8; 32] {
    key(seed).verifying_key().to_bytes()
}

fn dispatch() -> ProfileDispatch {
    ProfileDispatch::default()
}

fn scope(prefixes: &[&[u8]]) -> Scope {
    Scope::new(
        prefixes
            .iter()
            .map(|p| p.iter().map(|b| Term::typed(1, vec![*b])).collect())
            .collect(),
    )
    .unwrap()
}

fn challenge(a: u8, b: u8) -> LinkChallenge {
    LinkChallenge::new(
        public(a),
        public(b),
        "same-owner",
        scope(&[&[]]),
        vec![7],
        1_000,
    )
    .unwrap()
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
        signer_id: SIGNER.to_owned(),
        key: signer.verifying_key().to_bytes(),
        suite: SUITE_ED25519.to_owned(),
        payload_id: PayloadId::of(HashProfile::RawBlake3_256, &payload).unwrap(),
        payload_length: payload.len() as u64,
    };
    Envelope::sign(body, payload, signer).unwrap()
}

// ---------------------------------------------------------------------
// Independent reference: signature check and challenge field checker.

/// The challenge fields, read straight from the canonical array.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RefChallenge {
    first: [u8; 32],
    second: [u8; 32],
    purpose: String,
    scope: Value,
    nonce: Vec<u8>,
    expires: u64,
}

fn ref_challenge(bytes: &[u8]) -> Option<RefChallenge> {
    let Value::Array(items) = decode_canonical(bytes).ok()? else {
        return None;
    };
    let [
        Value::UInt(1),
        Value::Text(label),
        Value::Bytes(first),
        Value::Bytes(second),
        Value::Text(purpose),
        scope @ Value::Array(_),
        Value::Bytes(nonce),
        Value::UInt(expires),
    ] = items.as_slice()
    else {
        return None;
    };
    let first: [u8; 32] = first.as_slice().try_into().ok()?;
    let second: [u8; 32] = second.as_slice().try_into().ok()?;
    let ok = label == "id/key-link"
        && first < second
        && (1..=64).contains(&purpose.len())
        && (1..=64).contains(&nonce.len())
        && encode_canonical(&Value::Array(items.clone())).ok()? == bytes;
    ok.then(|| RefChallenge {
        first,
        second,
        purpose: purpose.clone(),
        scope: scope.clone(),
        nonce: nonce.clone(),
        expires: *expires,
    })
}

fn ref_signature_ok(envelope: &Envelope, domain: &str) -> bool {
    let body = &envelope.body;
    let Ok(verifier) = VerifyingKey::from_bytes(&body.key) else {
        return false;
    };
    body.domain == domain
        && body.context == "delegation/v1"
        && body.payload_id == PayloadId::of(HashProfile::RawBlake3_256, &envelope.payload).unwrap()
        && body.payload_length == envelope.payload.len() as u64
        && verifier
            .verify_strict(
                &signature_preimage(body).unwrap(),
                &Signature::from_bytes(&envelope.signature),
            )
            .is_ok()
}

/// A statement the reference accepts, as its challenge and signer.
fn ref_statement(envelope: &Envelope) -> Option<(RefChallenge, [u8; 32])> {
    if !ref_signature_ok(envelope, LINK_DOMAIN) {
        return None;
    }
    let fields = ref_challenge(&envelope.payload)?;
    let signer = envelope.body.key;
    (signer == fields.first || signer == fields.second).then_some((fields, signer))
}

// ---------------------------------------------------------------------
// Targeted cases.

#[test]
fn both_keys_over_identical_bytes_become_mutual() {
    let c = challenge(1, 2);
    let mut links = KeyLinks::new();
    let id = links
        .admit_statement(&c.sign(SIGNER, &key(1)).unwrap(), &dispatch())
        .unwrap();
    assert_eq!(id, c.id().unwrap());
    assert_eq!(
        links.status(&id, 900, &PROFILE),
        LinkStatus::Pending {
            signed_by: public(1)
        }
    );
    // The same statement again changes nothing.
    links
        .admit_statement(&c.sign(SIGNER, &key(1)).unwrap(), &dispatch())
        .unwrap();
    assert!(matches!(
        links.status(&id, 900, &PROFILE),
        LinkStatus::Pending { .. }
    ));
    assert_eq!(
        links.admit_statement(&c.sign(SIGNER, &key(2)).unwrap(), &dispatch()),
        Ok(id)
    );
    assert_eq!(links.status(&id, 900, &PROFILE), LinkStatus::Mutual);
    assert_eq!(links.challenge(&id), Some(&c));
    // Key order does not matter to the challenge.
    assert_eq!(challenge(2, 1), c);
}

#[test]
fn any_differing_field_is_a_different_challenge() {
    let base = challenge(1, 2);
    let variants = [
        LinkChallenge::new(
            public(1),
            public(3),
            "same-owner",
            scope(&[&[]]),
            vec![7],
            1_000,
        ),
        LinkChallenge::new(
            public(1),
            public(2),
            "backup-key",
            scope(&[&[]]),
            vec![7],
            1_000,
        ),
        LinkChallenge::new(
            public(1),
            public(2),
            "same-owner",
            scope(&[&[1]]),
            vec![7],
            1_000,
        ),
        LinkChallenge::new(
            public(1),
            public(2),
            "same-owner",
            scope(&[&[]]),
            vec![8],
            1_000,
        ),
        LinkChallenge::new(
            public(1),
            public(2),
            "same-owner",
            scope(&[&[]]),
            vec![7],
            1_001,
        ),
    ];
    for variant in variants {
        let variant = variant.unwrap();
        let mut links = KeyLinks::new();
        let a = links
            .admit_statement(&base.sign(SIGNER, &key(1)).unwrap(), &dispatch())
            .unwrap();
        let second_signer = if variant.keys().1 == &public(3) { 3 } else { 2 };
        let b = links
            .admit_statement(
                &variant.sign(SIGNER, &key(second_signer)).unwrap(),
                &dispatch(),
            )
            .unwrap();
        assert_ne!(a, b);
        assert!(matches!(
            links.status(&a, 900, &PROFILE),
            LinkStatus::Pending { .. }
        ));
        assert!(matches!(
            links.status(&b, 900, &PROFILE),
            LinkStatus::Pending { .. }
        ));
    }
}

#[test]
fn outsiders_tampering_and_other_domains_are_refused() {
    let c = challenge(1, 2);
    assert_eq!(c.sign(SIGNER, &key(3)), Err(LinkError::WrongSigner));
    let bytes = c.challenge_bytes().unwrap();
    let mut links = KeyLinks::new();
    let outsider = record(LINK_DOMAIN, "delegation/v1", bytes.clone(), &key(3));
    assert_eq!(
        links.admit_statement(&outsider, &dispatch()),
        Err(LinkError::WrongSigner)
    );
    let other_domain = record("id/other", "delegation/v1", bytes.clone(), &key(1));
    assert_eq!(
        links
            .admit_statement(&other_domain, &dispatch())
            .map_err(|e| e.class()),
        Err("wrong_domain")
    );
    let other_profile = record(LINK_DOMAIN, "delegation/v2", bytes.clone(), &key(1));
    assert_eq!(
        links
            .admit_statement(&other_profile, &dispatch())
            .map_err(|e| e.class()),
        Err("wrong_domain")
    );
    let revocation_as_statement = record(LINK_REVOCATION_DOMAIN, "delegation/v1", bytes, &key(1));
    assert_eq!(
        links
            .admit_statement(&revocation_as_statement, &dispatch())
            .map_err(|e| e.class()),
        Err("wrong_domain")
    );
    let mut tampered = c.sign(SIGNER, &key(1)).unwrap();
    tampered.payload[5] ^= 1;
    assert_eq!(
        links
            .admit_statement(&tampered, &dispatch())
            .map_err(|e| e.class()),
        Err("bad_signature")
    );
    assert_eq!(
        links.status(&c.id().unwrap(), 900, &PROFILE),
        LinkStatus::Unknown
    );
}

#[test]
fn challenge_bytes_are_canonical_and_bounded() {
    assert_eq!(
        LinkChallenge::new(public(1), public(1), "x", scope(&[&[]]), vec![1], 1),
        Err(LinkError::SameKey)
    );
    for (purpose, nonce) in [
        ("", vec![1]),
        (&*"p".repeat(65), vec![1]),
        ("x", vec![]),
        ("x", vec![0; 65]),
    ] {
        assert_eq!(
            LinkChallenge::new(public(1), public(2), purpose, scope(&[&[]]), nonce, 1)
                .map_err(|e| e.class()),
            Err("malformed")
        );
    }
    let longest = LinkChallenge::new(
        public(1),
        public(2),
        &"p".repeat(64),
        scope(&[&[]]),
        vec![0; 64],
        1,
    )
    .unwrap();
    let bytes = longest.challenge_bytes().unwrap();
    assert_eq!(LinkChallenge::from_bytes(&bytes), Ok(longest));

    let c = challenge(1, 2);
    let Value::Array(items) = decode_canonical(&c.challenge_bytes().unwrap()).unwrap() else {
        panic!("challenge is an array");
    };
    let decode = |items: Vec<Value>| {
        LinkChallenge::from_bytes(&encode_canonical(&Value::Array(items)).unwrap())
            .map_err(|e| e.class())
    };
    assert!(decode(items.clone()).is_ok());
    let mut swapped = items.clone();
    swapped.swap(2, 3);
    assert_eq!(decode(swapped), Err("malformed"));
    let mut same = items.clone();
    same[3] = same[2].clone();
    assert_eq!(decode(same), Err("malformed"));
    let mut version = items.clone();
    version[0] = Value::UInt(2);
    assert_eq!(decode(version), Err("malformed"));
    let mut label = items.clone();
    label[1] = Value::Text("id/other".to_owned());
    assert_eq!(decode(label), Err("malformed"));
    let mut short_key = items.clone();
    short_key[2] = Value::Bytes(vec![0; 31]);
    assert_eq!(decode(short_key), Err("malformed"));
    let mut bad_scope = items;
    bad_scope[5] = Value::UInt(0);
    assert_eq!(decode(bad_scope), Err("malformed"));
    let mut unsorted_scope = c;
    unsorted_scope.scope = scope(&[&[1], &[0]]);
    let Value::Array(mut items) =
        decode_canonical(&unsorted_scope.challenge_bytes().unwrap()).unwrap()
    else {
        panic!("challenge is an array");
    };
    let Value::Array(prefixes) = &mut items[5] else {
        panic!("scope is an array");
    };
    prefixes.reverse();
    assert_eq!(decode(items.clone()), Err("malformed"));
    items.pop();
    assert_eq!(decode(items), Err("malformed"));
    assert_eq!(
        LinkChallenge::from_bytes(&[0xff]).map_err(|e| e.class()),
        Err("malformed")
    );
}

#[test]
fn expiry_and_revocation_end_a_link() {
    let c = challenge(1, 2);
    let mut links = KeyLinks::new();
    let id = links
        .admit_statement(&c.sign(SIGNER, &key(1)).unwrap(), &dispatch())
        .unwrap();
    links
        .admit_statement(&c.sign(SIGNER, &key(2)).unwrap(), &dispatch())
        .unwrap();
    // Expiry honours the skew allowance exactly.
    assert_eq!(links.status(&id, 1_300, &PROFILE), LinkStatus::Mutual);
    assert_eq!(links.status(&id, 1_301, &PROFILE), LinkStatus::Expired);

    let by_outsider = sign_link_revocation(id, SIGNER, &key(3)).unwrap();
    assert_eq!(
        links.admit_revocation(&by_outsider, &dispatch()),
        Err(LinkError::WrongSigner)
    );
    let unknown = sign_link_revocation(challenge(1, 3).id().unwrap(), SIGNER, &key(1)).unwrap();
    assert_eq!(
        links.admit_revocation(&unknown, &dispatch()),
        Err(LinkError::UnknownChallenge)
    );
    let as_statement = record(
        LINK_DOMAIN,
        "delegation/v1",
        by_outsider.payload.clone(),
        &key(2),
    );
    assert_eq!(
        links
            .admit_revocation(&as_statement, &dispatch())
            .map_err(|e| e.class()),
        Err("wrong_domain")
    );
    for payload in [
        encode_canonical(&Value::UInt(1)).unwrap(),
        encode_canonical(&Value::Array(vec![Value::UInt(1)])).unwrap(),
        encode_canonical(&Value::Array(vec![
            Value::UInt(1),
            Value::Bytes(vec![1; 4]),
        ]))
        .unwrap(),
        encode_canonical(&Value::Array(vec![Value::UInt(2), {
            let Value::Array(items) = decode_canonical(&by_outsider.payload).unwrap() else {
                panic!("revocation is an array");
            };
            items[1].clone()
        }]))
        .unwrap(),
    ] {
        let bad = record(LINK_REVOCATION_DOMAIN, "delegation/v1", payload, &key(2));
        assert_eq!(
            links
                .admit_revocation(&bad, &dispatch())
                .map_err(|e| e.class()),
            Err("malformed")
        );
    }
    assert_eq!(links.status(&id, 900, &PROFILE), LinkStatus::Mutual);

    // Either key may withdraw, and a revoked link stays revoked even if
    // its statements arrive again.
    let by_second = sign_link_revocation(id, SIGNER, &key(2)).unwrap();
    assert_eq!(links.admit_revocation(&by_second, &dispatch()), Ok(id));
    assert_eq!(links.status(&id, 900, &PROFILE), LinkStatus::Revoked);
    links
        .admit_statement(&c.sign(SIGNER, &key(1)).unwrap(), &dispatch())
        .unwrap();
    assert_eq!(links.status(&id, 900, &PROFILE), LinkStatus::Revoked);
    assert_eq!(links.status(&id, 5_000, &PROFILE), LinkStatus::Revoked);
}

#[test]
fn a_mutual_link_grants_no_authority() {
    let owner = 0;
    let c = challenge(owner, 1);
    let mut links = KeyLinks::new();
    for seed in [owner, 1] {
        links
            .admit_statement(&c.sign(SIGNER, &key(seed)).unwrap(), &dispatch())
            .unwrap();
    }
    assert_eq!(
        links.status(&c.id().unwrap(), 900, &PROFILE),
        LinkStatus::Mutual
    );
    let grants = DelegationSet::new();
    let request = Request {
        space: SpaceAddress::for_key(public(owner)).unwrap(),
        key: public(1),
        path: vec![],
        permission: Permission::Read,
    };
    assert_eq!(
        grants.authorize(
            &request,
            None,
            delegation::Clock {
                verifier_utc: 900,
                frontier_age_secs: 0
            },
            &PROFILE
        ),
        Err(DelegationError::NotAuthorized("no grant presented"))
    );
}

#[test]
fn error_classes_are_stable() {
    let all = [
        (LinkError::Malformed("x"), "malformed"),
        (LinkError::WrongDomain(String::new()), "wrong_domain"),
        (LinkError::WrongSigner, "wrong_signer"),
        (LinkError::SameKey, "same_key"),
        (LinkError::UnknownChallenge, "unknown_challenge"),
    ];
    let mut seen = HashSet::new();
    for (error, class) in all {
        assert_eq!(error.class(), class);
        assert!(seen.insert(class));
        assert_ne!(error.to_string(), "");
    }
}

// ---------------------------------------------------------------------
// Property: the module's status matches the reference on random sets of
// statements over a base challenge and its near-miss variants.

const KEYS: u8 = 4;

#[derive(Debug, Clone)]
struct Statement {
    /// 0 = the base challenge; 1..=5 change one field.
    variant: u8,
    signer: u8,
    /// 0 = honest; 1 = flip a payload byte; 2 = wrong domain.
    damage: u8,
}

fn variant(base: (u8, u8), which: u8) -> LinkChallenge {
    let (a, b) = base;
    let mut c = challenge(a, b);
    match which {
        0 => {}
        1 => c = challenge(a, (0..KEYS).find(|k| *k != a && *k != b).unwrap()),
        2 => "backup-key".clone_into(&mut c.purpose),
        3 => c.scope = scope(&[&[1]]),
        4 => c.nonce = vec![8],
        _ => c.expires += 50,
    }
    c
}

fn make(c: &LinkChallenge, s: &Statement) -> Envelope {
    let bytes = c.challenge_bytes().unwrap();
    let domain = if s.damage == 2 {
        LINK_REVOCATION_DOMAIN
    } else {
        LINK_DOMAIN
    };
    let mut envelope = record(domain, "delegation/v1", bytes, &key(s.signer));
    if s.damage == 1 {
        envelope.payload[3] ^= 1;
    }
    envelope
}

fn statement() -> impl Strategy<Value = Statement> {
    (
        prop_oneof![3 => Just(0u8), 2 => 1..6u8],
        0..KEYS,
        prop_oneof![8 => Just(0u8), 1 => Just(1u8), 1 => Just(2u8)],
    )
        .prop_map(|(variant, signer, damage)| Statement {
            variant,
            signer,
            damage,
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn status_agrees_with_the_reference(
        pair in (0..KEYS, 1..KEYS).prop_map(|(a, d)| (a, (a + d) % KEYS)),
        statements in prop::collection::vec(statement(), 0..7),
        revokers in prop::collection::vec((0..6u8, 0..KEYS), 0..2),
        utc in prop_oneof![Just(900u64), 1_250..1_400u64],
    ) {
        let mut links = KeyLinks::new();
        // Reference state: exact challenge bytes -> signers.
        let mut ref_signed: HashMap<Vec<u8>, HashSet<[u8; 32]>> = HashMap::new();
        for s in &statements {
            let c = variant(pair, s.variant);
            let envelope = make(&c, s);
            let accepted = links.admit_statement(&envelope, &dispatch()).is_ok();
            let reference = ref_statement(&envelope);
            prop_assert_eq!(accepted, reference.is_some(), "statement {:?}", s);
            if let Some((_, signer)) = reference {
                ref_signed.entry(envelope.payload.clone()).or_default().insert(signer);
            }
        }
        let mut ref_revoked: HashSet<Vec<u8>> = HashSet::new();
        for (which, seed) in &revokers {
            let c = variant(pair, *which);
            let bytes = c.challenge_bytes().unwrap();
            let revocation = sign_link_revocation(c.id().unwrap(), SIGNER, &key(*seed)).unwrap();
            let accepted = links.admit_revocation(&revocation, &dispatch()).is_ok();
            let fields = ref_challenge(&bytes).unwrap();
            let expected = ref_signed.contains_key(&bytes)
                && (public(*seed) == fields.first || public(*seed) == fields.second);
            prop_assert_eq!(accepted, expected);
            if expected {
                ref_revoked.insert(bytes);
            }
        }
        for which in 0..6 {
            let c = variant(pair, which);
            let bytes = c.challenge_bytes().unwrap();
            let fields = ref_challenge(&bytes).unwrap();
            let id: ChallengeId = c.id().unwrap();
            let got = links.status(&id, utc, &PROFILE);
            let expected = match ref_signed.get(&bytes) {
                None => "unknown",
                Some(_) if ref_revoked.contains(&bytes) => "revoked",
                Some(_) if fields.expires + PROFILE.clock_skew_secs < utc => "expired",
                Some(signers) if signers.contains(&fields.first) && signers.contains(&fields.second) => "mutual",
                Some(_) => "pending",
            };
            let got_class = match &got {
                LinkStatus::Unknown => "unknown",
                LinkStatus::Revoked => "revoked",
                LinkStatus::Expired => "expired",
                LinkStatus::Mutual => "mutual",
                LinkStatus::Pending { signed_by } => {
                    prop_assert!(ref_signed[&bytes].contains(signed_by));
                    "pending"
                }
            };
            prop_assert_eq!(got_class, expected, "variant {}", which);
        }
    }
}
