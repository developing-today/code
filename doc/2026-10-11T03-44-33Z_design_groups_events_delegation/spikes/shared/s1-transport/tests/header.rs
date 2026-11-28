//! S1 checks on the event header (parts a, b, c).
//!
//! Integration tests link p2panda-core as an ordinary dependency, so its `cfg(test)` is off
//! and `feature = "test_utils"` must stay off too (see Cargo.toml). In that configuration
//! `Header::verify` is the `Provenance` trait method, which returns `true`.

use ed25519_dalek::Verifier;
use p2panda_core::operation::HeaderError;
use p2panda_core::traits::Provenance;
use p2panda_core::{
    Hash, Header, OperationError, Signature, SigningKey, VerifyingKey, validate_backlink,
    validate_header,
};
use s1_transport::header::{ChainExt, ChainExtPlus, Event, MAX_AFTER, build_event, check_received};

fn ext(after: usize) -> ChainExt {
    ChainExt {
        kind: "group.policy".into(),
        after: (0..after as u32)
            .map(|i| Hash::digest(i.to_be_bytes()))
            .collect(),
        group: None,
        domain: "id.events.v1".into(),
    }
}

fn event(key: &SigningKey, seq: u32, backlink: Option<Hash>) -> Event {
    build_event(key, seq, backlink, ext(1), Some(b"body"))
}

#[test]
fn round_trip_for_every_shape() {
    let key = SigningKey::generate();
    let prior = Hash::digest(b"prior");
    for after in [0usize, 1, 8, 32] {
        for body in [None, Some(vec![7u8; 100])] {
            for seq in [0u32, 1] {
                let backlink = (seq > 0).then_some(prior);
                let ev = build_event(&key, seq, backlink, ext(after), body.as_deref());
                let bytes = ev.encode();
                let back = Header::<ChainExt>::decode(&bytes).expect("decode");
                assert_eq!(back, ev, "after={after} seq={seq}");
                assert_eq!(back.encode(), bytes, "re-encode after={after} seq={seq}");
                let checked = check_received(&bytes).expect("our check accepts a valid event");
                assert_eq!(checked.id, ev.hash());
                assert_eq!(checked.after_len, after);
            }
        }
    }
    let grouped = build_event(
        &key,
        1,
        Some(prior),
        ChainExt {
            group: Some(Hash::digest(b"g")),
            ..ext(1)
        },
        None,
    );
    let bytes = grouped.encode();
    assert_eq!(Header::<ChainExt>::decode(&bytes).unwrap(), grouped);
    assert!(check_received(&bytes).is_ok());
}

/// (b) The trap. In a non-test build of p2panda-core, `Header::verify` returns true without
/// checking anything. Our check rejects the same bytes.
#[test]
fn typed_verify_accepts_a_forged_signature_in_a_non_test_build() {
    let key = SigningKey::generate();
    let mut forged = event(&key, 1, Some(Hash::digest(b"prior")));
    let mut sig = forged.signature.to_bytes();
    sig[0] ^= 1;
    forged.signature = Signature::from_bytes(&sig);

    assert!(
        Provenance::verify(&forged),
        "Header::verify (non-test build) returns true for a bad signature"
    );
    assert!(
        validate_header(&forged).is_ok(),
        "validate_header inherits the same true"
    );

    let bytes = forged.encode();
    let decoded =
        Header::<ChainExt>::decode(&bytes).expect_err("p2panda decode checks the signature");
    assert!(
        matches!(decoded, HeaderError::InvalidSignature),
        "{decoded:?}"
    );

    let ours = check_received(&bytes).expect_err("our check rejects the bytes");
    assert!(ours.contains("verify_strict"), "{ours}");
}

/// (b) Tampering with a typed field after signing also passes the typed check. The cached
/// hash does not change either, so the ID is stale.
#[test]
fn typed_verify_ignores_a_tampered_field_and_hash_is_cached() {
    let key = SigningKey::generate();
    let mut tampered = event(&key, 1, Some(Hash::digest(b"prior")));
    let original_id = tampered.hash();
    tampered.seq_num = 5;

    assert!(Provenance::verify(&tampered));
    assert!(validate_header(&tampered).is_ok());
    assert_eq!(
        tampered.hash(),
        original_id,
        "typed hash() is the cached digest"
    );
    assert_ne!(
        Hash::digest(tampered.encode()),
        original_id,
        "BLAKE3 over the bytes that would be sent differs"
    );
    assert!(check_received(&tampered.encode()).is_err());
}

/// (b) Plain Ed25519 accepts an identity public key with R = identity and S = 0 for any
/// message. verify_strict rejects it. p2panda's decode and our check both use verify_strict.
#[test]
fn identity_key_with_zero_signature_is_rejected_by_verify_strict() {
    let key = SigningKey::generate();
    let mut ev = event(&key, 0, None);
    let identity = {
        let mut b = [0u8; 32];
        b[0] = 1;
        b
    };
    ev.verifying_key = VerifyingKey::from_bytes(&identity).expect("identity encodes");
    let mut sig = [0u8; 64];
    sig[0] = 1;
    ev.signature = Signature::from_bytes(&sig);

    let signed = {
        let full = ev.encode();
        let value = cbor_core::Value::decode(&full).unwrap();
        let mut items = value.into_array().unwrap();
        items.remove(2);
        cbor_core::Value::from(items).encode()
    };
    let plain = ed25519_dalek::VerifyingKey::from_bytes(&identity)
        .unwrap()
        .verify(&signed, &ed25519_dalek::Signature::from_bytes(&sig));
    assert!(
        plain.is_ok(),
        "plain verify accepts the identity-key forgery"
    );

    let strict = ed25519_dalek::VerifyingKey::from_bytes(&identity)
        .unwrap()
        .verify_strict(&signed, &ed25519_dalek::Signature::from_bytes(&sig));
    assert!(strict.is_err(), "verify_strict rejects it");

    assert!(Header::<ChainExt>::decode(&ev.encode()).is_err());
    assert!(check_received(&ev.encode()).is_err());
}

/// (c) Base 0: the first event has no backlink, and the next one points at it.
#[test]
fn sequence_base_zero_chains_from_the_first_event() {
    let key = SigningKey::generate();
    let h0 = event(&key, 0, None);
    let h1 = event(&key, 1, Some(h0.hash()));
    assert!(validate_header(&h0).is_ok());
    assert!(validate_header(&h1).is_ok());
    assert!(validate_backlink(&h0, &h1).is_ok());
    assert!(check_received(&h0.encode()).is_ok());
    assert!(check_received(&h1.encode()).is_ok());
}

/// (c) Base 1: the first event at seq 1 needs a backlink that names nothing.
#[test]
fn sequence_base_one_needs_a_backlink_for_the_first_event() {
    let key = SigningKey::generate();
    let first = event(&key, 1, None);
    let err = validate_header(&first)
        .expect_err("validate_header rejects a seq-1 event with no backlink");
    assert!(matches!(err, OperationError::BacklinkMissing), "{err:?}");
    let decode =
        Header::<ChainExt>::decode(&first.encode()).expect_err("decode requires the backlink");
    // With no backlink, the extension map sits where the backlink hash should be.
    assert!(
        matches!(decode, HeaderError::UnexpectedFieldType(_, "backlink")),
        "{decode:?}"
    );

    let phantom = event(&key, 1, Some(Hash::digest(b"no such event")));
    assert!(
        validate_header(&phantom).is_ok(),
        "a phantom backlink passes header validation"
    );
    assert!(check_received(&phantom.encode()).is_ok());
}

/// (c) Fork gap: two different events at the same seq both pass validate_backlink against
/// the same predecessor. p2panda does not detect the fork. Our store must enforce it.
#[test]
fn two_siblings_at_the_same_seq_both_validate() {
    let key = SigningKey::generate();
    let h0 = event(&key, 0, None);
    let a = build_event(&key, 1, Some(h0.hash()), ext(1), Some(b"A"));
    let b = build_event(&key, 1, Some(h0.hash()), ext(1), Some(b"B"));
    assert_ne!(a.hash(), b.hash());
    assert!(validate_backlink(&h0, &a).is_ok());
    assert!(validate_backlink(&h0, &b).is_ok());
}

/// Extra: p2panda's typed decode ignores unknown extension keys. Our check rejects them.
#[test]
fn unknown_extension_key_is_ignored_by_p2panda_and_rejected_by_us() {
    let key = SigningKey::generate();
    let plus = ChainExtPlus {
        kind: "group.policy".into(),
        after: vec![],
        group: None,
        domain: "id.events.v1".into(),
        extra: 9,
    };
    let ev = Header::<ChainExtPlus>::builder()
        .seq_num(0)
        .build(&key, plus);
    let bytes = ev.encode();
    assert!(
        Header::<ChainExt>::decode(&bytes).is_ok(),
        "typed decode ignores the extra key"
    );
    let err = check_received(&bytes).expect_err("our check rejects the extra key");
    assert!(err.contains("unknown"), "{err}");
}

/// Extra: p2panda does not enforce MAX_AFTER. Our check does.
#[test]
fn after_over_max_is_decoded_by_p2panda_and_rejected_by_us() {
    let key = SigningKey::generate();
    let ev = build_event(&key, 0, None, ext(MAX_AFTER + 1), None);
    let bytes = ev.encode();
    assert!(Header::<ChainExt>::decode(&bytes).is_ok());
    let err = check_received(&bytes).expect_err("our check enforces MAX_AFTER");
    assert!(err.contains("max 32"), "{err}");
}
